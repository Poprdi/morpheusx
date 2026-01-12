use super::entry::BootEntry;
use super::iso_boot::IsoScanner;
use crate::uefi::file_system::{get_loaded_image, open_file_read, FileProtocol};
use crate::BootServices;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use morpheus_core::iso::IsoStorageManager;

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

        // Scan for ISO files in .iso directory (legacy single-file ISOs)
        let iso_scanner = IsoScanner::new(self.boot_services, self.image_handle);
        let iso_entries = iso_scanner.scan_iso_files();
        entries.extend(iso_entries);

        // Scan for chunked ISOs from storage manager
        let chunked_entries = self.scan_chunked_isos();
        entries.extend(chunked_entries);

        if entries.is_empty() {
            entries.push(self.create_fallback_entry());
        }

        entries
    }

    /// Scan for ISOs stored in chunked format via IsoStorageManager
    fn scan_chunked_isos(&self) -> Vec<BootEntry> {
        let mut entries = Vec::new();

        morpheus_core::logger::log("=== SCANNING FOR CHUNKED ISOS ===");

        // Get disk info for storage manager
        let (esp_lba, disk_lba) = unsafe {
            let bs = &*self.boot_services;
            let mut dm = morpheus_core::disk::manager::DiskManager::new();
            if crate::uefi::disk::enumerate_disks(bs, &mut dm).is_ok() && dm.disk_count() > 0 {
                if let Some(disk) = dm.get_disk(0) {
                    morpheus_core::logger::log(
                        format!("Disk found: {} blocks", disk.last_block + 1).leak(),
                    );
                    (2048_u64, disk.last_block + 1)
                } else {
                    morpheus_core::logger::log("No disk found");
                    return entries;
                }
            } else {
                morpheus_core::logger::log("Failed to enumerate disks");
                return entries;
            }
        };

        // Create storage manager and load persisted manifests from ESP
        let mut storage = IsoStorageManager::new(esp_lba, disk_lba);

        unsafe {
            let bs = &*self.boot_services;
            // Load manifests from /.iso/*.MFS on ESP
            match crate::tui::distro_downloader::manifest_io::load_manifests_from_esp(
                bs,
                self.image_handle,
                &mut storage,
            ) {
                Ok(count) => {
                    morpheus_core::logger::log(
                        format!("load_manifests_from_esp returned {} manifests", count).leak(),
                    );
                }
                Err(e) => {
                    morpheus_core::logger::log(
                        format!("load_manifests_from_esp FAILED: {:?}", e).leak(),
                    );
                }
            }
        }

        morpheus_core::logger::log(format!("Storage has {} entries", storage.count()).leak());

        for (idx, entry) in storage.iter().enumerate() {
            let manifest = &entry.1.manifest;
            morpheus_core::logger::log(
                format!(
                    "Entry {}: name='{}', complete={}, flags=0x{:02x}",
                    idx,
                    manifest.name_str(),
                    manifest.is_complete(),
                    manifest.flags
                )
                .leak(),
            );

            // Only show complete ISOs
            if !manifest.is_complete() {
                morpheus_core::logger::log("  -> Skipping (not complete)");
                continue;
            }

            let name = manifest.name_str();
            let distro_name = Self::extract_distro_from_name(name);

            // Create entry with special chunked: prefix to indicate chunked ISO
            entries.push(BootEntry::new(
                format!("{} (Chunked ISO)", distro_name),
                format!("chunked:{}", idx), // Special path indicating chunked ISO index
                None,
                format!("chunked_iso:{}", idx),
            ));
        }

        morpheus_core::logger::log(format!("Found {} chunked ISOs", entries.len()).leak());

        entries
    }

    fn extract_distro_from_name(name: &str) -> String {
        let name_lower = name.to_lowercase();

        if name_lower.contains("tails") {
            "Tails".to_string()
        } else if name_lower.contains("ubuntu") {
            "Ubuntu".to_string()
        } else if name_lower.contains("debian") {
            "Debian".to_string()
        } else if name_lower.contains("arch") {
            "Arch".to_string()
        } else if name_lower.contains("fedora") {
            "Fedora".to_string()
        } else if name_lower.contains("kali") {
            "Kali".to_string()
        } else {
            // Remove .iso extension if present
            name.strip_suffix(".iso")
                .or_else(|| name.strip_suffix(".ISO"))
                .unwrap_or(name)
                .to_string()
        }
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

        let status =
            ((*self.boot_services).handle_protocol)(device_handle, &guid, &mut file_system);

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
        // EFI_FILE_INFO: Attribute is at offset 72 (8 bytes)
        if data.len() < 82 {
            return None;
        }

        // Check if it's a directory (attribute bit 4)
        let attr = u64::from_le_bytes([
            data[72], data[73], data[74], data[75], data[76], data[77], data[78], data[79],
        ]);
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
        // EFI_FILE_INFO structure:
        // offset 0:  Size (8 bytes)
        // offset 8:  FileSize (8 bytes)
        // offset 16: PhysicalSize (8 bytes)
        // offset 24: CreateTime (16 bytes)
        // offset 40: LastAccessTime (16 bytes)
        // offset 56: ModificationTime (16 bytes)
        // offset 72: Attribute (8 bytes)
        // offset 80: FileName[] (CHAR16, null-terminated)

        if data.len() < 82 {
            return None;
        }

        let mut name = String::new();
        let mut i = 80;

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

        if name.is_empty() || name == "." || name == ".." {
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
                let status =
                    ((*entries_dir).read)(entries_dir, &mut buffer_size, buffer.as_mut_ptr());

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
        let file = open_file_read(root, &path_utf16).map_err(|_| ())?;

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

            let mut parts = line.splitn(2, |c: char| c.is_whitespace());
            let key = parts.next().unwrap_or("");
            let value = parts.next().unwrap_or("").trim();

            match key {
                "title" => title = value.to_string(),
                "linux" => linux = value.replace('/', "\\").to_string(),
                "initrd" => initrd = Some(value.replace('/', "\\").to_string()),
                "options" => options = value.to_string(),
                _ => {}
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
        // Proper UTF-16 encoding, not just ASCII bytes
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }
}
