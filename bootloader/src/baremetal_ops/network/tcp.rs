use super::state;

pub(super) unsafe fn net_tcp_socket_impl() -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };

    let Ok(socket) = stack.tcp_socket() else {
        return -1;
    };

    if let Some(handle) = state::alloc_tcp_slot(socket) {
        handle
    } else {
        stack.remove_socket(socket);
        -1
    }
}

pub(super) unsafe fn net_tcp_connect_impl(handle: i64, ip: u32, port: u16) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };

    if stack.tcp_connect(socket, state::ip_from_nbo(ip), port).is_ok() {
        0
    } else {
        -1
    }
}

pub(super) unsafe fn net_tcp_send_impl(handle: i64, buf: *const u8, len: usize) -> i64 {
    if len == 0 {
        return 0;
    }
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };

    let data = core::slice::from_raw_parts(buf, len);
    match stack.tcp_send(socket, data) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

pub(super) unsafe fn net_tcp_recv_impl(handle: i64, buf: *mut u8, len: usize) -> i64 {
    if len == 0 {
        return 0;
    }
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };

    let data = core::slice::from_raw_parts_mut(buf, len);
    match stack.tcp_recv(socket, data) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

pub(super) unsafe fn net_tcp_close_impl(handle: i64) {
    let Some(stack) = state::user_net_stack_mut() else {
        return;
    };
    let Some(socket) = state::take_tcp_slot(handle) else {
        return;
    };

    stack.tcp_close(socket);
    stack.remove_socket(socket);
}

pub(super) unsafe fn net_tcp_state_impl(handle: i64) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };
    stack.tcp_state_code(socket) as i64
}

pub(super) unsafe fn net_tcp_listen_impl(handle: i64, port: u16) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };

    if stack.tcp_listen(socket, port).is_ok() {
        0
    } else {
        -1
    }
}

pub(super) unsafe fn net_tcp_accept_impl(listen_handle: i64) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(listen_socket) = state::get_tcp_slot(listen_handle) else {
        return -1;
    };

    let state_code = stack.tcp_state_code(listen_socket);
    if state_code != 4 && state_code != 7 {
        return -1;
    }

    let Some(local_port) = stack.tcp_local_port(listen_socket) else {
        return -1;
    };

    let Ok(new_listen_socket) = stack.tcp_socket() else {
        return -1;
    };
    if stack.tcp_listen(new_listen_socket, local_port).is_err() {
        stack.remove_socket(new_listen_socket);
        return -1;
    }

    if !state::set_tcp_slot(listen_handle, new_listen_socket) {
        stack.remove_socket(new_listen_socket);
        return -1;
    }

    state::alloc_tcp_slot(listen_socket).unwrap_or_else(|| {
        stack.tcp_close(listen_socket);
        stack.remove_socket(listen_socket);
        -1
    })
}

pub(super) unsafe fn net_tcp_shutdown_impl(handle: i64) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };

    if stack.tcp_shutdown(socket).is_ok() {
        0
    } else {
        -1
    }
}

pub(super) unsafe fn net_tcp_nodelay_impl(handle: i64, on: i64) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };

    stack.tcp_set_nodelay(socket, on != 0);
    0
}

pub(super) unsafe fn net_tcp_keepalive_impl(handle: i64, ms: u64) -> i64 {
    let Some(stack) = state::user_net_stack_mut() else {
        return -1;
    };
    let Some(socket) = state::get_tcp_slot(handle) else {
        return -1;
    };

    stack.tcp_set_keepalive(socket, ms);
    0
}
