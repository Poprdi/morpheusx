//! Kernel main loop — spawn /bin/msh and forward keyboard input.
//!
//! The kernel TUI/WM is gone. After boot, the kernel:
//! 1. Shows the boot log on screen, waits for a keypress
//! 2. Clears the live console hook (gives up the framebuffer)
//! 3. Spawns /bin/msh as the init user process (it owns the framebuffer via SYS_FB_MAP)
//! 4. Loops forever forwarding PS/2 keyboard → stdin ring buffer

use alloc::format;
use alloc::vec::Vec;
use morpheus_display::types::FramebufferInfo;
use morpheus_hwinit::serial::{clear_live_console_hook, puts};

use super::input::Keyboard;

/// Load an ELF binary from `/bin/<name>`.
fn load_elf_from_fs(name: &str) -> Option<Vec<u8>> {
    use alloc::string::String;

    let mut path = String::from("/bin/");
    path.push_str(name);

    let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
        Some(f) => f,
        None => {
            puts("[LOAD-ELF] FAIL: no filesystem\n");
            return None;
        }
    };

    let stat = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, &path) {
        Ok(s) => s,
        Err(e) => {
            puts("[LOAD-ELF] FAIL: vfs_stat ");
            puts(&path);
            puts(": ");
            puts(match e {
                morpheus_helix::error::HelixError::NotFound => "NotFound",
                morpheus_helix::error::HelixError::MountNotFound => "MountNotFound",
                _ => "other",
            });
            puts("\n");
            return None;
        }
    };

    if stat.size == 0 {
        puts("[LOAD-ELF] FAIL: size is 0\n");
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
            puts("[LOAD-ELF] FAIL: vfs_open: ");
            puts(&format!("{:?}", e));
            puts("\n");
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
            puts("[LOAD-ELF] FAIL: vfs_read: ");
            puts(&format!("{:?}", e));
            puts("\n");
            return None;
        }
    };

    buf.truncate(n);
    let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);

    puts("[LOAD-ELF] loaded ");
    puts(&path);
    puts(" (");
    morpheus_hwinit::serial::put_hex32(n as u32);
    puts(" bytes)\n");
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
    puts("[KERNEL] preparing to launch /bin/msh\n");

    let mut keyboard = Keyboard::new();
    show_boot_log_screen(&mut keyboard);

    // Load and spawn the userland shell
    let elf_data = match load_elf_from_fs("msh") {
        Some(data) => data,
        None => {
            puts("[FATAL] /bin/msh not found — cannot start shell\n");
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let _msh_pid = match unsafe {
        morpheus_hwinit::process::scheduler::spawn_user_process("msh", &elf_data, &[], 0, false)
    } {
        Ok(pid) => {
            puts("[KERNEL] msh spawned as PID ");
            morpheus_hwinit::serial::put_hex32(pid);
            puts("\n");
            pid
        }
        Err(e) => {
            puts("[FATAL] failed to spawn msh: ");
            puts(e);
            puts("\n");
            loop {
                core::hint::spin_loop();
            }
        }
    };

    // Drop the ELF data — msh is loaded into its own address space now
    drop(elf_data);

    puts("[KERNEL] entering input forwarding loop\n");

    let mut mouse = super::mouse::Mouse::new();

    // Main kernel loop: poll PS/2 keyboard + mouse, feed to consumers.
    loop {
        let raw = unsafe { super::input::asm_ps2_poll_any() };
        if raw == 0 {
            // Nothing available — halt CPU until next interrupt (timer/keyboard/mouse).
            // Record the TSC at this moment so the scheduler can split our quantum
            // into active work time (before this point) and HLT idle time (after).
            // This gives sysvis accurate absolute CPU% instead of a relative share.
            morpheus_hwinit::process::scheduler::mark_kernel_hlt();
            unsafe {
                core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            }
            continue;
        }

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
            if input.unicode_char > 0 && input.unicode_char < 128 {
                let ch = input.unicode_char as u8;

                if ch == 0x03 {
                    let fg = morpheus_hwinit::stdin::foreground_pid();
                    if fg != 0 {
                        unsafe {
                            let _ = morpheus_hwinit::process::SCHEDULER
                                .send_signal(fg, morpheus_hwinit::process::signals::Signal::SIGINT);
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
}
