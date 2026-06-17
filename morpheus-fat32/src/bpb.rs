//! BIOS Parameter Block + FSInfo parsing.
//!
//! Layout per Microsoft FAT spec (fatgen103) §3.1/§3.3. All multi-byte fields
//! are little-endian; offsets are byte offsets into the boot sector.

use crate::error::Fat32Error;

fn rd16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Geometry derived from the boot-sector BPB, in sectors/clusters.
#[derive(Debug, Clone, Copy)]
pub struct Bpb {
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub reserved_sectors: u32,
    pub num_fats: u32,
    pub sectors_per_fat: u32,
    pub root_cluster: u32,
    pub total_sectors: u32,
}

impl Bpb {
    /// Parse the 512-byte boot sector. `buf` must be at least one sector.
    pub fn parse(buf: &[u8]) -> Result<Bpb, Fat32Error> {
        if buf.len() < 512 {
            return Err(Fat32Error::NotFat32);
        }
        // 0x1FE: boot signature 0x55AA.
        if buf[510] != 0x55 || buf[511] != 0xAA {
            return Err(Fat32Error::NotFat32);
        }

        let bytes_per_sector = rd16(buf, 11) as u32;
        let sectors_per_cluster = buf[13] as u32;
        let reserved_sectors = rd16(buf, 14) as u32;
        let num_fats = buf[16] as u32;

        // FAT32 zeroes the 16-bit counts and uses the 32-bit ones instead.
        let root_entry_count = rd16(buf, 17);
        let total_sectors_16 = rd16(buf, 19);
        let sectors_per_fat_16 = rd16(buf, 22);
        let total_sectors_32 = rd32(buf, 32);
        let sectors_per_fat_32 = rd32(buf, 36);
        let root_cluster = rd32(buf, 44);

        // FAT32 is identified by zero 16-bit FAT size and a zero root-entry count.
        if sectors_per_fat_16 != 0 || root_entry_count != 0 {
            return Err(Fat32Error::NotFat32);
        }

        let sectors_per_fat = sectors_per_fat_32;
        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16 as u32
        } else {
            total_sectors_32
        };

        if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096) {
            return Err(Fat32Error::BadGeometry);
        }
        if !sectors_per_cluster.is_power_of_two() || sectors_per_cluster == 0 {
            return Err(Fat32Error::BadGeometry);
        }
        if num_fats == 0
            || sectors_per_fat == 0
            || reserved_sectors == 0
            || total_sectors == 0
            || root_cluster < 2
        {
            return Err(Fat32Error::BadGeometry);
        }

        Ok(Bpb {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            sectors_per_fat,
            root_cluster,
            total_sectors,
        })
    }

    /// First sector of the FAT region (partition-relative).
    pub fn fat_start_sector(&self) -> u32 {
        self.reserved_sectors
    }

    /// First sector of the cluster heap (partition-relative).
    pub fn data_start_sector(&self) -> u32 {
        self.reserved_sectors + self.num_fats * self.sectors_per_fat
    }

    pub fn bytes_per_cluster(&self) -> u32 {
        self.bytes_per_sector * self.sectors_per_cluster
    }

    /// Count of usable data clusters; bounds chain walks against garbage links.
    pub fn cluster_count(&self) -> u32 {
        let data_sectors = self.total_sectors.saturating_sub(self.data_start_sector());
        data_sectors / self.sectors_per_cluster
    }

    /// Partition-relative sector where `cluster` (>= 2) begins.
    pub fn cluster_to_sector(&self, cluster: u32) -> u32 {
        self.data_start_sector() + (cluster - 2) * self.sectors_per_cluster
    }
}
