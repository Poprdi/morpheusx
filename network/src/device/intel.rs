//! Intel e1000/e1000e/i219/i225 NIC drivers
//!
//! Intel NICs are very common on desktops, laptops, and servers (~45% market share).
//! The e1000 driver is well-documented and a good reference implementation.
//!
//! TODO: Implement Intel drivers
//! - e1000 (legacy, vendor 0x8086, device 0x100E, 0x100F, etc.)
//! - e1000e (modern, various device IDs)
//! - i219/i225/i226 (recent Intel chipsets)
//! - Register initialization sequence
//! - RX/TX descriptor rings
//! - EEPROM/NVM MAC address reading
//!
//! Reference: Intel 8254x/8257x Software Developer Manual

use crate::device::NetworkDevice;
use crate::error::{NetworkError, Result};

/// Intel e1000 legacy Gigabit Ethernet driver.
///
/// Supports older Intel NICs and most VM emulations (QEMU, VMware).
pub struct E1000Device {
    // TODO: MMIO base address
    // TODO: RX/TX descriptor rings
    // TODO: MAC address
    _private: (),
}

impl E1000Device {
    /// Probe PCI bus for e1000 device.
    pub fn probe() -> Option<Self> {
        // TODO: Scan PCI bus for Intel vendor ID (0x8086)
        // TODO: Match e1000 device IDs
        // TODO: Initialize device
        None
    }
}

impl NetworkDevice for E1000Device {
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

/// Intel e1000e modern Gigabit Ethernet driver.
pub struct E1000eDevice {
    _private: (),
}

impl E1000eDevice {
    /// Probe PCI bus for e1000e device.
    pub fn probe() -> Option<Self> {
        None
    }
}

impl NetworkDevice for E1000eDevice {
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

/// Intel i219/i225/i226 driver for recent Intel chipsets.
pub struct IntelI219Device {
    _private: (),
}

impl IntelI219Device {
    /// Probe PCI bus for i219/i225/i226 device.
    pub fn probe() -> Option<Self> {
        None
    }
}

impl NetworkDevice for IntelI219Device {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_e1000_probe_returns_none_without_hardware() {
        assert!(E1000Device::probe().is_none());
    }

    #[test]
    fn test_e1000e_probe_returns_none_without_hardware() {
        assert!(E1000eDevice::probe().is_none());
    }

    #[test]
    fn test_i219_probe_returns_none_without_hardware() {
        assert!(IntelI219Device::probe().is_none());
    }
}
