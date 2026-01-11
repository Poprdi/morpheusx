//! FAT32 formatter for post-EBS.
//!
//! Allocation-free FAT32 filesystem formatting using stack buffers.
//! Creates minimal FAT32 filesystem suitable for ISO chunk storage.

use gpt_disk_io::BlockIo;
use gpt_disk_types::{Lba, LbaLe};

use super::types::{DiskError, DiskResult, SECTOR_SIZE};

/// FAT32 filesystem formatter
pub struct Fat32Formatter;

impl Fat32Formatter {
    /// Format a partition as FAT32
    ///
    /// Creates minimal FAT32 with:
    /// - Boot sector at LBA 0
    /// - FSInfo at LBA 1  
    /// - Backup boot sector at LBA 6
    /// - Two FAT tables starting at LBA 32
    /// - Root directory at data start
    pub fn format<B: BlockIo>(
        block_io: &mut B,
        partition_start_lba: u64,
        partition_sectors: u64,
        volume_label: &str,
    ) -> DiskResult<Fat32Info> {
        // Validate partition size (min ~65MB, max ~2TB for FAT32)
        if partition_sectors < 133120 {
            return Err(DiskError::InvalidSize);
        }

        let total_sectors = partition_sectors as u32;
        let reserved_sectors = 32u16;
        let sectors_per_cluster = Self::optimal_cluster_size(total_sectors);

        // Calculate FAT size
        let fat_size =
            Self::calculate_fat_size(total_sectors, reserved_sectors, sectors_per_cluster);

        // Calculate cluster count
        let fat_sectors = fat_size * 2; // Two FAT copies
        let data_sectors = total_sectors - reserved_sectors as u32 - fat_sectors;
        let cluster_count = data_sectors / sectors_per_cluster as u32;

        // Build and write boot sector
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

        // Write FSInfo sector
        let fsinfo = Self::build_fsinfo(cluster_count - 1);
        let fsinfo_lba = Lba(partition_start_lba + 1);
        block_io
            .write_blocks(fsinfo_lba, &fsinfo)
            .map_err(|_| DiskError::IoError)?;

        // Write backup boot sector
        let backup_lba = Lba(partition_start_lba + 6);
        block_io
            .write_blocks(backup_lba, &boot_sector)
            .map_err(|_| DiskError::IoError)?;

        // Initialize first FAT sector (reserved entries + root cluster)
        let mut fat_sector = [0u8; SECTOR_SIZE];
        // Entry 0: Media type marker (0x0FFFFFF8)
        fat_sector[0] = 0xF8;
        fat_sector[1] = 0xFF;
        fat_sector[2] = 0xFF;
        fat_sector[3] = 0x0F;
        // Entry 1: End-of-chain marker
        fat_sector[4] = 0xFF;
        fat_sector[5] = 0xFF;
        fat_sector[6] = 0xFF;
        fat_sector[7] = 0xFF;
        // Entry 2: Root directory cluster (end-of-chain)
        fat_sector[8] = 0xFF;
        fat_sector[9] = 0xFF;
        fat_sector[10] = 0xFF;
        fat_sector[11] = 0x0F;

        // Write first FAT sector
        let fat1_lba = Lba(partition_start_lba + reserved_sectors as u64);
        block_io
            .write_blocks(fat1_lba, &fat_sector)
            .map_err(|_| DiskError::IoError)?;

        // Write second FAT sector
        let fat2_lba = Lba(partition_start_lba + reserved_sectors as u64 + fat_size as u64);
        block_io
            .write_blocks(fat2_lba, &fat_sector)
            .map_err(|_| DiskError::IoError)?;

        // Zero out root directory (first cluster of data area)
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

    /// Calculate optimal cluster size for partition
    fn optimal_cluster_size(total_sectors: u32) -> u8 {
        // Based on partition size, choose appropriate cluster size
        // to stay within FAT32 limits (max ~268M clusters)
        let size_mb = total_sectors / 2048; // Approx MB

        match size_mb {
            0..=512 => 1,        // <=512MB: 512B clusters
            513..=8192 => 8,     // <=8GB: 4KB clusters
            8193..=16384 => 16,  // <=16GB: 8KB clusters
            16385..=32768 => 32, // <=32GB: 16KB clusters
            _ => 64,             // >32GB: 32KB clusters
        }
    }

    /// Calculate FAT size in sectors
    fn calculate_fat_size(total_sectors: u32, reserved: u16, spc: u8) -> u32 {
        // Microsoft formula for FAT32 FAT size calculation
        let tmp1 = total_sectors - reserved as u32;
        let tmp2 = (256 * spc as u32) + 2;
        (tmp1 + tmp2 - 1) / tmp2
    }

    /// Build FAT32 boot sector
    fn build_boot_sector(
        total_sectors: u32,
        fat_size: u32,
        spc: u8,
        hidden_sectors: u32,
        label: &str,
    ) -> [u8; SECTOR_SIZE] {
        let mut bs = [0u8; SECTOR_SIZE];

        // Jump instruction
        bs[0] = 0xEB;
        bs[1] = 0x58;
        bs[2] = 0x90;

        // OEM name
        bs[3..11].copy_from_slice(b"MORPHEUS");

        // BPB (BIOS Parameter Block)
        bs[11..13].copy_from_slice(&512u16.to_le_bytes()); // Bytes per sector
        bs[13] = spc; // Sectors per cluster
        bs[14..16].copy_from_slice(&32u16.to_le_bytes()); // Reserved sectors
        bs[16] = 2; // Number of FATs
        bs[17..19].copy_from_slice(&0u16.to_le_bytes()); // Root entries (0 for FAT32)
        bs[19..21].copy_from_slice(&0u16.to_le_bytes()); // Total sectors 16 (0 for FAT32)
        bs[21] = 0xF8; // Media type
        bs[22..24].copy_from_slice(&0u16.to_le_bytes()); // FAT size 16 (0 for FAT32)
        bs[24..26].copy_from_slice(&63u16.to_le_bytes()); // Sectors per track
        bs[26..28].copy_from_slice(&255u16.to_le_bytes()); // Number of heads
        bs[28..32].copy_from_slice(&hidden_sectors.to_le_bytes());
        bs[32..36].copy_from_slice(&total_sectors.to_le_bytes());

        // FAT32 specific
        bs[36..40].copy_from_slice(&fat_size.to_le_bytes()); // FAT size 32
        bs[40..42].copy_from_slice(&0u16.to_le_bytes()); // Ext flags
        bs[42..44].copy_from_slice(&0u16.to_le_bytes()); // FS version
        bs[44..48].copy_from_slice(&2u32.to_le_bytes()); // Root cluster
        bs[48..50].copy_from_slice(&1u16.to_le_bytes()); // FSInfo sector
        bs[50..52].copy_from_slice(&6u16.to_le_bytes()); // Backup boot sector
                                                         // Reserved[12] at 52-63 already zero

        bs[64] = 0x80; // Drive number
        bs[65] = 0; // Reserved
        bs[66] = 0x29; // Boot signature
        bs[67..71].copy_from_slice(&0x12345678u32.to_le_bytes()); // Volume ID

        // Volume label (11 bytes, space-padded)
        let label_bytes = label.as_bytes();
        let mut label_buf = [b' '; 11];
        let copy_len = label_bytes.len().min(11);
        label_buf[..copy_len].copy_from_slice(&label_bytes[..copy_len]);
        bs[71..82].copy_from_slice(&label_buf);

        // FS type
        bs[82..90].copy_from_slice(b"FAT32   ");

        // Boot sector signature
        bs[510] = 0x55;
        bs[511] = 0xAA;

        bs
    }

    /// Build FSInfo sector
    fn build_fsinfo(free_clusters: u32) -> [u8; SECTOR_SIZE] {
        let mut fs = [0u8; SECTOR_SIZE];

        // Lead signature
        fs[0..4].copy_from_slice(&0x41615252u32.to_le_bytes());

        // Structure signature
        fs[484..488].copy_from_slice(&0x61417272u32.to_le_bytes());

        // Free cluster count
        fs[488..492].copy_from_slice(&free_clusters.to_le_bytes());

        // Next free cluster
        fs[492..496].copy_from_slice(&3u32.to_le_bytes());

        // Trail signature
        fs[508..512].copy_from_slice(&0xAA550000u32.to_le_bytes());

        fs
    }
}

/// Information about formatted FAT32 filesystem
#[derive(Debug, Clone, Copy)]
pub struct Fat32Info {
    /// Number of reserved sectors
    pub reserved_sectors: u16,
    /// Sectors per cluster
    pub sectors_per_cluster: u8,
    /// FAT size in sectors
    pub fat_size: u32,
    /// First LBA of data area
    pub data_start_lba: u64,
    /// Total cluster count
    pub cluster_count: u32,
}
