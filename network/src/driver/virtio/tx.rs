//! VirtIO TX logic.
//!
//! Fire-and-forget transmit - never wait for completion!
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §4.6

use crate::dma::BufferPool;
use crate::types::{VirtqueueState, VirtioNetHdr};
use crate::driver::traits::TxError;

/// Maximum frame size including VirtIO header.
pub const MAX_TX_FRAME_SIZE: usize = VirtioNetHdr::SIZE + 1514;

/// Transmit a packet via VirtIO.
///
/// # Arguments
/// - `tx_state`: TX virtqueue state
/// - `tx_pool`: TX buffer pool
/// - `frame`: Ethernet frame (without VirtIO header)
///
/// # Returns
/// - `Ok(())`: Frame queued (fire-and-forget)
/// - `Err(TxError)`: Transmission failed
///
/// # Contract
/// - MUST return immediately (no completion wait)
/// - Caller should call `collect_tx_completions` periodically
#[cfg(target_arch = "x86_64")]
pub fn transmit(
    tx_state: &mut VirtqueueState,
    tx_pool: &mut BufferPool,
    frame: &[u8],
) -> Result<(), TxError> {
    use crate::asm::drivers::virtio::{tx as asm_tx, notify};
    
    // Check frame size
    let total_len = VirtioNetHdr::SIZE + frame.len();
    if total_len > MAX_TX_FRAME_SIZE {
        return Err(TxError::FrameTooLarge);
    }
    
    // Collect any pending completions first (reclaim buffers)
    collect_completions(tx_state, tx_pool);
    
    // Allocate TX buffer
    let buf = tx_pool.alloc().ok_or(TxError::QueueFull)?;
    let buf_idx = buf.index();
    
    // Write VirtIO header (12 bytes, all zeros)
    let hdr = VirtioNetHdr::zeroed();
    buf.as_mut_slice()[..VirtioNetHdr::SIZE].copy_from_slice(hdr.as_bytes());
    
    // Copy frame after header
    buf.as_mut_slice()[VirtioNetHdr::SIZE..total_len].copy_from_slice(frame);
    
    // Mark device-owned BEFORE submit
    unsafe { buf.mark_device_owned(); }
    
    // Submit via ASM (includes barriers)
    let success = asm_tx::submit(tx_state, buf_idx, total_len as u16);
    
    if !success {
        // Queue was full (shouldn't happen after collect, but handle it)
        if let Some(buf) = tx_pool.get_mut(buf_idx) {
            unsafe { buf.mark_driver_owned(); }
        }
        tx_pool.free(buf_idx);
        return Err(TxError::QueueFull);
    }
    
    // Notify device
    unsafe { notify::notify(tx_state); }
    
    // *** DO NOT WAIT FOR COMPLETION ***
    // Completion collected in main loop Phase 5
    
    Ok(())
}

/// Collect TX completions.
///
/// Call periodically (main loop Phase 5) to reclaim TX buffers.
#[cfg(target_arch = "x86_64")]
pub fn collect_completions(
    tx_state: &mut VirtqueueState,
    tx_pool: &mut BufferPool,
) {
    use crate::asm::drivers::virtio::tx as asm_tx;
    
    loop {
        let idx = asm_tx::poll_complete(tx_state);
        match idx {
            Some(buf_idx) => {
                // Return buffer to pool
                if let Some(buf) = tx_pool.get_mut(buf_idx) {
                    unsafe { buf.mark_driver_owned(); }
                    tx_pool.free(buf_idx);
                }
            }
            None => break, // No more completions
        }
    }
}

// Stubs for non-x86_64 platforms
#[cfg(not(target_arch = "x86_64"))]
pub fn transmit(
    _tx_state: &mut VirtqueueState,
    _tx_pool: &mut BufferPool,
    _frame: &[u8],
) -> Result<(), TxError> {
    Err(TxError::DeviceNotReady)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn collect_completions(
    _tx_state: &mut VirtqueueState,
    _tx_pool: &mut BufferPool,
) {}
