// FAT32 filesystem operations - minimal implementation for bootloader installation
// We only need to write files, not read them (UEFI does that for us)

use super::Fat32Error;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

const SECTOR_SIZE: usize = 512;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;

/// FAT32 directory entry (32 bytes)
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct DirEntry {
    name: [u8; 11], // 8.3 filename
    attr: u8,       // File attributes
    _reserved: u8,
    _create_time_tenth: u8,
    _create_time: u16,
    _create_date: u16,
    _access_date: u16,
    cluster_high: u16, // High word of first cluster
    _modify_time: u16,
    _modify_date: u16,
    cluster_low: u16, // Low word of first cluster
    file_size: u32,   // File size in bytes
}

impl DirEntry {
    fn empty() -> Self {
        Self {
            name: [0; 11],
            attr: 0,
            _reserved: 0,
            _create_time_tenth: 0,
            _create_time: 0,
            _create_date: 0,
            _access_date: 0,
            cluster_high: 0,
            _modify_time: 0,
            _modify_date: 0,
            cluster_low: 0,
            file_size: 0,
        }
    }

    fn is_free(&self) -> bool {
        self.name[0] == 0x00 || self.name[0] == 0xE5
    }

    fn set_name(&mut self, name: &str) {
        // Convert to 8.3 format (simple, no LFN)
        self.name = [0x20; 11]; // Fill with spaces

        let parts: Vec<&str> = name.split('.').collect();
        let basename = parts[0].as_bytes();
        let ext = if parts.len() > 1 {
            parts[1].as_bytes()
        } else {
            b""
        };

        let base_len = basename.len().min(8);
        self.name[..base_len].copy_from_slice(&basename[..base_len]);

        let ext_len = ext.len().min(3);
        self.name[8..8 + ext_len].copy_from_slice(&ext[..ext_len]);

        // Convert to uppercase
        for byte in &mut self.name {
            if *byte >= b'a' && *byte <= b'z' {
                *byte -= 32;
            }
        }
    }

    fn first_cluster(&self) -> u32 {
        ((self.cluster_high as u32) << 16) | (self.cluster_low as u32)
    }

    fn set_first_cluster(&mut self, cluster: u32) {
        self.cluster_high = (cluster >> 16) as u16;
        self.cluster_low = (cluster & 0xFFFF) as u16;
    }
}

/// FAT32 filesystem context
struct Fat32Context {
    sectors_per_cluster: u32,
    reserved_sectors: u32,
    fat_size: u32,
    num_fats: u32,
    root_cluster: u32,
    data_start_sector: u32,
}

impl Fat32Context {
    fn from_boot_sector<B: BlockIo>(
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

    fn cluster_to_sector(&self, cluster: u32) -> u32 {
        self.data_start_sector + ((cluster - 2) * self.sectors_per_cluster)
    }

    fn read_fat_entry<B: BlockIo>(
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

    fn write_fat_entry<B: BlockIo>(
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

    fn find_free_cluster<B: BlockIo>(
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

    fn allocate_cluster<B: BlockIo>(
        &self,
        block_io: &mut B,
        partition_start: u64,
    ) -> Result<u32, Fat32Error> {
        let cluster = self.find_free_cluster(block_io, partition_start, 2)?;
        self.write_fat_entry(block_io, partition_start, cluster, 0x0FFFFFF8)?; // EOC marker
        Ok(cluster)
    }
}

/// Write file to FAT32 partition
pub fn write_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
    data: &[u8],
) -> Result<(), Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;

    // Parse path
    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();

    // Navigate/create directory structure
    let mut current_cluster = ctx.root_cluster;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        if !is_last {
            // This is a directory component
            current_cluster = ensure_directory_exists(
                block_io,
                partition_lba_start,
                &ctx,
                current_cluster,
                part,
            )?;
        } else {
            // This is the file name - create/write it
            write_file_in_directory(
                block_io,
                partition_lba_start,
                &ctx,
                current_cluster,
                part,
                data,
            )?;
        }
    }

    block_io.flush().map_err(|_| Fat32Error::IoError)?;
    Ok(())
}

/// Ensure directory exists in parent, return its cluster
fn ensure_directory_exists<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    parent_cluster: u32,
    name: &str,
) -> Result<u32, Fat32Error> {
    // Read parent directory
    let sector = ctx.cluster_to_sector(parent_cluster);
    let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

    for sec_offset in 0..ctx.sectors_per_cluster {
        let mut sector_data = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(
                Lba(partition_start + sector as u64 + sec_offset as u64),
                &mut sector_data,
            )
            .map_err(|_| Fat32Error::IoError)?;

        let entries = unsafe {
            core::slice::from_raw_parts(sector_data.as_ptr() as *const DirEntry, entries_per_sector)
        };

        // Check if directory alrady exists
        for entry in entries {
            if !entry.is_free() && entry.attr & ATTR_DIRECTORY != 0 {
                let mut entry_name = [0u8; 11];
                entry_name.copy_from_slice(&entry.name);

                let mut test_entry = DirEntry::empty();
                test_entry.set_name(name);

                if entry_name == test_entry.name {
                    return Ok(entry.first_cluster());
                }
            }
        }
    }

    // Directory doesn't exist - create it
    create_directory_in_parent(block_io, partition_start, ctx, parent_cluster, name)
}

/// Create new directory entry in parent
fn create_directory_in_parent<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    parent_cluster: u32,
    name: &str,
) -> Result<u32, Fat32Error> {
    let new_cluster = ctx.allocate_cluster(block_io, partition_start)?;

    // Initialize new directory cluster with . and .. entries
    let cluster_size = (ctx.sectors_per_cluster * SECTOR_SIZE as u32) as usize;
    let mut cluster_data = vec![0u8; cluster_size];

    // Create '.' entry (points to self)
    let mut dot_entry = DirEntry::empty();
    dot_entry.name = *b".          "; // '.' padded with spaces
    dot_entry.attr = ATTR_DIRECTORY;
    dot_entry.set_first_cluster(new_cluster);

    // Create '..' entry (points to parent)
    let mut dotdot_entry = DirEntry::empty();
    dotdot_entry.name = *b"..         "; // '..' padded with spaces
    dotdot_entry.attr = ATTR_DIRECTORY;
    dotdot_entry.set_first_cluster(parent_cluster);

    // Write entries to cluster data
    let entries =
        unsafe { core::slice::from_raw_parts_mut(cluster_data.as_mut_ptr() as *mut DirEntry, 2) };
    entries[0] = dot_entry;
    entries[1] = dotdot_entry;

    let sector = ctx.cluster_to_sector(new_cluster);
    for sec_offset in 0..ctx.sectors_per_cluster {
        let start = (sec_offset * SECTOR_SIZE as u32) as usize;
        let end = start + SECTOR_SIZE;
        block_io
            .write_blocks(
                Lba(partition_start + sector as u64 + sec_offset as u64),
                &cluster_data[start..end],
            )
            .map_err(|_| Fat32Error::IoError)?;
    }

    // Add entry to parent directory
    add_dir_entry_to_cluster(
        block_io,
        partition_start,
        ctx,
        parent_cluster,
        name,
        new_cluster,
        0,
        ATTR_DIRECTORY,
    )?;

    Ok(new_cluster)
}

/// Write file data in directory
fn write_file_in_directory<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    dir_cluster: u32,
    name: &str,
    data: &[u8],
) -> Result<(), Fat32Error> {
    // Allocate clusters for file data
    let cluster_size = (ctx.sectors_per_cluster * SECTOR_SIZE as u32) as usize;
    let clusters_needed = ((data.len() + cluster_size - 1) / cluster_size).max(1);

    let mut file_clusters = Vec::new();
    for _ in 0..clusters_needed {
        let cluster = ctx.allocate_cluster(block_io, partition_start)?;
        file_clusters.push(cluster);
    }

    // Chain clusters together in FAT
    for i in 0..file_clusters.len() - 1 {
        ctx.write_fat_entry(
            block_io,
            partition_start,
            file_clusters[i],
            file_clusters[i + 1],
        )?;
    }
    // Last cluster is already marked with EOC by allocate_cluster

    // Write file data to clusters
    for (i, &cluster) in file_clusters.iter().enumerate() {
        let data_offset = i * cluster_size;
        let data_end = (data_offset + cluster_size).min(data.len());
        let chunk_size = data_end - data_offset;

        let mut cluster_data = vec![0u8; cluster_size];
        cluster_data[..chunk_size].copy_from_slice(&data[data_offset..data_end]);

        let sector = ctx.cluster_to_sector(cluster);
        for sec_offset in 0..ctx.sectors_per_cluster {
            let start = (sec_offset * SECTOR_SIZE as u32) as usize;
            let end = start + SECTOR_SIZE;
            block_io
                .write_blocks(
                    Lba(partition_start + sector as u64 + sec_offset as u64),
                    &cluster_data[start..end],
                )
                .map_err(|_| Fat32Error::IoError)?;
        }
    }

    // Add directory entry
    add_dir_entry_to_cluster(
        block_io,
        partition_start,
        ctx,
        dir_cluster,
        name,
        file_clusters[0],
        data.len() as u32,
        ATTR_ARCHIVE,
    )?;

    Ok(())
}

/// Add directory entry to cluster
fn add_dir_entry_to_cluster<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    cluster: u32,
    name: &str,
    first_cluster: u32,
    file_size: u32,
    attr: u8,
) -> Result<(), Fat32Error> {
    let sector = ctx.cluster_to_sector(cluster);
    let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

    for sec_offset in 0..ctx.sectors_per_cluster {
        let mut sector_data = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(
                Lba(partition_start + sector as u64 + sec_offset as u64),
                &mut sector_data,
            )
            .map_err(|_| Fat32Error::IoError)?;

        let entries = unsafe {
            core::slice::from_raw_parts_mut(
                sector_data.as_mut_ptr() as *mut DirEntry,
                entries_per_sector,
            )
        };

        // Find first free entry
        for entry in entries.iter_mut() {
            if entry.is_free() {
                entry.set_name(name);
                entry.attr = attr;
                entry.set_first_cluster(first_cluster);
                entry.file_size = file_size;

                block_io
                    .write_blocks(
                        Lba(partition_start + sector as u64 + sec_offset as u64),
                        &sector_data,
                    )
                    .map_err(|_| Fat32Error::IoError)?;

                return Ok(());
            }
        }
    }

    Err(Fat32Error::IoError) // Directory full
}

/// Create directory (creates full path)
pub fn create_directory<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<(), Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;

    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();

    let mut current_cluster = ctx.root_cluster;
    for part in parts {
        current_cluster =
            ensure_directory_exists(block_io, partition_lba_start, &ctx, current_cluster, part)?;
    }

    block_io.flush().map_err(|_| Fat32Error::IoError)?;
    Ok(())
}

/// Read file data from FAT32 partition
pub fn read_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<Vec<u8>, Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;

    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();

    let mut current_cluster = ctx.root_cluster;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        let sector = ctx.cluster_to_sector(current_cluster);
        let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

        let mut found = false;
        for sec_offset in 0..ctx.sectors_per_cluster {
            let mut sector_data = [0u8; SECTOR_SIZE];
            block_io
                .read_blocks(
                    Lba(partition_lba_start + sector as u64 + sec_offset as u64),
                    &mut sector_data,
                )
                .map_err(|_| Fat32Error::IoError)?;

            let entries = unsafe {
                core::slice::from_raw_parts(
                    sector_data.as_ptr() as *const DirEntry,
                    entries_per_sector,
                )
            };

            for entry in entries {
                if !entry.is_free() {
                    let mut test_entry = DirEntry::empty();
                    test_entry.set_name(part);

                    if entry.name == test_entry.name {
                        if is_last {
                            // Found the file - read its data
                            if entry.attr & ATTR_DIRECTORY != 0 {
                                return Err(Fat32Error::IoError); // Can't read directory as file
                            }

                            let file_size = entry.file_size as usize;
                            let mut data = vec![0u8; file_size];
                            let mut data_offset = 0;
                            let cluster_size =
                                (ctx.sectors_per_cluster * SECTOR_SIZE as u32) as usize;

                            // Follow cluster chain
                            let mut current_file_cluster = entry.first_cluster();
                            while current_file_cluster < 0x0FFFFFF8 {
                                let sector = ctx.cluster_to_sector(current_file_cluster);
                                let bytes_to_read = (file_size - data_offset).min(cluster_size);

                                // Read cluster data
                                let mut cluster_data = vec![0u8; cluster_size];
                                for sec_offset in 0..ctx.sectors_per_cluster {
                                    let start = (sec_offset * SECTOR_SIZE as u32) as usize;
                                    let end = start + SECTOR_SIZE;
                                    block_io
                                        .read_blocks(
                                            Lba(partition_lba_start
                                                + sector as u64
                                                + sec_offset as u64),
                                            &mut cluster_data[start..end],
                                        )
                                        .map_err(|_| Fat32Error::IoError)?;
                                }

                                data[data_offset..data_offset + bytes_to_read]
                                    .copy_from_slice(&cluster_data[..bytes_to_read]);
                                data_offset += bytes_to_read;

                                if data_offset >= file_size {
                                    break;
                                }

                                // Get next cluster from FAT
                                current_file_cluster = ctx.read_fat_entry(
                                    block_io,
                                    partition_lba_start,
                                    current_file_cluster,
                                )?;
                            }

                            return Ok(data);
                        } else {
                            current_cluster = entry.first_cluster();
                            found = true;
                            break;
                        }
                    }
                }
            }

            if found {
                break;
            }
        }

        if !found {
            return Err(Fat32Error::IoError);
        } // Path not found
    }

    Err(Fat32Error::IoError)
}

/// Check if file exists
pub fn file_exists<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<bool, Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;

    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();

    let mut current_cluster = ctx.root_cluster;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        let sector = ctx.cluster_to_sector(current_cluster);
        let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

        let mut found = false;
        for sec_offset in 0..ctx.sectors_per_cluster {
            let mut sector_data = [0u8; SECTOR_SIZE];
            block_io
                .read_blocks(
                    Lba(partition_lba_start + sector as u64 + sec_offset as u64),
                    &mut sector_data,
                )
                .map_err(|_| Fat32Error::IoError)?;

            let entries = unsafe {
                core::slice::from_raw_parts(
                    sector_data.as_ptr() as *const DirEntry,
                    entries_per_sector,
                )
            };

            for entry in entries {
                if !entry.is_free() {
                    let mut test_entry = DirEntry::empty();
                    test_entry.set_name(part);

                    if entry.name == test_entry.name {
                        if is_last {
                            return Ok(entry.attr & ATTR_DIRECTORY == 0); // True if it's a file
                        } else {
                            current_cluster = entry.first_cluster();
                            found = true;
                            break;
                        }
                    }
                }
            }

            if found {
                break;
            }
        }

        if !found {
            return Ok(false);
        }
    }

    Ok(false)
}
