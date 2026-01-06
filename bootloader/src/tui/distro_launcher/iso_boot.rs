//! ISO boot entry scanner
//!
//! Scans for ISO files on ESP and creates boot entries from them

use super::entry::BootEntry;
use crate::uefi::file_system::{FileProtocol, open_file_read};
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::format;

const ISO_DIR_PATH: &str = "\\isos";
const MAX_ISO_SIZE: usize = 2 * 1024 * 1024 * 1024; // 2GB max

pub struct IsoScanner {
    boot_services: *const BootServices,
    image_handle: *mut (),
}

impl IsoScanner {
    pub fn new(boot_services: *const BootServices, image_handle: *mut ()) -> Self {
        Self {
            boot_services,
            image_handle,
        }
    }

    /// Scan for ISO files and create boot entries
    pub fn scan_iso_files(&self) -> Vec<BootEntry> {
        let mut entries = Vec::new();

        unsafe {
            if let Ok(root) = self.get_esp_root() {
                let iso_dir_path = Self::str_to_utf16(ISO_DIR_PATH);
                
                if let Ok(iso_dir) = open_file_read(root, &iso_dir_path) {
                    if let Ok(iso_entries) = self.enumerate_isos(iso_dir) {
                        entries.extend(iso_entries);
                    }
                    ((*iso_dir).close)(iso_dir);
                }

                ((*root).close)(root);
            }
        }

        entries
    }

    unsafe fn get_esp_root(&self) -> Result<*mut FileProtocol, ()> {
        let loaded_image = crate::uefi::file_system::get_loaded_image(&*self.boot_services, self.image_handle)?;
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

    unsafe fn enumerate_isos(&self, dir: *mut FileProtocol) -> Result<Vec<BootEntry>, ()> {
        let mut entries = Vec::new();
        let mut buffer = [0u8; 512];
        
        loop {
            let mut buffer_size = buffer.len();
            let status = ((*dir).read)(dir, &mut buffer_size, buffer.as_mut_ptr());
            
            if status != 0 || buffer_size == 0 {
                break;
            }

            if let Some(entry) = self.parse_iso_file(&buffer[..buffer_size]) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    fn parse_iso_file(&self, data: &[u8]) -> Option<BootEntry> {
        if data.len() < 82 {
            return None;
        }

        // Check if it's a directory (attribute bit 4)
        let attr = u64::from_le_bytes([
            data[72], data[73], data[74], data[75],
            data[76], data[77], data[78], data[79],
        ]);
        if attr & 0x10 != 0 {
            return None; // Skip directories
        }

        let filename = Self::extract_filename(data)?;
        
        // Only process .iso files
        if !filename.to_lowercase().ends_with(".iso") {
            return None;
        }

        // TODO: Mount ISO and extract boot info using iso9660 crate
        // For now, create boot entry with ISO path as "kernel"
        let iso_path = format!("\\isos\\{}", filename);
        let distro_name = Self::extract_distro_from_filename(&filename);
        
        Some(BootEntry::new(
            format!("{} (ISO)", distro_name),
            iso_path.clone(), // ISO path as kernel for now
            None,
            format!("iso={} boot=live", iso_path),
        ))
    }

    fn extract_filename(data: &[u8]) -> Option<String> {
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

    fn extract_distro_from_filename(filename: &str) -> String {
        let name_lower = filename.to_lowercase();
        
        if name_lower.contains("tails") {
            "Tails"
        } else if name_lower.contains("ubuntu") {
            "Ubuntu"
        } else if name_lower.contains("debian") {
            "Debian"
        } else if name_lower.contains("arch") {
            "Arch"
        } else if name_lower.contains("fedora") {
            "Fedora"
        } else if name_lower.contains("kali") {
            "Kali"
        } else {
            filename.strip_suffix(".iso")
                .or_else(|| filename.strip_suffix(".ISO"))
                .unwrap_or(filename)
        }.to_string()
    }

    fn str_to_utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }
}

/// Mount an ISO file and extract kernel + initrd information
/// 
/// Returns (kernel_data, initrd_data, boot_params)
pub fn extract_iso_boot_files(
    iso_path: &str,
    esp_root: *mut FileProtocol,
) -> Result<(Vec<u8>, Option<Vec<u8>>, String), IsoBootError> {
    unsafe {
        // Open ISO file
        let iso_path_utf16: Vec<u16> = iso_path.encode_utf16().chain(core::iter::once(0)).collect();
        let iso_file = open_file_read(esp_root, &iso_path_utf16)
            .map_err(|_| IsoBootError::IsoNotFound)?;

        // Get file size
        let file_size = get_file_size(iso_file)?;
        
        if file_size > MAX_ISO_SIZE {
            ((*iso_file).close)(iso_file);
            return Err(IsoBootError::IsoTooLarge);
        }

        // Read ISO into memory (required for iso9660 parsing)
        let mut iso_data = alloc::vec![0u8; file_size];
        let mut read_size = file_size;
        let status = ((*iso_file).read)(iso_file, &mut read_size, iso_data.as_mut_ptr());
        ((*iso_file).close)(iso_file);

        if status != 0 || read_size != file_size {
            return Err(IsoBootError::ReadFailed);
        }

        // Create in-memory block device
        let mut mem_device = MemoryBlockDevice::new(iso_data);

        // Mount ISO using iso9660
        let volume = iso9660::mount(&mut mem_device, 0)
            .map_err(|_| IsoBootError::MountFailed)?;

        // Try to find kernel - common paths for live distros
        let kernel_paths = [
            "/casper/vmlinuz",           // Ubuntu
            "/live/vmlinuz",             // Debian/Tails
            "/arch/boot/x86_64/vmlinuz", // Arch
            "/isolinux/vmlinuz",         // Generic
            "/boot/vmlinuz",             // Fallback
        ];

        let mut kernel_entry = None;
        for path in &kernel_paths {
            if let Ok(entry) = iso9660::find_file(&mut mem_device, &volume, path) {
                kernel_entry = Some(entry);
                break;
            }
        }

        let kernel = kernel_entry.ok_or(IsoBootError::KernelNotFound)?;

        // Read kernel data
        let mut kernel_data = alloc::vec![0u8; kernel.size as usize];
        iso9660::read_file(&mut mem_device, &kernel, &mut kernel_data)
            .map_err(|_| IsoBootError::ReadFailed)?;

        // Try to find initrd
        let initrd_paths = [
            "/casper/initrd",
            "/casper/initrd.lz",
            "/live/initrd.img",
            "/arch/boot/x86_64/archiso.img",
            "/isolinux/initrd.img",
            "/boot/initrd.img",
        ];

        let mut initrd_data = None;
        for path in &initrd_paths {
            if let Ok(entry) = iso9660::find_file(&mut mem_device, &volume, path) {
                let mut data = alloc::vec![0u8; entry.size as usize];
                if iso9660::read_file(&mut mem_device, &entry, &mut data).is_ok() {
                    initrd_data = Some(data);
                    break;
                }
            }
        }

        // Generate boot parameters
        let boot_params = format!("boot=live iso-scan/filename={}", iso_path);

        Ok((kernel_data, initrd_data, boot_params))
    }
}

unsafe fn get_file_size(file: *mut FileProtocol) -> Result<usize, IsoBootError> {
    let mut info_buffer = [0u8; 512];
    let mut buffer_size = info_buffer.len();
    
    // EFI_FILE_INFO GUID
    let info_guid = uguid::guid!("09576e92-6d3f-11d2-8e39-00a0c969723b");
    
    let status = ((*file).get_info)(
        file,
        &info_guid,
        &mut buffer_size,
        info_buffer.as_mut_ptr() as *mut (),
    );

    if status != 0 {
        return Err(IsoBootError::ReadFailed);
    }

    // File size is at offset 8 (8 bytes)
    if buffer_size >= 16 {
        let size = u64::from_le_bytes([
            info_buffer[8], info_buffer[9], info_buffer[10], info_buffer[11],
            info_buffer[12], info_buffer[13], info_buffer[14], info_buffer[15],
        ]);
        Ok(size as usize)
    } else {
        Err(IsoBootError::ReadFailed)
    }
}

/// In-memory block device for ISO data
struct MemoryBlockDevice {
    data: Vec<u8>,
}

impl MemoryBlockDevice {
    fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

impl gpt_disk_io::BlockIo for MemoryBlockDevice {
    fn read_blocks(&mut self, lba: gpt_disk_types::Lba, buffer: &mut [u8]) -> Result<(), ()> {
        let offset = lba.0 as usize * 2048;
        if offset + buffer.len() > self.data.len() {
            return Err(());
        }
        buffer.copy_from_slice(&self.data[offset..offset + buffer.len()]);
        Ok(())
    }

    fn write_blocks(&mut self, _lba: gpt_disk_types::Lba, _buffer: &[u8]) -> Result<(), ()> {
        Err(()) // Read-only
    }

    fn block_size(&self) -> u64 {
        2048
    }
}

#[derive(Debug)]
pub enum IsoBootError {
    IsoNotFound,
    IsoTooLarge,
    ReadFailed,
    MountFailed,
    KernelNotFound,
    InvalidIso,
}
