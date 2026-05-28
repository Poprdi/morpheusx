//! Terminal states — success and failure endpoints.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};
use morpheus_block::block_traits::BlockDriver;
use morpheus_nic::traits::NetworkDriver;

/// Success terminal state.
pub(crate) struct DoneState {
    logged: bool,
    flushed: bool,
    rebooting: bool,
}

impl DoneState {
    pub fn new() -> Self {
        Self {
            logged: false,
            flushed: false,
            rebooting: false,
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn reboot() {
        serial::println("");
        serial::println("=====================================");
        serial::println("  ISO Download Complete!");
        serial::println("=====================================");
        serial::println("");
        serial::println("Initiating safe system reboot...");

        unsafe {
            // Keyboard controller reset: most broadly compatible.
            serial::println("[REBOOT] Using keyboard controller reset (0x64 -> 0xFE)");
            core::arch::asm!(
                "out dx, al",
                in("dx") 0x64u16,
                in("al") 0xFEu8,
                options(nomem, nostack)
            );

            for _ in 0..50_000_000 {
                core::hint::spin_loop();
            }

            // Fallback: PCI reset control port 0xCF9 (modern systems).
            serial::println("[REBOOT] Fallback: Port 0xCF9 reset");
            core::arch::asm!(
                "out dx, al",
                in("dx") 0xCF9u16,
                in("al") 0x06u8,
                options(nomem, nostack)
            );

            for _ in 0..50_000_000 {
                core::hint::spin_loop();
            }

            serial::println("[REBOOT] Reboot methods failed - halting system");
            serial::println("[REBOOT] Please manually power cycle the system");
            loop {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn reboot() {
        serial::println("[REBOOT] Reboot not supported on this architecture");
        loop {
            core::hint::spin_loop();
        }
    }
}

impl Default for DoneState {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: NetworkDriver> State<D> for DoneState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if !self.flushed {
            self.flushed = true;
            if let Some(ref mut blk) = ctx.blk_device {
                serial::println("[DISK] Syncing disk cache...");
                match blk.flush() {
                    Ok(()) => serial::println("[OK] Disk cache synced"),
                    Err(e) => {
                        serial::print("[WARN] Disk sync: ");
                        serial::println(match e {
                            morpheus_block::block_traits::BlockError::Unsupported => {
                                "not supported (assuming durable)"
                            },
                            morpheus_block::block_traits::BlockError::Timeout => "timeout",
                            morpheus_block::block_traits::BlockError::DeviceError => "device error",
                            _ => "unknown",
                        });
                    },
                }
            }
        }

        if !self.logged {
            serial::println("=================================");
            serial::println("        DOWNLOAD COMPLETE        ");
            serial::println("=================================");
            serial::print("Total bytes: ");
            serial::print_u32((ctx.bytes_downloaded / 1024) as u32);
            serial::println(" KB");
            if ctx.bytes_written > 0 {
                serial::print("Written to disk: ");
                serial::print_u32((ctx.bytes_written / 1024) as u32);
                serial::println(" KB");
            }
            self.logged = true;
        }

        // Never returns.
        if !self.rebooting {
            self.rebooting = true;
            Self::reboot();
        }

        // Unreachable; satisfies the return type.
        (self, StepResult::Done)
    }

    fn name(&self) -> &'static str {
        "Done"
    }
}

/// Failure terminal state.
pub(crate) struct FailedState {
    reason: &'static str,
    logged: bool,
}

impl FailedState {
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason,
            logged: false,
        }
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl<D: NetworkDriver> State<D> for FailedState {
    fn step(
        mut self: Box<Self>,
        _ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if !self.logged {
            serial::println("=================================");
            serial::println("        DOWNLOAD FAILED          ");
            serial::println("=================================");
            serial::print("Reason: ");
            serial::println(self.reason);
            self.logged = true;
        }

        let reason = self.reason;
        (self, StepResult::Failed(reason))
    }

    fn name(&self) -> &'static str {
        "Failed"
    }
}
