//! xHCI MSI-X interrupt wiring. The ISR does NOT drain the event ring — it
//! bumps a counter, W1Cs IMAN.IP, and EOIs the LAPIC; the polling `wait_cmd` /
//! `wait_xfer` paths remain authoritative. IDT/LAPIC/MSI programming routes
//! through `morpheus_kernel::hal()`'s `InterruptController` trait; the IMAN ack
//! stays inline since the trait exposes no raw register access.

use crate::regs::RT_IR0_IMAN;
use core::sync::atomic::{AtomicU64, Ordering};
use morpheus_hal_api::{BusAddr, IsrFn, MsiError};
use morpheus_x86_asm::mmio;

/// IDT vector reserved for xHCI MSI-X. Above the PIC remap range (0x20-0x2F),
/// clear of the timer (0x20).
pub const XHCI_VECTOR: u8 = 0x40;

/// Diagnostic counter of observed xHCI MSI-X interrupts.
pub static XHCI_EVENT_COUNT: AtomicU64 = AtomicU64::new(0);

/// xHCI runtime register base, published by `wire_msix` so the ISR can W1C the
/// interrupter IP bit without borrowing the (Phase-9-owned) controller struct.
static XHCI_RT_BASE: AtomicU64 = AtomicU64::new(0);

/// Interrupt-context handler: no alloc, no sleeping locks. Acks at the device,
/// then EOIs the LAPIC.
extern "C" fn xhci_isr_rust() {
    XHCI_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);

    // Ack the interrupter: W1C the IP bit (bit 0) of IMAN. Preserve IE (bit 1).
    let rt_base = XHCI_RT_BASE.load(Ordering::Relaxed);
    if rt_base != 0 {
        // SAFETY: rt_base was published by `wire_msix` from the xHCI
        // controller's verified UC MMIO mapping. Single-vector W1C of IP is
        // race-safe (no preempt on this vector; LAPIC EOI happens below).
        unsafe {
            let iman = mmio::read32(rt_base + RT_IR0_IMAN);
            mmio::write32(rt_base + RT_IR0_IMAN, iman | 0x01);
        }
    }

    morpheus_kernel::hal().intr().send_lapic_eoi();
}

/// Thunk: save caller-saved GPRs, call the Rust handler (MS x64 ABI + shadow
/// space), restore, `iretq`. No vector/frame passed.
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

/// Wire the controller's MSI-X (or MSI fallback) to `XHCI_VECTOR`. On any
/// failure logs a warning and returns; polling continues to work.
///
/// # Safety
/// Must run after the IDT is initialized and after the LAPIC is enabled on
/// the BSP. `pci_addr` must identify the same physical xHCI controller whose
/// `rt_base` is passed. Caller must ensure the BAR holding the MSI-X table is
/// mapped UC.
pub unsafe fn wire_msix(pci_addr: BusAddr, rt_base: u64) {
    // Publish rt_base so the ISR can ack the interrupter.
    XHCI_RT_BASE.store(rt_base, Ordering::Relaxed);

    let intr = morpheus_kernel::hal().intr();

    // Install the IDT entry first so a spurious interrupt during programming
    // does not triple-fault. `set_handler` installs an interrupt gate
    // (IF cleared on entry); DPL=0; IST=0.
    intr.set_handler(
        XHCI_VECTOR,
        IsrFn(xhci_isr_entry as unsafe extern "C" fn()),
        0,
        0,
    );

    let apic_id = intr.read_lapic_id();

    match intr.enable_msix_single(pci_addr, apic_id, XHCI_VECTOR) {
        Ok(_) => {
            crate::logger::ok("XHCI", 950, "MSI-X enabled (vector 0x40)");
            return;
        },
        Err(MsiError::CapabilityNotFound) => {
            crate::logger::info("XHCI", 951, "no MSI-X capability; trying MSI");
        },
        Err(_) => {
            crate::logger::warn("XHCI", 952, "MSI-X enable failed; falling back to polling");
            return;
        },
    }

    match intr.enable_msi_single(pci_addr, apic_id, XHCI_VECTOR) {
        Ok(_) => crate::logger::ok("XHCI", 953, "MSI enabled (vector 0x40)"),
        Err(_) => crate::logger::warn("XHCI", 954, "no MSI either; polling-only mode"),
    }
}
