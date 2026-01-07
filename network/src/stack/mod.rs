//! smoltcp integration layer.
//!
//! This module provides the bridge between MorpheusX network device drivers
//! and the smoltcp TCP/IP stack.
//!
//! # Components
//!
//! - [`DeviceAdapter`] - Adapts `NetworkDevice` to smoltcp's `Device` trait
//! - [`NetInterface`] - Full IP stack with TCP sockets and DHCP
//! - [`NetworkStack`] - High-level convenience wrapper for HTTP client
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::stack::{NetInterface, NetConfig};
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::device::hal::StaticHal;
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
//!
//! // Create TCP socket and connect
//! let socket = iface.tcp_socket()?;
//! iface.tcp_connect(socket, remote_ip, 80)?;
//! ```

mod interface;
pub mod setup;

use crate::device::NetworkDevice;
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use core::marker::PhantomData;

pub use interface::{NetInterface, NetConfig, NetState, MAX_TCP_SOCKETS};
pub use setup::{NetworkStack, init_virtio_network, init_qemu_network, EcamConfigAccess};
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
        if self.inner.can_receive() {
            let device_ptr: *mut D = &mut self.inner;
            Some((
                AdapterRxToken {
                    device: device_ptr,
                    buffer: [0u8; MTU],
                    _p: PhantomData,
                },
                AdapterTxToken {
                    device: device_ptr,
                    _p: PhantomData,
                },
            ))
        } else {
            None
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
    _p: PhantomData<&'a mut D>,
}

impl<'a, D: NetworkDevice> RxToken for AdapterRxToken<'a, D> {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let len = match unsafe { (*self.device).receive(&mut self.buffer) } {
            Ok(Some(size)) => size,
            Ok(None) => 0,
            Err(_) => 0,
        };
        f(&mut self.buffer[..len])
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
        // Best-effort transmit; ignore errors for now.
        let _ = unsafe { (*self.device).transmit(&buffer[..len]) };
        result
    }
}

/// Minimal holder for future smoltcp interface wiring.
pub struct NetworkStack<D: NetworkDevice> {
    pub device: DeviceAdapter<D>,
}

impl<D: NetworkDevice> NetworkStack<D> {
    pub fn new(device: D) -> Self {
        Self {
            device: DeviceAdapter::new(device),
        }
    }

    pub fn capabilities(&self) -> DeviceCapabilities {
        self.device.capabilities()
    }
}
