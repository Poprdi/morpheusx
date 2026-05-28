//! smoltcp integration: `DeviceAdapter` bridges `NetworkDevice` to smoltcp's
//! `Device` trait; `NetInterface` is the full IP stack.

mod interface;

use core::marker::PhantomData;
use morpheus_nic::device::NetworkDevice;
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

pub use interface::{NetConfig, NetInterface, NetState, MAX_TCP_SOCKETS};
pub use morpheus_nic::device::pci::ecam_bases;
pub use smoltcp::iface::SocketHandle;
pub use smoltcp::socket::dns::QueryHandle as DnsQueryHandle;

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
    type RxToken<'a>
        = AdapterRxToken<'a, D>
    where
        D: 'a;
    type TxToken<'a>
        = AdapterTxToken<'a, D>
    where
        D: 'a;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = MTU;
        caps.medium = Medium::Ethernet;
        caps
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Only hand out tokens when a packet is actually waiting.
        let mut temp_buf = [0u8; MTU];
        match self.inner.receive(&mut temp_buf) {
            Ok(Some(len)) if len > 0 => {
                RX_PACKET_COUNTER.increment();

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
            },
            _ => None,
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

        TX_PACKET_COUNTER.increment();

        // Count DHCP DISCOVERs: Ethernet(IPv4) + IP(UDP) + UDP dst port 67.
        if len >= 42 {
            let ethertype = u16::from_be_bytes([buffer[12], buffer[13]]);
            if ethertype == 0x0800 {
                let protocol = buffer[23];
                if protocol == 17 {
                    let dst_port = u16::from_be_bytes([buffer[36], buffer[37]]);
                    if dst_port == 67 {
                        DHCP_DISCOVER_COUNTER.increment();
                    }
                }
            }
        }

        // Must return `result` regardless of TX outcome; smoltcp expects it.
        match unsafe { (*self.device).transmit(&buffer[..len]) } {
            Ok(()) => {},
            Err(_e) => {
                TX_ERROR_COUNTER.increment();
            },
        }

        result
    }
}

const PACKET_COUNTER_SIZE: usize = 256;

struct PacketCounterRing {
    head: core::sync::atomic::AtomicUsize,
    total: core::sync::atomic::AtomicU32,
}

impl PacketCounterRing {
    const fn new() -> Self {
        Self {
            head: core::sync::atomic::AtomicUsize::new(0),
            total: core::sync::atomic::AtomicU32::new(0),
        }
    }

    fn increment(&self) {
        self.head
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        self.total
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }

    fn count(&self) -> u32 {
        self.total.load(core::sync::atomic::Ordering::Relaxed)
    }

    fn reset(&self) {
        self.head.store(0, core::sync::atomic::Ordering::Relaxed);
        self.total.store(0, core::sync::atomic::Ordering::Relaxed);
    }
}

static TX_ERROR_COUNTER: PacketCounterRing = PacketCounterRing::new();
static TX_PACKET_COUNTER: PacketCounterRing = PacketCounterRing::new();
static DHCP_DISCOVER_COUNTER: PacketCounterRing = PacketCounterRing::new();
static RX_PACKET_COUNTER: PacketCounterRing = PacketCounterRing::new();

static DEBUG_INIT_STAGE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

const DEBUG_MSG_LEN: usize = 64;
const DEBUG_RING_SIZE: usize = 16;

#[derive(Clone, Copy)]
pub struct DebugLogEntry {
    pub msg: [u8; DEBUG_MSG_LEN],
    pub len: usize,
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

/// Log to serial and push to the ring buffer.
pub fn debug_log(stage: u32, msg: &str) {
    // Serial first — visible even if the ring lock is wedged.
    #[cfg(target_arch = "x86_64")]
    {
        use morpheus_hal_x86_64::serial::{putc, puts, puts_dec_u32};
        puts("[NET:");
        puts_dec_u32(stage);
        puts("] ");
        puts(msg);
        putc(b'\n');
    }

    // Bounded try-lock: single-threaded, so a held lock means a bug; skip rather than spin.
    if DEBUG_RING_LOCK.swap(true, core::sync::atomic::Ordering::Acquire) {
        return;
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
            // Full: overwrite oldest.
            DEBUG_RING.read_idx = (DEBUG_RING.read_idx + 1) % DEBUG_RING_SIZE;
        }
    }

    DEBUG_RING_LOCK.store(false, core::sync::atomic::Ordering::Release);
}

/// Pop the oldest debug message, or None if empty.
pub fn debug_log_pop() -> Option<DebugLogEntry> {
    if DEBUG_RING_LOCK.swap(true, core::sync::atomic::Ordering::Acquire) {
        return None;
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

pub fn debug_log_available() -> bool {
    unsafe { DEBUG_RING.count > 0 }
}

pub fn debug_log_clear() {
    if DEBUG_RING_LOCK.swap(true, core::sync::atomic::Ordering::Acquire) {
        return;
    }

    unsafe {
        DEBUG_RING.write_idx = 0;
        DEBUG_RING.read_idx = 0;
        DEBUG_RING.count = 0;
    }

    DEBUG_RING_LOCK.store(false, core::sync::atomic::Ordering::Release);
}

pub fn set_debug_stage(stage: u32) {
    DEBUG_INIT_STAGE.store(stage, core::sync::atomic::Ordering::Relaxed);

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

pub fn debug_stage() -> u32 {
    DEBUG_INIT_STAGE.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn tx_error_count() -> u32 {
    TX_ERROR_COUNTER.count()
}

pub fn tx_packet_count() -> u32 {
    TX_PACKET_COUNTER.count()
}

pub fn dhcp_discover_count() -> u32 {
    DHCP_DISCOVER_COUNTER.count()
}

pub fn rx_packet_count() -> u32 {
    RX_PACKET_COUNTER.count()
}
