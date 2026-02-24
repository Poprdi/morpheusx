//! Networking — exokernel NIC + IP stack API.
//!
//! MorpheusX provides two complementary networking layers:
//!
//! ## Layer 1: Raw NIC (syscalls 32-37)
//! Direct Ethernet frame TX/RX, MAC read, link status, descriptor refill.
//! Use these to build completely custom protocol stacks in userspace.
//!
//! ## Layer 1.5: NIC Hardware Control (via `nic_ctrl`)
//! Promiscuous mode, MAC spoofing, VLAN tags, checksum/TSO offloads,
//! ring buffer sizing, interrupt coalescing, capability queries.
//! Full exokernel hardware control from Ring 3.
//!
//! ## Layer 2: TCP/IP Stack (syscalls 38-41)
//! Kernel-side smoltcp stack: TCP sockets, DNS resolution, DHCP, static
//! IP config, hostname, stack polling.  Programs that just want TCP/IP
//! use this layer.  TLS runs in userspace on top of `tcp_send`/`tcp_recv`.
//!
//! Both layers coexist.  A program can use raw NIC for custom protocols
//! (ARP probing, ICMP, custom L2) while simultaneously using the TCP
//! stack for HTTP connections.

use crate::raw::*;

// ═══════════════════════════════════════════════════════════════════════════
// RAW NIC (Layer 1)
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
// NIC HARDWARE CONTROL (Layer 1.5) — exokernel escape hatch
// ═══════════════════════════════════════════════════════════════════════════
//
// These route through SYS_NET_CFG with subcmd >= 128, which dispatches
// directly to the NIC driver's ctrl function pointer.

// NIC control command constants.
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

// NIC capability bits.
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
    if crate::is_error(ret) { Err(ret) } else { Ok(ret) }
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

// ═══════════════════════════════════════════════════════════════════════════
// TCP SOCKETS (Layer 2) — smoltcp-backed convenience API
// ═══════════════════════════════════════════════════════════════════════════

// Sub-command constants (must match hwinit handler).
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
    if crate::is_error(ret) { Err(ret) } else { Ok(ret) }
}

/// Connect a TCP socket to a remote host.
///
/// `ip` is a 4-byte IPv4 address in **network byte order** (big-endian).
pub fn tcp_connect(handle: TcpHandle, ip: u32, port: u16) -> Result<(), u64> {
    let ret = unsafe { syscall4(SYS_NET, NET_TCP_CONNECT, handle, ip as u64, port as u64) };
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
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
        syscall4(SYS_NET, NET_TCP_SEND, handle, data.as_ptr() as u64, data.len() as u64)
    };
    if crate::is_error(ret) { Err(ret) } else { Ok(ret as usize) }
}

/// Receive data from a connected TCP socket.
///
/// Returns the number of bytes received (0 if no data available yet).
pub fn tcp_recv(handle: TcpHandle, buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall4(SYS_NET, NET_TCP_RECV, handle, buf.as_mut_ptr() as u64, buf.len() as u64)
    };
    if crate::is_error(ret) { Err(ret) } else { Ok(ret as usize) }
}

/// Close a TCP socket (sends FIN, initiates graceful shutdown).
pub fn tcp_close(handle: TcpHandle) {
    unsafe { syscall2(SYS_NET, NET_TCP_CLOSE, handle); }
}

/// Query a TCP socket's current state.
pub fn tcp_state(handle: TcpHandle) -> Result<TcpState, u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_STATE, handle) };
    if crate::is_error(ret) { Err(ret) } else { Ok(TcpState::from_raw(ret)) }
}

/// Bind a socket and start listening for incoming connections.
pub fn tcp_listen(handle: TcpHandle, port: u16) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_LISTEN, handle, port as u64) };
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}

/// Accept an incoming connection on a listening socket.
///
/// Returns a new handle for the accepted connection, or an error
/// if no connection is pending.
pub fn tcp_accept(listen_handle: TcpHandle) -> Result<TcpHandle, u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_ACCEPT, listen_handle) };
    if crate::is_error(ret) { Err(ret) } else { Ok(ret) }
}

/// Shutdown the write half of a TCP connection.
pub fn tcp_shutdown(handle: TcpHandle) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_SHUTDOWN, handle) };
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}

/// Set TCP_NODELAY (disable Nagle's algorithm).
pub fn tcp_set_nodelay(handle: TcpHandle, on: bool) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_NODELAY, handle, on as u64) };
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}

/// Set TCP keepalive interval (0 = disable).
pub fn tcp_set_keepalive(handle: TcpHandle, interval_ms: u64) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_KEEPALIVE, handle, interval_ms) };
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}

// ═══════════════════════════════════════════════════════════════════════════
// DNS RESOLUTION
// ═══════════════════════════════════════════════════════════════════════════

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
        syscall3(SYS_DNS, DNS_START_CMD, hostname.as_ptr() as u64, hostname.len() as u64)
    };
    if crate::is_error(ret) { Err(ret) } else { Ok(ret) }
}

/// Poll a DNS query.
///
/// Returns `Ok(Some(ip_nbo))` when resolved, `Ok(None)` if still pending,
/// or `Err` on failure.
pub fn dns_poll(query: DnsQuery) -> Result<Option<u32>, u64> {
    let mut ip: u32 = 0;
    let ret = unsafe {
        syscall3(SYS_DNS, DNS_RESULT_CMD, query, &mut ip as *mut u32 as u64)
    };
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
                // Yield to let the network stack process packets
                unsafe { syscall0(SYS_YIELD); }
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
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}

// ═══════════════════════════════════════════════════════════════════════════
// STACK CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

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
    let ret = unsafe {
        syscall2(SYS_NET_CFG, CFG_GET, &mut info as *mut NetConfigInfo as u64)
    };
    if crate::is_error(ret) { Err(ret) } else { Ok(info) }
}

/// Switch to DHCP mode.
pub fn net_dhcp() -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_NET_CFG, CFG_DHCP) };
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
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
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}

/// Set the hostname (for DHCP FQDN, mDNS, etc.).
pub fn net_set_hostname(hostname: &str) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(SYS_NET_CFG, CFG_HOSTNAME, hostname.as_ptr() as u64, hostname.len() as u64)
    };
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}

// ═══════════════════════════════════════════════════════════════════════════
// STACK POLLING & STATISTICS
// ═══════════════════════════════════════════════════════════════════════════

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
    let ret = unsafe {
        syscall2(SYS_NET_POLL, POLL_STATS, &mut stats as *mut NetStats as u64)
    };
    if crate::is_error(ret) { Err(ret) } else { Ok(stats) }
}

// ═══════════════════════════════════════════════════════════════════════════
// CONVENIENCE — IPv4 helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a dotted-decimal IPv4 to network byte order u32.
pub const fn ipv4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_be_bytes([a, b, c, d])
}

/// Convert a network byte order u32 to 4-byte array.
pub const fn ipv4_bytes(ip: u32) -> [u8; 4] {
    ip.to_be_bytes()
}

// ═══════════════════════════════════════════════════════════════════════════
// UDP SOCKETS (via SYS_NET subcmds 11-14)
// ═══════════════════════════════════════════════════════════════════════════

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
    if crate::is_error(ret) { Err(ret) } else { Ok(ret) }
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
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
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
    if crate::is_error(ret) { Err(ret) } else { Ok(()) }
}
