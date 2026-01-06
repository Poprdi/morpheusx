//! Broadcom NetXtreme/NetXtreme II NIC drivers
//!
//! Broadcom NICs are common on enterprise servers and some workstations (~10% market).
//!
//! TODO: Implement Broadcom drivers
//! - NetXtreme (tg3 driver, vendor 0x14E4)
//! - NetXtreme II (bnx2 driver)
//! - Complex initialization sequences
//! - Firmware loading requirements
//!
//! Reference: Linux tg3/bnx2 driver source code

use crate::device::NetworkDevice;
use crate::error::{NetworkError, Result};

/// Broadcom NetXtreme (tg3) Gigabit Ethernet driver.
pub struct BroadcomTg3Device {
    _private: (),
}

impl BroadcomTg3Device {
    /// Probe PCI bus for Broadcom NetXtreme device.
    pub fn probe() -> Option<Self> {
        // TODO: Scan PCI bus for Broadcom vendor ID (0x14E4)
        // TODO: Match tg3 device IDs
        None
    }
}

impl NetworkDevice for BroadcomTg3Device {
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

/// Broadcom NetXtreme II (bnx2) Gigabit Ethernet driver.
pub struct BroadcomBnx2Device {
    _private: (),
}

impl BroadcomBnx2Device {
    /// Probe PCI bus for Broadcom NetXtreme II device.
    pub fn probe() -> Option<Self> {
        None
    }
}

impl NetworkDevice for BroadcomBnx2Device {
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
    fn test_tg3_probe_returns_none_without_hardware() {
        assert!(BroadcomTg3Device::probe().is_none());
    }

    #[test]
    fn test_bnx2_probe_returns_none_without_hardware() {
        assert!(BroadcomBnx2Device::probe().is_none());
    }
}
