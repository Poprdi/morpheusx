//! VirtIO RX logic.
//!
//! Poll-based receive - never block!
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §4.7

use crate::dma::BufferPool;
use crate::driver::traits::RxError;
use crate::types::{RxResult, VirtioNetHdr, VirtqueueState};

/// Receive a packet via VirtIO.
///
/// # Arguments
/// - `rx_state`: RX virtqueue state
/// - `rx_pool`: RX buffer pool
/// - `out_buffer`: Buffer to copy received frame into
///
/// # Returns
/// - `Ok(Some(len))`: Frame received, `len` bytes copied (without VirtIO header)
/// - `Ok(None)`: No frame available (normal)
/// - `Err(RxError)`: Receive error
///
/// # Contract
/// - MUST return immediately (no blocking)
#[cfg(target_arch = "x86_64")]
pub fn receive(
    rx_state: &mut VirtqueueState,
    rx_pool: &mut BufferPool,
    out_buffer: &mut [u8],
) -> Result<Option<usize>, RxError> {
    use crate::asm::drivers::virtio::rx as asm_rx;

    // Poll via ASM (includes barriers)
    let rx_result = asm_rx::poll(rx_state);

    let result = match rx_result {
        Some(r) => r,
        None => return Ok(None), // No packet available
    };

    // Get buffer (now driver-owned)
    let buf = rx_pool
        .get_mut(result.buffer_idx)
        .ok_or(RxError::DeviceError)?;
    unsafe {
        buf.mark_driver_owned();
    }

    // Calculate frame length (skip 12-byte VirtIO header)
    let frame_len = result.length as usize;
    if frame_len < VirtioNetHdr::SIZE {
        // Invalid - resubmit and report error
        resubmit_buffer(rx_state, rx_pool, result.buffer_idx);
        return Err(RxError::DeviceError);
    }

    let payload_len = frame_len - VirtioNetHdr::SIZE;

    // Check if caller's buffer is large enough
    if payload_len > out_buffer.len() {
        // Frame too large - resubmit our buffer but return error
        resubmit_buffer(rx_state, rx_pool, result.buffer_idx);
        return Err(RxError::BufferTooSmall {
            needed: payload_len,
        });
    }

    // Copy frame (skip VirtIO header)
    out_buffer[..payload_len]
        .copy_from_slice(&buf.as_slice()[VirtioNetHdr::SIZE..VirtioNetHdr::SIZE + payload_len]);

    // Resubmit buffer to RX queue
    resubmit_buffer(rx_state, rx_pool, result.buffer_idx);

    Ok(Some(payload_len))
}

/// Resubmit RX buffer after processing.
///
/// Notifies device immediately to ensure packets keep flowing.
#[cfg(target_arch = "x86_64")]
fn resubmit_buffer(rx_state: &mut VirtqueueState, rx_pool: &mut BufferPool, buf_idx: u16) {
    use crate::asm::drivers::virtio::{notify, rx as asm_rx};

    if let Some(buf) = rx_pool.get_mut(buf_idx) {
        // Mark device-owned before submit
        unsafe {
            buf.mark_device_owned();
        }

        let capacity = buf.capacity() as u16;
        let success = asm_rx::submit(rx_state, buf_idx, capacity);

        if !success {
            // Queue full - this shouldn't happen with proper sizing
            unsafe {
                buf.mark_driver_owned();
            }
            // Buffer will be reclaimed on next refill cycle
        } else {
            // Notify device that buffer is available
            notify::notify(rx_state);
        }
    }
}

/// Refill RX queue with available buffers.
///
/// Call in main loop Phase 1.
#[cfg(target_arch = "x86_64")]
pub fn refill_queue(rx_state: &mut VirtqueueState, rx_pool: &mut BufferPool) {
    use crate::asm::drivers::virtio::{notify, rx as asm_rx};

    let mut submitted = 0;

    while let Some(buf) = rx_pool.alloc() {
        let buf_idx = buf.index();
        let capacity = buf.capacity() as u16;

        // Mark device-owned before submit
        unsafe {
            buf.mark_device_owned();
        }

        let success = asm_rx::submit(rx_state, buf_idx, capacity);

        if !success {
            // Queue full - return buffer and stop
            unsafe {
                buf.mark_driver_owned();
            }
            rx_pool.free(buf_idx);
            break;
        }

        submitted += 1;
    }

    // Notify device if we submitted any buffers
    if submitted > 0 {
        notify::notify(rx_state);
    }
}

/// Pre-fill RX queue during initialization.
///
/// Should be called after queue setup, before DRIVER_OK.
#[cfg(target_arch = "x86_64")]
pub fn prefill_queue(
    rx_state: &mut VirtqueueState,
    rx_pool: &mut BufferPool,
) -> Result<usize, RxError> {
    use crate::asm::drivers::virtio::{notify, rx as asm_rx};

    let mut filled = 0;

    while let Some(buf) = rx_pool.alloc() {
        let buf_idx = buf.index();
        let capacity = buf.capacity() as u16;

        // Mark device-owned before submit
        unsafe {
            buf.mark_device_owned();
        }

        let success = asm_rx::submit(rx_state, buf_idx, capacity);

        if !success {
            // Queue full - return buffer and stop
            unsafe {
                buf.mark_driver_owned();
            }
            rx_pool.free(buf_idx);
            break;
        }

        filled += 1;
    }

    // Notify device
    if filled > 0 {
        notify::notify(rx_state);
    }

    Ok(filled)
}

// Stubs for non-x86_64 platforms
#[cfg(not(target_arch = "x86_64"))]
pub fn receive(
    _rx_state: &mut VirtqueueState,
    _rx_pool: &mut BufferPool,
    _out_buffer: &mut [u8],
) -> Result<Option<usize>, RxError> {
    Ok(None)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn refill_queue(_rx_state: &mut VirtqueueState, _rx_pool: &mut BufferPool) {}

#[cfg(not(target_arch = "x86_64"))]
pub fn prefill_queue(
    _rx_state: &mut VirtqueueState,
    _rx_pool: &mut BufferPool,
) -> Result<usize, RxError> {
    Ok(0)
}
