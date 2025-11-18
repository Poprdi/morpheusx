use super::entry::BootEntry;
use crate::uefi::file_system::{FileProtocol, open_file_read, get_loaded_image};
use crate::BootServices;
use alloc::vec::Vec;
use alloc::string::{String, ToString};

const BOOT_ENTRIES_PATH: &str = "\\loader\\entries";
const MAX_PATH_LEN: usize = 256;
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

        entries.push(BootEntry::new(
            "Arch Linux".to_string(),
            "\\kernels\\vmlinuz-arch".to_string(),
            Some("\\initrds\\initramfs-arch.img".to_string()),
            "root=/dev/ram0 rw console=ttyS0,115200 console=tty0 debug".to_string(),
        ));

        entries.push(BootEntry::new(
            "Ubuntu 20.04".to_string(),
            "\\kernels\\vmlinuz-ubuntu2004".to_string(),
            None,
            "boot=casper quiet splash console=ttyS0,115200 console=tty0".to_string(),
        ));

        entries.push(BootEntry::new(
            "Fedora 36".to_string(),
            "\\kernels\\vmlinuz-fedora36".to_string(),
            None,
            "rd.live.image quiet console=ttyS0,115200 console=tty0".to_string(),
        ));

        entries.push(BootEntry::new(
            "Generic Kernel".to_string(),
            "\\kernels\\vmlinuz".to_string(),
            None,
            "console=ttyS0,115200 console=tty0".to_string(),
        ));

        entries.push(BootEntry::new(
            "Tails OS".to_string(),
            "\\kernels\\vmlinuz-tails".to_string(),
            Some("\\initrds\\initrd-tails.img".to_string()),
            "boot=live live-media-path=/initrds nopersistence noprompt timezone=Etc/UTC console=ttyS0,115200 console=tty1".to_string(),
        ));

        entries
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
                "boot=live live-media-path=/initrds nopersistence noprompt timezone=Etc/UTC console=ttyS0,115200 console=tty1".to_string()
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
            ((*entries_dir).close)(entries_dir);
        }

        Ok(entries)
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
