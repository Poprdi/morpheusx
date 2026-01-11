// Bootloader self-installation module
// Handles installing Morpheus to EFI System Partition

use crate::BootServices;
extern crate alloc;

#[derive(Debug)]
pub enum InstallError {
    NoEsp,            // No EFI System Partition found
    EspTooSmall,      // ESP exists but not enough free space
    IoError,          // Disk I/O error
    ProtocolError,    // Failed to access UEFI protocols
    AlreadyInstalled, // Morpheus already installed
    NoFreeSpc,        // No free space to create ESP
    FormatFailed,     // Failed to format ESP
}

/// Information about located ESP
#[derive(Debug)]
pub struct EspInfo {
    pub disk_index: usize,
    pub partition_index: usize,
    pub start_lba: u64,
    pub size_mb: u64,
}

/// Find EFI System Partition on any disk
mod operations;
pub use operations::*;
