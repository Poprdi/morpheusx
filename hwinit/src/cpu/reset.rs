//! Reset helpers for machine restart paths.

#[inline(always)]
fn shutdown_stage(msg: &str) {
    crate::serial::fb_puts(msg);
    crate::serial::fb_puts("\n");
    crate::serial::checkpoint(msg);
}

#[inline(always)]
unsafe fn io_wait() {
    crate::cpu::pio::outb(0x80, 0);
}

#[inline(always)]
unsafe fn kbc_wait_input_empty() {
    for _ in 0..100_000 {
        if (crate::cpu::pio::inb(0x64) & 0x02) == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

#[inline(always)]
unsafe fn reset_via_cf9() {
    crate::cpu::pio::outb(0xCF9, 0x02);
    io_wait();
    crate::cpu::pio::outb(0xCF9, 0x06);
}

#[inline(always)]
unsafe fn reset_via_port92() {
    let mut v = crate::cpu::pio::inb(0x92);
    v &= !0x01;
    crate::cpu::pio::outb(0x92, v);
    io_wait();
    crate::cpu::pio::outb(0x92, v | 0x01);
}

#[inline(always)]
unsafe fn reset_via_8042() {
    kbc_wait_input_empty();
    crate::cpu::pio::outb(0x64, 0xFE);
}

#[inline(always)]
unsafe fn triple_fault_reset() -> ! {
    #[repr(C, packed)]
    struct Idtr {
        limit: u16,
        base: u64,
    }
    let idtr = Idtr { limit: 0, base: 0 };
    core::arch::asm!("lidt [{}]", in(reg) &idtr, options(readonly, nostack));
    core::arch::asm!("ud2", options(noreturn));
}

pub unsafe fn reset_machine_now() -> ! {
    let core_idx = crate::cpu::per_cpu::current_core_index();
    if crate::cpu::per_cpu::reboot_owner().is_none() {
        crate::cpu::per_cpu::set_reboot_owner(core_idx);
    }
    if let Some(owner) = crate::cpu::per_cpu::reboot_owner() {
        if owner != core_idx {
            shutdown_stage("shutdown: non-owner reset attempt blocked");
            crate::cpu::idt::disable_interrupts();
            loop {
                core::arch::asm!("hlt", options(nostack, nomem));
            }
        }
    }

    crate::serial::set_checkpoints_enabled(true);
    shutdown_stage("shutdown: request ap quiesce");

    // Ask APs to park before we start poking reset controls.
    crate::cpu::per_cpu::request_shutdown_quiesce();
    let quiesced = crate::cpu::per_cpu::wait_for_shutdown_quiesce(500);
    if quiesced {
        shutdown_stage("shutdown: ap quiesce complete");
    } else {
        shutdown_stage("shutdown: ap quiesce timeout; forcing reset");
    }

    crate::cpu::idt::disable_interrupts();

    shutdown_stage("shutdown: reset via cf9");
    reset_via_cf9();
    for _ in 0..64 {
        io_wait();
    }

    shutdown_stage("shutdown: reset via port92");
    reset_via_port92();
    for _ in 0..64 {
        io_wait();
    }

    shutdown_stage("shutdown: reset via 8042");
    reset_via_8042();
    for _ in 0..64 {
        io_wait();
    }

    shutdown_stage("shutdown: reset via triple-fault");
    triple_fault_reset()
}

pub unsafe fn wait_for_keypress_or_timeout_ms(timeout_ms: u64) {
    let tsc_hz = crate::process::scheduler::tsc_frequency();
    let deadline = if tsc_hz > 0 {
        let ticks_per_ms = tsc_hz / 1000;
        Some(
            crate::cpu::tsc::read_tsc()
                .saturating_add(timeout_ms.saturating_mul(ticks_per_ms.max(1))),
        )
    } else {
        None
    };

    loop {
        let status = crate::cpu::pio::inb(0x64);
        if (status & 0x01) != 0 {
            let _ = crate::cpu::pio::inb(0x60);
            break;
        }

        if let Some(d) = deadline {
            if crate::cpu::tsc::read_tsc() >= d {
                break;
            }
        }

        core::hint::spin_loop();
    }
}
