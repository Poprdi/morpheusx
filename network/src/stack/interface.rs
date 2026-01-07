//! Full smoltcp network interface with TCP/IP stack.
//!
//! This provides a complete IP stack over any `NetworkDevice`:
//! - Ethernet frame handling
//! - ARP resolution
//! - IPv4 with DHCP or static configuration
//! - TCP socket management
//! - DNS resolution (optional)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    NetInterface                             │
//! │  (manages smoltcp Interface + socket set)                   │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              smoltcp::iface::Interface                      │
//! │  (IP routing, ARP, fragmentation)                           │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              DeviceAdapter<D: NetworkDevice>                │
//! │  (bridges our drivers to smoltcp Device trait)              │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              NetworkDevice implementations                  │
//! │  VirtIO | Intel | Realtek | Broadcom | ...                  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::stack::{NetInterface, NetConfig};
//! use morpheus_network::device::virtio::VirtioNetDevice;
//!
//! // Create network device
//! let device = VirtioNetDevice::new(transport)?;
//!
//! // Create interface with DHCP
//! let mut iface = NetInterface::new(device, NetConfig::dhcp());
//!
//! // Poll until we have an IP
//! while !iface.has_ip() {
//!     iface.poll(get_time_ms());
//! }
//!
//! // Now ready for TCP connections
//! let socket = iface.tcp_connect(remote_ip, remote_port)?;
//! ```

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddrV4};

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer, State as TcpState};
use smoltcp::socket::dhcpv4::{Socket as DhcpSocket, Event as DhcpEvent};
use smoltcp::socket::dns::{Socket as DnsSocket, GetQueryResultError};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address, Ipv4Cidr};

use crate::device::NetworkDevice;
use crate::error::{NetworkError, Result};
use super::DeviceAdapter;

/// Network interface configuration.
#[derive(Debug, Clone)]
pub enum NetConfig {
    /// Use DHCP to obtain IP address.
    Dhcp,
    /// Static IP configuration.
    Static {
        ip: Ipv4Addr,
        prefix_len: u8,
        gateway: Option<Ipv4Addr>,
        dns: Option<Ipv4Addr>,
    },
}

impl NetConfig {
    /// Create DHCP configuration.
    pub fn dhcp() -> Self {
        Self::Dhcp
    }

    /// Create static configuration.
    pub fn static_ip(ip: Ipv4Addr, prefix_len: u8, gateway: Option<Ipv4Addr>) -> Self {
        Self::Static {
            ip,
            prefix_len,
            gateway,
            dns: None,
        }
    }

    /// Create static configuration with DNS.
    pub fn static_with_dns(
        ip: Ipv4Addr,
        prefix_len: u8,
        gateway: Option<Ipv4Addr>,
        dns: Ipv4Addr,
    ) -> Self {
        Self::Static {
            ip,
            prefix_len,
            gateway,
            dns: Some(dns),
        }
    }
}

/// Network interface state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetState {
    /// Interface created, not configured.
    Unconfigured,
    /// DHCP discovery in progress.
    DhcpDiscovering,
    /// IP address configured, ready for connections.
    Ready,
    /// Interface error.
    Error,
}

/// Maximum number of concurrent TCP sockets.
pub const MAX_TCP_SOCKETS: usize = 4;

/// TCP receive buffer size.
pub const TCP_RX_BUFFER_SIZE: usize = 65535;

/// TCP transmit buffer size.
pub const TCP_TX_BUFFER_SIZE: usize = 65535;

/// Full network interface with IP stack.
///
/// Wraps a `NetworkDevice` with complete smoltcp integration.
pub struct NetInterface<D: NetworkDevice> {
    /// The underlying device adapter.
    device: DeviceAdapter<D>,
    /// smoltcp interface.
    iface: Interface,
    /// Socket set.
    sockets: SocketSet<'static>,
    /// DHCP socket handle (if using DHCP).
    dhcp_handle: Option<SocketHandle>,
    /// DNS socket handle.
    dns_handle: SocketHandle,
    /// Current state.
    state: NetState,
    /// Configured gateway.
    gateway: Option<Ipv4Address>,
    /// Configured DNS server.
    dns: Option<Ipv4Address>,
    /// Last poll timestamp (milliseconds).
    last_poll_ms: u64,
}

impl<D: NetworkDevice> NetInterface<D> {
    /// Create a new network interface.
    ///
    /// # Arguments
    ///
    /// * `device` - The network device to use
    /// * `config` - IP configuration (DHCP or static)
    pub fn new(device: D, config: NetConfig) -> Self {
        super::set_debug_stage(10); // Stage 10: entered NetInterface::new
        
        let mac = device.mac_address();
        let ethernet_addr = EthernetAddress(mac);
        super::set_debug_stage(11); // Stage 11: got MAC

        let mut device_adapter = DeviceAdapter::new(device);
        super::set_debug_stage(12); // Stage 12: created DeviceAdapter

        // Create smoltcp config
        let smoltcp_config = Config::new(ethernet_addr.into());
        super::set_debug_stage(13); // Stage 13: created Config

        // Create interface
        super::set_debug_stage(14); // Stage 14: about to create Interface
        let mut iface = Interface::new(smoltcp_config, &mut device_adapter, Instant::from_millis(0));
        super::set_debug_stage(15); // Stage 15: Interface created

        // Create socket storage
        let mut sockets = SocketSet::new(vec![]);
        super::set_debug_stage(16); // Stage 16: SocketSet created

        // Default DNS servers (Cloudflare and Google)
        let default_dns_servers: &[IpAddress] = &[
            IpAddress::v4(1, 1, 1, 1),   // Cloudflare
            IpAddress::v4(8, 8, 8, 8),   // Google
        ];
        
        // Create DNS socket with default servers
        super::set_debug_stage(17); // Stage 17: about to create DNS socket
        let dns_queries: [Option<smoltcp::socket::dns::DnsQuery>; 1] = [None];
        let dns_socket = DnsSocket::new(default_dns_servers, dns_queries);
        let dns_handle = sockets.add(dns_socket);
        super::set_debug_stage(18); // Stage 18: DNS socket added

        let (state, dhcp_handle, gateway, dns) = match config {
            NetConfig::Dhcp => {
                super::set_debug_stage(19); // Stage 19: creating DHCP socket
                // Add DHCP socket
                let dhcp_socket = DhcpSocket::new();
                let handle = sockets.add(dhcp_socket);
                super::set_debug_stage(20); // Stage 20: DHCP socket added
                (NetState::DhcpDiscovering, Some(handle), None, None)
            }
            NetConfig::Static {
                ip,
                prefix_len,
                gateway,
                dns,
            } => {
                // Configure static IP
                let ip_addr = Ipv4Address::from_bytes(&ip.octets());
                let cidr = Ipv4Cidr::new(ip_addr, prefix_len);
                iface.update_ip_addrs(|addrs| {
                    addrs.push(IpCidr::Ipv4(cidr)).ok();
                });

                // Set gateway
                let gw = gateway.map(|g| Ipv4Address::from_bytes(&g.octets()));
                if let Some(gw_addr) = gw {
                    iface.routes_mut().add_default_ipv4_route(gw_addr).ok();
                }

                let dns_addr = dns.map(|d| Ipv4Address::from_bytes(&d.octets()));

                (NetState::Ready, None, gw, dns_addr)
            }
        };

        super::set_debug_stage(25); // Stage 25: about to return Self
        Self {
            device: device_adapter,
            iface,
            sockets,
            dhcp_handle,
            dns_handle,
            state,
            gateway,
            dns,
            last_poll_ms: 0,
        }
    }

    /// Get current state.
    pub fn state(&self) -> NetState {
        self.state
    }

    /// Check if interface has an IP address configured.
    pub fn has_ip(&self) -> bool {
        self.state == NetState::Ready
    }

    /// Get the current IPv4 address (if configured).
    pub fn ipv4_addr(&self) -> Option<Ipv4Addr> {
        for cidr in self.iface.ip_addrs() {
            let IpCidr::Ipv4(v4) = cidr;
            let addr = v4.address();
            let bytes = addr.as_bytes();
            return Some(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]));
        }
        None
    }

    /// Get the gateway address.
    pub fn gateway(&self) -> Option<Ipv4Addr> {
        self.gateway.map(|g| {
            let bytes = g.as_bytes();
            Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
        })
    }

    /// Get the DNS server address.
    pub fn dns(&self) -> Option<Ipv4Addr> {
        self.dns.map(|d| {
            let bytes = d.as_bytes();
            Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
        })
    }

    /// Start a DNS query for a hostname. Returns a query handle.
    pub fn start_dns_query(&mut self, hostname: &str) -> Result<smoltcp::socket::dns::QueryHandle> {
        super::debug_log(80, \"start_dns_query\");\n        let dns_socket = self.sockets.get_mut::<DnsSocket>(self.dns_handle);
        dns_socket
            .start_query(self.iface.context(), hostname, smoltcp::wire::DnsQueryType::A)
            .map_err(|_| {\n                super::debug_log(81, \"DNS query start err\");\n                NetworkError::DnsResolutionFailed\n            })
    }
    
    /// Check DNS query result. Returns Ok(Some(ip)) if resolved, Ok(None) if pending, Err if failed.
    pub fn get_dns_result(&mut self, handle: smoltcp::socket::dns::QueryHandle) -> Result<Option<Ipv4Addr>> {
        let dns_socket = self.sockets.get_mut::<DnsSocket>(self.dns_handle);
        match dns_socket.get_query_result(handle) {
            Ok(addrs) => {
                super::debug_log(82, \"DNS got result\");\n                // Find first IPv4 address
                for addr in addrs {
                    if let IpAddress::Ipv4(v4) = addr {
                        let bytes = v4.as_bytes();
                        return Ok(Some(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])));
                    }
                }
                super::debug_log(83, \"DNS no IPv4 addr\");\n                Err(NetworkError::DnsResolutionFailed)
            }
            Err(GetQueryResultError::Pending) => Ok(None),
            Err(GetQueryResultError::Failed) => {\n                super::debug_log(84, \"DNS query failed\");\n                Err(NetworkError::DnsResolutionFailed)\n            }
        }
    }

    /// Get the MAC address.
    pub fn mac_address(&self) -> [u8; 6] {
        self.device.inner.mac_address()
    }

    /// Poll the interface - must be called regularly.
    ///
    /// Returns `true` if any socket activity occurred.
    pub fn poll(&mut self, timestamp_ms: u64) -> bool {
        self.last_poll_ms = timestamp_ms;
        let timestamp = Instant::from_millis(timestamp_ms as i64);

        // Poll the interface
        let activity = self.iface.poll(timestamp, &mut self.device, &mut self.sockets);

        // Handle DHCP if active
        if let Some(dhcp_handle) = self.dhcp_handle {
            let event = self.sockets.get_mut::<DhcpSocket>(dhcp_handle).poll();
            match event {
                Some(DhcpEvent::Configured(config)) => {
                    super::debug_log(30, "DHCP configured!");
                    
                    // Copy config data we need before releasing the borrow
                    let address = config.address;
                    let router = config.router;
                    let dns_servers: Vec<Ipv4Address> = config.dns_servers.iter().copied().collect();
                    drop(config); // Explicitly release the borrow
                    
                    // Apply DHCP configuration
                    self.iface.update_ip_addrs(|addrs| {
                        addrs.clear();
                        addrs.push(IpCidr::Ipv4(address)).ok();
                    });

                    if let Some(router) = router {
                        self.iface.routes_mut().add_default_ipv4_route(router).ok();
                        self.gateway = Some(router);
                    }

                    // Update DNS servers: DHCP-provided first, then real-world fallbacks
                    // This ensures QEMU/virtual DNS works while keeping real DNS for hardware
                    let mut dns_addrs: Vec<IpAddress> = dns_servers
                        .iter()
                        .map(|a| IpAddress::Ipv4(*a))
                        .collect();
                    
                    // Add real-world DNS servers as fallbacks (for real hardware)
                    // Only add if not already present from DHCP
                    let cloudflare = IpAddress::v4(1, 1, 1, 1);
                    let google = IpAddress::v4(8, 8, 8, 8);
                    if !dns_addrs.contains(&cloudflare) {
                        dns_addrs.push(cloudflare);
                    }
                    if !dns_addrs.contains(&google) {
                        dns_addrs.push(google);
                    }
                    
                    // Update DNS socket with combined servers
                    let dns_socket = self.sockets.get_mut::<DnsSocket>(self.dns_handle);
                    dns_socket.update_servers(&dns_addrs);
                    
                    if !dns_servers.is_empty() {
                        self.dns = Some(dns_servers[0]);
                    }

                    self.state = NetState::Ready;
                    super::debug_log(31, "DHCP state -> Ready");
                }
                Some(DhcpEvent::Deconfigured) => {
                    super::debug_log(32, "DHCP deconfigured");
                    self.iface.update_ip_addrs(|addrs| addrs.clear());
                    self.iface.routes_mut().remove_default_ipv4_route();
                    self.gateway = None;
                    self.dns = None;
                    self.state = NetState::DhcpDiscovering;
                }
                None => {}
            }
        }

        activity
    }

    /// Create a TCP socket and return its handle.
    pub fn tcp_socket(&mut self) -> Result<SocketHandle> {
        super::debug_log(90, \"tcp_socket create\");
        let rx_buffer = TcpSocketBuffer::new(vec![0u8; TCP_RX_BUFFER_SIZE]);
        let tx_buffer = TcpSocketBuffer::new(vec![0u8; TCP_TX_BUFFER_SIZE]);
        let socket = TcpSocket::new(rx_buffer, tx_buffer);
        let handle = self.sockets.add(socket);
        Ok(handle)
    }

    /// Connect a TCP socket to a remote endpoint.
    pub fn tcp_connect(
        &mut self,
        handle: SocketHandle,
        remote_ip: Ipv4Addr,
        remote_port: u16,
    ) -> Result<()> {
        super::debug_log(91, \"tcp_connect start\");
        let remote_addr = Ipv4Address::from_bytes(&remote_ip.octets());
        let endpoint = IpEndpoint::new(IpAddress::Ipv4(remote_addr), remote_port);

        // Get ephemeral local port first (before borrowing sockets mutably)
        let local_port = self.ephemeral_port();

        let socket = self.sockets.get_mut::<TcpSocket>(handle);

        socket
            .connect(self.iface.context(), endpoint, local_port)
            .map_err(|_| {
                super::debug_log(92, \"tcp_connect FAILED\");
                NetworkError::ConnectionFailed
            })?;

        super::debug_log(93, \"tcp_connect initiated\");
        Ok(())
    }

    /// Check if a TCP socket is connected.
    pub fn tcp_is_connected(&self, handle: SocketHandle) -> bool {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.state() == TcpState::Established
    }

    /// Check if a TCP socket can send data.
    pub fn tcp_can_send(&self, handle: SocketHandle) -> bool {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.can_send()
    }

    /// Check if a TCP socket can receive data.
    pub fn tcp_can_recv(&self, handle: SocketHandle) -> bool {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.can_recv()
    }

    /// Send data on a TCP socket.
    pub fn tcp_send(&mut self, handle: SocketHandle, data: &[u8]) -> Result<usize> {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket
            .send_slice(data)
            .map_err(|_| NetworkError::SendFailed)
    }

    /// Receive data from a TCP socket.
    pub fn tcp_recv(&mut self, handle: SocketHandle, buffer: &mut [u8]) -> Result<usize> {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket
            .recv_slice(buffer)
            .map_err(|_| NetworkError::ReceiveFailed)
    }

    /// Close a TCP socket.
    pub fn tcp_close(&mut self, handle: SocketHandle) {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket.close();
    }

    /// Remove a socket from the set.
    pub fn remove_socket(&mut self, handle: SocketHandle) {
        self.sockets.remove(handle);
    }

    /// Get TCP socket state.
    pub fn tcp_state(&self, handle: SocketHandle) -> TcpState {
        self.sockets.get::<TcpSocket>(handle).state()
    }

    /// Generate an ephemeral port number.
    fn ephemeral_port(&self) -> u16 {
        // Simple ephemeral port allocation based on timestamp
        // In a real implementation, track used ports
        let base = 49152u16;
        let range = 16383u16;
        let offset = (self.last_poll_ms % range as u64) as u16;
        base + offset
    }

    /// Get reference to the underlying device.
    pub fn device(&self) -> &D {
        &self.device.inner
    }

    /// Get mutable reference to the underlying device.
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device.inner
    }
}
