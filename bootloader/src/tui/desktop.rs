//! Kernel main loop — spawn /bin/msh and forward keyboard input.
//!
//! The kernel TUI/WM is gone. After boot, the kernel:
//! 1. Shows the boot log on screen, waits for a keypress
//! 2. Clears the live console hook (gives up the framebuffer)
//! 3. Spawns /bin/msh as the init user process (it owns the framebuffer via SYS_FB_MAP)
//! 4. Loops forever forwarding PS/2 keyboard → stdin ring buffer

use alloc::vec::Vec;
use morpheus_display::types::FramebufferInfo;
use morpheus_hwinit::serial::{clear_live_console_hook, log_error, log_info, log_ok, puts};

use super::input::Keyboard;

/// Load an ELF binary from `/bin/<name>`.
fn load_elf_from_fs(name: &str) -> Option<Vec<u8>> {
    use alloc::string::String;

    let mut path = String::from("/bin/");
    path.push_str(name);

    let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
        Some(f) => f,
        None => {
            log_error("ELF", 920, "no filesystem mounted");
            return None;
        }
    };

    let stat = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, &path) {
        Ok(s) => s,
        Err(e) => {
            let _ = e;
            log_error("ELF", 921, "vfs_stat failed");
            return None;
        }
    };

    if stat.size == 0 {
        log_error("ELF", 922, "ELF file size is zero");
        return None;
    }

    let mut fd_table = morpheus_helix::vfs::FdTable::new();
    let ts = morpheus_hwinit::cpu::tsc::read_tsc();
    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        &mut fd_table,
        &path,
        morpheus_helix::types::open_flags::O_READ,
        ts,
    ) {
        Ok(f) => f,
        Err(e) => {
            let _ = e;
            log_error("ELF", 923, "vfs_open failed");
            return None;
        }
    };

    let mut buf = alloc::vec![0u8; stat.size as usize];
    let n = match morpheus_helix::vfs::vfs_read(
        &mut fs.device,
        &fs.mount_table,
        &mut fd_table,
        fd,
        &mut buf,
    ) {
        Ok(bytes) => bytes,
        Err(e) => {
            let _ = e;
            log_error("ELF", 924, "vfs_read failed");
            return None;
        }
    };

    buf.truncate(n);
    let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);

    let _ = (path, n);
    log_ok("ELF", 925, "ELF image loaded from filesystem");
    Some(buf)
}

fn show_boot_log_screen(keyboard: &mut Keyboard) {
    puts("\n");
    puts("Press any key to launch msh...");
    keyboard.wait_for_key();
    puts("\n");
    clear_live_console_hook();
}

pub fn run_desktop(_display_info: &FramebufferInfo) -> ! {
    log_info("KERNEL", 926, "preparing to launch /bin/init");

    let mut keyboard = Keyboard::new();
    let mut mouse = super::mouse::Mouse::new();
    show_boot_log_screen(&mut keyboard);

    // Load and spawn the userland init supervisor
    let elf_data = match load_elf_from_fs("init") {
        Some(data) => data,
        None => {
            puts("[FATAL] /bin/init not found — cannot start desktop environment\n");
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let _init_pid = match unsafe {
        morpheus_hwinit::process::scheduler::spawn_user_process("init", &elf_data, &[], 0, false)
    } {
        Ok(pid) => {
            let _ = pid;
            log_ok("KERNEL", 927, "init process spawned");
            pid
        }
        Err(e) => {
            let _ = e;
            log_error("KERNEL", 928, "failed to spawn init");
            loop {
                core::hint::spin_loop();
            }
        }
    };

    // Drop the ELF data — init is loaded into its own address space now
    drop(elf_data);

    log_info("KERNEL", 929, "entering input forwarding loop");

    // Main kernel loop: poll PS/2 keyboard + mouse, feed to consumers.
    //
    // DRAIN-ALL: process up to 64 buffered bytes per outer iteration before
    // considering HLT.  Without this, mouse input lags badly when keyboard
    // is active because only one byte was serviced before halting.
    loop {
        let mut had_work = false;

        for _ in 0..64 {
            let raw = unsafe { super::input::asm_ps2_poll_any() };
            if raw == 0 {
                break;
            }
            had_work = true;

            let device = (raw >> 8) & 0xFF;
            let byte = (raw & 0xFF) as u8;

            if device == 0x03 {
                // Mouse byte
                if let Some(pkt) = mouse.feed(byte) {
                    morpheus_hwinit::mouse::accumulate(pkt.dx, pkt.dy, pkt.buttons);
                }
                continue;
            }

            // Keyboard byte — feed through the decoder
            if let Some(input) = keyboard.feed_raw(byte) {
                // Accept any non-zero unicode_char that fits in a u8.
                // This passes ASCII (1-127) AND Latin-1 chars > 127
                // (German umlauts ä=0xE4, ö=0xF6, ü=0xFC, ß=0xDF, etc.)
                if input.unicode_char > 0 && input.unicode_char <= 0xFF {
                    let ch = input.unicode_char as u8;

                    if ch == 0x03 {
                        // Ctrl+C → SIGINT to foreground process
                        let fg = morpheus_hwinit::stdin::foreground_pid();
                        if fg != 0 {
                            unsafe {
                                let _ = morpheus_hwinit::process::SCHEDULER.send_signal(
                                    fg,
                                    morpheus_hwinit::process::signals::Signal::SIGINT,
                                );
                            }
                        } else {
                            morpheus_hwinit::stdin::push(ch);
                            unsafe {
                                morpheus_hwinit::process::wake_stdin_waiters();
                            }
                        }
                    } else {
                        morpheus_hwinit::stdin::push(ch);
                        unsafe {
                            morpheus_hwinit::process::wake_stdin_waiters();
                        }
                    }
                }
            }
        }

        if !had_work {
            // Nothing available — halt CPU until next interrupt.
            morpheus_hwinit::process::scheduler::mark_kernel_hlt();
            unsafe {
                core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            }
        }
    }
}
