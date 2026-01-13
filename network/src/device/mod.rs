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

use crate::dma::DmaRegion;
use crate::driver::intel::{E1000eDriver, E1000eError};
use crate::driver::traits::NetworkDriver;
use crate::driver::virtio::{VirtioNetDriver, VirtioInitError};
use crate::error::{NetworkError, Result};

pub mod pci;
pub mod registers;

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK DEVICE TRAIT
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
// UNIFIED NETWORK DEVICE
// ═══════════════════════════════════════════════════════════════════════════

/// Unified network device that works with both VirtIO and Intel e1000e.
///
/// This is the main entry point for network operations. It automatically
/// detects whether it's running in QEMU (VirtIO) or on real hardware (Intel)
/// and uses the appropriate driver transparently.
///
/// # Example
///
/// ```ignore
/// // Probe and create - works anywhere
/// let mut device = UnifiedNetDevice::probe(&dma, tsc_freq)?;
///
/// // All operations are the same regardless of underlying hardware
/// let mac = device.mac_address();
/// device.transmit(&frame)?;
/// if let Some(len) = device.receive(&mut buffer)? {
///     // process received frame
/// }
/// ```
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

impl From<VirtioInitError> for UnifiedDeviceError {
    fn from(e: VirtioInitError) -> Self {
        UnifiedDeviceError::VirtioError(e)
    }
}

impl From<E1000eError> for UnifiedDeviceError {
    fn from(e: E1000eError) -> Self {
        UnifiedDeviceError::IntelError(e)
    }
}

impl UnifiedNetDevice {
    /// Probe for network device and create appropriate driver.
    ///
    /// This is the main entry point. It scans the PCI bus for supported NICs
    /// (Intel e1000e first, then VirtIO) and creates the appropriate driver.
    ///
    /// # Arguments
    /// - `dma`: Pre-allocated DMA region (2MB minimum)
    /// - `tsc_freq`: Calibrated TSC frequency (ticks/second)
    ///
    /// # Returns
    /// - `Ok(UnifiedNetDevice)` - Ready to use network device
    /// - `Err(UnifiedDeviceError::NoDevice)` - No supported NIC found
    ///
    /// # Safety
    /// - DMA region must be properly allocated with correct bus addresses
    /// - TSC frequency must be calibrated at boot
    pub unsafe fn probe(dma: &DmaRegion, tsc_freq: u64) -> core::result::Result<Self, UnifiedDeviceError> {
        use crate::boot::probe::{probe_and_create_driver, ProbeResult, ProbeError};

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

    /// Check if link is up.
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
            UnifiedNetDevice::VirtIO(d) => {
                d.transmit(packet).map_err(|e| match e {
                    crate::driver::traits::TxError::QueueFull => NetworkError::BufferExhausted,
                    crate::driver::traits::TxError::FrameTooLarge => NetworkError::PacketTooLarge,
                    crate::driver::traits::TxError::DeviceNotReady => NetworkError::DeviceNotReady,
                })
            }
            UnifiedNetDevice::Intel(d) => {
                d.transmit(packet).map_err(|e| match e {
                    crate::driver::traits::TxError::QueueFull => NetworkError::BufferExhausted,
                    crate::driver::traits::TxError::FrameTooLarge => NetworkError::PacketTooLarge,
                    crate::driver::traits::TxError::DeviceNotReady => NetworkError::DeviceNotReady,
                })
            }
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>> {
        match self {
            UnifiedNetDevice::VirtIO(d) => {
                d.receive(buffer).map_err(|e| match e {
                    crate::driver::traits::RxError::BufferTooSmall { .. } => NetworkError::BufferTooSmall,
                    crate::driver::traits::RxError::DeviceError => NetworkError::ReceiveError,
                })
            }
            UnifiedNetDevice::Intel(d) => {
                d.receive(buffer).map_err(|e| match e {
                    crate::driver::traits::RxError::BufferTooSmall { .. } => NetworkError::BufferTooSmall,
                    crate::driver::traits::RxError::DeviceError => NetworkError::ReceiveError,
                })
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// UNIFIED BLOCK DEVICE
// ═══════════════════════════════════════════════════════════════════════════

use crate::driver::ahci::{AhciDriver, AhciInitError};
use crate::driver::block_traits::{BlockCompletion, BlockDeviceInfo, BlockDriver, BlockError};
use crate::driver::virtio_blk::{VirtioBlkDriver, VirtioBlkInitError};

/// Unified block device that works with both VirtIO-blk and AHCI.
///
/// This provides automatic hardware detection and driver selection:
/// - QEMU/VMs: Uses VirtIO-blk
/// - Real hardware (ThinkPad T450s): Uses AHCI SATA driver
///
/// # Example
///
/// ```ignore
/// // Probe and create - works anywhere
/// let mut device = UnifiedBlockDevice::probe(&blk_dma, tsc_freq)?;
///
/// // Check what was detected
/// println!("Using: {}", device.driver_type());  // "VirtIO-blk" or "AHCI SATA"
///
/// // All operations are the same regardless of underlying hardware
/// let info = device.info();
/// device.submit_read(sector, buffer_phys, count, request_id)?;
/// if let Some(completion) = device.poll_completion() {
///     // handle completion
/// }
/// ```
pub enum UnifiedBlockDevice {
    /// VirtIO-blk driver (QEMU, cloud VMs)
    VirtIO(VirtioBlkDriver),
    /// AHCI SATA driver (real hardware - ThinkPad T450s)
    Ahci(AhciDriver),
}

/// Errors from unified block device operations.
#[derive(Debug)]
pub enum UnifiedBlockError {
    /// No supported block device found
    NoDevice,
    /// VirtIO-blk initialization failed
    VirtioError(VirtioBlkInitError),
    /// AHCI initialization failed
    AhciError(AhciInitError),
}

impl From<VirtioBlkInitError> for UnifiedBlockError {
    fn from(e: VirtioBlkInitError) -> Self {
        UnifiedBlockError::VirtioError(e)
    }
}

impl From<AhciInitError> for UnifiedBlockError {
    fn from(e: AhciInitError) -> Self {
        UnifiedBlockError::AhciError(e)
    }
}

impl UnifiedBlockDevice {
    /// Get which driver type is being used.
    pub fn driver_type(&self) -> &'static str {
        match self {
            UnifiedBlockDevice::VirtIO(_) => "VirtIO-blk",
            UnifiedBlockDevice::Ahci(_) => "AHCI SATA",
        }
    }

    /// Check if device is ready for I/O.
    pub fn is_ready(&self) -> bool {
        match self {
            UnifiedBlockDevice::VirtIO(_) => true, // VirtIO always ready after init
            UnifiedBlockDevice::Ahci(d) => d.link_up(),
        }
    }
}

impl BlockDriver for UnifiedBlockDevice {
    fn info(&self) -> BlockDeviceInfo {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.info(),
            UnifiedBlockDevice::Ahci(d) => d.info(),
        }
    }

    fn can_submit(&self) -> bool {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.can_submit(),
            UnifiedBlockDevice::Ahci(d) => d.can_submit(),
        }
    }

    fn submit_read(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> core::result::Result<(), BlockError> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.submit_read(sector, buffer_phys, num_sectors, request_id),
            UnifiedBlockDevice::Ahci(d) => d.submit_read(sector, buffer_phys, num_sectors, request_id),
        }
    }

    fn submit_write(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> core::result::Result<(), BlockError> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.submit_write(sector, buffer_phys, num_sectors, request_id),
            UnifiedBlockDevice::Ahci(d) => d.submit_write(sector, buffer_phys, num_sectors, request_id),
        }
    }

    fn poll_completion(&mut self) -> Option<BlockCompletion> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.poll_completion(),
            UnifiedBlockDevice::Ahci(d) => d.poll_completion(),
        }
    }

    fn notify(&mut self) {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.notify(),
            UnifiedBlockDevice::Ahci(d) => d.notify(),
        }
    }

    fn flush(&mut self) -> core::result::Result<(), BlockError> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.flush(),
            UnifiedBlockDevice::Ahci(d) => d.flush(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NULL DEVICE (for testing/early bring-up)
// ═══════════════════════════════════════════════════════════════════════════

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

