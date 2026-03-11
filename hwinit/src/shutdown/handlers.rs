
use super::prepare::{
    register_poweroff_handler, register_prepare_handler, register_restart_handler, TransitionKind,
};

fn prepare_disable_pci_bus_mastering(_kind: TransitionKind) -> bool {
    let mut touched = 0u32;

    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let addr = crate::pci::PciAddr::new(bus, device, 0);
            let vendor = crate::pci::pci_cfg_read16(addr, crate::pci::offset::VENDOR_ID);
            if vendor == 0xFFFF || vendor == 0x0000 {
                continue;
            }

            touched += clear_bus_master_on_function(addr) as u32;

            let header_type = crate::pci::pci_cfg_read16(addr, crate::pci::offset::HEADER_TYPE) as u8;
            if (header_type & 0x80) != 0 {
                for function in 1..8u8 {
                    let faddr = crate::pci::PciAddr::new(bus, device, function);
                    let fv = crate::pci::pci_cfg_read16(faddr, crate::pci::offset::VENDOR_ID);
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
fn clear_bus_master_on_function(addr: crate::pci::PciAddr) -> bool {
    const CMD_BUS_MASTER: u16 = 1 << 2;
    const CMD_MEM_SPACE: u16 = 1 << 1;

    let cmd = crate::pci::pci_cfg_read16(addr, crate::pci::offset::COMMAND);
    let new_cmd = cmd & !(CMD_BUS_MASTER | CMD_MEM_SPACE);
    if cmd != new_cmd {
        crate::pci::pci_cfg_write16(addr, crate::pci::offset::COMMAND, new_cmd);
        true
    } else {
        false
    }
}

fn prepare_display_release(_kind: TransitionKind) -> bool {
    unsafe {
        crate::syscall::handler::shutdown_release_display_ownership();
    }
    crate::serial::checkpoint("shutdown-prepare-display");
    true
}

fn prepare_storage_sync(_kind: TransitionKind) -> bool {
    let rc = unsafe { crate::syscall::handler::sys_fs_sync() };
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
        }
        TransitionKind::ShutdownGraceful | TransitionKind::ShutdownForce => {}
    }
}

fn poweroff_marker(kind: TransitionKind) {
    match kind {
        TransitionKind::ShutdownGraceful | TransitionKind::ShutdownForce => {
            crate::serial::checkpoint("shutdown-poweroff-handlers-done");
        }
        TransitionKind::RebootGraceful | TransitionKind::RebootForce => {}
    }
}

pub fn register_builtin_handlers() {
    register_prepare_handler(prepare_display_release);
    register_prepare_handler(prepare_disable_pci_bus_mastering);
    register_prepare_handler(prepare_storage_sync);
    register_restart_handler(restart_marker);
    register_poweroff_handler(poweroff_marker);
}
