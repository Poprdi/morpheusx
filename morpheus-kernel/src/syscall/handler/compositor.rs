// Compositor / per-process surface syscalls.

use super::common::*;
use super::fb::COMPOSITOR_PID;
use super::hw::sys_map_phys;
use super::nic_fb::fb_registered;
use crate::process::ProcessState;
use crate::schedular::{PROCESS_TABLE, PROCESS_TABLE_LOCK, SCHEDULER};

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

/// `SYS_COMPOSITOR_SET() → 0` — register the calling process as the WM.
pub unsafe fn sys_compositor_set() -> u64 {
    use core::sync::atomic::Ordering::Relaxed;
    let pid = SCHEDULER.current_pid();
    let cur = COMPOSITOR_PID.load(Relaxed);
    if cur != 0 && cur != pid {
        let _ = cur;
        crate::serial::log_warn("COMP", 780, "compositor already registered (EBUSY)");
        return EBUSY;
    }
    COMPOSITOR_PID.store(pid, Relaxed);
    let _ = pid;
    crate::serial::log_ok("COMP", 781, "compositor registered");
    0
}

pub unsafe fn sys_win_surface_list(buf_ptr: u64, max_count: u64) -> u64 {
    use core::sync::atomic::Ordering::Relaxed;
    let cpid = COMPOSITOR_PID.load(Relaxed);
    if cpid == 0 {
        return u64::MAX;
    }

    let pid = SCHEDULER.current_pid();
    if pid != cpid {
        return 0;
    }

    let fb_info = match fb_registered() {
        Some(i) => i,
        None => return 0,
    };

    let mut total = 0u64;
    {
        PROCESS_TABLE_LOCK.lock();
        for proc in PROCESS_TABLE.iter().flatten() {
            if !proc.is_free()
                && !matches!(proc.state, ProcessState::Zombie)
                && proc.fb_surface_phys != 0
            {
                total += 1;
            }
        }
        PROCESS_TABLE_LOCK.unlock();
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
        PROCESS_TABLE_LOCK.lock();
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
        PROCESS_TABLE_LOCK.unlock();
    }
    written as u64
}

pub unsafe fn sys_win_surface_map(target_pid: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID.load(core::sync::atomic::Ordering::Relaxed) {
        return EPERM;
    }

    let target_pid_u32 = target_pid as u32;

    let (phys, pages) = {
        PROCESS_TABLE_LOCK.lock();
        let result = match PROCESS_TABLE.get(target_pid_u32 as usize) {
            Some(Some(proc)) if !proc.is_free() && proc.fb_surface_phys != 0 => {
                Some((proc.fb_surface_phys, proc.fb_surface_pages))
            },
            _ => None,
        };
        PROCESS_TABLE_LOCK.unlock();
        match result {
            Some(r) => r,
            None => return EINVAL,
        }
    };

    sys_map_phys(phys, pages, 0x01)
}

pub unsafe fn sys_mouse_forward(target_pid: u64, packed: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID.load(core::sync::atomic::Ordering::Relaxed) {
        return EPERM;
    }

    let target_pid_u32 = target_pid as u32;
    let dx = packed as i16 as i32;
    let dy = (packed >> 16) as i16 as i32;
    let buttons = (packed >> 32) as u8;

    PROCESS_TABLE_LOCK.lock();
    let result = match PROCESS_TABLE.get_mut(target_pid_u32 as usize) {
        Some(Some(proc)) if !proc.is_free() => {
            proc.mouse_dx = proc.mouse_dx.saturating_add(dx);
            proc.mouse_dy = proc.mouse_dy.saturating_add(dy);
            proc.mouse_buttons = buttons;
            0
        },
        _ => EINVAL,
    };
    PROCESS_TABLE_LOCK.unlock();
    result
}

pub unsafe fn sys_win_surface_dirty_clear(target_pid: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID.load(core::sync::atomic::Ordering::Relaxed) {
        return EPERM;
    }

    let target_pid_u32 = target_pid as u32;

    PROCESS_TABLE_LOCK.lock();
    let result = match PROCESS_TABLE.get_mut(target_pid_u32 as usize) {
        Some(Some(proc)) if !proc.is_free() => {
            proc.fb_surface_dirty = false;
            0
        },
        _ => EINVAL,
    };
    PROCESS_TABLE_LOCK.unlock();
    result
}

pub unsafe fn sys_try_wait(pid: u64) -> u64 {
    crate::schedular::try_wait_child(pid as u32)
}

/// Push keyboard bytes into a target process's per-process input buffer.
pub unsafe fn sys_forward_input(target_pid: u64, ptr: u64, len: u64) -> u64 {
    let pid = SCHEDULER.current_pid();
    if pid != COMPOSITOR_PID.load(core::sync::atomic::Ordering::Relaxed) {
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

    PROCESS_TABLE_LOCK.lock();
    let written = match PROCESS_TABLE.get_mut(target as usize) {
        Some(Some(proc)) if !proc.is_free() => {
            let mut n = 0usize;
            for &byte in data {
                let next = proc.input_head.wrapping_add(1);
                if next == proc.input_tail {
                    break;
                }
                proc.input_buf[proc.input_head as usize] = byte;
                proc.input_head = next;
                n += 1;
            }
            n
        },
        _ => {
            PROCESS_TABLE_LOCK.unlock();
            return EINVAL;
        },
    };
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(target as usize) {
        if matches!(
            proc.state,
            ProcessState::Blocked(crate::process::BlockReason::InputRead)
        ) {
            proc.state = ProcessState::Ready;
            crate::schedular::clear_input_waiter(target);
        }
    }
    PROCESS_TABLE_LOCK.unlock();

    written as u64
}
