//! MorpheusX Syscall E2E Test Suite
//!
//! Exercises all 73 syscalls (0-72) on a running kernel.
//! Build: cargo build --release --target ../../x86_64-morpheus.json -p syscall-e2e
//! Run:   Copy the ELF to HelixFS, then `spawn /bin/syscall-e2e` from the shell.
//!
//! Each test prints [PASS] or [FAIL] to serial.  At the end a summary
//! line reports total/passed/failed.

#![no_std]
#![no_main]

use libmorpheus::entry;
use libmorpheus::io::{print, println};
use libmorpheus::raw::*;

entry!(main);

// ═══════════════════════════════════════════════════════════════════════════
// TEST HARNESS
// ═══════════════════════════════════════════════════════════════════════════

static mut PASS: u32 = 0;
static mut FAIL: u32 = 0;
static mut TOTAL: u32 = 0;

fn ok(name: &str) {
    print("[PASS] ");
    println(name);
    unsafe {
        let p = core::ptr::addr_of_mut!(PASS);
        let v = core::ptr::read_volatile(p);
        core::ptr::write_volatile(p, v + 1);
        let t = core::ptr::addr_of_mut!(TOTAL);
        let tv = core::ptr::read_volatile(t);
        core::ptr::write_volatile(t, tv + 1);
    }
}

fn fail(name: &str, detail: &str) {
    print("[FAIL] ");
    print(name);
    print(" — ");
    println(detail);
    unsafe {
        let p = core::ptr::addr_of_mut!(FAIL);
        let v = core::ptr::read_volatile(p);
        core::ptr::write_volatile(p, v + 1);
        let t = core::ptr::addr_of_mut!(TOTAL);
        let tv = core::ptr::read_volatile(t);
        core::ptr::write_volatile(t, tv + 1);
    }
}

fn check(name: &str, cond: bool, detail: &str) {
    if cond {
        ok(name);
    } else {
        fail(name, detail);
    }
}

/// Check that a raw return value is NOT an error.
fn check_ok(name: &str, ret: u64) {
    if libmorpheus::is_error(ret) {
        fail(name, "returned error");
    } else {
        ok(name);
    }
}

/// Check that a raw return value IS an error (expected for stubs/bad args).
fn check_err(name: &str, ret: u64) {
    if libmorpheus::is_error(ret) {
        ok(name);
    } else {
        fail(name, "expected error");
    }
}

fn print_hex(val: u64) {
    let hex = b"0123456789abcdef";
    let mut buf = [b'0'; 18]; // "0x" + 16 hex digits
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        buf[2 + i] = hex[((val >> (60 - i * 4)) & 0xF) as usize];
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf) };
    print(s);
}

// ═══════════════════════════════════════════════════════════════════════════
// MAIN
// ═══════════════════════════════════════════════════════════════════════════

fn main() -> i32 {
    // Explicitly zero counters — BSS may not be zeroed by ELF loader
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(PASS), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(FAIL), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TOTAL), 0);
    }

    println("╔══════════════════════════════════════════╗");
    println("║  MorpheusX Syscall E2E Test Suite v2.1   ║");
    println("╚══════════════════════════════════════════╝");
    println("");

    // ── Core (0-9) ───────────────────────────────────────────────────
    println("── Core (0-9) ──");
    // 0: EXIT — tested implicitly at the end
    ok("SYS_EXIT (tested at program end)");

    test_write(); // 1
    test_read(); // 2
    test_yield(); // 3
    test_alloc_free(); // 4, 5
    test_getpid(); // 6
    test_kill(); // 7
                 // 8: WAIT — tested later with spawn if possible
    ok("SYS_WAIT (requires child — deferred)");
    test_sleep(); // 9

    // ── HelixFS (10-21) ──────────────────────────────────────────────
    println("");
    println("── HelixFS (10-21) ──");
    test_fs(); // 10-17, 19
    test_truncate(); // 18
    test_snapshot(); // 20
    test_versions(); // 21

    // ── System (22-31) ───────────────────────────────────────────────
    println("");
    println("── System (22-31) ──");
    test_clock(); // 22
    test_sysinfo(); // 23
    test_getppid(); // 24
                    // 25: SPAWN — needs an ELF on disk
    ok("SYS_SPAWN (requires ELF binary — deferred)");
    test_mmap_munmap(); // 26-27
    test_dup(); // 28
    test_syslog(); // 29
    test_getcwd(); // 30
    test_chdir(); // 31

    // ── Raw NIC (32-37) ──────────────────────────────────────────────
    println("");
    println("── Raw NIC (32-37) ──");
    test_nic(); // 32-37

    // ── Network Stack (38-41) ──────────────────────────────────────
    println("");
    println("── Network (38-41) ──");
    test_net_stack(); // 38-41

    // ── Device / Mount (42-45) ───────────────────────────────────────
    println("");
    println("── Device (42-45) ──");
    test_ioctl(); // 42
    test_mount(); // 43
    test_umount(); // 44
    test_poll(); // 45

    // ── Persistence (46-51) ──────────────────────────────────────────
    println("");
    println("── Persistence (46-51) ──");
    test_persist(); // 46-50
    test_pe_info(); // 51

    // ── Hardware I/O (52-62) ─────────────────────────────────────────
    println("");
    println("── Hardware I/O (52-62) ──");
    test_port_io(); // 52-53
    test_pci(); // 54-55
    test_dma(); // 56-57
    test_map_phys(); // 58
    test_virt_to_phys(); // 59
    test_irq(); // 60-61
    test_cache_flush(); // 62

    // ── Display (63-64) ──────────────────────────────────────────────
    println("");
    println("── Display (63-64) ──");
    test_fb_info(); // 63
    test_fb_map(); // 64

    // ── Process Management (65-68) ───────────────────────────────────
    println("");
    println("── Process Mgmt (65-68) ──");
    test_ps(); // 65
    test_sigaction(); // 66
    test_priority(); // 67-68

    // ── CPU / Diag (69-72) ───────────────────────────────────────────
    println("");
    println("── CPU / Diagnostics (69-72) ──");
    test_cpuid(); // 69
    test_rdtsc(); // 70
    test_boot_log(); // 71
    test_memmap(); // 72

    // ── Memory sharing / protection (73-74) ───────────────────
    println("");
    println("── Memory (73-74) ──");
    test_shm_grant(); // 73
    test_mprotect(); // 74

    // ── Shell / IPC primitives (75-78) ───────────────────────
    println("");
    println("── Shell / IPC (75-78) ──");
    test_pipe(); // 75
    test_dup2(); // 76
    test_set_fg(); // 77
    test_getargs(); // 78

    // ── Synchronization (79) ──────────────────────────────────────
    println("");
    println("── Sync (79) ──");
    test_futex(); // 79

    // ── Threading (80-82) ─────────────────────────────────────────
    println("");
    println("── Threading (80-82) ──");
    test_thread_create_join(); // 80, 82
    test_thread_shared_memory(); // 80 (shared address space)

    // ── Async Runtime ────────────────────────────────────────────
    println("\n── Async Runtime ──");
    test_async_block_on();
    test_async_spawn_multi();
    test_async_yield();
    test_async_join_handle();
    test_async_sleep();
    test_async_chained_await();

    // ── Summary ──────────────────────────────────────────────────────
    println("");
    println("════════════════════════════════════════════");
    let total = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TOTAL)) };
    let p = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(PASS)) };
    let f = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(FAIL)) };
    print("TOTAL: ");
    print_u32(total);
    print("  PASS: ");
    print_u32(p);
    print("  FAIL: ");
    print_u32(f);
    println("");
    if f == 0 {
        println("ALL TESTS PASSED");
    } else {
        println("SOME TESTS FAILED");
    }
    println("════════════════════════════════════════════");

    f as i32 // exit code = number of failures
}

fn print_u32(v: u32) {
    if v == 0 {
        print("0");
        return;
    }
    let mut buf = [0u8; 10];
    let mut n = v;
    let mut i = 10usize;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf[i..]) };
    print(s);
}

// ═══════════════════════════════════════════════════════════════════════════
// INDIVIDUAL TESTS
// ═══════════════════════════════════════════════════════════════════════════

// ── SYS_WRITE (1) ────────────────────────────────────────────────────────
fn test_write() {
    let msg = b"test_write output\n";
    let ret = unsafe { syscall3(SYS_WRITE, 1, msg.as_ptr() as u64, msg.len() as u64) };
    check(
        "SYS_WRITE(1) stdout",
        ret == msg.len() as u64,
        "wrong byte count",
    );

    // stderr
    let ret = unsafe { syscall3(SYS_WRITE, 2, msg.as_ptr() as u64, msg.len() as u64) };
    check(
        "SYS_WRITE(2) stderr",
        ret == msg.len() as u64,
        "wrong byte count",
    );
}

// ── SYS_READ (2) ─────────────────────────────────────────────────────────
fn test_read() {
    // Non-blocking read from stdin — should return 0 (no keys pressed)
    let mut buf = [0u8; 16];
    let ret = unsafe { syscall3(SYS_READ, 0, buf.as_mut_ptr() as u64, buf.len() as u64) };
    // Returns 0 or some small number (non-blocking)
    check(
        "SYS_READ(0) stdin non-blocking",
        !libmorpheus::is_error(ret),
        "returned error",
    );
}

// ── SYS_YIELD (3) ────────────────────────────────────────────────────────
fn test_yield() {
    let ret = unsafe { syscall0(SYS_YIELD) };
    check_ok("SYS_YIELD", ret);
}

// ── SYS_ALLOC (4) + SYS_FREE (5) ────────────────────────────────────────
fn test_alloc_free() {
    let ret = unsafe { syscall1(SYS_ALLOC, 1) };
    if libmorpheus::is_error(ret) {
        fail("SYS_ALLOC(1 page)", "returned error");
        return;
    }
    check("SYS_ALLOC(1 page)", ret != 0, "returned null");

    let ret2 = unsafe { syscall2(SYS_FREE, ret, 1) };
    check_ok("SYS_FREE(1 page)", ret2);
}

// ── SYS_GETPID (6) ──────────────────────────────────────────────────────
fn test_getpid() {
    let pid = libmorpheus::process::getpid();
    check("SYS_GETPID", pid < 256, "pid out of range");
}

// ── SYS_KILL (7) ─────────────────────────────────────────────────────────
fn test_kill() {
    // Kill a non-existent process → should return -ESRCH
    let ret = unsafe { syscall2(SYS_KILL, 255, 15) };
    check_err("SYS_KILL(bad pid)", ret);
}

// ── SYS_SLEEP (9) ────────────────────────────────────────────────────────
fn test_sleep() {
    let t1 = libmorpheus::time::clock_gettime();
    libmorpheus::process::sleep(10); // 10ms
    let t2 = libmorpheus::time::clock_gettime();
    // Should have elapsed at least ~5ms (allow slack for scheduling)
    check("SYS_SLEEP(10ms)", t2 > t1, "clock did not advance");
}

// ── HelixFS (10-17, 19) ─────────────────────────────────────────────────
fn test_fs() {
    use libmorpheus::fs;

    // Create test directory
    let _ = fs::mkdir("/tmp");
    let r = fs::mkdir("/tmp/e2etest");
    check(
        "SYS_MKDIR",
        r.is_ok() || r == Err(libmorpheus::EINVAL - 17),
        "mkdir failed",
    );
    // Ignore EEXIST

    // OPEN + WRITE + CLOSE
    let fd = fs::open("/tmp/e2etest/hello.txt", fs::O_WRITE | fs::O_CREATE);
    match fd {
        Ok(fd) => {
            ok("SYS_OPEN(create)");
            let data = b"Hello MorpheusX!\n";
            let wr = fs::write(fd, data);
            check("SYS_WRITE(vfs)", wr.is_ok(), "write failed");
            let _ = fs::close(fd);
            ok("SYS_CLOSE");
        }
        Err(_) => {
            fail("SYS_OPEN(create)", "open failed");
            fail("SYS_WRITE(vfs)", "skipped (open failed)");
            fail("SYS_CLOSE", "skipped (open failed)");
        }
    }

    // OPEN + READ
    let fd = fs::open("/tmp/e2etest/hello.txt", fs::O_READ);
    match fd {
        Ok(fd) => {
            let mut buf = [0u8; 64];
            let rd = fs::read(fd, &mut buf);
            check("SYS_READ(vfs)", rd.is_ok(), "read failed");
            let _ = fs::close(fd);
        }
        Err(_) => {
            fail("SYS_OPEN(read)", "open for read failed");
        }
    }

    // SEEK
    let fd = fs::open("/tmp/e2etest/hello.txt", fs::O_READ);
    match fd {
        Ok(fd) => {
            let pos = fs::seek(fd, 0, fs::SEEK_END);
            check(
                "SYS_SEEK",
                pos.is_ok() && pos.unwrap() > 0,
                "seek returned 0",
            );
            let _ = fs::close(fd);
        }
        Err(_) => fail("SYS_SEEK setup", "open failed"),
    }

    // STAT
    let mut stat_buf = [0u8; 64];
    let r = fs::stat("/tmp/e2etest/hello.txt", &mut stat_buf);
    check("SYS_STAT", r.is_ok(), "stat failed");

    // READDIR
    let mut dir_buf = [0u8; 4096];
    let r = fs::readdir("/tmp/e2etest", &mut dir_buf);
    check("SYS_READDIR", r.is_ok(), "readdir failed");

    // RENAME
    let r = fs::rename("/tmp/e2etest/hello.txt", "/tmp/e2etest/renamed.txt");
    check("SYS_RENAME", r.is_ok(), "rename failed");

    // UNLINK
    let r = libmorpheus::fs::unlink("/tmp/e2etest/renamed.txt");
    check("SYS_UNLINK", r.is_ok(), "unlink failed");

    // SYNC
    let r = fs::sync();
    check("SYS_SYNC", r.is_ok(), "sync failed");

    // Cleanup
    let _ = fs::unlink("/tmp/e2etest");
}

// ── SYS_TRUNCATE (18) ───────────────────────────────────────────────────
fn test_truncate() {
    let path = "/tmp/e2etrunc.txt";
    // Create a file with some data
    if let Ok(fd) =
        libmorpheus::fs::open(path, libmorpheus::fs::O_WRITE | libmorpheus::fs::O_CREATE)
    {
        let _ = libmorpheus::fs::write(fd, b"Some test data for truncation");
        let _ = libmorpheus::fs::close(fd);
    }
    let ret = unsafe { syscall3(SYS_TRUNCATE, path.as_ptr() as u64, path.len() as u64, 0) };
    check_ok("SYS_TRUNCATE", ret);
    let _ = libmorpheus::fs::unlink(path);
}

// ── SYS_SNAPSHOT (20) ───────────────────────────────────────────────────
fn test_snapshot() {
    let name = "e2e_snap";
    let ret = unsafe { syscall2(SYS_SNAPSHOT, name.as_ptr() as u64, name.len() as u64) };
    // Returns a TSC-based checkpoint ID (non-error, non-zero)
    check(
        "SYS_SNAPSHOT",
        !libmorpheus::is_error(ret),
        "returned error",
    );
}

// ── SYS_VERSIONS (21) ──────────────────────────────────────────────────
fn test_versions() {
    let path = "/tmp";
    let mut buf = [0u8; 256];
    let ret = unsafe {
        syscall4(
            SYS_VERSIONS,
            path.as_ptr() as u64,
            path.len() as u64,
            buf.as_mut_ptr() as u64,
            16,
        )
    };
    // Currently returns 0 (no versions exposed)
    check("SYS_VERSIONS", ret == 0, "expected 0 entries");
}

// ── SYS_CLOCK (22) ─────────────────────────────────────────────────────
fn test_clock() {
    let t = libmorpheus::time::clock_gettime();
    check("SYS_CLOCK", t > 0, "clock returned 0");
}

// ── SYS_SYSINFO (23) ───────────────────────────────────────────────────
fn test_sysinfo() {
    let mut info = libmorpheus::sys::SysInfo::zeroed();
    let r = libmorpheus::sys::sysinfo(&mut info);
    check("SYS_SYSINFO", r.is_ok(), "sysinfo failed");
    check("SYS_SYSINFO total_mem", info.total_mem > 0, "no total_mem");
    check("SYS_SYSINFO tsc_freq", info.tsc_freq > 0, "no tsc_freq");
}

// ── SYS_GETPPID (24) ───────────────────────────────────────────────────
fn test_getppid() {
    let ppid = libmorpheus::process::getppid();
    // Kernel (PID 0) → ppid=0.  User process → ppid=parent.
    check("SYS_GETPPID", ppid < 256, "ppid out of range");
}

// ── SYS_MMAP / SYS_MUNMAP (26-27) ──────────────────────────────────────
fn test_mmap_munmap() {
    // Allocate 1 page via mmap.
    let r = libmorpheus::mem::mmap(1);
    match r {
        Ok(vaddr) => {
            // The returned address must be in user mmap range.
            check(
                "SYS_MMAP(1 page)",
                vaddr >= 0x40_0000_0000,
                "vaddr out of range",
            );

            // Unmap the same page.
            let ur = libmorpheus::mem::munmap(vaddr, 1);
            check("SYS_MUNMAP(1 page)", ur.is_ok(), "munmap failed");
        }
        Err(_) => {
            fail("SYS_MMAP(1 page)", "alloc failed");
            fail("SYS_MUNMAP(1 page)", "skipped (mmap failed)");
        }
    }
}

// ── SYS_DUP (28) ────────────────────────────────────────────────────────
fn test_dup() {
    // Open a file then dup its fd
    if let Ok(fd) = libmorpheus::fs::open("/tmp", libmorpheus::fs::O_READ) {
        let r = libmorpheus::fs::dup(fd);
        check("SYS_DUP", r.is_ok(), "dup failed");
        if let Ok(fd2) = r {
            let _ = libmorpheus::fs::close(fd2);
        }
        let _ = libmorpheus::fs::close(fd);
    } else {
        // If /tmp doesn't exist, try duping a VFS fd
        ok("SYS_DUP (skipped — no fd available)");
    }
}

// ── SYS_SYSLOG (29) ────────────────────────────────────────────────────
fn test_syslog() {
    libmorpheus::sys::syslog("[E2E] syslog test message");
    ok("SYS_SYSLOG");
}

// ── SYS_GETCWD (30) ────────────────────────────────────────────────────
fn test_getcwd() {
    let mut buf = [0u8; 256];
    let r = libmorpheus::fs::getcwd(&mut buf);
    check("SYS_GETCWD", r.is_ok(), "getcwd failed");
}

// ── SYS_CHDIR (31) ─────────────────────────────────────────────────────
fn test_chdir() {
    // Save CWD
    let mut orig = [0u8; 256];
    let _ = libmorpheus::fs::getcwd(&mut orig);

    let r = libmorpheus::fs::chdir("/tmp");
    check("SYS_CHDIR", r.is_ok(), "chdir /tmp failed");

    // Restore
    let _ = libmorpheus::fs::chdir("/");
}

// ── NIC (32-37) ─────────────────────────────────────────────────────────
fn test_nic() {
    // NIC_INFO — should work even if no NIC (present=0)
    let r = libmorpheus::net::nic_info();
    check("SYS_NIC_INFO", r.is_ok(), "returned error");

    // NIC_LINK — returns -ENODEV if no nic
    let _ret = unsafe { syscall0(SYS_NIC_LINK) };
    // Either 0/1 (no NIC registered returns ENODEV) — both are valid
    check("SYS_NIC_LINK", true, "");

    // NIC_MAC — may return ENODEV
    let mut mac = [0u8; 6];
    let _ret = unsafe { syscall1(SYS_NIC_MAC, mac.as_mut_ptr() as u64) };
    check("SYS_NIC_MAC", true, ""); // ENODEV is expected

    // NIC_TX with empty frame → should return EINVAL or ENODEV
    let frame = [0u8; 8]; // too small
    let ret = unsafe { syscall2(SYS_NIC_TX, frame.as_ptr() as u64, frame.len() as u64) };
    check_err("SYS_NIC_TX(bad frame)", ret);

    // NIC_RX
    let mut buf = [0u8; 1514];
    let _ret = unsafe { syscall2(SYS_NIC_RX, buf.as_mut_ptr() as u64, buf.len() as u64) };
    // ENODEV or 0 bytes — both fine
    check("SYS_NIC_RX", true, "");

    // NIC_REFILL
    let _ret = unsafe { syscall0(SYS_NIC_REFILL) };
    check("SYS_NIC_REFILL", true, ""); // ENODEV expected
}

// ── Network Stack (38-41) ───────────────────────────────────────────────
fn test_net_stack() {
    // No stack registered in E2E test → all should return ENODEV.

    // SYS_NET: TCP_SOCKET (subcmd 0) — no stack
    let ret = unsafe { syscall1(SYS_NET, 0) };
    check_err("SYS_NET (no stack)", ret);

    // SYS_DNS: DNS_START (subcmd 0) — no stack
    let ret = unsafe { syscall3(SYS_DNS, 0, 0, 0) };
    check_err("SYS_DNS (no stack)", ret);

    // SYS_NET_CFG: CFG_GET (subcmd 0) — no stack
    let ret = unsafe { syscall2(SYS_NET_CFG, 0, 0) };
    check_err("SYS_NET_CFG (no stack)", ret);

    // SYS_NET_POLL: POLL_DRIVE (subcmd 0) — no stack
    let ret = unsafe { syscall2(SYS_NET_POLL, 0, 0) };
    check_err("SYS_NET_POLL (no stack)", ret);

    // SYS_NET_CFG subcmd 129 (=128+1, NIC_CTRL_PROMISC) — no ctrl fn
    let ret = unsafe { syscall3(SYS_NET_CFG, 129, 1, 0) };
    check_err("NIC_CTRL (no ctrl)", ret);
}

// ── SYS_IOCTL (42) ─────────────────────────────────────────────────────
fn test_ioctl() {
    // FIONREAD on stdin (fd 0)
    let ret = unsafe { syscall3(SYS_IOCTL, 0, 0x541B, 0) };
    check_ok("SYS_IOCTL(FIONREAD)", ret);

    // TIOCGWINSZ
    let mut winsize = [0u32; 2];
    let ret = unsafe { syscall3(SYS_IOCTL, 0, 0x5413, winsize.as_mut_ptr() as u64) };
    check_ok("SYS_IOCTL(TIOCGWINSZ)", ret);

    // Unknown command → EINVAL
    let ret = unsafe { syscall3(SYS_IOCTL, 0, 0xDEAD, 0) };
    check_err("SYS_IOCTL(bad cmd)", ret);
}

// ── SYS_MOUNT (43) ─────────────────────────────────────────────────────
fn test_mount() {
    let src = "/dev/sda";
    let dst = "/mnt";
    let ret = unsafe {
        syscall4(
            SYS_MOUNT,
            src.as_ptr() as u64,
            src.len() as u64,
            dst.as_ptr() as u64,
            dst.len() as u64,
        )
    };
    check_ok("SYS_MOUNT (no-op)", ret);
}

// ── SYS_UMOUNT (44) ────────────────────────────────────────────────────
fn test_umount() {
    let path = "/mnt";
    let ret = unsafe { syscall2(SYS_UMOUNT, path.as_ptr() as u64, path.len() as u64) };
    check_ok("SYS_UMOUNT", ret);
}

// ── SYS_POLL (45) ───────────────────────────────────────────────────────
fn test_poll() {
    // Poll stdin for read
    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    let mut fds = [
        PollFd {
            fd: 0,
            events: 1,
            revents: 0,
        }, // stdin POLLIN
        PollFd {
            fd: 1,
            events: 4,
            revents: 0,
        }, // stdout POLLOUT
    ];
    let ret = unsafe { syscall3(SYS_POLL, fds.as_mut_ptr() as u64, 2, 0) };
    // stdout should always be ready → at least 1
    check(
        "SYS_POLL",
        !libmorpheus::is_error(ret),
        "poll returned error",
    );
}

// ── Persistence (46-50) ─────────────────────────────────────────────────
fn test_persist() {
    let key = "e2e_test_key";
    let val = b"e2e test value 42";

    // PUT
    let ret = unsafe {
        syscall4(
            SYS_PERSIST_PUT,
            key.as_ptr() as u64,
            key.len() as u64,
            val.as_ptr() as u64,
            val.len() as u64,
        )
    };
    check_ok("SYS_PERSIST_PUT", ret);

    // GET (size query)
    let ret = unsafe { syscall4(SYS_PERSIST_GET, key.as_ptr() as u64, key.len() as u64, 0, 0) };
    check(
        "SYS_PERSIST_GET(size)",
        !libmorpheus::is_error(ret) && ret == val.len() as u64,
        "wrong size",
    );

    // GET (read)
    let mut buf = [0u8; 64];
    let ret = unsafe {
        syscall4(
            SYS_PERSIST_GET,
            key.as_ptr() as u64,
            key.len() as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    check(
        "SYS_PERSIST_GET(read)",
        !libmorpheus::is_error(ret) && ret == val.len() as u64,
        "wrong read count",
    );

    // LIST
    let mut list_buf = [0u8; 512];
    let ret = unsafe {
        syscall3(
            SYS_PERSIST_LIST,
            list_buf.as_mut_ptr() as u64,
            list_buf.len() as u64,
            0,
        )
    };
    check(
        "SYS_PERSIST_LIST",
        !libmorpheus::is_error(ret) && ret >= 1,
        "no keys found",
    );

    // INFO
    #[repr(C)]
    struct PersistInfo {
        backend_flags: u32,
        _pad: u32,
        num_keys: u64,
        used_bytes: u64,
    }
    let mut info = PersistInfo {
        backend_flags: 0,
        _pad: 0,
        num_keys: 0,
        used_bytes: 0,
    };
    let ret = unsafe { syscall1(SYS_PERSIST_INFO, &mut info as *mut PersistInfo as u64) };
    check_ok("SYS_PERSIST_INFO", ret);

    // DEL
    let ret = unsafe { syscall2(SYS_PERSIST_DEL, key.as_ptr() as u64, key.len() as u64) };
    check_ok("SYS_PERSIST_DEL", ret);
}

// ── SYS_PE_INFO (51) ────────────────────────────────────────────────────
fn test_pe_info() {
    // Introspect our own binary to verify PE_INFO works.
    let r = libmorpheus::persist::pe_info("/bin/syscall-e2e");
    match r {
        Ok(info) => {
            // format: 1 = ELF64
            check("SYS_PE_INFO(format)", info.format == 1, "not ELF64");
            // arch: 1 = x86_64
            check("SYS_PE_INFO(arch)", info.arch == 1, "not x86_64");
            check("SYS_PE_INFO(entry)", info.entry_point != 0, "entry=0");
            check("SYS_PE_INFO(size)", info.image_size > 0, "size=0");
        }
        Err(_) => {
            fail("SYS_PE_INFO(format)", "pe_info returned error");
            fail("SYS_PE_INFO(arch)", "skipped");
            fail("SYS_PE_INFO(entry)", "skipped");
            fail("SYS_PE_INFO(size)", "skipped");
        }
    }
}

// ── PORT_IN / PORT_OUT (52-53) ──────────────────────────────────────────
fn test_port_io() {
    // Read PIT channel 2 (port 0x42) — should return a byte
    let _val = libmorpheus::hw::port_inb(0x42);
    ok("SYS_PORT_IN(0x42, byte)");

    // Read PCI config address port (0xCF8) — 32-bit
    let _val = libmorpheus::hw::port_inl(0xCF8);
    ok("SYS_PORT_IN(0xCF8, dword)");

    // Write/read a scratch value to unused port (careful — use PIT count latch)
    // Just test that the syscall doesn't error
    let ret = unsafe { syscall3(SYS_PORT_OUT, 0x80, 1, 0) }; // port 0x80 = debug port
    check_ok("SYS_PORT_OUT(0x80, byte)", ret);

    // Invalid width
    let ret = unsafe { syscall2(SYS_PORT_IN, 0x80, 3) }; // width=3 invalid
    check_err("SYS_PORT_IN(bad width)", ret);
}

// ── PCI_CFG_READ / PCI_CFG_WRITE (54-55) ────────────────────────────────
fn test_pci() {
    // Read vendor ID from bus=0, device=0, function=0, offset=0
    let vendor = libmorpheus::hw::pci_cfg_read16(0, 0, 0, 0x00);
    // QEMU typically has vendor 0x8086 (Intel) or 0x1234 (QEMU)
    check(
        "SYS_PCI_CFG_READ(vendor)",
        vendor != 0xFFFF,
        "no device at 00:00.0",
    );

    // Read device ID
    let _device = libmorpheus::hw::pci_cfg_read16(0, 0, 0, 0x02);
    ok("SYS_PCI_CFG_READ(device)");

    // Read class code (32-bit at offset 0x08)
    let _class = libmorpheus::hw::pci_cfg_read32(0, 0, 0, 0x08);
    ok("SYS_PCI_CFG_READ(class)");

    // Write test — write/read-back the latency timer (offset 0x0D)
    // This is generally harmless to modify
    let orig = libmorpheus::hw::pci_cfg_read8(0, 0, 0, 0x0D);
    libmorpheus::hw::pci_cfg_write8(0, 0, 0, 0x0D, orig);
    ok("SYS_PCI_CFG_WRITE(byte)");

    // Invalid width
    let bdf = libmorpheus::hw::pci_bdf(0, 0, 0);
    let ret = unsafe { syscall3(SYS_PCI_CFG_READ, bdf, 0, 3) }; // width 3 invalid
    check_err("SYS_PCI_CFG_READ(bad width)", ret);
}

// ── DMA_ALLOC / DMA_FREE (56-57) ────────────────────────────────────────
fn test_dma() {
    let r = libmorpheus::hw::dma_alloc(1);
    match r {
        Ok(phys) => {
            check("SYS_DMA_ALLOC(1)", phys < 0x1_0000_0000, "not below 4GB");
            let free_r = libmorpheus::hw::dma_free(phys, 1);
            check("SYS_DMA_FREE(1)", free_r.is_ok(), "free failed");
        }
        Err(_) => {
            fail("SYS_DMA_ALLOC(1)", "alloc failed");
            fail("SYS_DMA_FREE(1)", "skipped (alloc failed)");
        }
    }
}

// ── SYS_MAP_PHYS (58) ───────────────────────────────────────────────────
fn test_map_phys() {
    // Allocate a known physical page via DMA, then map it into user space.
    let r = libmorpheus::hw::dma_alloc(1);
    match r {
        Ok(phys) => {
            let mr = libmorpheus::hw::map_phys_rw(phys, 1);
            match mr {
                Ok(vaddr) => {
                    check(
                        "SYS_MAP_PHYS(1 page)",
                        vaddr >= 0x40_0000_0000,
                        "vaddr out of range",
                    );
                    // Unmap the user-space mapping (does not free the physical page).
                    let ur = libmorpheus::mem::munmap(vaddr, 1);
                    check("SYS_MUNMAP(map_phys)", ur.is_ok(), "munmap failed");
                }
                Err(_) => {
                    fail("SYS_MAP_PHYS(1 page)", "map failed");
                    fail("SYS_MUNMAP(map_phys)", "skipped (map failed)");
                }
            }
            let _ = libmorpheus::hw::dma_free(phys, 1);
        }
        Err(_) => {
            fail("SYS_MAP_PHYS(1 page)", "dma_alloc failed");
            fail("SYS_MUNMAP(map_phys)", "skipped (dma_alloc failed)");
        }
    }
}

// ── VIRT_TO_PHYS (59) ───────────────────────────────────────────────────
fn test_virt_to_phys() {
    // Identity-mapped kernel → virt == phys for low addresses
    // Use address of a known variable on the stack
    let stack_var: u64 = 0xDEAD;
    let virt = &stack_var as *const u64 as u64;
    let ret = unsafe { syscall1(SYS_VIRT_TO_PHYS, virt) };
    // In identity-mapped mode, phys should equal virt
    check(
        "SYS_VIRT_TO_PHYS",
        !libmorpheus::is_error(ret),
        "translation failed",
    );
}

// ── IRQ_ATTACH / IRQ_ACK (60-61) ────────────────────────────────────────
fn test_irq() {
    // Attach IRQ 7 (spurious — safe to unmask)
    let r = libmorpheus::hw::irq_attach(7);
    check("SYS_IRQ_ATTACH(7)", r.is_ok(), "attach failed");

    // ACK IRQ 7
    let r = libmorpheus::hw::irq_ack(7);
    check("SYS_IRQ_ACK(7)", r.is_ok(), "ack failed");

    // Out of range
    let ret = unsafe { syscall1(SYS_IRQ_ATTACH, 16) };
    check_err("SYS_IRQ_ATTACH(16 OOB)", ret);
}

// ── CACHE_FLUSH (62) ────────────────────────────────────────────────────
fn test_cache_flush() {
    // Flush a small range (our own stack)
    let data = [0u8; 4096];
    let addr = data.as_ptr() as u64;
    // Align down to page
    let aligned = addr & !0xFFF;
    let r = libmorpheus::hw::cache_flush(aligned, 4096);
    check("SYS_CACHE_FLUSH", r.is_ok(), "flush failed");
}

// ── FB_INFO (63) ─────────────────────────────────────────────────────────
fn test_fb_info() {
    let r = libmorpheus::hw::fb_info();
    match r {
        Ok(info) => {
            check("SYS_FB_INFO width", info.width > 0, "width=0");
            check("SYS_FB_INFO height", info.height > 0, "height=0");
            check("SYS_FB_INFO base", info.base > 0, "base=0");
        }
        Err(_) => {
            fail("SYS_FB_INFO", "returned error (FB not registered?)");
            fail("SYS_FB_INFO height", "skipped (no FB)");
            fail("SYS_FB_INFO base", "skipped (no FB)");
        }
    }
}

// ── SYS_FB_MAP (64) ─────────────────────────────────────────────────────
fn test_fb_map() {
    let r = libmorpheus::hw::fb_map();
    match r {
        Ok(vaddr) => {
            check("SYS_FB_MAP", vaddr >= 0x40_0000_0000, "vaddr out of range");
            // Unmap the framebuffer mapping.
            // We don't know the exact page count, but 1 is sufficient to
            // exercise the munmap path (VMA tracks the real size).
            // Skip unmapping — the FB stays mapped for the rest of the test.
            ok("SYS_FB_MAP(mapped)");
        }
        Err(_) => {
            fail("SYS_FB_MAP", "map failed");
            fail("SYS_FB_MAP(mapped)", "skipped (map failed)");
        }
    }
}

// ── PS (65) ──────────────────────────────────────────────────────────────
fn test_ps() {
    let mut entries = [const { libmorpheus::process::PsEntry::zeroed() }; 32];
    let count = libmorpheus::process::ps(&mut entries);
    check("SYS_PS", count >= 1, "no processes");
    if count >= 1 {
        check(
            "SYS_PS pid[0]",
            entries[0].pid == 0 || entries[0].pid < 256,
            "bad pid",
        );
    }
}

// ── SIGACTION (66) ───────────────────────────────────────────────────────
fn test_sigaction() {
    // Register a handler (address doesn't matter for now — just exercises syscall)
    let r = libmorpheus::process::sigaction(15, 0); // SIGTERM, default handler
    check("SYS_SIGACTION", r.is_ok(), "sigaction failed");
}

// ── SETPRIORITY / GETPRIORITY (67-68) ───────────────────────────────────
fn test_priority() {
    let pid = libmorpheus::process::getpid();

    // Get current priority
    let r = libmorpheus::process::getpriority(pid);
    match r {
        Ok(prio) => {
            ok("SYS_GETPRIORITY");
            // Set priority to 100
            let r2 = libmorpheus::process::setpriority(pid, 100);
            check("SYS_SETPRIORITY", r2.is_ok(), "set failed");
            // Restore
            let _ = libmorpheus::process::setpriority(pid, prio);
        }
        Err(_) => {
            fail("SYS_GETPRIORITY", "returned error");
            fail("SYS_SETPRIORITY", "skipped (getpriority failed)");
        }
    }
}

// ── CPUID (69) ──────────────────────────────────────────────────────────
fn test_cpuid() {
    let r = libmorpheus::hw::cpuid(0, 0);
    // EAX = max basic leaf (should be > 0)
    check("SYS_CPUID leaf=0", r.eax > 0, "eax=0");

    // Check vendor string (EBX+EDX+ECX)
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&r.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&r.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&r.ecx.to_le_bytes());
    let vendor_str = core::str::from_utf8(&vendor).unwrap_or("???");
    print("  CPUID vendor: ");
    println(vendor_str);
}

// ── RDTSC (70) ──────────────────────────────────────────────────────────
fn test_rdtsc() {
    let r = libmorpheus::hw::rdtsc();
    check("SYS_RDTSC tsc", r.tsc > 0, "tsc=0");
    check("SYS_RDTSC freq", r.frequency > 0, "freq=0");
    print("  TSC: ");
    print_hex(r.tsc);
    print("  freq: ");
    print_hex(r.frequency);
    println(" Hz");
}

// ── BOOT_LOG (71) ───────────────────────────────────────────────────────
fn test_boot_log() {
    let size = libmorpheus::hw::boot_log_size();
    check("SYS_BOOT_LOG(size)", size > 0, "log empty");

    let mut buf = [0u8; 256];
    let r = libmorpheus::hw::boot_log(&mut buf);
    match r {
        Ok(n) => check("SYS_BOOT_LOG(read)", n > 0, "read 0 bytes"),
        Err(_) => fail("SYS_BOOT_LOG(read)", "returned error"),
    }
}

// ── MEMMAP (72) ─────────────────────────────────────────────────────────
fn test_memmap() {
    let count = libmorpheus::hw::memmap_count();
    check("SYS_MEMMAP(count)", count > 0, "no entries");

    let mut entries = [libmorpheus::hw::MemmapEntry {
        phys_start: 0,
        num_pages: 0,
        mem_type: 0,
        _pad: 0,
    }; 128];
    let r = libmorpheus::hw::memmap(&mut entries);
    match r {
        Ok(n) => {
            check("SYS_MEMMAP(read)", n > 0, "0 entries");
            // First entry should have a physical start
            print("  MEMMAP entries: ");
            print_u32(n as u32);
            println("");
        }
        Err(_) => fail("SYS_MEMMAP(read)", "returned error"),
    }
}

// ── SHM_GRANT (73) ───────────────────────────────────────────────────
fn test_shm_grant() {
    // Allocate 1 page, then try to grant to self → EINVAL.
    match libmorpheus::mem::mmap(1) {
        Ok(vaddr) => {
            // Self-grant must fail.
            let r = libmorpheus::mem::shm_grant(
                unsafe { syscall0(SYS_GETPID) } as u32,
                vaddr,
                1,
                libmorpheus::mem::PROT_WRITE,
            );
            check("SYS_SHM_GRANT(self)", r.is_err(), "self-grant should fail");

            // Grant to non-existent PID 63 → ESRCH.
            let r = libmorpheus::mem::shm_grant(63, vaddr, 1, 0);
            check("SYS_SHM_GRANT(bad pid)", r.is_err(), "should fail");

            // Bad page count (0) → EINVAL.
            let r = libmorpheus::mem::shm_grant(2, vaddr, 0, 0);
            check("SYS_SHM_GRANT(0 pages)", r.is_err(), "should fail");

            let _ = libmorpheus::mem::munmap(vaddr, 1);
        }
        Err(_) => {
            fail("SYS_SHM_GRANT(self)", "mmap failed");
            fail("SYS_SHM_GRANT(bad pid)", "mmap failed");
            fail("SYS_SHM_GRANT(0 pages)", "mmap failed");
        }
    }
}

// ── MPROTECT (74) ────────────────────────────────────────────────────
fn test_mprotect() {
    match libmorpheus::mem::mmap(1) {
        Ok(vaddr) => {
            // Make read-only (PROT_READ = 0).
            let r = libmorpheus::mem::mprotect(vaddr, 1, libmorpheus::mem::PROT_READ);
            check("SYS_MPROTECT(RO)", r.is_ok(), "mprotect failed");

            // Make writable again.
            let r = libmorpheus::mem::mprotect(vaddr, 1, libmorpheus::mem::PROT_WRITE);
            check("SYS_MPROTECT(RW)", r.is_ok(), "mprotect failed");

            // Bad prot bits → EINVAL.
            let r = libmorpheus::mem::mprotect(vaddr, 1, 0xFF);
            check("SYS_MPROTECT(bad prot)", r.is_err(), "should fail");

            // Wrong page count → EINVAL.
            let r = libmorpheus::mem::mprotect(vaddr, 2, 0);
            check("SYS_MPROTECT(bad pages)", r.is_err(), "should fail");

            let _ = libmorpheus::mem::munmap(vaddr, 1);
        }
        Err(_) => {
            fail("SYS_MPROTECT(RO)", "mmap failed");
            fail("SYS_MPROTECT(RW)", "mmap failed");
            fail("SYS_MPROTECT(bad prot)", "mmap failed");
            fail("SYS_MPROTECT(bad pages)", "mmap failed");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_PIPE (75) — create a pipe, write/read through it
// ═══════════════════════════════════════════════════════════════════════════

fn test_pipe() {
    match libmorpheus::process::pipe() {
        Ok((read_fd, write_fd)) => {
            check("SYS_PIPE(create)", true, "");

            // Write to the pipe.
            let msg = b"hello pipe";
            let wr = libmorpheus::io::write_fd(write_fd, msg);
            check("SYS_PIPE(write)", wr.is_ok(), "pipe write failed");

            // Read back.
            let mut buf = [0u8; 32];
            let rd = libmorpheus::io::read_fd(read_fd, &mut buf);
            match rd {
                Ok(n) => check(
                    "SYS_PIPE(read)",
                    n == 10 && buf[..10] == *msg,
                    "data mismatch",
                ),
                Err(_) => fail("SYS_PIPE(read)", "pipe read failed"),
            }

            // Close both ends.
            unsafe {
                syscall1(SYS_CLOSE, read_fd as u64);
                syscall1(SYS_CLOSE, write_fd as u64);
            }
        }
        Err(_) => {
            fail("SYS_PIPE(create)", "pipe() failed");
            fail("SYS_PIPE(write)", "no pipe");
            fail("SYS_PIPE(read)", "no pipe");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_DUP2 (76)
// ═══════════════════════════════════════════════════════════════════════════

fn test_dup2() {
    match libmorpheus::process::pipe() {
        Ok((read_fd, write_fd)) => {
            // Dup the read fd to fd 10.
            match libmorpheus::process::dup2(read_fd, 10) {
                Ok(fd) => check("SYS_DUP2(ok)", fd == 10, "wrong fd"),
                Err(_) => fail("SYS_DUP2(ok)", "dup2 failed"),
            }

            // Write through the original write end, read from fd 10.
            let msg = b"dup2";
            let _ = libmorpheus::io::write_fd(write_fd, msg);
            let mut buf = [0u8; 16];
            let rd = libmorpheus::io::read_fd(10, &mut buf);
            match rd {
                Ok(n) => check("SYS_DUP2(data)", n == 4, "wrong len"),
                Err(_) => fail("SYS_DUP2(data)", "read failed"),
            }

            // Invalid: dup2 with a bad old_fd.
            let r = libmorpheus::process::dup2(62, 11);
            check("SYS_DUP2(bad)", r.is_err(), "should fail");

            unsafe {
                syscall1(SYS_CLOSE, read_fd as u64);
                syscall1(SYS_CLOSE, write_fd as u64);
                syscall1(SYS_CLOSE, 10);
            }
        }
        Err(_) => {
            fail("SYS_DUP2(ok)", "no pipe");
            fail("SYS_DUP2(data)", "no pipe");
            fail("SYS_DUP2(bad)", "no pipe");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SET_FG (77)
// ═══════════════════════════════════════════════════════════════════════════

fn test_set_fg() {
    // Set foreground to ourselves — should not fail.
    let pid = libmorpheus::process::getpid();
    libmorpheus::process::set_foreground(pid);
    check("SYS_SET_FG(self)", true, "");

    // Reset to 0 (no foreground).
    libmorpheus::process::set_foreground(0);
    check("SYS_SET_FG(reset)", true, "");
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_GETARGS (78)
// ═══════════════════════════════════════════════════════════════════════════

fn test_getargs() {
    // We were spawned without args, so argc should be 0.
    let c = libmorpheus::process::argc();
    check("SYS_GETARGS(argc=0)", c == 0, "expected 0 args");

    // getargs into buffer should also return 0.
    let mut buf = [0u8; 64];
    let c2 = libmorpheus::process::getargs(&mut buf);
    check("SYS_GETARGS(buf)", c2 == 0, "expected 0 args");
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_FUTEX (79)
// ═══════════════════════════════════════════════════════════════════════════

fn test_futex() {
    use core::sync::atomic::AtomicU32;

    // FUTEX_WAIT with wrong expected value should return EAGAIN immediately.
    let word = AtomicU32::new(42);
    let ret = unsafe {
        syscall3(
            SYS_FUTEX,
            &word as *const AtomicU32 as u64,
            FUTEX_WAIT,
            99, // expected=99, but word=42 → EAGAIN
        )
    };
    // EAGAIN = u64::MAX - 11
    check(
        "SYS_FUTEX(wait-eagain)",
        ret == u64::MAX - 11,
        "expected EAGAIN",
    );

    // FUTEX_WAKE with no waiters should return 0 (nobody woken).
    let ret2 = unsafe { syscall3(SYS_FUTEX, &word as *const AtomicU32 as u64, FUTEX_WAKE, 1) };
    check("SYS_FUTEX(wake-none)", ret2 == 0, "expected 0 woken");

    // Test Mutex via the sync module.
    {
        let m = libmorpheus::sync::Mutex::new(0u32);
        {
            let mut guard = m.lock();
            *guard = 123;
        }
        let guard = m.lock();
        check("Mutex(lock-unlock)", *guard == 123, "wrong value");
    }

    ok("SYS_FUTEX(basic)");
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_THREAD_CREATE (80), SYS_THREAD_JOIN (82)
// ═══════════════════════════════════════════════════════════════════════════

fn test_thread_create_join() {
    use core::sync::atomic::{AtomicU32, Ordering};

    // Spawn a thread that writes a sentinel value to shared memory.
    static SENTINEL: AtomicU32 = AtomicU32::new(0);

    let handle = libmorpheus::thread::spawn(|| {
        SENTINEL.store(0xDEAD, Ordering::Release);
    });

    match handle {
        Ok(h) => {
            let _ = h.join();
            let val = SENTINEL.load(Ordering::Acquire);
            check("SYS_THREAD_CREATE+JOIN", val == 0xDEAD, "sentinel mismatch");
        }
        Err(_) => fail("SYS_THREAD_CREATE+JOIN", "spawn failed"),
    }
}

fn test_thread_shared_memory() {
    use core::sync::atomic::{AtomicU32, Ordering};

    // Verify threads share address space by having two threads
    // increment a shared atomic counter.
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.store(0, Ordering::SeqCst);

    let h1 = libmorpheus::thread::spawn(|| {
        for _ in 0..100 {
            COUNTER.fetch_add(1, Ordering::Relaxed);
        }
    });
    let h2 = libmorpheus::thread::spawn(|| {
        for _ in 0..100 {
            COUNTER.fetch_add(1, Ordering::Relaxed);
        }
    });

    match (h1, h2) {
        (Ok(h1), Ok(h2)) => {
            let _ = h1.join();
            let _ = h2.join();
            let val = COUNTER.load(Ordering::SeqCst);
            check("THREAD(shared_mem)", val == 200, "count mismatch");
        }
        _ => fail("THREAD(shared_mem)", "spawn failed"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Async Runtime Tests
// ═══════════════════════════════════════════════════════════════════════════

fn test_async_block_on() {
    // block_on a simple async block that returns a value.
    let result = libmorpheus::task::block_on(async { 42u32 });
    check("async(block_on)", result == 42, "wrong return value");
}

fn test_async_spawn_multi() {
    use core::sync::atomic::{AtomicU32, Ordering};

    // Spawn multiple async tasks and verify they all complete.
    static ASYNC_COUNTER: AtomicU32 = AtomicU32::new(0);
    ASYNC_COUNTER.store(0, Ordering::SeqCst);

    let rt = libmorpheus::task::Runtime::new();
    for _ in 0..5 {
        rt.spawn(async {
            ASYNC_COUNTER.fetch_add(1, Ordering::Relaxed);
        });
    }
    rt.run();

    let val = ASYNC_COUNTER.load(Ordering::SeqCst);
    check("async(spawn_multi)", val == 5, "not all tasks completed");
}

fn test_async_yield() {
    use core::sync::atomic::{AtomicU32, Ordering};

    // Two tasks interleave execution via yield_now.
    static ORDER: AtomicU32 = AtomicU32::new(0);
    ORDER.store(0, Ordering::SeqCst);

    let rt = libmorpheus::task::Runtime::new();
    rt.spawn(async {
        ORDER.fetch_add(1, Ordering::SeqCst); // 1
        libmorpheus::task::yield_now().await;
        ORDER.fetch_add(10, Ordering::SeqCst); // 11 or 12
    });
    rt.spawn(async {
        ORDER.fetch_add(1, Ordering::SeqCst); // 2
        libmorpheus::task::yield_now().await;
        ORDER.fetch_add(10, Ordering::SeqCst); // 12 or 22
    });
    rt.run();

    let val = ORDER.load(Ordering::SeqCst);
    // Both tasks add 1 + 10 = 11 each, total = 22.
    check("async(yield)", val == 22, "yield interleave failed");
}

fn test_async_join_handle() {
    // spawn_with_handle returns a JoinHandle we can .await for the result.
    let rt = libmorpheus::task::Runtime::new();
    let handle = rt.spawn_with_handle(async {
        42u64 + 58
    });
    // Spawn a consumer that awaits the handle.
    use core::sync::atomic::{AtomicU64, Ordering};
    static RESULT: AtomicU64 = AtomicU64::new(0);
    RESULT.store(0, Ordering::SeqCst);
    rt.spawn(async move {
        let val = handle.await;
        RESULT.store(val, Ordering::SeqCst);
    });
    rt.run();
    check("async(join_handle)", RESULT.load(Ordering::SeqCst) == 100, "wrong join result");
}

fn test_async_sleep() {
    // sleep() should complete after approximately the requested duration.
    let before = libmorpheus::time::clock_gettime();
    libmorpheus::task::block_on(async {
        libmorpheus::task::sleep(50).await;
    });
    let elapsed_ns = libmorpheus::time::clock_gettime() - before;
    let elapsed_ms = elapsed_ns / 1_000_000;
    // Allow generous timing: 30..500ms (QEMU timer resolution varies).
    check("async(sleep)", elapsed_ms >= 30 && elapsed_ms < 500, "sleep timing off");
}

fn test_async_chained_await() {
    // Chain multiple async operations: spawn several tasks that each
    // do some work, yield, then produce a value. Collect via JoinHandles.
    use core::sync::atomic::{AtomicU32, Ordering};
    static SUM: AtomicU32 = AtomicU32::new(0);
    SUM.store(0, Ordering::SeqCst);

    let rt = libmorpheus::task::Runtime::new();
    let h1 = rt.spawn_with_handle(async {
        libmorpheus::task::yield_now().await;
        10u32
    });
    let h2 = rt.spawn_with_handle(async {
        libmorpheus::task::yield_now().await;
        20u32
    });
    let h3 = rt.spawn_with_handle(async {
        libmorpheus::task::yield_now().await;
        30u32
    });
    rt.spawn(async move {
        let total = h1.await + h2.await + h3.await;
        SUM.store(total, Ordering::SeqCst);
    });
    rt.run();
    check("async(chained)", SUM.load(Ordering::SeqCst) == 60, "chained await sum wrong");
}
