// Partition information and management

#[derive(Copy, Clone, Debug)]
pub struct PartitionInfo {
    pub index: u32,
    pub partition_type: PartitionType,
    pub start_lba: u64,
    pub end_lba: u64,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PartitionType {
    EfiSystem,
    LinuxFilesystem,
    LinuxSwap,
    BasicData,
    Unknown,
}

impl PartitionInfo {
    pub fn size_mb(&self) -> u64 {
        ((self.end_lba - self.start_lba + 1) * 512) / (1024 * 1024)
    }

    pub fn type_name(&self) -> &'static str {
        match self.partition_type {
            PartitionType::EfiSystem => "EFI System",
            PartitionType::LinuxFilesystem => "Linux FS",
            PartitionType::LinuxSwap => "Linux Swap",
            PartitionType::BasicData => "Basic Data",
            PartitionType::Unknown => "Unknown",
        }
    }
}

impl PartitionType {
    /// Convert from gpt_disk_types GUID to PartitionType
    pub fn from_gpt_guid(guid: &gpt_disk_types::GptPartitionType) -> Self {
        use gpt_disk_types::{guid, GptPartitionType as GptType};

        if guid == &GptType::EFI_SYSTEM {
            PartitionType::EfiSystem
        } else if guid == &GptType::BASIC_DATA {
            PartitionType::BasicData
        } else {
            // Check Linux types
            let linux_fs = GptType(guid!("0fc63daf-8483-4772-8e79-3d69d8477de4"));
            let linux_swap = GptType(guid!("0657fd6d-a4ab-43c4-84e5-0933c84b4f4f"));

            if guid == &linux_fs {
                PartitionType::LinuxFilesystem
            } else if guid == &linux_swap {
                PartitionType::LinuxSwap
            } else {
                PartitionType::Unknown
            }
        }
    }

    /// Convert to gpt_disk_types GUID
    pub fn to_gpt_guid(&self) -> gpt_disk_types::GptPartitionType {
        use gpt_disk_types::{guid, GptPartitionType as GptType};

        match self {
            PartitionType::EfiSystem => GptType::EFI_SYSTEM,
            PartitionType::BasicData => GptType::BASIC_DATA,
            PartitionType::LinuxFilesystem => {
                GptType(guid!("0fc63daf-8483-4772-8e79-3d69d8477de4"))
            }
            PartitionType::LinuxSwap => GptType(guid!("0657fd6d-a4ab-43c4-84e5-0933c84b4f4f")),
            PartitionType::Unknown => GptType::UNUSED,
        }
    }
}

/// Partition table for a disk
pub struct PartitionTable {
    partitions: [Option<PartitionInfo>; 16],
    count: usize,
    pub has_gpt: bool,
}

impl PartitionTable {
    pub const fn new() -> Self {
        Self {
            partitions: [None; 16],
            count: 0,
            has_gpt: false,
        }
    }

    pub fn clear(&mut self) {
        self.partitions = [None; 16];
        self.count = 0;
        self.has_gpt = false;
    }

    pub fn add_partition(&mut self, info: PartitionInfo) -> Result<(), ()> {
        if self.count >= 16 {
            return Err(());
        }

        self.partitions[self.count] = Some(info);
        self.count += 1;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn get(&self, index: usize) -> Option<&PartitionInfo> {
        if index < self.count {
            self.partitions[index].as_ref()
        } else {
            None
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &PartitionInfo> {
        self.partitions[..self.count]
            .iter()
            .filter_map(|p| p.as_ref())
    }
}
