// BSD socket family over the unified fd table (sockets are O_SOCKET fds).
//
// Each socket is an FdKind::Socket fd whose `cookie` carries the smoltcp backend
// handle plus the cached address/option state std needs for getsockname /
// getpeername / SO_ERROR. Ops route through the typed bridge in `handler::net`;
// this layer never touches the raw `NetStackOps`.
//
// Blocking ops do NOT busy-poll: they park on the per-socket io::readiness token
// for one poll slice, drive the stack against the monotonic clock, and re-check.
// Once the net glue calls `set_ready` on arrival, wakeups go event-driven and the
// slice deadline is just a safety re-poll.

use super::common::*;
use super::net::{
    bridge_tcp_accept, bridge_tcp_can_recv, bridge_tcp_can_send, bridge_tcp_close,
    bridge_tcp_connect, bridge_tcp_keepalive, bridge_tcp_listen, bridge_tcp_nodelay,
    bridge_tcp_recv, bridge_tcp_send, bridge_tcp_shutdown, bridge_tcp_socket, bridge_tcp_state,
    bridge_udp_close, bridge_udp_recv_from, bridge_udp_send_to, bridge_udp_socket, monotonic_ms,
    net_drive, net_present, BRIDGE_ABSENT,
};
use crate::hal;
use crate::io::readiness;
use crate::schedular::SCHEDULER;
use morpheus_foundation::errno::{
    EADDRINUSE, EAFNOSUPPORT, EAGAIN, ECONNREFUSED, EDESTADDRREQ, EINPROGRESS, EISCONN, EMSGSIZE,
    ENOPROTOOPT, ENOTCONN, ENOTSOCK, EOPNOTSUPP, EPIPE, EPROTONOSUPPORT, ETIMEDOUT,
};
use morpheus_foundation::flags::open_flags::{O_CLOEXEC, O_NONBLOCK, O_SOCKET};
use morpheus_foundation::flags::{EPOLLERR, EPOLLHUP, EPOLLIN, EPOLLOUT, EPOLLRDHUP};
use morpheus_foundation::net::{
    AF_INET, AF_INET6, IPPROTO_IP, IPPROTO_TCP, IP_TTL, MSG_DONTWAIT, SHUT_RD, SHUT_RDWR, SHUT_WR,
    SOCK_CLOEXEC, SOCK_DGRAM, SOCK_NONBLOCK, SOCK_STREAM, SOL_SOCKET, SO_BROADCAST, SO_ERROR,
    SO_KEEPALIVE, SO_RCVBUF, SO_RCVTIMEO, SO_REUSEADDR, SO_REUSEPORT, SO_SNDBUF, SO_SNDTIMEO,
    TCP_NODELAY,
};
use morpheus_foundation::types::{KTimeval, SockAddrIn, SockAddrStorage};

// smoltcp TcpState ordinals (mirror of libmorpheus::net::TcpState).
const ST_CLOSED: i64 = 0;
const ST_SYN_SENT: i64 = 2;
const ST_SYN_RECEIVED: i64 = 3;
const ST_ESTABLISHED: i64 = 4;
const ST_FIN_WAIT1: i64 = 5;
const ST_FIN_WAIT2: i64 = 6;
const ST_CLOSE_WAIT: i64 = 7;

const SOCK_STREAM_TAG: u8 = 1;
const SOCK_DGRAM_TAG: u8 = 2;

// cookie[10] state bits.
const SF_BOUND: u8 = 0x01;
const SF_CONNECTED: u8 = 0x02;
const SF_LISTENING: u8 = 0x04;
const SF_SHUT_RD: u8 = 0x08;
const SF_SHUT_WR: u8 = 0x10;

/// Re-poll granularity while a blocking op is parked (ms). Short enough that
/// stack timers stay live, long enough that the thread actually sleeps.
const POLL_SLICE_MS: u64 = 2;

/// Decoded view of a socket fd's `cookie` (see module layout). 32-byte cookie:
/// `[0..8]` handle, `[8]` type, `[9]` domain, `[10]` state, `[11]` ttl,
/// `[12..14]` local_port, `[14..16]` peer_port, `[16..20]` peer_ip(nbo),
/// `[20..24]` local_ip(nbo), `[24..28]` rcvtimeo_ms, `[28..32]` sndtimeo_ms.
#[derive(Clone, Copy)]
struct SockMeta {
    handle: i64,
    ty: u8,
    domain: u8,
    sflags: u8,
    ttl: u8,
    local_port: u16,
    peer_port: u16,
    peer_ip: u32,
    local_ip: u32,
    rcvtimeo_ms: u32,
    sndtimeo_ms: u32,
}

impl SockMeta {
    fn from_cookie(c: &[u8; 32]) -> Self {
        let rd = |a: usize, b: usize| {
            let mut t = [0u8; 8];
            t[..b - a].copy_from_slice(&c[a..b]);
            t
        };
        Self {
            handle: i64::from_ne_bytes(rd(0, 8)),
            ty: c[8],
            domain: c[9],
            sflags: c[10],
            ttl: c[11],
            local_port: u16::from_le_bytes([c[12], c[13]]),
            peer_port: u16::from_le_bytes([c[14], c[15]]),
            peer_ip: u32::from_ne_bytes([c[16], c[17], c[18], c[19]]),
            local_ip: u32::from_ne_bytes([c[20], c[21], c[22], c[23]]),
            rcvtimeo_ms: u32::from_le_bytes([c[24], c[25], c[26], c[27]]),
            sndtimeo_ms: u32::from_le_bytes([c[28], c[29], c[30], c[31]]),
        }
    }

    fn to_cookie(self) -> [u8; 32] {
        let mut c = [0u8; 32];
        c[..8].copy_from_slice(&self.handle.to_ne_bytes());
        c[8] = self.ty;
        c[9] = self.domain;
        c[10] = self.sflags;
        c[11] = self.ttl;
        c[12..14].copy_from_slice(&self.local_port.to_le_bytes());
        c[14..16].copy_from_slice(&self.peer_port.to_le_bytes());
        c[16..20].copy_from_slice(&self.peer_ip.to_ne_bytes());
        c[20..24].copy_from_slice(&self.local_ip.to_ne_bytes());
        c[24..28].copy_from_slice(&self.rcvtimeo_ms.to_le_bytes());
        c[28..32].copy_from_slice(&self.sndtimeo_ms.to_le_bytes());
        c
    }

    fn is_stream(&self) -> bool {
        self.ty == SOCK_STREAM_TAG
    }
}

/// Fetch the socket meta for `fd`, validating it is an open socket.
unsafe fn meta_of(fd: u64) -> Result<SockMeta, u64> {
    let t = SCHEDULER.current_fd_table_mut();
    match t.get(fd as usize) {
        Some(d) if d.is_socket() => {
            let mut c = [0u8; 32];
            c.copy_from_slice(&d.cookie[..32]);
            Ok(SockMeta::from_cookie(&c))
        },
        Some(_) => Err(ENOTSOCK),
        None => Err(EBADF),
    }
}

/// Persist updated meta back into `fd`'s cookie.
unsafe fn store_meta(fd: u64, m: &SockMeta) {
    let t = SCHEDULER.current_fd_table_mut();
    if let Some(d) = t.get_mut(fd as usize) {
        d.cookie[..32].copy_from_slice(&m.to_cookie());
    }
}

/// True if the OFD has `O_NONBLOCK`, or `flags` carries `MSG_DONTWAIT`.
unsafe fn nonblocking(fd: u64, msg_flags: u64) -> bool {
    if msg_flags & MSG_DONTWAIT != 0 {
        return true;
    }
    let t = SCHEDULER.current_fd_table_mut();
    t.status_flags(fd as usize).unwrap_or(0) & O_NONBLOCK != 0
}

/// Map a bridge negative return to an errno. `BRIDGE_ABSENT` (op never wired) is
/// honest `ENOSYS`; any other negative is the stack reporting a generic failure.
fn bridge_err(rc: i64) -> u64 {
    if rc == BRIDGE_ABSENT {
        ENOSYS
    } else {
        EIO
    }
}

/// TSC deadline one poll slice out (0 if the timer has no calibrated frequency,
/// which `wait_ready` treats as block-forever — acceptable, a later set_ready or
/// the next tick re-checks).
fn slice_deadline() -> u64 {
    let hz = hal().timer().tsc_frequency();
    if hz == 0 {
        return 0;
    }
    hal().timer().read_tsc() + (hz / 1000).max(1) * POLL_SLICE_MS
}

/// Park on `token` for one poll slice (or until readiness), having driven the
/// stack first so in-flight segments make progress.
unsafe fn park_one_slice(token: u64, interest: u32) {
    net_drive();
    readiness::register(token);
    let _ = readiness::wait_ready(
        token,
        interest | EPOLLERR | EPOLLHUP | EPOLLRDHUP,
        slice_deadline(),
    );
}

/// Read a `SockAddrStorage`/`SockAddrIn` from user memory. Returns
/// `(ip_nbo, port_host)`. Only AF_INET is backed by the IPv4-only bridge.
unsafe fn read_sockaddr(addr: u64, addrlen: u64) -> Result<(u32, u16), u64> {
    if addrlen < core::mem::size_of::<SockAddrIn>() as u64 {
        return Err(EINVAL);
    }
    if !validate_user_buf(addr, core::mem::size_of::<SockAddrIn>() as u64) {
        return Err(EFAULT);
    }
    let sa = &*(addr as *const SockAddrStorage);
    match sa.sa_family as u64 {
        AF_INET => {
            let sin = &*(addr as *const SockAddrIn);
            // sin_addr is already network byte order (bridge wants nbo); sin_port
            // is network byte order and the bridge wants host order.
            Ok((sin.sin_addr, u16::from_be(sin.sin_port)))
        },
        AF_INET6 => Err(EAFNOSUPPORT),
        _ => Err(EAFNOSUPPORT),
    }
}

/// Write an AF_INET address back to a user `*sa` + `*addrlen` (in/out capacity).
/// `addr == 0` skips (POSIX: caller does not want the address). `ip_nbo` network
/// byte order, `port_host` host order.
unsafe fn write_sockaddr_in(
    addr: u64,
    addrlen_ptr: u64,
    ip_nbo: u32,
    port_host: u16,
) -> Result<(), u64> {
    if addr == 0 {
        return Ok(());
    }
    let want = core::mem::size_of::<SockAddrIn>();
    let cap = if addrlen_ptr != 0 {
        if !validate_user_buf(addrlen_ptr, 4) {
            return Err(EFAULT);
        }
        *(addrlen_ptr as *const u32) as usize
    } else {
        want
    };
    let n = cap.min(want);
    if n > 0 && !validate_user_buf(addr, n as u64) {
        return Err(EFAULT);
    }
    let sin = SockAddrIn {
        sin_family: AF_INET as u16,
        sin_port: port_host.to_be(),
        sin_addr: ip_nbo,
        sin_zero: [0u8; 8],
    };
    let src = core::slice::from_raw_parts(&sin as *const SockAddrIn as *const u8, want);
    core::ptr::copy_nonoverlapping(src.as_ptr(), addr as *mut u8, n);
    if addrlen_ptr != 0 {
        *(addrlen_ptr as *mut u32) = want as u32;
    }
    Ok(())
}

/// SYS_SOCKET: `domain,type,protocol -> fd | -errno`.
pub unsafe fn sys_socket(domain: u64, ty: u64, _protocol: u64) -> u64 {
    if !net_present() {
        return ENODEV;
    }
    if domain == AF_INET6 {
        return EAFNOSUPPORT;
    }
    if domain != AF_INET {
        return EAFNOSUPPORT;
    }
    let base = ty & 0xff;
    let tag = match base {
        SOCK_STREAM => SOCK_STREAM_TAG,
        SOCK_DGRAM => SOCK_DGRAM_TAG,
        _ => return EPROTONOSUPPORT,
    };
    let nonblock = ty & SOCK_NONBLOCK != 0;
    let cloexec = ty & SOCK_CLOEXEC != 0;

    let handle = if tag == SOCK_STREAM_TAG {
        bridge_tcp_socket()
    } else {
        bridge_udp_socket()
    };
    if handle < 0 {
        return if handle == BRIDGE_ABSENT {
            ENOSYS
        } else {
            ENOMEM
        };
    }

    let t = SCHEDULER.current_fd_table_mut();
    let fd = match t.alloc() {
        Some(fd) => fd,
        None => {
            if tag == SOCK_STREAM_TAG {
                bridge_tcp_close(handle);
            } else {
                bridge_udp_close(handle);
            }
            return EMFILE;
        },
    };

    let meta = SockMeta {
        handle,
        ty: tag,
        domain: AF_INET as u8,
        sflags: 0,
        ttl: 64,
        local_port: 0,
        peer_port: 0,
        peer_ip: 0,
        local_ip: 0,
        rcvtimeo_ms: 0,
        sndtimeo_ms: 0,
    };

    let mut state = crate::storage::fs_api::FdState::empty();
    state.kind = crate::storage::fs_api::FdKind::Socket;
    state.flags =
        O_SOCKET | if nonblock { O_NONBLOCK } else { 0 } | if cloexec { O_CLOEXEC } else { 0 };
    state.cloexec = cloexec;
    state.cookie[..32].copy_from_slice(&meta.to_cookie());

    if !t.set(fd, state) {
        if tag == SOCK_STREAM_TAG {
            bridge_tcp_close(handle);
        } else {
            bridge_udp_close(handle);
        }
        return EMFILE;
    }
    readiness::register(readiness::socket_token(handle as u64));
    fd as u64
}

/// SYS_BIND: `fd,*const SockAddrStorage,addrlen -> 0 | -errno`. The IPv4 bridge
/// has no standalone bind; TCP binds via `listen(port)`, UDP auto-binds. We cache
/// the requested local port so getsockname reports it and listen reuses it.
pub unsafe fn sys_bind(fd: u64, addr: u64, addrlen: u64) -> u64 {
    let mut m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let (ip_nbo, port) = match read_sockaddr(addr, addrlen) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if m.sflags & SF_BOUND != 0 {
        return EINVAL;
    }
    m.local_ip = ip_nbo;
    m.local_port = port;
    m.sflags |= SF_BOUND;
    store_meta(fd, &m);
    0
}

/// SYS_LISTEN: `fd,backlog -> 0 | -errno`.
pub unsafe fn sys_listen(fd: u64, _backlog: u64) -> u64 {
    let mut m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if !m.is_stream() {
        return EOPNOTSUPP;
    }
    let rc = bridge_tcp_listen(m.handle, m.local_port);
    if rc < 0 {
        // The stack rejects a re-listen / in-use port; surface EADDRINUSE.
        return if rc == BRIDGE_ABSENT {
            ENOSYS
        } else {
            EADDRINUSE
        };
    }
    m.sflags |= SF_LISTENING;
    store_meta(fd, &m);
    readiness::register(readiness::socket_token(m.handle as u64));
    0
}

/// SYS_ACCEPT: `fd,*mut SockAddrStorage,*mut u32 addrlen,flags -> newfd | -errno`.
/// The IPv4 bridge mints the accepted connection as a fresh handle, re-arms the
/// listener, and reports the connecting peer's endpoint (network-byte-order ip +
/// host-order port) so the returned `*sa` / `getpeername` are the real client.
pub unsafe fn sys_accept(fd: u64, addr: u64, addrlen: u64, flags: u64) -> u64 {
    let m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if !m.is_stream() {
        return EOPNOTSUPP;
    }
    if m.sflags & SF_LISTENING == 0 {
        return EINVAL;
    }
    let nb = nonblocking(fd, 0);
    let token = readiness::socket_token(m.handle as u64);
    let start = monotonic_ms();

    loop {
        net_drive();
        let mut peer_ip: u32 = 0;
        let mut peer_port: u16 = 0;
        let h = bridge_tcp_accept(m.handle, &mut peer_ip, &mut peer_port);
        if h >= 0 {
            return finish_accept(h, peer_ip, peer_port, m, addr, addrlen, flags);
        }
        if h == BRIDGE_ABSENT {
            return ENOSYS;
        }
        readiness::clear_ready(token, EPOLLIN);
        if nb {
            return EAGAIN;
        }
        if timed_out(start, m.rcvtimeo_ms) {
            return EAGAIN;
        }
        park_one_slice(token, EPOLLIN);
    }
}

/// Install an accepted backend handle as a new connected socket fd. `peer_ip`
/// (network byte order) / `peer_port` (host order) come from the accept bridge;
/// `listener` supplies the accepted socket's local endpoint (same port).
unsafe fn finish_accept(
    handle: i64,
    peer_ip: u32,
    peer_port: u16,
    listener: SockMeta,
    addr: u64,
    addrlen: u64,
    flags: u64,
) -> u64 {
    let cloexec = flags & SOCK_CLOEXEC != 0;
    let nonblock = flags & SOCK_NONBLOCK != 0;

    let t = SCHEDULER.current_fd_table_mut();
    let newfd = match t.alloc() {
        Some(fd) => fd,
        None => {
            bridge_tcp_close(handle);
            return EMFILE;
        },
    };
    // Cache the peer so getpeername() and the accepted stream's peer_addr() are
    // correct; local_ip/port inherit the listener (the accepted socket shares it).
    let meta = SockMeta {
        handle,
        ty: SOCK_STREAM_TAG,
        domain: AF_INET as u8,
        sflags: SF_CONNECTED,
        ttl: 64,
        local_port: listener.local_port,
        peer_port,
        peer_ip,
        local_ip: listener.local_ip,
        rcvtimeo_ms: 0,
        sndtimeo_ms: 0,
    };
    let mut state = crate::storage::fs_api::FdState::empty();
    state.kind = crate::storage::fs_api::FdKind::Socket;
    state.flags =
        O_SOCKET | if nonblock { O_NONBLOCK } else { 0 } | if cloexec { O_CLOEXEC } else { 0 };
    state.cloexec = cloexec;
    state.cookie[..32].copy_from_slice(&meta.to_cookie());
    if !t.set(newfd, state) {
        bridge_tcp_close(handle);
        return EMFILE;
    }
    readiness::register(readiness::socket_token(handle as u64));
    // Report the real peer ip:port. Best-effort: a bad user buffer must not leak
    // the freshly minted fd, so this write happens last.
    let _ = write_sockaddr_in(addr, addrlen, peer_ip, peer_port);
    newfd as u64
}

/// SYS_CONNECT: `fd,*const SockAddrStorage,addrlen -> 0 | -errno`.
pub unsafe fn sys_connect(fd: u64, addr: u64, addrlen: u64) -> u64 {
    let mut m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let (ip_nbo, port) = match read_sockaddr(addr, addrlen) {
        Ok(v) => v,
        Err(e) => return e,
    };

    if !m.is_stream() {
        // Connected UDP: just cache the default destination.
        m.peer_ip = ip_nbo;
        m.peer_port = port;
        m.sflags |= SF_CONNECTED;
        store_meta(fd, &m);
        return 0;
    }

    if m.sflags & SF_CONNECTED != 0 {
        return EISCONN;
    }

    let token = readiness::socket_token(m.handle as u64);
    let nb = nonblocking(fd, 0);

    // Initiate the active open only once.
    if m.sflags & SF_SHUT_WR == 0 && m.peer_port == 0 {
        let rc = bridge_tcp_connect(m.handle, ip_nbo, port);
        if rc < 0 {
            return bridge_err(rc);
        }
        m.peer_ip = ip_nbo;
        m.peer_port = port;
        store_meta(fd, &m);
    }

    // Grace before declaring a `CLOSED` socket "refused": smoltcp may still read
    // CLOSED for a tick after connect() before the SYN goes out, so we only treat
    // CLOSED as a refusal once the handshake has visibly progressed OR the grace
    // has elapsed (covers a RST that bounces straight back to CLOSED).
    const CONNECT_GRACE_MS: u64 = 250;
    let start = monotonic_ms();
    let mut saw_progress = false;
    loop {
        net_drive();
        let st = bridge_tcp_state(m.handle);
        if st == ST_ESTABLISHED {
            m.sflags |= SF_CONNECTED;
            store_meta(fd, &m);
            readiness::set_ready(token, EPOLLOUT);
            return 0;
        }
        if matches!(st, ST_SYN_SENT | ST_SYN_RECEIVED) {
            saw_progress = true;
        }
        let elapsed = monotonic_ms().saturating_sub(start);
        let refused = matches!(st, ST_CLOSE_WAIT | ST_FIN_WAIT1 | ST_FIN_WAIT2)
            || (st == ST_CLOSED && (saw_progress || elapsed >= CONNECT_GRACE_MS));
        if refused {
            readiness::set_ready(token, EPOLLERR | EPOLLHUP);
            return ECONNREFUSED;
        }
        if nb {
            return EINPROGRESS;
        }
        if timed_out(start, m.sndtimeo_ms) {
            return ETIMEDOUT;
        }
        park_one_slice(token, EPOLLOUT);
    }
}

/// SYS_SENDTO: `fd,buf,len,flags,*const SockAddrStorage,addrlen -> n | -errno`.
pub unsafe fn sys_sendto(fd: u64, buf: u64, len: u64, flags: u64, addr: u64, addrlen: u64) -> u64 {
    let m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if len > 0 && !validate_user_buf(buf, len) {
        return EFAULT;
    }
    if m.is_stream() {
        do_tcp_send(fd, m, buf, len, flags)
    } else {
        do_udp_send(fd, m, buf, len, flags, addr, addrlen)
    }
}

unsafe fn do_tcp_send(fd: u64, m: SockMeta, buf: u64, len: u64, flags: u64) -> u64 {
    if m.sflags & SF_CONNECTED == 0 {
        return ENOTCONN;
    }
    if m.sflags & SF_SHUT_WR != 0 {
        return EPIPE;
    }
    let token = readiness::socket_token(m.handle as u64);
    let nb = nonblocking(fd, flags);
    let start = monotonic_ms();
    loop {
        net_drive();
        let rc = bridge_tcp_send(m.handle, buf as *const u8, len as usize);
        if rc < 0 {
            return bridge_err(rc);
        }
        if rc > 0 {
            readiness::set_ready(token, EPOLLOUT);
            return rc as u64;
        }
        if len == 0 {
            return 0;
        }
        // Send buffer full — is the connection even writable?
        let st = bridge_tcp_state(m.handle);
        if !matches!(
            st,
            ST_ESTABLISHED | ST_CLOSE_WAIT | ST_SYN_SENT | ST_SYN_RECEIVED
        ) {
            return EPIPE;
        }
        readiness::clear_ready(token, EPOLLOUT);
        if nb {
            return EAGAIN;
        }
        if timed_out(start, m.sndtimeo_ms) {
            return EAGAIN;
        }
        park_one_slice(token, EPOLLOUT);
    }
}

unsafe fn do_udp_send(
    fd: u64,
    m: SockMeta,
    buf: u64,
    len: u64,
    _flags: u64,
    addr: u64,
    addrlen: u64,
) -> u64 {
    let (ip_nbo, port) = if addr != 0 {
        match read_sockaddr(addr, addrlen) {
            Ok(v) => v,
            Err(e) => return e,
        }
    } else if m.sflags & SF_CONNECTED != 0 {
        (m.peer_ip, m.peer_port)
    } else {
        return EDESTADDRREQ;
    };
    if len > 65507 {
        return EMSGSIZE;
    }
    let _ = fd;
    net_drive();
    let rc = bridge_udp_send_to(m.handle, ip_nbo, port, buf as *const u8, len as usize);
    if rc < 0 {
        return bridge_err(rc);
    }
    rc as u64
}

/// SYS_RECVFROM: `fd,buf,len,flags,*mut SockAddrStorage,*mut u32 addrlen -> n | -errno`.
pub unsafe fn sys_recvfrom(
    fd: u64,
    buf: u64,
    len: u64,
    flags: u64,
    addr: u64,
    addrlen: u64,
) -> u64 {
    let m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if len > 0 && !validate_user_buf(buf, len) {
        return EFAULT;
    }
    if m.is_stream() {
        do_tcp_recv(fd, m, buf, len, flags, addr, addrlen)
    } else {
        do_udp_recv(fd, m, buf, len, flags, addr, addrlen)
    }
}

unsafe fn do_tcp_recv(
    fd: u64,
    m: SockMeta,
    buf: u64,
    len: u64,
    flags: u64,
    addr: u64,
    addrlen: u64,
) -> u64 {
    if m.sflags & SF_CONNECTED == 0 {
        return ENOTCONN;
    }
    if len == 0 {
        return 0;
    }
    if m.sflags & SF_SHUT_RD != 0 {
        return 0;
    }
    let token = readiness::socket_token(m.handle as u64);
    let nb = nonblocking(fd, flags);
    let start = monotonic_ms();
    loop {
        net_drive();
        let rc = bridge_tcp_recv(m.handle, buf as *mut u8, len as usize);
        if rc < 0 {
            return bridge_err(rc);
        }
        if rc > 0 {
            readiness::set_ready(token, EPOLLIN);
            let _ = write_sockaddr_in(addr, addrlen, m.peer_ip, m.peer_port);
            return rc as u64;
        }
        // No data: distinguish would-block from peer-closed EOF.
        let st = bridge_tcp_state(m.handle);
        let readable_state = matches!(
            st,
            ST_ESTABLISHED | ST_SYN_SENT | ST_SYN_RECEIVED | ST_FIN_WAIT1 | ST_FIN_WAIT2
        );
        if !readable_state {
            return 0; // orderly EOF (peer FIN drained / connection gone)
        }
        readiness::clear_ready(token, EPOLLIN);
        if nb {
            return EAGAIN;
        }
        if timed_out(start, m.rcvtimeo_ms) {
            return EAGAIN;
        }
        park_one_slice(token, EPOLLIN);
    }
}

unsafe fn do_udp_recv(
    fd: u64,
    m: SockMeta,
    buf: u64,
    len: u64,
    flags: u64,
    addr: u64,
    addrlen: u64,
) -> u64 {
    let token = readiness::socket_token(m.handle as u64);
    let nb = nonblocking(fd, flags);
    let start = monotonic_ms();
    // 8-byte src descriptor the bridge fills: ip(nbo,4) + port(host,2) + pad(2).
    let mut src = [0u8; 8];
    loop {
        net_drive();
        let rc = bridge_udp_recv_from(m.handle, buf as *mut u8, len as usize, src.as_mut_ptr());
        // rc >= 0 means a datagram was dequeued (possibly empty); rc < 0 is the
        // bridge's "no datagram queued" signal (would-block).
        if rc >= 0 {
            readiness::set_ready(token, EPOLLIN);
            let ip = u32::from_ne_bytes([src[0], src[1], src[2], src[3]]);
            let port = u16::from_le_bytes([src[4], src[5]]);
            let _ = write_sockaddr_in(addr, addrlen, ip, port);
            return rc as u64;
        }
        readiness::clear_ready(token, EPOLLIN);
        if nb {
            return EAGAIN;
        }
        if timed_out(start, m.rcvtimeo_ms) {
            return EAGAIN;
        }
        park_one_slice(token, EPOLLIN);
    }
}

/// SYS_GETSOCKNAME: `fd,*mut SockAddrStorage,*mut u32 addrlen -> 0 | -errno`.
pub unsafe fn sys_getsockname(fd: u64, addr: u64, addrlen: u64) -> u64 {
    let m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match write_sockaddr_in(addr, addrlen, m.local_ip, m.local_port) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// SYS_GETPEERNAME: `fd,*mut SockAddrStorage,*mut u32 addrlen -> 0 | -errno`.
pub unsafe fn sys_getpeername(fd: u64, addr: u64, addrlen: u64) -> u64 {
    let m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if m.sflags & SF_CONNECTED == 0 {
        return ENOTCONN;
    }
    match write_sockaddr_in(addr, addrlen, m.peer_ip, m.peer_port) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// SYS_SETSOCKOPT: `fd,level,optname,*const optval,optlen -> 0 | -errno`.
pub unsafe fn sys_setsockopt(fd: u64, level: u64, optname: u64, optval: u64, optlen: u64) -> u64 {
    let mut m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let read_i32 = || -> Result<i32, u64> {
        if optlen < 4 || !validate_user_buf(optval, 4) {
            return Err(EINVAL);
        }
        Ok(unsafe { *(optval as *const i32) })
    };

    match level {
        SOL_SOCKET => match optname {
            SO_REUSEADDR | SO_REUSEPORT | SO_BROADCAST | SO_SNDBUF | SO_RCVBUF => {
                // Accepted; smoltcp manages buffers itself, REUSEADDR is implicit.
                let _ = read_i32();
                0
            },
            SO_KEEPALIVE => {
                let on = match read_i32() {
                    Ok(v) => v != 0,
                    Err(e) => return e,
                };
                if m.is_stream() {
                    // Default keepalive cadence; TCP_KEEPIDLE refines it.
                    let _ = bridge_tcp_keepalive(m.handle, if on { 75_000 } else { 0 });
                }
                0
            },
            SO_RCVTIMEO | SO_SNDTIMEO => {
                if optlen < core::mem::size_of::<KTimeval>() as u64
                    || !validate_user_buf(optval, core::mem::size_of::<KTimeval>() as u64)
                {
                    return EINVAL;
                }
                let tv = *(optval as *const KTimeval);
                let ms = (tv.tv_sec as u64) * 1000 + (tv.tv_usec as u64) / 1000;
                if optname == SO_RCVTIMEO {
                    m.rcvtimeo_ms = ms.min(u32::MAX as u64) as u32;
                } else {
                    m.sndtimeo_ms = ms.min(u32::MAX as u64) as u32;
                }
                store_meta(fd, &m);
                0
            },
            _ => ENOPROTOOPT,
        },
        IPPROTO_TCP => match optname {
            TCP_NODELAY => {
                let on = match read_i32() {
                    Ok(v) => v != 0,
                    Err(e) => return e,
                };
                if !m.is_stream() {
                    return EOPNOTSUPP;
                }
                let rc = bridge_tcp_nodelay(m.handle, on);
                if rc < 0 {
                    bridge_err(rc)
                } else {
                    0
                }
            },
            _ => ENOPROTOOPT,
        },
        IPPROTO_IP => match optname {
            IP_TTL => {
                let v = match read_i32() {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                m.ttl = (v as u32 & 0xff) as u8;
                store_meta(fd, &m);
                0
            },
            _ => ENOPROTOOPT,
        },
        _ => ENOPROTOOPT,
    }
}

/// SYS_GETSOCKOPT: `fd,level,optname,*mut optval,*mut u32 optlen -> 0 | -errno`.
pub unsafe fn sys_getsockopt(fd: u64, level: u64, optname: u64, optval: u64, optlen: u64) -> u64 {
    let m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let write_i32 = |val: i32| -> u64 {
        if optlen == 0 || !validate_user_buf(optlen, 4) {
            return EINVAL;
        }
        let cap = unsafe { *(optlen as *const u32) } as usize;
        if cap < 4 || !validate_user_buf(optval, 4) {
            return EINVAL;
        }
        unsafe {
            *(optval as *mut i32) = val;
            *(optlen as *mut u32) = 4;
        }
        0
    };

    match level {
        SOL_SOCKET => match optname {
            SO_ERROR => {
                // std checks SO_ERROR to complete a non-blocking connect.
                let err = if m.is_stream() {
                    match bridge_tcp_state(m.handle) {
                        ST_ESTABLISHED => 0,
                        ST_CLOSED if m.peer_port != 0 => 111, // ECONNREFUSED (numeric)
                        _ => 0,
                    }
                } else {
                    0
                };
                write_i32(err)
            },
            SO_KEEPALIVE => write_i32(0),
            SO_REUSEADDR | SO_REUSEPORT | SO_BROADCAST => write_i32(0),
            SO_RCVBUF | SO_SNDBUF => write_i32(64 * 1024),
            SO_RCVTIMEO | SO_SNDTIMEO => {
                let ms = if optname == SO_RCVTIMEO {
                    m.rcvtimeo_ms
                } else {
                    m.sndtimeo_ms
                };
                if optlen == 0 || !validate_user_buf(optlen, 4) {
                    return EINVAL;
                }
                let cap = *(optlen as *const u32) as usize;
                let want = core::mem::size_of::<KTimeval>();
                if cap < want || !validate_user_buf(optval, want as u64) {
                    return EINVAL;
                }
                let tv = KTimeval {
                    tv_sec: (ms / 1000) as i64,
                    tv_usec: ((ms % 1000) * 1000) as i64,
                };
                *(optval as *mut KTimeval) = tv;
                *(optlen as *mut u32) = want as u32;
                0
            },
            _ => ENOPROTOOPT,
        },
        IPPROTO_TCP => match optname {
            TCP_NODELAY => write_i32(0),
            _ => ENOPROTOOPT,
        },
        IPPROTO_IP => match optname {
            IP_TTL => write_i32(m.ttl as i32),
            _ => ENOPROTOOPT,
        },
        _ => ENOPROTOOPT,
    }
}

/// SYS_SHUTDOWN: `fd,how -> 0 | -errno`.
pub unsafe fn sys_shutdown(fd: u64, how: u64) -> u64 {
    let mut m = match meta_of(fd) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if m.is_stream() && m.sflags & SF_CONNECTED == 0 {
        return ENOTCONN;
    }
    match how {
        SHUT_RD => m.sflags |= SF_SHUT_RD,
        SHUT_WR => {
            m.sflags |= SF_SHUT_WR;
            if m.is_stream() {
                let _ = bridge_tcp_shutdown(m.handle);
            }
        },
        SHUT_RDWR => {
            m.sflags |= SF_SHUT_RD | SF_SHUT_WR;
            if m.is_stream() {
                let _ = bridge_tcp_shutdown(m.handle);
            }
        },
        _ => return EINVAL,
    }
    store_meta(fd, &m);
    let token = readiness::socket_token(m.handle as u64);
    readiness::set_ready(token, EPOLLIN | EPOLLOUT | EPOLLRDHUP);
    0
}

// Glue consumed by the fd dispatch in handler::fs.

/// `read(2)` on a socket fd == `recvfrom` with no source address.
pub unsafe fn socket_read(fd: u64, buf: u64, len: u64) -> u64 {
    sys_recvfrom(fd, buf, len, 0, 0, 0)
}

/// `write(2)` on a socket fd == `sendto` to the connected peer.
pub unsafe fn socket_write(fd: u64, buf: u64, len: u64) -> u64 {
    sys_sendto(fd, buf, len, 0, 0, 0)
}

/// `close(2)` of a socket fd: tear down the backend handle + readiness slot. The
/// caller (`handler::fs::sys_fs_close`) still frees the fd-table slot.
pub unsafe fn socket_close_backend(state: &crate::storage::fs_api::FdState) {
    let mut c = [0u8; 32];
    c.copy_from_slice(&state.cookie[..32]);
    let m = SockMeta::from_cookie(&c);
    let token = readiness::socket_token(m.handle as u64);
    readiness::set_ready(token, EPOLLHUP | EPOLLERR);
    if m.is_stream() {
        bridge_tcp_close(m.handle);
    } else {
        bridge_udp_close(m.handle);
    }
    readiness::unregister(token);
}

/// Recompute every stream socket's level-triggered readiness from the live stack
/// state. Driven by `net_drive` after each poll so `epoll_wait` reflects packet
/// arrival without a prior recv/accept syscall (the smoltcp glue never touches
/// readiness on its own). Only stream sockets are scanned — the TCP/UDP handle
/// spaces overlap, and UDP keeps its existing syscall-side readiness. Only newly
/// set bits wake parked waiters; EPOLLERR/EPOLLHUP are sticky and never cleared
/// here so a recorded connect error / hangup survives the rescan.
pub(crate) unsafe fn refresh_socket_readiness() {
    use crate::storage::fs_api::FD_TABLE_LEN;

    // Snapshot (handle, listening) first so no fd-table borrow is held across the
    // bridge probes and readiness mutations below.
    let mut probes: [(i64, bool); FD_TABLE_LEN] = [(0, false); FD_TABLE_LEN];
    let mut n = 0usize;
    {
        let t = SCHEDULER.current_fd_table_mut();
        for fd in 0..FD_TABLE_LEN {
            if let Some(d) = t.get(fd) {
                if d.is_socket() {
                    let mut c = [0u8; 32];
                    c.copy_from_slice(&d.cookie[..32]);
                    let m = SockMeta::from_cookie(&c);
                    if m.is_stream() {
                        probes[n] = (m.handle, m.sflags & SF_LISTENING != 0);
                        n += 1;
                    }
                }
            }
        }
    }

    // Level bits this scan owns and may clear; ERR/HUP are additive only.
    const MANAGED: u32 = EPOLLIN | EPOLLOUT | EPOLLRDHUP;

    for &(handle, listening) in probes.iter().take(n) {
        let token = readiness::socket_token(handle as u64);
        let state = bridge_tcp_state(handle);
        let mut desired: u32 = 0;
        if listening {
            // The accept bridge mints a connection once the listen socket reaches
            // ESTABLISHED/CLOSE_WAIT — i.e. exactly when `accept()` would succeed.
            if state == ST_ESTABLISHED || state == ST_CLOSE_WAIT {
                desired |= EPOLLIN;
            }
        } else {
            if bridge_tcp_can_recv(handle) == 1 {
                desired |= EPOLLIN;
            }
            if bridge_tcp_can_send(handle) == 1 {
                desired |= EPOLLOUT;
            }
            // Peer FIN: deliver EOF-readable + read-closed.
            if state == ST_CLOSE_WAIT {
                desired |= EPOLLIN | EPOLLRDHUP;
            }
            // Fully torn down / reset.
            if state == ST_CLOSED {
                desired |= EPOLLHUP;
            }
        }

        let current = readiness::ready_mask(token);
        let newly = desired & !current;
        if newly != 0 {
            readiness::set_ready(token, newly);
        }
        let to_clear = current & MANAGED & !desired;
        if to_clear != 0 {
            readiness::clear_ready(token, to_clear);
        }
    }
}

/// True if `start_ms + timeo_ms` has elapsed (timeo==0 means no timeout).
fn timed_out(start_ms: u64, timeo_ms: u32) -> bool {
    timeo_ms != 0 && monotonic_ms().saturating_sub(start_ms) >= timeo_ms as u64
}
