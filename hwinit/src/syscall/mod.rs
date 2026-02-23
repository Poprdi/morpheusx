//! Syscall interface — dispatch table and MSR setup.
//!
//! # Syscall numbers
//!
//! | Number | Name            | Args                              | Return        |
//! |--------|-----------------|-----------------------------------|---------------|
//! |  0     | SYS_EXIT        | (code: i32)                       | never         |
//! |  1     | SYS_WRITE       | (fd, ptr, len)                    | bytes_written |
//! |  2     | SYS_READ        | (fd, ptr, len)                    | bytes_read    |
//! |  3     | SYS_YIELD       | ()                                | 0             |
//! |  4     | SYS_ALLOC       | (pages: u64)                      | phys_base     |
//! |  5     | SYS_FREE        | (phys_base, pages)                | 0             |
//! |  6     | SYS_GETPID      | ()                                | pid           |
//! |  7     | SYS_KILL        | (pid, signal)                     | 0             |
//! |  8     | SYS_WAIT        | (pid)                             | exit_code     |
//! |  9     | SYS_SLEEP       | (millis)                          | 0             |
//! | 10     | SYS_OPEN        | (path_ptr, path_len, flags)       | fd            |
//! | 11     | SYS_CLOSE       | (fd)                              | 0             |
//! | 12     | SYS_SEEK        | (fd, offset, whence)              | new_offset    |
//! | 13     | SYS_STAT        | (path_ptr, path_len, stat_buf)    | 0             |
//! | 14     | SYS_READDIR     | (path_ptr, path_len, buf_ptr)     | count         |
//! | 15     | SYS_MKDIR       | (path_ptr, path_len)              | 0             |
//! | 16     | SYS_UNLINK      | (path_ptr, path_len)              | 0             |
//! | 17     | SYS_RENAME      | (old_ptr, old_len, new_ptr,new_l) | 0             |
//! | 18     | SYS_TRUNCATE    | (fd, new_size) [stub]             | -ENOSYS       |
//! | 19     | SYS_SYNC        | ()                                | 0             |
//! | 20     | SYS_SNAPSHOT    | (name_ptr, name_len) [stub]       | -ENOSYS       |
//! | 21     | SYS_VERSIONS    | (path_ptr, path_len, buf, max)[s] | -ENOSYS       |
//! | 22     | SYS_CLOCK       | ()                                | nanos         |
//! | 23     | SYS_SYSINFO     | (buf_ptr)                         | 0             |
//! | 24     | SYS_GETPPID     | ()                                | parent_pid    |
//! | 25     | SYS_SPAWN       | (path_ptr, path_len)              | child_pid     |
//! | 26     | SYS_MMAP        | (pages)                           | virt_addr     |
//! | 27     | SYS_MUNMAP      | (vaddr, pages)                    | 0             |
//! | 28     | SYS_DUP         | (old_fd)                          | new_fd        |
//! | 29     | SYS_SYSLOG      | (ptr, len)                        | len           |
//! | 30     | SYS_GETCWD      | (buf_ptr, buf_len)                | cwd_len       |
//! | 31     | SYS_CHDIR       | (path_ptr, path_len)              | 0             |
//! | 32-41  | SYS_SOCKET..    | (reserved for networking)         | -ENOSYS       |
//! | 42     | SYS_IOCTL       | (fd, cmd, arg) [stub]             | -ENOSYS       |
//! | 43     | SYS_MOUNT       | (src_ptr,src_len,dst_ptr,dst_len) | -ENOSYS       |
//! | 44     | SYS_UMOUNT      | (path_ptr, path_len) [stub]       | -ENOSYS       |
//! | 45     | SYS_POLL        | (fds_ptr, nfds, timeout) [stub]   | -ENOSYS       |
//! | 46     | SYS_PERSIST_PUT | (key_ptr,key_len,data_ptr,data_l) | 0             |
//! | 47     | SYS_PERSIST_GET | (key_ptr,key_len,buf_ptr,buf_len) | bytes_read    |
//! | 48     | SYS_PERSIST_DEL | (key_ptr, key_len)                | 0             |
//! | 49     | SYS_PERSIST_LIST| (buf_ptr, buf_len, offset)        | count         |
//! | 50     | SYS_PERSIST_INFO| (info_ptr)                        | 0             |
//! | 51     | SYS_PE_INFO     | (path_ptr, path_len, info_ptr)    | 0             |

pub mod handler;

use crate::serial::puts;
use handler::*;

// ═══════════════════════════════════════════════════════════════════════════
// SYSCALL NUMBERS — core (0-9)
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

// ── HelixFS file system syscalls (10-21) ─────────────────────────────
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

// ── Networking (32-41) — reserved, all return ENOSYS ─────────────────
pub const SYS_SOCKET: u64 = 32;
pub const SYS_CONNECT: u64 = 33;
pub const SYS_SEND: u64 = 34;
pub const SYS_RECV: u64 = 35;
pub const SYS_BIND: u64 = 36;
pub const SYS_LISTEN: u64 = 37;
pub const SYS_ACCEPT: u64 = 38;
pub const SYS_SHUTDOWN: u64 = 39;
pub const SYS_SETSOCKOPT: u64 = 40;
pub const SYS_DNS_RESOLVE: u64 = 41;

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

// ═══════════════════════════════════════════════════════════════════════════
// EXTERN ASM FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════

extern "C" {
    /// Set up IA32_STAR / IA32_LSTAR / IA32_FMASK MSRs.
    pub fn syscall_init();
}

/// Standard ENOSYS return value (used for stubs and unknown syscalls).
const ENOSYS_RET: u64 = u64::MAX - 37;

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
        // ── System / process / memory ─────────────────────────────
        SYS_CLOCK => sys_clock(),
        SYS_SYSINFO => sys_sysinfo(a1),
        SYS_GETPPID => sys_getppid(),
        SYS_SPAWN => sys_spawn(a1, a2),
        SYS_MMAP => sys_mmap(a1),
        SYS_MUNMAP => sys_munmap(a1, a2),
        SYS_DUP => sys_dup(a1),
        SYS_SYSLOG => sys_syslog(a1, a2),
        SYS_GETCWD => sys_getcwd(a1, a2),
        SYS_CHDIR => sys_chdir(a1, a2),
        // ── Networking stubs ──────────────────────────────────────
        SYS_SOCKET..=SYS_DNS_RESOLVE => ENOSYS_RET,
        // ── Device / mount stubs ──────────────────────────────────
        SYS_IOCTL..=SYS_POLL => ENOSYS_RET,        // ── Persistence / introspection ───────────────────────────────
        SYS_PERSIST_PUT => sys_persist_put(a1, a2, a3, a4),
        SYS_PERSIST_GET => sys_persist_get(a1, a2, a3, a4),
        SYS_PERSIST_DEL => sys_persist_del(a1, a2),
        SYS_PERSIST_LIST => sys_persist_list(a1, a2, a3),
        SYS_PERSIST_INFO => sys_persist_info(a1),
        SYS_PE_INFO => sys_pe_info(a1, a2, a3),        unknown => {
            puts("[SYSCALL] unknown nr=");
            crate::serial::put_hex32(unknown as u32);
            puts("\n");
            ENOSYS_RET
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

unsafe fn sys_free(phys_base: u64, pages: u64) -> u64 {
    if phys_base == 0 || pages == 0 || pages > 1024 {
        return u64::MAX; // -EINVAL
    }
    if !crate::memory::is_registry_initialized() {
        return u64::MAX; // -ENOMEM
    }
    let registry = crate::memory::global_registry_mut();
    match registry.free_pages(phys_base, pages) {
        Ok(()) => 0,
        Err(_) => u64::MAX, // -EINVAL
    }
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
