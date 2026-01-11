//! Driver trait definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §8.2

use crate::types::MacAddress;

/// TX error types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    /// TX queue is full, try again after completions collected.
    QueueFull,
    /// Device not ready.
    DeviceNotReady,
    /// Frame too large.
    FrameTooLarge,
}

/// RX error types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxError {
    /// Provided buffer too small for frame.
    BufferTooSmall {
        /// Required buffer size.
        needed: usize,
    },
    /// Device error.
    DeviceError,
}

/// Core network device interface.
///
/// All NIC drivers must implement this trait. Higher layers
/// (smoltcp adapter, state machines) use this interface.
pub trait NetworkDriver {
    /// Get MAC address.
    fn mac_address(&self) -> MacAddress;

    /// Check if device can accept a TX frame.
    ///
    /// Returns true if `transmit()` will succeed.
    fn can_transmit(&self) -> bool;

    /// Check if device has a received frame ready.
    ///
    /// Returns true if `receive()` will return `Ok(Some(_))`.
    fn can_receive(&self) -> bool;

    /// Transmit an Ethernet frame.
    ///
    /// # Arguments
    /// - `frame`: Complete Ethernet frame (no VirtIO header)
    ///
    /// # Returns
    /// - `Ok(())`: Frame queued (fire-and-forget)
    /// - `Err(TxError::QueueFull)`: No space, try again later
    ///
    /// # Contract
    /// - MUST return immediately (no completion wait)
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError>;

    /// Receive an Ethernet frame.
    ///
    /// # Arguments
    /// - `buffer`: Buffer to copy frame into
    ///
    /// # Returns
    /// - `Ok(Some(len))`: Frame received, `len` bytes copied
    /// - `Ok(None)`: No frame available (normal)
    /// - `Err(RxError)`: Receive error
    ///
    /// # Contract
    /// - MUST return immediately (no blocking)
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError>;

    /// Refill RX queue with available buffers.
    ///
    /// Called in main loop Phase 1.
    fn refill_rx_queue(&mut self);

    /// Collect TX completions.
    ///
    /// Called in main loop Phase 5.
    fn collect_tx_completions(&mut self);

    /// Get link status.
    fn link_up(&self) -> bool {
        true
    }
}

/// Driver initialization trait.
pub trait DriverInit: Sized {
    /// Error type for initialization failures.
    type Error: core::fmt::Debug;

    /// Configuration type.
    type Config;

    /// PCI vendor IDs this driver supports.
    fn supported_vendors() -> &'static [u16];

    /// PCI device IDs this driver supports.
    fn supported_devices() -> &'static [u16];

    /// Check if driver supports a PCI device.
    fn supports_device(vendor: u16, device: u16) -> bool {
        Self::supported_vendors().contains(&vendor) && Self::supported_devices().contains(&device)
    }

    /// Create driver from MMIO base and configuration.
    ///
    /// # Safety
    /// - `mmio_base` must be valid device MMIO address
    /// - Configuration must be valid
    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error>;
}
