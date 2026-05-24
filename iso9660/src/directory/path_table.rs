//! Path table records (ISO 9660 §6.9). Optional shortcut to directory extents.

/// Fixed-header portion of a path table record. Followed by `dir_id_len` name
/// bytes plus an odd-length pad byte.
#[repr(C, packed)]
pub struct PathTableRecord {
    /// Length of directory identifier in bytes.
    pub dir_id_len: u8,
    /// Length of any preceding Extended Attribute Record.
    pub extended_attr_len: u8,
    /// Directory extent LBA.
    pub extent_lba: u32,
    /// 1-based index of parent directory in the path table.
    pub parent_dir_num: u16,
}

/// L-type (LE) and M-type (BE) path tables are stored separately on the volume.
#[derive(Debug, Clone, Copy)]
pub enum PathTableType {
    /// L-type, little-endian.
    LittleEndian,
    /// M-type, big-endian.
    BigEndian,
}
