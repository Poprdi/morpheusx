//! Buffered disk writer for streaming ISO downloads.
//!
//! Accumulates data in a static buffer and flushes to disk in
//! sector-aligned chunks. Works with both VirtIO-blk and AHCI.

use crate::device::UnifiedBlockDevice;
use crate::driver::block_traits::BlockDriver;
use crate::mainloop::serial;

/// Write buffer size: 64KB = 128 sectors.
const BUFFER_SIZE: usize = 64 * 1024;

/// Static buffer for accumulating data before disk write.
static mut WRITE_BUFFER: [u8; BUFFER_SIZE] = [0u8; BUFFER_SIZE];

/// Current fill level of write buffer.
static mut BUFFER_FILL: usize = 0;

/// Next sector to write to.
static mut NEXT_SECTOR: u64 = 0;

/// Total bytes written to disk.
static mut TOTAL_WRITTEN: u64 = 0;

/// Next request ID for block driver.
static mut NEXT_REQUEST_ID: u32 = 1;

/// Disk writer state.
pub struct DiskWriter {
    start_sector: u64,
    enabled: bool,
}

impl DiskWriter {
    /// Create a new disk writer starting at the given sector.
    pub fn new(start_sector: u64) -> Self {
        unsafe {
            BUFFER_FILL = 0;
            NEXT_SECTOR = start_sector;
            TOTAL_WRITTEN = 0;
            NEXT_REQUEST_ID = 1;
        }
        Self {
            start_sector,
            enabled: true,
        }
    }

    /// Create a disabled disk writer (for download-only mode).
    pub fn disabled() -> Self {
        Self {
            start_sector: 0,
            enabled: false,
        }
    }

    /// Check if disk writing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get total bytes written to disk.
    pub fn bytes_written(&self) -> u64 {
        unsafe { TOTAL_WRITTEN }
    }

    /// Get current sector position.
    pub fn current_sector(&self) -> u64 {
        unsafe { NEXT_SECTOR }
    }

    /// Write data to disk (buffered).
    ///
    /// Data is accumulated in an internal buffer and flushed to disk
    /// when the buffer is full. Returns number of bytes consumed.
    pub fn write(&mut self, blk: &mut UnifiedBlockDevice, data: &[u8]) -> usize {
        if !self.enabled {
            return data.len(); // Pretend we wrote it
        }
        unsafe { buffer_write(blk, data) }
    }

    /// Flush any remaining buffered data to disk.
    ///
    /// Must be called at end of download to write partial buffer.
    pub fn flush(&mut self, blk: &mut UnifiedBlockDevice) -> bool {
        if !self.enabled {
            return true;
        }
        unsafe { flush_remaining(blk) }
    }
}

/// Flush the write buffer to disk.
unsafe fn flush_buffer(blk: &mut UnifiedBlockDevice) -> usize {
    if BUFFER_FILL == 0 {
        return 0;
    }

    let bytes_to_write = BUFFER_FILL;
    let num_sectors = ((bytes_to_write + 511) / 512) as u32;

    // Identity mapped post-EBS, so virtual == physical
    let buffer_phys = (&raw const WRITE_BUFFER).cast::<u8>() as u64;

    let request_id = NEXT_REQUEST_ID;
    NEXT_REQUEST_ID = NEXT_REQUEST_ID.wrapping_add(1);

    // Drain pending completions
    while let Some(_) = blk.poll_completion() {}

    if !blk.can_submit() {
        serial::println("[DISK] ERROR: Queue full");
        return 0;
    }

    if blk.submit_write(NEXT_SECTOR, buffer_phys, num_sectors, request_id).is_err() {
        serial::print("[DISK] ERROR: Submit failed at sector ");
        serial::print_hex(NEXT_SECTOR);
        serial::println("");
        return 0;
    }

    blk.notify();

    // Poll for completion with timeout
    let start_tsc = read_tsc();
    let timeout: u64 = 4_000_000_000; // ~1s at 4GHz

    loop {
        if let Some(completion) = blk.poll_completion() {
            if completion.request_id == request_id {
                if completion.status == 0 {
                    NEXT_SECTOR += num_sectors as u64;
                    TOTAL_WRITTEN += bytes_to_write as u64;
                    BUFFER_FILL = 0;
                    return bytes_to_write;
                } else {
                    serial::print("[DISK] ERROR: Status ");
                    serial::print_u32(completion.status as u32);
                    serial::println("");
                    return 0;
                }
            }
        }

        if read_tsc().wrapping_sub(start_tsc) > timeout {
            serial::println("[DISK] ERROR: Timeout");
            return 0;
        }

        core::hint::spin_loop();
    }
}

/// Buffer data and flush when full.
unsafe fn buffer_write(blk: &mut UnifiedBlockDevice, data: &[u8]) -> usize {
    let mut consumed = 0;
    let mut remaining = data;

    while !remaining.is_empty() {
        let space = BUFFER_SIZE - BUFFER_FILL;
        let to_copy = remaining.len().min(space);

        let dst = BUFFER_FILL;
        WRITE_BUFFER[dst..dst + to_copy].copy_from_slice(&remaining[..to_copy]);
        BUFFER_FILL += to_copy;
        consumed += to_copy;
        remaining = &remaining[to_copy..];

        if BUFFER_FILL >= BUFFER_SIZE {
            if flush_buffer(blk) == 0 {
                break;
            }
        }
    }

    consumed
}

/// Flush remaining data (pad with zeros for sector alignment).
unsafe fn flush_remaining(blk: &mut UnifiedBlockDevice) -> bool {
    if BUFFER_FILL == 0 {
        return true;
    }

    // Zero-pad to sector boundary
    for i in BUFFER_FILL..BUFFER_SIZE {
        WRITE_BUFFER[i] = 0;
    }

    flush_buffer(blk) > 0
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem));
    }
    ((hi as u64) << 32) | (lo as u64)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn read_tsc() -> u64 {
    0
}
