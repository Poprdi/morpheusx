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
//! | 18     | SYS_TRUNCATE    | (path_ptr, path_len, new_size)    | 0             |
//! | 19     | SYS_SYNC        | ()                                | 0             |
//! | 20     | SYS_SNAPSHOT    | (name_ptr, name_len)              | snapshot_id   |
//! | 21     | SYS_VERSIONS    | (path_ptr, path_len, buf, max)    | count         |
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
//! | 32     | SYS_NIC_INFO    | (buf_ptr)                         | 0 / -ENODEV   |
//! | 33     | SYS_NIC_TX      | (frame_ptr, frame_len)            | 0 / -ENODEV   |
//! | 34     | SYS_NIC_RX      | (buf_ptr, buf_len)                | bytes / -ENODEV|
//! | 35     | SYS_NIC_LINK    | ()                                | 0/1 / -ENODEV |
//! | 36     | SYS_NIC_MAC     | (buf_ptr)                         | 0 / -ENODEV   |
//! | 37     | SYS_NIC_REFILL  | ()                                | 0 / -ENODEV   |
//! | 38     | SYS_NET         | (subcmd, a2, a3, a4)              | result        |
//! | 39     | SYS_DNS         | (subcmd, a2, a3)                  | result        |
//! | 40     | SYS_NET_CFG     | (subcmd, a2, a3, a4)              | result        |
//! | 41     | SYS_NET_POLL    | (subcmd, a2)                      | result        |
//! | 42     | SYS_IOCTL       | (fd, cmd, arg)                    | depends       |
//! | 43     | SYS_MOUNT       | (src_ptr,src_len,dst_ptr,dst_len) | 0             |
//! | 44     | SYS_UMOUNT      | (path_ptr, path_len)              | 0             |
//! | 45     | SYS_POLL        | (fds_ptr, nfds, timeout_ms)       | ready_count   |
//! | 46     | SYS_PERSIST_PUT | (key_ptr,key_len,data_ptr,data_l) | 0             |
//! | 47     | SYS_PERSIST_GET | (key_ptr,key_len,buf_ptr,buf_len) | bytes_read    |
//! | 48     | SYS_PERSIST_DEL | (key_ptr, key_len)                | 0             |
//! | 49     | SYS_PERSIST_LIST| (buf_ptr, buf_len, offset)        | count         |
//! | 50     | SYS_PERSIST_INFO| (info_ptr)                        | 0             |
//! | 51     | SYS_PE_INFO     | (path_ptr, path_len, info_ptr)    | 0             |
//! | 52     | SYS_PORT_IN     | (port, width[1/2/4])              | value         |
//! | 53     | SYS_PORT_OUT    | (port, width, value)              | 0             |
//! | 54     | SYS_PCI_CFG_READ| (bdf, offset, width)              | value         |
//! | 55     | SYS_PCI_CFG_WRITE|(bdf, offset, width, value)       | 0             |
//! | 56     | SYS_DMA_ALLOC   | (pages)                           | phys_addr     |
//! | 57     | SYS_DMA_FREE    | (phys, pages)                     | 0             |
//! | 58     | SYS_MAP_PHYS    | (phys, pages, flags)              | virt_addr     |
//! | 59     | SYS_VIRT_TO_PHYS| (virt)                            | phys          |
//! | 60     | SYS_IRQ_ATTACH  | (irq_num)                         | 0             |
//! | 61     | SYS_IRQ_ACK     | (irq_num)                         | 0             |
//! | 62     | SYS_CACHE_FLUSH | (addr, len)                       | 0             |
//! | 63     | SYS_FB_INFO     | (buf_ptr)                         | 0             |
//! | 64     | SYS_FB_MAP      | ()                                | virt_addr     |
//! | 65     | SYS_PS          | (buf_ptr, max_count)              | count         |
//! | 66     | SYS_SIGACTION   | (signum, handler_addr)            | old_handler   |
//! | 67     | SYS_SETPRIORITY | (pid, priority)                   | 0             |
//! | 68     | SYS_GETPRIORITY | (pid)                             | priority      |
//! | 69     | SYS_CPUID       | (leaf, subleaf, result_ptr)       | 0             |
//! | 70     | SYS_RDTSC       | (result_ptr)                      | tsc_value     |
//! | 71     | SYS_BOOT_LOG    | (buf_ptr, buf_len)                | bytes_written |
//! | 72     | SYS_MEMMAP      | (buf_ptr, max_entries)            | count         |
//! | 73     | SYS_SHM_GRANT   | (pid, vaddr, pages, flags)        | target_vaddr  |
//! | 74     | SYS_MPROTECT    | (vaddr, pages, prot)              | 0             |/// | 75     | SYS_PIPE        | (result_ptr)                      | 0             |
/// | 76     | SYS_DUP2        | (old_fd, new_fd)                  | new_fd        |
/// | 77     | SYS_SET_FG      | (pid)                             | 0             |
/// | 78     | SYS_GETARGS     | (buf_ptr, buf_len)                | argc          |
/// | 79     | SYS_FUTEX       | (addr, op, val, timeout_ms)       | woken / 0     |
/// | 80     | SYS_THREAD_CREATE|(entry, stack_top, arg)            | tid           |
/// | 81     | SYS_THREAD_EXIT | (code)                            | never         |
/// | 82     | SYS_THREAD_JOIN | (tid)                             | exit_code     |
pub mod handler;

use crate::serial::puts;
use handler::*;

// SYSCALL NUMBERS — core (0-9)

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

// helixfs file system syscalls (10-21)
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

// system / process / memory (22-31)
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

// networking (32-41) — raw nic primitives (exokernel)
pub const SYS_NIC_INFO: u64 = 32;
pub const SYS_NIC_TX: u64 = 33;
pub const SYS_NIC_RX: u64 = 34;
pub const SYS_NIC_LINK: u64 = 35;
pub const SYS_NIC_MAC: u64 = 36;
pub const SYS_NIC_REFILL: u64 = 37;
pub const SYS_NET: u64 = 38;
pub const SYS_DNS: u64 = 39;
pub const SYS_NET_CFG: u64 = 40;
pub const SYS_NET_POLL: u64 = 41;

// device / mount (42-45) — reserved stubs
pub const SYS_IOCTL: u64 = 42;
pub const SYS_MOUNT: u64 = 43;
pub const SYS_UMOUNT: u64 = 44;
pub const SYS_POLL: u64 = 45;

// persistence / introspection (46-51)
pub const SYS_PERSIST_PUT: u64 = 46;
pub const SYS_PERSIST_GET: u64 = 47;
pub const SYS_PERSIST_DEL: u64 = 48;
pub const SYS_PERSIST_LIST: u64 = 49;
pub const SYS_PERSIST_INFO: u64 = 50;
pub const SYS_PE_INFO: u64 = 51;

// hardware primitives — exokernel essentials (52-62)
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

// display (63-64)
pub const SYS_FB_INFO: u64 = 63;
pub const SYS_FB_MAP: u64 = 64;

// process management (65-68)
pub const SYS_PS: u64 = 65;
pub const SYS_SIGACTION: u64 = 66;
pub const SYS_SETPRIORITY: u64 = 67;
pub const SYS_GETPRIORITY: u64 = 68;

// cpu features / diagnostics (69-72)
pub const SYS_CPUID: u64 = 69;
pub const SYS_RDTSC: u64 = 70;
pub const SYS_BOOT_LOG: u64 = 71;
pub const SYS_MEMMAP: u64 = 72;

// memory sharing / protection (73-74)
pub const SYS_SHM_GRANT: u64 = 73;
pub const SYS_MPROTECT: u64 = 74;

// shell / ipc primitives (75-78)
pub const SYS_PIPE: u64 = 75;
pub const SYS_DUP2: u64 = 76;
pub const SYS_SET_FG: u64 = 77;
pub const SYS_GETARGS: u64 = 78;

// synchronization (79)
pub const SYS_FUTEX: u64 = 79;

// threading (80-82)
pub const SYS_THREAD_CREATE: u64 = 80;
pub const SYS_THREAD_EXIT: u64 = 81;
pub const SYS_THREAD_JOIN: u64 = 82;

// EXTERN ASM FUNCTIONS

extern "C" {
    /// Set up IA32_STAR / IA32_LSTAR / IA32_FMASK MSRs.
    pub fn syscall_init();
}

/// Standard ENOSYS return value (used for stubs and unknown syscalls).
const ENOSYS_RET: u64 = u64::MAX - 37;

// DISPATCH — called from syscall.s (MS x64 ABI)

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
        // helixfs syscalls
        SYS_OPEN => sys_fs_open(a1, a2, a3),
        SYS_CLOSE => sys_fs_close(a1),
        SYS_SEEK => sys_fs_seek(a1, a2, a3),
        SYS_STAT => sys_fs_stat(a1, a2, a3),
        SYS_READDIR => sys_fs_readdir(a1, a2, a3),
        SYS_MKDIR => sys_fs_mkdir(a1, a2),
        SYS_UNLINK => sys_fs_unlink(a1, a2),
        SYS_RENAME => sys_fs_rename(a1, a2, a3, a4),
        SYS_TRUNCATE => sys_fs_truncate(a1, a2, a3),
        SYS_SYNC => sys_fs_sync(),
        SYS_SNAPSHOT => sys_fs_snapshot(a1, a2),
        SYS_VERSIONS => sys_fs_versions(a1, a2, a3, a4),
        // system / process / memory
        SYS_CLOCK => sys_clock(),
        SYS_SYSINFO => sys_sysinfo(a1),
        SYS_GETPPID => sys_getppid(),
        SYS_SPAWN => sys_spawn(a1, a2, a3, a4),
        SYS_MMAP => sys_mmap(a1),
        SYS_MUNMAP => sys_munmap(a1, a2),
        SYS_DUP => sys_dup(a1),
        SYS_SYSLOG => sys_syslog(a1, a2),
        SYS_GETCWD => sys_getcwd(a1, a2),
        SYS_CHDIR => sys_chdir(a1, a2),
        // networking stubs
        SYS_NIC_INFO => sys_nic_info(a1),
        SYS_NIC_TX => sys_nic_tx(a1, a2),
        SYS_NIC_RX => sys_nic_rx(a1, a2),
        SYS_NIC_LINK => sys_nic_link(),
        SYS_NIC_MAC => sys_nic_mac(a1),
        SYS_NIC_REFILL => sys_nic_refill(),
        SYS_NET => sys_net(a1, a2, a3, a4),
        SYS_DNS => sys_dns(a1, a2, a3),
        SYS_NET_CFG => sys_net_cfg(a1, a2, a3, a4),
        SYS_NET_POLL => sys_net_poll(a1, a2),
        // device / mount
        SYS_IOCTL => sys_ioctl(a1, a2, a3),
        SYS_MOUNT => sys_mount(a1, a2, a3, a4),
        SYS_UMOUNT => sys_umount(a1, a2),
        SYS_POLL => sys_poll(a1, a2, a3), // persistence / introspection
        SYS_PERSIST_PUT => sys_persist_put(a1, a2, a3, a4),
        SYS_PERSIST_GET => sys_persist_get(a1, a2, a3, a4),
        SYS_PERSIST_DEL => sys_persist_del(a1, a2),
        SYS_PERSIST_LIST => sys_persist_list(a1, a2, a3),
        SYS_PERSIST_INFO => sys_persist_info(a1),
        SYS_PE_INFO => sys_pe_info(a1, a2, a3),
        // hardware primitives (exokernel)
        SYS_PORT_IN => sys_port_in(a1, a2),
        SYS_PORT_OUT => sys_port_out(a1, a2, a3),
        SYS_PCI_CFG_READ => sys_pci_cfg_read(a1, a2, a3),
        SYS_PCI_CFG_WRITE => sys_pci_cfg_write(a1, a2, a3, a4),
        SYS_DMA_ALLOC => sys_dma_alloc(a1),
        SYS_DMA_FREE => sys_dma_free(a1, a2),
        SYS_MAP_PHYS => sys_map_phys(a1, a2, a3),
        SYS_VIRT_TO_PHYS => sys_virt_to_phys(a1),
        SYS_IRQ_ATTACH => sys_irq_attach(a1),
        SYS_IRQ_ACK => sys_irq_ack(a1),
        SYS_CACHE_FLUSH => sys_cache_flush(a1, a2),
        // display
        SYS_FB_INFO => sys_fb_info(a1),
        SYS_FB_MAP => sys_fb_map(),
        // process management
        SYS_PS => sys_ps(a1, a2),
        SYS_SIGACTION => sys_sigaction(a1, a2),
        SYS_SETPRIORITY => sys_setpriority(a1, a2),
        SYS_GETPRIORITY => sys_getpriority(a1),
        // cpu features / diagnostics
        SYS_CPUID => sys_cpuid(a1, a2, a3),
        SYS_RDTSC => sys_rdtsc(a1),
        SYS_BOOT_LOG => sys_boot_log(a1, a2),
        SYS_MEMMAP => sys_memmap(a1, a2),
        // memory sharing / protection
        SYS_SHM_GRANT => sys_shm_grant(a1, a2, a3, a4),
        SYS_MPROTECT => sys_mprotect(a1, a2, a3),
        // shell / ipc primitives
        SYS_PIPE => sys_pipe(a1),
        SYS_DUP2 => sys_dup2(a1, a2),
        SYS_SET_FG => sys_set_fg(a1),
        SYS_GETARGS => sys_getargs(a1, a2),
        SYS_FUTEX => sys_futex(a1, a2, a3, a4),
        SYS_THREAD_CREATE => sys_thread_create(a1, a2, a3),
        SYS_THREAD_EXIT => sys_thread_exit(a1),
        SYS_THREAD_JOIN => sys_thread_join(a1),
        unknown => {
            puts("[SYSCALL] unknown nr=");
            crate::serial::put_hex32(unknown as u32);
            puts("\n");
            ENOSYS_RET
        }
    }
}

// SYS_ALLOC / SYS_FREE  (physical page allocation)

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

// INITIALIZATION

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
