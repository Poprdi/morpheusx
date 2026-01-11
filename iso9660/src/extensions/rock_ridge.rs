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
    /// System Use Entry header
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
    /// POSIX file attributes signature
    pub const POSIX_ATTRS: &[u8; 2] = b"PX";
    /// POSIX device number signature
    pub const POSIX_DEV: &[u8; 2] = b"PN";
    /// Symbolic link signature
    pub const SYMLINK: &[u8; 2] = b"SL";
    /// Alternate name signature
    pub const ALTERNATE_NAME: &[u8; 2] = b"NM";
    /// Child link signature
    pub const CHILD_LINK: &[u8; 2] = b"CL";
    /// Parent link signature
    pub const PARENT_LINK: &[u8; 2] = b"PL";
    /// Relocated directory signature
    pub const RELOCATED_DIR: &[u8; 2] = b"RE";
    /// Timestamps signature
    pub const TIMESTAMPS: &[u8; 2] = b"TF";
}
