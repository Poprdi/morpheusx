//! Intel e1000e main driver implementation.
//!
//! Implements the `NetworkDriver` trait for smoltcp integration.
//!
//! # Reference
//! Intel 82579 Datasheet, NETWORK_IMPL_GUIDE.md §8

use crate::driver::traits::{DriverInit, NetworkDriver, RxError, TxError};
use crate::types::MacAddress;

use super::init::{init_e1000e, E1000eConfig, E1000eInitError};
use super::phy::PhyManager;
use super::rx::RxRing;
use super::tx::TxRing;
use super::{E1000E_DEVICE_IDS, INTEL_VENDOR_ID};

// ═══════════════════════════════════════════════════════════════════════════
// DRIVER ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// E1000e driver errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E1000eError {
    /// Initialization failed.
    InitFailed(E1000eInitError),
    /// Device not ready.
    NotReady,
    /// Link is down.
    LinkDown,
}

impl From<E1000eInitError> for E1000eError {
    fn from(err: E1000eInitError) -> Self {
        E1000eError::InitFailed(err)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DRIVER
// ═══════════════════════════════════════════════════════════════════════════

/// Intel e1000e network driver.
///
/// Supports Intel I218-LM, 82579, and compatible NICs.
pub struct E1000eDriver {
    /// MMIO base address.
    mmio_base: u64,
    /// MAC address.
    mac: MacAddress,
    /// PHY manager.
    phy: PhyManager,
    /// RX descriptor ring.
    rx_ring: RxRing,
    /// TX descriptor ring.
    tx_ring: TxRing,
    /// Device is initialized.
    initialized: bool,
}

impl E1000eDriver {
    /// Create and initialize a new e1000e driver.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `config`: Driver configuration
    ///
    /// # Safety
    /// - `mmio_base` must be a valid, mapped MMIO address
    /// - DMA region must be properly allocated and mapped
    pub unsafe fn new(mmio_base: u64, config: E1000eConfig) -> Result<Self, E1000eError> {
        // Initialize device
        let result = init_e1000e(mmio_base, &config)?;

        // Create PHY manager
        let phy = PhyManager::new(mmio_base, config.tsc_freq);

        Ok(Self {
            mmio_base,
            mac: result.mac,
            phy,
            rx_ring: result.rx_ring,
            tx_ring: result.tx_ring,
            initialized: true,
        })
    }

    /// Get MMIO base address.
    pub fn mmio_base(&self) -> u64 {
        self.mmio_base
    }

    /// Get PHY manager reference.
    pub fn phy(&mut self) -> &mut PhyManager {
        &mut self.phy
    }

    /// Check if device is initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Wait for link up with timeout.
    ///
    /// # Arguments
    /// - `timeout_us`: Timeout in microseconds
    ///
    /// # Returns
    /// `true` if link came up, `false` on timeout.
    pub fn wait_for_link(&mut self, timeout_us: u64) -> bool {
        self.phy.wait_for_link(timeout_us).is_ok()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK DRIVER IMPLEMENTATION
// ═══════════════════════════════════════════════════════════════════════════

impl NetworkDriver for E1000eDriver {
    /// Get MAC address.
    fn mac_address(&self) -> MacAddress {
        self.mac
    }

    /// Check if device can accept a TX frame.
    fn can_transmit(&self) -> bool {
        self.initialized && self.tx_ring.can_transmit()
    }

    /// Check if device has a received frame ready.
    fn can_receive(&self) -> bool {
        self.initialized && self.rx_ring.can_receive()
    }

    /// Transmit an Ethernet frame (fire-and-forget).
    ///
    /// Returns immediately after queuing the frame.
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        if !self.initialized {
            return Err(TxError::DeviceNotReady);
        }

        // Map tx::TxError to traits::TxError
        self.tx_ring.transmit(frame).map_err(|e| match e {
            super::tx::TxError::QueueFull => TxError::QueueFull,
            super::tx::TxError::FrameTooLarge { .. } => TxError::FrameTooLarge,
        })
    }

    /// Receive an Ethernet frame (non-blocking).
    ///
    /// Returns immediately with None if no frame available.
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        if !self.initialized {
            return Err(RxError::DeviceError);
        }

        // Map rx::RxError to traits::RxError
        self.rx_ring.receive(buffer).map_err(|e| match e {
            super::rx::RxError::BufferTooSmall { needed, .. } => RxError::BufferTooSmall { needed },
            super::rx::RxError::PacketError(_) => RxError::DeviceError,
        })
    }

    /// Refill RX queue (called in main loop Phase 1).
    ///
    /// For e1000e, descriptors are automatically recycled in receive().
    fn refill_rx_queue(&mut self) {
        // No-op for e1000e - descriptors are resubmitted in receive()
    }

    /// Collect TX completions (called in main loop Phase 5).
    fn collect_tx_completions(&mut self) {
        if self.initialized {
            self.tx_ring.collect_completions();
        }
    }

    /// Get link status.
    fn link_up(&self) -> bool {
        // Use cached status to avoid MMIO on every call
        // The phy.link_status() call updates the cache
        self.phy.cached_link_status().link_up
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DRIVER INIT IMPLEMENTATION
// ═══════════════════════════════════════════════════════════════════════════

impl DriverInit for E1000eDriver {
    type Error = E1000eError;
    type Config = E1000eConfig;

    fn supported_vendors() -> &'static [u16] {
        &[INTEL_VENDOR_ID]
    }

    fn supported_devices() -> &'static [u16] {
        E1000E_DEVICE_IDS
    }

    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error> {
        Self::new(mmio_base, config)
    }
}

// Safety: E1000eDriver is Send as it only holds raw pointers that are valid
// for the lifetime of the driver. The driver is designed for single-threaded use.
unsafe impl Send for E1000eDriver {}

// ═══════════════════════════════════════════════════════════════════════════
// DEVICE TRAIT IMPLEMENTATION (for smoltcp integration)
// ═══════════════════════════════════════════════════════════════════════════

impl crate::device::NetworkDevice for E1000eDriver {
    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }

    fn can_transmit(&self) -> bool {
        self.initialized && self.tx_ring.can_transmit()
    }

    fn can_receive(&self) -> bool {
        self.initialized && self.rx_ring.can_receive()
    }

    fn transmit(&mut self, packet: &[u8]) -> crate::error::Result<()> {
        if !self.initialized {
            return Err(crate::error::NetworkError::DeviceNotReady);
        }

        // Map tx::TxError to NetworkError
        self.tx_ring.transmit(packet).map_err(|e| match e {
            super::tx::TxError::QueueFull => crate::error::NetworkError::BufferExhausted,
            super::tx::TxError::FrameTooLarge { .. } => crate::error::NetworkError::PacketTooLarge,
        })
    }

    fn receive(&mut self, buffer: &mut [u8]) -> crate::error::Result<Option<usize>> {
        if !self.initialized {
            return Err(crate::error::NetworkError::DeviceNotReady);
        }

        // Map rx::RxError to NetworkError
        self.rx_ring.receive(buffer).map_err(|e| match e {
            super::rx::RxError::BufferTooSmall { .. } => crate::error::NetworkError::BufferTooSmall,
            super::rx::RxError::PacketError(_) => crate::error::NetworkError::ReceiveError,
        })
    }
}
