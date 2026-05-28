//! Network device abstraction for smoltcp integration.
//!
//! This module provides:
//! - [`NetworkDevice`] trait that all NIC drivers must implement
//! - [`UnifiedNetDevice`] - Auto-detecting wrapper for VirtIO or Intel e1000e
//! - PCI discovery utilities for device enumeration
//!
//! # Architecture (ASM-First)
//!
//! The MorpheusX network stack uses an ASM-first design where all hardware access
//! (MMIO, barriers, descriptor rings) is performed by hand-written assembly for
//! guaranteed correctness in bare-metal post-ExitBootServices execution.
//!
//! # Usage
//!
//! ```ignore
//! // Auto-detect and create the appropriate driver
//! let device = UnifiedNetDevice::probe(&dma, tsc_freq)?;
//!
//! // Use it - works the same for QEMU (VirtIO) or real hardware (Intel)
//! device.transmit(&frame)?;
//! ```

use morpheus_virtio::dma::DmaRegion;
// NIC drivers now live in this crate (Wave 3).
use crate::intel::{E1000eDriver, E1000eError};
use crate::virtio::{VirtioInitError, VirtioNetDriver};
// Wave 4 sank `NetworkError` into `morpheus-foundation` so this crate and
// `morpheus-net-stack` share one source of truth.
use crate::traits::NetworkDriver;
use morpheus_foundation::error::{NetworkError, Result};

pub mod pci;
pub mod registers;

/// Local copy of the `impl From<Src> for Dst` macro mirrored from
/// `morpheus-network::impl_from`. Cannot reach the network crate's macro from
/// here without a back-edge dependency.
#[macro_export]
macro_rules! impl_from {
    ($src:ty => $dst:ty : $variant:ident) => {
        impl From<$src> for $dst {
            fn from(e: $src) -> Self {
                <$dst>::$variant(e)
            }
        }
    };
    ($src:ty => $dst:ty : $variant:ident(_)) => {
        impl From<$src> for $dst {
            fn from(_: $src) -> Self {
                <$dst>::$variant
            }
        }
    };
}

/// Unified network device interface MorpheusX drivers must implement.
pub trait NetworkDevice {
    /// MAC address of the interface.
    fn mac_address(&self) -> [u8; 6];

    /// Whether the device has space to transmit a frame.
    fn can_transmit(&self) -> bool;

    /// Whether the device has a frame ready to read.
    fn can_receive(&self) -> bool;

    /// Transmit a frame.
    fn transmit(&mut self, packet: &[u8]) -> Result<()>;

    /// Receive a frame into the provided buffer.
    ///
    /// Returns `Ok(Some(len))` when a frame was read, `Ok(None)` when no frame
    /// is available, or an error on failure.
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>>;
}

/// Unified network device that works with both VirtIO and Intel e1000e.
///
/// This is the main entry point for network operations. It automatically
/// detects whether it's running in QEMU (VirtIO) or on real hardware (Intel)
/// and uses the appropriate driver transparently.
pub enum UnifiedNetDevice {
    /// VirtIO-net driver (QEMU, cloud VMs)
    VirtIO(VirtioNetDriver),
    /// Intel e1000e driver (ThinkPad T450s, real hardware)
    Intel(E1000eDriver),
}

/// Errors from unified device operations.
#[derive(Debug)]
pub enum UnifiedDeviceError {
    /// No supported network device found
    NoDevice,
    /// VirtIO initialization failed
    VirtioError(VirtioInitError),
    /// Intel e1000e initialization failed
    IntelError(E1000eError),
}

crate::impl_from!(VirtioInitError => UnifiedDeviceError : VirtioError);
crate::impl_from!(E1000eError => UnifiedDeviceError : IntelError);

impl UnifiedNetDevice {
    /// Probe for network device and create appropriate driver.
    ///
    /// This is the main entry point. It scans the PCI bus for supported NICs
    /// (Intel e1000e first, then VirtIO) and creates the appropriate driver.
    ///
    /// # Safety
    /// - DMA region must be properly allocated with correct bus addresses
    /// - TSC frequency must be calibrated at boot
    pub unsafe fn probe(
        dma: &DmaRegion,
        tsc_freq: u64,
    ) -> core::result::Result<Self, UnifiedDeviceError> {
        use crate::boot_probe::{probe_and_create_driver, ProbeError, ProbeResult};

        match probe_and_create_driver(dma, tsc_freq) {
            Ok(ProbeResult::Intel(driver)) => Ok(UnifiedNetDevice::Intel(driver)),
            Ok(ProbeResult::VirtIO(driver)) => Ok(UnifiedNetDevice::VirtIO(driver)),
            Err(ProbeError::NoDevice) => Err(UnifiedDeviceError::NoDevice),
            Err(ProbeError::IntelInitFailed) => Err(UnifiedDeviceError::NoDevice),
            Err(ProbeError::VirtioInitFailed) => Err(UnifiedDeviceError::NoDevice),
            Err(_) => Err(UnifiedDeviceError::NoDevice),
        }
    }

    /// Get which driver type is being used.
    pub fn driver_type(&self) -> &'static str {
        match self {
            UnifiedNetDevice::VirtIO(_) => "VirtIO-net",
            UnifiedNetDevice::Intel(_) => "Intel e1000e",
        }
    }

    pub fn link_up(&self) -> bool {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.link_up(),
            UnifiedNetDevice::Intel(d) => d.link_up(),
        }
    }

    /// Refill RX queue (call in main loop Phase 1).
    pub fn refill_rx_queue(&mut self) {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.refill_rx_queue(),
            UnifiedNetDevice::Intel(d) => d.refill_rx_queue(),
        }
    }

    /// Collect TX completions (call in main loop Phase 5).
    pub fn collect_tx_completions(&mut self) {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.collect_tx_completions(),
            UnifiedNetDevice::Intel(d) => d.collect_tx_completions(),
        }
    }
}

impl NetworkDevice for UnifiedNetDevice {
    fn mac_address(&self) -> [u8; 6] {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.mac_address(),
            UnifiedNetDevice::Intel(d) => d.mac_address(),
        }
    }

    fn can_transmit(&self) -> bool {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.can_transmit(),
            UnifiedNetDevice::Intel(d) => d.can_transmit(),
        }
    }

    fn can_receive(&self) -> bool {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.can_receive(),
            UnifiedNetDevice::Intel(d) => d.can_receive(),
        }
    }

    fn transmit(&mut self, packet: &[u8]) -> Result<()> {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.transmit(packet).map_err(|e| match e {
                crate::traits::TxError::QueueFull => NetworkError::BufferExhausted,
                crate::traits::TxError::FrameTooLarge => NetworkError::PacketTooLarge,
                crate::traits::TxError::DeviceNotReady => NetworkError::DeviceNotReady,
            }),
            UnifiedNetDevice::Intel(d) => d.transmit(packet).map_err(|e| match e {
                crate::traits::TxError::QueueFull => NetworkError::BufferExhausted,
                crate::traits::TxError::FrameTooLarge => NetworkError::PacketTooLarge,
                crate::traits::TxError::DeviceNotReady => NetworkError::DeviceNotReady,
            }),
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>> {
        match self {
            UnifiedNetDevice::VirtIO(d) => d.receive(buffer).map_err(|e| match e {
                crate::traits::RxError::BufferTooSmall { .. } => NetworkError::BufferTooSmall,
                crate::traits::RxError::DeviceError => NetworkError::ReceiveError,
            }),
            UnifiedNetDevice::Intel(d) => d.receive(buffer).map_err(|e| match e {
                crate::traits::RxError::BufferTooSmall { .. } => NetworkError::BufferTooSmall,
                crate::traits::RxError::DeviceError => NetworkError::ReceiveError,
            }),
        }
    }
}

/// Placeholder NIC that does nothing. Useful for early bring-up.
pub struct NullDevice;

impl NetworkDevice for NullDevice {
    fn mac_address(&self) -> [u8; 6] {
        [0u8; 6]
    }

    fn can_transmit(&self) -> bool {
        false
    }

    fn can_receive(&self) -> bool {
        false
    }

    fn transmit(&mut self, _packet: &[u8]) -> Result<()> {
        Err(NetworkError::ProtocolNotAvailable)
    }

    fn receive(&mut self, _buffer: &mut [u8]) -> Result<Option<usize>> {
        Ok(None)
    }
}
