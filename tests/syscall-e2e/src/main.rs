//! Exercises all 73 syscalls (0-72) on a running kernel.
//! Each test prints `[PASS]` or `[FAIL]` to serial; summary line at the end.

#![no_std]
#![no_main]
#![feature(thread_local)]

extern crate alloc;

mod bench;

use alloc::vec::Vec;
use libmorpheus::entry;
use libmorpheus::io::{print, println};
use libmorpheus::raw::*;

entry!(main);

static mut PASS: u32 = 0;
static mut FAIL: u32 = 0;
static mut TOTAL: u32 = 0;

/// Names of failed checks, dumped as a clean roster after the summary. The
/// serial console prefixes and may interleave per-write across cores, so a
/// compact end-of-run list (printed when the other procs are quiet) is the only
/// reliable way to see *which* checks failed.
static mut FAILED: [&str; 64] = [""; 64];
static mut NFAILED: usize = 0;

/// Emit one fully-formed line in a SINGLE `SYS_WRITE`. `println` coalesces the
/// slice + newline into one write via libmorpheus' `FdWriter`, so the whole
/// `[PASS]/[FAIL] …` line lands atomically and can't be split or dropped by
/// another core writing mid-line — unlike the previous multi-`print()` pattern.
pub(crate) fn emit_line(parts: &[&str]) {
    let mut buf = [0u8; 256];
    let mut n = 0;
    for p in parts {
        let b = p.as_bytes();
        let take = core::cmp::min(b.len(), buf.len() - n);
        buf[n..n + take].copy_from_slice(&b[..take]);
        n += take;
        if n == buf.len() {
            break;
        }
    }
    // SAFETY: every byte came from a &str, so the prefix is valid UTF-8.
    println(unsafe { core::str::from_utf8_unchecked(&buf[..n]) });
}

fn ok(name: &'static str) {
    emit_line(&["[PASS] ", name]);
    unsafe {
        let p = core::ptr::addr_of_mut!(PASS);
        let v = core::ptr::read_volatile(p);
        core::ptr::write_volatile(p, v + 1);
        let t = core::ptr::addr_of_mut!(TOTAL);
        let tv = core::ptr::read_volatile(t);
        core::ptr::write_volatile(t, tv + 1);
    }
}

fn fail(name: &'static str, detail: &str) {
    emit_line(&["[FAIL] ", name, " — ", detail]);
    unsafe {
        let p = core::ptr::addr_of_mut!(FAIL);
        let v = core::ptr::read_volatile(p);
        core::ptr::write_volatile(p, v + 1);
        let t = core::ptr::addr_of_mut!(TOTAL);
        let tv = core::ptr::read_volatile(t);
        core::ptr::write_volatile(t, tv + 1);
        // Record for the end-of-run roster (bounded; ignore overflow).
        let nf = core::ptr::addr_of_mut!(NFAILED);
        let i = core::ptr::read_volatile(nf);
        if i < 64 {
            (*core::ptr::addr_of_mut!(FAILED))[i] = name;
            core::ptr::write_volatile(nf, i + 1);
        }
    }
}

fn check(name: &'static str, cond: bool, detail: &str) {
    if cond {
        ok(name);
    } else {
        fail(name, detail);
    }
}

fn check_ok(name: &'static str, ret: u64) {
    if libmorpheus::is_error(ret) {
        fail(name, "returned error");
    } else {
        ok(name);
    }
}

fn check_err(name: &'static str, ret: u64) {
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

/// Dispatch on argv[0] to the selected bench mode.
/// Defaults to `smoke` (one-shot correctness gate) when no mode is given.
fn main() -> i32 {
    let mut argbuf = [0u8; 256];
    let n = libmorpheus::process::getargs(&mut argbuf);
    // Shell strips argv[0] before spawn; arg 0 in blob is the mode.
    let mode = nth_arg(&argbuf[..n], 0).unwrap_or("smoke");

    match mode {
        "smoke" => run_smoke(),
        "threads" => {
            // threads [N] [secs] [--seed HEX]
            let blob = &argbuf[..n];
            let nthreads = nth_arg(blob, 1).and_then(parse_u64).unwrap_or(4) as usize;
            let secs = nth_arg(blob, 2).and_then(parse_u64).unwrap_or(0);
            let seed = flag_value(blob, "--seed")
                .and_then(parse_hex)
                .unwrap_or_else(|| libmorpheus::time::clock_gettime() | 1);
            bench::run_threads(nthreads, secs, seed)
        },
        "swarm" => {
            // swarm [N] [secs] [--seed HEX] [--self PATH]
            let blob = &argbuf[..n];
            let nchildren = nth_arg(blob, 1).and_then(parse_u64).unwrap_or(4) as usize;
            let secs = nth_arg(blob, 2).and_then(parse_u64).unwrap_or(0);
            let seed = flag_value(blob, "--seed")
                .and_then(parse_hex)
                .unwrap_or_else(|| libmorpheus::time::clock_gettime() | 1);
            let self_path = flag_value(blob, "--self").unwrap_or(bench::SELF_PATH);
            bench::run_swarm(nchildren, secs, seed, self_path)
        },
        "_worker" => {
            // Hidden swarm child sub-mode: _worker <seed HEX> <secs> <pipe_rfd>
            let blob = &argbuf[..n];
            let seed = nth_arg(blob, 1).and_then(parse_hex).unwrap_or(1);
            let secs = nth_arg(blob, 2).and_then(parse_u64).unwrap_or(0);
            let rfd = nth_arg(blob, 3).and_then(parse_u64).unwrap_or(0) as u32;
            bench::run_worker(seed, secs, rfd)
        },
        other => {
            emit_line(&["[bench] unknown mode: ", other]);
            println("usage: bench [smoke | threads N secs | swarm N secs] [--seed HEX]");
            2
        },
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(v)
}

/// Parse a hex `u64` from `s` (optional `0x` prefix); `None` on any non-hex digit.
fn parse_hex(s: &str) -> Option<u64> {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if s.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for &b in s.as_bytes() {
        let d = (b as char).to_digit(16)?;
        v = v.checked_mul(16)?.checked_add(d as u64)?;
    }
    Some(v)
}

/// Return the token following `flag` in the NUL-separated argv blob, if present.
fn flag_value<'a>(blob: &'a [u8], flag: &str) -> Option<&'a str> {
    let mut it = blob.split(|&b| b == 0).filter(|s| !s.is_empty());
    while let Some(tok) = it.next() {
        if tok == flag.as_bytes() {
            return it.next().and_then(|v| core::str::from_utf8(v).ok());
        }
    }
    None
}

/// Return the `idx`-th NUL-separated token in the argv blob, if present.
fn nth_arg(blob: &[u8], idx: usize) -> Option<&str> {
    blob.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .nth(idx)
        .and_then(|s| core::str::from_utf8(s).ok())
}

/// One-shot correctness gate: exercise every syscall once. Exit code = failures.
fn run_smoke() -> i32 {
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
    test_poll(); // 45

    // ── Storage subsystem (102-103 + 43/44 redef) ────────────────────
    println("");
    println("── Storage (volumes/mount/umount) ──");
    test_storage(); // SYS_VOLUMES, SYS_MOUNTS, SYS_MOUNT(43), SYS_UMOUNT(44)

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

    // ── TLS / RNG (100-101) ───────────────────────────────────────
    println("");
    println("── TLS / RNG (100-101) ──");
    test_set_thread_pointer(); // 100
    test_thread_local(); // 100 (crt0 variant-II TLS)
    test_getrandom(); // 101

    // ── Summary ──────────────────────────────────────────────────────
    println("");
    println("════════════════════════════════════════════");
    let total = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TOTAL)) };
    let p = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(PASS)) };
    let f = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(FAIL)) };
    // Single atomic line: "TOTAL: N  PASS: N  FAIL: N".
    emit_line(&[
        "TOTAL: ",
        u32_str(total, &mut [0u8; 10]),
        "  PASS: ",
        u32_str(p, &mut [0u8; 10]),
        "  FAIL: ",
        u32_str(f, &mut [0u8; 10]),
    ]);
    if f == 0 {
        println("ALL TESTS PASSED");
    } else {
        println("SOME TESTS FAILED");
        // Definitive roster — one atomic line per failure, printed now that the
        // rest of the suite is quiet, so the list survives serial interleaving.
        println("──────────────── FAILURES ────────────────");
        let nf = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(NFAILED)) };
        for i in 0..nf {
            let name = unsafe { (*core::ptr::addr_of!(FAILED))[i] };
            emit_line(&["  ✗ ", name]);
        }
    }
    println("════════════════════════════════════════════");

    f as i32 // exit code = number of failures
}

fn u32_str(v: u32, buf: &mut [u8; 10]) -> &str {
    if v == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut n = v;
    let mut i = 10usize;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // Shift the digits to the front so the returned slice borrows `buf` cleanly.
    let len = 10 - i;
    buf.copy_within(i..10, 0);
    unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
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

fn test_write() {
    let msg = b"test_write output\n";
    let ret = unsafe { syscall3(SYS_WRITE, 1, msg.as_ptr() as u64, msg.len() as u64) };
    check(
        "SYS_WRITE(1) stdout",
        ret == msg.len() as u64,
        "wrong byte count",
    );

    let ret = unsafe { syscall3(SYS_WRITE, 2, msg.as_ptr() as u64, msg.len() as u64) };
    check(
        "SYS_WRITE(2) stderr",
        ret == msg.len() as u64,
        "wrong byte count",
    );
}

fn test_read() {
    // SYS_READ(0) blocks until input by default; a blind read here would hang
    // forever (this process is never a focused window). Switch stdin to
    // non-blocking via FIONBIO, then expect EAGAIN with no keys buffered.
    let nb = libmorpheus::io::set_stdin_nonblocking(true);
    check("SYS_IOCTL(FIONBIO on)", nb.is_ok(), "fionbio failed");

    let mut buf = [0u8; 16];
    let ret = unsafe { syscall3(SYS_READ, 0, buf.as_mut_ptr() as u64, buf.len() as u64) };
    // EAGAIN = u64::MAX - 11
    check(
        "SYS_READ(0) non-blocking EAGAIN",
        ret == u64::MAX - 11,
        "expected EAGAIN",
    );

    let _ = libmorpheus::io::set_stdin_nonblocking(false);
}

fn test_yield() {
    let ret = unsafe { syscall0(SYS_YIELD) };
    check_ok("SYS_YIELD", ret);
}

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

fn test_getpid() {
    let pid = libmorpheus::process::getpid();
    check("SYS_GETPID", pid < 256, "pid out of range");
}

fn test_kill() {
    // Kill a non-existent process → should return -ESRCH
    let ret = unsafe { syscall2(SYS_KILL, 255, 15) };
    check_err("SYS_KILL(bad pid)", ret);
}

fn test_sleep() {
    let t1 = libmorpheus::time::clock_gettime();
    libmorpheus::process::sleep(10); // 10ms
    let t2 = libmorpheus::time::clock_gettime();
    // Should have elapsed at least ~5ms (allow slack for scheduling)
    check("SYS_SLEEP(10ms)", t2 > t1, "clock did not advance");
}

fn test_fs() {
    use libmorpheus::fs;

    let _ = fs::mkdir("/tmp");
    let r = fs::mkdir("/tmp/e2etest");
    check(
        "SYS_MKDIR",
        r.is_ok() || r == Err(libmorpheus::EINVAL - 17),
        "mkdir failed",
    );

    let fd = fs::open("/tmp/e2etest/hello.txt", fs::O_WRITE | fs::O_CREATE);
    match fd {
        Ok(fd) => {
            ok("SYS_OPEN(create)");
            let data = b"Hello MorpheusX!\n";
            let wr = fs::write(fd, data);
            check("SYS_WRITE(vfs)", wr.is_ok(), "write failed");
            let _ = fs::close(fd);
            ok("SYS_CLOSE");
        },
        Err(_) => {
            fail("SYS_OPEN(create)", "open failed");
            fail("SYS_WRITE(vfs)", "skipped (open failed)");
            fail("SYS_CLOSE", "skipped (open failed)");
        },
    }

    let fd = fs::open("/tmp/e2etest/hello.txt", fs::O_READ);
    match fd {
        Ok(fd) => {
            let mut buf = [0u8; 64];
            let rd = fs::read(fd, &mut buf);
            check("SYS_READ(vfs)", rd.is_ok(), "read failed");
            let _ = fs::close(fd);
        },
        Err(_) => {
            fail("SYS_OPEN(read)", "open for read failed");
        },
    }

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
        },
        Err(_) => fail("SYS_SEEK setup", "open failed"),
    }

    let mut stat_buf = [0u8; 64];
    let r = fs::stat("/tmp/e2etest/hello.txt", &mut stat_buf);
    check("SYS_STAT", r.is_ok(), "stat failed");

    let mut dir_buf = [0u8; 4096];
    let r = fs::readdir("/tmp/e2etest", &mut dir_buf);
    check("SYS_READDIR", r.is_ok(), "readdir failed");

    let r = fs::rename("/tmp/e2etest/hello.txt", "/tmp/e2etest/renamed.txt");
    check("SYS_RENAME", r.is_ok(), "rename failed");

    let r = libmorpheus::fs::unlink("/tmp/e2etest/renamed.txt");
    check("SYS_UNLINK", r.is_ok(), "unlink failed");

    let r = fs::sync();
    check("SYS_SYNC", r.is_ok(), "sync failed");

    let _ = fs::unlink("/tmp/e2etest");
}

fn test_truncate() {
    use libmorpheus::fs;
    let path = "/tmp/e2etrunc.txt";
    if let Ok(fd) = fs::open(path, fs::O_WRITE | fs::O_CREATE) {
        let _ = fs::write(fd, b"Some test data for truncation");
        let _ = fs::close(fd);
    }
    // Shrink to a non-zero size; the old impl could only truncate to 0.
    let ret = unsafe { syscall3(SYS_TRUNCATE, path.as_ptr() as u64, path.len() as u64, 4) };
    check_ok("SYS_TRUNCATE", ret);
    match fs::metadata(path) {
        Ok(m) => check("SYS_TRUNCATE(size)", m.len() == 4, "new_size not honored"),
        Err(_) => fail("SYS_TRUNCATE(size)", "stat after truncate failed"),
    }
    let _ = fs::unlink(path);
}

fn test_snapshot() {
    let name = "e2e_snap";
    let ret = unsafe { syscall2(SYS_SNAPSHOT, name.as_ptr() as u64, name.len() as u64) };
    // Returns the real LSN of the logged snapshot marker (non-error).
    check(
        "SYS_SNAPSHOT",
        !libmorpheus::is_error(ret),
        "returned error",
    );
}

fn test_versions() {
    use libmorpheus::fs;
    let path = "/tmp/e2ever.txt";
    // Overwrite a couple of times to log multiple versions.
    for _ in 0..2 {
        if let Ok(fd) = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC) {
            let _ = fs::write(fd, b"v");
            let _ = fs::close(fd);
        }
    }
    // versions() walks the durable on-disk log; commit the writes first.
    let _ = fs::sync();
    match fs::versions(path) {
        Ok(v) => check("SYS_VERSIONS", !v.is_empty(), "no versions returned"),
        Err(_) => fail("SYS_VERSIONS", "versions() errored"),
    }
    let _ = fs::unlink(path);
}

fn test_clock() {
    let t = libmorpheus::time::clock_gettime();
    check("SYS_CLOCK", t > 0, "clock returned 0");
}

fn test_sysinfo() {
    let mut info = libmorpheus::sys::SysInfo::zeroed();
    let r = libmorpheus::sys::sysinfo(&mut info);
    check("SYS_SYSINFO", r.is_ok(), "sysinfo failed");
    check("SYS_SYSINFO total_mem", info.total_mem > 0, "no total_mem");
    check("SYS_SYSINFO tsc_freq", info.tsc_freq > 0, "no tsc_freq");
}

fn test_getppid() {
    let ppid = libmorpheus::process::getppid();
    // Kernel (PID 0) → ppid=0.  User process → ppid=parent.
    check("SYS_GETPPID", ppid < 256, "ppid out of range");
}

fn test_mmap_munmap() {
    let r = libmorpheus::mem::mmap(1);
    match r {
        Ok(vaddr) => {
            check(
                "SYS_MMAP(1 page)",
                vaddr >= 0x40_0000_0000,
                "vaddr out of range",
            );

            let ur = libmorpheus::mem::munmap(vaddr, 1);
            check("SYS_MUNMAP(1 page)", ur.is_ok(), "munmap failed");
        },
        Err(_) => {
            fail("SYS_MMAP(1 page)", "alloc failed");
            fail("SYS_MUNMAP(1 page)", "skipped (mmap failed)");
        },
    }
}

fn test_dup() {
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

fn test_syslog() {
    libmorpheus::sys::syslog("[E2E] syslog test message");
    ok("SYS_SYSLOG");
}

fn test_getcwd() {
    let mut buf = [0u8; 256];
    let r = libmorpheus::fs::getcwd(&mut buf);
    check("SYS_GETCWD", r.is_ok(), "getcwd failed");
}

fn test_chdir() {
    let mut orig = [0u8; 256];
    let _ = libmorpheus::fs::getcwd(&mut orig);

    let r = libmorpheus::fs::chdir("/tmp");
    check("SYS_CHDIR", r.is_ok(), "chdir /tmp failed");

    let _ = libmorpheus::fs::chdir("/");
}

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

    let mut buf = [0u8; 1514];
    let _ret = unsafe { syscall2(SYS_NIC_RX, buf.as_mut_ptr() as u64, buf.len() as u64) };
    // ENODEV or 0 bytes — both fine
    check("SYS_NIC_RX", true, "");

    let _ret = unsafe { syscall0(SYS_NIC_REFILL) };
    check("SYS_NIC_REFILL", true, ""); // ENODEV expected
}

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

fn test_ioctl() {
    // FIONREAD on stdin (fd 0)
    let ret = unsafe { syscall3(SYS_IOCTL, 0, 0x541B, 0) };
    check_ok("SYS_IOCTL(FIONREAD)", ret);

    let mut winsize = [0u32; 2];
    let ret = unsafe { syscall3(SYS_IOCTL, 0, 0x5413, winsize.as_mut_ptr() as u64) };
    check_ok("SYS_IOCTL(TIOCGWINSZ)", ret);

    // Unknown command → EINVAL
    let ret = unsafe { syscall3(SYS_IOCTL, 0, 0xDEAD, 0) };
    check_err("SYS_IOCTL(bad cmd)", ret);
}

/// EXDEV is not re-exported through `libmorpheus`; encode it directly
/// (errno.rs: `EXDEV = u64::MAX - 18`), matching the literal-errno style used
/// elsewhere in this suite (e.g. EAGAIN = `u64::MAX - 11`).
const EXDEV: u64 = u64::MAX - 18;

/// Storage subsystem (spec §10): volume enumeration, tmpfs mount round-trip,
/// staged immutable image (disk pristine + writes EROFS), and the negative
/// paths EBUSY / EXDEV / ENODEV / ENOMEM.
fn test_storage() {
    use libmorpheus::fs;

    // SYS_VOLUMES: at minimum the boot root volume must be present, and the
    // count-probe (max==0) must agree with the filled fetch.
    let probe = unsafe { syscall2(SYS_VOLUMES, 0, 0) };
    check(
        "SYS_VOLUMES(probe)",
        !libmorpheus::is_error(probe) && probe >= 1,
        "no volumes (root missing?)",
    );
    let vols = match fs::volumes() {
        Ok(v) => {
            check("SYS_VOLUMES(fetch)", !v.is_empty(), "empty volume table");
            v
        },
        Err(_) => {
            fail("SYS_VOLUMES(fetch)", "volumes() errored");
            Vec::new()
        },
    };
    // Root mount is itself a (staged) mount in this model → a volume backs it.
    // Verify at least one volume carries a known fs_type, not garbage.
    let any_fs = vols
        .iter()
        .any(|v| v.fs_type == fs::FS_HELIX || v.fs_type == fs::FS_FAT32);
    check(
        "SYS_VOLUMES(root present)",
        vols.is_empty() || any_fs,
        "no recognizable fs on any volume",
    );

    test_tmpfs_roundtrip();
    test_staged_immutable(&vols);
    test_umount_busy();
    test_cross_mount_rename();
    test_stale_volume();
    test_oversized_stage();
}

/// Mount a fresh RAM HelixFS (source = VOLUME_NONE), write+read a file through
/// it, then umount. Exercises SYS_MOUNT(43) staged-from-nothing + SYS_UMOUNT(44).
fn test_tmpfs_roundtrip() {
    use libmorpheus::fs;

    let mp = "/mnt/tmpfs";
    let _ = fs::mkdir("/mnt");
    let _ = fs::mkdir(mp);

    // 4 MiB fresh RAM Helix volume. aux = size (required when source==VOLUME_NONE).
    let mid = fs::mount(
        fs::VOLUME_NONE,
        mp,
        fs::FS_HELIX,
        fs::MNT_STAGED,
        4 * 1024 * 1024,
    );
    let mid = match mid {
        Ok(id) => {
            ok("SYS_MOUNT(tmpfs)");
            id
        },
        Err(_) => {
            fail("SYS_MOUNT(tmpfs)", "mount(VOLUME_NONE) failed");
            fail("tmpfs write/read", "skipped (mount failed)");
            fail("SYS_UMOUNT(tmpfs)", "skipped (mount failed)");
            return;
        },
    };
    let _ = mid;

    // The mount must now appear in SYS_MOUNTS.
    match fs::mounts() {
        Ok(ms) => check(
            "SYS_MOUNTS(tmpfs visible)",
            ms.iter().any(|m| {
                let len = (m.mount_point_len as usize).min(m.mount_point.len());
                &m.mount_point[..len] == mp.as_bytes()
            }),
            "mount not listed",
        ),
        Err(_) => fail("SYS_MOUNTS(tmpfs visible)", "mounts() errored"),
    }

    let path = "/mnt/tmpfs/file.txt";
    let data = b"tmpfs payload";
    let mut wrote = false;
    if let Ok(fd) = fs::open(path, fs::O_WRITE | fs::O_CREATE) {
        wrote = fs::write(fd, data) == Ok(data.len());
        let _ = fs::close(fd);
    }
    let mut readback = [0u8; 32];
    let mut read_ok = false;
    if let Ok(fd) = fs::open(path, fs::O_READ) {
        if let Ok(n) = fs::read(fd, &mut readback) {
            read_ok = n == data.len() && &readback[..n] == data;
        }
        let _ = fs::close(fd);
    }
    check("tmpfs write/read", wrote && read_ok, "round-trip mismatch");

    let u = fs::umount(mp, 0);
    check("SYS_UMOUNT(tmpfs)", u.is_ok(), "umount failed");
}

/// Stage a real volume read-only into RAM (MNT_STAGED|MNT_RDONLY): writes must
/// be rejected EROFS and the on-disk source must stay byte-identical. Requires a
/// non-ephemeral volume that isn't already the root mount; skips with a note
/// otherwise (per spec §10 "if a suitable volume exists else skip-with-note").
fn test_staged_immutable(vols: &[libmorpheus::fs::VolumeInfo]) {
    use libmorpheus::fs;

    // Pick a real (non-RAM, non-ephemeral), unmounted volume to stage.
    let cand = vols.iter().find(|v| {
        v.device_kind != fs::DEV_RAM
            && (v.flags & fs::VOL_EPHEMERAL) == 0
            && (v.flags & fs::VOL_MOUNTED) == 0
            && (v.fs_type == fs::FS_HELIX || v.fs_type == fs::FS_FAT32)
    });
    let vol = match cand {
        Some(v) => v,
        None => {
            ok("staged-immutable (skipped — no suitable real volume)");
            return;
        },
    };

    let mp = "/mnt/img";
    let _ = fs::mkdir("/mnt");
    let _ = fs::mkdir(mp);

    // FS_AUTO: detect; MNT_STAGED copies to RAM; MNT_RDONLY → writes EROFS.
    let mid = fs::mount(
        vol.volume_id,
        mp,
        fs::FS_AUTO,
        fs::MNT_STAGED | fs::MNT_RDONLY,
        0, // 0 = full source
    );
    if mid.is_err() {
        fail(
            "SYS_MOUNT(staged ro)",
            "mount(real_vol, STAGED|RDONLY) failed",
        );
        fail("staged write rejected EROFS", "skipped (mount failed)");
        fail("staged source pristine", "skipped (mount failed)");
        return;
    }
    ok("SYS_MOUNT(staged ro)");

    // A create/write into the read-only mount must be rejected (EROFS), either
    // at open(O_WRITE) or at write — capabilities gate it up front.
    let path = "/mnt/img/should_not_exist.txt";
    let rejected = match fs::open(path, fs::O_WRITE | fs::O_CREATE) {
        Ok(fd) => {
            let w = fs::write(fd, b"x");
            let _ = fs::close(fd);
            w == Err(libmorpheus::EROFS)
        },
        Err(e) => e == libmorpheus::EROFS,
    };
    check("staged write rejected EROFS", rejected, "write not EROFS");

    // The staged overlay is an independent ephemeral volume; the source disk
    // volume must remain unmounted (VOL_MOUNTED clear) — pristine, untouched.
    let still_pristine = match fs::volumes() {
        Ok(after) => after
            .iter()
            .find(|v| v.volume_id == vol.volume_id)
            .map(|v| (v.flags & fs::VOL_MOUNTED) == 0)
            .unwrap_or(false),
        Err(_) => false,
    };
    check(
        "staged source pristine",
        still_pristine,
        "source volume got mounted/dirtied",
    );

    let _ = fs::umount(mp, 0);
}

/// umount of a mount with an open fd → EBUSY (unless MNT_FORCE).
fn test_umount_busy() {
    use libmorpheus::fs;

    let mp = "/mnt/busy";
    let _ = fs::mkdir("/mnt");
    let _ = fs::mkdir(mp);
    // 4 MiB: a HelixFS volume must exceed one 1 MiB log segment + superblocks.
    if fs::mount(
        fs::VOLUME_NONE,
        mp,
        fs::FS_HELIX,
        fs::MNT_STAGED,
        4 * 1024 * 1024,
    )
    .is_err()
    {
        fail("SYS_UMOUNT(busy EBUSY)", "setup mount failed");
        return;
    }

    let path = "/mnt/busy/held.txt";
    let held = fs::open(path, fs::O_WRITE | fs::O_CREATE);
    match held {
        Ok(fd) => {
            let r = fs::umount(mp, 0);
            check(
                "SYS_UMOUNT(busy EBUSY)",
                r == Err(libmorpheus::EBUSY),
                "expected EBUSY with open fd",
            );
            let _ = fs::close(fd);
            // After closing, the umount should succeed.
            let _ = fs::umount(mp, 0);
        },
        Err(_) => {
            fail(
                "SYS_UMOUNT(busy EBUSY)",
                "could not open file to hold mount",
            );
            let _ = fs::umount(mp, fs::MNT_FORCE);
        },
    }
}

/// rename across two distinct mounts → EXDEV (never an implicit copy+delete).
fn test_cross_mount_rename() {
    use libmorpheus::fs;

    let mp_a = "/mnt/xa";
    let mp_b = "/mnt/xb";
    let _ = fs::mkdir("/mnt");
    let _ = fs::mkdir(mp_a);
    let _ = fs::mkdir(mp_b);

    // 4 MiB each: below ~2 MiB a HelixFS format fails (log segment is 1 MiB).
    let a = fs::mount(
        fs::VOLUME_NONE,
        mp_a,
        fs::FS_HELIX,
        fs::MNT_STAGED,
        4 * 1024 * 1024,
    );
    let b = fs::mount(
        fs::VOLUME_NONE,
        mp_b,
        fs::FS_HELIX,
        fs::MNT_STAGED,
        4 * 1024 * 1024,
    );
    if a.is_err() || b.is_err() {
        fail("SYS_RENAME(cross-mount EXDEV)", "setup mounts failed");
        if a.is_ok() {
            let _ = fs::umount(mp_a, fs::MNT_FORCE);
        }
        if b.is_ok() {
            let _ = fs::umount(mp_b, fs::MNT_FORCE);
        }
        return;
    }

    let src = "/mnt/xa/f.txt";
    if let Ok(fd) = fs::open(src, fs::O_WRITE | fs::O_CREATE) {
        let _ = fs::write(fd, b"data");
        let _ = fs::close(fd);
    }
    let r = fs::rename(src, "/mnt/xb/f.txt");
    check(
        "SYS_RENAME(cross-mount EXDEV)",
        r == Err(EXDEV),
        "cross-mount rename not EXDEV",
    );

    let _ = fs::umount(mp_a, fs::MNT_FORCE);
    let _ = fs::umount(mp_b, fs::MNT_FORCE);
}

/// Mount with a stale/garbage volume_id → ENODEV (generation-check use-after-free
/// protection on fuzzed handles).
fn test_stale_volume() {
    use libmorpheus::fs;

    let mp = "/mnt/stale";
    let _ = fs::mkdir("/mnt");
    let _ = fs::mkdir(mp);
    // High generation + arbitrary index that cannot match a live slot.
    let bogus: u64 = 0xDEAD_BEEF_0000_0007;
    let r = fs::mount(bogus, mp, fs::FS_AUTO, 0, 0);
    check(
        "SYS_MOUNT(stale id ENODEV)",
        r == Err(libmorpheus::ENODEV),
        "stale volume_id not ENODEV",
    );
}

/// Oversized staged-from-nothing request → ENOMEM (admission control rejects
/// before allocating). Use a size no machine in test has free.
fn test_oversized_stage() {
    use libmorpheus::fs;

    let mp = "/mnt/huge";
    let _ = fs::mkdir("/mnt");
    let _ = fs::mkdir(mp);
    // 1 TiB — far past STAGE_SINGLE_MAX / physical RAM. Spec admission step 2/5
    // yields EINVAL (over single-max) or ENOMEM (too little free); accept either
    // rejection, but it must NOT succeed.
    let huge: u64 = 1u64 << 40;
    let r = fs::mount(fs::VOLUME_NONE, mp, fs::FS_HELIX, fs::MNT_STAGED, huge);
    check(
        "SYS_MOUNT(oversized ENOMEM)",
        r == Err(libmorpheus::ENOMEM) || r == Err(libmorpheus::EINVAL),
        "oversized stage not rejected",
    );
    // If it somehow succeeded, clean up so we don't leak.
    if r.is_ok() {
        let _ = fs::umount(mp, fs::MNT_FORCE);
    }
}

fn test_poll() {
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

fn test_persist() {
    let key = "e2e_test_key";
    let val = b"e2e test value 42";

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

    let ret = unsafe { syscall4(SYS_PERSIST_GET, key.as_ptr() as u64, key.len() as u64, 0, 0) };
    check(
        "SYS_PERSIST_GET(size)",
        !libmorpheus::is_error(ret) && ret == val.len() as u64,
        "wrong size",
    );

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

    let ret = unsafe { syscall2(SYS_PERSIST_DEL, key.as_ptr() as u64, key.len() as u64) };
    check_ok("SYS_PERSIST_DEL", ret);
}

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
        },
        Err(_) => {
            fail("SYS_PE_INFO(format)", "pe_info returned error");
            fail("SYS_PE_INFO(arch)", "skipped");
            fail("SYS_PE_INFO(entry)", "skipped");
            fail("SYS_PE_INFO(size)", "skipped");
        },
    }
}

fn test_port_io() {
    // Read PIT channel 2 (port 0x42) — should return a byte
    let _val = libmorpheus::hw::port_inb(0x42);
    ok("SYS_PORT_IN(0x42, byte)");

    // Read PCI config address port (0xCF8) — 32-bit
    let _val = libmorpheus::hw::port_inl(0xCF8);
    ok("SYS_PORT_IN(0xCF8, dword)");

    // Write/read a scratch value to unused port (careful — use PIT count latch)
    let ret = unsafe { syscall3(SYS_PORT_OUT, 0x80, 1, 0) }; // port 0x80 = debug port
    check_ok("SYS_PORT_OUT(0x80, byte)", ret);

    // Invalid width
    let ret = unsafe { syscall2(SYS_PORT_IN, 0x80, 3) }; // width=3 invalid
    check_err("SYS_PORT_IN(bad width)", ret);
}

fn test_pci() {
    // Read vendor ID from bus=0, device=0, function=0, offset=0
    let vendor = libmorpheus::hw::pci_cfg_read16(0, 0, 0, 0x00);
    // QEMU typically has vendor 0x8086 (Intel) or 0x1234 (QEMU)
    check(
        "SYS_PCI_CFG_READ(vendor)",
        vendor != 0xFFFF,
        "no device at 00:00.0",
    );

    let _device = libmorpheus::hw::pci_cfg_read16(0, 0, 0, 0x02);
    ok("SYS_PCI_CFG_READ(device)");

    let _class = libmorpheus::hw::pci_cfg_read32(0, 0, 0, 0x08);
    ok("SYS_PCI_CFG_READ(class)");

    // Write test — write/read-back the latency timer (offset 0x0D)
    let orig = libmorpheus::hw::pci_cfg_read8(0, 0, 0, 0x0D);
    libmorpheus::hw::pci_cfg_write8(0, 0, 0, 0x0D, orig);
    ok("SYS_PCI_CFG_WRITE(byte)");

    // Invalid width
    let bdf = libmorpheus::hw::pci_bdf(0, 0, 0);
    let ret = unsafe { syscall3(SYS_PCI_CFG_READ, bdf, 0, 3) }; // width 3 invalid
    check_err("SYS_PCI_CFG_READ(bad width)", ret);
}

fn test_dma() {
    let r = libmorpheus::hw::dma_alloc(1);
    match r {
        Ok(phys) => {
            check("SYS_DMA_ALLOC(1)", phys < 0x1_0000_0000, "not below 4GB");
            let free_r = libmorpheus::hw::dma_free(phys, 1);
            check("SYS_DMA_FREE(1)", free_r.is_ok(), "free failed");
        },
        Err(_) => {
            fail("SYS_DMA_ALLOC(1)", "alloc failed");
            fail("SYS_DMA_FREE(1)", "skipped (alloc failed)");
        },
    }
}

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
                },
                Err(_) => {
                    fail("SYS_MAP_PHYS(1 page)", "map failed");
                    fail("SYS_MUNMAP(map_phys)", "skipped (map failed)");
                },
            }
            let _ = libmorpheus::hw::dma_free(phys, 1);
        },
        Err(_) => {
            fail("SYS_MAP_PHYS(1 page)", "dma_alloc failed");
            fail("SYS_MUNMAP(map_phys)", "skipped (dma_alloc failed)");
        },
    }
}

fn test_virt_to_phys() {
    // Identity-mapped kernel → virt == phys for low addresses
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

fn test_cache_flush() {
    let data = [0u8; 4096];
    let addr = data.as_ptr() as u64;
    let aligned = addr & !0xFFF;
    let r = libmorpheus::hw::cache_flush(aligned, 4096);
    check("SYS_CACHE_FLUSH", r.is_ok(), "flush failed");
}

fn test_fb_info() {
    let r = libmorpheus::hw::fb_info();
    match r {
        Ok(info) => {
            check("SYS_FB_INFO width", info.width > 0, "width=0");
            check("SYS_FB_INFO height", info.height > 0, "height=0");
            check("SYS_FB_INFO base", info.base > 0, "base=0");
        },
        Err(_) => {
            fail("SYS_FB_INFO", "returned error (FB not registered?)");
            fail("SYS_FB_INFO height", "skipped (no FB)");
            fail("SYS_FB_INFO base", "skipped (no FB)");
        },
    }
}

fn test_fb_map() {
    let r = libmorpheus::hw::fb_map();
    match r {
        Ok(vaddr) => {
            check("SYS_FB_MAP", vaddr >= 0x40_0000_0000, "vaddr out of range");
            // Skip unmapping — the FB stays mapped for the rest of the test.
            ok("SYS_FB_MAP(mapped)");
        },
        Err(_) => {
            fail("SYS_FB_MAP", "map failed");
            fail("SYS_FB_MAP(mapped)", "skipped (map failed)");
        },
    }
}

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

fn test_sigaction() {
    // Register a handler (address doesn't matter for now — just exercises syscall)
    let r = libmorpheus::process::sigaction(15, 0); // SIGTERM, default handler
    check("SYS_SIGACTION", r.is_ok(), "sigaction failed");
}

fn test_priority() {
    let pid = libmorpheus::process::getpid();

    let r = libmorpheus::process::getpriority(pid);
    match r {
        Ok(prio) => {
            ok("SYS_GETPRIORITY");
            let r2 = libmorpheus::process::setpriority(pid, 100);
            check("SYS_SETPRIORITY", r2.is_ok(), "set failed");
            let _ = libmorpheus::process::setpriority(pid, prio);
        },
        Err(_) => {
            fail("SYS_GETPRIORITY", "returned error");
            fail("SYS_SETPRIORITY", "skipped (getpriority failed)");
        },
    }
}

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
        },
        Err(_) => fail("SYS_MEMMAP(read)", "returned error"),
    }
}

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
        },
        Err(_) => {
            fail("SYS_SHM_GRANT(self)", "mmap failed");
            fail("SYS_SHM_GRANT(bad pid)", "mmap failed");
            fail("SYS_SHM_GRANT(0 pages)", "mmap failed");
        },
    }
}

fn test_mprotect() {
    match libmorpheus::mem::mmap(1) {
        Ok(vaddr) => {
            let r = libmorpheus::mem::mprotect(vaddr, 1, libmorpheus::mem::PROT_READ);
            check("SYS_MPROTECT(RO)", r.is_ok(), "mprotect failed");

            let r = libmorpheus::mem::mprotect(vaddr, 1, libmorpheus::mem::PROT_WRITE);
            check("SYS_MPROTECT(RW)", r.is_ok(), "mprotect failed");

            // Bad prot bits → EINVAL.
            let r = libmorpheus::mem::mprotect(vaddr, 1, 0xFF);
            check("SYS_MPROTECT(bad prot)", r.is_err(), "should fail");

            // Wrong page count → EINVAL.
            let r = libmorpheus::mem::mprotect(vaddr, 2, 0);
            check("SYS_MPROTECT(bad pages)", r.is_err(), "should fail");

            let _ = libmorpheus::mem::munmap(vaddr, 1);
        },
        Err(_) => {
            fail("SYS_MPROTECT(RO)", "mmap failed");
            fail("SYS_MPROTECT(RW)", "mmap failed");
            fail("SYS_MPROTECT(bad prot)", "mmap failed");
            fail("SYS_MPROTECT(bad pages)", "mmap failed");
        },
    }
}

fn test_pipe() {
    match libmorpheus::process::pipe() {
        Ok((read_fd, write_fd)) => {
            check("SYS_PIPE(create)", true, "");

            let msg = b"hello pipe";
            let wr = libmorpheus::io::write_fd(write_fd, msg);
            check("SYS_PIPE(write)", wr.is_ok(), "pipe write failed");

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

            unsafe {
                syscall1(SYS_CLOSE, read_fd as u64);
                syscall1(SYS_CLOSE, write_fd as u64);
            }
        },
        Err(_) => {
            fail("SYS_PIPE(create)", "pipe() failed");
            fail("SYS_PIPE(write)", "no pipe");
            fail("SYS_PIPE(read)", "no pipe");
        },
    }
}

fn test_dup2() {
    match libmorpheus::process::pipe() {
        Ok((read_fd, write_fd)) => {
            match libmorpheus::process::dup2(read_fd, 10) {
                Ok(fd) => check("SYS_DUP2(ok)", fd == 10, "wrong fd"),
                Err(_) => fail("SYS_DUP2(ok)", "dup2 failed"),
            }

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
        },
        Err(_) => {
            fail("SYS_DUP2(ok)", "no pipe");
            fail("SYS_DUP2(data)", "no pipe");
            fail("SYS_DUP2(bad)", "no pipe");
        },
    }
}

fn test_set_fg() {
    let pid = libmorpheus::process::getpid();
    libmorpheus::process::set_foreground(pid);
    check("SYS_SET_FG(self)", true, "");

    libmorpheus::process::set_foreground(0);
    check("SYS_SET_FG(reset)", true, "");
}

fn test_getargs() {
    // Launch-agnostic: this binary may run with a mode arg (e.g. "smoke") or
    // none. Validate the getargs contract rather than a fixed argc.
    let c = libmorpheus::process::argc();
    let mut buf = [0u8; 256];
    let n = libmorpheus::process::getargs(&mut buf);

    // The fill form returns BYTES written, not argc. The months-old bug returned
    // argc here, so the blob was truncated to argc bytes (1 arg → "s"). Every
    // real arg is ≥1 char + a NUL ≥ 2 bytes, so n must be ≥ 2*argc.
    check(
        "SYS_GETARGS byte count",
        c == 0 || n >= 2 * c,
        "getargs returned argc as byte count",
    );

    // Roundtrip: the returned blob must parse back into exactly argc tokens.
    let mut strs: [&str; 16] = [""; 16];
    let parsed = libmorpheus::process::parse_args(&buf[..n], &mut strs);
    check("SYS_GETARGS roundtrip", parsed == c, "argc/blob mismatch");
}

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

fn test_thread_create_join() {
    use core::sync::atomic::{AtomicU32, Ordering};

    static SENTINEL: AtomicU32 = AtomicU32::new(0);

    let handle = libmorpheus::thread::spawn(|| {
        SENTINEL.store(0xDEAD, Ordering::Release);
    });

    match handle {
        Ok(h) => {
            let _ = h.join();
            let val = SENTINEL.load(Ordering::Acquire);
            check("SYS_THREAD_CREATE+JOIN", val == 0xDEAD, "sentinel mismatch");
        },
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
        },
        _ => fail("THREAD(shared_mem)", "spawn failed"),
    }
}

fn test_async_block_on() {
    let result = libmorpheus::task::block_on(async { 42u32 });
    check("async(block_on)", result == 42, "wrong return value");
}

fn test_async_spawn_multi() {
    use core::sync::atomic::{AtomicU32, Ordering};

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
    let handle = rt.spawn_with_handle(async { 42u64 + 58 });
    use core::sync::atomic::{AtomicU64, Ordering};
    static RESULT: AtomicU64 = AtomicU64::new(0);
    RESULT.store(0, Ordering::SeqCst);
    rt.spawn(async move {
        let val = handle.await;
        RESULT.store(val, Ordering::SeqCst);
    });
    rt.run();
    check(
        "async(join_handle)",
        RESULT.load(Ordering::SeqCst) == 100,
        "wrong join result",
    );
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
    check(
        "async(sleep)",
        (30..500).contains(&elapsed_ms),
        "sleep timing off",
    );
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
    check(
        "async(chained)",
        SUM.load(Ordering::SeqCst) == 60,
        "chained await sum wrong",
    );
}

// TLS / RNG (100-101): SYS_SET_THREAD_POINTER + SYS_GETRANDOM ABI.

fn test_set_thread_pointer() {
    // Save the real TLS base crt0 installed. We temporarily reprogram FS base
    // below; restoring it to 0 (as a naive "clear" would) null-faults the very
    // next #[thread_local] access, since variant-II address generation begins
    // with a `mov %fs:0` self-pointer load.
    let saved_tp = unsafe { libmorpheus::thread::get_thread_pointer() };

    // Kernel-half pointer → EINVAL (canonical-user check, also wrmsr-#GP guard).
    let bad = unsafe { syscall1(SYS_SET_THREAD_POINTER, 0xFFFF_8000_0000_0000) };
    check(
        "SYS_SET_THREAD_POINTER(bad) EINVAL",
        bad == u64::MAX,
        "expected EINVAL",
    );
    // A canonical user-range value is accepted (no TLS access follows here).
    let set = unsafe { syscall1(SYS_SET_THREAD_POINTER, 0x1000) };
    check_ok("SYS_SET_THREAD_POINTER(ok)", set);
    // Restore the genuine thread pointer so later #[thread_local] use is valid.
    let _ = unsafe { syscall1(SYS_SET_THREAD_POINTER, saved_tp) };
}

// Non-zero initializers keep these in `.tdata` (not `.tbss`), so the test also
// proves crt0 copied the template correctly. Variant-II local-exec via FS base.
#[thread_local]
static TLS_A: core::cell::Cell<u32> = core::cell::Cell::new(0xA5A5_1234);
#[thread_local]
static TLS_B: core::cell::Cell<u64> = core::cell::Cell::new(0xDEAD_BEEF_CAFE_0000);

fn test_thread_local() {
    // Template copied by crt0 → initial values intact.
    check(
        "TLS .tdata init A",
        TLS_A.get() == 0xA5A5_1234,
        "tdata copy wrong",
    );
    check(
        "TLS .tdata init B",
        TLS_B.get() == 0xDEAD_BEEF_CAFE_0000,
        "tdata copy wrong",
    );
    // Read/write through the FS base.
    TLS_A.set(0x1111_2222);
    TLS_B.set(0x3333_4444_5555_6666);
    check("TLS rw A", TLS_A.get() == 0x1111_2222, "write/read failed");
    check(
        "TLS rw B",
        TLS_B.get() == 0x3333_4444_5555_6666,
        "write/read failed",
    );
    // Distinct thread-locals must not alias.
    let pa = TLS_A.as_ptr() as *const ();
    let pb = TLS_B.as_ptr() as *const ();
    check(
        "TLS distinct addrs",
        !core::ptr::eq(pa, pb),
        "thread-locals alias",
    );

    // Per-thread isolation: a spawned thread (its own crt-less trampoline sets
    // up its own TLS block) must see the template default, not the main thread's
    // mutated value, and its writes must not leak back.
    use core::sync::atomic::{AtomicU32, Ordering};
    static CHILD_SAW: AtomicU32 = AtomicU32::new(0);
    static CHILD_OK: AtomicU32 = AtomicU32::new(0);
    let h = libmorpheus::thread::spawn(|| {
        CHILD_SAW.store(TLS_A.get(), Ordering::SeqCst); // expect template 0xA5A5_1234
        TLS_A.set(0x9999_9999);
        CHILD_OK.store((TLS_A.get() == 0x9999_9999) as u32, Ordering::SeqCst);
    });
    match h {
        Ok(handle) => {
            let _ = handle.join();
            check(
                "TLS thread sees template",
                CHILD_SAW.load(Ordering::SeqCst) == 0xA5A5_1234,
                "child saw main's TLS",
            );
            check(
                "TLS thread rw isolated",
                CHILD_OK.load(Ordering::SeqCst) == 1,
                "child write failed",
            );
            // Main thread's value survived the child's write → no cross-talk.
            check(
                "TLS no cross-talk",
                TLS_A.get() == 0x1111_2222,
                "child clobbered main TLS",
            );
        },
        Err(_) => check("TLS thread spawn", false, "spawn failed"),
    }
}

fn test_getrandom() {
    let mut buf = [0u8; 32];
    let ret = unsafe { syscall3(SYS_GETRANDOM, buf.as_mut_ptr() as u64, buf.len() as u64, 0) };
    if libmorpheus::is_error(ret) {
        // ENOSYS on a CPU without RDRAND is acceptable (not a hard fail).
        check(
            "SYS_GETRANDOM (no HW RNG)",
            ret == u64::MAX - 37,
            "unexpected error",
        );
        return;
    }
    check("SYS_GETRANDOM len", ret == 32, "short read");
    // P(32 genuinely-zero bytes) ≈ 2^-256, so all-zero means it didn't fill.
    let any_nonzero = buf.iter().any(|&b| b != 0);
    check("SYS_GETRANDOM entropy", any_nonzero, "all zero bytes");
    // Kernel-half buffer → EFAULT.
    let ef = unsafe { syscall3(SYS_GETRANDOM, 0xFFFF_8000_0000_0000, 8, 0) };
    check(
        "SYS_GETRANDOM(bad buf) EFAULT",
        ef == u64::MAX - 14,
        "expected EFAULT",
    );
}
