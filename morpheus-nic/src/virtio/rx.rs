//! VirtIO-net poll-based RX. Never blocks.

use super::VirtioNetHdr;
use crate::traits::RxError;
use morpheus_virtio::dma::BufferPool;
use morpheus_virtio::types::VirtqueueState;

#[cfg(target_arch = "x86_64")]
pub fn receive(
    rx_state: &mut VirtqueueState,
    rx_pool: &mut BufferPool,
    out_buffer: &mut [u8],
) -> Result<Option<usize>, RxError> {
    use morpheus_virtio::asm::rx as asm_rx;

    // Poll includes barriers.
    let rx_result = asm_rx::poll(rx_state);

    let result = match rx_result {
        Some(r) => r,
        None => return Ok(None),
    };

    let buf = rx_pool
        .get_mut(result.buffer_idx)
        .ok_or(RxError::DeviceError)?;
    unsafe {
        buf.mark_driver_owned();
    }

    let frame_len = result.length as usize;
    if frame_len < VirtioNetHdr::SIZE {
        resubmit_buffer(rx_state, rx_pool, result.buffer_idx);
        return Err(RxError::DeviceError);
    }

    let payload_len = frame_len - VirtioNetHdr::SIZE;

    if payload_len > out_buffer.len() {
        resubmit_buffer(rx_state, rx_pool, result.buffer_idx);
        return Err(RxError::BufferTooSmall {
            needed: payload_len,
        });
    }

    // Skip the 12-byte VirtIO header.
    out_buffer[..payload_len]
        .copy_from_slice(&buf.as_slice()[VirtioNetHdr::SIZE..VirtioNetHdr::SIZE + payload_len]);

    resubmit_buffer(rx_state, rx_pool, result.buffer_idx);

    Ok(Some(payload_len))
}

#[cfg(target_arch = "x86_64")]
fn resubmit_buffer(rx_state: &mut VirtqueueState, rx_pool: &mut BufferPool, buf_idx: u16) {
    use morpheus_virtio::asm::{notify, rx as asm_rx};

    if let Some(buf) = rx_pool.get_mut(buf_idx) {
        unsafe {
            buf.mark_device_owned();
        }

        let capacity = buf.capacity() as u16;
        let success = asm_rx::submit(rx_state, buf_idx, capacity);

        if !success {
            // Submit failed => device never took ownership; force back. Buffer
            // is reclaimed next refill.
            buf.force_driver_owned();
        } else {
            notify::notify(rx_state);
        }
    }
}

/// Mainloop Phase 1.
#[cfg(target_arch = "x86_64")]
pub fn refill_queue(rx_state: &mut VirtqueueState, rx_pool: &mut BufferPool) {
    use morpheus_virtio::asm::{notify, rx as asm_rx};

    let mut submitted = 0;

    while let Some(buf) = rx_pool.alloc() {
        let buf_idx = buf.index();
        let capacity = buf.capacity() as u16;

        unsafe {
            buf.mark_device_owned();
        }

        let success = asm_rx::submit(rx_state, buf_idx, capacity);

        if !success {
            // Submit failed => device never took ownership; force back and stop.
            buf.force_driver_owned();
            rx_pool.free(buf_idx);
            break;
        }

        submitted += 1;
    }

    if submitted > 0 {
        notify::notify(rx_state);
    }
}

/// Call after queue setup, before DRIVER_OK.
#[cfg(target_arch = "x86_64")]
pub fn prefill_queue(
    rx_state: &mut VirtqueueState,
    rx_pool: &mut BufferPool,
) -> Result<usize, RxError> {
    use morpheus_virtio::asm::{notify, rx as asm_rx};

    let mut filled = 0;

    while let Some(buf) = rx_pool.alloc() {
        let buf_idx = buf.index();
        let capacity = buf.capacity() as u16;

        unsafe {
            buf.mark_device_owned();
        }

        let success = asm_rx::submit(rx_state, buf_idx, capacity);

        if !success {
            // Submit failed => device never took ownership; force back and stop.
            buf.force_driver_owned();
            rx_pool.free(buf_idx);
            break;
        }

        filled += 1;
    }

    if filled > 0 {
        notify::notify(rx_state);
    }

    Ok(filled)
}

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
