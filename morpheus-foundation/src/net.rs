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
