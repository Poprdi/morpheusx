#[derive(Debug)]
pub enum Fat32Error {
    IoError,
    PartitionTooSmall,
    PartitionTooLarge,
    InvalidBlockSize,
    NotImplemented,
}
