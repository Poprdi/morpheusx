//! Path Table parsing (optional fast lookup)
//!
//! Path tables provide quick directory hierarchy traversal.

/// Path Table Record
#[repr(C, packed)]
pub struct PathTableRecord {
    /// Directory identifier length
    pub dir_id_len: u8,

    /// Extended attribute record length
    pub extended_attr_len: u8,

    /// Extent location (32-bit)
    pub extent_lba: u32,

    /// Parent directory number
    pub parent_dir_num: u16,
    // Followed by directory identifier (dir_id_len bytes)
    // Followed by padding byte if dir_id_len is odd
}

/// Path table type
#[derive(Debug, Clone, Copy)]
pub enum PathTableType {
    /// Little-endian (Type L)
    LittleEndian,
    /// Big-endian (Type M)
    BigEndian,
}
