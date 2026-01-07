//! ISO Boot Support
//!
//! Boots Linux kernels from chunked ISO storage using iso9660 parsing.
//!
//! # Boot Flow
//!
//! ```text
//! 1. Get IsoReadContext from IsoStorageManager
//! 2. Create IsoBlockIoAdapter wrapping disk block I/O
//! 3. Mount ISO via iso9660::mount()
//! 4. Find boot image via iso9660::find_boot_image()
//! 5. Extract kernel (vmlinuz) and initrd from ISO
//! 6. Call boot_linux_kernel() with extracted files
//! ```

use crate::boot::loader::{boot_linux_kernel, BootError};
use crate::tui::renderer::Screen;
use crate::BootServices;
use morpheus_core::iso::{IsoBlockIoAdapter, IsoReadContext, IsoError};
use gpt_disk_io::BlockIo;

extern crate alloc;
use alloc::vec::Vec;

/// Error during ISO boot process
#[derive(Debug)]
pub enum IsoBootError {
    /// ISO storage error
    Storage(IsoError),
    /// Failed to mount ISO filesystem
    MountFailed,
    /// No bootable image in ISO
    NoBootImage,
    /// Failed to find kernel
    KernelNotFound,
    /// Failed to read kernel
    KernelReadFailed,
    /// Failed to find initrd
    InitrdNotFound,
    /// Failed to read initrd
    InitrdReadFailed,
    /// Boot process failed
    BootFailed(BootError),
}

/// Boot from a chunked ISO
///
/// # Arguments
/// * `boot_services` - UEFI boot services
/// * `system_table` - UEFI system table
/// * `image_handle` - Current image handle
/// * `ctx` - ISO read context from storage manager
/// * `block_io` - Underlying disk block I/O
/// * `cmdline` - Kernel command line
/// * `screen` - Screen for progress display
///
/// # Safety
/// This function never returns on success - it transfers control to the kernel.
pub unsafe fn boot_from_iso<B: BlockIo>(
    boot_services: &BootServices,
    system_table: *mut (),
    image_handle: *mut (),
    ctx: IsoReadContext,
    block_io: &mut B,
    cmdline: &str,
    screen: &mut Screen,
) -> Result<core::convert::Infallible, IsoBootError> {
    use crate::tui::renderer::{EFI_BLACK, EFI_LIGHTGREEN, EFI_YELLOW};

    let mut log_y = 5;

    // Create adapter
    screen.put_str_at(5, log_y, "Mounting ISO filesystem...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;

    let mut adapter = IsoBlockIoAdapter::new(ctx, block_io);

    // Mount ISO
    let volume = iso9660::mount(&mut adapter, 0).map_err(|_| IsoBootError::MountFailed)?;

    screen.put_str_at(5, log_y, "Finding boot image...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;

    // Try to find El Torito boot image first
    let boot_image = iso9660::find_boot_image(&mut adapter, &volume);

    // Determine kernel and initrd paths
    let (kernel_path, initrd_path) = if boot_image.is_ok() {
        // Standard live ISO layout
        // Try common paths
        let kernel_paths = [
            "/casper/vmlinuz",           // Ubuntu
            "/live/vmlinuz",             // Debian/Tails
            "/isolinux/vmlinuz",         // Generic
            "/boot/vmlinuz",             // Alpine
            "/boot/x86_64/loader/linux", // openSUSE
        ];
        
        let initrd_paths = [
            "/casper/initrd",
            "/live/initrd.img",
            "/isolinux/initrd.img",
            "/boot/initramfs",
            "/boot/x86_64/loader/initrd",
        ];

        let mut found_kernel = None;
        let mut found_initrd = None;

        for path in kernel_paths.iter() {
            if iso9660::find_file(&mut adapter, &volume, path).is_ok() {
                found_kernel = Some(*path);
                break;
            }
        }

        for path in initrd_paths.iter() {
            if iso9660::find_file(&mut adapter, &volume, path).is_ok() {
                found_initrd = Some(*path);
                break;
            }
        }

        (found_kernel, found_initrd)
    } else {
        // Fallback to standard paths
        (Some("/boot/vmlinuz"), Some("/boot/initrd.img"))
    };

    let kernel_path = kernel_path.ok_or(IsoBootError::KernelNotFound)?;

    screen.put_str_at(5, log_y, "Loading kernel from ISO...", EFI_LIGHTGREEN, EFI_BLACK);
    screen.put_str_at(7, log_y + 1, kernel_path, EFI_YELLOW, EFI_BLACK);
    log_y += 2;

    // Read kernel
    let kernel_file = iso9660::find_file(&mut adapter, &volume, kernel_path)
        .map_err(|_| IsoBootError::KernelNotFound)?;
    let kernel_data = iso9660::read_file_vec(&mut adapter, &kernel_file)
        .map_err(|_| IsoBootError::KernelReadFailed)?;

    // Read initrd if available
    let initrd_data: Option<Vec<u8>> = if let Some(path) = initrd_path {
        screen.put_str_at(5, log_y, "Loading initrd from ISO...", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(7, log_y + 1, path, EFI_YELLOW, EFI_BLACK);
        log_y += 2;

        match iso9660::find_file(&mut adapter, &volume, path) {
            Ok(file) => {
                match iso9660::read_file_vec(&mut adapter, &file) {
                    Ok(data) => Some(data),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };

    screen.put_str_at(5, log_y, "Booting kernel...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;

    // Boot the kernel
    let result = boot_linux_kernel(
        boot_services,
        system_table,
        image_handle,
        &kernel_data,
        initrd_data.as_deref(),
        cmdline,
        screen,
    );

    result.map_err(IsoBootError::BootFailed)
}

/// Get default command line for a distro
///
/// Returns appropriate boot parameters based on ISO name.
pub fn default_cmdline_for_iso(iso_name: &str) -> &'static str {
    let name_lower = iso_name.as_bytes();
    
    // Check for common distros
    if contains_ignore_case(name_lower, b"tails") {
        return "boot=live noautologin";
    }
    if contains_ignore_case(name_lower, b"ubuntu") {
        return "boot=casper quiet splash";
    }
    if contains_ignore_case(name_lower, b"debian") {
        return "boot=live quiet";
    }
    if contains_ignore_case(name_lower, b"kali") {
        return "boot=live noconfig=sudo username=kali";
    }
    if contains_ignore_case(name_lower, b"fedora") {
        return "rd.live.image quiet";
    }
    if contains_ignore_case(name_lower, b"arch") {
        return "archisolabel=ARCH";
    }
    if contains_ignore_case(name_lower, b"alpine") {
        return "modules=loop,squashfs quiet";
    }

    // Generic fallback
    "boot=live quiet"
}

/// Case-insensitive substring check (no_std friendly)
fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }

    for i in 0..=(haystack.len() - needle.len()) {
        let mut matches = true;
        for j in 0..needle.len() {
            let h = haystack[i + j].to_ascii_lowercase();
            let n = needle[j].to_ascii_lowercase();
            if h != n {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
    }
    false
}
