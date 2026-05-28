use super::prepare::{
    register_poweroff_handler, register_prepare_handler, register_restart_handler, TransitionKind,
};
use crate::hal;
use core::sync::atomic::{AtomicPtr, Ordering};
use morpheus_hal_api::BusAddr;

// PCI config-space offsets used directly here.
const PCI_VENDOR_ID: u16 = 0x00;
const PCI_COMMAND: u16 = 0x04;
const PCI_HEADER_TYPE: u16 = 0x0E;

// Optional external hooks. Set by the kernel's own syscall handler module
// (post-K8 wire-up) — declared here so the shutdown module compiles even
// before the syscall handler crate-internal symbols exist.

type DisplayReleaseFn = unsafe fn();
type FsSyncFn = unsafe fn() -> u64;

static DISPLAY_RELEASE_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static FS_SYNC_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Install the "release display ownership" hook. Called by the kernel
/// syscall-handler module once both live in the same crate.
pub fn set_display_release_hook(hook: DisplayReleaseFn) {
    DISPLAY_RELEASE_HOOK.store(hook as *mut (), Ordering::Release);
}

pub fn set_fs_sync_hook(hook: FsSyncFn) {
    FS_SYNC_HOOK.store(hook as *mut (), Ordering::Release);
}

fn prepare_disable_pci_bus_mastering(_kind: TransitionKind) -> bool {
    let mut touched = 0u32;

    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let addr = BusAddr::new(bus, device, 0);
            let vendor = hal().bus().cfg_read16(addr, PCI_VENDOR_ID);
            if vendor == 0xFFFF || vendor == 0x0000 {
                continue;
            }

            touched += clear_bus_master_on_function(addr) as u32;

            let header_type = hal().bus().cfg_read16(addr, PCI_HEADER_TYPE) as u8;
            if (header_type & 0x80) != 0 {
                for function in 1..8u8 {
                    let faddr = BusAddr::new(bus, device, function);
                    let fv = hal().bus().cfg_read16(faddr, PCI_VENDOR_ID);
                    if fv == 0xFFFF || fv == 0x0000 {
                        continue;
                    }
                    touched += clear_bus_master_on_function(faddr) as u32;
                }
            }
        }
    }

    if touched > 0 {
        crate::serial::checkpoint("shutdown-prepare-pci-bm-off");
    } else {
        crate::serial::checkpoint("shutdown-prepare-pci-bm-already-off");
    }
    true
}

#[inline(always)]
fn clear_bus_master_on_function(addr: BusAddr) -> bool {
    const CMD_BUS_MASTER: u16 = 1 << 2;
    const CMD_MEM_SPACE: u16 = 1 << 1;

    let cmd = hal().bus().cfg_read16(addr, PCI_COMMAND);
    let new_cmd = cmd & !(CMD_BUS_MASTER | CMD_MEM_SPACE);
    if cmd != new_cmd {
        hal().bus().cfg_write16(addr, PCI_COMMAND, new_cmd);
        true
    } else {
        false
    }
}

fn prepare_display_release(_kind: TransitionKind) -> bool {
    let p = DISPLAY_RELEASE_HOOK.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: the registered fn ptr was published by `set_display_release_hook`
        // with Release ordering; we read with Acquire and the type matches.
        unsafe {
            let f: DisplayReleaseFn = core::mem::transmute(p);
            f();
        }
    }
    crate::serial::checkpoint("shutdown-prepare-display");
    true
}

fn prepare_storage_sync(_kind: TransitionKind) -> bool {
    let p = FS_SYNC_HOOK.load(Ordering::Acquire);
    if p.is_null() {
        crate::serial::checkpoint("shutdown-prepare-sync-skipped");
        return true;
    }
    // SAFETY: see above; same publish/acquire discipline.
    let rc = unsafe {
        let f: FsSyncFn = core::mem::transmute(p);
        f()
    };
    if rc == 0 {
        crate::serial::checkpoint("shutdown-prepare-sync-ok");
        true
    } else {
        crate::serial::checkpoint("shutdown-prepare-sync-fail");
        false
    }
}

fn restart_marker(kind: TransitionKind) {
    match kind {
        TransitionKind::RebootGraceful | TransitionKind::RebootForce => {
            crate::serial::checkpoint("shutdown-restart-handlers-done");
        },
        TransitionKind::ShutdownGraceful | TransitionKind::ShutdownForce => {},
    }
}

fn poweroff_marker(kind: TransitionKind) {
    match kind {
        TransitionKind::ShutdownGraceful | TransitionKind::ShutdownForce => {
            crate::serial::checkpoint("shutdown-poweroff-handlers-done");
        },
        TransitionKind::RebootGraceful | TransitionKind::RebootForce => {},
    }
}

pub fn register_builtin_handlers() {
    register_prepare_handler(prepare_display_release);
    register_prepare_handler(prepare_disable_pci_bus_mastering);
    register_prepare_handler(prepare_storage_sync);
    register_restart_handler(restart_marker);
    register_poweroff_handler(poweroff_marker);
}
