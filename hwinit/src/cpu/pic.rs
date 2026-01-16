//! Interrupt Controllers - PIC and APIC
//!
//! We need to manage interrupt controllers to:
//! 1. Remap PIC to avoid conflicts with CPU exceptions
//! 2. Optionally initialize APIC for better interrupt handling
//! 3. Provide a clean interrupt interface for drivers
//!
//! # PIC Remapping
//!
//! ```text
//! Default vectors (conflict with exceptions):
//!   PIC1 (master): IRQ 0-7  → vectors 0x08-0x0F
//!   PIC2 (slave):  IRQ 8-15 → vectors 0x70-0x77
//!
//! After remapping:
//!   PIC1 (master): IRQ 0-7  → vectors 0x20-0x27
//!   PIC2 (slave):  IRQ 8-15 → vectors 0x28-0x2F
//! ```

use crate::cpu::pio::{outb, inb};
use crate::serial::puts;

// ═══════════════════════════════════════════════════════════════════════════
// PIC PORTS
// ═══════════════════════════════════════════════════════════════════════════

/// PIC1 (master) command port
const PIC1_COMMAND: u16 = 0x20;
/// PIC1 (master) data port
const PIC1_DATA: u16 = 0x21;
/// PIC2 (slave) command port
const PIC2_COMMAND: u16 = 0xA0;
/// PIC2 (slave) data port
const PIC2_DATA: u16 = 0xA1;

/// End of interrupt command
const PIC_EOI: u8 = 0x20;

// ═══════════════════════════════════════════════════════════════════════════
// INTERRUPT VECTORS
// ═══════════════════════════════════════════════════════════════════════════

/// Base vector for PIC1 (master) IRQs after remapping
pub const PIC1_VECTOR_OFFSET: u8 = 0x20;
/// Base vector for PIC2 (slave) IRQs after remapping
pub const PIC2_VECTOR_OFFSET: u8 = 0x28;

/// IRQ numbers
pub mod irq {
    pub const TIMER: u8 = 0;
    pub const KEYBOARD: u8 = 1;
    pub const CASCADE: u8 = 2;  // Slave PIC cascade
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

// ═══════════════════════════════════════════════════════════════════════════
// PIC STATE
// ═══════════════════════════════════════════════════════════════════════════

static mut PIC_INITIALIZED: bool = false;
static mut PIC_MASK1: u8 = 0xFF; // All masked initially
static mut PIC_MASK2: u8 = 0xFF;

// ═══════════════════════════════════════════════════════════════════════════
// PIC INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize and remap the 8259 PIC.
///
/// This remaps IRQs to vectors 0x20-0x2F to avoid conflicts with CPU exceptions.
///
/// # Safety
/// Must be called before enabling interrupts.
pub unsafe fn init_pic() {
    if PIC_INITIALIZED {
        puts("[PIC] WARNING: already initialized\n");
        return;
    }

    // ICW1: Initialize + ICW4 needed
    outb(PIC1_COMMAND, 0x11);
    io_wait();
    outb(PIC2_COMMAND, 0x11);
    io_wait();

    // ICW2: Vector offsets
    outb(PIC1_DATA, PIC1_VECTOR_OFFSET);
    io_wait();
    outb(PIC2_DATA, PIC2_VECTOR_OFFSET);
    io_wait();

    // ICW3: Cascade configuration
    outb(PIC1_DATA, 0x04); // IRQ2 has slave
    io_wait();
    outb(PIC2_DATA, 0x02); // Slave ID 2
    io_wait();

    // ICW4: 8086 mode
    outb(PIC1_DATA, 0x01);
    io_wait();
    outb(PIC2_DATA, 0x01);
    io_wait();

    // Mask all interrupts initially
    outb(PIC1_DATA, 0xFF);
    outb(PIC2_DATA, 0xFF);

    PIC_MASK1 = 0xFF;
    PIC_MASK2 = 0xFF;
    PIC_INITIALIZED = true;

    puts("[PIC] remapped to vectors 0x20-0x2F\n");
}

/// Disable the PIC (when using APIC).
///
/// Masks all interrupts on both PICs.
pub unsafe fn disable_pic() {
    outb(PIC1_DATA, 0xFF);
    outb(PIC2_DATA, 0xFF);
    puts("[PIC] disabled\n");
}

// ═══════════════════════════════════════════════════════════════════════════
// IRQ MANAGEMENT
// ═══════════════════════════════════════════════════════════════════════════

/// Enable an IRQ.
pub unsafe fn enable_irq(irq: u8) {
    if irq < 8 {
        PIC_MASK1 &= !(1 << irq);
        outb(PIC1_DATA, PIC_MASK1);
    } else if irq < 16 {
        // Also enable cascade IRQ2 if needed
        if PIC_MASK1 & (1 << 2) != 0 {
            PIC_MASK1 &= !(1 << 2);
            outb(PIC1_DATA, PIC_MASK1);
        }
        PIC_MASK2 &= !(1 << (irq - 8));
        outb(PIC2_DATA, PIC_MASK2);
    }
}

/// Disable an IRQ.
pub unsafe fn disable_irq(irq: u8) {
    if irq < 8 {
        PIC_MASK1 |= 1 << irq;
        outb(PIC1_DATA, PIC_MASK1);
    } else if irq < 16 {
        PIC_MASK2 |= 1 << (irq - 8);
        outb(PIC2_DATA, PIC_MASK2);
    }
}

/// Send end-of-interrupt signal.
pub unsafe fn send_eoi(irq: u8) {
    if irq >= 8 {
        outb(PIC2_COMMAND, PIC_EOI);
    }
    outb(PIC1_COMMAND, PIC_EOI);
}

/// Get IRQ number from interrupt vector.
pub fn vector_to_irq(vector: u8) -> Option<u8> {
    if vector >= PIC1_VECTOR_OFFSET && vector < PIC1_VECTOR_OFFSET + 8 {
        Some(vector - PIC1_VECTOR_OFFSET)
    } else if vector >= PIC2_VECTOR_OFFSET && vector < PIC2_VECTOR_OFFSET + 8 {
        Some(vector - PIC2_VECTOR_OFFSET + 8)
    } else {
        None
    }
}

/// Get interrupt vector from IRQ number.
pub fn irq_to_vector(irq: u8) -> Option<u8> {
    if irq < 8 {
        Some(PIC1_VECTOR_OFFSET + irq)
    } else if irq < 16 {
        Some(PIC2_VECTOR_OFFSET + irq - 8)
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SPURIOUS INTERRUPT HANDLING
// ═══════════════════════════════════════════════════════════════════════════

/// Check if this is a spurious IRQ7 (master PIC).
pub unsafe fn is_spurious_irq7() -> bool {
    // Read In-Service Register
    outb(PIC1_COMMAND, 0x0B);
    let isr = inb(PIC1_COMMAND);
    (isr & 0x80) == 0 // Bit 7 not set = spurious
}

/// Check if this is a spurious IRQ15 (slave PIC).
pub unsafe fn is_spurious_irq15() -> bool {
    outb(PIC2_COMMAND, 0x0B);
    let isr = inb(PIC2_COMMAND);
    if (isr & 0x80) == 0 {
        // Spurious from slave, but must still ACK master
        outb(PIC1_COMMAND, PIC_EOI);
        true
    } else {
        false
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Small delay for PIC I/O
#[inline(always)]
unsafe fn io_wait() {
    // Write to unused port for delay
    outb(0x80, 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// LOCAL APIC (TODO - STUB)
// ═══════════════════════════════════════════════════════════════════════════

/// Local APIC base address (default)
pub const LAPIC_BASE: u64 = 0xFEE0_0000;

/// Check if APIC is available (via CPUID)
pub fn apic_available() -> bool {
    let edx: u32;
    unsafe {
        // Save rbx (used by LLVM), run cpuid, restore rbx
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
    (edx & (1 << 9)) != 0 // APIC flag
}

/// Check if x2APIC is available
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
    (ecx & (1 << 21)) != 0 // x2APIC flag
}
