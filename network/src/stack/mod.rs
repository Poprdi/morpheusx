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

// Debug marker for tracing initialization steps
static DEBUG_INIT_STAGE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

// ============================================================================
// Debug Log Ring Buffer
// ============================================================================

/// Maximum length of a single debug message
const DEBUG_MSG_LEN: usize = 64;
/// Number of messages in the ring buffer
const DEBUG_RING_SIZE: usize = 16;

/// A single debug log entry
#[derive(Clone, Copy)]
pub struct DebugLogEntry {
    /// Message content (null-terminated or full)
    pub msg: [u8; DEBUG_MSG_LEN],
    /// Actual length of message
    pub len: usize,
    /// Stage number when this was logged
    pub stage: u32,
}

impl Default for DebugLogEntry {
    fn default() -> Self {
        Self {
            msg: [0u8; DEBUG_MSG_LEN],
            len: 0,
            stage: 0,
        }
    }
}

/// Ring buffer for debug logs
struct DebugRing {
    entries: [DebugLogEntry; DEBUG_RING_SIZE],
    write_idx: usize,
    read_idx: usize,
    count: usize,
}

impl DebugRing {
    const fn new() -> Self {
        Self {
            entries: [DebugLogEntry {
                msg: [0u8; DEBUG_MSG_LEN],
                len: 0,
                stage: 0,
            }; DEBUG_RING_SIZE],
            write_idx: 0,
            read_idx: 0,
            count: 0,
        }
    }
}

static mut DEBUG_RING: DebugRing = DebugRing::new();
static DEBUG_RING_LOCK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Push a debug message to the ring buffer
pub fn debug_log(stage: u32, msg: &str) {
    // Simple spinlock
    while DEBUG_RING_LOCK.swap(true, core::sync::atomic::Ordering::Acquire) {
        core::hint::spin_loop();
    }
    
    unsafe {
        let entry = &mut DEBUG_RING.entries[DEBUG_RING.write_idx];
        let bytes = msg.as_bytes();
        let copy_len = bytes.len().min(DEBUG_MSG_LEN);
        entry.msg[..copy_len].copy_from_slice(&bytes[..copy_len]);
        entry.len = copy_len;
        entry.stage = stage;
        
        DEBUG_RING.write_idx = (DEBUG_RING.write_idx + 1) % DEBUG_RING_SIZE;
        if DEBUG_RING.count < DEBUG_RING_SIZE {
            DEBUG_RING.count += 1;
        } else {
            // Overwrite oldest - advance read pointer
            DEBUG_RING.read_idx = (DEBUG_RING.read_idx + 1) % DEBUG_RING_SIZE;
        }
    }
    
    DEBUG_RING_LOCK.store(false, core::sync::atomic::Ordering::Release);
}

/// Pop the next debug message from the ring buffer (FIFO order)
/// Returns None if buffer is empty
pub fn debug_log_pop() -> Option<DebugLogEntry> {
    // Simple spinlock
    while DEBUG_RING_LOCK.swap(true, core::sync::atomic::Ordering::Acquire) {
        core::hint::spin_loop();
    }
    
    let result = unsafe {
        if DEBUG_RING.count == 0 {
            None
        } else {
            let entry = DEBUG_RING.entries[DEBUG_RING.read_idx];
            DEBUG_RING.read_idx = (DEBUG_RING.read_idx + 1) % DEBUG_RING_SIZE;
            DEBUG_RING.count -= 1;
            Some(entry)
        }
    };
    
    DEBUG_RING_LOCK.store(false, core::sync::atomic::Ordering::Release);
    result
}

/// Check if there are pending debug messages
pub fn debug_log_available() -> bool {
    unsafe { DEBUG_RING.count > 0 }
}

/// Clear all debug messages
pub fn debug_log_clear() {
    while DEBUG_RING_LOCK.swap(true, core::sync::atomic::Ordering::Acquire) {
        core::hint::spin_loop();
    }
    
    unsafe {
        DEBUG_RING.write_idx = 0;
        DEBUG_RING.read_idx = 0;
        DEBUG_RING.count = 0;
    }
    
    DEBUG_RING_LOCK.store(false, core::sync::atomic::Ordering::Release);
}

/// Set debug init stage and log it
pub fn set_debug_stage(stage: u32) {
    DEBUG_INIT_STAGE.store(stage, core::sync::atomic::Ordering::Relaxed);
    
    // Also log to ring buffer with description
    let desc = match stage {
        10 => "entered NetInterface::new",
        11 => "got MAC address",
        12 => "created DeviceAdapter",
        13 => "created smoltcp Config",
        14 => "about to create Interface",
        15 => "Interface created",
        16 => "SocketSet created",
        17 => "about to create DNS socket",
        18 => "DNS socket added",
        19 => "creating DHCP socket",
        20 => "DHCP socket added",
        25 => "returning from new()",
        _ => "unknown",
    };
    debug_log(stage, desc);
}

/// Get current debug init stage.
pub fn debug_stage() -> u32 {
    DEBUG_INIT_STAGE.load(core::sync::atomic::Ordering::Relaxed)
}

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
