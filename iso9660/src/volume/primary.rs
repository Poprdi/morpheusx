//! Primary Volume Descriptor parsing
//!
//! The Primary Volume Descriptor (PVD) is always present and describes
//! the basic ISO9660 filesystem structure.

use crate::error::Result;

/// Primary Volume Descriptor (sector 16, type 1)
///
/// See ECMA-119 8.4 for full specification
#[repr(C, packed)]
pub struct PrimaryVolumeDescriptor {
    // Header (7 bytes)
    /// Type code (1 for primary)
    pub type_code: u8,
    /// Standard identifier "CD001"
    pub identifier: [u8; 5],
    /// Version (1)
    pub version: u8,
    
    // Body (2041 bytes)
    /// Unused (1 byte)
    pub unused1: u8,
    
    /// System identifier (32 a-characters)
    pub system_id: [u8; 32],
    
    /// Volume identifier (32 d-characters)
    pub volume_id: [u8; 32],
    
    /// Unused (8 bytes)
    pub unused2: [u8; 8],
    
    /// Volume space size (both-endian 32-bit)
    pub volume_space_size: BothEndian32,
    
    /// Unused (32 bytes)
    pub unused3: [u8; 32],
    
    /// Volume set size (both-endian 16-bit)
    pub volume_set_size: BothEndian16,
    
    /// Volume sequence number (both-endian 16-bit)
    pub volume_sequence_number: BothEndian16,
    
    /// Logical block size (both-endian 16-bit, usually 2048)
    pub logical_block_size: BothEndian16,
    
    /// Path table size (both-endian 32-bit)
    pub path_table_size: BothEndian32,
    
    /// Type L path table location (32-bit LE)
    pub type_l_path_table: u32,
    
    /// Optional type L path table location (32-bit LE)
    pub optional_type_l_path_table: u32,
    
    /// Type M path table location (32-bit BE)
    pub type_m_path_table: u32,
    
    /// Optional type M path table location (32-bit BE)
    pub optional_type_m_path_table: u32,
    
    /// Root directory record (34 bytes)
    pub root_directory_record: [u8; 34],
    
    // Remainder: various metadata fields
    // Total size: 2048 bytes
}

/// Both-endian 32-bit value (stored as LE then BE)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BothEndian32 {
    /// Little-endian value
    pub le: [u8; 4],
    /// Big-endian value
    pub be: [u8; 4],
}

impl BothEndian32 {
    /// Get value (uses little-endian)
    pub fn get(&self) -> u32 {
        u32::from_le_bytes(self.le)
    }
}

/// Both-endian 16-bit value (stored as LE then BE)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BothEndian16 {
    /// Little-endian value
    pub le: [u8; 2],
    /// Big-endian value
    pub be: [u8; 2],
}

impl BothEndian16 {
    /// Get value (uses little-endian)
    pub fn get(&self) -> u16 {
        u16::from_le_bytes(self.le)
    }
}

/// Parse Primary Volume Descriptor from sector data
pub fn parse(data: &[u8]) -> Result<&PrimaryVolumeDescriptor> {
    use crate::error::Iso9660Error;
    
    // Validate minimum length
    if data.len() < core::mem::size_of::<PrimaryVolumeDescriptor>() {
        return Err(Iso9660Error::InvalidSignature);
    }
    
    // Cast to struct (safe because we checked size)
    let pvd = unsafe { &*(data.as_ptr() as *const PrimaryVolumeDescriptor) };
    
    // Validate header
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
