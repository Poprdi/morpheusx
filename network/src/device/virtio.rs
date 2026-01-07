//! VirtIO-net driver powered by the `virtio-drivers` crate.
//!
//! This module provides a high-level wrapper around `virtio_drivers`'s
//! `VirtIONetRaw` that implements our `NetworkDevice` trait.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    VirtioNetDevice                          │
//! │  (implements NetworkDevice for smoltcp integration)         │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │           virtio_drivers::device::net::VirtIONetRaw         │
//! │  (raw packet TX/RX, virtqueue management)                   │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              virtio_drivers::transport::*                   │
//! │  (PciTransport or MmioTransport)                            │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    StaticHal (dma-pool)                     │
//! │  (Firmware-agnostic DMA memory, address translation)        │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::device::hal::StaticHal;
//!
//! // Initialize HAL (once at boot)
//! StaticHal::init();
//!
//! // Create from PCI transport
//! let device = VirtioNetDevice::<StaticHal, _>::new(transport)?;
//!
//! // Use with smoltcp via DeviceAdapter
//! let adapter = DeviceAdapter::new(device);
//! ```

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::marker::PhantomData;

use virtio_drivers::device::net::VirtIONetRaw;
use virtio_drivers::transport::Transport;
use virtio_drivers::Hal;

use crate::device::NetworkDevice;
use crate::error::{NetworkError, Result};

/// Default virtqueue size for network device.
pub const DEFAULT_QUEUE_SIZE: usize = 16;

/// Maximum transmission unit.
pub const MTU: usize = 1514;

/// VirtIO network header size.
pub const VIRTIO_NET_HDR_SIZE: usize = 12;

/// VirtIO network device wrapper.
///
/// This wraps `virtio_drivers::VirtIONetRaw` and implements our `NetworkDevice`
/// trait for integration with smoltcp.
///
/// # Type Parameters
///
/// - `H`: HAL implementation (UefiHal or BareHal)
/// - `T`: Transport (PciTransport or MmioTransport)
pub struct VirtioNetDevice<H: Hal, T: Transport> {
    /// The underlying VirtIO network driver.
    inner: VirtIONetRaw<H, T, DEFAULT_QUEUE_SIZE>,
    /// Receive buffer pool.
    rx_buffers: Vec<RxBuffer>,
    /// Currently pending RX buffer index.
    pending_rx: Option<usize>,
    /// MAC address cache.
    mac: [u8; 6],
    /// Phantom for HAL type.
    _hal: PhantomData<H>,
}

/// Receive buffer with token tracking.
struct RxBuffer {
    /// Buffer data.
    data: Box<[u8; MTU + VIRTIO_NET_HDR_SIZE]>,
    /// Virtqueue token (if submitted to device).
    token: Option<u16>,
}

impl RxBuffer {
    fn new() -> Self {
        Self {
            data: Box::new([0u8; MTU + VIRTIO_NET_HDR_SIZE]),
            token: None,
        }
    }
}

impl<H: Hal, T: Transport> VirtioNetDevice<H, T> {
    /// Create a new VirtIO network device from a transport.
    ///
    /// # Arguments
    ///
    /// * `transport` - The VirtIO transport (PCI or MMIO)
    ///
    /// # Errors
    ///
    /// Returns an error if device initialization fails.
    pub fn new(transport: T) -> Result<Self> {
        let inner = VirtIONetRaw::new(transport).map_err(|e| {
            NetworkError::DeviceError(alloc::format!("VirtIO init failed: {:?}", e))
        })?;

        let mac = inner.mac_address();

        // Pre-allocate RX buffers
        let mut rx_buffers = Vec::with_capacity(DEFAULT_QUEUE_SIZE);
        for _ in 0..DEFAULT_QUEUE_SIZE {
            rx_buffers.push(RxBuffer::new());
        }

        let mut device = Self {
            inner,
            rx_buffers,
            pending_rx: None,
            mac,
            _hal: PhantomData,
        };

        // Submit initial RX buffers to the device
        device.refill_rx_queue()?;

        Ok(device)
    }

    /// Get the underlying driver for advanced operations.
    pub fn inner(&self) -> &VirtIONetRaw<H, T, DEFAULT_QUEUE_SIZE> {
        &self.inner
    }

    /// Get mutable access to the underlying driver.
    pub fn inner_mut(&mut self) -> &mut VirtIONetRaw<H, T, DEFAULT_QUEUE_SIZE> {
        &mut self.inner
    }

    /// Refill the RX queue with available buffers.
    fn refill_rx_queue(&mut self) -> Result<()> {
        for (idx, buf) in self.rx_buffers.iter_mut().enumerate() {
            if buf.token.is_none() {
                match unsafe { self.inner.receive_begin(buf.data.as_mut_slice()) } {
                    Ok(token) => {
                        buf.token = Some(token);
                    }
                    Err(virtio_drivers::Error::QueueFull) => {
                        // Queue is full, stop refilling
                        break;
                    }
                    Err(e) => {
                        return Err(NetworkError::DeviceError(alloc::format!(
                            "RX submit failed: {:?}",
                            e
                        )));
                    }
                }
                // Track pending for receive polling
                if self.pending_rx.is_none() {
                    self.pending_rx = Some(idx);
                }
            }
        }
        Ok(())
    }

    /// Poll for completed RX buffers.
    ///
    /// Returns (buffer_index, header_len, packet_len) if a packet is ready.
    fn poll_rx(&mut self) -> Option<(usize, usize, usize)> {
        // Use poll_receive to check for completion
        if let Some(token) = self.inner.poll_receive() {
            // Find the buffer with this token
            for (idx, buf) in self.rx_buffers.iter_mut().enumerate() {
                if buf.token == Some(token) {
                    // Complete the receive
                    match unsafe {
                        self.inner
                            .receive_complete(token, buf.data.as_mut_slice())
                    } {
                        Ok((hdr_len, pkt_len)) => {
                            buf.token = None;
                            return Some((idx, hdr_len, pkt_len));
                        }
                        Err(_) => {
                            buf.token = None;
                            return None;
                        }
                    }
                }
            }
        }
        None
    }

    /// Acknowledge interrupt (call after handling packets).
    ///
    /// Returns true if there was a queue notification.
    pub fn ack_interrupt(&mut self) -> bool {
        use virtio_drivers::transport::InterruptStatus;
        let status = self.inner.ack_interrupt();
        status.contains(InterruptStatus::QUEUE_INTERRUPT)
    }
}

impl<H: Hal, T: Transport> NetworkDevice for VirtioNetDevice<H, T> {
    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }

    fn can_transmit(&self) -> bool {
        self.inner.can_send()
    }

    fn can_receive(&self) -> bool {
        // Check if any RX buffer has completed
        for buf in &self.rx_buffers {
            if buf.token.is_some() {
                // Can't check completion without mutable access, assume yes
                return true;
            }
        }
        false
    }

    fn transmit(&mut self, packet: &[u8]) -> Result<()> {
        if packet.len() > MTU {
            return Err(NetworkError::BufferTooSmall);
        }

        // Create TX buffer with header
        let mut tx_buf = vec![0u8; packet.len() + VIRTIO_NET_HDR_SIZE];
        // Header is already zeroed (no offload features)
        tx_buf[VIRTIO_NET_HDR_SIZE..].copy_from_slice(packet);

        // Send and wait for completion (blocking for simplicity)
        self.inner.send(&tx_buf).map_err(|e| {
            NetworkError::DeviceError(alloc::format!("TX failed: {:?}", e))
        })?;

        Ok(())
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>> {
        // Poll for completed RX
        if let Some((idx, hdr_len, pkt_len)) = self.poll_rx() {
            // Received a packet
            let rx_buf = &self.rx_buffers[idx];

            if buffer.len() < pkt_len {
                // Buffer too small, but we still consumed the packet
                // Refill and return error
                let _ = self.refill_rx_queue();
                return Err(NetworkError::BufferTooSmall);
            }

            // Copy packet data (skip header)
            buffer[..pkt_len].copy_from_slice(&rx_buf.data[hdr_len..hdr_len + pkt_len]);

            // Refill the RX queue
            let _ = self.refill_rx_queue();

            return Ok(Some(pkt_len));
        }

        // No packet available
        Ok(None)
    }
}

/// VirtIO PCI device information.
#[derive(Debug, Clone, Copy)]
pub struct VirtioPciInfo {
    /// PCI bus number.
    pub bus: u8,
    /// PCI device number.
    pub device: u8,
    /// PCI function number.
    pub function: u8,
    /// VirtIO device type.
    pub device_type: VirtioDeviceType,
}

/// VirtIO device types we care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioDeviceType {
    /// Network device.
    Network,
    /// Block device.
    Block,
    /// Other/unknown.
    Other(u8),
}

impl From<u8> for VirtioDeviceType {
    fn from(val: u8) -> Self {
        match val {
            1 => VirtioDeviceType::Network,
            2 => VirtioDeviceType::Block,
            other => VirtioDeviceType::Other(other),
        }
    }
}

/// VirtIO vendor ID.
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

/// VirtIO network device IDs.
pub const VIRTIO_NET_DEVICE_ID_LEGACY: u16 = 0x1000;
pub const VIRTIO_NET_DEVICE_ID_MODERN: u16 = 0x1041;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_type_from_u8() {
        assert_eq!(VirtioDeviceType::from(1), VirtioDeviceType::Network);
        assert_eq!(VirtioDeviceType::from(2), VirtioDeviceType::Block);
        assert_eq!(VirtioDeviceType::from(99), VirtioDeviceType::Other(99));
    }

    #[test]
    fn test_constants() {
        assert_eq!(VIRTIO_VENDOR_ID, 0x1AF4);
        assert_eq!(VIRTIO_NET_DEVICE_ID_LEGACY, 0x1000);
        assert_eq!(VIRTIO_NET_DEVICE_ID_MODERN, 0x1041);
        assert_eq!(MTU, 1514);
        assert_eq!(VIRTIO_NET_HDR_SIZE, 12);
    }

    #[test]
    fn test_rx_buffer_new() {
        let buf = RxBuffer::new();
        assert!(buf.token.is_none());
        assert_eq!(buf.data.len(), MTU + VIRTIO_NET_HDR_SIZE);
    }

    #[test]
    fn test_virtio_pci_info() {
        let info = VirtioPciInfo {
            bus: 0,
            device: 3,
            function: 0,
            device_type: VirtioDeviceType::Network,
        };
        assert_eq!(info.bus, 0);
        assert_eq!(info.device, 3);
        assert_eq!(info.device_type, VirtioDeviceType::Network);
    }
}
