//! Network device abstraction for smoltcp integration.
//!
//! This keeps the core stack generic over concrete NIC drivers (PCIe/USB/SPI).

use crate::error::{NetworkError, Result};

pub mod virtio;
pub mod realtek;
pub mod intel;
pub mod broadcom;
pub mod registers;

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
