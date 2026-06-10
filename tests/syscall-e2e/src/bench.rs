//! Torture/soak bench spine: workload registry, shared `Stats`, seeded `Rng`,
//! and the `threads` driver (Approach A). Modes share this spine; see
//! docs/superpowers/specs/2026-06-08-syscall-torture-soak-design.md.

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

use libmorpheus::io::{println, read_fd, write_fd};
use libmorpheus::{fs, mem, process, thread, time};

use crate::emit_line;

/// Set by the SIGINT handler (Ctrl-C from the shell) to stop all workers.
static STOP: AtomicBool = AtomicBool::new(false);

/// SIGINT handler: request stop and return. No allocation (signal-handler rule).
extern "C" fn on_sigint() {
    STOP.store(true, Relaxed);
    process::sigreturn();
}

/// Workload categories — index into the `Stats` counter arrays.
#[allow(dead_code)] // not every category has ops yet; the set grows incrementally
#[derive(Clone, Copy)]
pub enum Cat {
    Fs = 0,
    Mem,
    Ipc,
    Sync,
    Hw,
    Proc,
    Tls,
    Rng,
}

pub const N_CAT: usize = 8;
const CAT_NAMES: [&str; N_CAT] = ["fs", "mem", "ipc", "sync", "hw", "proc", "tls", "rng"];

/// Shared counters. `#[repr(C)]` + atomics so the identical type can live either
/// in-process (threads mode) or in a shared page (swarm mode, later).
#[repr(C)]
pub struct Stats {
    pub ops: AtomicU64,
    pub ok: [AtomicU64; N_CAT],
    pub fail: [AtomicU64; N_CAT],
    pub started: AtomicU32,
}

impl Stats {
    pub const fn new() -> Self {
        Self {
            ops: AtomicU64::new(0),
            ok: [const { AtomicU64::new(0) }; N_CAT],
            fail: [const { AtomicU64::new(0) }; N_CAT],
            started: AtomicU32::new(0),
        }
    }

    #[inline]
    fn record(&self, cat: Cat, ok: bool) {
        self.ops.fetch_add(1, Relaxed);
        let arr = if ok { &self.ok } else { &self.fail };
        arr[cat as usize].fetch_add(1, Relaxed);
    }

    fn total_fail(&self) -> u64 {
        self.fail.iter().map(|c| c.load(Relaxed)).sum()
    }
}

/// Deterministic xorshift64. Seeded per worker so runs replay from one seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1) // avoid the all-zero fixed point
    }
    #[inline]
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// One torture operation: does a syscall, validates the result, records outcome.
pub struct Op {
    pub name: &'static str,
    pub cat: Cat,
    pub run: fn(&mut Rng, &Stats),
}

// ── Starter registry ──────────────────────────────────────────────────────
// Concurrency-safe, self-contained, self-releasing ops that hammer the contended
// paths (page allocator, paging, scheduler). More categories (fs/ipc/sync/...)
// land in later increments.

fn op_alloc_free(_rng: &mut Rng, stats: &Stats) {
    match mem::alloc_pages(1) {
        Ok(phys) => {
            let freed = mem::free_pages(phys, 1).is_ok();
            stats.record(Cat::Mem, freed);
        },
        // OOM is a legitimate transient under contention, not a correctness fail.
        Err(_) => stats.record(Cat::Mem, true),
    }
}

fn op_mmap_munmap(_rng: &mut Rng, stats: &Stats) {
    match mem::mmap(1) {
        Ok(vaddr) => {
            // Touch the page: it must be present and writable.
            unsafe { core::ptr::write_volatile(vaddr as *mut u64, 0xA5A5_5A5A) };
            let ok = unsafe { core::ptr::read_volatile(vaddr as *const u64) } == 0xA5A5_5A5A;
            let unmapped = mem::munmap(vaddr, 1).is_ok();
            stats.record(Cat::Mem, ok && unmapped);
        },
        Err(_) => stats.record(Cat::Mem, true),
    }
}

fn op_mprotect(_rng: &mut Rng, stats: &Stats) {
    const PROT_READ: u64 = 0x1;
    const PROT_WRITE: u64 = 0x2;
    match mem::mmap(1) {
        Ok(vaddr) => {
            let ro = mem::mprotect(vaddr, 1, PROT_READ).is_ok();
            let rw = mem::mprotect(vaddr, 1, PROT_READ | PROT_WRITE).is_ok();
            let unmapped = mem::munmap(vaddr, 1).is_ok();
            stats.record(Cat::Mem, ro && rw && unmapped);
        },
        Err(_) => stats.record(Cat::Mem, true),
    }
}

fn op_yield(_rng: &mut Rng, stats: &Stats) {
    process::yield_cpu();
    stats.record(Cat::Proc, true);
}

fn op_getppid(_rng: &mut Rng, stats: &Stats) {
    // Spawned by the shell → parent pid must be non-zero and stable.
    stats.record(Cat::Proc, process::getppid() != 0);
}

fn op_clock_monotonic(_rng: &mut Rng, stats: &Stats) {
    // Two back-to-back reads must be non-decreasing.
    let a = time::clock_gettime();
    let b = time::clock_gettime();
    stats.record(Cat::Hw, b >= a);
}

static REGISTRY: &[Op] = &[
    Op {
        name: "mem.alloc_free",
        cat: Cat::Mem,
        run: op_alloc_free,
    },
    Op {
        name: "mem.mmap_munmap",
        cat: Cat::Mem,
        run: op_mmap_munmap,
    },
    Op {
        name: "mem.mprotect",
        cat: Cat::Mem,
        run: op_mprotect,
    },
    Op {
        name: "proc.yield",
        cat: Cat::Proc,
        run: op_yield,
    },
    Op {
        name: "proc.getppid",
        cat: Cat::Proc,
        run: op_getppid,
    },
    Op {
        name: "hw.clock_monotonic",
        cat: Cat::Hw,
        run: op_clock_monotonic,
    },
];

// ── threads driver (Approach A) ────────────────────────────────────────────

/// Run N worker threads hammering the registry for `secs` seconds, printing a
/// heartbeat every ~2s and a final summary. (Increment 2: bounded by time;
/// Ctrl-C/SIGINT stop lands in a later increment.)
pub fn run_threads(n: usize, secs: u64, seed: u64) -> i32 {
    static STATS: Stats = Stats::new();

    let n = n.max(1);
    // `secs == 0` means run until Ctrl-C (the user-invoked/user-closed model).
    let mut secsbuf = [0u8; 20];
    let secs_str = if secs == 0 {
        "until-ctrl-c"
    } else {
        fmt_u64(secs, &mut secsbuf)
    };
    emit_line(&[
        "[bench] threads n=",
        fmt_u64(n as u64, &mut [0u8; 20]),
        " secs=",
        secs_str,
        " seed=",
        fmt_hex(seed, &mut [0u8; 18]),
    ]);
    for op in REGISTRY {
        emit_line(&["  op ", op.name, " [", CAT_NAMES[op.cat as usize], "]"]);
    }

    // Take over Ctrl-C so the user can stop a soak cleanly and get a summary.
    STOP.store(false, Relaxed);
    process::set_foreground(process::getpid());
    let _ = process::sigaction(process::signal::SIGINT, on_sigint as *const () as u64);

    let start = time::clock_gettime();
    let deadline = if secs == 0 {
        u64::MAX
    } else {
        start.saturating_add(secs.saturating_mul(1_000_000_000))
    };

    let mut handles = Vec::new();
    for i in 0..n {
        let wseed = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        match thread::spawn(move || worker(wseed, deadline, &STATS)) {
            Ok(h) => handles.push(h),
            Err(_) => emit_line(&[
                "[bench] worker spawn failed at i=",
                fmt_u64(i as u64, &mut [0u8; 20]),
            ]),
        }
    }

    // Heartbeat on the main thread until the deadline or Ctrl-C.
    let mut last_ops = 0u64;
    loop {
        thread::sleep_ms(2000);
        let ops = STATS.ops.load(Relaxed);
        let delta = ops.wrapping_sub(last_ops);
        last_ops = ops;
        emit_line(&[
            "[hb] ops=",
            fmt_u64(ops, &mut [0u8; 20]),
            " +",
            fmt_u64(delta, &mut [0u8; 20]),
            "/2s fail=",
            fmt_u64(STATS.total_fail(), &mut [0u8; 20]),
        ]);
        if STOP.load(Relaxed) || time::clock_gettime() >= deadline {
            break;
        }
    }

    for h in handles {
        let _ = h.join();
    }

    summary(&STATS, seed)
}

// ── swarm driver (Approach C) ──────────────────────────────────────────────

/// Default deploy path of this binary (setup-dev.sh maps it here). The swarm
/// re-execs this same binary as worker children; overridable via `--self`.
pub const SELF_PATH: &str = "/bin/syscall-e2e";

const PROT_RW: u64 = 0x3; // READ | WRITE, for the granted stats page

/// Run N child PROCESSES hammering the registry into one shared `Stats` page.
///
/// Bootstrap per child: create an inherited pipe → `spawn` self as `_worker` →
/// `shm_grant` the stats page (kernel maps it at the child's mmap_brk, which we
/// can't predict because crt0 already mmap'd TLS) → hand the child the grant's
/// returned address over the pipe. The child's blocking pipe read is the
/// go-gate: it can't touch the page until the address (and thus the mapping)
/// arrives.
pub fn run_swarm(n: usize, secs: u64, seed: u64, self_path: &str) -> i32 {
    let n = n.max(1);

    // A freshly mmap'd page is zeroed, and an all-zero `Stats` is a valid initial
    // state (every atomic == 0), so we can treat the page as `&Stats` directly.
    let page = match mem::mmap(1) {
        Ok(v) => v,
        Err(_) => {
            println("[bench] swarm: stats mmap failed");
            return 2;
        },
    };
    let stats: &Stats = unsafe { &*(page as *const Stats) };

    let mut secsbuf = [0u8; 20];
    let secs_str = if secs == 0 {
        "until-ctrl-c"
    } else {
        fmt_u64(secs, &mut secsbuf)
    };
    emit_line(&[
        "[bench] swarm n=",
        fmt_u64(n as u64, &mut [0u8; 20]),
        " secs=",
        secs_str,
        " seed=",
        fmt_hex(seed, &mut [0u8; 18]),
    ]);

    STOP.store(false, Relaxed);
    process::set_foreground(process::getpid());
    let _ = process::sigaction(process::signal::SIGINT, on_sigint as *const () as u64);

    let mut pids: Vec<u32> = Vec::new();
    for i in 0..n {
        let (rfd, wfd) = match process::pipe() {
            Ok(p) => p,
            Err(_) => {
                emit_line(&[
                    "[bench] swarm: pipe failed at i=",
                    fmt_u64(i as u64, &mut [0u8; 20]),
                ]);
                break;
            },
        };
        let wseed = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut sb = [0u8; 18];
        let mut cb = [0u8; 20];
        let mut rb = [0u8; 20];
        let argv = [
            "_worker",
            fmt_hex(wseed, &mut sb),
            fmt_u64(secs, &mut cb),
            fmt_u64(rfd as u64, &mut rb),
        ];

        let pid = match process::spawn_with_args(self_path, &argv) {
            Ok(p) => p,
            Err(_) => {
                emit_line(&["[bench] swarm: spawn failed (path ", self_path, ")"]);
                let _ = fs::close(rfd as usize);
                let _ = fs::close(wfd as usize);
                continue;
            },
        };

        // Map the shared page into the child; hand it the address (0 on failure so
        // the child unblocks and exits cleanly instead of hanging on the read).
        let child_vaddr = mem::shm_grant(pid, page, 1, PROT_RW).unwrap_or(0);
        let _ = write_fd(wfd, &child_vaddr.to_ne_bytes());

        // Parent keeps the shared page, not the pipe (the child has its own rfd copy).
        let _ = fs::close(rfd as usize);
        let _ = fs::close(wfd as usize);
        pids.push(pid);
    }

    // Heartbeat off the shared page until Ctrl-C or all children exit.
    let mut last_ops = 0u64;
    loop {
        thread::sleep_ms(2000);
        let ops = stats.ops.load(Relaxed);
        let delta = ops.wrapping_sub(last_ops);
        last_ops = ops;
        let mut alive = 0u64;
        for &pid in &pids {
            if let Ok(None) = process::try_wait(pid) {
                alive += 1;
            }
        }
        emit_line(&[
            "[hb] ops=",
            fmt_u64(ops, &mut [0u8; 20]),
            " +",
            fmt_u64(delta, &mut [0u8; 20]),
            "/2s fail=",
            fmt_u64(stats.total_fail(), &mut [0u8; 20]),
            " alive=",
            fmt_u64(alive, &mut [0u8; 20]),
        ]);
        if STOP.load(Relaxed) || alive == 0 {
            break;
        }
    }

    // Stop and reap every child. A child that exited abnormally (faulted) is a
    // failure of the OS under load — count it.
    let mut abnormal = 0u64;
    for &pid in &pids {
        let _ = process::kill(pid, process::signal::SIGKILL);
    }
    for &pid in &pids {
        match process::wait(pid) {
            Ok(code) if code != 0 => abnormal += 1,
            Err(_) => abnormal += 1,
            _ => {},
        }
    }
    if abnormal != 0 {
        emit_line(&[
            "[bench] swarm: abnormal child exits=",
            fmt_u64(abnormal, &mut [0u8; 20]),
        ]);
    }

    summary(stats, seed)
}

/// Hidden swarm child sub-mode: receive the shared `Stats` address over the
/// inherited pipe `rfd`, then hammer the registry into it until `secs` elapses
/// (or the parent kills us). Not user-facing.
pub fn run_worker(seed: u64, secs: u64, rfd: u32) -> i32 {
    // Blocking read of the 8-byte shared-page address = our go-gate.
    let mut buf = [0u8; 8];
    let mut got = 0usize;
    while got < 8 {
        match read_fd(rfd, &mut buf[got..]) {
            Ok(0) => return 2, // pipe closed before handshake
            Ok(r) => got += r,
            Err(_) => return 2,
        }
    }
    let vaddr = u64::from_ne_bytes(buf);
    if vaddr == 0 {
        return 2; // parent's grant failed
    }
    let stats: &Stats = unsafe { &*(vaddr as *const Stats) };
    let deadline = if secs == 0 {
        u64::MAX
    } else {
        time::clock_gettime().saturating_add(secs.saturating_mul(1_000_000_000))
    };
    worker(seed, deadline, stats);
    0
}

fn worker(seed: u64, deadline: u64, stats: &Stats) {
    stats.started.fetch_add(1, Relaxed);
    let mut rng = Rng::new(seed);
    while !STOP.load(Relaxed) && time::clock_gettime() < deadline {
        let op = &REGISTRY[(rng.next() as usize) % REGISTRY.len()];
        (op.run)(&mut rng, stats);
    }
}

fn summary(stats: &Stats, seed: u64) -> i32 {
    let fails = stats.total_fail();
    println("════════════════════════════════════════════");
    emit_line(&[
        "[bench] done ops=",
        fmt_u64(stats.ops.load(Relaxed), &mut [0u8; 20]),
        " fail=",
        fmt_u64(fails, &mut [0u8; 20]),
        " seed=",
        fmt_hex(seed, &mut [0u8; 18]),
    ]);
    for c in 0..N_CAT {
        let ok = stats.ok[c].load(Relaxed);
        let fl = stats.fail[c].load(Relaxed);
        if ok == 0 && fl == 0 {
            continue; // category not exercised
        }
        emit_line(&[
            "  ",
            CAT_NAMES[c],
            ": ok=",
            fmt_u64(ok, &mut [0u8; 20]),
            " fail=",
            fmt_u64(fl, &mut [0u8; 20]),
        ]);
    }
    println("════════════════════════════════════════════");
    // Exit code saturates at 255 (single byte status).
    fails.min(255) as i32
}

// ── small no_std formatters (single-write friendly) ─────────────────────────

/// Decimal-format `v` into `buf`, returning the populated slice as `&str`.
pub fn fmt_u64(v: u64, buf: &mut [u8; 20]) -> &str {
    if v == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut n = v;
    let mut i = 20usize;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let len = 20 - i;
    buf.copy_within(i..20, 0);
    unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
}

/// `0x`-prefixed 16-digit hex of `v` into `buf` (18 bytes).
pub fn fmt_hex(v: u64, buf: &mut [u8; 18]) -> &str {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        buf[2 + i] = HEX[((v >> (60 - i * 4)) & 0xF) as usize];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..]) }
}
