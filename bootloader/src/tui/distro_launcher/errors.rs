// Distro launcher - select and boot a kernel

use super::ui::{DistroLauncher, KernelEntry};
use crate::boot::loader::BootError;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED};
use alloc::string::String;
use alloc::vec::Vec;

const MAX_KERNEL_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

impl DistroLauncher {
    pub(super) fn await_failure(
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        start_line: usize,
        message: &str,
        log_tag: &'static str,
    ) {
        morpheus_core::logger::log(log_tag);
        screen.put_str_at(5, start_line, message, EFI_RED, EFI_BLACK);
        screen.put_str_at(
            5,
            start_line + 2,
            "Press any key to return...",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
        keyboard.wait_for_key();
    }
    pub(super) fn dump_logs_to_screen(screen: &mut Screen) {
        let logs = morpheus_core::logger::get_logs();
        let start_y = 20;

        screen.put_str_at(5, start_y, "=== DEBUG LOGS ===", EFI_LIGHTGREEN, EFI_BLACK);

        for (i, log_entry) in logs.iter().enumerate() {
            let y = start_y + 1 + i;
            if y >= screen.height() - 1 {
                break;
            }

            if let Some(msg) = log_entry {
                screen.put_str_at(7, y, msg, EFI_GREEN, EFI_BLACK);
            }
        }
    }
    pub(super) fn describe_boot_error(error: &BootError) -> alloc::string::String {
        match error {
            BootError::KernelParse(e) => alloc::format!("Kernel parse failed: {:?}", e),
            BootError::KernelAllocation(e) => alloc::format!("Kernel allocation failed: {:?}", e),
            BootError::KernelLoad(e) => alloc::format!("Kernel load failed: {:?}", e),
            BootError::BootParamsAllocation(e) => {
                alloc::format!("Boot params allocation failed: {:?}", e)
            }
            BootError::CmdlineAllocation(e) => alloc::format!("Cmdline allocation failed: {:?}", e),
            BootError::InitrdAllocation(e) => alloc::format!("Initrd allocation failed: {:?}", e),
            BootError::MemorySnapshot(e) => alloc::format!("Memory map build failed: {:?}", e),
            BootError::ExitBootServices(e) => alloc::format!("ExitBootServices failed: {:?}", e),
        }
    }
}
