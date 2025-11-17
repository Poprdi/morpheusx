// FAT32 filesystem formatter

use gpt_disk_io::BlockIo;

#[derive(Debug)]
pub enum Fat32Error {
    IoError,
    PartitionTooSmall,
    PartitionTooLarge,
    InvalidBlockSize,
    NotImplemented,
}

/// FAT32 Boot Sector (first 512 bytes of partition)
#[repr(C, packed)]
struct Fat32BootSector {
    jmp_boot: [u8; 3],       // Jump instruction
    oem_name: [u8; 8],       // OEM name
    bytes_per_sector: u16,   // Bytes per sector (usually 512)
    sectors_per_cluster: u8, // Sectors per cluster
    reserved_sectors: u16,   // Reserved sectors (usually 32 for FAT32)
    num_fats: u8,            // Number of FAT copies (usually 2)
    root_entry_count: u16,   // Root entries (0 for FAT32)
    total_sectors_16: u16,   // Total sectors (0 for FAT32)
    media_type: u8,          // Media descriptor (0xF8 for hard disk)
    fat_size_16: u16,        // FAT size (0 for FAT32)
    sectors_per_track: u16,  // Sectors per track
    num_heads: u16,          // Number of heads
    hidden_sectors: u32,     // Hidden sectors (LBA start)
    total_sectors_32: u32,   // Total sectors (actual count)
    fat_size_32: u32,        // FAT size in sectors
    ext_flags: u16,          // Extension flags
    fs_version: u16,         // Filesystem version
    root_cluster: u32,       // Root directory cluster (usually 2)
    fs_info_sector: u16,     // FSInfo sector (usually 1)
    backup_boot_sector: u16, // Backup boot sector (usually 6)
    reserved: [u8; 12],      // Reserved
    drive_number: u8,        // Drive number
    reserved1: u8,           // Reserved
    boot_signature: u8,      // Boot signature (0x29)
    volume_id: u32,          // Volume serial number
    volume_label: [u8; 11],  // Volume label
    fs_type: [u8; 8],        // Filesystem type ("FAT32   ")
    boot_code: [u8; 420],    // Boot code
    boot_sector_sig: u16,    // Boot sector signature (0xAA55)
}

impl Fat32BootSector {
    fn new(total_sectors: u32, fat_size: u32, hidden_sectors: u32) -> Self {
        let mut bs = Self {
            jmp_boot: [0xEB, 0x58, 0x90], // JMP short + NOP
            oem_name: *b"MORPHEUS",
            bytes_per_sector: 512,
            sectors_per_cluster: 8, // 4KB clusters
            reserved_sectors: 32,
            num_fats: 2,
            root_entry_count: 0, // FAT32 uses cluster chain
            total_sectors_16: 0, // Use 32-bit field
            media_type: 0xF8,    // Hard disk
            fat_size_16: 0,      // Use 32-bit field
            sectors_per_track: 63,
            num_heads: 255,
            hidden_sectors,
            total_sectors_32: total_sectors,
            fat_size_32: fat_size,
            ext_flags: 0,
            fs_version: 0,
            root_cluster: 2, // Root starts at cluster 2
            fs_info_sector: 1,
            backup_boot_sector: 6,
            reserved: [0; 12],
            drive_number: 0x80, // Hard disk
            reserved1: 0,
            boot_signature: 0x29,
            volume_id: 0x12345678, // Random-ish serial
            volume_label: *b"MORPHEUS   ",
            fs_type: *b"FAT32   ",
            boot_code: [0; 420],
            boot_sector_sig: 0xAA55,
        };

        // Simple boot code: just print message and halt
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

/// FSInfo sector (sector 1)
#[repr(C, packed)]
struct FsInfoSector {
    lead_sig: u32, // 0x41615252
    reserved1: [u8; 480],
    struc_sig: u32,  // 0x61417272
    free_count: u32, // Free cluster count (-1 = unknown)
    next_free: u32,  // Next free cluster
    reserved2: [u8; 12],
    trail_sig: u32, // 0xAA550000
}

impl FsInfoSector {
    fn new(free_count: u32) -> Self {
        Self {
            lead_sig: 0x41615252,
            reserved1: [0; 480],
            struc_sig: 0x61417272,
            free_count,
            next_free: 3, // Start allocating from cluster 3
            reserved2: [0; 12],
            trail_sig: 0xAA550000,
        }
    }

    fn to_bytes(&self) -> [u8; 512] {
        unsafe { core::mem::transmute_copy(self) }
    }
}

/// Calculate FAT size based on partition size
fn calculate_fat_size(total_sectors: u32, reserved_sectors: u16, sectors_per_cluster: u8) -> u32 {
    // FAT32 calculation based on Microsoft formula
    // Each FAT entry is 4 bytes, FAT has 2 copies

    let num_fats = 2u32;
    let bytes_per_sector = 512u32;

    // Calculate: ((total_sectors - reserved) * bytes_per_sector) / ((sectors_per_cluster * bytes_per_sector) + (num_fats * 4))
    let tmp1 = total_sectors - reserved_sectors as u32;
    let tmp2 = (bytes_per_sector * sectors_per_cluster as u32) + (num_fats * 4);

    (tmp1 * bytes_per_sector).div_ceil(tmp2)
}

/// Format partition as FAT32
pub fn verify_fat32<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
) -> Result<(), Fat32Error> {
    let mut buffer = [0u8; 512];

    // Read and verify boot sector
    let boot_lba = gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(partition_lba_start));
    block_io
        .read_blocks(boot_lba, &mut buffer)
        .map_err(|_| Fat32Error::IoError)?;

    // Check boot signature
    if buffer[510] != 0x55 || buffer[511] != 0xAA {
        return Err(Fat32Error::IoError);
    }

    // Verify OEM name
    if &buffer[3..11] != b"MORPHEUS" {
        return Err(Fat32Error::IoError);
    }

    // Verify bytes per sector
    let bytes_per_sector = u16::from_le_bytes([buffer[11], buffer[12]]);
    if bytes_per_sector != 512 {
        return Err(Fat32Error::InvalidBlockSize);
    }

    // Verify reserved sectors
    let reserved_sectors = u16::from_le_bytes([buffer[14], buffer[15]]);
    if reserved_sectors != 32 {
        return Err(Fat32Error::IoError);
    }

    // Verify num FATs
    if buffer[16] != 2 {
        return Err(Fat32Error::IoError);
    }

    // Verify root cluster
    let root_cluster = u32::from_le_bytes([buffer[44], buffer[45], buffer[46], buffer[47]]);
    if root_cluster != 2 {
        return Err(Fat32Error::IoError);
    }

    // Verify FSInfo sector number
    let fsinfo_sector = u16::from_le_bytes([buffer[48], buffer[49]]);
    if fsinfo_sector != 1 {
        return Err(Fat32Error::IoError);
    }

    // Verify backup boot sector number
    let backup_sector = u16::from_le_bytes([buffer[50], buffer[51]]);
    if backup_sector != 6 {
        return Err(Fat32Error::IoError);
    }

    // Verify filesystem type string
    if &buffer[82..90] != b"FAT32   " {
        return Err(Fat32Error::IoError);
    }

    // Extract FAT size for later checks
    let fat_size = u32::from_le_bytes([buffer[36], buffer[37], buffer[38], buffer[39]]);

    // Read and verify FSInfo sector
    let fsinfo_lba =
        gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(partition_lba_start + 1));
    block_io
        .read_blocks(fsinfo_lba, &mut buffer)
        .map_err(|_| Fat32Error::IoError)?;

    // Check FSInfo lead signature
    let lead_sig = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    if lead_sig != 0x41615252 {
        return Err(Fat32Error::IoError);
    }

    // Check FSInfo struct signature
    let struc_sig = u32::from_le_bytes([buffer[484], buffer[485], buffer[486], buffer[487]]);
    if struc_sig != 0x61417272 {
        return Err(Fat32Error::IoError);
    }

    // Check FSInfo trail signature
    let trail_sig = u32::from_le_bytes([buffer[508], buffer[509], buffer[510], buffer[511]]);
    if trail_sig != 0xAA550000 {
        return Err(Fat32Error::IoError);
    }

    // Read and verify backup boot sector
    let backup_lba =
        gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(partition_lba_start + 6));
    block_io
        .read_blocks(backup_lba, &mut buffer)
        .map_err(|_| Fat32Error::IoError)?;

    // Verify backup has same boot signature
    if buffer[510] != 0x55 || buffer[511] != 0xAA {
        return Err(Fat32Error::IoError);
    }

    // Verify backup OEM name matches
    if &buffer[3..11] != b"MORPHEUS" {
        return Err(Fat32Error::IoError);
    }

    // Read and verify first FAT sector
    let fat1_lba = gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(
        partition_lba_start + reserved_sectors as u64,
    ));
    block_io
        .read_blocks(fat1_lba, &mut buffer)
        .map_err(|_| Fat32Error::IoError)?;

    // Check FAT media descriptor and EOC markers
    if buffer[0] != 0xF8 || buffer[1] != 0xFF || buffer[2] != 0xFF || buffer[3] != 0xFF {
        return Err(Fat32Error::IoError);
    }

    // Check FAT EOC marker
    if buffer[4] != 0xFF || buffer[5] != 0xFF || buffer[6] != 0xFF || buffer[7] != 0xFF {
        return Err(Fat32Error::IoError);
    }

    // Check root directory EOC marker
    if buffer[8] != 0xFF || buffer[9] != 0xFF || buffer[10] != 0xFF || buffer[11] != 0xFF {
        return Err(Fat32Error::IoError);
    }

    // Read and verify second FAT sector
    let fat2_lba = gpt_disk_types::Lba::from(gpt_disk_types::LbaLe::from_u64(
        partition_lba_start + reserved_sectors as u64 + fat_size as u64,
    ));
    block_io
        .read_blocks(fat2_lba, &mut buffer)
        .map_err(|_| Fat32Error::IoError)?;

    // Verify second FAT has same markers as first
    if buffer[0] != 0xF8 || buffer[1] != 0xFF || buffer[2] != 0xFF || buffer[3] != 0xFF {
        return Err(Fat32Error::IoError);
    }

    if buffer[4] != 0xFF || buffer[5] != 0xFF || buffer[6] != 0xFF || buffer[7] != 0xFF {
        return Err(Fat32Error::IoError);
    }

    if buffer[8] != 0xFF || buffer[9] != 0xFF || buffer[10] != 0xFF || buffer[11] != 0xFF {
        return Err(Fat32Error::IoError);
    }

    Ok(())
}
