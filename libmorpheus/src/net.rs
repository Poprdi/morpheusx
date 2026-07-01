//! Networking: raw NIC frames, hardware knobs, smoltcp TCP/IP.
//! Stack is explicitly polled — call [`net_poll_drive`] periodically; RAII types
//! poll internally but event loops must drive it too.

use crate::error::{self, Error};
use crate::io;
use crate::raw::*;

// Net boundary structs are canonical in morpheus-foundation — single source.
pub use morpheus_foundation::types::{NetConfigInfo, NetStats, NicHwStats, NicInfo};

pub fn nic_info() -> Result<NicInfo, u64> {
    let mut info = NicInfo {
        version: 0,
        struct_size: 0,
        link_up: 0,
        present: 0,
        mac: [0u8; 8],
        reserved: [0u8; 8],
    };
    let ret = unsafe { syscall1(SYS_NIC_INFO, &mut info as *mut NicInfo as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(info)
    }
}

/// `frame` must include the full Ethernet header (>= 14 bytes).
pub fn nic_tx(frame: &[u8]) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_NIC_TX, frame.as_ptr() as u64, frame.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Returns bytes received; 0 if no frame available.
pub fn nic_rx(buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe { syscall2(SYS_NIC_RX, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

pub fn nic_link_up() -> Result<bool, u64> {
    let ret = unsafe { syscall0(SYS_NIC_LINK) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret != 0)
    }
}

pub fn nic_mac() -> Result<[u8; 6], u64> {
    let mut mac = [0u8; 6];
    let ret = unsafe { syscall1(SYS_NIC_MAC, mac.as_mut_ptr() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(mac)
    }
}

pub fn nic_refill() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_NIC_REFILL) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// NIC hardware control routes through SYS_NET_CFG with subcmd >= 128,
// dispatching directly to the NIC driver's ctrl function pointer.

pub use morpheus_foundation::net::{
    NIC_CAP_IRQ_COALESCE, NIC_CAP_MAC_SET, NIC_CAP_MULTICAST, NIC_CAP_PROMISC, NIC_CAP_RX_CSUM,
    NIC_CAP_TSO, NIC_CAP_TX_CSUM, NIC_CAP_VLAN, NIC_CTRL_CAPS, NIC_CTRL_IRQ_COALESCE,
    NIC_CTRL_MAC_SET, NIC_CTRL_MTU, NIC_CTRL_MULTICAST, NIC_CTRL_PROMISC, NIC_CTRL_RX_CSUM,
    NIC_CTRL_RX_RING_SIZE, NIC_CTRL_STATS, NIC_CTRL_STATS_RESET, NIC_CTRL_TSO, NIC_CTRL_TX_CSUM,
    NIC_CTRL_TX_RING_SIZE, NIC_CTRL_VLAN,
};

/// Generic entry. Prefer the typed wrappers below.
pub fn nic_ctrl(cmd: u32, arg: u64) -> Result<u64, u64> {
    let subcmd = morpheus_foundation::net::NIC_CTRL_BASE + cmd as u64;
    let ret = unsafe { syscall3(SYS_NET_CFG, subcmd, arg, 0) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

pub fn nic_set_promisc(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_PROMISC, on as u64).map(|_| ())
}

pub fn nic_set_mac(mac: &[u8; 6]) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_MAC_SET, mac.as_ptr() as u64).map(|_| ())
}

pub fn nic_hw_stats() -> Result<NicHwStats, u64> {
    let mut stats = NicHwStats::default();
    nic_ctrl(NIC_CTRL_STATS, &mut stats as *mut NicHwStats as u64)?;
    Ok(stats)
}

pub fn nic_stats_reset() -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_STATS_RESET, 0).map(|_| ())
}

pub fn nic_set_mtu(mtu: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_MTU, mtu as u64).map(|_| ())
}

pub fn nic_set_multicast(accept_all: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_MULTICAST, accept_all as u64).map(|_| ())
}

pub fn nic_set_vlan(vlan_id: u16) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_VLAN, vlan_id as u64).map(|_| ())
}

pub fn nic_set_tx_csum(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_TX_CSUM, on as u64).map(|_| ())
}

pub fn nic_set_rx_csum(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_RX_CSUM, on as u64).map(|_| ())
}

pub fn nic_set_tso(on: bool) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_TSO, on as u64).map(|_| ())
}

pub fn nic_set_rx_ring(descriptors: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_RX_RING_SIZE, descriptors as u64).map(|_| ())
}

pub fn nic_set_tx_ring(descriptors: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_TX_RING_SIZE, descriptors as u64).map(|_| ())
}

/// `usec` = interrupt coalescing interval.
pub fn nic_set_irq_coalesce(usec: u32) -> Result<(), u64> {
    nic_ctrl(NIC_CTRL_IRQ_COALESCE, usec as u64).map(|_| ())
}

pub fn nic_caps() -> Result<u64, u64> {
    let mut caps: u64 = 0;
    nic_ctrl(NIC_CTRL_CAPS, &mut caps as *mut u64 as u64)?;
    Ok(caps)
}

// TCP sockets (smoltcp). Subcmds are canonical in morpheus_foundation::net.

use morpheus_foundation::net::{
    NET_TCP_ACCEPT, NET_TCP_CLOSE, NET_TCP_CONNECT, NET_TCP_KEEPALIVE, NET_TCP_LISTEN,
    NET_TCP_NODELAY, NET_TCP_RECV, NET_TCP_SEND, NET_TCP_SHUTDOWN, NET_TCP_SOCKET, NET_TCP_STATE,
    NET_UDP_CLOSE, NET_UDP_RECV_FROM, NET_UDP_SEND_TO, NET_UDP_SOCKET,
};

pub type TcpHandle = u64;

/// Ordinals match smoltcp `TcpState`.
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

pub fn tcp_socket() -> Result<TcpHandle, u64> {
    let ret = unsafe { syscall1(SYS_NET, NET_TCP_SOCKET) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// `ip` is network byte order (big-endian).
pub fn tcp_connect(handle: TcpHandle, ip: u32, port: u16) -> Result<(), u64> {
    let ret = unsafe { syscall4(SYS_NET, NET_TCP_CONNECT, handle, ip as u64, port as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn tcp_connect_ip(handle: TcpHandle, a: u8, b: u8, c: u8, d: u8, port: u16) -> Result<(), u64> {
    let ip = u32::from_be_bytes([a, b, c, d]);
    tcp_connect(handle, ip, port)
}

/// Returns bytes accepted into the send buffer.
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

/// Returns bytes received; 0 if no data available yet.
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

/// Sends FIN, initiates graceful shutdown.
pub fn tcp_close(handle: TcpHandle) {
    unsafe {
        syscall2(SYS_NET, NET_TCP_CLOSE, handle);
    }
}

pub fn tcp_state(handle: TcpHandle) -> Result<TcpState, u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_STATE, handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(TcpState::from_raw(ret))
    }
}

pub fn tcp_listen(handle: TcpHandle, port: u16) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_LISTEN, handle, port as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Returns a new handle, or an error if no connection is pending.
pub fn tcp_accept(listen_handle: TcpHandle) -> Result<TcpHandle, u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_ACCEPT, listen_handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Shuts down the write half.
pub fn tcp_shutdown(handle: TcpHandle) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_TCP_SHUTDOWN, handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn tcp_set_nodelay(handle: TcpHandle, on: bool) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_NODELAY, handle, on as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn tcp_set_keepalive(handle: TcpHandle, interval_ms: u64) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_NET, NET_TCP_KEEPALIVE, handle, interval_ms) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

use morpheus_foundation::net::{DNS_RESULT, DNS_SET_SERVERS, DNS_START};

pub type DnsQuery = u64;

/// Async lookup; call [`dns_poll`] until resolved.
pub fn dns_start(hostname: &str) -> Result<DnsQuery, u64> {
    let ret = unsafe {
        syscall3(
            SYS_DNS,
            DNS_START,
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

/// `Ok(Some(ip_nbo))` when resolved, `Ok(None)` if pending.
pub fn dns_poll(query: DnsQuery) -> Result<Option<u32>, u64> {
    let mut ip: u32 = 0;
    let ret = unsafe { syscall3(SYS_DNS, DNS_RESULT, query, &mut ip as *mut u32 as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else if ret == 1 {
        Ok(None)
    } else {
        Ok(Some(ip))
    }
}

/// Blocking; returns IPv4 in network byte order.
pub fn dns_resolve(hostname: &str) -> Result<u32, u64> {
    let query = dns_start(hostname)?;
    loop {
        net_poll_drive(0);
        match dns_poll(query)? {
            Some(ip) => return Ok(ip),
            None => {
                // sleep(1) not yield: avoids spinning when sole runnable proc.
                crate::process::sleep(1);
            },
        }
    }
}

/// `servers` is network byte order; max 4.
pub fn dns_set_servers(servers: &[u32]) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(
            SYS_DNS,
            DNS_SET_SERVERS,
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

const CFG_GET: u64 = 0;
const CFG_DHCP: u64 = 1;
const CFG_STATIC: u64 = 2;
const CFG_HOSTNAME: u64 = 3;
const CFG_ACTIVATE: u64 = 4;

pub use morpheus_foundation::net::{
    NET_FLAG_DHCP, NET_FLAG_HAS_DNS, NET_FLAG_HAS_GATEWAY, NET_STATE_DHCP_DISCOVERING,
    NET_STATE_ERROR, NET_STATE_READY, NET_STATE_UNCONFIGURED,
};

pub fn net_config() -> Result<NetConfigInfo, u64> {
    let mut info = unsafe { core::mem::zeroed::<NetConfigInfo>() };
    let ret = unsafe { syscall2(SYS_NET_CFG, CFG_GET, &mut info as *mut NetConfigInfo as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(info)
    }
}

pub fn net_dhcp() -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_NET_CFG, CFG_DHCP) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// `ip`/`gateway` in NBO; `prefix_len` is CIDR (e.g. 24 for /24).
pub fn net_static_ip(ip: u32, prefix_len: u8, gateway: u32) -> Result<(), u64> {
    // Pack `[63:32]=prefix_len`, `[31:0]=gateway` into one u64 arg.
    let packed = ((prefix_len as u64) << 32) | (gateway as u64);
    let ret = unsafe { syscall3(SYS_NET_CFG, CFG_STATIC, ip as u64, packed) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// For DHCP FQDN, mDNS, etc.
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

/// Requests NIC bring-up (boot default is offline).
/// Returns `0` when newly activated, `>0` when already active/no-op.
pub fn net_activate() -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_NET_CFG, CFG_ACTIVATE) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

const POLL_DRIVE: u64 = 0;
const POLL_STATS: u64 = 1;

/// Drives DHCP/ARP/TCP timers. Call periodically; returns true on activity.
pub fn net_poll_drive(timestamp_ms: u64) -> bool {
    let ret = unsafe { syscall2(SYS_NET_POLL, POLL_DRIVE, timestamp_ms) };
    ret == 1
}

pub fn net_stats() -> Result<NetStats, u64> {
    let mut stats = NetStats::default();
    let ret = unsafe { syscall2(SYS_NET_POLL, POLL_STATS, &mut stats as *mut NetStats as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(stats)
    }
}

/// Dotted-decimal → NBO u32.
pub const fn ipv4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from_be_bytes([a, b, c, d])
}

/// NBO u32 → 4-byte array.
pub const fn ipv4_bytes(ip: u32) -> [u8; 4] {
    ip.to_be_bytes()
}

pub use morpheus_foundation::types::{UdpRecvDesc, UdpSendDesc};

pub fn udp_socket() -> Result<u64, u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_UDP_SOCKET, 0) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// `ip` is NBO; `port` is host order.
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
            NET_UDP_SEND_TO,
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

/// Returns `(bytes, src_ip_nbo, src_port)`.
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
            NET_UDP_RECV_FROM,
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

pub fn udp_close(handle: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_NET, NET_UDP_CLOSE, handle) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

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

    pub const fn from_nbo(nbo: u32) -> Self {
        let b = nbo.to_be_bytes();
        Self::new(b[0], b[1], b[2], b[3])
    }

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

// std::net-shaped sockets over the unified fd ABI (SYS_SOCKET..SYS_SHUTDOWN): a
// socket is a real fd and ops block in-kernel (no userspace busy-poll). The
// legacy handle-based tcp_*/udp_* free functions above stay for raw SYS_NET.

use crate::raw::{
    sys_accept, sys_bind, sys_connect, sys_getpeername, sys_getsockname, sys_listen, sys_recvfrom,
    sys_sendto, sys_setsockopt, sys_shutdown, sys_socket, syscall1, SYS_CLOSE,
};
use morpheus_foundation::net::{
    AF_INET, IPPROTO_TCP, SHUT_RD, SHUT_RDWR, SHUT_WR, SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET,
    SO_RCVTIMEO, TCP_NODELAY,
};
use morpheus_foundation::types::{KTimeval, SockAddrIn, SockAddrStorage};

const SOCKADDR_IN_LEN: u64 = core::mem::size_of::<SockAddrIn>() as u64;

fn check(ret: u64) -> error::Result<u64> {
    if crate::is_error(ret) {
        Err(Error::from_raw(ret))
    } else {
        Ok(ret)
    }
}

fn encode_addr(ip: Ipv4Addr, port: u16) -> SockAddrStorage {
    let sin = SockAddrIn {
        sin_family: AF_INET as u16,
        sin_port: port.to_be(),
        sin_addr: ip.to_nbo(),
        sin_zero: [0u8; 8],
    };
    let mut st = SockAddrStorage::zeroed();
    // SAFETY: SockAddrIn (16B) fits inside the 128B storage; we copy its bytes
    // into the AF_INET overlay the kernel reads back by sa_family.
    unsafe {
        core::ptr::copy_nonoverlapping(
            &sin as *const SockAddrIn as *const u8,
            &mut st as *mut SockAddrStorage as *mut u8,
            core::mem::size_of::<SockAddrIn>(),
        );
    }
    st
}

fn decode_addr(st: &SockAddrStorage) -> SocketAddr {
    // SAFETY: AF_INET overlay; we only read the 16 SockAddrIn bytes.
    let sin = unsafe { &*(st as *const SockAddrStorage as *const SockAddrIn) };
    SocketAddr::new(Ipv4Addr::from_nbo(sin.sin_addr), u16::from_be(sin.sin_port))
}

unsafe fn raw_close(fd: i32) {
    let _ = syscall1(SYS_CLOSE, fd as u64);
}

/// A real-fd TCP connection. Blocks in the kernel by default; closes on drop.
pub struct TcpStream {
    fd: i32,
}

impl TcpStream {
    pub fn connect(ip: Ipv4Addr, port: u16) -> error::Result<Self> {
        let fd = check(unsafe { sys_socket(AF_INET, SOCK_STREAM, 0) })? as i32;
        let sa = encode_addr(ip, port);
        if let Err(e) =
            check(unsafe { sys_connect(fd as u64, &sa as *const _ as u64, SOCKADDR_IN_LEN) })
        {
            unsafe { raw_close(fd) };
            return Err(e);
        }
        Ok(Self { fd })
    }

    pub fn connect_host(hostname: &str, port: u16) -> error::Result<Self> {
        let ip_nbo = dns_resolve(hostname).map_err(Error::from_raw)?;
        Self::connect(Ipv4Addr::from_nbo(ip_nbo), port)
    }

    pub fn from_raw_fd(fd: i32) -> Self {
        Self { fd }
    }

    pub fn into_raw_fd(self) -> i32 {
        let fd = self.fd;
        core::mem::forget(self);
        fd
    }

    pub fn as_raw_fd(&self) -> i32 {
        self.fd
    }

    pub fn peer_addr(&self) -> error::Result<SocketAddr> {
        let mut st = SockAddrStorage::zeroed();
        let mut len = core::mem::size_of::<SockAddrStorage>() as u32;
        check(unsafe {
            sys_getpeername(
                self.fd as u64,
                &mut st as *mut _ as u64,
                &mut len as *mut u32 as u64,
            )
        })?;
        Ok(decode_addr(&st))
    }

    pub fn local_addr(&self) -> error::Result<SocketAddr> {
        let mut st = SockAddrStorage::zeroed();
        let mut len = core::mem::size_of::<SockAddrStorage>() as u32;
        check(unsafe {
            sys_getsockname(
                self.fd as u64,
                &mut st as *mut _ as u64,
                &mut len as *mut u32 as u64,
            )
        })?;
        Ok(decode_addr(&st))
    }

    /// Disables Nagle.
    pub fn set_nodelay(&self, on: bool) -> error::Result<()> {
        let v: i32 = on as i32;
        check(unsafe {
            sys_setsockopt(
                self.fd as u64,
                IPPROTO_TCP,
                TCP_NODELAY,
                &v as *const i32 as u64,
                4,
            )
        })
        .map(|_| ())
    }

    /// `None` = block forever (Linux SO_RCVTIMEO of 0).
    pub fn set_read_timeout(&self, dur_ms: Option<u64>) -> error::Result<()> {
        let ms = dur_ms.unwrap_or(0);
        let tv = KTimeval {
            tv_sec: (ms / 1000) as i64,
            tv_usec: ((ms % 1000) * 1000) as i64,
        };
        check(unsafe {
            sys_setsockopt(
                self.fd as u64,
                SOL_SOCKET,
                SO_RCVTIMEO,
                &tv as *const KTimeval as u64,
                core::mem::size_of::<KTimeval>() as u64,
            )
        })
        .map(|_| ())
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> error::Result<()> {
        set_fd_nonblocking(self.fd, nonblocking)
    }

    pub fn shutdown(&self, how: Shutdown) -> error::Result<()> {
        check(unsafe { sys_shutdown(self.fd as u64, how.as_raw()) }).map(|_| ())
    }
}

impl io::Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
        let n = check(unsafe {
            sys_recvfrom(
                self.fd as u64,
                buf.as_mut_ptr() as u64,
                buf.len() as u64,
                0,
                0,
                0,
            )
        })?;
        Ok(n as usize)
    }
}

impl io::Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
        let n = check(unsafe {
            sys_sendto(
                self.fd as u64,
                buf.as_ptr() as u64,
                buf.len() as u64,
                0,
                0,
                0,
            )
        })?;
        Ok(n as usize)
    }

    fn flush(&mut self) -> error::Result<()> {
        Ok(())
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        unsafe { raw_close(self.fd) };
    }
}

/// A real-fd listening TCP socket.
pub struct TcpListener {
    fd: i32,
}

impl TcpListener {
    pub fn bind(ip: Ipv4Addr, port: u16) -> error::Result<Self> {
        let fd = check(unsafe { sys_socket(AF_INET, SOCK_STREAM, 0) })? as i32;
        let sa = encode_addr(ip, port);
        let r = check(unsafe { sys_bind(fd as u64, &sa as *const _ as u64, SOCKADDR_IN_LEN) })
            .and_then(|_| check(unsafe { sys_listen(fd as u64, 128) }));
        if let Err(e) = r {
            unsafe { raw_close(fd) };
            return Err(e);
        }
        Ok(Self { fd })
    }

    /// Bind to all interfaces on `port`.
    pub fn bind_any(port: u16) -> error::Result<Self> {
        Self::bind(Ipv4Addr::UNSPECIFIED, port)
    }

    /// Blocks until a connection arrives unless `set_nonblocking(true)`.
    pub fn accept(&self) -> error::Result<(TcpStream, SocketAddr)> {
        let mut st = SockAddrStorage::zeroed();
        let mut len = core::mem::size_of::<SockAddrStorage>() as u32;
        let newfd = check(unsafe {
            sys_accept(
                self.fd as u64,
                &mut st as *mut _ as u64,
                &mut len as *mut u32 as u64,
                0,
            )
        })? as i32;
        Ok((TcpStream::from_raw_fd(newfd), decode_addr(&st)))
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> error::Result<()> {
        set_fd_nonblocking(self.fd, nonblocking)
    }

    pub fn as_raw_fd(&self) -> i32 {
        self.fd
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        unsafe { raw_close(self.fd) };
    }
}

/// A real-fd UDP socket.
pub struct UdpSocket {
    fd: i32,
}

impl UdpSocket {
    pub fn bind(ip: Ipv4Addr, port: u16) -> error::Result<Self> {
        let fd = check(unsafe { sys_socket(AF_INET, SOCK_DGRAM, 0) })? as i32;
        let sa = encode_addr(ip, port);
        if let Err(e) =
            check(unsafe { sys_bind(fd as u64, &sa as *const _ as u64, SOCKADDR_IN_LEN) })
        {
            unsafe { raw_close(fd) };
            return Err(e);
        }
        Ok(Self { fd })
    }

    pub fn bind_any(port: u16) -> error::Result<Self> {
        Self::bind(Ipv4Addr::UNSPECIFIED, port)
    }

    /// Set the default peer so `send`/`recv` work without an address.
    pub fn connect(&self, ip: Ipv4Addr, port: u16) -> error::Result<()> {
        let sa = encode_addr(ip, port);
        check(unsafe { sys_connect(self.fd as u64, &sa as *const _ as u64, SOCKADDR_IN_LEN) })
            .map(|_| ())
    }

    pub fn send_to(&self, data: &[u8], ip: Ipv4Addr, port: u16) -> error::Result<usize> {
        let sa = encode_addr(ip, port);
        let n = check(unsafe {
            sys_sendto(
                self.fd as u64,
                data.as_ptr() as u64,
                data.len() as u64,
                0,
                &sa as *const _ as u64,
                SOCKADDR_IN_LEN,
            )
        })?;
        Ok(n as usize)
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> error::Result<(usize, SocketAddr)> {
        let mut st = SockAddrStorage::zeroed();
        let mut len = core::mem::size_of::<SockAddrStorage>() as u32;
        let n = check(unsafe {
            sys_recvfrom(
                self.fd as u64,
                buf.as_mut_ptr() as u64,
                buf.len() as u64,
                0,
                &mut st as *mut _ as u64,
                &mut len as *mut u32 as u64,
            )
        })?;
        Ok((n as usize, decode_addr(&st)))
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> error::Result<()> {
        set_fd_nonblocking(self.fd, nonblocking)
    }

    pub fn as_raw_fd(&self) -> i32 {
        self.fd
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        unsafe { raw_close(self.fd) };
    }
}

/// `std::net::Shutdown` analogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}

impl Shutdown {
    fn as_raw(self) -> u64 {
        match self {
            Shutdown::Read => SHUT_RD,
            Shutdown::Write => SHUT_WR,
            Shutdown::Both => SHUT_RDWR,
        }
    }
}

/// Toggle `O_NONBLOCK` via `F_SETFL` (backs `set_nonblocking`).
fn set_fd_nonblocking(fd: i32, nonblocking: bool) -> error::Result<()> {
    use morpheus_foundation::flags::open_flags::O_NONBLOCK;
    use morpheus_foundation::flags::{F_GETFL, F_SETFL};
    let cur = check(unsafe { crate::raw::sys_fcntl(fd as u64, F_GETFL, 0) })? as u32;
    let next = if nonblocking {
        cur | O_NONBLOCK
    } else {
        cur & !O_NONBLOCK
    };
    check(unsafe { crate::raw::sys_fcntl(fd as u64, F_SETFL, next as u64) }).map(|_| ())
}
