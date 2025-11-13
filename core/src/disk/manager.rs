// Disk manager - handle enumeration and detection

use crate::disk::partition::PartitionTable;

/// Represents a physical disk device
pub struct DiskInfo {
    pub media_id: u32,
    pub block_size: u32,
    pub last_block: u64,
    pub removable: bool,
    pub read_only: bool,
    pub partitions: PartitionTable,
}

/// Manager for discovering and accessing disks
pub struct DiskManager {
    disks: [Option<DiskInfo>; 8], // Max 8 disks
    count: usize,
}

impl DiskManager {
    pub const fn new() -> Self {
        Self {
            disks: [None, None, None, None, None, None, None, None],
            count: 0,
        }
    }
    
    pub fn disk_count(&self) -> usize {
        self.count
    }
    
    pub fn get_disk(&self, index: usize) -> Option<&DiskInfo> {
        if index < self.count {
            self.disks[index].as_ref()
        } else {
            None
        }
    }
    
    /// Add a disk to the manager
    pub fn add_disk(&mut self, info: DiskInfo) -> Result<(), ()> {
        if self.count >= 8 {
            return Err(());
        }
        
        self.disks[self.count] = Some(info);
        self.count += 1;
        Ok(())
    }
    
    /// Clear all disks
    pub fn clear(&mut self) {
        self.disks = [None, None, None, None, None, None, None, None];
        self.count = 0;
    }
}

impl DiskInfo {
    pub fn new(
        media_id: u32,
        block_size: u32,
        last_block: u64,
        removable: bool,
        read_only: bool,
    ) -> Self {
        Self {
            media_id,
            block_size,
            last_block,
            removable,
            read_only,
            partitions: PartitionTable::new(),
        }
    }
    
    pub fn size_mb(&self) -> u64 {
        ((self.last_block + 1) * self.block_size as u64) / (1024 * 1024)
    }
}
