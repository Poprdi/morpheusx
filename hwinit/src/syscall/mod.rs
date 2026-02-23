//! Syscall interface — dispatch table and MSR setup.
//!
//! # Syscall numbers
//!
//! | Number | Name        | Args                              | Return      |
//! |--------|-------------|-----------------------------------|-------------|
//! |  0     | SYS_EXIT    | (code: i32)                       | never       |
//! |  1     | SYS_WRITE   | (ptr: *u8, len: usize)            | bytes_written|
//! |  2     | SYS_READ    | (fd, ptr, len)                    | bytes_read  |
//! |  3     | SYS_YIELD   | ()                                | 0           |
//! |  4     | SYS_ALLOC   | (pages: u64)                      | phys_base   |
//! |  5     | SYS_FREE    | (phys_base, pages)                | 0           |
//! |  6     | SYS_GETPID  | ()                                | pid         |
//! |  7     | SYS_KILL    | (pid, signal)                     | 0           |
//! |  8     | SYS_WAIT    | (pid)                             | exit_code   |
//! |  9     | SYS_SLEEP   | (ticks)                           | 0           |

pub mod handler;

use crate::serial::puts;
use handler::*;

// ═══════════════════════════════════════════════════════════════════════════
// SYSCALL NUMBERS
// ═══════════════════════════════════════════════════════════════════════════

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

// ── HelixFS file system syscalls ─────────────────────────────────────
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

// ═══════════════════════════════════════════════════════════════════════════
// EXTERN ASM FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════

extern "C" {
    /// Set up IA32_STAR / IA32_LSTAR / IA32_FMASK MSRs.
    pub fn syscall_init();
}

// ═══════════════════════════════════════════════════════════════════════════
// DISPATCH — called from syscall.s (MS x64 ABI)
// ═══════════════════════════════════════════════════════════════════════════

/// Main syscall dispatcher.  Called by the `syscall_entry` ASM stub with the
/// syscall number in `nr` and up to 5 arguments in `a1`..`a5`.
///
/// # Safety
/// Called from ASM with MS x64 ABI.  Arguments come directly from user/kernel
/// registers and must be validated before use.
#[no_mangle]
pub unsafe extern "C" fn syscall_dispatch(
    nr: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    _a5: u64,
) -> u64 {
    match nr {
        SYS_EXIT => sys_exit(a1),
        SYS_WRITE => sys_write(a1, a2, a3),
        SYS_READ => sys_read(a1, a2, a3),
        SYS_YIELD => sys_yield(),
        SYS_ALLOC => sys_alloc(a1),
        SYS_FREE => sys_free(a1, a2),
        SYS_GETPID => sys_getpid(),
        SYS_KILL => sys_kill(a1, a2),
        SYS_WAIT => sys_wait(a1),
        SYS_SLEEP => sys_sleep(a1),
        // ── HelixFS syscalls ──────────────────────────────────────
        SYS_OPEN => sys_fs_open(a1, a2, a3),
        SYS_CLOSE => sys_fs_close(a1),
        SYS_SEEK => sys_fs_seek(a1, a2, a3),
        SYS_STAT => sys_fs_stat(a1, a2, a3),
        SYS_READDIR => sys_fs_readdir(a1, a2, a3),
        SYS_MKDIR => sys_fs_mkdir(a1, a2),
        SYS_UNLINK => sys_fs_unlink(a1, a2),
        SYS_RENAME => sys_fs_rename(a1, a2, a3, a4),
        SYS_TRUNCATE => sys_fs_truncate(a1, a2),
        SYS_SYNC => sys_fs_sync(),
        SYS_SNAPSHOT => sys_fs_snapshot(a1, a2),
        SYS_VERSIONS => sys_fs_versions(a1, a2, a3, a4),
        unknown => {
            puts("[SYSCALL] unknown nr=");
            crate::serial::put_hex32(unknown as u32);
            puts("\n");
            u64::MAX - 37 // -ENOSYS
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_ALLOC / SYS_FREE  (physical page allocation)
// ═══════════════════════════════════════════════════════════════════════════

unsafe fn sys_alloc(pages: u64) -> u64 {
    if pages == 0 || pages > 1024 {
        return u64::MAX; // -EINVAL
    }
    if !crate::memory::is_registry_initialized() {
        return u64::MAX; // -ENOMEM
    }
    let registry = crate::memory::global_registry_mut();
    registry
        .allocate_pages(
            crate::memory::AllocateType::AnyPages,
            crate::memory::MemoryType::Allocated,
            pages,
        )
        .unwrap_or(u64::MAX)
}

unsafe fn sys_free(_phys_base: u64, _pages: u64) -> u64 {
    // TODO: call registry.free_pages() once that API is wired up.
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize the SYSCALL/SYSRET mechanism and install the timer ISR.
///
/// Call once, after IDT and PIC are configured.
///
/// # Safety
/// Must be called in long mode with interrupts disabled.
pub unsafe fn init_syscall() {
    syscall_init();
    puts("[SYSCALL] SYSCALL/SYSRET enabled (IA32_LSTAR configured)\n");
}
