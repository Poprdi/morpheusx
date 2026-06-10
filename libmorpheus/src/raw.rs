//! Raw syscall wrappers — thin inline asm around the `syscall` instruction.

// Canonical SYS_* numbers live in morpheus-foundation::syscall_abi. Re-exported
// here for source compatibility with existing `libmorpheus::raw::SYS_*` callers
// and to guarantee the kernel-side dispatcher (hwinit) and this userland side
// cannot drift.
pub use morpheus_foundation::syscall_abi::*;

// futex ops + sysctl modes are canonical in morpheus-foundation — single source.
pub use morpheus_foundation::flags::{
    FUTEX_WAIT, FUTEX_WAKE, SYSCTL_REBOOT_FORCE, SYSCTL_REBOOT_GRACEFUL, SYSCTL_SHUTDOWN_FORCE,
    SYSCTL_SHUTDOWN_GRACEFUL, SYSCTL_SHUTDOWN_PANIC,
};

/// # Safety
/// `nr` must be a valid syscall number for the kernel ABI. The caller is
/// responsible for upholding any invariants the selected syscall requires.
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

/// # Safety
/// `nr` must be a valid syscall number for the kernel ABI and `a1` must be a
/// valid argument for it: any pointer/length passed must reference memory that
/// is valid for the duration and access pattern the syscall performs.
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

/// # Safety
/// `nr` must be a valid syscall number for the kernel ABI and `a1`/`a2` must be
/// valid arguments for it: any pointer/length passed must reference memory that
/// is valid for the duration and access pattern the syscall performs.
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

/// # Safety
/// `nr` must be a valid syscall number for the kernel ABI and `a1`..`a3` must be
/// valid arguments for it: any pointer/length passed must reference memory that
/// is valid for the duration and access pattern the syscall performs.
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

/// # Safety
/// `nr` must be a valid syscall number for the kernel ABI and `a1`..`a4` must be
/// valid arguments for it: any pointer/length passed must reference memory that
/// is valid for the duration and access pattern the syscall performs.
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

/// # Safety
/// `nr` must be a valid syscall number for the kernel ABI and `a1`..`a5` must be
/// valid arguments for it: any pointer/length passed must reference memory that
/// is valid for the duration and access pattern the syscall performs.
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
