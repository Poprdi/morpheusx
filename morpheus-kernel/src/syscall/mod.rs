//! Syscall interface — dispatch table and MSR setup.
//!
//! Canonical SYS_* numbers live in `morpheus-foundation::syscall_abi`; re-exported
//! here for legacy `crate::syscall::SYS_*` callers.

pub mod handler;

use crate::hal;
use crate::process::ProcessState;
use crate::schedular::SCHEDULER;
use handler::clock::{sys_clock_gettime, sys_nanosleep};
use handler::compositor::{
    sys_compositor_set, sys_forward_input, sys_mouse_forward, sys_try_wait,
    sys_win_surface_dirty_clear, sys_win_surface_list, sys_win_surface_map,
};
use handler::core::{
    sys_exit, sys_getpid, sys_keyboard_read, sys_kill, sys_read, sys_sleep, sys_system_control,
    sys_wait, sys_write, sys_yield,
};
use handler::epoll::{sys_epoll_create, sys_epoll_ctl, sys_epoll_wait};
use handler::fb::{
    fb_lock_holder, is_composited_client, sys_fb_blit, sys_fb_info, sys_fb_lock, sys_fb_map,
    sys_fb_present, sys_fb_unlock,
};
use handler::fd::{sys_chdir, sys_dup, sys_fcntl, sys_getcwd, sys_syslog};
use handler::fs::{
    sys_fs_close, sys_fs_fstat, sys_fs_fsync, sys_fs_ftruncate, sys_fs_mkdir, sys_fs_open,
    sys_fs_readdir, sys_fs_rename, sys_fs_rmdir, sys_fs_seek, sys_fs_snapshot, sys_fs_stat,
    sys_fs_sync, sys_fs_truncate, sys_fs_unlink, sys_fs_versions, sys_mount, sys_mounts,
    sys_umount, sys_volumes,
};
use handler::hw::{
    sys_cache_flush, sys_dma_alloc, sys_dma_free, sys_getrandom, sys_irq_ack, sys_irq_attach,
    sys_map_phys, sys_pci_cfg_read, sys_pci_cfg_write, sys_port_in, sys_port_out, sys_virt_to_phys,
};
use handler::ipc::{sys_dup2, sys_getargs, sys_getenv, sys_pipe, sys_set_fg, sys_shm_grant};
use handler::mem::{sys_mmap, sys_mprotect, sys_munmap};
use handler::net::{sys_dns, sys_net, sys_net_cfg, sys_net_poll};
use handler::nic_fb::fb_mark_dirty;
use handler::nic_io::{
    sys_ioctl, sys_nic_info, sys_nic_link, sys_nic_mac, sys_nic_refill, sys_nic_rx, sys_nic_tx,
    sys_poll,
};
use handler::persist::{
    sys_pe_info, sys_persist_del, sys_persist_get, sys_persist_info, sys_persist_list,
    sys_persist_put,
};
use handler::proc::{
    sys_clock, sys_getppid, sys_reparent, sys_set_thread_pointer, sys_spawn, sys_sysinfo,
};
use handler::socket::{
    sys_accept, sys_bind, sys_connect, sys_getpeername, sys_getsockname, sys_getsockopt,
    sys_listen, sys_recvfrom, sys_sendto, sys_setsockopt, sys_shutdown, sys_socket,
};
use handler::sync::{
    sys_futex, sys_gettid, sys_mouse_read, sys_sigreturn, sys_thread_create, sys_thread_detach,
    sys_thread_exit, sys_thread_join,
};
use handler::sysinfo::{
    sys_boot_log, sys_cpuid, sys_getpriority, sys_memmap, sys_ps, sys_rdtsc, sys_setpriority,
    sys_sigaction,
};

pub use morpheus_foundation::syscall_abi::*;

use morpheus_foundation::errno::{EINVAL, ENOMEM, ENOSYS};

/// Dispatched from `syscall_entry` (MS x64 ABI). All args are user-controlled.
///
/// # Safety
/// Called from asm; arguments unvalidated.
#[no_mangle]
pub unsafe extern "C" fn syscall_dispatch(
    nr: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
) -> u64 {
    match nr {
        SYS_EXIT => sys_exit(a1),
        SYS_WRITE => sys_write(a1, a2, a3),
        SYS_READ => sys_read(a1, a2, a3),
        SYS_KEYBOARD_READ => sys_keyboard_read(a1, a2),
        SYS_YIELD => sys_yield(),
        SYS_ALLOC => sys_alloc(a1),
        SYS_FREE => sys_free(a1, a2),
        SYS_GETPID => sys_getpid(),
        SYS_KILL => sys_kill(a1, a2),
        SYS_WAIT => sys_wait(a1, a2, a3, a4),
        SYS_SLEEP => sys_sleep(a1),
        SYS_OPEN => sys_fs_open(a1, a2, a3),
        SYS_CLOSE => sys_fs_close(a1),
        SYS_SEEK => sys_fs_seek(a1, a2, a3),
        SYS_STAT => sys_fs_stat(a1, a2, a3),
        SYS_READDIR => sys_fs_readdir(a1, a2, a3, a4),
        SYS_MKDIR => sys_fs_mkdir(a1, a2),
        SYS_UNLINK => sys_fs_unlink(a1, a2),
        SYS_RENAME => sys_fs_rename(a1, a2, a3, a4),
        SYS_TRUNCATE => sys_fs_truncate(a1, a2, a3),
        SYS_SYNC => sys_fs_sync(),
        SYS_SNAPSHOT => sys_fs_snapshot(a1, a2),
        SYS_VERSIONS => sys_fs_versions(a1, a2, a3, a4),
        SYS_CLOCK => sys_clock(),
        SYS_SYSINFO => sys_sysinfo(a1),
        SYS_GETPPID => sys_getppid(),
        SYS_SPAWN => sys_spawn(a1),
        SYS_MMAP => sys_mmap(a1, a2, a3, a4),
        SYS_MUNMAP => sys_munmap(a1, a2),
        SYS_DUP => sys_dup(a1),
        SYS_SYSLOG => sys_syslog(a1, a2),
        SYS_GETCWD => sys_getcwd(a1, a2),
        SYS_CHDIR => sys_chdir(a1, a2),
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
        SYS_IOCTL => sys_ioctl(a1, a2, a3),
        SYS_MOUNT => sys_mount(a1, a2, a3, a4, a5, a6),
        SYS_UMOUNT => sys_umount(a1, a2, a3),
        SYS_POLL => sys_poll(a1, a2, a3),
        SYS_PERSIST_PUT => sys_persist_put(a1, a2, a3, a4),
        SYS_PERSIST_GET => sys_persist_get(a1, a2, a3, a4),
        SYS_PERSIST_DEL => sys_persist_del(a1, a2),
        SYS_PERSIST_LIST => sys_persist_list(a1, a2, a3),
        SYS_PERSIST_INFO => sys_persist_info(a1),
        SYS_PE_INFO => sys_pe_info(a1, a2, a3),
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
        SYS_FB_INFO => sys_fb_info(a1),
        SYS_FB_MAP => sys_fb_map(),
        SYS_PS => sys_ps(a1, a2),
        SYS_SIGACTION => sys_sigaction(a1, a2),
        SYS_SETPRIORITY => sys_setpriority(a1, a2),
        SYS_GETPRIORITY => sys_getpriority(a1),
        SYS_CPUID => sys_cpuid(a1, a2, a3),
        SYS_RDTSC => sys_rdtsc(a1),
        SYS_BOOT_LOG => sys_boot_log(a1, a2),
        SYS_MEMMAP => sys_memmap(a1, a2),
        SYS_SHM_GRANT => sys_shm_grant(a1, a2, a3, a4),
        SYS_MPROTECT => sys_mprotect(a1, a2, a3),
        SYS_PIPE => sys_pipe(a1, a2),
        SYS_DUP2 => sys_dup2(a1, a2),
        SYS_SET_FG => sys_set_fg(a1),
        SYS_GETARGS => sys_getargs(a1, a2),
        SYS_FUTEX => sys_futex(a1, a2, a3, a4),
        SYS_THREAD_CREATE => sys_thread_create(a1, a2, a3, a4, a5, a6),
        SYS_THREAD_EXIT => sys_thread_exit(a1),
        SYS_THREAD_JOIN => sys_thread_join(a1),
        SYS_SIGRETURN => sys_sigreturn(),
        SYS_MOUSE_READ => sys_mouse_read(),
        SYS_FB_LOCK => sys_fb_lock(),
        SYS_FB_UNLOCK => sys_fb_unlock(),
        SYS_FB_IS_LOCKED => fb_lock_holder() as u64,
        SYS_FB_PRESENT => sys_fb_present(),
        SYS_FB_BLIT => sys_fb_blit(),
        SYS_FB_MARK_DIRTY => {
            if is_composited_client() {
                let proc = SCHEDULER.current_process_mut();
                proc.fb_surface_dirty = true;
            } else {
                fb_mark_dirty();
            }
            0
        },
        SYS_COMPOSITOR_SET => sys_compositor_set(),
        SYS_WIN_SURFACE_LIST => sys_win_surface_list(a1, a2),
        SYS_WIN_SURFACE_MAP => sys_win_surface_map(a1),
        SYS_MOUSE_FORWARD => sys_mouse_forward(a1, a2),
        SYS_WIN_SURFACE_DIRTY_CLEAR => sys_win_surface_dirty_clear(a1),
        SYS_TRY_WAIT => sys_try_wait(a1),
        SYS_FORWARD_INPUT => sys_forward_input(a1, a2, a3),
        SYS_SYSTEM_CONTROL => sys_system_control(a1),
        SYS_SET_THREAD_POINTER => sys_set_thread_pointer(a1),
        SYS_GETRANDOM => sys_getrandom(a1, a2, a3),
        SYS_VOLUMES => sys_volumes(a1, a2),
        SYS_MOUNTS => sys_mounts(a1, a2),
        SYS_REPARENT => sys_reparent(a1, a2),
        SYS_CLOCK_GETTIME => sys_clock_gettime(a1, a2),
        SYS_NANOSLEEP => sys_nanosleep(a1, a2),
        SYS_FSTAT => sys_fs_fstat(a1, a2),
        SYS_THREAD_DETACH => sys_thread_detach(a1),
        SYS_GETTID => sys_gettid(),
        SYS_SOCKET => sys_socket(a1, a2, a3),
        SYS_BIND => sys_bind(a1, a2, a3),
        SYS_LISTEN => sys_listen(a1, a2),
        SYS_ACCEPT => sys_accept(a1, a2, a3, a4),
        SYS_CONNECT => sys_connect(a1, a2, a3),
        SYS_SENDTO => sys_sendto(a1, a2, a3, a4, a5, a6),
        SYS_RECVFROM => sys_recvfrom(a1, a2, a3, a4, a5, a6),
        SYS_GETSOCKNAME => sys_getsockname(a1, a2, a3),
        SYS_GETPEERNAME => sys_getpeername(a1, a2, a3),
        SYS_SETSOCKOPT => sys_setsockopt(a1, a2, a3, a4, a5),
        SYS_GETSOCKOPT => sys_getsockopt(a1, a2, a3, a4, a5),
        SYS_SHUTDOWN => sys_shutdown(a1, a2),
        SYS_EPOLL_CREATE => sys_epoll_create(a1),
        SYS_EPOLL_CTL => sys_epoll_ctl(a1, a2, a3, a4),
        SYS_EPOLL_WAIT => sys_epoll_wait(a1, a2, a3, a4),
        SYS_GETENV => sys_getenv(a1, a2),
        SYS_FSYNC => sys_fs_fsync(a1, a2),
        SYS_FTRUNCATE => sys_fs_ftruncate(a1, a2),
        SYS_FCNTL => sys_fcntl(a1, a2, a3),
        SYS_RMDIR => sys_fs_rmdir(a1, a2),
        unknown => {
            crate::serial::log_warn("SYSCALL", 801, "unknown syscall number");
            let _ = unknown;
            let _ = ProcessState::Ready;
            ENOSYS
        },
    }
}

unsafe fn sys_alloc(pages: u64) -> u64 {
    if pages == 0 || pages > 1024 {
        return EINVAL;
    }
    let phys = hal().phys();
    if !phys.is_initialized() {
        return ENOMEM;
    }
    phys.allocate_pages(
        morpheus_hal_api::AllocKind::AnyPages,
        morpheus_hal_api::MemoryType::Allocated,
        pages,
    )
    .unwrap_or(ENOMEM)
}

unsafe fn sys_free(phys_base: u64, pages: u64) -> u64 {
    if phys_base == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    let phys = hal().phys();
    if !phys.is_initialized() {
        return EINVAL;
    }
    match phys.free_pages(phys_base, pages) {
        Ok(()) => 0,
        Err(_) => EINVAL,
    }
}

/// # Safety
/// Long mode, interrupts disabled, after IDT/PIC.
pub unsafe fn init_syscall() {
    hal().cpu().install_syscall_msrs();

    // Wire the kernel-internal sched_hooks now that handlers are in-tree.
    // K7 ELF + the HAL paging gap still leave the user-page hooks owned by
    // hwinit, but the FB compositor hooks are pure kernel.
    crate::sched_hooks::install_fb_present_tick(fb_present_tick_trampoline);
    crate::sched_hooks::install_compositor_active(compositor_active_trampoline);
    crate::sched_hooks::install_release_fb_lock_if_holder(release_fb_lock_trampoline);
}

// `extern "C"` trampolines: `sched_hooks` declares the hook types with that
// ABI so the indirection stays callable from any TU.
unsafe extern "C" fn fb_present_tick_trampoline() {
    handler::fb::fb_present_tick();
}

unsafe extern "C" fn compositor_active_trampoline() -> bool {
    handler::fb::compositor_active()
}

unsafe extern "C" fn release_fb_lock_trampoline(pid: u32) {
    handler::fb::release_fb_lock_if_holder(pid);
}
