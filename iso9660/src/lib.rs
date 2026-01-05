//! ISO9660 Filesystem Implementation
//!
//! A `no_std` implementation of the ISO9660 filesystem with El Torito boot support.
//!
//! # Overview
//!
//! ISO9660 is the standard filesystem for CD-ROMs and DVDs. This crate provides:
//! - Volume descriptor parsing (Primary, Supplementary, Boot Record)
//! - Directory tree navigation
//! - File reading from extent-based storage
//! - El Torito bootable CD support for kernel extraction
//! - Optional Rock Ridge (POSIX) and Joliet (Unicode) extensions
//!
//! # Architecture
//!
//! The implementation is layered:
//! 1. **Volume layer** - Parses volume descriptors from sectors 16+
//! 2. **Directory layer** - Navigates directory records and path tables
//! 3. **File layer** - Reads file data from extents
//! 4. **Boot layer** - El Torito boot catalog parsing
//!
//! # Usage
//!
//! ```ignore
//! use iso9660::{mount, find_file, read_file};
//! 
//! // Mount ISO from block device at given start sector
//! let volume = mount(&mut block_io, start_sector)?;
//! 
//! // Find a file by path
//! let file = find_file(&mut block_io, &volume, "/isolinux/vmlinuz")?;
//! 
//! // Read file contents
//! let kernel_data = read_file(&mut block_io, &volume, &file)?;
//! ```
//!
//! # El Torito Boot Support
//!
//! ```ignore
//! use iso9660::find_boot_image;
//! 
//! // Extract bootable image (kernel) from ISO
//! let boot = find_boot_image(&mut block_io, &volume)?;
//! let kernel = read_file(&mut block_io, &volume, &boot.file)?;
//! ```

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

pub mod error;
pub mod types;
pub mod volume;
pub mod directory;
pub mod file;
pub mod boot;
pub mod extensions;
pub mod utils;

pub use error::{Iso9660Error, Result};
pub use types::{VolumeInfo, FileEntry, BootImage};

// High-level API exports
pub use volume::mount;
pub use directory::find_file;
pub use file::read_file;
pub use boot::find_boot_image;
