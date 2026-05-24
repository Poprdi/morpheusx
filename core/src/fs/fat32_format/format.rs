// FAT32 formatter. Layout per Microsoft FAT spec.

use super::Fat32Error;
use gpt_disk_io::BlockIo;

#[repr(C, packed)]
struct Fat32BootSector {
    jmp_boot: [u8; 3],
    oem_name: [u8; 8],
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    total_sectors_16: u16,
    media_type: u8,
    fat_size_16: u16,
    sectors_per_track: u16,
    num_heads: u16,
    hidden_sectors: u32,
    total_sectors_32: u32,
    fat_size_32: u32,
    ext_flags: u16,
    fs_version: u16,
    root_cluster: u32,
    fs_info_sector: u16,
    backup_boot_sector: u16,
    reserved: [u8; 12],
    drive_number: u8,
    reserved1: u8,
    boot_signature: u8,
    volume_id: u32,
    volume_label: [u8; 11],
    fs_type: [u8; 8],
    boot_code: [u8; 420],
    boot_sector_sig: u16,
}

impl Fat32BootSector {
    fn new(total_sectors: u32, fat_size: u32, hidden_sectors: u32) -> Self {
        let mut bs = Self {
            jmp_boot: [0xEB, 0x58, 0x90],
            oem_name: *b"MORPHEUS",
            bytes_per_sector: 512,
            sectors_per_cluster: 8, // 4 KiB clusters
            reserved_sectors: 32,
            num_fats: 2,
            root_entry_count: 0,
            total_sectors_16: 0,
            media_type: 0xF8,
            fat_size_16: 0,
            sectors_per_track: 63,
            num_heads: 255,
            hidden_sectors,
            total_sectors_32: total_sectors,
            fat_size_32: fat_size,
            ext_flags: 0,
            fs_version: 0,
            root_cluster: 2,
            fs_info_sector: 1,
            backup_boot_sector: 6,
            reserved: [0; 12],
            drive_number: 0x80,
            reserved1: 0,
            boot_signature: 0x29,
            volume_id: 0x12345678,
            volume_label: *b"MORPHEUS   ",
            fs_type: *b"FAT32   ",
            boot_code: [0; 420],
            boot_sector_sig: 0xAA55,
        };

        // Print 'M' via BIOS teletype, then HLT. We never legacy-boot, but
        // some firmware sniffs the boot code for sanity.
        bs.boot_code[0] = 0xB4; // MOV AH, 0x0E
        bs.boot_code[1] = 0x0E;
        bs.boot_code[2] = 0xB0; // MOV AL, 'M'
        bs.boot_code[3] = b'M';
        bs.boot_code[4] = 0xCD; // INT 0x10
        bs.boot_code[5] = 0x10;
        bs.boot_code[6] = 0xF4; // HLT

        bs
    }

    fn to_bytes(&self) -> [u8; 512] {
        unsafe { core::mem::transmute_copy(self) }
    }
}

#[repr(C, packed)]
struct FsInfoSector {
    lead_sig: u32,
    reserved1: [u8; 480],
    struc_sig: u32,
    /// 0xFFFFFFFF means unknown.
    free_count: u32,
    next_free: u32,
    reserved2: [u8; 12],
    trail_sig: u32,
}

impl FsInfoSector {
    fn new(free_count: u32) -> Self {
        Self {
            lead_sig: 0x41615252,
            reserved1: [0; 480],
            struc_sig: 0x61417272,
            free_count,
            next_free: 3,
            reserved2: [0; 12],
            trail_sig: 0xAA550000,
        }
    }

    fn to_bytes(&self) -> [u8; 512] {
        unsafe { core::mem::transmute_copy(self) }
    }
}

/// Microsoft FAT spec sizing formula. Two FATs, 4 bytes per entry.
fn calculate_fat_size(total_sectors: u32, reserved_sectors: u16, sectors_per_cluster: u8) -> u32 {
    let num_fats = 2u32;
    let bytes_per_sector = 512u32;

    let tmp1 = total_sectors - reserved_sectors as u32;
    let tmp2 = (bytes_per_sector * sectors_per_cluster as u32) + (num_fats * 4);

    (tmp1 * bytes_per_sector).div_ceil(tmp2)
}

pub fn format_fat32<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    partition_sectors: u64,
) -> Result<(), Fat32Error> {
    if partition_sectors < 133120 {
        return Err(Fat32Error::PartitionTooSmall);
    }

    if partition_sectors > 0xFFFFFFFF {
        return Err(Fat32Error::PartitionTooLarge);
    }

    let total_sectors = partition_sectors as u32;
    let reserved_sectors = 32u16;
    let sectors_per_cluster = 8u8;

    let fat_size = calculate_fat_size(total_sectors, reserved_sectors, sectors_per_cluster);

    let fat_sectors = fat_size * 2;
    let data_sectors = total_sectors - reserved_sectors as u32 - fat_sectors;
    let cluster_count = data_sectors / sectors_per_cluster as u32;

    let boot_sector = Fat32BootSector::new(total_sectors, fat_size, partition_lba_start as u32);
    let boot_bytes = boot_sector.to_bytes();

    let lba = gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(partition_lba_start));
    block_io
        .write_blocks(lba, &boot_bytes)
        .map_err(|_| Fat32Error::IoError)?;

    let fsinfo = FsInfoSector::new(cluster_count - 1); // less root cluster
    let fsinfo_bytes = fsinfo.to_bytes();
    let fsinfo_lba =
        gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(partition_lba_start + 1));
    block_io
        .write_blocks(fsinfo_lba, &fsinfo_bytes)
        .map_err(|_| Fat32Error::IoError)?;

    let backup_lba =
        gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(partition_lba_start + 6));
    block_io
        .write_blocks(backup_lba, &boot_bytes)
        .map_err(|_| Fat32Error::IoError)?;

    // Reserved: 0xFFFFFFF8 (media), 0xFFFFFFFF (EOC). Cluster 2 (root): EOC.
    let mut fat_sector = [0u8; 512];
    fat_sector[0] = 0xF8;
    fat_sector[1] = 0xFF;
    fat_sector[2] = 0xFF;
    fat_sector[3] = 0xFF;
    fat_sector[4] = 0xFF;
    fat_sector[5] = 0xFF;
    fat_sector[6] = 0xFF;
    fat_sector[7] = 0xFF;
    fat_sector[8] = 0xFF;
    fat_sector[9] = 0xFF;
    fat_sector[10] = 0xFF;
    fat_sector[11] = 0xFF;

    let fat1_lba = gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(
        partition_lba_start + reserved_sectors as u64,
    ));
    block_io
        .write_blocks(fat1_lba, &fat_sector)
        .map_err(|_| Fat32Error::IoError)?;

    let fat2_lba = gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(
        partition_lba_start + reserved_sectors as u64 + fat_size as u64,
    ));
    block_io
        .write_blocks(fat2_lba, &fat_sector)
        .map_err(|_| Fat32Error::IoError)?;

    // Zero out cluster 2 (root dir).
    let root_lba_val = partition_lba_start + reserved_sectors as u64 + (fat_size * 2) as u64;
    let empty_sector = [0u8; 512];
    for i in 0..sectors_per_cluster {
        let lba =
            gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(root_lba_val + i as u64));
        block_io
            .write_blocks(lba, &empty_sector)
            .map_err(|_| Fat32Error::IoError)?;
    }

    Ok(())
}
