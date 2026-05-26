pub mod fat32_format;
pub mod fat32_ops;

/// Disk sector size. Fixed at 512 — FAT32, GPT, and the AHCI/VirtIO paths
/// all assume this. Drives reporting 4Kn are not supported here.
pub const SECTOR_SIZE: usize = 512;

pub use fat32_format::{format_fat32, verify_fat32, Fat32Error};
pub use fat32_ops::filename::generate_8_3_manifest_name;
pub use fat32_ops::{create_directory, file_exists, read_file, write_file};
