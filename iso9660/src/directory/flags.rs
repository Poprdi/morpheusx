//! File flags parsing and manipulation

use crate::types::FileFlags;

impl FileFlags {
    /// Parse from raw byte
    pub fn from_byte(byte: u8) -> Self {
        Self {
            hidden: byte & 0x01 != 0,
            directory: byte & 0x02 != 0,
            associated: byte & 0x04 != 0,
            extended_format: byte & 0x08 != 0,
            extended_permissions: byte & 0x10 != 0,
            not_final: byte & 0x80 != 0,
        }
    }
    
    /// Convert to raw byte
    pub fn to_byte(&self) -> u8 {
        let mut byte = 0u8;
        if self.hidden { byte |= 0x01; }
        if self.directory { byte |= 0x02; }
        if self.associated { byte |= 0x04; }
        if self.extended_format { byte |= 0x08; }
        if self.extended_permissions { byte |= 0x10; }
        if self.not_final { byte |= 0x80; }
        byte
    }
}

impl Default for FileFlags {
    fn default() -> Self {
        Self {
            hidden: false,
            directory: false,
            associated: false,
            extended_format: false,
            extended_permissions: false,
            not_final: false,
        }
    }
}
