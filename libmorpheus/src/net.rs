//! Networking — raw NIC (exokernel) API.
//!
//! The MorpheusX exokernel exposes raw Ethernet frame TX/RX instead of
//! BSD sockets. Userland can build TCP/IP stacks on top of these primitives,
//! or use PORT_IN/PORT_OUT + PCI config + DMA to drive NICs directly.
//!
//! See also: `hw::port_in`, `hw::pci_cfg_read`, `hw::dma_alloc`.

use crate::raw::*;

/// NIC information.
#[repr(C)]
pub struct NicInfo {
    /// 6-byte MAC address, padded to 8.
    pub mac: [u8; 8],
    /// 1 if link up, 0 if down.
    pub link_up: u32,
    /// 1 if NIC is registered with kernel, 0 if not.
    pub present: u32,
}

/// Query NIC information (MAC address, link status, presence).
pub fn nic_info() -> Result<NicInfo, u64> {
    let mut info = NicInfo {
        mac: [0u8; 8],
        link_up: 0,
        present: 0,
    };
    let ret = unsafe { syscall1(SYS_NIC_INFO, &mut info as *mut NicInfo as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(info)
    }
}

/// Transmit a raw Ethernet frame.
///
/// `frame` must include the full Ethernet header (14 bytes minimum).
pub fn nic_tx(frame: &[u8]) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_NIC_TX, frame.as_ptr() as u64, frame.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Receive a raw Ethernet frame.
///
/// Returns the number of bytes received (0 if no frame available).
pub fn nic_rx(buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe { syscall2(SYS_NIC_RX, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Check if the NIC link is up.
///
/// Returns `true` if link is up, `false` if down.
pub fn nic_link_up() -> Result<bool, u64> {
    let ret = unsafe { syscall0(SYS_NIC_LINK) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret != 0)
    }
}

/// Get the NIC's 6-byte MAC address.
pub fn nic_mac() -> Result<[u8; 6], u64> {
    let mut mac = [0u8; 6];
    let ret = unsafe { syscall1(SYS_NIC_MAC, mac.as_mut_ptr() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(mac)
    }
}

/// Refill the NIC's RX descriptor ring.
pub fn nic_refill() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_NIC_REFILL) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}
