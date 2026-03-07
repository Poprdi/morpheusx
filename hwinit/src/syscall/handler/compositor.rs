
// ═══════════════════════════════════════════════════════════════════════
// COMPOSITOR SYSCALLS (91-95)
// ═══════════════════════════════════════════════════════════════════════

/// Surface entry returned by SYS_WIN_SURFACE_LIST.
/// Must match `libmorpheus::compositor::SurfaceEntry` exactly.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SurfaceEntry {
    pub pid: u32,
    pub _pad: u32,
    pub phys_addr: u64,
    pub pages: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub dirty: u32,
    pub _pad2: u32,
}

/// `SYS_COMPOSITOR_SET() → 0`
///
/// Register the calling process as the window compositor.  Only one
/// process can be compositor at a time.  The compositor gets the real
/// back buffer via SYS_FB_MAP; all other processes get private surfaces.
/// Returns EBUSY if another compositor is already registered.
pub unsafe fn sys_compositor_set() -> u64 {
    let pid = SCHEDULER.current_pid();
    if COMPOSITOR_PID != 0 && COMPOSITOR_PID != pid {
        return EBUSY;
    }
    COMPOSITOR_PID = pid;
    0
}

/// `SYS_WIN_SURFACE_LIST(buf_ptr, max_count) → count`
///
/// Returns a list of all active per-process framebuffer surfaces.
/// Only callable by the compositor process.
///
/// Each entry is a `SurfaceEntry` struct.  Returns the number of
/// surfaces written (or total count if buf_ptr is 0).
pub unsafe fn sys_win_surface_list(buf_ptr: u64, max_count: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID {
        return EPERM;
    }

    let fb_info = match FB_REGISTERED {
        Some(i) => i,
        None => return 0,
    };

    // Count surfaces (exclude zombies — about to be reaped).
    let mut total = 0u64;
    {
        use crate::process::scheduler::PROCESS_TABLE;
        use crate::process::ProcessState;
        for proc in PROCESS_TABLE.iter().flatten() {
            if !proc.is_free()
                && !matches!(proc.state, ProcessState::Zombie)
                && proc.fb_surface_phys != 0
            {
                total += 1;
            }
        }
    }

    if buf_ptr == 0 || max_count == 0 {
        return total;
    }

    let entry_size = core::mem::size_of::<SurfaceEntry>() as u64;
    let total_size = max_count.saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total_size) {
        return EFAULT;
    }

    let out = core::slice::from_raw_parts_mut(buf_ptr as *mut SurfaceEntry, max_count as usize);
    let mut written = 0usize;

    {
        use crate::process::scheduler::PROCESS_TABLE;
        use crate::process::ProcessState;
        for slot in PROCESS_TABLE.iter() {
            if written >= max_count as usize {
                break;
            }
            if let Some(proc) = slot {
                if !proc.is_free()
                    && !matches!(proc.state, ProcessState::Zombie)
                    && proc.fb_surface_phys != 0
                {
                    out[written] = SurfaceEntry {
                        pid: proc.pid,
                        _pad: 0,
                        phys_addr: proc.fb_surface_phys,
                        pages: proc.fb_surface_pages,
                        width: fb_info.width,
                        height: fb_info.height,
                        stride: fb_info.stride,
                        format: fb_info.format,
                        dirty: if proc.fb_surface_dirty { 1 } else { 0 },
                        _pad2: 0,
                    };
                    written += 1;
                }
            }
        }
    }

    written as u64
}

/// `SYS_WIN_SURFACE_MAP(target_pid) → virt_addr`
///
/// Map another process's per-process framebuffer surface into the
/// compositor's address space (read-only).  Only callable by the compositor.
///
/// Returns the virtual address in the compositor's address space, or
/// EINVAL if the target has no surface, or EPERM if caller isn't compositor.
pub unsafe fn sys_win_surface_map(target_pid: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID {
        return EPERM;
    }

    let target_pid_u32 = target_pid as u32;

    // Find the target's surface physical address and page count.
    let (phys, pages) = {
        use crate::process::scheduler::PROCESS_TABLE;
        match PROCESS_TABLE.get(target_pid_u32 as usize) {
            Some(Some(proc)) if !proc.is_free() && proc.fb_surface_phys != 0 => {
                (proc.fb_surface_phys, proc.fb_surface_pages)
            }
            _ => return EINVAL,
        }
    };

    // Map into compositor's address space (writable so compositor can
    // potentially clear/init the surface; actual compositing only reads).
    sys_map_phys(phys, pages, 0x01)
}

/// `SYS_MOUSE_FORWARD(target_pid, packed_state) → 0`
///
/// Forward mouse input to a specific process's per-process accumulator.
/// Only callable by the compositor.
///
/// packed_state: bits [15:0] = dx (i16), [31:16] = dy (i16), [39:32] = buttons.
pub unsafe fn sys_mouse_forward(target_pid: u64, packed: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID {
        return EPERM;
    }

    let target_pid_u32 = target_pid as u32;
    let dx = packed as i16 as i32;
    let dy = (packed >> 16) as i16 as i32;
    let buttons = (packed >> 32) as u8;

    use crate::process::scheduler::PROCESS_TABLE;
    match PROCESS_TABLE.get_mut(target_pid_u32 as usize) {
        Some(Some(proc)) if !proc.is_free() => {
            proc.mouse_dx = proc.mouse_dx.saturating_add(dx);
            proc.mouse_dy = proc.mouse_dy.saturating_add(dy);
            proc.mouse_buttons = buttons;
            0
        }
        _ => EINVAL,
    }
}

/// `SYS_WIN_SURFACE_DIRTY_CLEAR(target_pid) → 0`
///
/// Clear the dirty flag on a target process's surface.
/// Only callable by the compositor (after it has read the surface).
pub unsafe fn sys_win_surface_dirty_clear(target_pid: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID {
        return EPERM;
    }

    let target_pid_u32 = target_pid as u32;

    use crate::process::scheduler::PROCESS_TABLE;
    match PROCESS_TABLE.get_mut(target_pid_u32 as usize) {
        Some(Some(proc)) if !proc.is_free() => {
            proc.fb_surface_dirty = false;
            0
        }
        _ => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// NON-BLOCKING WAIT
// ═══════════════════════════════════════════════════════════════════════

/// `SYS_TRY_WAIT(pid) → exit_code | EAGAIN | ESRCH`
///
/// Non-blocking wait: if the child is a zombie, reap it and return the
/// exit code.  If the child is still running, return EAGAIN.
pub unsafe fn sys_try_wait(pid: u64) -> u64 {
    crate::process::scheduler::try_wait_child(pid as u32)
}

// ═══════════════════════════════════════════════════════════════════════
// COMPOSITOR INPUT FORWARDING (97)
// ═══════════════════════════════════════════════════════════════════════

/// `SYS_FORWARD_INPUT(target_pid, ptr, len) → bytes_written`
///
/// Push keyboard bytes into a target process's per-process input buffer.
/// Only callable by the compositor.  This is the Wayland model done right:
/// the compositor sees all input first, makes routing decisions (focus,
/// window management, Alt+Tab), then forwards the relevant bytes to the
/// focused child.  No pipes.  No indirection.  No prayer-based IPC.
///
/// The target process may be blocked on `BlockReason::InputRead` — if so,
/// we wake it immediately.  If it's running, the bytes accumulate in the
/// ring buffer until the next `read(fd=0)`.
pub unsafe fn sys_forward_input(target_pid: u64, ptr: u64, len: u64) -> u64 {
    // Only the compositor gets to play input router.
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID {
        return EPERM;
    }

    if len == 0 {
        return 0;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }

    let target = target_pid as u32;
    let data = core::slice::from_raw_parts(ptr as *const u8, len as usize);

    use crate::process::scheduler::PROCESS_TABLE;
    let written = match PROCESS_TABLE.get_mut(target as usize) {
        Some(Some(proc)) if !proc.is_free() => {
            // Write into the per-process input ring buffer.
            // 256-byte ring, wraps modulo 256 (u8 overflow does this for free).
            let mut n = 0usize;
            for &byte in data {
                let next = proc.input_head.wrapping_add(1);
                if next == proc.input_tail {
                    break; // full — compositor is shouting into a mailbox nobody's checking
                }
                proc.input_buf[proc.input_head as usize] = byte;
                proc.input_head = next;
                n += 1;
            }
            n
        }
        _ => return EINVAL,
    };

    // Wake the target if it was politely waiting for input.
    crate::process::scheduler::wake_input_reader(target);

    written as u64
}
