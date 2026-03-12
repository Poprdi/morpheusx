use core::net::Ipv4Addr;

use super::state;

pub(super) unsafe fn net_udp_socket_impl() -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };

    let Ok(socket) = stack.udp_socket() else {
        return -1;
    };

    if let Some(handle) = state::alloc_udp_slot(socket) {
        handle
    } else {
        stack.remove_socket(socket);
        -1
    }
}

pub(super) unsafe fn net_udp_send_to_impl(
    handle: i64,
    dest_ip: u32,
    dest_port: u16,
    buf: *const u8,
    len: usize,
) -> i64 {
    if len == 0 {
        return 0;
    }
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_udp_slot(handle) else {
        return -1;
    };

    let data = core::slice::from_raw_parts(buf, len);
    match stack.udp_send_to(socket, state::ip_from_nbo(dest_ip), dest_port, data) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

pub(super) unsafe fn net_udp_recv_from_impl(
    handle: i64,
    buf: *mut u8,
    len: usize,
    src_out: *mut u8,
) -> i64 {
    if len == 0 {
        return 0;
    }
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_udp_slot(handle) else {
        return -1;
    };

    let data = core::slice::from_raw_parts_mut(buf, len);
    match stack.udp_recv_from(socket, data) {
        Ok((n, ip, port)) => {
            let ip_nbo = state::ip_to_nbo(ip);
            core::ptr::copy_nonoverlapping((&ip_nbo as *const u32).cast::<u8>(), src_out, 4);
            core::ptr::copy_nonoverlapping((&port as *const u16).cast::<u8>(), src_out.add(4), 2);
            core::ptr::write_bytes(src_out.add(6), 0, 2);
            n as i64
        }
        Err(_) => -1,
    }
}

pub(super) unsafe fn net_udp_close_impl(handle: i64) {
    let Some(stack) = state::user_net_stack_mut() else {
        return;
    };
    let Some(socket) = state::take_udp_slot(handle) else {
        return;
    };

    stack.remove_socket(socket);
}

pub(super) unsafe fn net_dns_start_impl(name: *const u8, len: usize) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    if name.is_null() || len == 0 {
        return -1;
    }

    let name_bytes = core::slice::from_raw_parts(name, len);
    let Ok(hostname) = core::str::from_utf8(name_bytes) else {
        return -1;
    };

    let Ok(query) = stack.start_dns_query(hostname) else {
        return -1;
    };

    state::alloc_dns_query_slot(query).unwrap_or(-1)
}

pub(super) unsafe fn net_dns_result_impl(query: i64, out: *mut u8) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(query_handle) = state::get_dns_query_slot(query) else {
        return -1;
    };

    match stack.get_dns_result(query_handle) {
        Ok(Some(ip)) => {
            let nbo = state::ip_to_nbo(ip);
            core::ptr::copy_nonoverlapping((&nbo as *const u32).cast::<u8>(), out, 4);
            state::clear_dns_query_slot(query);
            0
        }
        Ok(None) => 1,
        Err(_) => {
            state::clear_dns_query_slot(query);
            -1
        }
    }
}

pub(super) unsafe fn net_dns_set_servers_impl(servers: *const u32, count: usize) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    if servers.is_null() || count == 0 {
        return -1;
    }

    let server_nbo = core::slice::from_raw_parts(servers, count);
    let mut server_list = [Ipv4Addr::new(0, 0, 0, 0); 4];
    let mut used = 0usize;
    for ip in server_nbo.iter().take(server_list.len()) {
        server_list[used] = state::ip_from_nbo(*ip);
        used += 1;
    }

    if stack.set_dns_servers(&server_list[..used]).is_ok() {
        0
    } else {
        -1
    }
}
