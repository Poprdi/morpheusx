//! xHCI MSI-X interrupt wiring (Phase 2 — stub ISR only).
//!
//! This module installs a minimal interrupt handler for the xHCI controller
//! on a dedicated IDT vector. The handler does not yet drain the event ring —
//! it bumps a counter, acks IMAN.IP, and EOIs the LAPIC. The existing
//! polling-based `wait_cmd` / `wait_xfer` paths remain authoritative.
//!
//! Proves MSI-X delivery works before Phase 3 swaps the polling waiters for
//! interrupt-driven HLT-based waits.
//!
//! See `.claude/skills/interrupt-driven-refactor/SKILL.md` (Steps 1, 2, 4).

use crate::cpu::apic;
use crate::cpu::idt::set_interrupt_handler;
use crate::cpu::mmio;
use crate::pci::config::PciAddr;
use crate::pci::msi;
use crate::serial::{log_info, log_ok, log_warn};
use crate::usb::regs::RT_IR0_IMAN;
use core::sync::atomic::{AtomicU64, Ordering};

/// IDT vector reserved for xHCI MSI-X. Picked above the standard PIC remap
/// range (0x20–0x2F) and clear of the existing timer (0x20).
pub const XHCI_VECTOR: u8 = 0x40;

/// Counts every xHCI MSI-X interrupt the ISR observes. Diagnostic-only for
/// Phase 2; reachable from a debug syscall or serial dump.
pub static XHCI_EVENT_COUNT: AtomicU64 = AtomicU64::new(0);

/// Runtime register base of the xHCI controller. Published by `wire_msix` so
/// the ISR can W1C the interrupter's IP bit without a wider borrow of the
/// controller struct (which is owned by Phase 9 init and dropped afterwards).
static XHCI_RT_BASE: AtomicU64 = AtomicU64::new(0);

/// Rust-side handler. Runs in interrupt context: no allocation, no sleeping
/// locks. Acknowledges at the device, then EOIs the LAPIC.
extern "C" fn xhci_isr_rust() {
    XHCI_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);

    // Ack the interrupter: W1C the IP bit (bit 0) of IMAN. Preserve IE (bit 1).
    let rt_base = XHCI_RT_BASE.load(Ordering::Relaxed);
    if rt_base != 0 {
        unsafe {
            let iman = mmio::read32(rt_base + RT_IR0_IMAN);
            mmio::write32(rt_base + RT_IR0_IMAN, iman | 0x01);
        }
    }

    unsafe {
        apic::send_eoi();
    }
}

/// Assembly thunk that saves caller-saved GPRs, calls the Rust handler under
/// the MS x64 ABI (with shadow space), restores, and `iretq`s. No vector or
/// frame is passed — this ISR doesn't need them.
#[unsafe(naked)]
unsafe extern "C" fn xhci_isr_entry() {
    core::arch::naked_asm!(
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "sub rsp, 32", // MS x64 shadow space
        "call {}",
        "add rsp, 32",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "iretq",
        sym xhci_isr_rust,
    );
}

/// Attempt to wire the xHCI controller's MSI-X (or MSI as fallback) to
/// `XHCI_VECTOR`. On any failure — no capability, BAR unmapped, etc. — logs
/// a warning and returns. Polling continues to work.
///
/// # Safety
/// Must run after `init_idt()` and after the LAPIC is enabled on the BSP.
/// `pci_addr` must identify the same physical xHCI controller whose `rt_base`
/// is passed. Caller must ensure the BAR holding the MSI-X table is mapped UC.
pub unsafe fn wire_msix(pci_addr: PciAddr, rt_base: u64) {
    // Publish rt_base so the ISR can ack the interrupter.
    XHCI_RT_BASE.store(rt_base, Ordering::Relaxed);

    // Install the IDT entry first so a spurious interrupt during programming
    // does not triple-fault. set_interrupt_handler installs an interrupt gate
    // (IF cleared on entry); DPL=0; IST=0.
    set_interrupt_handler(XHCI_VECTOR, xhci_isr_entry as u64, 0, 0);

    let apic_id = apic::read_lapic_id();

    match msi::enable_msix_single(pci_addr, apic_id, XHCI_VECTOR) {
        Ok(_) => {
            log_ok("XHCI", 950, "MSI-X enabled (vector 0x40)");
            return;
        }
        Err(msi::MsiError::NoCapability) => {
            log_info("XHCI", 951, "no MSI-X capability; trying MSI");
        }
        Err(_) => {
            log_warn("XHCI", 952, "MSI-X enable failed; falling back to polling");
            return;
        }
    }

    match msi::enable_msi_single(pci_addr, apic_id, XHCI_VECTOR) {
        Ok(_) => log_ok("XHCI", 953, "MSI enabled (vector 0x40)"),
        Err(_) => log_warn("XHCI", 954, "no MSI either; polling-only mode"),
    }
}
