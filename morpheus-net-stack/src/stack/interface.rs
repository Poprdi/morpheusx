//! Full smoltcp IP stack over any `NetworkDevice`: ARP, IPv4 (DHCP or static),
//! TCP/UDP sockets, and DNS.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddrV4};

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::dhcpv4::{Event as DhcpEvent, Socket as DhcpSocket};
use smoltcp::socket::dns::{GetQueryResultError, Socket as DnsSocket};
use smoltcp::socket::tcp::{
    Socket as TcpSocket, SocketBuffer as TcpSocketBuffer, State as TcpState,
};
use smoltcp::socket::udp::{
    PacketBuffer as UdpPacketBuffer, PacketMetadata as UdpPacketMetadata, Socket as UdpSocket,
};
use smoltcp::time::Duration;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address, Ipv4Cidr};

use super::DeviceAdapter;
use crate::error::{NetworkError, Result};
use morpheus_nic::device::NetworkDevice;

#[derive(Debug, Clone)]
pub enum NetConfig {
    Dhcp,
    Static {
        ip: Ipv4Addr,
        prefix_len: u8,
        gateway: Option<Ipv4Addr>,
        dns: Option<Ipv4Addr>,
    },
}

impl NetConfig {
    pub fn dhcp() -> Self {
        Self::Dhcp
    }

    pub fn static_ip(ip: Ipv4Addr, prefix_len: u8, gateway: Option<Ipv4Addr>) -> Self {
        Self::Static {
            ip,
            prefix_len,
            gateway,
            dns: None,
        }
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetState {
    Unconfigured,
    DhcpDiscovering,
    Ready,
    Error,
}

pub const MAX_TCP_SOCKETS: usize = 4;

pub const TCP_RX_BUFFER_SIZE: usize = 65535;

pub const TCP_TX_BUFFER_SIZE: usize = 65535;

pub const UDP_PACKET_META_COUNT: usize = 8;

/// UDP payload storage per direction.
pub const UDP_PACKET_DATA_BYTES: usize = 8192;

pub struct NetInterface<D: NetworkDevice> {
    device: DeviceAdapter<D>,
    iface: Interface,
    sockets: SocketSet<'static>,
    dhcp_handle: Option<SocketHandle>,
    dns_handle: SocketHandle,
    state: NetState,
    gateway: Option<Ipv4Address>,
    dns: Option<Ipv4Address>,
    last_poll_ms: u64,
}

impl<D: NetworkDevice> NetInterface<D> {
    pub fn new(device: D, config: NetConfig) -> Self {
        super::set_debug_stage(10);
        super::debug_log(10, "NetInterface::new() entered");

        let mac = device.mac_address();
        let ethernet_addr = EthernetAddress(mac);
        super::set_debug_stage(11);
        super::debug_log(11, "Got MAC address");

        let mut device_adapter = DeviceAdapter::new(device);
        super::set_debug_stage(12);
        super::debug_log(12, "Created DeviceAdapter");

        let smoltcp_config = Config::new(ethernet_addr.into());
        super::set_debug_stage(13);
        super::debug_log(13, "Created smoltcp Config");

        super::set_debug_stage(14);
        super::debug_log(14, "Creating smoltcp Interface...");
        let mut iface =
            Interface::new(smoltcp_config, &mut device_adapter, Instant::from_millis(0));
        super::set_debug_stage(15);
        super::debug_log(15, "smoltcp Interface created");

        super::debug_log(15, "Creating SocketSet...");
        let mut sockets = SocketSet::new(vec![]);
        super::set_debug_stage(16);
        super::debug_log(16, "SocketSet created");

        // One entry to match small DNS_MAX_SERVER_COUNT builds.
        let default_dns_servers: &[IpAddress] = &[IpAddress::v4(1, 1, 1, 1)];

        super::set_debug_stage(17);
        super::debug_log(17, "Creating DNS socket...");
        let dns_queries: [Option<smoltcp::socket::dns::DnsQuery>; 1] = [None];
        let dns_socket = DnsSocket::new(default_dns_servers, dns_queries);
        let dns_handle = sockets.add(dns_socket);
        super::set_debug_stage(18);
        super::debug_log(18, "DNS socket added");

        let (state, dhcp_handle, gateway, dns) = match config {
            NetConfig::Dhcp => {
                super::set_debug_stage(19);
                super::debug_log(19, "Creating DHCP socket...");
                let dhcp_socket = DhcpSocket::new();
                let handle = sockets.add(dhcp_socket);
                super::set_debug_stage(20);
                super::debug_log(20, "DHCP socket added");
                (NetState::DhcpDiscovering, Some(handle), None, None)
            },
            NetConfig::Static {
                ip,
                prefix_len,
                gateway,
                dns,
            } => {
                let ip_addr = Ipv4Address::from_bytes(&ip.octets());
                let cidr = Ipv4Cidr::new(ip_addr, prefix_len);
                iface.update_ip_addrs(|addrs| {
                    addrs.push(IpCidr::Ipv4(cidr)).ok();
                });

                let gw = gateway.map(|g| Ipv4Address::from_bytes(&g.octets()));
                if let Some(gw_addr) = gw {
                    iface.routes_mut().add_default_ipv4_route(gw_addr).ok();
                }

                let dns_addr = dns.map(|d| Ipv4Address::from_bytes(&d.octets()));

                (NetState::Ready, None, gw, dns_addr)
            },
        };

        super::set_debug_stage(25);
        super::debug_log(25, "NetInterface::new() complete");
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

    pub fn state(&self) -> NetState {
        self.state
    }

    pub fn has_ip(&self) -> bool {
        self.state == NetState::Ready
    }

    pub fn ipv4_addr(&self) -> Option<Ipv4Addr> {
        for cidr in self.iface.ip_addrs() {
            let IpCidr::Ipv4(v4) = cidr;
            let addr = v4.address();
            let bytes = addr.as_bytes();
            return Some(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]));
        }
        None
    }

    pub fn gateway(&self) -> Option<Ipv4Addr> {
        self.gateway.map(|g| {
            let bytes = g.as_bytes();
            Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
        })
    }

    pub fn dns(&self) -> Option<Ipv4Addr> {
        self.dns.map(|d| {
            let bytes = d.as_bytes();
            Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
        })
    }

    pub fn restart_dhcp(&mut self) -> Result<()> {
        let Some(dhcp_handle) = self.dhcp_handle else {
            return Err(NetworkError::ProtocolNotAvailable);
        };

        let dhcp = self.sockets.get_mut::<DhcpSocket>(dhcp_handle);
        dhcp.reset();

        // Drop current lease immediately so userspace sees discovery state.
        self.iface.update_ip_addrs(|addrs| addrs.clear());
        self.iface.routes_mut().remove_default_ipv4_route();
        self.gateway = None;
        self.dns = None;
        self.state = NetState::DhcpDiscovering;

        Ok(())
    }

    pub fn start_dns_query(&mut self, hostname: &str) -> Result<smoltcp::socket::dns::QueryHandle> {
        super::debug_log(80, "start_dns_query");
        let dns_socket = self.sockets.get_mut::<DnsSocket>(self.dns_handle);
        dns_socket
            .start_query(
                self.iface.context(),
                hostname,
                smoltcp::wire::DnsQueryType::A,
            )
            .map_err(|_| {
                super::debug_log(81, "DNS query start err");
                NetworkError::DnsResolutionFailed
            })
    }

    pub fn set_dns_servers(&mut self, servers: &[Ipv4Addr]) -> Result<()> {
        if servers.is_empty() {
            return Err(NetworkError::DnsResolutionFailed);
        }

        let mut list: Vec<IpAddress> = Vec::with_capacity(servers.len());
        for server in servers {
            list.push(IpAddress::Ipv4(Ipv4Address::from_bytes(&server.octets())));
        }

        let dns_socket = self.sockets.get_mut::<DnsSocket>(self.dns_handle);
        dns_socket.update_servers(&list);
        self.dns = Some(Ipv4Address::from_bytes(&servers[0].octets()));
        Ok(())
    }

    /// Ok(Some) resolved, Ok(None) pending, Err failed.
    pub fn get_dns_result(
        &mut self,
        handle: smoltcp::socket::dns::QueryHandle,
    ) -> Result<Option<Ipv4Addr>> {
        let dns_socket = self.sockets.get_mut::<DnsSocket>(self.dns_handle);
        match dns_socket.get_query_result(handle) {
            Ok(addrs) => {
                super::debug_log(82, "DNS got result");
                for addr in addrs {
                    let IpAddress::Ipv4(v4) = addr;
                    let bytes = v4.as_bytes();
                    return Ok(Some(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])));
                }
                super::debug_log(83, "DNS no IPv4 addr");
                Err(NetworkError::DnsResolutionFailed)
            },
            Err(GetQueryResultError::Pending) => Ok(None),
            Err(GetQueryResultError::Failed) => {
                super::debug_log(84, "DNS query failed");
                Err(NetworkError::DnsResolutionFailed)
            },
        }
    }

    pub fn mac_address(&self) -> [u8; 6] {
        self.device.inner.mac_address()
    }

    /// Poll the stack; returns true if any socket saw activity.
    pub fn poll(&mut self, timestamp_ms: u64) -> bool {
        self.last_poll_ms = timestamp_ms;
        let timestamp = Instant::from_millis(timestamp_ms as i64);

        let activity = self
            .iface
            .poll(timestamp, &mut self.device, &mut self.sockets);

        if let Some(dhcp_handle) = self.dhcp_handle {
            let event = self.sockets.get_mut::<DhcpSocket>(dhcp_handle).poll();
            match event {
                Some(DhcpEvent::Configured(config)) => {
                    super::debug_log(30, "DHCP configured!");

                    // Copy out before dropping the socket borrow.
                    let address = config.address;
                    let router = config.router;
                    let dns_servers: Vec<Ipv4Address> =
                        config.dns_servers.iter().copied().collect();
                    drop(config);

                    self.iface.update_ip_addrs(|addrs| {
                        addrs.clear();
                        addrs.push(IpCidr::Ipv4(address)).ok();
                    });

                    if let Some(router) = router {
                        self.iface.routes_mut().add_default_ipv4_route(router).ok();
                        self.gateway = Some(router);
                    }

                    // Single entry avoids panic when DNS_MAX_SERVER_COUNT == 1.
                    let primary_dns = dns_servers
                        .first()
                        .copied()
                        .unwrap_or(Ipv4Address::new(1, 1, 1, 1));
                    let dns_socket = self.sockets.get_mut::<DnsSocket>(self.dns_handle);
                    dns_socket.update_servers(&[IpAddress::Ipv4(primary_dns)]);
                    self.dns = Some(primary_dns);

                    self.state = NetState::Ready;
                    super::debug_log(31, "DHCP state -> Ready");
                },
                Some(DhcpEvent::Deconfigured) => {
                    super::debug_log(32, "DHCP deconfigured");
                    self.iface.update_ip_addrs(|addrs| addrs.clear());
                    self.iface.routes_mut().remove_default_ipv4_route();
                    self.gateway = None;
                    self.dns = None;
                    self.state = NetState::DhcpDiscovering;
                },
                None => {},
            }
        }

        activity
    }

    pub fn tcp_socket(&mut self) -> Result<SocketHandle> {
        super::debug_log(90, "tcp_socket create");
        let rx_buffer = TcpSocketBuffer::new(vec![0u8; TCP_RX_BUFFER_SIZE]);
        let tx_buffer = TcpSocketBuffer::new(vec![0u8; TCP_TX_BUFFER_SIZE]);
        let socket = TcpSocket::new(rx_buffer, tx_buffer);
        let handle = self.sockets.add(socket);
        Ok(handle)
    }

    pub fn tcp_connect(
        &mut self,
        handle: SocketHandle,
        remote_ip: Ipv4Addr,
        remote_port: u16,
    ) -> Result<()> {
        super::debug_log(91, "tcp_connect start");
        let remote_addr = Ipv4Address::from_bytes(&remote_ip.octets());
        let endpoint = IpEndpoint::new(IpAddress::Ipv4(remote_addr), remote_port);

        // Allocate before borrowing sockets mutably.
        let local_port = self.ephemeral_port();

        let socket = self.sockets.get_mut::<TcpSocket>(handle);

        socket
            .connect(self.iface.context(), endpoint, local_port)
            .map_err(|_| {
                super::debug_log(92, "tcp_connect FAILED");
                NetworkError::ConnectionFailed
            })?;

        super::debug_log(93, "tcp_connect initiated");
        Ok(())
    }

    pub fn tcp_is_connected(&self, handle: SocketHandle) -> bool {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.state() == TcpState::Established
    }

    pub fn tcp_can_send(&self, handle: SocketHandle) -> bool {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.can_send()
    }

    pub fn tcp_can_recv(&self, handle: SocketHandle) -> bool {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.can_recv()
    }

    pub fn tcp_send(&mut self, handle: SocketHandle, data: &[u8]) -> Result<usize> {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket
            .send_slice(data)
            .map_err(|_| NetworkError::SendFailed)
    }

    pub fn tcp_recv(&mut self, handle: SocketHandle, buffer: &mut [u8]) -> Result<usize> {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket
            .recv_slice(buffer)
            .map_err(|_| NetworkError::ReceiveFailed)
    }

    pub fn tcp_close(&mut self, handle: SocketHandle) {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket.close();
    }

    /// Start listening for inbound TCP on `port`.
    pub fn tcp_listen(&mut self, handle: SocketHandle, port: u16) -> Result<()> {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket
            .listen(port)
            .map_err(|_| NetworkError::ConnectionFailed)
    }

    /// Returns true if the socket is currently in listening state.
    pub fn tcp_is_listening(&self, handle: SocketHandle) -> bool {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.is_listening()
    }

    pub fn tcp_local_port(&self, handle: SocketHandle) -> Option<u16> {
        let socket = self.sockets.get::<TcpSocket>(handle);
        socket.local_endpoint().map(|ep| ep.port)
    }

    /// Half-close TCP (write side).
    pub fn tcp_shutdown(&mut self, handle: SocketHandle) -> Result<()> {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket.close();
        Ok(())
    }

    /// Toggle Nagle algorithm. `on = true` means TCP_NODELAY.
    pub fn tcp_set_nodelay(&mut self, handle: SocketHandle, on: bool) {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        socket.set_nagle_enabled(!on);
    }

    /// Set TCP keepalive interval in milliseconds. `0` disables.
    pub fn tcp_set_keepalive(&mut self, handle: SocketHandle, interval_ms: u64) {
        let socket = self.sockets.get_mut::<TcpSocket>(handle);
        if interval_ms == 0 {
            socket.set_keep_alive(None);
        } else {
            socket.set_keep_alive(Some(Duration::from_millis(interval_ms)));
        }
    }

    pub fn udp_socket(&mut self) -> Result<SocketHandle> {
        let rx_meta = vec![UdpPacketMetadata::EMPTY; UDP_PACKET_META_COUNT];
        let tx_meta = vec![UdpPacketMetadata::EMPTY; UDP_PACKET_META_COUNT];
        let rx_data = vec![0u8; UDP_PACKET_DATA_BYTES];
        let tx_data = vec![0u8; UDP_PACKET_DATA_BYTES];

        let socket = UdpSocket::new(
            UdpPacketBuffer::new(rx_meta, rx_data),
            UdpPacketBuffer::new(tx_meta, tx_data),
        );
        Ok(self.sockets.add(socket))
    }

    pub fn udp_bind(&mut self, handle: SocketHandle, port: u16) -> Result<()> {
        let socket = self.sockets.get_mut::<UdpSocket>(handle);
        socket
            .bind(port)
            .map_err(|_| NetworkError::ConnectionFailed)
    }

    pub fn udp_send_to(
        &mut self,
        handle: SocketHandle,
        remote_ip: Ipv4Addr,
        remote_port: u16,
        data: &[u8],
    ) -> Result<usize> {
        let remote = Ipv4Address::from_bytes(&remote_ip.octets());
        let endpoint = IpEndpoint::new(IpAddress::Ipv4(remote), remote_port);
        let local_port = self.ephemeral_port();

        let socket = self.sockets.get_mut::<UdpSocket>(handle);
        if !socket.is_open() {
            socket
                .bind(local_port)
                .map_err(|_| NetworkError::ConnectionFailed)?;
        }

        socket
            .send_slice(data, endpoint)
            .map(|_| data.len())
            .map_err(|_| NetworkError::SendFailed)
    }

    pub fn udp_recv_from(
        &mut self,
        handle: SocketHandle,
        buffer: &mut [u8],
    ) -> Result<(usize, Ipv4Addr, u16)> {
        let socket = self.sockets.get_mut::<UdpSocket>(handle);
        match socket.recv_slice(buffer) {
            Ok((n, meta)) => {
                let IpAddress::Ipv4(v4) = meta.endpoint.addr;
                let octets = v4.as_bytes();
                Ok((
                    n,
                    Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]),
                    meta.endpoint.port,
                ))
            },
            Err(_) => Ok((0, Ipv4Addr::new(0, 0, 0, 0), 0)),
        }
    }

    pub fn remove_socket(&mut self, handle: SocketHandle) {
        self.sockets.remove(handle);
    }

    pub fn tcp_state(&self, handle: SocketHandle) -> TcpState {
        self.sockets.get::<TcpSocket>(handle).state()
    }

    /// Get TCP state as syscall ABI ordinal (0..10).
    pub fn tcp_state_code(&self, handle: SocketHandle) -> u8 {
        match self.tcp_state(handle) {
            TcpState::Closed => 0,
            TcpState::Listen => 1,
            TcpState::SynSent => 2,
            TcpState::SynReceived => 3,
            TcpState::Established => 4,
            TcpState::FinWait1 => 5,
            TcpState::FinWait2 => 6,
            TcpState::CloseWait => 7,
            TcpState::Closing => 8,
            TcpState::LastAck => 9,
            TcpState::TimeWait => 10,
        }
    }

    /// Ephemeral port from the IANA dynamic range (49152..65535), keyed on
    /// last poll time. Does not track in-use ports.
    fn ephemeral_port(&self) -> u16 {
        let base = 49152u16;
        let range = 16383u16;
        let offset = (self.last_poll_ms % range as u64) as u16;
        base + offset
    }

    pub fn device(&self) -> &D {
        &self.device.inner
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device.inner
    }
}
