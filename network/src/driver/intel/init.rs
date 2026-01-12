//! Intel e1000e initialization sequence.
//!
//! Rust orchestration layer for device initialization.
//! All hardware access is via ASM bindings.
//!
//! # Initialization Sequence
//! 1. Disable interrupts
//! 2. Global reset
//! 3. Wait for reset completion
//! 4. Disable interrupts again (reset re-enables)
//! 5. Read MAC address
//! 6. Clear multicast table
//! 7. Setup RX descriptor ring
//! 8. Setup TX descriptor ring
//! 9. Enable RX
//! 10. Enable TX
//! 11. Set link up
//!
//! # Reference
//! Intel 82579 Datasheet, Section 14 (Initialization)

use crate::asm::drivers::intel::{
    asm_intel_clear_mta, asm_intel_disable_interrupts, asm_intel_enable_rx, asm_intel_enable_tx,
    asm_intel_read_mac, asm_intel_reset, asm_intel_set_link_up, asm_intel_setup_rx_ring,
    asm_intel_setup_tx_ring,
};
use crate::dma::DmaRegion;
use crate::types::MacAddress;

use super::regs;
use super::rx::RxRing;
use super::tx::TxRing;

// ═══════════════════════════════════════════════════════════════════════════
// CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// E1000e driver configuration.
#[derive(Debug, Clone)]
pub struct E1000eConfig {
    /// Number of RX descriptors.
    pub rx_queue_size: u16,
    /// Number of TX descriptors.
    pub tx_queue_size: u16,
    /// Size of each buffer.
    pub buffer_size: usize,
    /// TSC frequency (ticks per second) for timeouts.
    pub tsc_freq: u64,
    /// DMA region CPU base pointer.
    pub dma_cpu_base: *mut u8,
    /// DMA region bus address.
    pub dma_bus_base: u64,
}

impl E1000eConfig {
    /// Create configuration with default values.
    ///
    /// # Safety
    /// DMA pointers must be valid.
    pub unsafe fn new(dma_cpu_base: *mut u8, dma_bus_base: u64, tsc_freq: u64) -> Self {
        Self {
            rx_queue_size: regs::DEFAULT_QUEUE_SIZE,
            tx_queue_size: regs::DEFAULT_QUEUE_SIZE,
            buffer_size: regs::DEFAULT_BUFFER_SIZE,
            tsc_freq,
            dma_cpu_base,
            dma_bus_base,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// Initialization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E1000eInitError {
    /// Device reset timed out.
    ResetTimeout,
    /// MAC address invalid (all zeros or all ones).
    InvalidMac,
    /// MMIO access failed (device not responding).
    MmioError,
    /// Link did not come up.
    LinkTimeout,
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION RESULT
// ═══════════════════════════════════════════════════════════════════════════

/// Result of successful initialization.
pub struct E1000eInitResult {
    /// MAC address.
    pub mac: MacAddress,
    /// RX ring.
    pub rx_ring: RxRing,
    /// TX ring.
    pub tx_ring: TxRing,
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize the e1000e device.
///
/// # Arguments
/// - `mmio_base`: Device MMIO base address
/// - `config`: Driver configuration
///
/// # Returns
/// Initialization result with MAC and rings, or error.
///
/// # Safety
/// - `mmio_base` must be a valid, mapped MMIO address
/// - DMA region must be properly allocated
pub unsafe fn init_e1000e(
    mmio_base: u64,
    config: &E1000eConfig,
) -> Result<E1000eInitResult, E1000eInitError> {
    // ═══════════════════════════════════════════════════════════════════
    // STEP 1: DISABLE INTERRUPTS
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_disable_interrupts(mmio_base);

    // ═══════════════════════════════════════════════════════════════════
    // STEP 2-3: GLOBAL RESET + WAIT
    // ═══════════════════════════════════════════════════════════════════
    let reset_result = asm_intel_reset(mmio_base, config.tsc_freq);
    if reset_result != 0 {
        return Err(E1000eInitError::ResetTimeout);
    }

    // ═══════════════════════════════════════════════════════════════════
    // STEP 4: DISABLE INTERRUPTS AGAIN (reset re-enables them)
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_disable_interrupts(mmio_base);

    // ═══════════════════════════════════════════════════════════════════
    // STEP 5: READ MAC ADDRESS
    // ═══════════════════════════════════════════════════════════════════
    let mut mac: MacAddress = [0u8; 6];
    let mac_result = asm_intel_read_mac(mmio_base, &mut mac);
    if mac_result != 0 {
        return Err(E1000eInitError::InvalidMac);
    }

    // Validate MAC (not all zeros or all ones)
    if mac == [0, 0, 0, 0, 0, 0] || mac == [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF] {
        return Err(E1000eInitError::InvalidMac);
    }

    // ═══════════════════════════════════════════════════════════════════
    // STEP 6: CLEAR MULTICAST TABLE
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_clear_mta(mmio_base);

    // ═══════════════════════════════════════════════════════════════════
    // STEP 7: SETUP RX DESCRIPTOR RING
    // ═══════════════════════════════════════════════════════════════════
    let rx_desc_cpu = config.dma_cpu_base.add(DmaRegion::RX_DESC_OFFSET);
    let rx_desc_bus = config.dma_bus_base + DmaRegion::RX_DESC_OFFSET as u64;
    let rx_buffer_cpu = config.dma_cpu_base.add(DmaRegion::RX_BUFFERS_OFFSET);
    let rx_buffer_bus = config.dma_bus_base + DmaRegion::RX_BUFFERS_OFFSET as u64;

    let rx_ring_len_bytes = (config.rx_queue_size as u32) * (regs::DESC_SIZE as u32);

    // Configure hardware RX ring
    asm_intel_setup_rx_ring(mmio_base, rx_desc_bus, rx_ring_len_bytes);

    // Create RX ring structure
    let mut rx_ring = RxRing::new(
        mmio_base,
        rx_desc_cpu,
        rx_desc_bus,
        rx_buffer_cpu,
        rx_buffer_bus,
        config.buffer_size,
        config.rx_queue_size,
    );

    // Initialize all RX descriptors with buffer addresses
    rx_ring.init_descriptors();

    // ═══════════════════════════════════════════════════════════════════
    // STEP 8: SETUP TX DESCRIPTOR RING
    // ═══════════════════════════════════════════════════════════════════
    let tx_desc_cpu = config.dma_cpu_base.add(DmaRegion::TX_DESC_OFFSET);
    let tx_desc_bus = config.dma_bus_base + DmaRegion::TX_DESC_OFFSET as u64;
    let tx_buffer_cpu = config.dma_cpu_base.add(DmaRegion::TX_BUFFERS_OFFSET);
    let tx_buffer_bus = config.dma_bus_base + DmaRegion::TX_BUFFERS_OFFSET as u64;

    let tx_ring_len_bytes = (config.tx_queue_size as u32) * (regs::DESC_SIZE as u32);

    // Configure hardware TX ring
    asm_intel_setup_tx_ring(mmio_base, tx_desc_bus, tx_ring_len_bytes);

    // Create TX ring structure
    let mut tx_ring = TxRing::new(
        mmio_base,
        tx_desc_cpu,
        tx_desc_bus,
        tx_buffer_cpu,
        tx_buffer_bus,
        config.buffer_size,
        config.tx_queue_size,
    );

    // Initialize all TX descriptors
    tx_ring.init_descriptors();

    // ═══════════════════════════════════════════════════════════════════
    // STEP 9: ENABLE RX
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_enable_rx(mmio_base);

    // Update RX tail to enable receiving
    rx_ring.update_tail();

    // ═══════════════════════════════════════════════════════════════════
    // STEP 10: ENABLE TX
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_enable_tx(mmio_base);

    // ═══════════════════════════════════════════════════════════════════
    // STEP 11: SET LINK UP
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_set_link_up(mmio_base);

    // Return success
    Ok(E1000eInitResult {
        mac,
        rx_ring,
        tx_ring,
    })
}

/// Generate a locally-administered MAC address from a seed.
///
/// Used as fallback if EEPROM MAC is invalid.
pub fn generate_fallback_mac(seed: u64) -> MacAddress {
    let mut mac = [0u8; 6];
    let bytes = seed.to_le_bytes();

    // Set locally-administered bit, clear multicast bit
    mac[0] = (bytes[0] & 0xFE) | 0x02;
    mac[1] = bytes[1];
    mac[2] = bytes[2];
    mac[3] = bytes[3];
    mac[4] = bytes[4];
    mac[5] = bytes[5];

    mac
}
