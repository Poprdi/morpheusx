use morpheus_network::stack::NetState;

use super::state;

pub(super) unsafe fn net_cfg_get(buf: *mut u8) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };

    let out = &mut *(buf as *mut morpheus_hwinit::NetConfigInfo);
    core::ptr::write_bytes(
        out as *mut _ as *mut u8,
        0,
        core::mem::size_of::<morpheus_hwinit::NetConfigInfo>(),
    );

    out.state = match stack.state() {
        NetState::Unconfigured => 0,
        NetState::DhcpDiscovering => 1,
        NetState::Ready => 2,
        NetState::Error => 3,
    };

    out.flags |= 1;

    if let Some(ip) = stack.ipv4_addr() {
        out.ipv4_addr = u32::from_be_bytes(ip.octets());
        out.prefix_len = 24;
    }
    if let Some(gw) = stack.gateway() {
        out.gateway = u32::from_be_bytes(gw.octets());
        out.flags |= 1 << 1;
    }
    if let Some(dns) = stack.dns() {
        out.dns_primary = u32::from_be_bytes(dns.octets());
        out.flags |= 1 << 2;
    }

    let mac = stack.mac_address();
    out.mac[..6].copy_from_slice(&mac);
    out.mtu = 1500;
    state::write_hostname_to(out);

    0
}

pub(super) unsafe fn net_cfg_dhcp() -> i64 {
    if state::user_net_stack_mut().is_some() {
        0
    } else {
        -1
    }
}

pub(super) unsafe fn net_cfg_static_ip(_ip: u32, _prefix_len: u8, _gateway: u32) -> i64 {
    -1
}

pub(super) unsafe fn net_cfg_hostname(name: *const u8, len: usize) -> i64 {
    state::set_hostname(name, len)
}

pub(super) unsafe fn net_poll_drive(timestamp_ms: u64) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };

    stack.device_mut().refill_rx_queue();
    let activity = stack.poll(timestamp_ms);
    stack.device_mut().collect_tx_completions();
    if activity { 1 } else { 0 }
}

pub(super) unsafe fn net_poll_stats(buf: *mut u8) -> i64 {
    if state::user_net_stack_mut().is_none() {
        return -1;
    }
    let out = &mut *(buf as *mut morpheus_hwinit::NetStats);
    core::ptr::write_bytes(
        out as *mut _ as *mut u8,
        0,
        core::mem::size_of::<morpheus_hwinit::NetStats>(),
    );
    out.tcp_active = state::tcp_active_count();
    0
}
