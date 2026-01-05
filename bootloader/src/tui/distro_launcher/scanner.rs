use super::entry::BootEntry;
use crate::uefi::file_system::{FileProtocol, open_file_read, get_loaded_image};
use crate::BootServices;
use alloc::vec::Vec;
use alloc::string::{String, ToString};

const BOOT_ENTRIES_PATH: &str = "\\loader\\entries";
const MAX_FILE_SIZE: usize = 4096;

pub struct EntryScanner {
    boot_services: *const BootServices,
    image_handle: *mut (),
}

impl EntryScanner {
    pub fn new(boot_services: *const BootServices, image_handle: *mut ()) -> Self {
        Self {
            boot_services,
            image_handle,
        }
    }

    pub fn scan_boot_entries(&self) -> Vec<BootEntry> {
        let mut entries = Vec::new();

        unsafe {
            if let Ok(root) = self.get_esp_root() {
                if let Ok(conf_entries) = self.scan_loader_entries(root) {
                    for entry in conf_entries {
                        if self.kernel_exists(root, &entry.kernel_path) {
                            entries.push(entry);
                        }
                    }
                }

                if entries.is_empty() {
                    if let Ok(kernel_entries) = self.scan_kernels(root) {
                        entries.extend(kernel_entries);
                    }
                }

                ((*root).close)(root);
            }
        }

        if entries.is_empty() {
            entries.push(self.create_fallback_entry());
        }

        entries
    }

    unsafe fn kernel_exists(&self, root: *mut FileProtocol, path: &str) -> bool {
        let path_utf16 = Self::str_to_utf16(path);
        if let Ok(file) = open_file_read(root, &path_utf16) {
            ((*file).close)(file);
            true
        } else {
            false
        }
    }

    unsafe fn get_esp_root(&self) -> Result<*mut FileProtocol, ()> {
        let loaded_image = get_loaded_image(&*self.boot_services, self.image_handle)?;
        let device_handle = (*loaded_image).device_handle;
        
        let mut file_system: *mut () = core::ptr::null_mut();
        let guid = crate::uefi::file_system::SIMPLE_FILE_SYSTEM_PROTOCOL_GUID;
        
        let status = ((*self.boot_services).handle_protocol)(
            device_handle,
            &guid,
            &mut file_system,
        );

        if status != 0 {
            return Err(());
        }

        let fs_proto = file_system as *mut crate::uefi::file_system::SimpleFileSystemProtocol;
        let mut root: *mut FileProtocol = core::ptr::null_mut();
        
        let status = ((*fs_proto).open_volume)(fs_proto, &mut root);
        if status != 0 {
            return Err(());
        }

        Ok(root)
    }

    unsafe fn scan_kernels(&self, root: *mut FileProtocol) -> Result<Vec<BootEntry>, ()> {
        let mut entries = Vec::new();
        let kernel_dir_path = Self::str_to_utf16("\\kernels");
        
        if let Ok(kernel_dir) = open_file_read(root, &kernel_dir_path) {
            entries.extend(self.enumerate_kernels(kernel_dir)?);
            ((*kernel_dir).close)(kernel_dir);
        }

        Ok(entries)
    }

    unsafe fn enumerate_kernels(&self, dir: *mut FileProtocol) -> Result<Vec<BootEntry>, ()> {
        let mut entries = Vec::new();
        let mut buffer = [0u8; 512];
        
        loop {
            let mut buffer_size = buffer.len();
            let status = ((*dir).read)(dir, &mut buffer_size, buffer.as_mut_ptr());
            
            if status != 0 || buffer_size == 0 {
                break;
            }

            if let Some(entry) = self.parse_file_info(&buffer[..buffer_size]) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    fn parse_file_info(&self, data: &[u8]) -> Option<BootEntry> {
        if data.len() < 8 {
            return None;
        }

        let attr = data[4];
        if attr & 0x10 != 0 {
            return None;
        }

        let filename = Self::extract_filename(data)?;
        
        if !filename.starts_with("vmlinuz") {
            return None;
        }

        let kernel_path = alloc::format!("\\kernels\\{}", filename);
        let distro_name = Self::extract_distro_name(&filename);
        
        let initrd_path = Self::guess_initrd_path(&filename);
        let cmdline = Self::generate_cmdline(&distro_name);

        Some(BootEntry::new(
            distro_name,
            kernel_path,
            initrd_path,
            cmdline,
        ))
    }

    fn extract_filename(data: &[u8]) -> Option<String> {
        let mut name = String::new();
        let mut i = 8;
        
        while i + 1 < data.len() {
            let ch = u16::from_le_bytes([data[i], data[i + 1]]);
            if ch == 0 {
                break;
            }
            if ch < 128 {
                name.push(ch as u8 as char);
            }
            i += 2;
        }

        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    fn extract_distro_name(filename: &str) -> String {
        if let Some(suffix) = filename.strip_prefix("vmlinuz-") {
            suffix.to_string()
        } else {
            "Unknown".to_string()
        }
    }

    fn guess_initrd_path(kernel_name: &str) -> Option<String> {
        if let Some(suffix) = kernel_name.strip_prefix("vmlinuz-") {
            let initrd = alloc::format!("\\initrds\\initrd-{}.img", suffix);
            Some(initrd)
        } else {
            None
        }
    }

    fn generate_cmdline(distro: &str) -> String {
        match distro {
            name if name.contains("tails") => {
                "boot=live live-media-path=/live nopersistence noprompt timezone=Etc/UTC console=ttyS0,115200 console=tty1".to_string()
            }
            name if name.contains("ubuntu") => {
                "boot=casper quiet splash console=ttyS0,115200 console=tty*".to_string()
            }
            name if name.contains("debian") => {
                "boot=live quiet console=ttyS0,115200 console=tty*".to_string()
            }
            name if name.contains("arch") => {
                "root=/dev/ram0 rw console=ttyS0,115200 console=tty* debug".to_string()
            }
            name if name.contains("fedora") => {
                "rd.live.image quiet console=ttyS0,115200 console=tty*".to_string()
            }
            name if name.contains("kali") => {
                "boot=live quiet console=ttyS0,115200 console=tty*".to_string()
            }
            _ => {
                "console=ttyS0,115200 console=tty0".to_string()
            }
        }
    }

    unsafe fn scan_loader_entries(&self, root: *mut FileProtocol) -> Result<Vec<BootEntry>, ()> {
        let mut entries = Vec::new();
        let entries_path = Self::str_to_utf16(BOOT_ENTRIES_PATH);
        
        if let Ok(entries_dir) = open_file_read(root, &entries_path) {
            let mut buffer = [0u8; 512];
            
            loop {
                let mut buffer_size = buffer.len();
                let status = ((*entries_dir).read)(entries_dir, &mut buffer_size, buffer.as_mut_ptr());
                
                if status != 0 || buffer_size == 0 {
                    break;
                }

                if let Some(filename) = Self::extract_filename(&buffer[..buffer_size]) {
                    if filename.ends_with(".conf") {
                        let conf_path = alloc::format!("{}\\{}", BOOT_ENTRIES_PATH, filename);
                        if let Ok(entry) = self.parse_conf_file(root, &conf_path) {
                            entries.push(entry);
                        }
                    }
                }
            }
            
            ((*entries_dir).close)(entries_dir);
        }

        Ok(entries)
    }

    unsafe fn parse_conf_file(&self, root: *mut FileProtocol, path: &str) -> Result<BootEntry, ()> {
        let path_utf16 = Self::str_to_utf16(path);
        let file = open_file_read(root, &path_utf16)?;
        
        let mut buffer = [0u8; MAX_FILE_SIZE];
        let mut size = MAX_FILE_SIZE;
        let status = ((*file).read)(file, &mut size, buffer.as_mut_ptr());
        ((*file).close)(file);
        
        if status != 0 {
            return Err(());
        }

        let content = core::str::from_utf8(&buffer[..size]).map_err(|_| ())?;
        
        let mut title = String::new();
        let mut linux = String::new();
        let mut initrd = None;
        let mut options = String::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(value) = line.strip_prefix("title") {
                title = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("linux") {
                linux = value.trim().replace('/', "\\").to_string();
            } else if let Some(value) = line.strip_prefix("initrd") {
                initrd = Some(value.trim().replace('/', "\\").to_string());
            } else if let Some(value) = line.strip_prefix("options") {
                options = value.trim().to_string();
            }
        }

        if title.is_empty() || linux.is_empty() {
            return Err(());
        }

        Ok(BootEntry::new(title, linux, initrd, options))
    }

    fn create_fallback_entry(&self) -> BootEntry {
        BootEntry::new(
            "Fallback (No OS Found)".to_string(),
            "\\EFI\\BOOT\\BOOTX64.EFI".to_string(),
            None,
            "".to_string(),
        )
    }

    fn str_to_utf16(s: &str) -> Vec<u16> {
        let mut buf = Vec::with_capacity(s.len() + 1);
        for byte in s.bytes() {
            buf.push(byte as u16);
        }
        buf.push(0);
        buf
    }
}
