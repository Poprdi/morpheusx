//! Rock Ridge SUSP / RRIP entries (POSIX metadata over ISO 9660).

/// SUSP System Use Entry header. Entry-specific bytes follow.
#[repr(C, packed)]
pub struct SystemUseEntry {
    /// Two-character signature (e.g. "PX", "NM").
    pub signature: [u8; 2],
    /// Total entry length including header.
    pub length: u8,
    /// SUSP version.
    pub version: u8,
}

/// PX entry: POSIX mode, link count, uid, gid (each both-endian).
#[repr(C, packed)]
pub struct PosixAttributes {
    /// SUSP header.
    pub header: SystemUseEntry,
    /// POSIX file mode.
    pub mode: [u8; 8],
    /// Hard link count.
    pub links: [u8; 8],
    /// User ID.
    pub uid: [u8; 8],
    /// Group ID.
    pub gid: [u8; 8],
}

/// NM entry: alternate (long / POSIX) name.
pub struct AlternateName {
    /// CONTINUE / CURRENT / PARENT bits.
    pub flags: u8,
    /// Decoded name fragment.
    pub name: alloc::string::String,
}

/// SUSP / RRIP entry signatures.
pub mod signatures {
    /// POSIX file attributes (PX) entry signature.
    pub const POSIX_ATTRS: &[u8; 2] = b"PX";
    /// POSIX device number (PN) entry signature.
    pub const POSIX_DEV: &[u8; 2] = b"PN";
    /// Symbolic link (SL) entry signature.
    pub const SYMLINK: &[u8; 2] = b"SL";
    /// Alternate name (NM) entry signature.
    pub const ALTERNATE_NAME: &[u8; 2] = b"NM";
    /// Child link (CL) entry signature.
    pub const CHILD_LINK: &[u8; 2] = b"CL";
    /// Parent link (PL) entry signature.
    pub const PARENT_LINK: &[u8; 2] = b"PL";
    /// Relocated directory (RE) entry signature.
    pub const RELOCATED_DIR: &[u8; 2] = b"RE";
    /// Timestamps (TF) entry signature.
    pub const TIMESTAMPS: &[u8; 2] = b"TF";
}
