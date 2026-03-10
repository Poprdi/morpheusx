
// SYS_FB_INFO (63) — get framebuffer information

/// `SYS_FB_INFO(buf_ptr) → 0`
///
/// Copy the `FbInfo` struct to the user buffer.  Returns -ENODEV if
/// no framebuffer has been registered.
pub unsafe fn sys_fb_info(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<FbInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }
    match fb_registered() {
        Some(info) => {
            core::ptr::write(buf_ptr as *mut FbInfo, info);
            0
        }
        None => ENODEV,
    }
}

// SYS_FB_LOCK (85) / SYS_FB_UNLOCK (86) — exclusive framebuffer access

/// PID that currently holds exclusive framebuffer access (0 = unlocked).
static FB_LOCK_PID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

// COMPOSITOR — the compositor PID owns the real back buffer.
// Non-compositor processes get per-process offscreen surfaces.

/// PID of the registered compositor process (0 = no compositor).
static COMPOSITOR_PID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Returns true if a compositor is active and the caller is NOT the compositor.
#[inline]
pub unsafe fn is_composited_client() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let cpid = COMPOSITOR_PID.load(Relaxed);
    cpid != 0 && SCHEDULER.current_pid() != cpid
}

/// Returns true when a compositor process is registered.
#[inline]
pub unsafe fn compositor_active() -> bool {
    COMPOSITOR_PID.load(core::sync::atomic::Ordering::Relaxed) != 0
}

pub unsafe fn release_fb_lock_if_holder(pid: u32) {
    use core::sync::atomic::Ordering::Relaxed;
    let _ = FB_LOCK_PID.compare_exchange(pid, 0, Relaxed, Relaxed);
    // If the compositor exits, kill all composited children so they don't
    // continue writing to invisible per-process surfaces.  Without this,
    // children's already-mapped pointers target their private surface
    // while sys_fb_present/blit falls through to the legacy path that
    // operates on the real back buffer — the children's display freezes.
    if COMPOSITOR_PID.load(Relaxed) == pid {
        use crate::process::scheduler::PROCESS_TABLE;
        for proc in PROCESS_TABLE.iter().flatten() {
            if !proc.is_free() && proc.pid != pid && proc.fb_surface_phys != 0 {
                let _ = SCHEDULER.send_signal_inner(proc.pid, crate::process::signals::Signal::SIGKILL);
            }
        }
        COMPOSITOR_PID.store(0, Relaxed);
    }
}

/// `SYS_FB_LOCK() → 0`
///
/// Claim exclusive framebuffer access. Other processes should check
/// `fb_is_locked()` before writing to the framebuffer.
/// When a compositor is active, non-compositor processes get a no-op
/// (they have their own private surface — no contention).
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

/// `SYS_FB_UNLOCK() → 0`
///
/// Release exclusive framebuffer access. Only the lock holder can unlock.
/// No-op for composited clients.
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

/// `SYS_FB_IS_LOCKED()` — returns holder PID.
/// Composited clients always see 0 (unlocked from their perspective).
pub fn fb_lock_holder() -> u32 {
    use core::sync::atomic::Ordering::Relaxed;
    let cpid = COMPOSITOR_PID.load(Relaxed);
    if cpid != 0 && SCHEDULER.current_pid() != cpid {
        return 0;
    }
    FB_LOCK_PID.load(Relaxed)
}

// SYS_FB_MAP (64) — map the back buffer into user virtual address space

/// `SYS_FB_MAP() → virt_addr`
///
/// When no compositor is active (legacy mode): allocates a shared back buffer
/// and shadow buffer, maps the back buffer into the caller's address space.
///
/// When a compositor IS active:
///   - Compositor itself → gets the real back buffer (unchanged).
///   - All other processes → get a private per-process offscreen surface
///     (same dimensions as the real framebuffer).  The compositor reads
///     these surfaces to composite windows.
pub unsafe fn sys_fb_map() -> u64 {
    let info = match fb_registered() {
        Some(i) => i,
        None => {
            crate::serial::log_warn("FB", 790, "fb_map with no framebuffer registered");
            return ENODEV;
        }
    };

    // Compositor mode: non-compositor processes get their own surface.
    if is_composited_client() {
        crate::serial::log_info("FB", 791, "mapped private composited surface");
        return sys_fb_map_surface(&info);
    }

    // Compositor or legacy mode: map the real back buffer.
    crate::serial::log_info("FB", 792, "mapped real back buffer");
    // Allocate back + shadow buffers on first call.
    if FB_BACK_PHYS.load(core::sync::atomic::Ordering::Relaxed) == 0 {
        if let Err(e) = fb_map_alloc_buffers(&info) {
            return e;
        }
    }

    // Map back buffer as writable cacheable (flag 0x01 = writable, no 0x02 uncacheable).
    sys_map_phys(
        FB_BACK_PHYS.load(core::sync::atomic::Ordering::Relaxed),
        FB_BACK_PAGES.load(core::sync::atomic::Ordering::Relaxed),
        0x01,
    )
}

/// Allocate a per-process framebuffer surface for a composited client.
/// Same dimensions as the real framebuffer, zeroed.
unsafe fn sys_fb_map_surface(info: &FbInfo) -> u64 {
    let proc = SCHEDULER.current_memory_leader_mut();

    // If this process already has a surface, just re-map it.
    if proc.fb_surface_phys != 0 {
        return sys_map_phys(proc.fb_surface_phys, proc.fb_surface_pages, 0x01);
    }

    // Allocate physical pages for the surface.
    // CRITICAL: registry guard must be dropped BEFORE sys_map_phys.
    // sys_map_phys → ensure_user_table → global_registry_mut() would
    // self-deadlock on GLOBAL_REGISTRY if we still hold it here.
    let pages = info.size.div_ceil(4096);
    let phys = {
        let mut registry = crate::memory::global_registry_mut();
        match registry.allocate_pages(
            crate::memory::AllocateType::AnyPages,
            crate::memory::MemoryType::Allocated,
            pages,
        ) {
            Ok(p) => p,
            Err(_) => return ENOMEM,
        }
        // registry dropped here. lock released. interrupts restored.
    };

    // Zero the surface (start with black).
    {
        let _guard = crate::memory::KernelCr3Guard::enter();
        core::ptr::write_bytes(phys as *mut u8, 0, info.size as usize);
    }

    // Record in process struct. Update pages_allocated for proper cleanup
    // via free_process_resources which only frees VMA-tracked pages with
    // owns_phys=true. We track the surface separately and clean it up
    // explicitly.
    let proc = SCHEDULER.current_process_mut();
    proc.fb_surface_phys = phys;
    proc.fb_surface_pages = pages;
    proc.fb_surface_dirty = false;

    // Map into the process's address space.
    // GLOBAL_REGISTRY is NOT held here. sys_map_phys can freely allocate
    // page-table pages via ensure_user_table without deadlocking.
    sys_map_phys(phys, pages, 0x01)
}

/// Allocate back + shadow buffers, copy VRAM content into both.
unsafe fn fb_map_alloc_buffers(info: &FbInfo) -> Result<(), u64> {
    let pages = info.size.div_ceil(4096);
    let mut registry = crate::memory::global_registry_mut();

    // allocate_pages handles CR3 switching internally.
    let back_phys = registry
        .allocate_pages(
            crate::memory::AllocateType::AnyPages,
            crate::memory::MemoryType::Allocated,
            pages,
        )
        .map_err(|_| ENOMEM)?;

    let shadow_phys = match registry.allocate_pages(
        crate::memory::AllocateType::AnyPages,
        crate::memory::MemoryType::Allocated,
        pages,
    ) {
        Ok(p) => p,
        Err(_) => {
            let _ = registry.free_pages(back_phys, pages);
            return Err(ENOMEM);
        }
    };

    // VRAM and allocated buffers are at physical addresses accessed via
    // identity mapping — need kernel CR3.
    {
        let _guard = crate::memory::KernelCr3Guard::enter();
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

    // store pages first so readers see valid page count once phys is non-zero
    FB_BACK_PAGES.store(pages, core::sync::atomic::Ordering::Relaxed);
    FB_SHADOW_PHYS.store(shadow_phys, core::sync::atomic::Ordering::Relaxed);
    FB_BACK_PHYS.store(back_phys, core::sync::atomic::Ordering::Release);
    Ok(())
}

/// Push back-buffer delta to VRAM (called from timer ISR).
/// Compares back buffer vs shadow and writes only changed pixels.
/// Gated by FB_DIRTY; no syscall.
pub unsafe fn fb_present_tick() {
    use core::sync::atomic::Ordering::Relaxed;
    let back = FB_BACK_PHYS.load(Relaxed);
    let shadow = FB_SHADOW_PHYS.load(Relaxed);
    if back == 0 || shadow == 0 {
        return;
    }
    // If a process holds the FB lock it manages presents itself via
    // SYS_FB_PRESENT — skip the automatic timer-driven present to
    // avoid redundant full-screen scans.
    if FB_LOCK_PID.load(Relaxed) != 0 {
        return;
    }
    // atomic swap: if dirty was false we skip; if true we clear and proceed
    if !FB_DIRTY.swap(false, Relaxed) {
        return;
    }

    let info = match fb_registered() {
        Some(i) => i,
        None => return,
    };

    // Buffers are at identity-mapped physical addresses — need kernel CR3.
    let _guard = crate::memory::KernelCr3Guard::enter();

    let stride_pixels = info.stride / 4;

    #[cfg(target_arch = "x86_64")]
    asm_fb_present_delta(
        back,
        shadow,
        info.base,
        info.width as u64,
        info.height as u64,
        stride_pixels as u64,
    );
}

/// SYS_FB_PRESENT (88) — on-demand framebuffer present.
///
/// Compositor/legacy: runs the delta presenter synchronously.
/// Composited client: marks the per-process surface as dirty (no VRAM write).
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

    let _guard = crate::memory::KernelCr3Guard::enter();

    let stride_pixels = info.stride / 4;

    #[cfg(target_arch = "x86_64")]
    asm_fb_present_delta(
        back,
        shadow,
        info.base,
        info.width as u64,
        info.height as u64,
        stride_pixels as u64,
    );

    0
}

/// SYS_FB_BLIT (89) — full-screen memcpy present (no delta).
///
/// Compositor/legacy: copies the entire back buffer directly to VRAM.
/// Composited client: marks the per-process surface as dirty.
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

    let _guard = crate::memory::KernelCr3Guard::enter();

    let bytes = info.size as usize;
    let back_ptr = back as *const u8;
    let vram = info.base as *mut u8;

    core::ptr::copy_nonoverlapping(back_ptr, vram, bytes);

    // Keep shadow in sync so delta presents (e.g. after app releases
    // the fb lock) don't see stale data and over-copy.
    let shadow = FB_SHADOW_PHYS.load(Relaxed);
    if shadow != 0 {
        let shadow_ptr = shadow as *mut u8;
        core::ptr::copy_nonoverlapping(back_ptr, shadow_ptr, bytes);
    }

    0
}
