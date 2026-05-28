//! 8259 PIC remap to vectors 0x20-0x2F. APIC support stubs at the bottom.

use crate::asm::pio::{inb, outb};
use crate::serial::{log_info, log_ok, log_warn};

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const PIC_EOI: u8 = 0x20;

pub const PIC1_VECTOR_OFFSET: u8 = 0x20;
pub const PIC2_VECTOR_OFFSET: u8 = 0x28;

pub mod irq {
    pub const TIMER: u8 = 0;
    pub const KEYBOARD: u8 = 1;
    pub const CASCADE: u8 = 2;
    pub const COM2: u8 = 3;
    pub const COM1: u8 = 4;
    pub const LPT2: u8 = 5;
    pub const FLOPPY: u8 = 6;
    pub const LPT1: u8 = 7;
    pub const RTC: u8 = 8;
    pub const FREE1: u8 = 9;
    pub const FREE2: u8 = 10;
    pub const FREE3: u8 = 11;
    pub const MOUSE: u8 = 12;
    pub const FPU: u8 = 13;
    pub const PRIMARY_ATA: u8 = 14;
    pub const SECONDARY_ATA: u8 = 15;
}

static mut PIC_INITIALIZED: bool = false;
/// All masked initially.
static mut PIC_MASK1: u8 = 0xFF;
static mut PIC_MASK2: u8 = 0xFF;

/// # Safety
/// Before IF=1.
pub unsafe fn init_pic() {
    if PIC_INITIALIZED {
        log_warn("PIC", 750, "already initialized");
        return;
    }

    // ICW1: init + ICW4 needed.
    outb(PIC1_COMMAND, 0x11);
    io_wait();
    outb(PIC2_COMMAND, 0x11);
    io_wait();

    // ICW2: vector offsets.
    outb(PIC1_DATA, PIC1_VECTOR_OFFSET);
    io_wait();
    outb(PIC2_DATA, PIC2_VECTOR_OFFSET);
    io_wait();

    // ICW3: cascade. IRQ2 has slave; slave ID 2.
    outb(PIC1_DATA, 0x04);
    io_wait();
    outb(PIC2_DATA, 0x02);
    io_wait();

    // ICW4: 8086 mode.
    outb(PIC1_DATA, 0x01);
    io_wait();
    outb(PIC2_DATA, 0x01);
    io_wait();

    outb(PIC1_DATA, 0xFF);
    outb(PIC2_DATA, 0xFF);

    PIC_MASK1 = 0xFF;
    PIC_MASK2 = 0xFF;
    PIC_INITIALIZED = true;

    log_ok("PIC", 751, "remapped to vectors 0x20-0x2f");
}

/// Mask all IRQs on both PICs (when using APIC).
///
/// # Safety
/// Performs raw port I/O to the 8259 PICs. Call from single-threaded init/IRQ
/// context only.
pub unsafe fn disable_pic() {
    outb(PIC1_DATA, 0xFF);
    outb(PIC2_DATA, 0xFF);
    log_info("PIC", 752, "disabled");
}

/// # Safety
/// Performs raw port I/O to the 8259 PICs and mutates the cached mask globals.
/// Call from single-threaded init/IRQ context only.
pub unsafe fn enable_irq(irq: u8) {
    if irq < 8 {
        PIC_MASK1 &= !(1 << irq);
        outb(PIC1_DATA, PIC_MASK1);
    } else if irq < 16 {
        // Also unmask cascade IRQ2.
        if PIC_MASK1 & (1 << 2) != 0 {
            PIC_MASK1 &= !(1 << 2);
            outb(PIC1_DATA, PIC_MASK1);
        }
        PIC_MASK2 &= !(1 << (irq - 8));
        outb(PIC2_DATA, PIC_MASK2);
    }
}

/// # Safety
/// Performs raw port I/O to the 8259 PICs and mutates the cached mask globals.
/// Call from single-threaded init/IRQ context only.
pub unsafe fn disable_irq(irq: u8) {
    if irq < 8 {
        PIC_MASK1 |= 1 << irq;
        outb(PIC1_DATA, PIC_MASK1);
    } else if irq < 16 {
        PIC_MASK2 |= 1 << (irq - 8);
        outb(PIC2_DATA, PIC_MASK2);
    }
}

/// # Safety
/// Performs raw port I/O to the 8259 PICs. Call only at the end of a PIC-sourced
/// IRQ for the given `irq`.
pub unsafe fn send_eoi(irq: u8) {
    if irq >= 8 {
        outb(PIC2_COMMAND, PIC_EOI);
    }
    outb(PIC1_COMMAND, PIC_EOI);
}

pub fn vector_to_irq(vector: u8) -> Option<u8> {
    if (PIC1_VECTOR_OFFSET..PIC1_VECTOR_OFFSET + 8).contains(&vector) {
        Some(vector - PIC1_VECTOR_OFFSET)
    } else if (PIC2_VECTOR_OFFSET..PIC2_VECTOR_OFFSET + 8).contains(&vector) {
        Some(vector - PIC2_VECTOR_OFFSET + 8)
    } else {
        None
    }
}

pub fn irq_to_vector(irq: u8) -> Option<u8> {
    if irq < 8 {
        Some(PIC1_VECTOR_OFFSET + irq)
    } else if irq < 16 {
        Some(PIC2_VECTOR_OFFSET + irq - 8)
    } else {
        None
    }
}

/// ISR bit 7 clear ⇒ spurious.
///
/// # Safety
/// Performs raw port I/O to the master PIC. Call from IRQ7 handler context only.
pub unsafe fn is_spurious_irq7() -> bool {
    outb(PIC1_COMMAND, 0x0B);
    let isr = inb(PIC1_COMMAND);
    (isr & 0x80) == 0
}

/// Slave spurious still requires master ACK.
///
/// # Safety
/// Performs raw port I/O to both PICs. Call from IRQ15 handler context only.
pub unsafe fn is_spurious_irq15() -> bool {
    outb(PIC2_COMMAND, 0x0B);
    let isr = inb(PIC2_COMMAND);
    if (isr & 0x80) == 0 {
        outb(PIC1_COMMAND, PIC_EOI);
        true
    } else {
        false
    }
}

/// Port 0x80 unused-port write = ~1 µs I/O delay.
#[inline(always)]
unsafe fn io_wait() {
    outb(0x80, 0);
}

// LAPIC stubs.

pub const LAPIC_BASE: u64 = 0xFEE0_0000;

pub fn apic_available() -> bool {
    let edx: u32;
    unsafe {
        // push/pop rbx — LLVM reserves it.
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            out("edx") edx,
            out("eax") _,
            out("ecx") _,
            options(nostack),
        );
    }
    // APIC flag.
    (edx & (1 << 9)) != 0
}

pub fn x2apic_available() -> bool {
    let ecx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            out("ecx") ecx,
            out("eax") _,
            out("edx") _,
            options(nostack),
        );
    }
    // x2APIC flag.
    (ecx & (1 << 21)) != 0
}
