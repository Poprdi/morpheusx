// SYS_NIC_INFO (32) — get NIC information

const ENODEV: u64 = u64::MAX - 19;

/// `SYS_NIC_INFO(buf_ptr) → 0`
pub unsafe fn sys_nic_info(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<NicInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }
    let mut info = NicInfo {
        mac: [0u8; 8],
        link_up: 0,
        present: 0,
    };

    if let Some(mac_fn) = NIC_OPS.mac {
        info.present = 1;
        mac_fn(info.mac.as_mut_ptr());
        if let Some(link_fn) = NIC_OPS.link_up {
            info.link_up = link_fn() as u32;
        }
    }

    core::ptr::write(buf_ptr as *mut NicInfo, info);
    0
}

// SYS_NIC_TX (33) — transmit a raw Ethernet frame

/// `SYS_NIC_TX(frame_ptr, frame_len) → 0`
pub unsafe fn sys_nic_tx(frame_ptr: u64, frame_len: u64) -> u64 {
    if !validate_user_buf(frame_ptr, frame_len) {
        return EFAULT;
    }
    if !(14..=9000).contains(&frame_len) {
        return EINVAL; // min Ethernet header, max jumbo frame
    }
    match NIC_OPS.tx {
        Some(tx_fn) => {
            let rc = tx_fn(frame_ptr as *const u8, frame_len as usize);
            if rc < 0 {
                EIO
            } else {
                0
            }
        }
        None => ENODEV,
    }
}

// SYS_NIC_RX (34) — receive a raw Ethernet frame

/// `SYS_NIC_RX(buf_ptr, buf_len) → bytes_received`
pub unsafe fn sys_nic_rx(buf_ptr: u64, buf_len: u64) -> u64 {
    if !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }
    match NIC_OPS.rx {
        Some(rx_fn) => {
            let rc = rx_fn(buf_ptr as *mut u8, buf_len as usize);
            if rc < 0 {
                EIO
            } else {
                rc as u64
            }
        }
        None => ENODEV,
    }
}

// SYS_NIC_LINK (35) — get link status

/// `SYS_NIC_LINK() → 0/1 (down/up)`
pub unsafe fn sys_nic_link() -> u64 {
    match NIC_OPS.link_up {
        Some(f) => f() as u64,
        None => ENODEV,
    }
}

// SYS_NIC_MAC (36) — get 6-byte MAC address

/// `SYS_NIC_MAC(buf_ptr) → 0`
pub unsafe fn sys_nic_mac(buf_ptr: u64) -> u64 {
    if !validate_user_buf(buf_ptr, 6) {
        return EFAULT;
    }
    match NIC_OPS.mac {
        Some(f) => {
            f(buf_ptr as *mut u8);
            0
        }
        None => ENODEV,
    }
}

// SYS_NIC_REFILL (37) — refill RX descriptor ring

/// `SYS_NIC_REFILL() → 0`
pub unsafe fn sys_nic_refill() -> u64 {
    match NIC_OPS.refill {
        Some(f) => {
            f();
            0
        }
        None => ENODEV,
    }
}

// NIC_CTRL — hardware-level NIC control (exokernel)

/// `sys_nic_ctrl(cmd, arg) → 0`
///
/// Direct hardware control: promiscuous mode, MAC spoofing, VLAN,
/// checksum offloads, ring sizing, interrupt coalescing, etc.
pub unsafe fn sys_nic_ctrl(cmd: u64, arg: u64) -> u64 {
    match NIC_OPS.ctrl {
        Some(f) => {
            let rc = f(cmd as u32, arg);
            if rc < 0 {
                EIO
            } else {
                rc as u64
            }
        }
        None => ENODEV,
    }
}

// SYS_IOCTL (42) — device control

// ioctl commands
const IOCTL_FIONREAD: u64 = 0x541B; // bytes available on fd (like FIONREAD)
const IOCTL_TIOCGWINSZ: u64 = 0x5413; // get terminal window size

/// `SYS_IOCTL(fd, cmd, arg) → result`
pub unsafe fn sys_ioctl(fd: u64, cmd: u64, arg: u64) -> u64 {
    match (fd, cmd) {
        // FIONREAD: bytes available on fd without blocking.
        (0, IOCTL_FIONREAD) => {
            // Where does stdin data live?
            //
            // 1. fd 0 has an explicit pipe → pipe_available()
            // 2. Composited client (no pipe) → per-process input buffer
            // 3. Everyone else → global stdin ring buffer
            //
            // This is the Wayland model: composited clients don't touch the
            // global stdin.  The compositor reads it, makes routing decisions,
            // and pushes bytes into the target's input_buf via SYS_FORWARD_INPUT.
            let fd_table = SCHEDULER.current_fd_table_mut();
            let avail = if let Ok(desc) = fd_table.get(0) {
                if desc.flags & O_PIPE_READ != 0 {
                    crate::pipe::pipe_available(desc.mount_idx)
                } else if is_composited_client() {
                    let proc = SCHEDULER.current_process_mut();
                    proc.input_head.wrapping_sub(proc.input_tail) as usize
                } else {
                    crate::stdin::available()
                }
            } else if is_composited_client() {
                let proc = SCHEDULER.current_process_mut();
                proc.input_head.wrapping_sub(proc.input_tail) as usize
            } else {
                crate::stdin::available()
            };
            if arg != 0 && validate_user_buf(arg, 4) {
                core::ptr::write(arg as *mut u32, avail as u32);
            }
            avail as u64
        }
        // Terminal window size: derive from framebuffer if available, else 80×25.
        (0..=2, IOCTL_TIOCGWINSZ) => {
            if arg != 0 && validate_user_buf(arg, 8) {
                let (rows, cols, xpix, ypix) = match fb_registered() {
                    Some(fb) => {
                        let c = fb.width / 8; // 8px font width
                        let r = fb.height / 16; // 16px font height
                        (r as u16, c as u16, fb.width as u16, fb.height as u16)
                    }
                    None => (25, 80, 0, 0),
                };
                let buf = arg as *mut u16;
                *buf = rows; // ws_row
                *buf.add(1) = cols; // ws_col
                *buf.add(2) = xpix; // ws_xpixel
                *buf.add(3) = ypix; // ws_ypixel
            }
            0
        }
        _ => EINVAL,
    }
}

// SYS_MOUNT (43) — mount a filesystem

/// `SYS_MOUNT(src_ptr, src_len, dst_ptr, dst_len) → 0`
///
/// Mount the HelixFS volume at `src` to the mount point `dst`.
/// Currently a no-op success since HelixFS auto-mounts at `/`.
pub unsafe fn sys_mount(src_ptr: u64, src_len: u64, dst_ptr: u64, dst_len: u64) -> u64 {
    let _src = match user_path(src_ptr, src_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let _dst = match user_path(dst_ptr, dst_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    // HelixFS is always mounted at "/" — additional mounts not supported yet.
    0
}

// SYS_UMOUNT (44) — unmount a filesystem

/// `SYS_UMOUNT(path_ptr, path_len) → 0`
///
/// Unmount the filesystem at `path`.  Syncs dirty data before unmounting.
/// Currently: syncs and returns success (root cannot be truly unmounted).
pub unsafe fn sys_umount(path_ptr: u64, path_len: u64) -> u64 {
    let _path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    // Sync all dirty data before "unmounting".
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let _ = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table);
    0
}

// SYS_POLL (45) — poll file descriptors for readiness

/// Poll entry (matches POSIX pollfd).
#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;

/// `SYS_POLL(fds_ptr, nfds, timeout_ms) → ready_count`
///
/// Check if file descriptors are ready for I/O.
/// - fd 0 (stdin): POLLIN if keyboard data available.
/// - fd 1/2 (stdout/stderr): always POLLOUT (serial is always writable).
/// - fd >= 3 (VFS): always POLLIN|POLLOUT (files are always ready).
pub unsafe fn sys_poll(fds_ptr: u64, nfds: u64, timeout_ms: u64) -> u64 {
    if nfds == 0 {
        // Just sleep for timeout_ms.
        if timeout_ms > 0 {
            let _ = sys_sleep(timeout_ms);
        }
        return 0;
    }
    let size = nfds * core::mem::size_of::<PollFd>() as u64;
    if !validate_user_buf(fds_ptr, size) {
        return EFAULT;
    }

    let fds = core::slice::from_raw_parts_mut(fds_ptr as *mut PollFd, nfds as usize);
    let mut ready = 0u64;

    for pfd in fds.iter_mut() {
        pfd.revents = 0;
        match pfd.fd {
            0 => {
                // Check if fd 0 is redirected to a pipe.
                let fd_table = SCHEDULER.current_fd_table_mut();
                if let Ok(desc) = fd_table.get(0) {
                    if desc.flags & O_PIPE_READ != 0 {
                        if pfd.events & POLLIN != 0
                            && crate::pipe::pipe_available(desc.mount_idx) > 0
                        {
                            pfd.revents |= POLLIN;
                            ready += 1;
                        }
                        continue;
                    }
                }
                // Fallback: kernel ring buffer stdin.
                if pfd.events & POLLIN != 0 && crate::stdin::available() > 0 {
                    pfd.revents |= POLLIN;
                    ready += 1;
                }
            }
            1 | 2 => {
                // stdout/stderr — serial always writable
                if pfd.events & POLLOUT != 0 {
                    pfd.revents |= POLLOUT;
                    ready += 1;
                }
            }
            fd if fd >= 3 => {
                // VFS files are always "ready"
                if pfd.events & POLLIN != 0 {
                    pfd.revents |= POLLIN;
                }
                if pfd.events & POLLOUT != 0 {
                    pfd.revents |= POLLOUT;
                }
                if pfd.revents != 0 {
                    ready += 1;
                }
            }
            _ => {
                pfd.revents = POLLERR;
                ready += 1;
            }
        }
    }

    // If nothing is ready yet and timeout > 0, sleep in small chunks and
    // re-check until something is ready or the timeout expires.
    // PERF FIX: removed 100ms cap — now uses full requested timeout,
    // sleeping in 10ms increments to balance latency and CPU usage.
    if ready == 0 && timeout_ms > 0 {
        let mut remaining_ms = timeout_ms;
        while remaining_ms > 0 {
            let chunk = remaining_ms.min(10);
            let _ = sys_sleep(chunk);
            remaining_ms = remaining_ms.saturating_sub(chunk);

            // Re-check all fds after each sleep chunk.
            for pfd in fds.iter_mut() {
                if pfd.fd == 0 && pfd.events & POLLIN != 0 {
                    let has_data = {
                        let fd_table = SCHEDULER.current_fd_table_mut();
                        if let Ok(desc) = fd_table.get(0) {
                            if desc.flags & O_PIPE_READ != 0 {
                                crate::pipe::pipe_available(desc.mount_idx) > 0
                            } else {
                                crate::stdin::available() > 0
                            }
                        } else {
                            crate::stdin::available() > 0
                        }
                    };
                    if has_data {
                        pfd.revents |= POLLIN;
                        ready += 1;
                    }
                }
            }
            if ready > 0 {
                break;
            }
        }
    }

    ready
}
