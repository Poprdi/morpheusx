//! Stack-buffered FAT32 formatter; minimal FS for ISO chunk storage.

use gpt_disk_io::BlockIo;
use gpt_disk_types::{Lba, LbaLe};

use super::types::{DiskError, DiskResult, SECTOR_SIZE};

pub struct Fat32Formatter;

impl Fat32Formatter {
    /// Layout: BS@0, FSInfo@1, backup BS@6, two FATs from LBA 32, root dir at data start.
    pub fn format<B: BlockIo>(
        block_io: &mut B,
        partition_start_lba: u64,
        partition_sectors: u64,
        volume_label: &str,
    ) -> DiskResult<Fat32Info> {
        // FAT32 min ~65 MB.
        if partition_sectors < 133120 {
            return Err(DiskError::InvalidSize);
        }

        let total_sectors = partition_sectors as u32;
        let reserved_sectors = 32u16;
        let sectors_per_cluster = Self::optimal_cluster_size(total_sectors);

        let fat_size =
            Self::calculate_fat_size(total_sectors, reserved_sectors, sectors_per_cluster);

        let fat_sectors = fat_size * 2;
        let data_sectors = total_sectors - reserved_sectors as u32 - fat_sectors;
        let cluster_count = data_sectors / sectors_per_cluster as u32;

        let boot_sector = Self::build_boot_sector(
            total_sectors,
            fat_size,
            sectors_per_cluster,
            partition_start_lba as u32,
            volume_label,
        );

        let lba = Lba(partition_start_lba);
        block_io
            .write_blocks(lba, &boot_sector)
            .map_err(|_| DiskError::IoError)?;

        let fsinfo = Self::build_fsinfo(cluster_count - 1);
        let fsinfo_lba = Lba(partition_start_lba + 1);
        block_io
            .write_blocks(fsinfo_lba, &fsinfo)
            .map_err(|_| DiskError::IoError)?;

        let backup_lba = Lba(partition_start_lba + 6);
        block_io
            .write_blocks(backup_lba, &boot_sector)
            .map_err(|_| DiskError::IoError)?;

        // First FAT sector: entry 0 = media type 0x0FFFFFF8, entry 1 = EOC,
        // entry 2 = root cluster EOC.
        let mut fat_sector = [0u8; SECTOR_SIZE];
        fat_sector[0] = 0xF8;
        fat_sector[1] = 0xFF;
        fat_sector[2] = 0xFF;
        fat_sector[3] = 0x0F;
        fat_sector[4] = 0xFF;
        fat_sector[5] = 0xFF;
        fat_sector[6] = 0xFF;
        fat_sector[7] = 0xFF;
        fat_sector[8] = 0xFF;
        fat_sector[9] = 0xFF;
        fat_sector[10] = 0xFF;
        fat_sector[11] = 0x0F;

        let fat1_lba = Lba(partition_start_lba + reserved_sectors as u64);
        block_io
            .write_blocks(fat1_lba, &fat_sector)
            .map_err(|_| DiskError::IoError)?;

        let fat2_lba = Lba(partition_start_lba + reserved_sectors as u64 + fat_size as u64);
        block_io
            .write_blocks(fat2_lba, &fat_sector)
            .map_err(|_| DiskError::IoError)?;

        // Zero root directory cluster.
        let data_start = partition_start_lba + reserved_sectors as u64 + (fat_size * 2) as u64;
        let empty_sector = [0u8; SECTOR_SIZE];
        for i in 0..sectors_per_cluster as u64 {
            block_io
                .write_blocks(Lba(data_start + i), &empty_sector)
                .map_err(|_| DiskError::IoError)?;
        }

        block_io.flush().map_err(|_| DiskError::IoError)?;

        Ok(Fat32Info {
            reserved_sectors,
            sectors_per_cluster,
            fat_size,
            data_start_lba: data_start,
            cluster_count,
        })
    }

    /// Keeps cluster count under FAT32's ~268M cap.
    fn optimal_cluster_size(total_sectors: u32) -> u8 {
        let size_mb = total_sectors / 2048;

        match size_mb {
            0..=512 => 1,
            513..=8192 => 8,
            8193..=16384 => 16,
            16385..=32768 => 32,
            _ => 64,
        }
    }

    /// Microsoft FAT32 FAT-size formula.
    fn calculate_fat_size(total_sectors: u32, reserved: u16, spc: u8) -> u32 {
        let tmp1 = total_sectors - reserved as u32;
        let tmp2 = (256 * spc as u32) + 2;
        tmp1.div_ceil(tmp2)
    }

    fn build_boot_sector(
        total_sectors: u32,
        fat_size: u32,
        spc: u8,
        hidden_sectors: u32,
        label: &str,
    ) -> [u8; SECTOR_SIZE] {
        let mut bs = [0u8; SECTOR_SIZE];

        // jmp + nop.
        bs[0] = 0xEB;
        bs[1] = 0x58;
        bs[2] = 0x90;

        bs[3..11].copy_from_slice(b"MORPHEUS");

        // BPB.
        bs[11..13].copy_from_slice(&512u16.to_le_bytes());
        bs[13] = spc;
        bs[14..16].copy_from_slice(&32u16.to_le_bytes());
        bs[16] = 2;
        bs[17..19].copy_from_slice(&0u16.to_le_bytes()); // root entries — FAT32: 0
        bs[19..21].copy_from_slice(&0u16.to_le_bytes()); // total_sectors_16 — FAT32: 0
        bs[21] = 0xF8;
        bs[22..24].copy_from_slice(&0u16.to_le_bytes()); // fat_size_16 — FAT32: 0
        bs[24..26].copy_from_slice(&63u16.to_le_bytes());
        bs[26..28].copy_from_slice(&255u16.to_le_bytes());
        bs[28..32].copy_from_slice(&hidden_sectors.to_le_bytes());
        bs[32..36].copy_from_slice(&total_sectors.to_le_bytes());

        // FAT32 EBPB.
        bs[36..40].copy_from_slice(&fat_size.to_le_bytes());
        bs[40..42].copy_from_slice(&0u16.to_le_bytes());
        bs[42..44].copy_from_slice(&0u16.to_le_bytes());
        bs[44..48].copy_from_slice(&2u32.to_le_bytes());
        bs[48..50].copy_from_slice(&1u16.to_le_bytes());
        bs[50..52].copy_from_slice(&6u16.to_le_bytes());

        bs[64] = 0x80;
        bs[65] = 0;
        bs[66] = 0x29;
        bs[67..71].copy_from_slice(&0x12345678u32.to_le_bytes());

        let label_bytes = label.as_bytes();
        let mut label_buf = [b' '; 11];
        let copy_len = label_bytes.len().min(11);
        label_buf[..copy_len].copy_from_slice(&label_bytes[..copy_len]);
        bs[71..82].copy_from_slice(&label_buf);

        bs[82..90].copy_from_slice(b"FAT32   ");

        bs[510] = 0x55;
        bs[511] = 0xAA;

        bs
    }

    fn build_fsinfo(free_clusters: u32) -> [u8; SECTOR_SIZE] {
        let mut fs = [0u8; SECTOR_SIZE];

        fs[0..4].copy_from_slice(&0x41615252u32.to_le_bytes());
        fs[484..488].copy_from_slice(&0x61417272u32.to_le_bytes());
        fs[488..492].copy_from_slice(&free_clusters.to_le_bytes());
        fs[492..496].copy_from_slice(&3u32.to_le_bytes());
        fs[508..512].copy_from_slice(&0xAA550000u32.to_le_bytes());

        fs
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Fat32Info {
    pub reserved_sectors: u16,
    pub sectors_per_cluster: u8,
    pub fat_size: u32,
    pub data_start_lba: u64,
    pub cluster_count: u32,
}
