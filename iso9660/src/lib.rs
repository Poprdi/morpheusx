//! `no_std` ISO 9660 reader with El Torito boot catalog and optional
//! Joliet / Rock Ridge support.

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

pub mod boot;
pub mod directory;
pub mod error;
pub mod extensions;
pub mod file;
pub mod types;
pub mod utils;
pub mod volume;

pub use error::{Iso9660Error, Result};
pub use types::{BootImage, BootMediaType, BootPlatform, FileEntry, FileFlags, VolumeInfo};

pub use boot::find_boot_image;
pub use directory::find_file;
pub use directory::iterator::DirectoryIterator;
pub use file::reader::FileReader;
pub use file::{read_file, read_file_vec};
pub use volume::mount;
