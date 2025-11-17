// Common error type for FAT32 formatting operations

#[derive(Debug)]
pub enum Fat32Error {
    IoError,
    PartitionTooSmall,
    PartitionTooLarge,
    InvalidBlockSize,
    NotImplemented,
}
