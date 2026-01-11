//! File metadata extraction

use crate::types::FileEntry;

impl FileEntry {
    /// Get file name as string
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get file extension
    pub fn extension(&self) -> Option<&str> {
        self.name.rsplit('.').nth(0)
    }

    /// Is this a regular file?
    pub fn is_file(&self) -> bool {
        !self.flags.directory
    }

    /// Is this a directory?
    pub fn is_directory(&self) -> bool {
        self.flags.directory
    }

    /// Is this hidden?
    pub fn is_hidden(&self) -> bool {
        self.flags.hidden
    }
}
