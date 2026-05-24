//! Boot Record VD (type 0). Carries the El Torito catalog LBA.

/// Type-0 volume descriptor pointing at the El Torito boot catalog.
#[repr(C, packed)]
pub struct BootRecordVolumeDescriptor {
    /// Type code; 0.
    pub type_code: u8,
    /// "CD001".
    pub identifier: [u8; 5],
    /// VD version (1).
    pub version: u8,
    /// "EL TORITO SPECIFICATION", space-padded.
    pub boot_system_id: [u8; 32],
    /// Reserved.
    pub unused: [u8; 32],
    /// LE-only catalog LBA (no both-endian here).
    pub boot_catalog_lba: u32,
}

impl BootRecordVolumeDescriptor {
    /// Boot system identifier prefix for El Torito.
    pub const EL_TORITO_MAGIC: &'static [u8; 23] = b"EL TORITO SPECIFICATION";

    /// Whether this VD advertises El Torito.
    pub fn validate(&self) -> bool {
        self.boot_system_id.starts_with(Self::EL_TORITO_MAGIC)
    }

    pub fn catalog_lba(&self) -> u32 {
        self.boot_catalog_lba
    }
}
