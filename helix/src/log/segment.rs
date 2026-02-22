//! Log segment utilities.

use crate::crc::crc32c;
use crate::types::*;

impl LogSegmentHeader {
    /// Create a new segment header for the given sequence / start LSN.
    pub fn new(sequence: u64, lsn_start: Lsn, timestamp_ns: u64) -> Self {
        let mut hdr = Self {
            magic: LOG_SEGMENT_MAGIC,
            _pad_magic: 0,
            sequence,
            lsn_start,
            record_count: 0,
            bytes_used: 0,
            timestamp_ns,
            crc32c: 0,
            _reserved: [0; 20],
        };
        hdr.update_crc();
        hdr
    }

    /// Recompute CRC.
    pub fn update_crc(&mut self) {
        self.crc32c = 0;
        let bytes = unsafe {
            core::slice::from_raw_parts(self as *const _ as *const u8, 40)
        };
        self.crc32c = crc32c(bytes);
    }

    /// Verify CRC.
    pub fn verify_crc(&self) -> bool {
        let mut copy = *self;
        copy.crc32c = 0;
        let bytes = unsafe {
            core::slice::from_raw_parts(&copy as *const _ as *const u8, 40)
        };
        crc32c(bytes) == self.crc32c
    }

    /// Check magic and CRC.
    pub fn is_valid(&self) -> bool {
        self.magic == LOG_SEGMENT_MAGIC && self.verify_crc()
    }
}
