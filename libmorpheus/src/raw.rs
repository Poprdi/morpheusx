//! Raw syscall wrappers — thin inline asm around the `syscall` instruction.

// Canonical SYS_* numbers live in morpheus-foundation::syscall_abi. Re-exported
// here for source compatibility with existing `libmorpheus::raw::SYS_*` callers
// and to guarantee the kernel-side dispatcher (hwinit) and this userland side
// cannot drift.
pub use morpheus_foundation::syscall_abi::*;

// libmorpheus-private constants kept here for now: futex ops + sysctl modes
// are userland-internal flag spaces that don't cross the syscall number ABI.
// (They will move to foundation later if the kernel ever needs to discriminate
// them by name, but for today they're values the kernel reads as opaque u64.)
pub const FUTEX_WAIT: u64 = 0;
pub const FUTEX_WAKE: u64 = 1;

// SYS_SYSTEM_CONTROL modes
pub const SYSCTL_REBOOT_GRACEFUL: u64 = 0;
pub const SYSCTL_REBOOT_FORCE: u64 = 1;
pub const SYSCTL_SHUTDOWN_GRACEFUL: u64 = 2;
pub const SYSCTL_SHUTDOWN_FORCE: u64 = 3;
pub const SYSCTL_SHUTDOWN_PANIC: u64 = 4;

#[inline(always)]
pub unsafe fn syscall0(nr: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        out("rcx") _,
        out("r11") _,
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
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
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
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
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
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
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
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
        out("r8") _,
        out("r9") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
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
        out("r9") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}
