// GPT (GUID Partition Table) parser

#[repr(C, packed)]
pub struct GptHeader {
    pub signature: [u8; 8],           // "EFI PART"
    pub revision: u32,
    pub header_size: u32,
    pub header_crc32: u32,
    pub reserved: u32,
    pub current_lba: u64,
    pub backup_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub partition_entry_lba: u64,
    pub num_partition_entries: u32,
    pub partition_entry_size: u32,
    pub partition_array_crc32: u32,
}

#[repr(C, packed)]
pub struct GptPartitionEntry {
    pub partition_type_guid: [u8; 16],
    pub unique_partition_guid: [u8; 16],
    pub starting_lba: u64,
    pub ending_lba: u64,
    pub attributes: u64,
    pub partition_name: [u16; 36],     // UTF-16LE
}

// Common partition type GUIDs
pub const GUID_EFI_SYSTEM: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11,
    0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

pub const GUID_LINUX_FILESYSTEM: [u8; 16] = [
    0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47,
    0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
];

pub const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";

impl GptHeader {
    pub fn validate(&self) -> bool {
        unsafe {
            let sig_ptr = core::ptr::addr_of!(self.signature);
            let rev_ptr = core::ptr::addr_of!(self.revision);
            core::ptr::read_unaligned(sig_ptr) == *GPT_SIGNATURE && 
            core::ptr::read_unaligned(rev_ptr) == 0x00010000
        }
    }
    
    pub fn from_bytes(data: &[u8]) -> Option<&Self> {
        if data.len() < core::mem::size_of::<Self>() {
            return None;
        }
        
        let header = unsafe { &*(data.as_ptr() as *const GptHeader) };
        
        if header.validate() {
            Some(header)
        } else {
            None
        }
    }
}

impl GptPartitionEntry {
    pub fn is_used(&self) -> bool {
        unsafe {
            let guid_ptr = core::ptr::addr_of!(self.partition_type_guid);
            core::ptr::read_unaligned(guid_ptr) != [0u8; 16]
        }
    }
    
    pub fn matches_type(&self, type_guid: &[u8; 16]) -> bool {
        unsafe {
            let guid_ptr = core::ptr::addr_of!(self.partition_type_guid);
            &core::ptr::read_unaligned(guid_ptr) == type_guid
        }
    }
    
    pub fn get_name(&self) -> [u8; 36] {
        let mut name = [0u8; 36];
        unsafe {
            let name_ptr = core::ptr::addr_of!(self.partition_name);
            let partition_name = core::ptr::read_unaligned(name_ptr);
            for (i, &c) in partition_name.iter().enumerate() {
                if i < name.len() {
                    name[i] = c as u8; // Simplified UTF-16 -> ASCII (works for ASCII names)
                }
            }
        }
        name
    }
}

pub struct GptPartitionTable<'a> {
    pub header: &'a GptHeader,
    pub entries: &'a [u8],
}

impl<'a> GptPartitionTable<'a> {
    pub fn new(header: &'a GptHeader, entries_data: &'a [u8]) -> Self {
        Self {
            header,
            entries: entries_data,
        }
    }
    
    pub fn get_entry(&self, index: u32) -> Option<&GptPartitionEntry> {
        if index >= self.header.num_partition_entries {
            return None;
        }
        
        let entry_size = self.header.partition_entry_size as usize;
        let offset = (index as usize) * entry_size;
        
        if offset + entry_size > self.entries.len() {
            return None;
        }
        
        let entry = unsafe {
            &*(self.entries.as_ptr().add(offset) as *const GptPartitionEntry)
        };
        
        if entry.is_used() {
            Some(entry)
        } else {
            None
        }
    }
    
    pub fn find_by_type(&self, type_guid: &[u8; 16]) -> Option<&GptPartitionEntry> {
        for i in 0..self.header.num_partition_entries {
            if let Some(entry) = self.get_entry(i) {
                if entry.matches_type(type_guid) {
                    return Some(entry);
                }
            }
        }
        None
    }
}
