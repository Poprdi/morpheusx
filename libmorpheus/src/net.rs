//! Networking. raw NIC frames, hardware knobs, and a smoltcp TCP/IP stack.
//! Use layer 1 for custom protocols, layer 2 for "just give me TCP".
//!
//! # Layers
//!
//! - **Raw functions**: `tcp_socket`, `tcp_connect`, `tcp_send`, etc.
//! - **RAII types**: [`TcpStream`], [`TcpListener`], [`UdpSocket`]
//!   Auto-close on drop, implement [`Read`]/[`Write`] where applicable.
//!
//! # Polling
//!
//! MorpheusX networking is **explicitly polled** — you must call
//! [`net_poll_drive`] periodically to drive DHCP, ARP, TCP timers, etc.
//! The RAII types poll internally where needed, but for event loops
//! you should call it yourself.

use crate::error::{self, Error, ErrorKind};
use crate::io;
use crate::raw::*;

// raw nic

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

// nic hardware control — the deep knobs
//
// These route through SYS_NET_CFG with subcmd >= 128, which dispatches
// directly to the NIC driver's ctrl function pointer.

// nic ctrl subcmds
pub const NIC_CTRL_PROMISC: u32 = 1;
pub const NIC_CTRL_MAC_SET: u32 = 2;
pub const NIC_CTRL_STATS: u32 = 3;
pub const NIC_CTRL_STATS_RESET: u32 = 4;
pub const NIC_CTRL_MTU: u32 = 5;
pub const NIC_CTRL_MULTICAST: u32 = 6;
pub const NIC_CTRL_VLAN: u32 = 7;
pub const NIC_CTRL_TX_CSUM: u32 = 8;
pub const NIC_CTRL_RX_CSUM: u32 = 9;
pub const NIC_CTRL_TSO: u32 = 10;
pub const NIC_CTRL_RX_RING_SIZE: u32 = 11;
pub const NIC_CTRL_TX_RING_SIZE: u32 = 12;
pub const NIC_CTRL_IRQ_COALESCE: u32 = 13;
pub const NIC_CTRL_CAPS: u32 = 14;

// nic capability bits
pub const NIC_CAP_PROMISC: u64 = 1 << 0;
pub const NIC_CAP_MAC_SET: u64 = 1 << 1;
pub const NIC_CAP_MULTICAST: u64 = 1 << 2;
pub const NIC_CAP_VLAN: u64 = 1 << 3;
pub const NIC_CAP_TX_CSUM: u64 = 1 << 4;
pub const NIC_CAP_RX_CSUM: u64 = 1 << 5;
pub const NIC_CAP_TSO: u64 = 1 << 6;
pub const NIC_CAP_IRQ_COALESCE: u64 = 1 << 7;

/// NIC hardware statistics.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NicHwStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    pub rx_dropped: u64,
    pub rx_crc_errors: u64,
    pub collisions: u64,
}

/// Send a raw NIC control command.
///
/// This is the generic entry point.  Prefer the typed wrappers below.
pub fn nic_ctrl(cmd: u32, arg: u64) -> Result<u64, u64> {
    let subcmd = 128 + cmd as u64;
    let ret = unsafe { syscall3(SYS_NET_CFG, subcmd, arg, 0) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Enable or disable promiscuous mode.
pub fn nic_set_promisc(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_PROMISC, on as u64).map(|_| ())
}

/// Set (spoof) the NIC's hardware MAC address.
pub fn nic_set_mac(mac: &[u8; 6]) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_MAC_SET, mac.as_ptr() as u64).map(|_| ())
}

/// Read NIC hardware statistics.
pub fn nic_hw_stats() -> Result<NicHwStats, u64> {
    let mut stats = NicHwStats::default();
    nic_ctrl(NIC_CTRL_STATS, &mut stats as *mut NicHwStats as u64)?;
    Ok(stats)
}

/// Reset NIC hardware statistics counters.
pub fn nic_stats_reset() -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_STATS_RESET, 0).map(|_| ())
}

/// Set NIC MTU.
pub fn nic_set_mtu(mtu: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_MTU, mtu as u64).map(|_| ())
}

/// Enable/disable accepting all multicast frames.
pub fn nic_set_multicast(accept_all: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_MULTICAST, accept_all as u64).map(|_| ())
}

/// Set VLAN tag (0 = disable).
pub fn nic_set_vlan(vlan_id: u16) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_VLAN, vlan_id as u64).map(|_| ())
}

/// Enable/disable TX checksum offload.
pub fn nic_set_tx_csum(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_TX_CSUM, on as u64).map(|_| ())
}

/// Enable/disable RX checksum offload.
pub fn nic_set_rx_csum(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_RX_CSUM, on as u64).map(|_| ())
}

/// Enable/disable TCP Segmentation Offload.
pub fn nic_set_tso(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_TSO, on as u64).map(|_| ())
}

/// Set RX ring buffer size (number of descriptors).
pub fn nic_set_rx_ring(descriptors: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_RX_RING_SIZE, descriptors as u64).map(|_| ())
}

/// Set TX ring buffer size (number of descriptors).
pub fn nic_set_tx_ring(descriptors: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_TX_RING_SIZE, descriptors as u64).map(|_| ())
}

/// Set interrupt coalescing interval (microseconds).
pub fn nic_set_irq_coalesce(usec: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_IRQ_COALESCE, usec as u64).map(|_| ())
}

/// Query NIC hardware capabilities bitmask.
pub fn nic_caps() -> Result<u64, u64> {
    let mut caps: u64 = 0;
    nic_ctrl(NIC_CTRL_CAPS, &mut caps as *mut u64 as u64)?;
    Ok(caps)
}

// tcp sockets (smoltcp)

// subcmds (must match kernel)
const NET_TCP_SOCKET: u64 = 0;
const NET_TCP_CONNECT: u64 = 1;
const NET_TCP_SEND: u64 = 2;
const NET_TCP_RECV: u64 = 3;
const NET_TCP_CLOSE: u64 = 4;
const NET_TCP_STATE: u64 = 5;
const NET_TCP_LISTEN: u64 = 6;
const NET_TCP_ACCEPT: u64 = 7;
const NET_TCP_SHUTDOWN: u64 = 8;
const NET_TCP_NODELAY: u64 = 9;
const NET_TCP_KEEPALIVE: u64 = 10;

/// Opaque TCP socket handle.
pub type TcpHandle = u64;

/// TCP socket state (matches smoltcp TcpState ordinals).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TcpState {
    Closed = 0,
    Listen = 1,
    SynSent = 2,
    SynReceived = 3,
    Established = 4,
    FinWait1 = 5,
    FinWait2 = 6,
    CloseWait = 7,
    Closing = 8,
    LastAck = 9,
    TimeWait = 10,
}

impl TcpState {
    pub fn from_raw(v: u64) -> Self {
        match v {
            1 => Self::Listen,
            2 => Self::SynSent,
            3 => Self::SynReceived,
            4 => Self::Established,
            5 => Self::FinWait1,
            6 => Self::FinWait2,
            7 => Self::CloseWait,
            8 => Self::Closing,
            9 => Self::LastAck,
            10 => Self::TimeWait,
            _ => Self::Closed,
        }
    }
}

/// Create a new TCP socket.
pub fn tcp_socket() -> Result<TcpHandle, u64> {
    let ret = unsafe { syscall1(SYS_NET, NET_TCP_SOCKET) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Connect a TCP socket to a remote host.
///
/// `ip` is a 4-byte IPv4 address in **network byte order** (big-endian).
pub fn tcp_connect(handle: TcpHandle, ip: u32, port: u16) -> Result<(), u64> {
    let ret = unsafe { syscall4(SYS_NET, NET_TCP_CONNECT, handle, ip as u64, port as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Connect using a dotted-decimal IPv4 address.
pub fn tcp_connect_ip(handle: TcpHandle, a: u8, b: u8, c: u8, d: u8, port: u16) -> Result<(), u64> {
    let ip = u32::from_be_bytes([a, b, c, d]);
    tcp_connect(handle, ip, port)
}

/// Send data on a connected TCP socket.
///
/// Returns the number of bytes accepted into the send buffer.
pub fn tcp_send(handle: TcpHandle, data: &[u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall4(
            SYS_NET,
            NET_TCP_SEND,
            handle,
            data.as_ptr() as u64,
            data.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Receive data from a connected TCP socket.
///
/// Returns the number of bytes received (0 if no data available yet).
pub fn tcp_recv(handle: TcpHandle, buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall4(
            SYS_NET,
            NET_TCP_RECV,
            handle,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Close a TCP socket (sends FIN, initiates graceful shutdown).
pub fn tcp_close(handle: TcpHandle) {
    unsafe {
        syscall2(SYS_NET, NET_TCP_CLOSE, handle);
    }
}

/// Query a TCP socket's current state.
pub fn tcp_state(handle: TcpHandle) -> Result<TcpState, u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_STATE, handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(TcpState::from_raw(ret))
    }
}

/// Bind a socket and start listening for incoming connections.
pub fn tcp_listen(handle: TcpHandle, port: u16) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_LISTEN, handle, port as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Accept an incoming connection on a listening socket.
///
/// Returns a new handle for the accepted connection, or an error
/// if no connection is pending.
pub fn tcp_accept(listen_handle: TcpHandle) -> Result<TcpHandle, u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_ACCEPT, listen_handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Shutdown the write half of a TCP connection.
pub fn tcp_shutdown(handle: TcpHandle) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_SHUTDOWN, handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Set TCP_NODELAY (disable Nagle's algorithm).
pub fn tcp_set_nodelay(handle: TcpHandle, on: bool) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_NODELAY, handle, on as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Set TCP keepalive interval (0 = disable).
pub fn tcp_set_keepalive(handle: TcpHandle, interval_ms: u64) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_KEEPALIVE, handle, interval_ms) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// dns

const DNS_START_CMD: u64 = 0;
const DNS_RESULT_CMD: u64 = 1;
const DNS_SET_SERVERS_CMD: u64 = 2;

/// Opaque DNS query handle.
pub type DnsQuery = u64;

/// Start an asynchronous DNS lookup.
///
/// Returns a query handle.  Call [`dns_poll`] in a loop until resolved.
pub fn dns_start(hostname: &str) -> Result<DnsQuery, u64> {
    let ret = unsafe {
        syscall3(
            SYS_DNS,
            DNS_START_CMD,
            hostname.as_ptr() as u64,
            hostname.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Poll a DNS query.
///
/// Returns `Ok(Some(ip_nbo))` when resolved, `Ok(None)` if still pending,
/// or `Err` on failure.
pub fn dns_poll(query: DnsQuery) -> Result<Option<u32>, u64> {
    let mut ip: u32 = 0;
    let ret = unsafe { syscall3(SYS_DNS, DNS_RESULT_CMD, query, &mut ip as *mut u32 as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else if ret == 1 {
        Ok(None) // pending
    } else {
        Ok(Some(ip)) // resolved
    }
}

/// Blocking DNS resolve (polls the stack in a loop).
///
/// Returns the IPv4 address in network byte order.
pub fn dns_resolve(hostname: &str) -> Result<u32, u64> {
    let query = dns_start(hostname)?;
    loop {
        // Drive the stack
        net_poll_drive(0);
        match dns_poll(query)? {
            Some(ip) => return Ok(ip),
            None => {
                // PERF FIX: sleep(1) instead of yield to avoid spinning
                // when this is the only runnable process.
                crate::process::sleep(1);
            }
        }
    }
}

/// Override the DNS server list.
///
/// `servers` is a slice of IPv4 addresses in network byte order (max 4).
pub fn dns_set_servers(servers: &[u32]) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(
            SYS_DNS,
            DNS_SET_SERVERS_CMD,
            servers.as_ptr() as u64,
            servers.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// stack config

const CFG_GET: u64 = 0;
const CFG_DHCP: u64 = 1;
const CFG_STATIC: u64 = 2;
const CFG_HOSTNAME: u64 = 3;

/// Network configuration snapshot.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetConfigInfo {
    pub state: u32,
    pub flags: u32,
    pub ipv4_addr: u32,
    pub prefix_len: u8,
    pub _pad0: [u8; 3],
    pub gateway: u32,
    pub dns_primary: u32,
    pub dns_secondary: u32,
    pub mac: [u8; 6],
    pub _pad1: [u8; 2],
    pub mtu: u32,
    pub hostname: [u8; 64],
}

/// Stack states.
pub const NET_STATE_UNCONFIGURED: u32 = 0;
pub const NET_STATE_DHCP_DISCOVERING: u32 = 1;
pub const NET_STATE_READY: u32 = 2;
pub const NET_STATE_ERROR: u32 = 3;

/// Config flags.
pub const NET_FLAG_DHCP: u32 = 1 << 0;
pub const NET_FLAG_HAS_GATEWAY: u32 = 1 << 1;
pub const NET_FLAG_HAS_DNS: u32 = 1 << 2;

/// Get the current network configuration.
pub fn net_config() -> Result<NetConfigInfo, u64> {
    let mut info = unsafe { core::mem::zeroed::<NetConfigInfo>() };
    let ret = unsafe { syscall2(SYS_NET_CFG, CFG_GET, &mut info as *mut NetConfigInfo as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(info)
    }
}

/// Switch to DHCP mode.
pub fn net_dhcp() -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_NET_CFG, CFG_DHCP) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Set a static IPv4 address.
///
/// `ip`, `gateway` are in network byte order.  `prefix_len` is the
/// CIDR prefix (e.g. 24 for /24).
pub fn net_static_ip(ip: u32, prefix_len: u8, gateway: u32) -> Result<(), u64> {
    // Pack prefix_len and gateway into a single u64:
    // bits [63:32] = prefix_len, bits [31:0] = gateway
    let packed = ((prefix_len as u64) << 32) | (gateway as u64);
    let ret = unsafe { syscall3(SYS_NET_CFG, CFG_STATIC, ip as u64, packed) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Set the hostname (for DHCP FQDN, mDNS, etc.).
pub fn net_set_hostname(hostname: &str) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(
            SYS_NET_CFG,
            CFG_HOSTNAME,
            hostname.as_ptr() as u64,
            hostname.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// stack polling

const POLL_DRIVE: u64 = 0;
const POLL_STATS: u64 = 1;

/// Network stack statistics.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NetStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    pub tcp_active: u32,
    pub _pad: u32,
}

/// Drive the network stack (DHCP, ARP, TCP timers, etc.).
///
/// Call this periodically to keep the stack alive.  Returns `true` if
/// any socket activity occurred.
pub fn net_poll_drive(timestamp_ms: u64) -> bool {
    let ret = unsafe { syscall2(SYS_NET_POLL, POLL_DRIVE, timestamp_ms) };
    ret == 1
}

/// Get network stack statistics.
pub fn net_stats() -> Result<NetStats, u64> {
    let mut stats = NetStats::default();
    let ret = unsafe { syscall2(SYS_NET_POLL, POLL_STATS, &mut stats as *mut NetStats as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(stats)
    }
}

// ipv4 helpers

/// Convert a dotted-decimal IPv4 to network byte order u32.
pub const fn ipv4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_be_bytes([a, b, c, d])
}

/// Convert a network byte order u32 to 4-byte array.
pub const fn ipv4_bytes(ip: u32) -> [u8; 4] {
    ip.to_be_bytes()
}

// udp sockets

/// UDP subcmd constants (must match hwinit/src/syscall/handler.rs).
const UDP_SOCKET: u64 = 11;
const UDP_SEND_TO: u64 = 12;
const UDP_RECV_FROM: u64 = 13;
const UDP_CLOSE: u64 = 14;

/// Descriptor passed to `SYS_NET(UDP_SEND_TO, handle, &desc, 0)`.
///
/// The kernel reads this struct from the pointer in `a3`.
#[repr(C)]
pub struct UdpSendDesc {
    pub ip: u32,
    pub port: u16,
    pub _pad: u16,
    pub buf: *const u8,
    pub len: u64,
}

/// Descriptor passed to `SYS_NET(UDP_RECV_FROM, handle, &desc, 0)`.
///
/// The kernel reads `buf`/`buf_len`, then writes back `src_ip`/`src_port`
/// after a successful receive.
#[repr(C)]
pub struct UdpRecvDesc {
    pub buf: *mut u8,
    pub buf_len: u64,
    pub src_ip: u32,
    pub src_port: u16,
    pub _pad: u16,
}

/// Open a UDP socket.
///
/// Returns a handle on success.
pub fn udp_socket() -> Result<u64, u64> {
    let ret = unsafe { syscall2(SYS_NET, UDP_SOCKET, 0) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Send a UDP datagram to `ip:port`.
///
/// `handle` is the value returned by [`udp_socket`].
/// `ip` is in network byte order; `port` in host byte order.
///
/// Returns 0 on success.
pub fn udp_send_to(handle: u64, ip: u32, port: u16, data: &[u8]) -> Result<(), u64> {
    let desc = UdpSendDesc {
        ip,
        port,
        _pad: 0,
        buf: data.as_ptr(),
        len: data.len() as u64,
    };
    let ret = unsafe {
        syscall4(
            SYS_NET,
            UDP_SEND_TO,
            handle,
            &desc as *const UdpSendDesc as u64,
            0,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Receive a UDP datagram.
///
/// On success returns `(bytes_received, src_ip, src_port)`.
/// `src_ip` is in network byte order.
pub fn udp_recv_from(handle: u64, buf: &mut [u8]) -> Result<(u64, u32, u16), u64> {
    let mut desc = UdpRecvDesc {
        buf: buf.as_mut_ptr(),
        buf_len: buf.len() as u64,
        src_ip: 0,
        src_port: 0,
        _pad: 0,
    };
    let ret = unsafe {
        syscall4(
            SYS_NET,
            UDP_RECV_FROM,
            handle,
            &mut desc as *mut UdpRecvDesc as u64,
            0,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok((ret, desc.src_ip, desc.src_port))
    }
}

/// Close a UDP socket.
pub fn udp_close(handle: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_NET, UDP_CLOSE, handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════
// IPv4 address type
// ═══════════════════════════════════════════════════════════════════════

/// An IPv4 address.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Addr {
    octets: [u8; 4],
}

impl Ipv4Addr {
    pub const LOCALHOST: Self = Self::new(127, 0, 0, 1);
    pub const UNSPECIFIED: Self = Self::new(0, 0, 0, 0);
    pub const BROADCAST: Self = Self::new(255, 255, 255, 255);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self {
            octets: [a, b, c, d],
        }
    }

    /// Create from network byte order u32.
    pub const fn from_nbo(nbo: u32) -> Self {
        let b = nbo.to_be_bytes();
        Self::new(b[0], b[1], b[2], b[3])
    }

    /// Convert to network byte order u32.
    pub const fn to_nbo(self) -> u32 {
        u32::from_be_bytes(self.octets)
    }

    pub const fn octets(&self) -> [u8; 4] {
        self.octets
    }
}

impl core::fmt::Debug for Ipv4Addr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let [a, b, c, d] = self.octets;
        write!(f, "{}.{}.{}.{}", a, b, c, d)
    }
}

impl core::fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let [a, b, c, d] = self.octets;
        write!(f, "{}.{}.{}.{}", a, b, c, d)
    }
}

/// A socket address: IPv4 + port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketAddr {
    pub ip: Ipv4Addr,
    pub port: u16,
}

impl SocketAddr {
    pub const fn new(ip: Ipv4Addr, port: u16) -> Self {
        Self { ip, port }
    }
}

impl core::fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// TcpStream — RAII TCP client socket
// ═══════════════════════════════════════════════════════════════════════

/// A connected TCP stream.  Closes the socket on drop.
///
/// Implements [`Read`](io::Read) and [`Write`](io::Write).
/// The kernel's TCP stack is **non-blocking** — reads return 0 if no data
/// is available, sends may accept fewer bytes than provided.
///
/// # Example
/// ```ignore
/// use libmorpheus::net::TcpStream;
/// let mut stream = TcpStream::connect(Ipv4Addr::new(10,0,2,2), 80)?;
/// stream.write_all(b"GET / HTTP/1.0\r\n\r\n")?;
/// let mut buf = [0u8; 1024];
/// let n = stream.read(&mut buf)?;
/// ```
pub struct TcpStream {
    handle: TcpHandle,
}

impl TcpStream {
    /// Create a TCP socket and connect to `ip:port`.
    ///
    /// This initiates the TCP handshake.  The connection is established
    /// asynchronously — poll [`state()`](Self::state) or just start
    /// reading/writing (the kernel buffers data during handshake).
    pub fn connect(ip: Ipv4Addr, port: u16) -> error::Result<Self> {
        let handle = tcp_socket().map_err(Error::from_raw)?;
        tcp_connect(handle, ip.to_nbo(), port).map_err(|e| {
            tcp_close(handle);
            Error::from_raw(e)
        })?;
        Ok(Self { handle })
    }

    /// Connect using a hostname (DNS resolution + TCP connect).
    pub fn connect_host(hostname: &str, port: u16) -> error::Result<Self> {
        let ip_nbo = dns_resolve(hostname).map_err(Error::from_raw)?;
        let handle = tcp_socket().map_err(Error::from_raw)?;
        tcp_connect(handle, ip_nbo, port).map_err(|e| {
            tcp_close(handle);
            Error::from_raw(e)
        })?;
        Ok(Self { handle })
    }

    /// Wrap an existing raw handle (e.g. from [`TcpListener::accept`]).
    pub fn from_raw_handle(handle: TcpHandle) -> Self {
        Self { handle }
    }

    /// Return the raw handle without closing it.
    pub fn into_raw_handle(self) -> TcpHandle {
        let h = self.handle;
        core::mem::forget(self);
        h
    }

    /// The raw socket handle.
    pub fn handle(&self) -> TcpHandle {
        self.handle
    }

    /// Query the TCP state machine.
    pub fn state(&self) -> error::Result<TcpState> {
        tcp_state(self.handle).map_err(Error::from_raw)
    }

    /// Shutdown the write half (sends FIN).
    pub fn shutdown(&self) -> error::Result<()> {
        tcp_shutdown(self.handle).map_err(Error::from_raw)
    }

    /// Set TCP_NODELAY (disable Nagle's algorithm).
    pub fn set_nodelay(&self, on: bool) -> error::Result<()> {
        tcp_set_nodelay(self.handle, on).map_err(Error::from_raw)
    }

    /// Set keepalive interval (0 = disable).
    pub fn set_keepalive(&self, interval_ms: u64) -> error::Result<()> {
        tcp_set_keepalive(self.handle, interval_ms).map_err(Error::from_raw)
    }

    /// Wait until fully connected or an error occurs.
    ///
    /// Polls the stack in a loop.  Use for blocking-style programming.
    pub fn wait_connected(&self) -> error::Result<()> {
        loop {
            net_poll_drive(0);
            match self.state()? {
                TcpState::Established => return Ok(()),
                TcpState::Closed => return Err(Error::new(ErrorKind::ConnectionRefused)),
                _ => {
                    // PERF FIX: sleep(1) instead of yield
                    crate::process::sleep(1);
                }
            }
        }
    }

    /// Blocking send: poll + retry until all data is sent or error.
    pub fn send_all(&self, mut data: &[u8]) -> error::Result<()> {
        while !data.is_empty() {
            net_poll_drive(0);
            match tcp_send(self.handle, data) {
                Ok(0) => {
                    // Check if connection is still alive.
                    match self.state()? {
                        TcpState::Established | TcpState::CloseWait => {
                            // PERF FIX: sleep(1) instead of yield
                            crate::process::sleep(1);
                        }
                        _ => return Err(Error::new(ErrorKind::BrokenPipe)),
                    }
                }
                Ok(n) => data = &data[n..],
                Err(e) => return Err(Error::from_raw(e)),
            }
        }
        Ok(())
    }

    /// Blocking receive: poll until at least 1 byte arrives or EOF/error.
    pub fn recv_blocking(&self, buf: &mut [u8]) -> error::Result<usize> {
        loop {
            net_poll_drive(0);
            match tcp_recv(self.handle, buf) {
                Ok(0) => {
                    match self.state()? {
                        TcpState::Established | TcpState::SynSent | TcpState::SynReceived => {
                            // PERF FIX: sleep(1) instead of yield
                            crate::process::sleep(1);
                        }
                        _ => return Ok(0), // EOF
                    }
                }
                Ok(n) => return Ok(n),
                Err(e) => return Err(Error::from_raw(e)),
            }
        }
    }
}

impl io::Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
        net_poll_drive(0);
        tcp_recv(self.handle, buf).map_err(Error::from_raw)
    }
}

impl io::Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
        net_poll_drive(0);
        tcp_send(self.handle, buf).map_err(Error::from_raw)
    }

    fn flush(&mut self) -> error::Result<()> {
        net_poll_drive(0);
        Ok(())
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        tcp_close(self.handle);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// TcpListener — RAII TCP server socket
// ═══════════════════════════════════════════════════════════════════════

/// A TCP listener that accepts incoming connections.
///
/// # Example
/// ```ignore
/// let listener = TcpListener::bind(8080)?;
/// loop {
///     if let Ok(stream) = listener.accept() {
///         // handle connection
///     }
///     net_poll_drive(0);
/// }
/// ```
pub struct TcpListener {
    handle: TcpHandle,
}

impl TcpListener {
    /// Create a socket and start listening on `port`.
    pub fn bind(port: u16) -> error::Result<Self> {
        let handle = tcp_socket().map_err(Error::from_raw)?;
        tcp_listen(handle, port).map_err(|e| {
            tcp_close(handle);
            Error::from_raw(e)
        })?;
        Ok(Self { handle })
    }

    /// Accept a pending connection.
    ///
    /// Returns `Err(WouldBlock)` if no connection is pending.
    pub fn accept(&self) -> error::Result<TcpStream> {
        net_poll_drive(0);
        match tcp_accept(self.handle) {
            Ok(new_handle) => Ok(TcpStream::from_raw_handle(new_handle)),
            Err(e) => Err(Error::from_raw(e)),
        }
    }

    /// Blocking accept: wait until a connection arrives.
    pub fn accept_blocking(&self) -> error::Result<TcpStream> {
        loop {
            match self.accept() {
                Ok(stream) => return Ok(stream),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    // PERF FIX: sleep(1) instead of yield
                    crate::process::sleep(1);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// The raw socket handle.
    pub fn handle(&self) -> TcpHandle {
        self.handle
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        tcp_close(self.handle);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// UdpSocket — RAII UDP socket
// ═══════════════════════════════════════════════════════════════════════

/// A UDP socket.  Closes on drop.
///
/// # Example
/// ```ignore
/// let sock = UdpSocket::new()?;
/// sock.send_to(Ipv4Addr::new(10,0,2,2), 53, &query)?;
/// let (n, src_ip, src_port) = sock.recv_from(&mut buf)?;
/// ```
pub struct UdpSocket {
    handle: u64,
}

impl UdpSocket {
    /// Create a new UDP socket.
    pub fn new() -> error::Result<Self> {
        let handle = udp_socket().map_err(Error::from_raw)?;
        Ok(Self { handle })
    }

    /// Wrap an existing handle.
    pub fn from_raw_handle(handle: u64) -> Self {
        Self { handle }
    }

    /// Send a datagram to `ip:port`.
    pub fn send_to(&self, ip: Ipv4Addr, port: u16, data: &[u8]) -> error::Result<()> {
        net_poll_drive(0);
        udp_send_to(self.handle, ip.to_nbo(), port, data).map_err(Error::from_raw)
    }

    /// Receive a datagram.  Returns `(bytes_read, src_ip, src_port)`.
    pub fn recv_from(&self, buf: &mut [u8]) -> error::Result<(usize, Ipv4Addr, u16)> {
        net_poll_drive(0);
        let (n, ip_nbo, port) = udp_recv_from(self.handle, buf).map_err(Error::from_raw)?;
        Ok((n as usize, Ipv4Addr::from_nbo(ip_nbo), port))
    }

    /// Blocking receive: poll until a datagram arrives.
    pub fn recv_from_blocking(&self, buf: &mut [u8]) -> error::Result<(usize, Ipv4Addr, u16)> {
        loop {
            net_poll_drive(0);
            match udp_recv_from(self.handle, buf) {
                Ok((0, _, _)) => {
                    // PERF FIX: sleep(1) instead of yield
                    crate::process::sleep(1);
                }
                Ok((n, ip_nbo, port)) => {
                    return Ok((n as usize, Ipv4Addr::from_nbo(ip_nbo), port));
                }
                Err(e) => return Err(Error::from_raw(e)),
            }
        }
    }

    /// The raw socket handle.
    pub fn handle(&self) -> u64 {
        self.handle
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let _ = udp_close(self.handle);
    }
}
