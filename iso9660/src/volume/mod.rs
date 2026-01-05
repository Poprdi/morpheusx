//! Volume descriptor parsing
//!
//! ISO9660 volume descriptors start at sector 16 and describe the filesystem layout.
//! Multiple descriptors may be present (Primary, Supplementary, Boot Record).

pub mod primary;
pub mod supplementary;
pub mod boot_record;

use crate::error::{Iso9660Error, Result};
use crate::types::{VolumeInfo, SECTOR_SIZE, VOLUME_DESCRIPTOR_START};
use crate::directory::record::DirectoryRecord;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Mount an ISO9660 volume from a block device
///
/// Reads volume descriptors starting at sector 16 and builds VolumeInfo.
///
/// # Arguments
/// * `block_io` - Block device containing the ISO
/// * `start_sector` - Starting sector of the ISO (0 if raw ISO)
///
/// # Returns
/// Parsed volume information
pub fn mount<B: BlockIo>(block_io: &mut B, start_sector: u64) -> Result<VolumeInfo> {
    let mut buffer = [0u8; SECTOR_SIZE];
    let mut boot_catalog_lba: Option<u32> = None;
    
    // Volume info that we'll build from Primary VD
    let mut volume_info: Option<VolumeInfo> = None;
    
    // Read volume descriptors starting at sector 16
    let mut sector = VOLUME_DESCRIPTOR_START;
    loop {
        // Read current volume descriptor sector
        let lba = Lba(start_sector + sector);
        block_io.read_blocks(lba, &mut buffer)
            .map_err(|_| Iso9660Error::IoError)?;
        
        // Parse header (first 7 bytes)
        let header = unsafe { &*(buffer.as_ptr() as *const VolumeDescriptorHeader) };
        
        // Validate header
        if !header.validate() {
            return Err(Iso9660Error::InvalidSignature);
        }
        
        // Process based on type
        match header.type_code {
            0 => {
                // Boot Record (El Torito)
                let boot_record = unsafe { 
                    &*(buffer.as_ptr() as *const boot_record::BootRecordVolumeDescriptor) 
                };
                if boot_record.validate() {
                    boot_catalog_lba = Some(boot_record.catalog_lba());
                }
            }
            1 => {
                // Primary Volume Descriptor
                let pvd = primary::parse(&buffer)?;
                
                // Extract root directory record (embedded at offset 156)
                let root_record = DirectoryRecord::parse(&pvd.root_directory_record)?;
                
                // Build VolumeInfo
                volume_info = Some(VolumeInfo {
                    volume_id: pvd.volume_id,
                    root_extent_lba: root_record.get_extent_lba(),
                    root_extent_len: root_record.get_data_length(),
                    logical_block_size: pvd.logical_block_size.get(),
                    volume_space_size: pvd.volume_space_size.get(),
                    boot_catalog_lba,
                    has_joliet: false, // Will be set if we find supplementary VD
                    has_rock_ridge: false, // TODO: Detect from root directory system use
                });
            }
            2 => {
                // Supplementary Volume Descriptor (Joliet)
                // Update volume info if already created
                if let Some(ref mut vi) = volume_info {
                    vi.has_joliet = true;
                }
            }
            255 => {
                // Terminator - we're done
                break;
            }
            _ => {
                // Unknown descriptor type - skip
            }
        }
        
        sector += 1;
        
        // Safety limit: stop after 100 descriptors
        if sector - VOLUME_DESCRIPTOR_START > 100 {
            break;
        }
    }
    
    // Return volume info or error if not found
    volume_info.ok_or(Iso9660Error::InvalidSignature)
}

/// Volume Descriptor header (first 7 bytes of each descriptor)
#[repr(C, packed)]
pub struct VolumeDescriptorHeader {
    /// Type code (0=boot, 1=primary, 2=supplementary, 255=terminator)
    pub type_code: u8,
    
    /// Standard identifier "CD001"
    pub identifier: [u8; 5],
    
    /// Version (always 1)
    pub version: u8,
}

impl VolumeDescriptorHeader {
    /// CD001 magic bytes
    pub const MAGIC: &'static [u8; 5] = b"CD001";
    
    /// Check if header is valid
    pub fn validate(&self) -> bool {
        &self.identifier == Self::MAGIC && self.version == 1
    }
}
