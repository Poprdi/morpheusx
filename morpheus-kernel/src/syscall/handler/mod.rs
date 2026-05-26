//! Syscall handlers routed by `syscall::syscall_dispatch`.
//! Shared helpers + statics live in `common`/`fb`/`nic_fb`.

pub mod common;

pub mod compositor;
pub mod core;
pub mod fb;
pub mod fd;
pub mod fs;
pub mod hw;
pub mod ipc;
pub mod mem;
pub mod net;
pub mod nic_fb;
pub mod nic_io;
pub mod persist;
pub mod proc;
pub mod sync;
pub mod sysinfo;

// Registration helpers + structs wired up from the boot path.
pub use fb::shutdown_release_display_ownership;
pub use net::{
    register_net_activation, register_net_stack, NetConfigInfo, NetStackOps, NetStats, DNS_RESULT,
    DNS_SET_SERVERS, DNS_START, NET_CFG_DHCP, NET_CFG_GET, NET_CFG_HOSTNAME, NET_CFG_STATIC,
    NET_POLL_DRIVE, NET_POLL_STATS, NET_TCP_ACCEPT, NET_TCP_CLOSE, NET_TCP_CONNECT,
    NET_TCP_KEEPALIVE, NET_TCP_LISTEN, NET_TCP_NODELAY, NET_TCP_RECV, NET_TCP_SEND,
    NET_TCP_SHUTDOWN, NET_TCP_SOCKET, NET_TCP_STATE, NET_UDP_CLOSE, NET_UDP_RECV_FROM,
    NET_UDP_SEND_TO, NET_UDP_SOCKET,
};
pub use nic_fb::{
    fb_mark_dirty, register_framebuffer, register_nic, FbInfo, NicHwStats, NicOps, NIC_CAP_IRQ_COALESCE,
    NIC_CAP_MAC_SET, NIC_CAP_MULTICAST, NIC_CAP_PROMISC, NIC_CAP_RX_CSUM, NIC_CAP_TSO,
    NIC_CAP_TX_CSUM, NIC_CAP_VLAN, NIC_CTRL_CAPS, NIC_CTRL_IRQ_COALESCE, NIC_CTRL_MAC_SET,
    NIC_CTRL_MTU, NIC_CTRL_MULTICAST, NIC_CTRL_PROMISC, NIC_CTRL_RX_CSUM, NIC_CTRL_RX_RING_SIZE,
    NIC_CTRL_STATS, NIC_CTRL_STATS_RESET, NIC_CTRL_TSO, NIC_CTRL_TX_CSUM, NIC_CTRL_TX_RING_SIZE,
    NIC_CTRL_VLAN,
};
pub use ipc::{PROT_EXEC, PROT_READ, PROT_WRITE};
