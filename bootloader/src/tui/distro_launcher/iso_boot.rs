//! ISO boot entry scanner
//!
//! Scans for ISO files on ESP and creates boot entries from them

use super::entry::BootEntry;
use crate::uefi::file_system::{FileProtocol, open_file_read, FILE_INFO_GUID};
use crate::BootServices;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};
use iso9660::{find_file, mount, read_file};
use core::fmt;

const ISO_DIR_PATH: &str = "\\.iso";
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
        morpheus_core::logger::log("IsoScanner::scan_iso_files() - starting");
        let mut entries = Vec::new();

        unsafe {
            if let Ok(root) = self.get_esp_root() {
                morpheus_core::logger::log("IsoScanner: got ESP root");
                let iso_dir_path = Self::str_to_utf16(ISO_DIR_PATH);
                
                if let Ok(iso_dir) = open_file_read(root, &iso_dir_path) {
                    morpheus_core::logger::log("IsoScanner: opened .iso directory");
                    if let Ok(iso_entries) = self.enumerate_isos(iso_dir) {
                        morpheus_core::logger::log(
                            alloc::format!("IsoScanner: found {} ISOs", iso_entries.len()).leak(),
                        );
                        entries.extend(iso_entries);
                    }
                    ((*iso_dir).close)(iso_dir);
                } else {
                    morpheus_core::logger::log("IsoScanner: failed to open .iso directory");
                }

                ((*root).close)(root);
            } else {
                morpheus_core::logger::log("IsoScanner: failed to get ESP root");
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

        let distro_name = Self::extract_distro_from_filename(&filename);
        let iso_path = format!("\\.iso\\{}", filename);

        // Create a lazy ISO entry. Actual extraction is deferred to boot time.
        Some(BootEntry::new(
            format!("{} (ISO)", distro_name),
            iso_path.clone(),
            None,
            format!("iso:{}", iso_path),
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
        morpheus_core::logger::log("extract_iso: starting");
        
        // 1. Open ISO file
        let iso_path_utf16: Vec<u16> = iso_path
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        let iso_file = open_file_read(esp_root, &iso_path_utf16)
            .map_err(|_| {
                morpheus_core::logger::log("extract_iso: FAIL open_file_read");
                IsoBootError::IsoNotFound
            })?;
        morpheus_core::logger::log("extract_iso: file opened");

        // 2. Determine size and guard against oversized images
        let file_size = get_file_size(iso_file)?;
        morpheus_core::logger::log(
            alloc::format!("extract_iso: file_size = {} bytes", file_size).leak(),
        );
        if file_size > MAX_ISO_SIZE {
            ((*iso_file).close)(iso_file);
            return Err(IsoBootError::IsoTooLarge);
        }

        // 3. Read ISO into memory (BlockIo requires contiguous backing)
        morpheus_core::logger::log("extract_iso: allocating buffer");
        let mut iso_data = alloc::vec![0u8; file_size];
        morpheus_core::logger::log("extract_iso: reading file");
        let mut read_size = file_size;
        let status = ((*iso_file).read)(iso_file, &mut read_size, iso_data.as_mut_ptr());
        ((*iso_file).close)(iso_file);

        morpheus_core::logger::log(
            alloc::format!("extract_iso: read status={}, read_size={}", status, read_size).leak(),
        );
        if status != 0 || read_size != file_size {
            morpheus_core::logger::log("extract_iso: FAIL file read mismatch");
            return Err(IsoBootError::ReadFailed);
        }

        // 4. Wrap buffer in a BlockIo implementation
        morpheus_core::logger::log("extract_iso: creating MemoryBlockDevice");
        let mut mem_device = MemoryBlockDevice::new(iso_data);

        // 5. Mount ISO9660 volume
        morpheus_core::logger::log("extract_iso: mounting ISO9660");
        let volume = mount(&mut mem_device, 0).map_err(|e| {
            morpheus_core::logger::log(
                alloc::format!("extract_iso: FAIL mount: {:?}", e).leak(),
            );
            IsoBootError::MountFailed
        })?;
        morpheus_core::logger::log("extract_iso: ISO mounted successfully");

        // 6. Locate kernel using common distro paths
        let kernel_paths = [
            "/casper/vmlinuz",           // Ubuntu/Kubuntu/Xubuntu
            "/casper/vmlinuz.efi",       // Ubuntu EFI
            "/live/vmlinuz",             // Debian/Tails
            "/arch/boot/x86_64/vmlinuz", // Arch Linux
            "/isolinux/vmlinuz",         // Generic syslinux
            "/boot/vmlinuz",             // Fallback
            "/EFI/boot/vmlinuz",         // EFI fallback
        ];

        let mut kernel_entry = None;
        let mut kernel_path_found = "";
        for path in &kernel_paths {
            if let Ok(entry) = find_file(&mut mem_device, &volume, path) {
                kernel_entry = Some(entry);
                kernel_path_found = path;
                break;
            }
        }

        let kernel = kernel_entry.ok_or(IsoBootError::KernelNotFound)?;

        // 7. Read kernel bytes
        let mut kernel_data = alloc::vec![0u8; kernel.size as usize];
        read_file(&mut mem_device, &kernel, &mut kernel_data)
            .map_err(|_| IsoBootError::ReadFailed)?;

        // 8. Locate initrd based on kernel placement
        let initrd_paths = match kernel_path_found {
            p if p.contains("casper") => vec![
                "/casper/initrd",
                "/casper/initrd.lz",
                "/casper/initrd.img",
            ],
            p if p.contains("live") => vec![
                "/live/initrd.img",
                "/live/initrd",
            ],
            p if p.contains("arch") => vec![
                "/arch/boot/x86_64/archiso.img",
                "/arch/boot/intel_ucode.img",
            ],
            _ => vec![
                "/isolinux/initrd.img",
                "/boot/initrd.img",
                "/initrd.img",
            ],
        };

        let mut initrd_data = None;
        for path in &initrd_paths {
            if let Ok(entry) = find_file(&mut mem_device, &volume, path) {
                let mut data = alloc::vec![0u8; entry.size as usize];
                if read_file(&mut mem_device, &entry, &mut data).is_ok() {
                    initrd_data = Some(data);
                    break;
                }
            }
        }

        // 9. Build cmdline tailored to distro layout
        let cmdline = generate_iso_cmdline(iso_path, kernel_path_found);

        Ok((kernel_data, initrd_data, cmdline))
    }
}

fn generate_iso_cmdline(iso_path: &str, kernel_path: &str) -> String {
    let linux_path = iso_path.replace('\\', "/");

    match kernel_path {
        p if p.contains("casper") => {
            // Ubuntu/derivatives
            alloc::format!(
                "boot=casper iso-scan/filename={} quiet splash console=ttyS0,115200 console=tty1",
                linux_path
            )
        }
        p if p.contains("live") => {
            // Debian/Tails live-boot
            alloc::format!(
                "boot=live findiso={} live-media-path=/live nopersistence console=ttyS0,115200 console=tty1",
                linux_path
            )
        }
        p if p.contains("arch") => {
            // Arch ISO
            alloc::format!(
                "archisobasedir=arch img_dev=/dev/disk/by-label/ESP img_loop={} console=ttyS0,115200",
                linux_path
            )
        }
        _ => {
            // Fallback
            alloc::format!(
                "root=/dev/ram0 rw iso-scan/filename={} console=ttyS0,115200",
                linux_path
            )
        }
    }
}

unsafe fn get_file_size(file: *mut FileProtocol) -> Result<usize, IsoBootError> {
    let mut info_buffer = [0u8; 512];
    let mut buffer_size = info_buffer.len();
    
    // Cast get_info from usize to proper function pointer
    type GetInfoFn = extern "efiapi" fn(
        this: *mut FileProtocol,
        info_type: *const [u8; 16],
        buffer_size: *mut usize,
        buffer: *mut u8,
    ) -> usize;
    
    let get_info_fn: GetInfoFn = core::mem::transmute((*file).get_info);
    
    let status = get_info_fn(
        file,
        &FILE_INFO_GUID,
        &mut buffer_size,
        info_buffer.as_mut_ptr(),
    );

    if status != 0 {
        return Err(IsoBootError::ReadFailed);
    }

    // File size is at offset 8 (8 bytes) in EFI_FILE_INFO
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

/// Error type for MemoryBlockDevice I/O operations
#[derive(Debug, Clone, Copy)]
struct MemoryBlockIoError;

impl fmt::Display for MemoryBlockIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "memory block device I/O error")
    }
}

impl BlockIo for MemoryBlockDevice {
    type Error = MemoryBlockIoError;

    fn block_size(&self) -> BlockSize {
        // ISO9660 sector size is 2048 bytes
        // SAFETY: 2048 is a valid block size (>= 512)
        BlockSize::new(2048).expect("2048 is a valid block size")
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok((self.data.len() / 2048) as u64)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 as usize * 2048;
        if offset + dst.len() > self.data.len() {
            return Err(MemoryBlockIoError);
        }
        dst.copy_from_slice(&self.data[offset..offset + dst.len()]);
        Ok(())
    }

    fn write_blocks(&mut self, _start_lba: Lba, _src: &[u8]) -> Result<(), Self::Error> {
        Err(MemoryBlockIoError) // Read-only
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(()) // No-op for in-memory device
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
