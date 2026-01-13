//! Unified Network Driver Abstraction.
//!
//! Provides a single driver type that abstracts over all supported NIC drivers:
//! - VirtIO-net (QEMU, KVM)
//! - Intel e1000e (ThinkPad T450s, X240, T440s, etc.)
//! - Future: Realtek, Broadcom
//!
//! The unified driver is created via dynamic probing at boot time, allowing
//! the network stack to work identically regardless of underlying hardware.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §8

use crate::boot::handoff::{BootHandoff, NIC_TYPE_INTEL, NIC_TYPE_VIRTIO};
use crate::driver::intel::{E1000eConfig, E1000eDriver, E1000eError};
use crate::driver::traits::{NetworkDriver, RxError, TxError};
use crate::driver::virtio::{VirtioConfig, VirtioInitError, VirtioNetDriver, VirtioTransport};
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
    // Future variants:
    // Realtek(RealtekDriver),
    // Broadcom(BroadcomDriver),
}

impl UnifiedNetworkDriver {
    /// Create a unified driver from boot handoff.
    ///
    /// Automatically selects the correct driver based on `nic_type` in handoff.
    ///
    /// # Safety
    /// - Handoff must be valid and properly initialized
    /// - DMA regions must be properly allocated
    pub unsafe fn from_handoff(handoff: &BootHandoff) -> Result<Self, UnifiedDriverError> {
        use crate::boot::handoff::{TRANSPORT_MMIO, TRANSPORT_PCI_MODERN};
        use crate::driver::virtio::PciModernConfig;

        match handoff.nic_type {
            NIC_TYPE_VIRTIO => {
                // Build VirtIO transport based on transport type
                let transport = match handoff.nic_transport_type {
                    TRANSPORT_PCI_MODERN => {
                        // PCI Modern transport uses PciModernConfig
                        VirtioTransport::pci_modern(PciModernConfig {
                            common_cfg: handoff.nic_common_cfg,
                            notify_cfg: handoff.nic_notify_cfg,
                            notify_off_multiplier: handoff.nic_notify_off_multiplier,
                            isr_cfg: handoff.nic_isr_cfg,
                            device_cfg: handoff.nic_device_cfg,
                            pci_cfg: 0,  // Not used for network
                        })
                    }
                    TRANSPORT_MMIO | _ => {
                        // Legacy MMIO transport
                        VirtioTransport::mmio(handoff.nic_mmio_base)
                    }
                };

                // Build VirtIO config
                let virtio_config = VirtioConfig {
                    queue_size: VirtioConfig::DEFAULT_QUEUE_SIZE,
                    buffer_size: VirtioConfig::DEFAULT_BUFFER_SIZE,
                    dma_cpu_base: handoff.dma_cpu_ptr as *mut u8,
                    dma_bus_base: handoff.dma_cpu_ptr, // Identity-mapped post-EBS
                    dma_size: handoff.dma_size as usize,
                };

                let driver =
                    VirtioNetDriver::new_with_transport(transport, virtio_config, handoff.tsc_freq)?;
                Ok(UnifiedNetworkDriver::VirtIO(driver))
            }

            NIC_TYPE_INTEL => {
                // Build e1000e config
                let config = E1000eConfig::new(
                    handoff.dma_cpu_ptr as *mut u8,
                    handoff.dma_bus_addr,
                    handoff.tsc_freq,
                );

                let driver = E1000eDriver::new(handoff.nic_mmio_base, config)?;
                Ok(UnifiedNetworkDriver::Intel(driver))
            }

            0 => Err(UnifiedDriverError::NoNicDetected),
            other => Err(UnifiedDriverError::UnsupportedNicType(other)),
        }
    }

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
