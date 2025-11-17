// FAT32 filesystem context and FAT operations

use super::super::Fat32Error;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

const SECTOR_SIZE: usize = 512;

/// FAT32 filesystem context
pub struct Fat32Context {
    pub sectors_per_cluster: u32,
    pub reserved_sectors: u32,
    pub fat_size: u32,
    pub num_fats: u32,
    pub root_cluster: u32,
    pub data_start_sector: u32,
}

impl Fat32Context {
    pub fn from_boot_sector<B: BlockIo>(
        block_io: &mut B,
        partition_start: u64,
    ) -> Result<Self, Fat32Error> {
        let mut boot_sector = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(partition_start), &mut boot_sector)
            .map_err(|_| Fat32Error::IoError)?;

        // Parse boot sector
        let sectors_per_cluster = boot_sector[0x0D] as u32;
        let reserved_sectors = u16::from_le_bytes([boot_sector[0x0E], boot_sector[0x0F]]) as u32;
        let num_fats = boot_sector[0x10] as u32;
        let fat_size = u32::from_le_bytes([
            boot_sector[0x24],
            boot_sector[0x25],
            boot_sector[0x26],
            boot_sector[0x27],
        ]);
        let root_cluster = u32::from_le_bytes([
            boot_sector[0x2C],
            boot_sector[0x2D],
            boot_sector[0x2E],
            boot_sector[0x2F],
        ]);

        let data_start_sector = reserved_sectors + (num_fats * fat_size);

        Ok(Self {
            sectors_per_cluster,
            reserved_sectors,
            fat_size,
            num_fats,
            root_cluster,
            data_start_sector,
        })
    }

    pub fn cluster_to_sector(&self, cluster: u32) -> u32 {
        self.data_start_sector + ((cluster - 2) * self.sectors_per_cluster)
    }

    pub fn read_fat_entry<B: BlockIo>(
        &self,
        block_io: &mut B,
        partition_start: u64,
        cluster: u32,
    ) -> Result<u32, Fat32Error> {
        let fat_offset = cluster * 4;
        let fat_sector = self.reserved_sectors + (fat_offset / SECTOR_SIZE as u32);
        let entry_offset = (fat_offset % SECTOR_SIZE as u32) as usize;

        let mut sector = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(partition_start + fat_sector as u64), &mut sector)
            .map_err(|_| Fat32Error::IoError)?;

        let entry = u32::from_le_bytes([
            sector[entry_offset],
            sector[entry_offset + 1],
            sector[entry_offset + 2],
            sector[entry_offset + 3],
        ]) & 0x0FFFFFFF; // FAT32 uses only 28 bits

        Ok(entry)
    }

    pub fn write_fat_entry<B: BlockIo>(
        &self,
        block_io: &mut B,
        partition_start: u64,
        cluster: u32,
        value: u32,
    ) -> Result<(), Fat32Error> {
        let fat_offset = cluster * 4;
        let fat_sector = self.reserved_sectors + (fat_offset / SECTOR_SIZE as u32);
        let entry_offset = (fat_offset % SECTOR_SIZE as u32) as usize;

        for fat_num in 0..self.num_fats {
            let sector_lba = partition_start
                + (self.reserved_sectors
                    + fat_num * self.fat_size
                    + (fat_offset / SECTOR_SIZE as u32)) as u64;

            let mut sector = [0u8; SECTOR_SIZE];
            block_io
                .read_blocks(Lba(sector_lba), &mut sector)
                .map_err(|_| Fat32Error::IoError)?;

            let masked_value = value & 0x0FFFFFFF;
            sector[entry_offset..entry_offset + 4].copy_from_slice(&masked_value.to_le_bytes());

            block_io
                .write_blocks(Lba(sector_lba), &sector)
                .map_err(|_| Fat32Error::IoError)?;
        }

        Ok(())
    }

    pub fn find_free_cluster<B: BlockIo>(
        &self,
        block_io: &mut B,
        partition_start: u64,
        start_from: u32,
    ) -> Result<u32, Fat32Error> {
        // Simple linear search for free cluster
        for cluster in start_from..0x0FFFFFF7 {
            let entry = self.read_fat_entry(block_io, partition_start, cluster)?;
            if entry == 0 {
                return Ok(cluster);
            }
        }
        Err(Fat32Error::IoError) // No free clusters
    }

    pub fn allocate_cluster<B: BlockIo>(
        &self,
        block_io: &mut B,
        partition_start: u64,
    ) -> Result<u32, Fat32Error> {
        let cluster = self.find_free_cluster(block_io, partition_start, 2)?;
        self.write_fat_entry(block_io, partition_start, cluster, 0x0FFFFFF8)?; // EOC marker
        Ok(cluster)
    }
}
