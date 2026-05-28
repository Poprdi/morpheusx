// Framebuffer syscalls: info, map, lock/unlock, present, blit, mark_dirty.

use super::common::*;
use super::hw::sys_map_phys;
use super::nic_fb::{fb_registered, FbInfo, FB_BACK_PAGES, FB_BACK_PHYS, FB_DIRTY, FB_SHADOW_PHYS};
use crate::hal;
use crate::schedular::SCHEDULER;
use morpheus_foundation::PAGE_SIZE;
use morpheus_hal_api::{AllocKind, MemoryType};

/// Per LD26: a transient CR3 switch to the kernel's PML4 so we can touch
/// identity-mapped physical pages even when the current address space is
/// a user PML4. On `drop`, restores the previously-loaded CR3.
///
/// If the kernel PML4 isn't available yet (pre-scheduler init) or we're
/// already running under it, drop is a no-op.
pub(crate) struct KernelCr3Guard {
    prev_cr3: u64,
    switched: bool,
}

impl KernelCr3Guard {
    pub(crate) fn enter() -> Self {
        let kernel_cr3 = hal().paging().kernel_pml4_phys();
        let prev = hal().paging().current_cr3();
        if kernel_cr3 == 0 {
            return Self {
                prev_cr3: prev,
                switched: false,
            };
        }
        if (prev & !0xFFF) == (kernel_cr3 & !0xFFF) {
            // Already on the kernel address space.
            return Self {
                prev_cr3: prev,
                switched: false,
            };
        }
        unsafe {
            hal().paging().write_cr3(kernel_cr3);
        }
        Self {
            prev_cr3: prev,
            switched: true,
        }
    }
}

impl Drop for KernelCr3Guard {
    fn drop(&mut self) {
        if self.switched {
            unsafe {
                hal().paging().write_cr3(self.prev_cr3);
            }
        }
    }
}

/// Copies `FbInfo` to user; ENODEV if no framebuffer registered.
pub unsafe fn sys_fb_info(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<FbInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }
    match fb_registered() {
        Some(info) => {
            core::ptr::write(buf_ptr as *mut FbInfo, info);
            0
        },
        None => ENODEV,
    }
}

/// 0 = unlocked.
static FB_LOCK_PID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// 0 = no compositor. Compositor owns the real back buffer; others get
/// per-process offscreen surfaces.
pub(crate) static COMPOSITOR_PID: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

#[inline]
pub unsafe fn is_composited_client() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let cpid = COMPOSITOR_PID.load(Relaxed);
    cpid != 0 && SCHEDULER.current_pid() != cpid
}

#[inline]
pub unsafe fn compositor_active() -> bool {
    COMPOSITOR_PID.load(core::sync::atomic::Ordering::Relaxed) != 0
}

pub unsafe fn release_fb_lock_if_holder(pid: u32) {
    use core::sync::atomic::Ordering::Relaxed;
    let _ = FB_LOCK_PID.compare_exchange(pid, 0, Relaxed, Relaxed);
    // Compositor exit: SIGKILL all composited children. Otherwise they keep
    // writing to invisible surfaces while present/blit silently target the
    // real back buffer (children's display freezes).
    if COMPOSITOR_PID.load(Relaxed) == pid {
        use crate::schedular::PROCESS_TABLE;
        for proc in PROCESS_TABLE.iter().flatten() {
            if !proc.is_free() && proc.pid != pid && proc.fb_surface_phys != 0 {
                let _ =
                    SCHEDULER.send_signal_inner(proc.pid, crate::process::signals::Signal::SIGKILL);
            }
        }
        COMPOSITOR_PID.store(0, Relaxed);
    }
}

/// No-op for composited clients (private surfaces don't contend).
pub unsafe fn sys_fb_lock() -> u64 {
    use core::sync::atomic::Ordering::Relaxed;
    if is_composited_client() {
        return 0;
    }
    let pid = SCHEDULER.current_pid();
    let cur = FB_LOCK_PID.load(Relaxed);
    if cur != 0 && cur != pid {
        return EBUSY;
    }
    FB_LOCK_PID.store(pid, Relaxed);
    0
}

/// Only the holder can unlock. No-op for composited clients.
pub unsafe fn sys_fb_unlock() -> u64 {
    use core::sync::atomic::Ordering::Relaxed;
    if is_composited_client() {
        return 0;
    }
    let pid = SCHEDULER.current_pid();
    let cur = FB_LOCK_PID.load(Relaxed);
    if cur != pid && cur != 0 {
        return EPERM;
    }
    FB_LOCK_PID.store(0, Relaxed);
    0
}

/// Holder PID. Composited clients always see 0.
pub fn fb_lock_holder() -> u32 {
    use core::sync::atomic::Ordering::Relaxed;
    let cpid = COMPOSITOR_PID.load(Relaxed);
    if cpid != 0 && SCHEDULER.current_pid() != cpid {
        return 0;
    }
    FB_LOCK_PID.load(Relaxed)
}

/// Shutdown helper: drops FB lock and compositor ownership.
pub unsafe fn shutdown_release_display_ownership() {
    use core::sync::atomic::Ordering::Relaxed;
    FB_LOCK_PID.store(0, Relaxed);
    COMPOSITOR_PID.store(0, Relaxed);
}

/// Legacy: maps the shared back buffer. With a compositor active, the
/// compositor still gets the real buffer; other processes get a private
/// per-process surface that the compositor reads from.
pub unsafe fn sys_fb_map() -> u64 {
    let info = match fb_registered() {
        Some(i) => i,
        None => {
            crate::serial::log_warn("FB", 790, "fb_map with no framebuffer registered");
            return ENODEV;
        },
    };

    if is_composited_client() {
        crate::serial::log_info("FB", 791, "mapped private composited surface");
        return sys_fb_map_surface(&info);
    }

    crate::serial::log_info("FB", 792, "mapped real back buffer");
    if FB_BACK_PHYS.load(core::sync::atomic::Ordering::Relaxed) == 0 {
        if let Err(e) = fb_map_alloc_buffers(&info) {
            return e;
        }
    }

    // 0x01 = writable cacheable.
    sys_map_phys(
        FB_BACK_PHYS.load(core::sync::atomic::Ordering::Relaxed),
        FB_BACK_PAGES.load(core::sync::atomic::Ordering::Relaxed),
        0x01,
    )
}

/// Per-process surface for composited clients. Dimensions match VRAM.
unsafe fn sys_fb_map_surface(info: &FbInfo) -> u64 {
    let proc = SCHEDULER.current_memory_leader_mut();

    if proc.fb_surface_phys != 0 {
        return sys_map_phys(proc.fb_surface_phys, proc.fb_surface_pages, 0x01);
    }

    // Drop registry before sys_map_phys — ensure_user_table re-acquires
    // GLOBAL_REGISTRY and would deadlock otherwise.
    let pages = info.size.div_ceil(PAGE_SIZE);
    let phys = match hal()
        .phys()
        .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages)
    {
        Ok(p) => p,
        Err(_) => return ENOMEM,
    };

    {
        let _guard = KernelCr3Guard::enter();
        core::ptr::write_bytes(phys as *mut u8, 0, info.size as usize);
    }

    let proc = SCHEDULER.current_process_mut();
    proc.fb_surface_phys = phys;
    proc.fb_surface_pages = pages;
    proc.fb_surface_dirty = false;

    sys_map_phys(phys, pages, 0x01)
}

/// Allocate back + shadow, seed both from VRAM.
unsafe fn fb_map_alloc_buffers(info: &FbInfo) -> Result<(), u64> {
    let pages = info.size.div_ceil(PAGE_SIZE);

    let back_phys = hal()
        .phys()
        .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages)
        .map_err(|_| ENOMEM)?;

    let shadow_phys =
        match hal()
            .phys()
            .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages)
        {
            Ok(p) => p,
            Err(_) => {
                let _ = hal().phys().free_pages(back_phys, pages);
                return Err(ENOMEM);
            },
        };

    // Identity-mapped phys access requires kernel CR3.
    {
        let _guard = KernelCr3Guard::enter();
        core::ptr::copy_nonoverlapping(
            info.base as *const u8,
            back_phys as *mut u8,
            info.size as usize,
        );
        core::ptr::copy_nonoverlapping(
            info.base as *const u8,
            shadow_phys as *mut u8,
            info.size as usize,
        );
    }

    // Store pages first so readers see a valid count once phys becomes nonzero.
    FB_BACK_PAGES.store(pages, core::sync::atomic::Ordering::Relaxed);
    FB_SHADOW_PHYS.store(shadow_phys, core::sync::atomic::Ordering::Relaxed);
    FB_BACK_PHYS.store(back_phys, core::sync::atomic::Ordering::Release);
    Ok(())
}

/// Timer-ISR-driven delta present: back vs shadow → VRAM.
/// Skipped when FB_LOCK_PID != 0 (holder calls SYS_FB_PRESENT directly).
pub unsafe fn fb_present_tick() {
    use core::sync::atomic::Ordering::Relaxed;
    let back = FB_BACK_PHYS.load(Relaxed);
    let shadow = FB_SHADOW_PHYS.load(Relaxed);
    if back == 0 || shadow == 0 {
        return;
    }
    if FB_LOCK_PID.load(Relaxed) != 0 {
        return;
    }
    if !FB_DIRTY.swap(false, Relaxed) {
        return;
    }

    let info = match fb_registered() {
        Some(i) => i,
        None => return,
    };

    let _guard = KernelCr3Guard::enter();

    let stride_pixels = info.stride / 4;
    hal().compositor().fb_present_delta(
        back,
        shadow,
        info.base,
        info.width as u64,
        info.height as u64,
        stride_pixels as u64,
    );
}

/// Synchronous delta present (compositor/legacy). Composited clients
/// just mark their surface dirty.
pub unsafe fn sys_fb_present() -> u64 {
    if is_composited_client() {
        let proc = SCHEDULER.current_process_mut();
        proc.fb_surface_dirty = true;
        return 0;
    }

    use core::sync::atomic::Ordering::Relaxed;
    let back = FB_BACK_PHYS.load(Relaxed);
    let shadow = FB_SHADOW_PHYS.load(Relaxed);
    if back == 0 || shadow == 0 {
        return ENODEV;
    }
    let info = match fb_registered() {
        Some(i) => i,
        None => return ENODEV,
    };

    let _guard = KernelCr3Guard::enter();

    let stride_pixels = info.stride / 4;
    hal().compositor().fb_present_delta(
        back,
        shadow,
        info.base,
        info.width as u64,
        info.height as u64,
        stride_pixels as u64,
    );

    0
}

/// Full-screen memcpy present, no delta.
pub unsafe fn sys_fb_blit() -> u64 {
    if is_composited_client() {
        let proc = SCHEDULER.current_process_mut();
        proc.fb_surface_dirty = true;
        return 0;
    }

    use core::sync::atomic::Ordering::Relaxed;
    let back = FB_BACK_PHYS.load(Relaxed);
    if back == 0 {
        return ENODEV;
    }
    let info = match fb_registered() {
        Some(i) => i,
        None => return ENODEV,
    };

    let _guard = KernelCr3Guard::enter();

    let bytes = info.size as usize;
    let back_ptr = back as *const u8;
    let vram = info.base as *mut u8;

    core::ptr::copy_nonoverlapping(back_ptr, vram, bytes);

    // Keep shadow in sync — a later delta present would otherwise over-copy.
    let shadow = FB_SHADOW_PHYS.load(Relaxed);
    if shadow != 0 {
        let shadow_ptr = shadow as *mut u8;
        core::ptr::copy_nonoverlapping(back_ptr, shadow_ptr, bytes);
    }

    0
}

/// Re-export so the dispatcher can use the public name. `fb_mark_dirty`
/// lives in `nic_fb` because static state for the back buffer was
/// physically split across the two files in the legacy tree.
#[inline]
pub fn fb_mark_dirty() {
    super::nic_fb::fb_mark_dirty();
}
