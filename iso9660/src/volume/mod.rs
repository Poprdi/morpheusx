//! Volume descriptor scan starting at sector 16.

pub mod boot_record;
pub mod primary;
pub mod supplementary;

use crate::directory::record::DirectoryRecord;
use crate::error::{Iso9660Error, Result};
use crate::types::{VolumeInfo, SECTOR_SIZE, VOLUME_DESCRIPTOR_START};
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Read VDs from `start_sector + 16` until the set terminator and return
/// `VolumeInfo` derived from the PVD. Boot Record and Supplementary VDs
/// observed along the way are folded in.
pub fn mount<B: BlockIo>(block_io: &mut B, start_sector: u64) -> Result<VolumeInfo> {
    let mut buffer = [0u8; SECTOR_SIZE];
    let mut boot_catalog_lba: Option<u32> = None;
    let mut volume_info: Option<VolumeInfo> = None;

    let mut sector = VOLUME_DESCRIPTOR_START;
    loop {
        let lba = Lba(start_sector + sector);

        block_io
            .read_blocks(lba, &mut buffer)
            .map_err(|_| Iso9660Error::IoError)?;

        // SAFETY: SECTOR_SIZE >> sizeof(VolumeDescriptorHeader).
        let header = unsafe { &*(buffer.as_ptr() as *const VolumeDescriptorHeader) };

        if !header.validate() {
            return Err(Iso9660Error::InvalidSignature);
        }

        match header.type_code {
            0 => {
                // SAFETY: BootRecordVolumeDescriptor fits in SECTOR_SIZE.
                let boot_record = unsafe {
                    &*(buffer.as_ptr() as *const boot_record::BootRecordVolumeDescriptor)
                };
                if boot_record.validate() {
                    let catalog_lba = boot_record.catalog_lba();
                    boot_catalog_lba = Some(catalog_lba);
                    if let Some(ref mut vi) = volume_info {
                        vi.boot_catalog_lba = Some(catalog_lba);
                    }
                }
            },
            1 => {
                let pvd = primary::parse(&buffer)?;
                // Root record is the 34-byte field embedded at PVD offset 156.
                let root_record = DirectoryRecord::parse(&pvd.root_directory_record)?;

                volume_info = Some(VolumeInfo {
                    volume_id: pvd.volume_id,
                    root_extent_lba: root_record.get_extent_lba(),
                    root_extent_len: root_record.get_data_length(),
                    logical_block_size: pvd.logical_block_size.get(),
                    volume_space_size: pvd.volume_space_size.get(),
                    boot_catalog_lba,
                    has_joliet: false,
                    // TODO: detect via root directory SUSP entries.
                    has_rock_ridge: false,
                });
            },
            2 => {
                if let Some(ref mut vi) = volume_info {
                    vi.has_joliet = true;
                }
            },
            255 => break,
            _ => {},
        }

        sector += 1;

        // Defensive bound: spec doesn't cap descriptor count but the set
        // terminator should appear well before this.
        if sector - VOLUME_DESCRIPTOR_START > 100 {
            break;
        }
    }

    volume_info.ok_or(Iso9660Error::InvalidSignature)
}

/// 7-byte VD prefix shared by every descriptor type.
#[repr(C, packed)]
pub struct VolumeDescriptorHeader {
    /// 0=boot, 1=primary, 2=supplementary, 255=terminator.
    pub type_code: u8,
    /// "CD001".
    pub identifier: [u8; 5],
    /// VD version (1, or 2 for some Joliet SVDs).
    pub version: u8,
}

impl VolumeDescriptorHeader {
    /// Required identifier bytes.
    pub const MAGIC: &'static [u8; 5] = b"CD001";

    /// Identifier matches and version is 1 or 2.
    pub fn validate(&self) -> bool {
        &self.identifier == Self::MAGIC && (self.version == 1 || self.version == 2)
    }
}
