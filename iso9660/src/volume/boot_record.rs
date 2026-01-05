//! Boot Record Volume Descriptor (El Torito)
//!
//! Points to the El Torito boot catalog which describes bootable images.

/// Boot Record Volume Descriptor (type 0)
#[repr(C, packed)]
pub struct BootRecordVolumeDescriptor {
    /// Type code (0 for boot record)
    pub type_code: u8,
    /// Standard identifier "CD001"
    pub identifier: [u8; 5],
    /// Version (1)
    pub version: u8,
    /// Boot system identifier "EL TORITO SPECIFICATION" (32 bytes)
    pub boot_system_id: [u8; 32],
    /// Unused (32 bytes)
    pub unused: [u8; 32],
    /// Absolute LBA of boot catalog (32-bit LE)
    pub boot_catalog_lba: u32,
    // Padding to 2048 bytes
}

impl BootRecordVolumeDescriptor {
    /// El Torito magic string
    pub const EL_TORITO_MAGIC: &'static [u8; 23] = b"EL TORITO SPECIFICATION";
    
    /// Validate boot record
    pub fn validate(&self) -> bool {
        self.boot_system_id.starts_with(Self::EL_TORITO_MAGIC)
    }
    
    /// Get boot catalog LBA
    pub fn catalog_lba(&self) -> u32 {
        self.boot_catalog_lba
    }
}
