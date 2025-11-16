// Filesystem operations

pub mod fat32_format;
pub mod fat32_ops;

pub use fat32_format::{format_fat32, verify_fat32, Fat32Error};
pub use fat32_ops::{create_directory, file_exists, read_file, write_file};
