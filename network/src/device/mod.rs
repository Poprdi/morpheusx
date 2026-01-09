//! Network device abstraction for smoltcp integration.
//!
//! This module provides the core `NetworkDevice` trait that all NIC drivers must implement,
//! along with PCI discovery utilities for device enumeration.
//!
//! # Architecture (ASM-First)
//!
//! The MorpheusX network stack uses an ASM-first design where all hardware access
//! (MMIO, barriers, descriptor rings) is performed by hand-written assembly for
//! guaranteed correctness in bare-metal post-ExitBootServices execution.
//!
//! The actual drivers are in `crate::driver::`:
//! - [`crate::driver::virtio::VirtioNetDriver`] - VirtIO-net using ASM layer
//!
//! # PCI Discovery
//!
//! The [`pci`] module provides PCI bus scanning for device discovery.

use crate::error::{NetworkError, Result};

pub mod pci;
pub mod registers;

// NOTE: Legacy modules removed (virtio.rs, factory.rs, hal/)
// The ASM-backed VirtIO driver is in crate::driver::virtio

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
