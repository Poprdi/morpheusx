//! Buffered disk writer for streaming ISO downloads. Accumulates into a static
//! buffer and flushes in sector-aligned chunks (VirtIO-blk and AHCI).

use crate::mainloop::serial;
use morpheus_block::block_traits::BlockDriver;
use morpheus_block::device::UnifiedBlockDevice;

/// 64 KB = 128 sectors.
const BUFFER_SIZE: usize = 64 * 1024;

static mut WRITE_BUFFER: [u8; BUFFER_SIZE] = [0u8; BUFFER_SIZE];
static mut BUFFER_FILL: usize = 0;
static mut NEXT_SECTOR: u64 = 0;
static mut TOTAL_WRITTEN: u64 = 0;
static mut NEXT_REQUEST_ID: u32 = 1;

pub struct DiskWriter {
    start_sector: u64,
    enabled: bool,
}

impl DiskWriter {
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

    /// Download-only mode: writes become no-ops.
    pub fn disabled() -> Self {
        Self {
            start_sector: 0,
            enabled: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn bytes_written(&self) -> u64 {
        unsafe { TOTAL_WRITTEN }
    }

    pub fn current_sector(&self) -> u64 {
        unsafe { NEXT_SECTOR }
    }

    /// Buffer data, flushing when full. Returns bytes consumed.
    pub fn write(&mut self, blk: &mut UnifiedBlockDevice, data: &[u8]) -> usize {
        if !self.enabled {
            return data.len();
        }
        unsafe { buffer_write(blk, data) }
    }

    /// Flush the partial buffer; call once at end of download.
    pub fn flush(&mut self, blk: &mut UnifiedBlockDevice) -> bool {
        if !self.enabled {
            return true;
        }
        unsafe { flush_remaining(blk) }
    }
}

unsafe fn flush_buffer(blk: &mut UnifiedBlockDevice) -> usize {
    if BUFFER_FILL == 0 {
        return 0;
    }

    let bytes_to_write = BUFFER_FILL;
    let num_sectors = bytes_to_write.div_ceil(512) as u32;

    // Identity-mapped post-EBS: virtual == physical.
    let buffer_phys = (&raw const WRITE_BUFFER).cast::<u8>() as u64;

    let request_id = NEXT_REQUEST_ID;
    NEXT_REQUEST_ID = NEXT_REQUEST_ID.wrapping_add(1);

    while blk.poll_completion().is_some() {}

    if !blk.can_submit() {
        serial::println("[DISK] ERROR: Queue full");
        return 0;
    }

    if blk
        .submit_write(NEXT_SECTOR, buffer_phys, num_sectors, request_id)
        .is_err()
    {
        serial::print("[DISK] ERROR: Submit failed at sector ");
        serial::print_hex(NEXT_SECTOR);
        serial::println("");
        return 0;
    }

    blk.notify();

    let start_tsc = read_tsc();
    let timeout: u64 = 4_000_000_000; // ~1s at 4 GHz

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

        if BUFFER_FILL >= BUFFER_SIZE && flush_buffer(blk) == 0 {
            break;
        }
    }

    consumed
}

/// Zero-pad to a sector boundary, then flush.
unsafe fn flush_remaining(blk: &mut UnifiedBlockDevice) -> bool {
    if BUFFER_FILL == 0 {
        return true;
    }

    for item in WRITE_BUFFER.iter_mut().skip(BUFFER_FILL) {
        *item = 0;
    }

    flush_buffer(blk) > 0
}

use morpheus_hal_x86_64::asm::tsc::read_tsc;
