//! `FileEntry` convenience accessors.

use crate::types::FileEntry;

impl FileEntry {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Extension after the final '.', if any.
    pub fn extension(&self) -> Option<&str> {
        self.name.rsplit('.').nth(0)
    }

    /// True for regular files.
    pub fn is_file(&self) -> bool {
        !self.flags.directory
    }

    /// True for directories.
    pub fn is_directory(&self) -> bool {
        self.flags.directory
    }

    pub fn is_hidden(&self) -> bool {
        self.flags.hidden
    }
}
