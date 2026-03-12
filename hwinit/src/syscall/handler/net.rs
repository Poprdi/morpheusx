
// NETWORK STACK — function-pointer bridge (TCP, DNS, config, poll)
//
// Like NicOps, the bootloader registers these after initialising the smoltcp
// stack.  hwinit has zero dependency on smoltcp — everything crosses the
// boundary as raw u64 / pointers / packed IPv4.
//
// Socket handles are opaque i64 values (smoltcp SocketHandle ordinals).
// Negative returns indicate errors.
//
// The raw NIC layer (32-37) + NIC_CTRL gives userspace full hardware
// control — userspace can build entirely custom protocol stacks from
// scratch.  The NET/DNS/CFG/POLL layer (38-41) is the *convenience*
// smoltcp-backed stack for programs that want TCP/IP without writing
// their own.  Both coexist; neither depends on the other.

// sub-commands for sys_net (38)
pub const NET_TCP_SOCKET: u64 = 0;
pub const NET_TCP_CONNECT: u64 = 1;
pub const NET_TCP_SEND: u64 = 2;
pub const NET_TCP_RECV: u64 = 3;
pub const NET_TCP_CLOSE: u64 = 4;
pub const NET_TCP_STATE: u64 = 5;
pub const NET_TCP_LISTEN: u64 = 6;
pub const NET_TCP_ACCEPT: u64 = 7;
pub const NET_TCP_SHUTDOWN: u64 = 8;
pub const NET_TCP_NODELAY: u64 = 9;
pub const NET_TCP_KEEPALIVE: u64 = 10;
// udp sub-commands for sys_net (38)
pub const NET_UDP_SOCKET: u64 = 11;
pub const NET_UDP_SEND_TO: u64 = 12;
pub const NET_UDP_RECV_FROM: u64 = 13;
pub const NET_UDP_CLOSE: u64 = 14;

// sub-commands for sys_dns (39)
pub const DNS_START: u64 = 0;
pub const DNS_RESULT: u64 = 1;
pub const DNS_SET_SERVERS: u64 = 2;

// sub-commands for sys_net_cfg (40)
pub const NET_CFG_GET: u64 = 0;
pub const NET_CFG_DHCP: u64 = 1;
pub const NET_CFG_STATIC: u64 = 2;
pub const NET_CFG_HOSTNAME: u64 = 3;
pub const NET_CFG_ACTIVATE: u64 = 4;

// sub-commands for sys_net_poll (41)
pub const NET_POLL_DRIVE: u64 = 0;
pub const NET_POLL_STATS: u64 = 1;

/// Network stack configuration snapshot, returned by NET_CFG_GET.
///
/// Packed C layout — userspace casts the result buffer to this.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetConfigInfo {
    /// Stack state: 0=unconfigured, 1=dhcp_discovering, 2=ready, 3=error.
    pub state: u32,
    /// Bit 0: DHCP active, bit 1: has gateway, bit 2: has DNS.
    pub flags: u32,
    /// IPv4 address (network byte order).
    pub ipv4_addr: u32,
    /// CIDR prefix length.
    pub prefix_len: u8,
    pub _pad0: [u8; 3],
    /// Gateway IPv4 (network byte order).
    pub gateway: u32,
    /// Primary DNS (network byte order).
    pub dns_primary: u32,
    /// Secondary DNS (network byte order).
    pub dns_secondary: u32,
    /// Current MAC address (6 bytes).
    pub mac: [u8; 6],
    pub _pad1: [u8; 2],
    /// Current MTU.
    pub mtu: u32,
    /// NUL-terminated hostname (max 63 + NUL).
    pub hostname: [u8; 64],
}

/// Network statistics, returned by NET_POLL_STATS.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    /// Number of active TCP sockets.
    pub tcp_active: u32,
    pub _pad: u32,
}

/// UDP send function signature.
type UdpSendFn =
    unsafe fn(handle: i64, dest_ip: u32, dest_port: u16, buf: *const u8, len: usize) -> i64;
/// UDP receive function signature.
type UdpRecvFn = unsafe fn(handle: i64, buf: *mut u8, len: usize, src_out: *mut u8) -> i64;

/// Network stack function-pointer table.
///
/// Registered by the bootloader after it creates a `NetInterface<D>`.
/// Every function returns >=0 on success (or a meaningful value),
/// negative on error.
#[repr(C)]
pub struct NetStackOps {
    // tcp
    /// Create a TCP socket.  Returns handle (>=0) or negative error.
    pub tcp_socket: Option<unsafe fn() -> i64>,
    /// Connect.  `ip` is network-byte-order IPv4.
    pub tcp_connect: Option<unsafe fn(handle: i64, ip: u32, port: u16) -> i64>,
    /// Send.  Returns bytes sent (>=0) or negative error.
    pub tcp_send: Option<unsafe fn(handle: i64, buf: *const u8, len: usize) -> i64>,
    /// Receive.  Returns bytes received (>=0) or negative error.
    pub tcp_recv: Option<unsafe fn(handle: i64, buf: *mut u8, len: usize) -> i64>,
    /// Close a socket.
    pub tcp_close: Option<unsafe fn(handle: i64)>,
    /// Query TCP state.  Returns state ordinal (0=Closed..10=TimeWait).
    pub tcp_state: Option<unsafe fn(handle: i64) -> i64>,
    /// Bind + listen on a local port.  Returns 0 or negative error.
    pub tcp_listen: Option<unsafe fn(handle: i64, port: u16) -> i64>,
    /// Accept an incoming connection.  Returns new handle or negative.
    pub tcp_accept: Option<unsafe fn(listen_handle: i64) -> i64>,
    /// Half-close: shutdown write side.  Returns 0 or negative.
    pub tcp_shutdown: Option<unsafe fn(handle: i64) -> i64>,
    /// Set TCP_NODELAY (Nagle disable). arg: 1=on 0=off.
    pub tcp_nodelay: Option<unsafe fn(handle: i64, on: i64) -> i64>,
    /// Set keepalive interval in ms.  0=disable.
    pub tcp_keepalive: Option<unsafe fn(handle: i64, ms: u64) -> i64>,

    // udp
    /// Create a UDP socket.  Returns handle (>=0) or negative error.
    pub udp_socket: Option<unsafe fn() -> i64>,
    /// Send a datagram.  `dest_ip` is NBO IPv4.  Returns bytes sent.
    pub udp_send_to: Option<UdpSendFn>,
    /// Receive a datagram.  Writes sender IP (NBO) + port into `src_out`.
    /// Returns bytes received (>=0), 0 if nothing available.
    /// `src_out` layout: [u32 ip_nbo, u16 port, u16 _pad] = 8 bytes.
    pub udp_recv_from: Option<UdpRecvFn>,
    /// Close a UDP socket.
    pub udp_close: Option<unsafe fn(handle: i64)>,

    // dns
    /// Start an async DNS query.  Returns query handle or negative.
    pub dns_start: Option<unsafe fn(name: *const u8, len: usize) -> i64>,
    /// Poll a DNS query.  Writes 4-byte IPv4 to `out`.
    /// Returns 0 if resolved, 1 if pending, negative on error.
    pub dns_result: Option<unsafe fn(query: i64, out: *mut u8) -> i64>,
    /// Override DNS servers.  `servers` points to packed u32 IPv4 addrs.
    pub dns_set_servers: Option<unsafe fn(servers: *const u32, count: usize) -> i64>,

    // configuration
    /// Fill a `NetConfigInfo` at `buf`.
    pub cfg_get: Option<unsafe fn(buf: *mut u8) -> i64>,
    /// Switch to DHCP mode.
    pub cfg_dhcp: Option<unsafe fn() -> i64>,
    /// Set a static IPv4.  `ip` and `gateway` are NBO.
    pub cfg_static_ip: Option<unsafe fn(ip: u32, prefix_len: u8, gateway: u32) -> i64>,
    /// Set the hostname (for DHCP FQDN option, etc.).
    pub cfg_hostname: Option<unsafe fn(name: *const u8, len: usize) -> i64>,

    // poll / stats
    /// Drive the smoltcp stack (DHCP, ARP, TCP timers).  Returns 1 if
    /// any socket activity occurred, 0 otherwise.
    pub poll_drive: Option<unsafe fn(timestamp_ms: u64) -> i64>,
    /// Fill a `NetStats` at `buf`.
    pub poll_stats: Option<unsafe fn(buf: *mut u8) -> i64>,
}

static mut NET_STACK_OPS: NetStackOps = NetStackOps {
    tcp_socket: None,
    tcp_connect: None,
    tcp_send: None,
    tcp_recv: None,
    tcp_close: None,
    tcp_state: None,
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

// userspace-triggered network bring-up callback.
// bootloader registers this so the kernel can stay offline by default.
static mut NET_ACTIVATE_FN: Option<unsafe fn() -> i64> = None;

/// Register network stack function pointers.
///
/// Called by the bootloader after it creates a `NetInterface<D>` and
/// wraps its methods into `unsafe fn` closures.
pub unsafe fn register_net_stack(ops: NetStackOps) {
    NET_STACK_OPS = ops;
}

/// Register userspace-triggered network activation callback.
///
/// This callback is invoked by `SYS_NET_CFG(NET_CFG_ACTIVATE)`.
/// Return values:
/// - `0`  => activated now
/// - `>0` => already active / no-op success
/// - `<0` => failure
pub unsafe fn register_net_activation(callback: unsafe fn() -> i64) {
    NET_ACTIVATE_FN = Some(callback);
}

/// Check if a network stack is registered.
fn net_stack_present() -> bool {
    unsafe { NET_STACK_OPS.tcp_socket.is_some() }
}

const ENOSYS_NET: u64 = u64::MAX - 37;

// SYS_NET (38) — TCP socket operations (multiplexed via subcmd)

/// `SYS_NET(subcmd, a2, a3, a4) → result`
pub unsafe fn sys_net(subcmd: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        // TCP_SOCKET() → handle
        NET_TCP_SOCKET => match NET_STACK_OPS.tcp_socket {
            Some(f) => {
                let h = f();
                if h < 0 {
                    ENOMEM
                } else {
                    h as u64
                }
            }
            None => ENOSYS_NET,
        },
        // TCP_CONNECT(handle, ipv4_nbo, port) → 0
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_SEND(handle, buf_ptr, buf_len) → bytes_sent
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_RECV(handle, buf_ptr, buf_len) → bytes_received
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_CLOSE(handle) → 0
        NET_TCP_CLOSE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_close {
                Some(f) => {
                    f(handle);
                    0
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_STATE(handle) → state ordinal
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_LISTEN(handle, port) → 0
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_ACCEPT(listen_handle) → new handle
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_SHUTDOWN(handle) → 0
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_NODELAY(handle, on) → 0
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
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_KEEPALIVE(handle, interval_ms) → 0
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
                }
                None => ENOSYS_NET,
            }
        }
        // udp sub-commands
        // UDP_SOCKET() → handle
        NET_UDP_SOCKET => match NET_STACK_OPS.udp_socket {
            Some(f) => {
                let h = f();
                if h < 0 {
                    ENOMEM
                } else {
                    h as u64
                }
            }
            None => ENOSYS_NET,
        },
        // UDP_SEND_TO(handle, dest_ip_nbo, dest_port | buf_ptr, buf_len)
        // a2 = handle, a3 = dest_ip_nbo | (port << 32), a4 = buf_ptr | (len << 32)
        // Re-pack: dest_ip in lower 32 of a3, port in upper 16 bits
        // Actually, with 4 args available (subcmd, a2, a3, a4):
        //   subcmd=12, a2=handle, a3=packed(ip:32|port:16|pad:16), a4=buf_ptr
        // But we need 5 args (handle, ip, port, buf, len). Solution: pack
        // ip+port into a3 and buf+len into a4 won't work (64-bit ptrs).
        // Use 5th arg via a4 as buf_ptr, and pass len via a2 upper bits.
        // Better: repack. handle=a2, dest_addr_ptr=a3 (8-byte struct), buf=a4.
        //
        // Cleanest ABI for UDP send_to with 4 args:
        //   a2 = handle
        //   a3 = pointer to UdpTarget { ip_nbo: u32, port: u16, _pad: u16 }
        //   a4 = pointer to (buf_ptr: u64, buf_len: u64) pair
        //
        // No — too many indirections. Use the 5-arg dispatch variant:
        //   The dispatch passes a1..a4 (4 user args after subcmd).
        //   a2=handle, a3=dest_ip_nbo, a4=dest_port|(len<<16), but len>65535
        //   is possible. Bad.
        //
        // Final design — message struct:
        //   a2 = handle
        //   a3 = pointer to UdpSendDesc { ip: u32, port: u16, _pad: u16, buf: *const u8, len: u64 }
        NET_UDP_SEND_TO => {
            let handle = a2 as i64;
            // a3 points to UdpSendDesc in user memory
            let desc_size = 24u64; // u32 + u16 + u16 + u64 + u64 = 24
            if !validate_user_buf(a3, desc_size) {
                return EFAULT;
            }
            let desc = a3 as *const u8;
            let ip = *(desc as *const u32);
            let port = *((desc.add(4)) as *const u16);
            let buf_ptr = *((desc.add(8)) as *const u64);
            let buf_len = *((desc.add(16)) as *const u64);
            if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
                return EFAULT;
            }
            if buf_len > 65535 {
                return EINVAL;
            } // UDP max payload
            match NET_STACK_OPS.udp_send_to {
                Some(f) => {
                    let rc = f(handle, ip, port, buf_ptr as *const u8, buf_len as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // UDP_RECV_FROM(handle, buf_ptr, buf_len)
        //   a2 = handle
        //   a3 = pointer to UdpRecvDesc { buf: *mut u8, buf_len: u64, src_ip: u32, src_port: u16, _pad: u16 }
        NET_UDP_RECV_FROM => {
            let handle = a2 as i64;
            let desc_size = 24u64; // *mut u8(8) + u64(8) + u32(4) + u16(2) + u16(2) = 24
            if !validate_user_buf(a3, desc_size) {
                return EFAULT;
            }
            let desc = a3 as *mut u8;
            let buf_ptr = *(desc as *const u64);
            let buf_len = *((desc.add(8)) as *const u64);
            if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
                return EFAULT;
            }
            // src_out is at offset 16 in the desc (4 + 2 + 2 = 8 bytes for src info)
            let src_out = desc.add(16);
            match NET_STACK_OPS.udp_recv_from {
                Some(f) => {
                    let rc = f(handle, buf_ptr as *mut u8, buf_len as usize, src_out);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // UDP_CLOSE(handle) → 0
        NET_UDP_CLOSE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.udp_close {
                Some(f) => {
                    f(handle);
                    0
                }
                None => ENOSYS_NET,
            }
        }
        _ => EINVAL,
    }
}

// SYS_DNS (39) — DNS resolution

/// `SYS_DNS(subcmd, a2, a3) → result`
pub unsafe fn sys_dns(subcmd: u64, a2: u64, a3: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        // DNS_START(name_ptr, name_len) → query handle
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
                }
                None => ENOSYS_NET,
            }
        }
        // DNS_RESULT(query_handle, result_buf_ptr) → 0=resolved, 1=pending
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
                }
                None => ENOSYS_NET,
            }
        }
        // DNS_SET_SERVERS(servers_ptr, count)
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
                }
                None => ENOSYS_NET,
            }
        }
        _ => EINVAL,
    }
}

// SYS_NET_CFG (40) — IP stack configuration

/// `SYS_NET_CFG(subcmd, a2, a3, a4) → result`
pub unsafe fn sys_net_cfg(subcmd: u64, a2: u64, a3: u64, _a4: u64) -> u64 {
    match subcmd {
        // CFG_GET(buf_ptr) — works even without stack (returns zeroed)
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
                }
                None => {
                    // No stack: zero-fill so userspace sees state=0 (unconfigured)
                    core::ptr::write_bytes(a2 as *mut u8, 0, size as usize);
                    0
                }
            }
        }
        // CFG_ACTIVATE() — explicit userspace network bring-up.
        // offline by default; userspace opts in when it wants networking.
        NET_CFG_ACTIVATE => match NET_ACTIVATE_FN {
            Some(f) => {
                let rc = f();
                if rc < 0 {
                    EIO
                } else {
                    rc as u64
                }
            }
            None => ENODEV,
        },
        // All remaining subcmds require the stack.
        _ if !net_stack_present() => ENODEV,

        // CFG_DHCP() — enable DHCP
        NET_CFG_DHCP => match NET_STACK_OPS.cfg_dhcp {
            Some(f) => {
                let rc = f();
                if rc < 0 {
                    EIO
                } else {
                    0
                }
            }
            None => ENOSYS_NET,
        },
        // CFG_STATIC(ip_nbo, prefix_gw_packed, 0)
        // prefix_gw_packed = (prefix_len << 32) | gateway_nbo
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
                }
                None => ENOSYS_NET,
            }
        }
        // CFG_HOSTNAME(name_ptr, name_len)
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
                }
                None => ENOSYS_NET,
            }
        }
        // nic hardware control (subcmd >= 128)
        // These go directly to NicOps.ctrl, bypassing the IP stack.
        // This is the exokernel escape hatch: promisc, MAC spoof,
        // VLAN, offloads, ring sizing, interrupt coalescing.
        128.. => {
            let nic_cmd = (subcmd - 128) as u32;
            sys_nic_ctrl(nic_cmd as u64, a2)
        }
        _ => EINVAL,
    }
}

// SYS_NET_POLL (41) — drive the stack & query statistics

/// `SYS_NET_POLL(subcmd, a2) → result`
pub unsafe fn sys_net_poll(subcmd: u64, a2: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        // POLL_DRIVE(timestamp_ms) → 0/1 (activity)
        NET_POLL_DRIVE => match NET_STACK_OPS.poll_drive {
            Some(f) => {
                let rc = f(a2);
                if rc < 0 {
                    EIO
                } else {
                    rc as u64
                }
            }
            None => ENOSYS_NET,
        },
        // POLL_STATS(buf_ptr) → 0
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
                }
                None => ENOSYS_NET,
            }
        }
        _ => EINVAL,
    }
}
