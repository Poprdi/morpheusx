//! Append-only circular log of [`LogRecord`]s in fixed 1 MB segments.
//!
//! Invariants:
//! - Records are immutable until the segment is recycled by the compactor.
//! - Superblock `committed_lsn` only advances after a flush.
//! - Recovery scans forward from `checkpoint_lsn`, validates each CRC, stops at first failure.

pub mod recovery;
pub mod segment;

use crate::crc::{crc32c, crc32c_two, crc64};
use crate::error::HelixError;
use crate::types::*;
use alloc::vec;
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

pub struct LogEngine {
    /// Partition-relative; log region start.
    log_start_block: u64,
    /// Partition-relative; log region end inclusive.
    log_end_block: u64,
    segment_count: u64,
    /// Circular index.
    head_segment: u64,
    /// Byte offset within head segment.
    head_offset: u32,
    /// Oldest live segment.
    tail_segment: u64,
    next_lsn: Lsn,
    write_buf: Vec<u8>,
    record_count: u32,
    partition_lba_start: u64,
    device_block_size: u32,
}

impl LogEngine {
    pub fn new(sb: &HelixSuperblock, partition_lba_start: u64, device_block_size: u32) -> Self {
        Self {
            log_start_block: sb.log_start_block,
            log_end_block: sb.log_end_block,
            segment_count: sb.log_segment_count,
            head_segment: sb.log_head_segment,
            head_offset: sb.log_head_offset,
            tail_segment: sb.log_tail_segment,
            next_lsn: sb.committed_lsn + 1,
            write_buf: vec![0u8; LOG_SEGMENT_BYTES as usize],
            record_count: 0,
            partition_lba_start,
            device_block_size,
        }
    }

    pub fn next_lsn(&self) -> Lsn {
        self.next_lsn
    }

    pub fn head_segment(&self) -> u64 {
        self.head_segment
    }

    pub fn segment_count(&self) -> u64 {
        self.segment_count
    }

    pub fn head_offset(&self) -> u32 {
        self.head_offset
    }

    pub fn tail_segment(&self) -> u64 {
        self.tail_segment
    }

    /// Recycle the ring once a checkpoint has captured all live state: the
    /// current head becomes the tail, freeing every superseded segment.
    pub fn reset_ring(&mut self) {
        self.tail_segment = self.head_segment;
    }

    /// Live segments (tail..=head) / total, percent. 100 = log full; warn near 80 for GC.
    pub fn log_utilization_pct(&self) -> u32 {
        let n = self.segment_count.max(1);
        let used = if self.head_segment >= self.tail_segment {
            self.head_segment - self.tail_segment + 1
        } else {
            n - self.tail_segment + self.head_segment + 1
        };
        ((used * 100) / n) as u32
    }

    fn segment_remaining(&self) -> u32 {
        LOG_SEGMENT_BYTES as u32 - self.head_offset
    }

    fn segment_to_block(&self, seg_idx: u64) -> u64 {
        self.log_start_block + seg_idx * LOG_SEGMENT_BLOCKS
    }

    fn abs_lba(&self, partition_block: u64) -> Lba {
        let blocks_per_sector = self.device_block_size as u64 / 512;
        if blocks_per_sector == 0 {
            Lba(self.partition_lba_start + partition_block * (BLOCK_SIZE as u64 / 512))
        } else {
            Lba(self.partition_lba_start
                + partition_block * (BLOCK_SIZE as u64 / self.device_block_size as u64))
        }
    }

    /// Append to write buffer; returns assigned LSN. Call `flush` to persist.
    /// `block_io` is needed only to persist a filled segment when one fills.
    pub fn append<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        op: LogOp,
        path_hash: u64,
        payload: &[u8],
        timestamp_ns: u64,
    ) -> Result<Lsn, HelixError> {
        self.append_full(block_io, op, 0, path_hash, 0, 0, payload, timestamp_ns)
    }

    /// Append with header `flags` + secondary_hash + tx_begin_lsn (extent, rename, tx ops).
    #[allow(clippy::too_many_arguments)]
    pub fn append_full<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        op: LogOp,
        flags: u8,
        path_hash: u64,
        secondary_hash: u64,
        tx_begin_lsn: Lsn,
        payload: &[u8],
        timestamp_ns: u64,
    ) -> Result<Lsn, HelixError> {
        let lsn = self.next_lsn;

        let payload_crc = if payload.is_empty() {
            0
        } else {
            crc64(payload)
        };

        let mut header = LogRecordHeader {
            lsn,
            timestamp_ns,
            op: op as u8,
            flags,
            _pad: [0; 2],
            payload_len: payload.len() as u32,
            path_hash,
            payload_crc64: payload_crc,
            secondary_hash,
            tx_begin_lsn,
            record_crc32c: 0,
            _reserved: 0,
        };

        let total_size = header.total_size() as u32;

        // Advance to next segment if current full; bail if head would lap tail.
        if self.head_offset + total_size > LOG_SEGMENT_BYTES as u32 {
            let next_seg = (self.head_segment + 1) % self.segment_count;
            if next_seg == self.tail_segment {
                return Err(HelixError::LogFull);
            }
            // The one-segment write buffer is about to be reused; persist the
            // filled segment first or its records are lost (the next flush only
            // writes the new head segment).
            self.flush(block_io)?;
            self.head_segment = next_seg;
            self.head_offset = core::mem::size_of::<LogSegmentHeader>() as u32;
            self.record_count = 0;
            for b in self.write_buf.iter_mut() {
                *b = 0;
            }
            let seg_hdr = LogSegmentHeader {
                magic: LOG_SEGMENT_MAGIC,
                _pad_magic: 0,
                sequence: self.head_segment,
                lsn_start: lsn,
                record_count: 0,
                bytes_used: 0,
                timestamp_ns,
                crc32c: 0,
                _reserved: [0; 20],
            };
            let hdr_bytes = unsafe {
                core::slice::from_raw_parts(
                    &seg_hdr as *const _ as *const u8,
                    core::mem::size_of::<LogSegmentHeader>(),
                )
            };
            self.write_buf[..hdr_bytes.len()].copy_from_slice(hdr_bytes);
        }

        // CRC over header (with record_crc32c=0) + payload.
        let hdr_bytes = unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                core::mem::size_of::<LogRecordHeader>(),
            )
        };
        header.record_crc32c = crc32c_two(hdr_bytes, payload);

        let off = self.head_offset as usize;
        let hdr_size = core::mem::size_of::<LogRecordHeader>();
        let hdr_bytes_final =
            unsafe { core::slice::from_raw_parts(&header as *const _ as *const u8, hdr_size) };
        self.write_buf[off..off + hdr_size].copy_from_slice(hdr_bytes_final);

        if !payload.is_empty() {
            let payload_off = off + hdr_size;
            self.write_buf[payload_off..payload_off + payload.len()].copy_from_slice(payload);
        }

        self.head_offset += total_size;
        self.record_count += 1;
        self.next_lsn += 1;

        Ok(lsn)
    }

    /// Persist the current write buffer; returns highest committed LSN.
    pub fn flush<B: BlockIo>(&mut self, block_io: &mut B) -> Result<Lsn, HelixError> {
        let seg_hdr_size = core::mem::size_of::<LogSegmentHeader>();
        if self.head_offset > seg_hdr_size as u32 {
            // LogSegmentHeader field offsets: record_count @24, bytes_used @28,
            // crc32c @40. CRC covers [0..40) (the canonical verify range).
            self.write_buf[24..28].copy_from_slice(&self.record_count.to_le_bytes());
            let used = self.head_offset - seg_hdr_size as u32;
            self.write_buf[28..32].copy_from_slice(&used.to_le_bytes());

            self.write_buf[40..44].copy_from_slice(&[0; 4]);
            let header_crc = crc32c(&self.write_buf[..40]);
            self.write_buf[40..44].copy_from_slice(&header_crc.to_le_bytes());
        }

        let blocks_used = (self.head_offset as u64).div_ceil(BLOCK_SIZE as u64);
        let seg_start = self.segment_to_block(self.head_segment);

        for i in 0..blocks_used {
            let block_off = (i * BLOCK_SIZE as u64) as usize;
            let block_end = block_off + BLOCK_SIZE as usize;
            let data = &self.write_buf[block_off..block_end];
            let lba = self.abs_lba(seg_start + i);
            block_io
                .write_blocks(lba, data)
                .map_err(|_| HelixError::IoWriteFailed)?;
        }

        block_io.flush().map_err(|_| HelixError::IoFlushFailed)?;

        Ok(self.next_lsn - 1)
    }

    pub fn read_record<B: BlockIo>(
        &self,
        block_io: &mut B,
        segment_idx: u64,
        byte_offset: u32,
    ) -> Result<(LogRecordHeader, Vec<u8>), HelixError> {
        let seg_block = self.segment_to_block(segment_idx);
        let block_idx = byte_offset as u64 / BLOCK_SIZE as u64;
        let block_off = byte_offset as usize % BLOCK_SIZE as usize;

        let mut buf = vec![0u8; BLOCK_SIZE as usize];
        let lba = self.abs_lba(seg_block + block_idx);
        block_io
            .read_blocks(lba, &mut buf)
            .map_err(|_| HelixError::IoReadFailed)?;

        // Header may straddle a block boundary.
        let hdr_size = core::mem::size_of::<LogRecordHeader>();
        let header: LogRecordHeader = if block_off + hdr_size <= BLOCK_SIZE as usize {
            unsafe {
                core::ptr::read_unaligned(buf[block_off..].as_ptr() as *const LogRecordHeader)
            }
        } else {
            let first_part = BLOCK_SIZE as usize - block_off;
            let mut tmp = [0u8; 64];
            tmp[..first_part].copy_from_slice(&buf[block_off..]);
            let mut buf2 = vec![0u8; BLOCK_SIZE as usize];
            let lba2 = self.abs_lba(seg_block + block_idx + 1);
            block_io
                .read_blocks(lba2, &mut buf2)
                .map_err(|_| HelixError::IoReadFailed)?;
            tmp[first_part..hdr_size].copy_from_slice(&buf2[..hdr_size - first_part]);
            unsafe { core::ptr::read_unaligned(tmp.as_ptr() as *const LogRecordHeader) }
        };

        if LogOp::from_u8(header.op).is_none() {
            return Err(HelixError::LogCrcMismatch);
        }

        // payload_len is attacker/corruption-controlled; a record cannot extend
        // past its segment, so reject before allocating anything.
        let payload_len = header.payload_len as usize;
        let max_payload =
            (LOG_SEGMENT_BYTES as usize).saturating_sub(byte_offset as usize + hdr_size);
        if payload_len > max_payload {
            return Err(HelixError::LogCrcMismatch);
        }
        let mut payload = vec![0u8; payload_len];
        if payload_len > 0 {
            let payload_start = byte_offset as usize + hdr_size;
            let mut read = 0;
            while read < payload_len {
                let abs_off = payload_start + read;
                let blk = abs_off / BLOCK_SIZE as usize;
                let off_in_blk = abs_off % BLOCK_SIZE as usize;
                let chunk = (BLOCK_SIZE as usize - off_in_blk).min(payload_len - read);

                let mut blk_buf = vec![0u8; BLOCK_SIZE as usize];
                let lba = self.abs_lba(seg_block + blk as u64);
                block_io
                    .read_blocks(lba, &mut blk_buf)
                    .map_err(|_| HelixError::IoReadFailed)?;
                payload[read..read + chunk]
                    .copy_from_slice(&blk_buf[off_in_blk..off_in_blk + chunk]);
                read += chunk;
            }
        }

        let mut crc_buf = Vec::with_capacity(hdr_size + payload_len);
        let mut hdr_copy = header;
        hdr_copy.record_crc32c = 0;
        let hdr_bytes =
            unsafe { core::slice::from_raw_parts(&hdr_copy as *const _ as *const u8, hdr_size) };
        crc_buf.extend_from_slice(hdr_bytes);
        crc_buf.extend_from_slice(&payload);
        let computed_crc = crc32c(&crc_buf);
        if computed_crc != header.record_crc32c {
            return Err(HelixError::LogCrcMismatch);
        }

        Ok((header, payload))
    }

    /// Load the head segment into the write buffer. MUST be called after
    /// `new()` on an existing volume — otherwise the next `flush()` clobbers
    /// the head segment with zeros.
    pub fn reload_head_segment<B: BlockIo>(&mut self, block_io: &mut B) -> Result<(), HelixError> {
        let seg_start = self.segment_to_block(self.head_segment);
        let blocks_to_read = (self.head_offset as u64).div_ceil(BLOCK_SIZE as u64);
        // Always read at least the segment header block.
        let blocks_to_read = blocks_to_read.clamp(1, LOG_SEGMENT_BLOCKS);

        for i in 0..blocks_to_read {
            let off = (i * BLOCK_SIZE as u64) as usize;
            let lba = self.abs_lba(seg_start + i);
            block_io
                .read_blocks(lba, &mut self.write_buf[off..off + BLOCK_SIZE as usize])
                .map_err(|_| HelixError::IoReadFailed)?;
        }

        // Rebuild record_count from what's already in the buffer.
        let mut offset = core::mem::size_of::<LogSegmentHeader>() as u32;
        let mut count = 0u32;
        while offset < self.head_offset {
            let hdr_size = core::mem::size_of::<LogRecordHeader>();
            if (offset as usize) + hdr_size > self.write_buf.len() {
                break;
            }
            let hdr: LogRecordHeader = unsafe {
                core::ptr::read_unaligned(
                    self.write_buf[offset as usize..].as_ptr() as *const LogRecordHeader
                )
            };
            if LogOp::from_u8(hdr.op).is_none() {
                break;
            }
            count += 1;
            offset += hdr.total_size() as u32;
        }
        self.record_count = count;

        Ok(())
    }

    /// Scan tail..=head, calling `visitor` per valid record. Stops on first
    /// CRC failure or at the head write position. Returns highest valid LSN.
    /// Reads whole segments; head segment reuses the loaded `write_buf`.
    pub fn scan_forward<B: BlockIo, F>(
        &self,
        block_io: &mut B,
        start_segment: u64,
        start_offset: u32,
        mut visitor: F,
    ) -> Result<Lsn, HelixError>
    where
        F: FnMut(&LogRecordHeader, &[u8]) -> Result<(), HelixError>,
    {
        let hdr_size = core::mem::size_of::<LogRecordHeader>();
        let seg_hdr_size = core::mem::size_of::<LogSegmentHeader>() as u32;
        let mut highest_lsn: Lsn = 0;
        let mut seg = start_segment;

        // Lazily allocated buffer for reading non-head segments.
        let mut seg_buf: Option<Vec<u8>> = None;

        loop {
            let is_head = seg == self.head_segment;
            let limit = if is_head {
                self.head_offset
            } else {
                LOG_SEGMENT_BYTES as u32
            };
            let first_offset = if seg == start_segment {
                start_offset
            } else {
                seg_hdr_size
            };

            if first_offset >= limit {
                if is_head {
                    break;
                }
                seg = (seg + 1) % self.segment_count;
                continue;
            }

            let buf: &[u8] = if is_head {
                &self.write_buf
            } else {
                let b = seg_buf.get_or_insert_with(|| vec![0u8; LOG_SEGMENT_BYTES as usize]);
                let seg_start = self.segment_to_block(seg);
                let blocks = (limit as u64).div_ceil(BLOCK_SIZE as u64);
                for i in 0..blocks {
                    let off = (i * BLOCK_SIZE as u64) as usize;
                    let lba = self.abs_lba(seg_start + i);
                    block_io
                        .read_blocks(lba, &mut b[off..off + BLOCK_SIZE as usize])
                        .map_err(|_| HelixError::IoReadFailed)?;
                }
                b
            };

            let mut offset = first_offset;
            loop {
                if (offset as usize) + hdr_size > limit as usize {
                    break;
                }

                let off = offset as usize;
                let header: LogRecordHeader = unsafe {
                    core::ptr::read_unaligned(buf[off..].as_ptr() as *const LogRecordHeader)
                };

                // Invalid op = end of valid records in this segment.
                if LogOp::from_u8(header.op).is_none() {
                    break;
                }

                let total = header.total_size() as u32;
                let payload_len = header.payload_len as usize;
                let payload_start = off + hdr_size;
                let payload_end = payload_start + payload_len;

                if payload_end > buf.len() || offset + total > LOG_SEGMENT_BYTES as u32 {
                    break;
                }

                let payload = &buf[payload_start..payload_end];

                let mut hdr_copy = header;
                hdr_copy.record_crc32c = 0;
                let hdr_bytes = unsafe {
                    core::slice::from_raw_parts(&hdr_copy as *const _ as *const u8, hdr_size)
                };
                let computed = crc32c_two(hdr_bytes, payload);
                if computed != header.record_crc32c {
                    break;
                }

                highest_lsn = header.lsn;
                visitor(&header, payload)?;
                offset += total;
            }

            if is_head {
                break;
            }
            seg = (seg + 1) % self.segment_count;
        }

        Ok(highest_lsn)
    }
}
