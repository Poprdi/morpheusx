//! VirtIO-net fire-and-forget TX. Never waits for completion.

use super::VirtioNetHdr;
use crate::traits::TxError;
use morpheus_virtio::dma::BufferPool;
use morpheus_virtio::types::VirtqueueState;

/// Max frame size including the VirtIO header.
pub const MAX_TX_FRAME_SIZE: usize = VirtioNetHdr::SIZE + 1514;

/// Returns immediately; caller drains via `collect_completions` (Phase 5).
#[cfg(target_arch = "x86_64")]
pub fn transmit(
    tx_state: &mut VirtqueueState,
    tx_pool: &mut BufferPool,
    frame: &[u8],
) -> Result<(), TxError> {
    use morpheus_virtio::asm::tx as asm_tx;

    let total_len = VirtioNetHdr::SIZE + frame.len();
    if total_len > MAX_TX_FRAME_SIZE {
        return Err(TxError::FrameTooLarge);
    }

    // Reclaim buffers first.
    collect_completions(tx_state, tx_pool);

    let buf = tx_pool.alloc().ok_or(TxError::QueueFull)?;
    let buf_idx = buf.index();

    // Zeroed 12-byte VirtIO header, then the frame.
    let hdr = VirtioNetHdr::zeroed();
    buf.as_mut_slice()[..VirtioNetHdr::SIZE].copy_from_slice(hdr.as_bytes());
    buf.as_mut_slice()[VirtioNetHdr::SIZE..total_len].copy_from_slice(frame);

    unsafe {
        buf.mark_device_owned();
    }

    // Submit includes barriers.
    let success = asm_tx::submit(tx_state, buf_idx, total_len as u16);

    if !success {
        // Submit failed => device never took ownership; bypass the state machine
        // and force back (error recovery path).
        if let Some(buf) = tx_pool.get_mut(buf_idx) {
            buf.force_driver_owned();
        }
        tx_pool.free(buf_idx);
        return Err(TxError::QueueFull);
    }

    // Notify is deferred to collect_completions() (Phase 5) to batch throughput.

    Ok(())
}

/// Mainloop Phase 5: batched notify, then reap completed TX buffers.
#[cfg(target_arch = "x86_64")]
pub fn collect_completions(tx_state: &mut VirtqueueState, tx_pool: &mut BufferPool) {
    use morpheus_virtio::asm::{notify, tx as asm_tx};

    // Batched notify; device ignores redundant ones.
    notify::notify(tx_state);

    loop {
        let idx = asm_tx::poll_complete(tx_state);
        match idx {
            Some(buf_idx) => {
                if let Some(buf) = tx_pool.get_mut(buf_idx) {
                    unsafe {
                        buf.mark_driver_owned();
                    }
                    tx_pool.free(buf_idx);
                }
            },
            None => break,
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn transmit(
    _tx_state: &mut VirtqueueState,
    _tx_pool: &mut BufferPool,
    _frame: &[u8],
) -> Result<(), TxError> {
    Err(TxError::DeviceNotReady)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn collect_completions(_tx_state: &mut VirtqueueState, _tx_pool: &mut BufferPool) {}
