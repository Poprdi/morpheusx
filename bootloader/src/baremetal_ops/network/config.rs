use morpheus_net_stack::stack::NetState;

use super::state;

pub(super) unsafe fn net_cfg_get(buf: *mut u8) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };

    let out = &mut *(buf as *mut morpheus_kernel::syscall::handler::NetConfigInfo);
    core::ptr::write_bytes(
        out as *mut _ as *mut u8,
        0,
        core::mem::size_of::<morpheus_kernel::syscall::handler::NetConfigInfo>(),
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
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };

    match stack.restart_dhcp() {
        Ok(()) => 0,
        Err(_) => -1,
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
    state::reap_expired_dns_queries(stack, timestamp_ms);
    if activity {
        1
    } else {
        0
    }
}

/// Milliseconds until smoltcp needs us next, or -1 if no stack / no deadline.
pub(super) unsafe fn net_poll_at(timestamp_ms: u64) -> i64 {
    match state::user_net_stack_mut() {
        Some(stack) => match stack.poll_delay_ms(timestamp_ms) {
            Some(ms) => ms.min(i64::MAX as u64) as i64,
            None => -1,
        },
        None => -1,
    }
}

pub(super) unsafe fn net_poll_stats(buf: *mut u8) -> i64 {
    if state::user_net_stack_mut().is_none() {
        return -1;
    }
    let out = &mut *(buf as *mut morpheus_kernel::syscall::handler::NetStats);
    core::ptr::write_bytes(
        out as *mut _ as *mut u8,
        0,
        core::mem::size_of::<morpheus_kernel::syscall::handler::NetStats>(),
    );
    out.tx_packets = morpheus_net_stack::stack::tx_packet_count() as u64;
    out.rx_packets = morpheus_net_stack::stack::rx_packet_count() as u64;
    out.tcp_active = state::tcp_active_count();
    0
}
