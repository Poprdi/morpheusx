//! Primary Volume Descriptor (ECMA-119 §8.4).

use crate::error::Result;

/// PVD layout up through the embedded root directory record.
/// Trailing metadata fields (publisher, copyright, dates, etc.) are not modeled.
#[repr(C, packed)]
pub struct PrimaryVolumeDescriptor {
    /// Type code; 1.
    pub type_code: u8,
    /// "CD001".
    pub identifier: [u8; 5],
    /// VD version; 1.
    pub version: u8,
    /// Reserved.
    pub unused1: u8,
    /// System identifier (a-characters, space-padded).
    pub system_id: [u8; 32],
    /// Volume identifier (d-characters, space-padded).
    pub volume_id: [u8; 32],
    /// Reserved.
    pub unused2: [u8; 8],
    /// Total sectors in the volume.
    pub volume_space_size: BothEndian32,
    /// Reserved.
    pub unused3: [u8; 32],
    /// Volume set size.
    pub volume_set_size: BothEndian16,
    /// Volume sequence number.
    pub volume_sequence_number: BothEndian16,
    /// Logical block size (almost always 2048).
    pub logical_block_size: BothEndian16,
    /// Path table size in bytes.
    pub path_table_size: BothEndian32,
    /// L-type path table LBA (LE only here).
    pub type_l_path_table: u32,
    /// Optional L-type path table LBA.
    pub optional_type_l_path_table: u32,
    /// M-type path table LBA (BE only here).
    pub type_m_path_table: u32,
    /// Optional M-type path table LBA.
    pub optional_type_m_path_table: u32,
    /// Embedded directory record for the root.
    pub root_directory_record: [u8; 34],
}

/// Both-endian 32-bit field: same value stored LE then BE (ISO 9660 §7.3.3).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BothEndian32 {
    /// LE half.
    pub le: [u8; 4],
    /// BE half.
    pub be: [u8; 4],
}

impl BothEndian32 {
    /// Read the LE half. The BE half is redundant and unchecked.
    pub fn get(&self) -> u32 {
        u32::from_le_bytes(self.le)
    }
}

/// Both-endian 16-bit field (ISO 9660 §7.2.3).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BothEndian16 {
    /// LE half.
    pub le: [u8; 2],
    /// BE half.
    pub be: [u8; 2],
}

impl BothEndian16 {
    /// Read the LE half.
    pub fn get(&self) -> u16 {
        u16::from_le_bytes(self.le)
    }
}

/// Reinterpret a sector as a PVD after identifier/version checks.
pub fn parse(data: &[u8]) -> Result<&PrimaryVolumeDescriptor> {
    use crate::error::Iso9660Error;

    if data.len() < core::mem::size_of::<PrimaryVolumeDescriptor>() {
        return Err(Iso9660Error::InvalidSignature);
    }

    // SAFETY: size checked above; struct is repr(C, packed).
    let pvd = unsafe { &*(data.as_ptr() as *const PrimaryVolumeDescriptor) };

    if pvd.type_code != 1 {
        return Err(Iso9660Error::InvalidSignature);
    }

    if &pvd.identifier != b"CD001" {
        return Err(Iso9660Error::InvalidSignature);
    }

    if pvd.version != 1 {
        return Err(Iso9660Error::UnsupportedVersion);
    }

    Ok(pvd)
}
