// smoltcp bridge by function pointers; the kernel doesn't link smoltcp.
// Sockets are opaque i64 handles. Raw NIC syscalls (32-37) let userspace
// build a stack from scratch; this convenience layer (38-41) is for programs
// that just want TCP/IP. They coexist.

use super::common::*;
use super::nic_io::sys_nic_ctrl;
use crate::hal;
use morpheus_foundation::types::{UdpRecvDesc, UdpSendDesc};

// Canonical net subcommand codes live in morpheus_foundation::net; re-exported
// here so kernel code referencing handler::net::NET_* still resolves.
pub use morpheus_foundation::net::{
    DNS_RESULT, DNS_SET_SERVERS, DNS_START, NET_CFG_ACTIVATE, NET_CFG_DHCP, NET_CFG_GET,
    NET_CFG_HOSTNAME, NET_CFG_STATIC, NET_POLL_DRIVE, NET_POLL_STATS, NET_TCP_ACCEPT,
    NET_TCP_CLOSE, NET_TCP_CONNECT, NET_TCP_KEEPALIVE, NET_TCP_LISTEN, NET_TCP_NODELAY,
    NET_TCP_RECV, NET_TCP_SEND, NET_TCP_SHUTDOWN, NET_TCP_SOCKET, NET_TCP_STATE, NET_UDP_CLOSE,
    NET_UDP_RECV_FROM, NET_UDP_SEND_TO, NET_UDP_SOCKET,
};

pub use morpheus_foundation::types::{NetConfigInfo, NetStats};

type UdpSendFn =
    unsafe fn(handle: i64, dest_ip: u32, dest_port: u16, buf: *const u8, len: usize) -> i64;
type UdpRecvFn = unsafe fn(handle: i64, buf: *mut u8, len: usize, src_out: *mut u8) -> i64;

#[repr(C)]
pub struct NetStackOps {
    pub tcp_socket: Option<unsafe fn() -> i64>,
    pub tcp_connect: Option<unsafe fn(handle: i64, ip: u32, port: u16) -> i64>,
    pub tcp_send: Option<unsafe fn(handle: i64, buf: *const u8, len: usize) -> i64>,
    pub tcp_recv: Option<unsafe fn(handle: i64, buf: *mut u8, len: usize) -> i64>,
    pub tcp_close: Option<unsafe fn(handle: i64)>,
    pub tcp_state: Option<unsafe fn(handle: i64) -> i64>,
    /// Non-consuming readiness probes (1/0/-1) for the epoll/net-poll readiness scan.
    pub tcp_can_recv: Option<unsafe fn(handle: i64) -> i64>,
    pub tcp_can_send: Option<unsafe fn(handle: i64) -> i64>,
    pub tcp_listen: Option<unsafe fn(handle: i64, port: u16) -> i64>,
    pub tcp_accept: Option<unsafe fn(listen_handle: i64) -> i64>,
    pub tcp_shutdown: Option<unsafe fn(handle: i64) -> i64>,
    pub tcp_nodelay: Option<unsafe fn(handle: i64, on: i64) -> i64>,
    pub tcp_keepalive: Option<unsafe fn(handle: i64, ms: u64) -> i64>,

    pub udp_socket: Option<unsafe fn() -> i64>,
    pub udp_send_to: Option<UdpSendFn>,
    pub udp_recv_from: Option<UdpRecvFn>,
    pub udp_close: Option<unsafe fn(handle: i64)>,

    pub dns_start: Option<unsafe fn(name: *const u8, len: usize) -> i64>,
    pub dns_result: Option<unsafe fn(query: i64, out: *mut u8) -> i64>,
    pub dns_set_servers: Option<unsafe fn(servers: *const u32, count: usize) -> i64>,

    pub cfg_get: Option<unsafe fn(buf: *mut u8) -> i64>,
    pub cfg_dhcp: Option<unsafe fn() -> i64>,
    pub cfg_static_ip: Option<unsafe fn(ip: u32, prefix_len: u8, gateway: u32) -> i64>,
    pub cfg_hostname: Option<unsafe fn(name: *const u8, len: usize) -> i64>,

    pub poll_drive: Option<unsafe fn(timestamp_ms: u64) -> i64>,
    pub poll_stats: Option<unsafe fn(buf: *mut u8) -> i64>,
}

static mut NET_STACK_OPS: NetStackOps = NetStackOps {
    tcp_socket: None,
    tcp_connect: None,
    tcp_send: None,
    tcp_recv: None,
    tcp_close: None,
    tcp_state: None,
    tcp_can_recv: None,
    tcp_can_send: None,
    tcp_listen: None,
    tcp_accept: None,
    tcp_shutdown: None,
    tcp_nodelay: None,
    tcp_keepalive: None,
    udp_socket: None,
    udp_send_to: None,
    udp_recv_from: None,
    udp_close: None,
    dns_start: None,
    dns_result: None,
    dns_set_servers: None,
    cfg_get: None,
    cfg_dhcp: None,
    cfg_static_ip: None,
    cfg_hostname: None,
    poll_drive: None,
    poll_stats: None,
};

// userspace-triggered network bring-up callback
static mut NET_ACTIVATE_FN: Option<unsafe fn() -> i64> = None;

pub unsafe fn register_net_stack(ops: NetStackOps) {
    NET_STACK_OPS = ops;
}

pub unsafe fn register_net_activation(callback: unsafe fn() -> i64) {
    NET_ACTIVATE_FN = Some(callback);
}

fn net_stack_present() -> bool {
    unsafe { NET_STACK_OPS.tcp_socket.is_some() }
}

pub unsafe fn sys_net(subcmd: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        NET_TCP_SOCKET => match NET_STACK_OPS.tcp_socket {
            Some(f) => {
                let h = f();
                if h < 0 {
                    ENOMEM
                } else {
                    h as u64
                }
            },
            None => ENOSYS,
        },
        NET_TCP_CONNECT => {
            let handle = a2 as i64;
            let ip = a3 as u32;
            let port = a4 as u16;
            match NET_STACK_OPS.tcp_connect {
                Some(f) => {
                    let rc = f(handle, ip, port);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_SEND => {
            let handle = a2 as i64;
            if a4 > 0 && !validate_user_buf(a3, a4) {
                return EFAULT;
            }
            match NET_STACK_OPS.tcp_send {
                Some(f) => {
                    let rc = f(handle, a3 as *const u8, a4 as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_RECV => {
            let handle = a2 as i64;
            if a4 > 0 && !validate_user_buf(a3, a4) {
                return EFAULT;
            }
            match NET_STACK_OPS.tcp_recv {
                Some(f) => {
                    let rc = f(handle, a3 as *mut u8, a4 as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_CLOSE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_close {
                Some(f) => {
                    f(handle);
                    0
                },
                None => ENOSYS,
            }
        },
        NET_TCP_STATE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_state {
                Some(f) => {
                    let s = f(handle);
                    if s < 0 {
                        EINVAL
                    } else {
                        s as u64
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_LISTEN => {
            let handle = a2 as i64;
            let port = a3 as u16;
            match NET_STACK_OPS.tcp_listen {
                Some(f) => {
                    let rc = f(handle, port);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_ACCEPT => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_accept {
                Some(f) => {
                    let h = f(handle);
                    if h < 0 {
                        EIO
                    } else {
                        h as u64
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_SHUTDOWN => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_shutdown {
                Some(f) => {
                    let rc = f(handle);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_NODELAY => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_nodelay {
                Some(f) => {
                    let rc = f(handle, a3 as i64);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        NET_TCP_KEEPALIVE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_keepalive {
                Some(f) => {
                    let rc = f(handle, a3);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        NET_UDP_SOCKET => match NET_STACK_OPS.udp_socket {
            Some(f) => {
                let h = f();
                if h < 0 {
                    ENOMEM
                } else {
                    h as u64
                }
            },
            None => ENOSYS,
        },
        NET_UDP_SEND_TO => {
            let handle = a2 as i64;
            if !validate_user_buf(a3, core::mem::size_of::<UdpSendDesc>() as u64) {
                return EFAULT;
            }
            let desc = &*(a3 as *const UdpSendDesc);
            let ip = desc.ip;
            let port = desc.port;
            let buf_ptr = desc.buf as u64;
            let buf_len = desc.len;
            if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
                return EFAULT;
            }
            if buf_len > 65535 {
                return EINVAL;
            }
            match NET_STACK_OPS.udp_send_to {
                Some(f) => {
                    let rc = f(handle, ip, port, buf_ptr as *const u8, buf_len as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                },
                None => ENOSYS,
            }
        },
        NET_UDP_RECV_FROM => {
            let handle = a2 as i64;
            if !validate_user_buf(a3, core::mem::size_of::<UdpRecvDesc>() as u64) {
                return EFAULT;
            }
            let desc = &mut *(a3 as *mut UdpRecvDesc);
            let buf_ptr = desc.buf as u64;
            let buf_len = desc.buf_len;
            if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
                return EFAULT;
            }
            let src_out = (&mut desc.src_ip) as *mut u32 as *mut u8;
            match NET_STACK_OPS.udp_recv_from {
                Some(f) => {
                    let rc = f(handle, buf_ptr as *mut u8, buf_len as usize, src_out);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                },
                None => ENOSYS,
            }
        },
        NET_UDP_CLOSE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.udp_close {
                Some(f) => {
                    f(handle);
                    0
                },
                None => ENOSYS,
            }
        },
        _ => EINVAL,
    }
}

pub unsafe fn sys_dns(subcmd: u64, a2: u64, a3: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        DNS_START => {
            if a3 == 0 || a3 > 253 {
                return EINVAL;
            }
            if !validate_user_buf(a2, a3) {
                return EFAULT;
            }
            match NET_STACK_OPS.dns_start {
                Some(f) => {
                    let h = f(a2 as *const u8, a3 as usize);
                    if h < 0 {
                        EIO
                    } else {
                        h as u64
                    }
                },
                None => ENOSYS,
            }
        },
        DNS_RESULT => {
            let query = a2 as i64;
            if !validate_user_buf(a3, 4) {
                return EFAULT;
            }
            match NET_STACK_OPS.dns_result {
                Some(f) => {
                    let rc = f(query, a3 as *mut u8);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                },
                None => ENOSYS,
            }
        },
        DNS_SET_SERVERS => {
            let count = a3;
            if count == 0 || count > 4 {
                return EINVAL;
            }
            if !validate_user_buf(a2, count * 4) {
                return EFAULT;
            }
            match NET_STACK_OPS.dns_set_servers {
                Some(f) => {
                    let rc = f(a2 as *const u32, count as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        _ => EINVAL,
    }
}

pub unsafe fn sys_net_cfg(subcmd: u64, a2: u64, a3: u64, _a4: u64) -> u64 {
    match subcmd {
        NET_CFG_GET => {
            let size = core::mem::size_of::<NetConfigInfo>() as u64;
            if !validate_user_buf(a2, size) {
                return EFAULT;
            }
            match NET_STACK_OPS.cfg_get {
                Some(f) => {
                    let rc = f(a2 as *mut u8);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => {
                    core::ptr::write_bytes(a2 as *mut u8, 0, size as usize);
                    0
                },
            }
        },
        NET_CFG_ACTIVATE => match NET_ACTIVATE_FN {
            Some(f) => {
                let rc = f();
                match rc {
                    0.. => rc as u64,
                    -1 => EIO,
                    -2 => ENODEV,
                    -4 => EIO,
                    -5 => EFAULT,
                    _ => EIO,
                }
            },
            None => ENODEV,
        },
        _ if !net_stack_present() => ENODEV,
        NET_CFG_DHCP => match NET_STACK_OPS.cfg_dhcp {
            Some(f) => {
                let rc = f();
                if rc < 0 {
                    EIO
                } else {
                    0
                }
            },
            None => ENOSYS,
        },
        NET_CFG_STATIC => {
            let ip_nbo = a2 as u32;
            let prefix_len = (a3 >> 32) as u8;
            let gw_nbo = a3 as u32;
            match NET_STACK_OPS.cfg_static_ip {
                Some(f) => {
                    let rc = f(ip_nbo, prefix_len, gw_nbo);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        NET_CFG_HOSTNAME => {
            if a3 == 0 || a3 > 63 {
                return EINVAL;
            }
            if !validate_user_buf(a2, a3) {
                return EFAULT;
            }
            match NET_STACK_OPS.cfg_hostname {
                Some(f) => {
                    let rc = f(a2 as *const u8, a3 as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        128.. => {
            let nic_cmd = (subcmd - 128) as u32;
            sys_nic_ctrl(nic_cmd as u64, a2)
        },
        _ => EINVAL,
    }
}

// Typed bridge for the BSD-socket fd layer (`handler::socket`): it must NOT poke
// `NET_STACK_OPS` directly. A negative return is the stack's own error code;
// `BRIDGE_ABSENT` means the op pointer was never installed.

/// Sentinel: the requested op was not registered in `NetStackOps`.
pub(crate) const BRIDGE_ABSENT: i64 = i64::MIN;

pub(crate) fn net_present() -> bool {
    net_stack_present()
}

/// Monotonic uptime (ms) from the TSC — the single time source fed to every stack
/// poll so its timers advance against real progress.
pub(crate) fn monotonic_ms() -> u64 {
    let hz = hal().timer().tsc_frequency();
    if hz == 0 {
        return 0;
    }
    let per_ms = (hz / 1000).max(1);
    hal().timer().read_tsc() / per_ms
}

/// Drive the stack once. Blocking socket ops call this between readiness re-checks
/// so retransmits/ACKs make progress.
pub(crate) unsafe fn net_drive() {
    if let Some(f) = NET_STACK_OPS.poll_drive {
        let _ = f(monotonic_ms());
        // Packet arrival never touches io::readiness on its own; recompute every
        // socket's level mask post-poll so epoll_wait is event-driven (a SYN on a
        // listener, RX data, connect-complete and FIN all surface here).
        super::socket::refresh_socket_readiness();
    }
}

pub(crate) unsafe fn bridge_tcp_socket() -> i64 {
    match NET_STACK_OPS.tcp_socket {
        Some(f) => f(),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_connect(handle: i64, ip_nbo: u32, port_host: u16) -> i64 {
    match NET_STACK_OPS.tcp_connect {
        Some(f) => f(handle, ip_nbo, port_host),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_send(handle: i64, buf: *const u8, len: usize) -> i64 {
    match NET_STACK_OPS.tcp_send {
        Some(f) => f(handle, buf, len),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_recv(handle: i64, buf: *mut u8, len: usize) -> i64 {
    match NET_STACK_OPS.tcp_recv {
        Some(f) => f(handle, buf, len),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_close(handle: i64) {
    if let Some(f) = NET_STACK_OPS.tcp_close {
        f(handle);
    }
}

pub(crate) unsafe fn bridge_tcp_state(handle: i64) -> i64 {
    match NET_STACK_OPS.tcp_state {
        Some(f) => f(handle),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_can_recv(handle: i64) -> i64 {
    match NET_STACK_OPS.tcp_can_recv {
        Some(f) => f(handle),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_can_send(handle: i64) -> i64 {
    match NET_STACK_OPS.tcp_can_send {
        Some(f) => f(handle),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_listen(handle: i64, port_host: u16) -> i64 {
    match NET_STACK_OPS.tcp_listen {
        Some(f) => f(handle, port_host),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_accept(handle: i64) -> i64 {
    match NET_STACK_OPS.tcp_accept {
        Some(f) => f(handle),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_shutdown(handle: i64) -> i64 {
    match NET_STACK_OPS.tcp_shutdown {
        Some(f) => f(handle),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_nodelay(handle: i64, on: bool) -> i64 {
    match NET_STACK_OPS.tcp_nodelay {
        Some(f) => f(handle, on as i64),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_tcp_keepalive(handle: i64, ms: u64) -> i64 {
    match NET_STACK_OPS.tcp_keepalive {
        Some(f) => f(handle, ms),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_udp_socket() -> i64 {
    match NET_STACK_OPS.udp_socket {
        Some(f) => f(),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_udp_send_to(
    handle: i64,
    ip_nbo: u32,
    port_host: u16,
    buf: *const u8,
    len: usize,
) -> i64 {
    match NET_STACK_OPS.udp_send_to {
        Some(f) => f(handle, ip_nbo, port_host, buf, len),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_udp_recv_from(
    handle: i64,
    buf: *mut u8,
    len: usize,
    src_out: *mut u8,
) -> i64 {
    match NET_STACK_OPS.udp_recv_from {
        Some(f) => f(handle, buf, len, src_out),
        None => BRIDGE_ABSENT,
    }
}

pub(crate) unsafe fn bridge_udp_close(handle: i64) {
    if let Some(f) = NET_STACK_OPS.udp_close {
        f(handle);
    }
}

pub unsafe fn sys_net_poll(subcmd: u64, a2: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        // `a2` (userspace timestamp) is ignored: the kernel is the sole authority on
        // monotonic time. Feeding the real clock (not `Instant(0)`) is what makes
        // smoltcp's retransmit / TIME_WAIT / keepalive / DHCP timers advance.
        NET_POLL_DRIVE => match NET_STACK_OPS.poll_drive {
            Some(f) => {
                let rc = f(monotonic_ms());
                // Recompute every socket's level mask post-poll: packet arrival never
                // touches io::readiness on its own, so a userspace poller (mio/tokio)
                // driving the stack via SYS_NET_POLL needs this to make epoll_wait
                // event-driven (a SYN on a listener, RX data, connect-complete, FIN).
                super::socket::refresh_socket_readiness();
                if rc < 0 {
                    EIO
                } else {
                    rc as u64
                }
            },
            None => ENOSYS,
        },
        NET_POLL_STATS => {
            let size = core::mem::size_of::<NetStats>() as u64;
            if !validate_user_buf(a2, size) {
                return EFAULT;
            }
            match NET_STACK_OPS.poll_stats {
                Some(f) => {
                    let rc = f(a2 as *mut u8);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                },
                None => ENOSYS,
            }
        },
        _ => EINVAL,
    }
}
