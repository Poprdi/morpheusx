//! Rock Ridge extension support
//!
//! Rock Ridge adds POSIX filesystem semantics (permissions, symlinks, long names).

/// System Use Entry header
#[repr(C, packed)]
pub struct SystemUseEntry {
    /// Signature (2 bytes, e.g. "PX", "PN", "SL")
    pub signature: [u8; 2],

    /// Length of entry
    pub length: u8,

    /// Version
    pub version: u8,
    // Followed by entry-specific data
}

/// POSIX file attributes (PX entry)
#[repr(C, packed)]
pub struct PosixAttributes {
    pub header: SystemUseEntry,

    /// File mode (both-endian)
    pub mode: [u8; 8],

    /// Number of links (both-endian)
    pub links: [u8; 8],

    /// User ID (both-endian)
    pub uid: [u8; 8],

    /// Group ID (both-endian)
    pub gid: [u8; 8],
}

/// Alternate name (NM entry)
pub struct AlternateName {
    /// Flags
    pub flags: u8,

    /// Name content
    pub name: alloc::string::String,
}

/// Signature constants
pub mod signatures {
    pub const POSIX_ATTRS: &[u8; 2] = b"PX";
    pub const POSIX_DEV: &[u8; 2] = b"PN";
    pub const SYMLINK: &[u8; 2] = b"SL";
    pub const ALTERNATE_NAME: &[u8; 2] = b"NM";
    pub const CHILD_LINK: &[u8; 2] = b"CL";
    pub const PARENT_LINK: &[u8; 2] = b"PL";
    pub const RELOCATED_DIR: &[u8; 2] = b"RE";
    pub const TIMESTAMPS: &[u8; 2] = b"TF";
}
