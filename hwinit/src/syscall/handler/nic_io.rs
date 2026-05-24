const ENODEV: u64 = u64::MAX - 19;

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

pub unsafe fn sys_nic_tx(frame_ptr: u64, frame_len: u64) -> u64 {
    if !validate_user_buf(frame_ptr, frame_len) {
        return EFAULT;
    }
    if !(14..=9000).contains(&frame_len) {
        return EINVAL; // 14B Ethernet header .. 9000B jumbo
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

/// 0 = down, 1 = up.
pub unsafe fn sys_nic_link() -> u64 {
    match NIC_OPS.link_up {
        Some(f) => f() as u64,
        None => ENODEV,
    }
}

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

pub unsafe fn sys_nic_refill() -> u64 {
    match NIC_OPS.refill {
        Some(f) => {
            f();
            0
        }
        None => ENODEV,
    }
}

/// Raw NIC control: promisc, MAC, VLAN, csum offloads, ring sizing, IRQ coalesce.
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

const IOCTL_FIONREAD: u64 = 0x541B;
const IOCTL_TIOCGWINSZ: u64 = 0x5413;

pub unsafe fn sys_ioctl(fd: u64, cmd: u64, arg: u64) -> u64 {
    match (fd, cmd) {
        (0, IOCTL_FIONREAD) => {
            // stdin source: explicit pipe → composited input_buf → global stdin ring.
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
        // TIOCGWINSZ: derive from FB (8x16 glyphs), fall back to 80x25.
        (0..=2, IOCTL_TIOCGWINSZ) => {
            if arg != 0 && validate_user_buf(arg, 8) {
                let (rows, cols, xpix, ypix) = match fb_registered() {
                    Some(fb) => {
                        let c = fb.width / 8;
                        let r = fb.height / 16;
                        (r as u16, c as u16, fb.width as u16, fb.height as u16)
                    }
                    None => (25, 80, 0, 0),
                };
                let buf = arg as *mut u16;
                *buf = rows;
                *buf.add(1) = cols;
                *buf.add(2) = xpix;
                *buf.add(3) = ypix;
            }
            0
        }
        _ => EINVAL,
    }
}

/// No-op success; HelixFS auto-mounts at `/`.
pub unsafe fn sys_mount(src_ptr: u64, src_len: u64, dst_ptr: u64, dst_len: u64) -> u64 {
    let _src = match user_path(src_ptr, src_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let _dst = match user_path(dst_ptr, dst_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    0
}

/// Syncs dirty data; root can't actually unmount.
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

/// fd 0: POLLIN iff stdin has data. fd 1/2: POLLOUT always. fd≥3 (VFS): both.
pub unsafe fn sys_poll(fds_ptr: u64, nfds: u64, timeout_ms: u64) -> u64 {
    if nfds == 0 {
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
