//! Raw syscall wrappers — thin inline asm around the `syscall` instruction.

// ═══════════════════════════════════════════════════════════════════════════
// Syscall numbers (must match hwinit/src/syscall/mod.rs)
// ═══════════════════════════════════════════════════════════════════════════

// ── Core (0-9) ───────────────────────────────────────────────────────
pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_YIELD: u64 = 3;
pub const SYS_ALLOC: u64 = 4;
pub const SYS_FREE: u64 = 5;
pub const SYS_GETPID: u64 = 6;
pub const SYS_KILL: u64 = 7;
pub const SYS_WAIT: u64 = 8;
pub const SYS_SLEEP: u64 = 9;

// ── HelixFS (10-21) ─────────────────────────────────────────────────
pub const SYS_OPEN: u64 = 10;
pub const SYS_CLOSE: u64 = 11;
pub const SYS_SEEK: u64 = 12;
pub const SYS_STAT: u64 = 13;
pub const SYS_READDIR: u64 = 14;
pub const SYS_MKDIR: u64 = 15;
pub const SYS_UNLINK: u64 = 16;
pub const SYS_RENAME: u64 = 17;
pub const SYS_TRUNCATE: u64 = 18;
pub const SYS_SYNC: u64 = 19;
pub const SYS_SNAPSHOT: u64 = 20;
pub const SYS_VERSIONS: u64 = 21;

// ── System / process / memory (22-31) ────────────────────────────────
pub const SYS_CLOCK: u64 = 22;
pub const SYS_SYSINFO: u64 = 23;
pub const SYS_GETPPID: u64 = 24;
pub const SYS_SPAWN: u64 = 25;
pub const SYS_MMAP: u64 = 26;
pub const SYS_MUNMAP: u64 = 27;
pub const SYS_DUP: u64 = 28;
pub const SYS_SYSLOG: u64 = 29;
pub const SYS_GETCWD: u64 = 30;
pub const SYS_CHDIR: u64 = 31;

// ── Networking (32-41) — raw NIC primitives (exokernel) ──────────────
pub const SYS_NIC_INFO: u64 = 32;
pub const SYS_NIC_TX: u64 = 33;
pub const SYS_NIC_RX: u64 = 34;
pub const SYS_NIC_LINK: u64 = 35;
pub const SYS_NIC_MAC: u64 = 36;
pub const SYS_NIC_REFILL: u64 = 37;
pub const SYS_NET_RSVD38: u64 = 38;
pub const SYS_NET_RSVD39: u64 = 39;
pub const SYS_NET_RSVD40: u64 = 40;
pub const SYS_NET_RSVD41: u64 = 41;

// ── Device / mount (42-45) — reserved stubs ──────────────────────────
pub const SYS_IOCTL: u64 = 42;
pub const SYS_MOUNT: u64 = 43;
pub const SYS_UMOUNT: u64 = 44;
pub const SYS_POLL: u64 = 45;

// ── Persistence / introspection (46-51) ──────────────────────────────
pub const SYS_PERSIST_PUT: u64 = 46;
pub const SYS_PERSIST_GET: u64 = 47;
pub const SYS_PERSIST_DEL: u64 = 48;
pub const SYS_PERSIST_LIST: u64 = 49;
pub const SYS_PERSIST_INFO: u64 = 50;
pub const SYS_PE_INFO: u64 = 51;

// ── Hardware primitives — exokernel essentials (52-62) ───────────────
pub const SYS_PORT_IN: u64 = 52;
pub const SYS_PORT_OUT: u64 = 53;
pub const SYS_PCI_CFG_READ: u64 = 54;
pub const SYS_PCI_CFG_WRITE: u64 = 55;
pub const SYS_DMA_ALLOC: u64 = 56;
pub const SYS_DMA_FREE: u64 = 57;
pub const SYS_MAP_PHYS: u64 = 58;
pub const SYS_VIRT_TO_PHYS: u64 = 59;
pub const SYS_IRQ_ATTACH: u64 = 60;
pub const SYS_IRQ_ACK: u64 = 61;
pub const SYS_CACHE_FLUSH: u64 = 62;

// ── Display (63-64) ─────────────────────────────────────────────────
pub const SYS_FB_INFO: u64 = 63;
pub const SYS_FB_MAP: u64 = 64;

// ── Process management (65-68) ──────────────────────────────────────
pub const SYS_PS: u64 = 65;
pub const SYS_SIGACTION: u64 = 66;
pub const SYS_SETPRIORITY: u64 = 67;
pub const SYS_GETPRIORITY: u64 = 68;

// ── CPU features / diagnostics (69-72) ──────────────────────────────
pub const SYS_CPUID: u64 = 69;
pub const SYS_RDTSC: u64 = 70;
pub const SYS_BOOT_LOG: u64 = 71;
pub const SYS_MEMMAP: u64 = 72;

#[inline(always)]
pub unsafe fn syscall0(nr: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall1(nr: u64, a1: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall2(nr: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall5(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}
