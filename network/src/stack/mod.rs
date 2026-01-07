//! smoltcp integration layer.
//!
//! This module provides the bridge between MorpheusX network device drivers
//! and the smoltcp TCP/IP stack.
//!
//! # Components
//!
//! - [`DeviceAdapter`] - Adapts `NetworkDevice` to smoltcp's `Device` trait
//! - [`NetInterface`] - Full IP stack with TCP sockets and DHCP
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::stack::{NetInterface, NetConfig};
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::device::hal::StaticHal;
//! use dma_pool::DmaPool;
//!
//! // Initialize DMA pool from caves in our PE image
//! unsafe { DmaPool::init_from_caves(image_base, image_end) };
//!
//! // Initialize HAL
//! StaticHal::init();
//!
//! // Create device
//! let device = VirtioNetDevice::<StaticHal, _>::new(transport)?;
//!
//! // Create interface with DHCP
//! let mut iface = NetInterface::new(device, NetConfig::dhcp());
//!
//! // Poll until IP configured
//! while !iface.has_ip() {
//!     iface.poll(get_time_ms());
//! }
//! ```

mod interface;

use crate::device::NetworkDevice;
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use core::marker::PhantomData;

pub use interface::{NetInterface, NetConfig, NetState, MAX_TCP_SOCKETS};
pub use crate::device::pci::ecam_bases;

const MTU: usize = 1536;

/// Thin adapter that exposes a `NetworkDevice` to smoltcp.
pub struct DeviceAdapter<D: NetworkDevice> {
    pub inner: D,
}

impl<D: NetworkDevice> DeviceAdapter<D> {
    pub fn new(inner: D) -> Self {
        Self { inner }
    }
}

impl<D: NetworkDevice> Device for DeviceAdapter<D> {
    type RxToken<'a> = AdapterRxToken<'a, D> where D: 'a;
    type TxToken<'a> = AdapterTxToken<'a, D> where D: 'a;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = MTU;
        caps.medium = Medium::Ethernet;
        caps
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Try to actually receive a packet first
        // Only return tokens if we have data ready
        let mut temp_buf = [0u8; MTU];
        match self.inner.receive(&mut temp_buf) {
            Ok(Some(len)) if len > 0 => {
                // Track received packets
                RX_PACKET_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                
                // We have a packet! Create tokens with the data
                let device_ptr: *mut D = &mut self.inner;
                let mut token = AdapterRxToken {
                    device: device_ptr,
                    buffer: [0u8; MTU],
                    len,
                    _p: PhantomData,
                };
                token.buffer[..len].copy_from_slice(&temp_buf[..len]);
                Some((
                    token,
                    AdapterTxToken {
                        device: device_ptr,
                        _p: PhantomData,
                    },
                ))
            }
            _ => None, // No packet available
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.inner.can_transmit() {
            let device_ptr: *mut D = &mut self.inner;
            Some(AdapterTxToken {
                device: device_ptr,
                _p: PhantomData,
            })
        } else {
            None
        }
    }
}

pub struct AdapterRxToken<'a, D: NetworkDevice> {
    device: *mut D,
    buffer: [u8; MTU],
    len: usize,
    _p: PhantomData<&'a mut D>,
}

impl<'a, D: NetworkDevice> RxToken for AdapterRxToken<'a, D> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // We already have the data in buffer from when receive() was called
        let mut buf = self.buffer;
        f(&mut buf[..self.len])
    }
}

pub struct AdapterTxToken<'a, D: NetworkDevice> {
    device: *mut D,
    _p: PhantomData<&'a mut D>,
}

impl<'a, D: NetworkDevice> TxToken for AdapterTxToken<'a, D> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = [0u8; MTU];
        let result = f(&mut buffer[..len]);
        
        // Track packets sent
        TX_PACKET_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        
        // Check if this is a DHCP packet (UDP port 67/68)
        if len >= 42 {
            // Check if Ethernet + IP + UDP to port 67 (DHCP server)
            let ethertype = u16::from_be_bytes([buffer[12], buffer[13]]);
            if ethertype == 0x0800 {  // IPv4
                let protocol = buffer[23];  // IP protocol field
                if protocol == 17 {  // UDP
                    let dst_port = u16::from_be_bytes([buffer[36], buffer[37]]);
                    if dst_port == 67 {
                        DHCP_DISCOVER_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
        
        // Attempt transmit and capture error for debugging
        // We still have to return `result` regardless of TX success
        // because that's what smoltcp expects
        match unsafe { (*self.device).transmit(&buffer[..len]) } {
            Ok(()) => {
                // TX succeeded
            }
            Err(_e) => {
                // TX failed - log to static flag for debugging
                // In a real implementation, we'd have a proper error channel
                TX_ERROR_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
        }
        
        result
    }
}

// Global counter for TX errors (debugging)
static TX_ERROR_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static TX_PACKET_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static DHCP_DISCOVER_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static RX_PACKET_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Get the number of TX errors that have occurred.
pub fn tx_error_count() -> u32 {
    TX_ERROR_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Reset the TX error counter.
pub fn reset_tx_error_count() {
    TX_ERROR_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
}

/// Get the number of packets transmitted.
pub fn tx_packet_count() -> u32 {
    TX_PACKET_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Get the number of DHCP discover packets sent.
pub fn dhcp_discover_count() -> u32 {
    DHCP_DISCOVER_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Get the number of packets received.
pub fn rx_packet_count() -> u32 {
    RX_PACKET_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Reset all counters.
pub fn reset_counters() {
    TX_ERROR_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
    TX_PACKET_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
    DHCP_DISCOVER_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
    RX_PACKET_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
}
