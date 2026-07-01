//! Late-init phases 10-12: scheduler, syscalls, LAPIC timer takeover, HelixFS root.
//!
//! Bootloader calls [`init`] after `platform_init_selfcontained` returns.
//! AP bring-up + Phase 10.5 BootServices reclaim stay bootloader-side per LD16
//! until the matching HAL trait methods land.

use morpheus_hal_api::{Hal, IsrFn, KernelHooks};

pub struct InitParams {
    /// Vector 0x20 ISR (`irq_timer_isr` in `context_switch.s`).
    pub timer_isr: IsrFn,
    /// Allocated by [`mount_root_fs`], called by the bootloader AFTER reclaim.
    pub root_fs_size: usize,
    /// Seeds BSP's `kernel_syscall_rsp` slot for the SYSCALL entry's ring 3→0 stack switch.
    pub kernel_stack_top: u64,
}

/// # Safety
/// - Exactly once on the BSP, after `platform_init_selfcontained` returns.
/// - HAL installed via [`crate::install_hal`].
/// - Enter with IF=0; returns with IF=1.
/// - Bootloader must have already called `hal.intr().disable_legacy_pic()`.
///
/// Bootloader call order after this returns:
///   1. `hal.phys().reclaim_boot_services()` (phase 10.5)
///   2. [`mount_root_fs`] (phase 11b)
///
/// Reclaim MUST run before mount_root_fs: the 16 MiB helixfs alloc carves the
/// buddy heavily, and reclaim's post-add `validate_free_lists` walk has
/// exposed real-hardware faults (canonical-but-unmapped `next` pointers) when
/// run against a buddy that's already been hammered.
pub unsafe fn init(hal: &'static dyn Hal, params: InitParams) {
    // ----- Phase 10: scheduler -----
    crate::schedular::init_scheduler();
    crate::schedular::set_tsc_frequency(hal.timer().tsc_frequency());
    hal.serial().boot_step_ok("scheduler");

    // ----- Phase 11a: syscall MSRs + LAPIC periodic timer takeover -----
    crate::syscall::init_syscall();

    // 100 Hz preemption, calibrated against TSC inside the HAL.
    hal.timer().setup_periodic(100);

    // Vector 0x20 — legacy number, LAPIC-sourced.
    hal.intr().set_handler(0x20, params.timer_isr, 0, 0);

    hal.cpu().enable_interrupts();

    // PID 0's SYSCALL entry reads kernel_syscall_rsp from the per-CPU block.
    hal.smp()
        .pcpu_set_kernel_syscall_rsp(params.kernel_stack_top);
    hal.serial().boot_step_ok("syscalls");

    let _ = params.root_fs_size;
}

/// Phase 11b: bootloader calls this AFTER `hal.phys().reclaim_boot_services()`.
///
/// Root mounting moved into the storage subsystem (`bootloader::storage`): root
/// is itself a staged `storage::mount` (spec §5/§7) chosen there from the pre-EBS
/// image, a real disk holding `/bin/init`, or a fresh RAM helix. The old
/// `init_root_fs` RAM-root allocation is now the `VOLUME_NONE` fresh-helix path,
/// allocated through the staging admission control. Kept as a no-op hook so the
/// bootloader call site and phase ordering stay intact.
///
/// # Safety
/// BSP, post-late_init, post-reclaim.
pub unsafe fn mount_root_fs(hal: &'static dyn Hal, size_bytes: usize) {
    let _ = (hal, size_bytes);
}

/// Hook bundle for a future `HalImpl::init(KernelHooks)` entry. Unset fields
/// stay `None`; the HAL must tolerate missing hooks.
pub fn build_kernel_hooks() -> KernelHooks {
    KernelHooks {
        // LAPIC ISR currently calls `scheduler_tick` by its `#[no_mangle]` symbol;
        // reserved for the HAL-driven IDT path.
        scheduler_tick: None,
        current_pid: None,
        process_lookup: None,
        process_exit: None,
        kernel_cr3: Some(kernel_cr3_hook),
        keyboard_sink: Some(crate::input::hid_keyboard_sink),
        mouse_sink: Some(crate::input::hid_mouse_sink),
    }
}

/// Kernel CR3 accessor for `KernelCr3Guard`; bootloader installs it into the
/// HAL after `init` (kernel can't call the arch HAL directly — portability gate).
///
/// # Safety
/// Returns 0 until `init_scheduler` sets the kernel CR3; callers tolerate 0.
pub unsafe fn kernel_cr3_hook() -> u64 {
    crate::schedular::get_kernel_cr3()
}
