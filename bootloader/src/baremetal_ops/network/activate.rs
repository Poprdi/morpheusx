use morpheus_network::boot::{probe_and_create_driver, ProbeError, ProbeResult};
use morpheus_network::device::UnifiedNetDevice;
use morpheus_network::stack::{NetConfig, NetInterface};

use super::{config, nic, state, tcp, udp_dns};

unsafe fn activate_network_from_userspace() -> i64 {
    morpheus_hwinit::serial::log_info("NET", 940, "userspace activation requested");

    if state::has_driver() {
        morpheus_hwinit::serial::log_info("NET", 941, "already active");
        return 1;
    }

    let Some((dma, tsc_freq)) = state::activation_context() else {
        morpheus_hwinit::serial::log_error("NET", 942, "activation failed: dma unavailable");
        return -1;
    };

    morpheus_hwinit::serial::log_info("NET", 943, "probing NIC via network::probe_and_create_driver");
    let driver = match probe_and_create_driver(dma, tsc_freq) {
        Ok(ProbeResult::VirtIO(v)) => {
            morpheus_hwinit::serial::log_info("NET", 949, "probe selected virtio NIC");
            UnifiedNetDevice::VirtIO(v)
        }
        Ok(ProbeResult::Intel(i)) => {
            morpheus_hwinit::serial::log_info("NET", 951, "probe selected intel NIC");
            UnifiedNetDevice::Intel(i)
        }
        Err(ProbeError::NoDevice) => {
            morpheus_hwinit::serial::log_error("NET", 944, "probe failed: no supported NIC detected");
            nic::log_pci_network_candidates();
            return -2;
        }
        Err(ProbeError::VirtioInitFailed) => {
            morpheus_hwinit::serial::log_error("NET", 950, "virtio init failed");
            return -3;
        }
        Err(ProbeError::IntelInitFailed) => {
            morpheus_hwinit::serial::log_error("NET", 952, "intel init failed");
            return -3;
        }
        Err(ProbeError::DeviceNotResponding) => {
            morpheus_hwinit::serial::log_error("NET", 955, "nic mmio not responding");
            return -4;
        }
        Err(ProbeError::BarMappingFailed) => {
            morpheus_hwinit::serial::log_error("NET", 956, "nic bar mapping failure");
            return -5;
        }
    };

    morpheus_hwinit::serial::log_ok("NET", 945, "driver initialized");

    let stack = NetInterface::new(driver, NetConfig::dhcp());
    state::clear_net_handle_tables();
    state::set_stack(stack);

    morpheus_hwinit::serial::log_info("NET", 946, "registering NIC ops");
    morpheus_hwinit::register_nic(morpheus_hwinit::NicOps {
        tx: Some(nic::user_net_tx),
        rx: Some(nic::user_net_rx),
        link_up: Some(nic::user_net_link_up),
        mac: Some(nic::user_net_mac),
        refill: Some(nic::user_net_refill),
        ctrl: Some(nic::user_net_ctrl),
    });

    morpheus_hwinit::serial::log_info("NET", 957, "registering net stack ops");
    morpheus_hwinit::register_net_stack(morpheus_hwinit::NetStackOps {
        tcp_socket: Some(tcp::net_tcp_socket_impl),
        tcp_connect: Some(tcp::net_tcp_connect_impl),
        tcp_send: Some(tcp::net_tcp_send_impl),
        tcp_recv: Some(tcp::net_tcp_recv_impl),
        tcp_close: Some(tcp::net_tcp_close_impl),
        tcp_state: Some(tcp::net_tcp_state_impl),
        tcp_listen: Some(tcp::net_tcp_listen_impl),
        tcp_accept: Some(tcp::net_tcp_accept_impl),
        tcp_shutdown: Some(tcp::net_tcp_shutdown_impl),
        tcp_nodelay: Some(tcp::net_tcp_nodelay_impl),
        tcp_keepalive: Some(tcp::net_tcp_keepalive_impl),
        udp_socket: Some(udp_dns::net_udp_socket_impl),
        udp_send_to: Some(udp_dns::net_udp_send_to_impl),
        udp_recv_from: Some(udp_dns::net_udp_recv_from_impl),
        udp_close: Some(udp_dns::net_udp_close_impl),
        dns_start: Some(udp_dns::net_dns_start_impl),
        dns_result: Some(udp_dns::net_dns_result_impl),
        dns_set_servers: Some(udp_dns::net_dns_set_servers_impl),
        cfg_get: Some(config::net_cfg_get),
        cfg_dhcp: Some(config::net_cfg_dhcp),
        cfg_static_ip: Some(config::net_cfg_static_ip),
        cfg_hostname: Some(config::net_cfg_hostname),
        poll_drive: Some(config::net_poll_drive),
        poll_stats: Some(config::net_poll_stats),
    });

    let _ = nic::user_net_refill();

    let link_now = nic::user_net_link_up();
    if link_now != 0 {
        morpheus_hwinit::serial::log_ok("NET", 947, "activation complete: link up");
    } else {
        morpheus_hwinit::serial::log_info("NET", 948, "activation complete: link down");
    }

    0
}

pub(super) unsafe fn init_userspace_network_activation(
    dma: morpheus_network::dma::DmaRegion,
    tsc_freq: u64,
) {
    state::set_activation_context(dma, tsc_freq);
    morpheus_hwinit::register_net_activation(activate_network_from_userspace);
}
