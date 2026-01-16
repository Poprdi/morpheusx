//! Unified Network Driver Abstraction.
//!
//! Provides a single driver type that abstracts over all supported NIC drivers:
//! - VirtIO-net (QEMU, KVM)
//! - Intel e1000e (ThinkPad T450s, X240, T440s, etc.)
//!
//! # Usage
//!
//! Drivers are created via `boot::probe::probe_and_create_driver()` or directly
//! via their respective constructors. The unified driver provides trait
//! dispatch for the `NetworkDriver` interface.
//!
//! ```ignore
//! // Via probe (scans PCI bus)
//! let driver = probe_and_create_driver(&dma, tsc_freq)?;
//!
//! // Or directly
//! let driver = UnifiedNetworkDriver::Intel(E1000eDriver::new(mmio_base, config)?);
//! ```

use crate::driver::intel::{E1000eDriver, E1000eError};
use crate::driver::traits::{NetworkDriver, RxError, TxError};
use crate::driver::virtio::{VirtioInitError, VirtioNetDriver};
use crate::types::MacAddress;

// ═══════════════════════════════════════════════════════════════════════════
// UNIFIED DRIVER ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// Errors during unified driver initialization.
#[derive(Debug, Clone, Copy)]
pub enum UnifiedDriverError {
    /// No NIC detected in handoff.
    NoNicDetected,
    /// Unsupported NIC type.
    UnsupportedNicType(u8),
    /// VirtIO initialization failed.
    VirtioError(VirtioInitError),
    /// Intel e1000e initialization failed.
    IntelError(E1000eError),
    /// Invalid handoff data.
    InvalidHandoff,
}

impl From<VirtioInitError> for UnifiedDriverError {
    fn from(e: VirtioInitError) -> Self {
        UnifiedDriverError::VirtioError(e)
    }
}

impl From<E1000eError> for UnifiedDriverError {
    fn from(e: E1000eError) -> Self {
        UnifiedDriverError::IntelError(e)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// UNIFIED NETWORK DRIVER
// ═══════════════════════════════════════════════════════════════════════════

/// Unified network driver that wraps all supported NIC drivers.
///
/// This enum provides a single type that can represent any supported NIC,
/// allowing the network stack to be driver-agnostic.
pub enum UnifiedNetworkDriver {
    /// VirtIO-net driver (QEMU, KVM).
    VirtIO(VirtioNetDriver),
    /// Intel e1000e driver (real hardware).
    Intel(E1000eDriver),
}

impl UnifiedNetworkDriver {
    /// Get the driver type name for logging.
    pub fn driver_name(&self) -> &'static str {
        match self {
            UnifiedNetworkDriver::VirtIO(_) => "VirtIO-net",
            UnifiedNetworkDriver::Intel(_) => "Intel e1000e",
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK DRIVER TRAIT IMPLEMENTATION
// ═══════════════════════════════════════════════════════════════════════════

impl NetworkDriver for UnifiedNetworkDriver {
    fn mac_address(&self) -> MacAddress {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.mac_address(),
            UnifiedNetworkDriver::Intel(d) => d.mac_address(),
        }
    }

    fn can_transmit(&self) -> bool {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.can_transmit(),
            UnifiedNetworkDriver::Intel(d) => d.can_transmit(),
        }
    }

    fn can_receive(&self) -> bool {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.can_receive(),
            UnifiedNetworkDriver::Intel(d) => d.can_receive(),
        }
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.transmit(frame),
            UnifiedNetworkDriver::Intel(d) => d.transmit(frame),
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.receive(buffer),
            UnifiedNetworkDriver::Intel(d) => d.receive(buffer),
        }
    }

    fn refill_rx_queue(&mut self) {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.refill_rx_queue(),
            UnifiedNetworkDriver::Intel(d) => d.refill_rx_queue(),
        }
    }

    fn collect_tx_completions(&mut self) {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.collect_tx_completions(),
            UnifiedNetworkDriver::Intel(d) => d.collect_tx_completions(),
        }
    }

    fn link_up(&self) -> bool {
        match self {
            UnifiedNetworkDriver::VirtIO(d) => d.link_up(),
            UnifiedNetworkDriver::Intel(d) => d.link_up(),
        }
    }
}

// Safety: UnifiedNetworkDriver is Send because all variants are Send
unsafe impl Send for UnifiedNetworkDriver {}
