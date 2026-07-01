//! Canonical network syscall subcommand codes and flag values shared
//! kernel↔userland. Single source of truth — consumers re-export, never
//! re-declare (these all cross the `SYS_NET` / `SYS_NET_CFG` / `SYS_DNS` /
//! `SYS_NET_POLL` boundary).

// SYS_NET subcommands (a1)
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
pub const NET_UDP_SOCKET: u64 = 11;
pub const NET_UDP_SEND_TO: u64 = 12;
pub const NET_UDP_RECV_FROM: u64 = 13;
pub const NET_UDP_CLOSE: u64 = 14;

// SYS_DNS subcommands
pub const DNS_START: u64 = 0;
pub const DNS_RESULT: u64 = 1;
pub const DNS_SET_SERVERS: u64 = 2;
pub const DNS_CANCEL: u64 = 3;

// SYS_NET_CFG subcommands
pub const NET_CFG_GET: u64 = 0;
pub const NET_CFG_DHCP: u64 = 1;
pub const NET_CFG_STATIC: u64 = 2;
pub const NET_CFG_HOSTNAME: u64 = 3;
pub const NET_CFG_ACTIVATE: u64 = 4;

// SYS_NET_POLL subcommands
pub const NET_POLL_DRIVE: u64 = 0;
pub const NET_POLL_STATS: u64 = 1;

// NIC hardware control (SYS_NET_CFG with subcmd = NIC_CTRL_BASE + cmd)
/// NIC control commands route through `SYS_NET_CFG` with `subcmd = 128 + cmd`.
pub const NIC_CTRL_BASE: u64 = 128;
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

// NIC capability bits (returned by NIC_CTRL_CAPS)
pub const NIC_CAP_PROMISC: u64 = 1 << 0;
pub const NIC_CAP_MAC_SET: u64 = 1 << 1;
pub const NIC_CAP_MULTICAST: u64 = 1 << 2;
pub const NIC_CAP_VLAN: u64 = 1 << 3;
pub const NIC_CAP_TX_CSUM: u64 = 1 << 4;
pub const NIC_CAP_RX_CSUM: u64 = 1 << 5;
pub const NIC_CAP_TSO: u64 = 1 << 6;
pub const NIC_CAP_IRQ_COALESCE: u64 = 1 << 7;

// Net config state + flags (NetConfigInfo.state / .flags)
pub const NET_STATE_UNCONFIGURED: u32 = 0;
pub const NET_STATE_DHCP_DISCOVERING: u32 = 1;
pub const NET_STATE_READY: u32 = 2;
pub const NET_STATE_ERROR: u32 = 3;

pub const NET_FLAG_DHCP: u32 = 1 << 0;
pub const NET_FLAG_HAS_GATEWAY: u32 = 1 << 1;
pub const NET_FLAG_HAS_DNS: u32 = 1 << 2;

// BSD-socket ABI (SYS_SOCKET..SYS_SHUTDOWN, 109-120). Sockets are real unified
// fds; addresses cross as the tagged `SockAddrStorage`. Ports/addrs are network
// byte order. Append-only Linux-numeric namespaces.

/// Address families.
pub const AF_UNSPEC: u64 = 0;
pub const AF_INET: u64 = 2;
pub const AF_INET6: u64 = 10;

/// Socket types; `SOCK_NONBLOCK`/`SOCK_CLOEXEC` are OR-able into the `type` arg.
pub const SOCK_STREAM: u64 = 1;
pub const SOCK_DGRAM: u64 = 2;
pub const SOCK_NONBLOCK: u64 = 0x800;
pub const SOCK_CLOEXEC: u64 = 0x80000;

/// Protocols (also `setsockopt`/`getsockopt` levels for IP/TCP/IPV6).
pub const IPPROTO_IP: u64 = 0;
pub const IPPROTO_TCP: u64 = 6;
pub const IPPROTO_UDP: u64 = 17;
pub const IPPROTO_IPV6: u64 = 41;

/// `setsockopt`/`getsockopt` level for socket-level options.
pub const SOL_SOCKET: u64 = 1;

/// `SOL_SOCKET` option names. `SO_LINGER` takes `Linger`; `SO_RCVTIMEO`/
/// `SO_SNDTIMEO` take `KTimeval` (microseconds, NOT `Timespec`).
pub const SO_REUSEADDR: u64 = 2;
pub const SO_ERROR: u64 = 4;
pub const SO_BROADCAST: u64 = 6;
pub const SO_SNDBUF: u64 = 7;
pub const SO_RCVBUF: u64 = 8;
pub const SO_KEEPALIVE: u64 = 9;
pub const SO_LINGER: u64 = 13;
pub const SO_REUSEPORT: u64 = 15;
pub const SO_RCVTIMEO: u64 = 20;
pub const SO_SNDTIMEO: u64 = 21;

/// `IPPROTO_TCP`-level option names.
pub const TCP_NODELAY: u64 = 1;
pub const TCP_KEEPIDLE: u64 = 4;
pub const TCP_KEEPINTVL: u64 = 5;
pub const TCP_KEEPCNT: u64 = 6;

/// `IPPROTO_IP`-level option names. `IP_ADD/DROP_MEMBERSHIP` take `IpMreq`.
pub const IP_TTL: u64 = 2;
pub const IP_MULTICAST_TTL: u64 = 33;
pub const IP_MULTICAST_LOOP: u64 = 34;
pub const IP_ADD_MEMBERSHIP: u64 = 35;
pub const IP_DROP_MEMBERSHIP: u64 = 36;

/// `IPPROTO_IPV6`-level option names. `IPV6_ADD/DROP_MEMBERSHIP` take `Ipv6Mreq`.
pub const IPV6_MULTICAST_LOOP: u64 = 19;
pub const IPV6_ADD_MEMBERSHIP: u64 = 20;
pub const IPV6_DROP_MEMBERSHIP: u64 = 21;
pub const IPV6_V6ONLY: u64 = 26;

/// `shutdown` how.
pub const SHUT_RD: u64 = 0;
pub const SHUT_WR: u64 = 1;
pub const SHUT_RDWR: u64 = 2;

/// `sendto`/`recvfrom` flags.
pub const MSG_PEEK: u64 = 0x2;
pub const MSG_DONTWAIT: u64 = 0x40;
pub const MSG_NOSIGNAL: u64 = 0x4000;
