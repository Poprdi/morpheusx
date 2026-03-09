
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
    match FB_REGISTERED {
        Some(info) => {
            core::ptr::write(buf_ptr as *mut FbInfo, info);
            0
        }
        None => ENODEV,
    }
}

// SYS_FB_LOCK (85) / SYS_FB_UNLOCK (86) — exclusive framebuffer access

/// PID that currently holds exclusive framebuffer access (0 = unlocked).
static mut FB_LOCK_PID: u32 = 0;

// COMPOSITOR — the compositor PID owns the real back buffer.
// Non-compositor processes get per-process offscreen surfaces.

/// PID of the registered compositor process (0 = no compositor).
static mut COMPOSITOR_PID: u32 = 0;

/// Returns true if a compositor is active and the caller is NOT the compositor.
#[inline]
pub unsafe fn is_composited_client() -> bool {
    COMPOSITOR_PID != 0 && SCHEDULER.current_pid() != COMPOSITOR_PID
}

/// Returns true when a compositor process is registered.
#[inline]
pub unsafe fn compositor_active() -> bool {
    COMPOSITOR_PID != 0
}

pub unsafe fn release_fb_lock_if_holder(pid: u32) {
    if FB_LOCK_PID == pid {
        FB_LOCK_PID = 0;
    }
    // If the compositor exits, kill all composited children so they don't
    // continue writing to invisible per-process surfaces.  Without this,
    // children's already-mapped pointers target their private surface
    // while sys_fb_present/blit falls through to the legacy path that
    // operates on the real back buffer — the children's display freezes.
    if COMPOSITOR_PID == pid {
        use crate::process::scheduler::PROCESS_TABLE;
        for proc in PROCESS_TABLE.iter().flatten() {
            if !proc.is_free() && proc.pid != pid && proc.fb_surface_phys != 0 {
                // use send_signal_inner — we're already under PROCESS_TABLE_LOCK
                // (called from terminate_process_inner which holds the lock)
                let _ = SCHEDULER.send_signal_inner(proc.pid, crate::process::signals::Signal::SIGKILL);
            }
        }
        COMPOSITOR_PID = 0;
    }
}

/// `SYS_FB_LOCK() → 0`
///
/// Claim exclusive framebuffer access. Other processes should check
/// `fb_is_locked()` before writing to the framebuffer.
/// When a compositor is active, non-compositor processes get a no-op
/// (they have their own private surface — no contention).
pub unsafe fn sys_fb_lock() -> u64 {
    if is_composited_client() {
        return 0; // no-op: they own their private surface
    }
    let pid = SCHEDULER.current_pid();
    if FB_LOCK_PID != 0 && FB_LOCK_PID != pid {
        return EBUSY;
    }
    FB_LOCK_PID = pid;
    0
}

/// `SYS_FB_UNLOCK() → 0`
///
/// Release exclusive framebuffer access. Only the lock holder can unlock.
/// No-op for composited clients.
pub unsafe fn sys_fb_unlock() -> u64 {
    if is_composited_client() {
        return 0;
    }
    let pid = SCHEDULER.current_pid();
    if FB_LOCK_PID != pid && FB_LOCK_PID != 0 {
        return EPERM;
    }
    FB_LOCK_PID = 0;
    0
}

/// `SYS_FB_IS_LOCKED()` — returns holder PID.
/// Composited clients always see 0 (unlocked from their perspective).
pub fn fb_lock_holder() -> u32 {
    unsafe {
        if COMPOSITOR_PID != 0 && SCHEDULER.current_pid() != COMPOSITOR_PID {
            return 0;
        }
        FB_LOCK_PID
    }
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
    use crate::serial::{puts, put_hex32};
    let info = match FB_REGISTERED {
        Some(i) => i,
        None => {
            puts("[FB_MAP] ENODEV: no framebuffer registered\n");
            return ENODEV;
        }
    };

    // Compositor mode: non-compositor processes get their own surface.
    if is_composited_client() {
        puts("[FB_MAP] pid ");
        put_hex32(SCHEDULER.current_pid());
        puts(" → private surface (composited client)\n");
        return sys_fb_map_surface(&info);
    }

    // Compositor or legacy mode: map the real back buffer.
    puts("[FB_MAP] pid ");
    put_hex32(SCHEDULER.current_pid());
    puts(" → real back buffer (comp_pid=");
    put_hex32(COMPOSITOR_PID);
    puts(")\n");
    // Allocate back + shadow buffers on first call.
    if FB_BACK_PHYS == 0 {
        if let Err(e) = fb_map_alloc_buffers(&info) {
            return e;
        }
    }

    // Map back buffer as writable cacheable (flag 0x01 = writable, no 0x02 uncacheable).
    sys_map_phys(FB_BACK_PHYS, FB_BACK_PAGES, 0x01)
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
    let pages = info.size.div_ceil(4096);
    let mut registry = crate::memory::global_registry_mut();
    let phys = match registry.allocate_pages(
        crate::memory::AllocateType::AnyPages,
        crate::memory::MemoryType::Allocated,
        pages,
    ) {
        Ok(p) => p,
        Err(_) => return ENOMEM,
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

    FB_BACK_PHYS = back_phys;
    FB_SHADOW_PHYS = shadow_phys;
    FB_BACK_PAGES = pages;
    Ok(())
}

/// Kernel-internal: push back-buffer delta to VRAM.
///
/// Called from the timer ISR (`scheduler_tick`).  Compares the back buffer
/// against the shadow and writes only changed pixel spans to real VRAM.
/// Transparent to userspace — no syscall required.
///
/// PERF: Gated by `FB_DIRTY` flag.  When nothing has been written to the
/// back buffer since the last present, this function returns immediately
/// without touching any framebuffer memory.  This eliminates ~1.6 GB/s
/// of wasted memory bandwidth at idle (1920×1080 @ 100 Hz).
pub unsafe fn fb_present_tick() {
    if FB_BACK_PHYS == 0 || FB_SHADOW_PHYS == 0 {
        return;
    }
    // If a process holds the FB lock it manages presents itself via
    // SYS_FB_PRESENT — skip the automatic timer-driven present to
    // avoid redundant full-screen scans.
    if FB_LOCK_PID != 0 {
        return;
    }
    // PERF: Skip the full-screen delta scan if nothing was written.
    if !FB_DIRTY {
        return;
    }
    FB_DIRTY = false;

    let info = match FB_REGISTERED {
        Some(i) => i,
        None => return,
    };

    // Buffers are at identity-mapped physical addresses — need kernel CR3.
    let _guard = crate::memory::KernelCr3Guard::enter();

    let stride_pixels = info.stride / 4;

    #[cfg(target_arch = "x86_64")]
    asm_fb_present_delta(
        FB_BACK_PHYS,
        FB_SHADOW_PHYS,
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

    if FB_BACK_PHYS == 0 || FB_SHADOW_PHYS == 0 {
        return ENODEV;
    }
    let info = match FB_REGISTERED {
        Some(i) => i,
        None => return ENODEV,
    };

    let _guard = crate::memory::KernelCr3Guard::enter();

    let stride_pixels = info.stride / 4;

    #[cfg(target_arch = "x86_64")]
    asm_fb_present_delta(
        FB_BACK_PHYS,
        FB_SHADOW_PHYS,
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

    if FB_BACK_PHYS == 0 {
        return ENODEV;
    }
    let info = match FB_REGISTERED {
        Some(i) => i,
        None => return ENODEV,
    };

    let _guard = crate::memory::KernelCr3Guard::enter();

    let bytes = info.size as usize;
    let back = FB_BACK_PHYS as *const u8;
    let vram = info.base as *mut u8;

    core::ptr::copy_nonoverlapping(back, vram, bytes);

    // Keep shadow in sync so delta presents (e.g. after app releases
    // the fb lock) don't see stale data and over-copy.
    if FB_SHADOW_PHYS != 0 {
        let shadow = FB_SHADOW_PHYS as *mut u8;
        core::ptr::copy_nonoverlapping(back, shadow, bytes);
    }

    0
}
