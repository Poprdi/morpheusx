//! Interrupt Descriptor Table (IDT) Management
//!
//! Sets up the IDT for exception and interrupt handling. After this,
//! we own all CPU exceptions and can handle hardware interrupts.
//!
//! # Vector Layout
//!
//! | Vector | Exception              | Type      |
//! |--------|------------------------|-----------|
//! | 0      | Divide Error           | Fault     |
//! | 1      | Debug                  | Fault/Trap|
//! | 2      | NMI                    | Interrupt |
//! | 3      | Breakpoint             | Trap      |
//! | 4      | Overflow               | Trap      |
//! | 5      | Bound Range Exceeded   | Fault     |
//! | 6      | Invalid Opcode         | Fault     |
//! | 7      | Device Not Available   | Fault     |
//! | 8      | Double Fault           | Abort     |
//! | 10     | Invalid TSS            | Fault     |
//! | 11     | Segment Not Present    | Fault     |
//! | 12     | Stack-Segment Fault    | Fault     |
//! | 13     | General Protection     | Fault     |
//! | 14     | Page Fault             | Fault     |
//! | 16     | x87 FP Exception       | Fault     |
//! | 17     | Alignment Check        | Fault     |
//! | 18     | Machine Check          | Abort     |
//! | 19     | SIMD FP Exception      | Fault     |
//! | 20     | Virtualization         | Fault     |
//! | 21     | Control Protection     | Fault     |
//! | 32-47  | IRQs (remapped PIC)    | Interrupt |
//! | 48-255 | User-defined           | Interrupt |

use crate::cpu::gdt::KERNEL_CS;
use crate::serial::{puts, put_hex64, put_hex8, newline};

// ═══════════════════════════════════════════════════════════════════════════
// IDT ENTRY
// ═══════════════════════════════════════════════════════════════════════════

/// IDT entry (16 bytes in long mode)
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,           // IST index (0 = no IST)
    type_attr: u8,     // Type and attributes
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    /// Create a null/absent entry
    pub const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    /// Create an interrupt gate entry
    ///
    /// # Arguments
    /// - `handler`: Handler function address
    /// - `ist`: IST index (1-7) or 0 for no IST
    /// - `dpl`: Descriptor privilege level (0-3)
    pub fn interrupt_gate(handler: u64, ist: u8, dpl: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector: KERNEL_CS,
            ist: ist & 0x7,
            // Present | DPL | Interrupt Gate (0xE)
            type_attr: 0x80 | ((dpl & 3) << 5) | 0x0E,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }

    /// Create a trap gate entry (doesn't disable interrupts)
    pub fn trap_gate(handler: u64, ist: u8, dpl: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector: KERNEL_CS,
            ist: ist & 0x7,
            // Present | DPL | Trap Gate (0xF)
            type_attr: 0x80 | ((dpl & 3) << 5) | 0x0F,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// IDT POINTER
// ═══════════════════════════════════════════════════════════════════════════

/// IDT pointer for lidt instruction
#[repr(C, packed)]
pub struct IdtPtr {
    pub limit: u16,
    pub base: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// EXCEPTION FRAME
// ═══════════════════════════════════════════════════════════════════════════

/// Exception stack frame pushed by CPU
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ExceptionFrame {
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Extended exception frame with error code
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ExceptionFrameWithError {
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// IDT TABLE
// ═══════════════════════════════════════════════════════════════════════════

/// Number of IDT entries
const IDT_ENTRIES: usize = 256;

/// The IDT
#[repr(C, align(16))]
pub struct Idt {
    entries: [IdtEntry; IDT_ENTRIES],
}

impl Idt {
    /// Create empty IDT
    pub const fn new() -> Self {
        Self {
            entries: [IdtEntry::missing(); IDT_ENTRIES],
        }
    }

    /// Set handler for a vector
    pub fn set_handler(&mut self, vector: u8, entry: IdtEntry) {
        self.entries[vector as usize] = entry;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Our IDT
static mut IDT: Idt = Idt::new();

/// IDT initialized flag
static mut IDT_INITIALIZED: bool = false;

// ═══════════════════════════════════════════════════════════════════════════
// EXCEPTION HANDLERS
// ═══════════════════════════════════════════════════════════════════════════

/// Exception names for debugging
const EXCEPTION_NAMES: [&str; 32] = [
    "Divide Error",
    "Debug",
    "NMI",
    "Breakpoint",
    "Overflow",
    "Bound Range",
    "Invalid Opcode",
    "Device Not Available",
    "Double Fault",
    "Coprocessor Segment",
    "Invalid TSS",
    "Segment Not Present",
    "Stack-Segment Fault",
    "General Protection",
    "Page Fault",
    "Reserved",
    "x87 FP Exception",
    "Alignment Check",
    "Machine Check",
    "SIMD FP Exception",
    "Virtualization",
    "Control Protection",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
];

/// Generic exception handler (called from assembly stubs)
#[no_mangle]
pub extern "C" fn exception_handler(vector: u64, error_code: u64, frame: &ExceptionFrame) {
    puts("\n!!! EXCEPTION ");
    put_hex8(vector as u8);
    puts(": ");

    if (vector as usize) < EXCEPTION_NAMES.len() {
        puts(EXCEPTION_NAMES[vector as usize]);
    } else {
        puts("Unknown");
    }
    newline();

    puts("  Error code: ");
    put_hex64(error_code);
    newline();

    puts("  RIP: ");
    put_hex64(frame.rip);
    newline();

    puts("  RSP: ");
    put_hex64(frame.rsp);
    newline();

    puts("  RFLAGS: ");
    put_hex64(frame.rflags);
    newline();

    // For now, halt on exception
    puts("!!! SYSTEM HALTED\n");
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

/// Page fault handler (vector 14) - more detailed
#[no_mangle]
pub extern "C" fn page_fault_handler(error_code: u64, frame: &ExceptionFrame) {
    // CR2 contains faulting address
    let cr2: u64;
    unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2); }

    puts("\n!!! PAGE FAULT\n");
    puts("  Faulting address: ");
    put_hex64(cr2);
    newline();

    puts("  Error code: ");
    put_hex64(error_code);
    puts(" (");
    if error_code & 1 != 0 { puts("P "); } else { puts("NP "); }
    if error_code & 2 != 0 { puts("W "); } else { puts("R "); }
    if error_code & 4 != 0 { puts("U "); } else { puts("S "); }
    if error_code & 8 != 0 { puts("RSVD "); }
    if error_code & 16 != 0 { puts("I "); }
    puts(")\n");

    puts("  RIP: ");
    put_hex64(frame.rip);
    newline();

    puts("!!! SYSTEM HALTED\n");
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ASSEMBLY STUBS
// ═══════════════════════════════════════════════════════════════════════════

// Exception stubs - these push the vector number and jump to common handler
// Exceptions with error codes: 8, 10, 11, 12, 13, 14, 17, 21
// Others don't have error code, so we push a dummy 0

macro_rules! exception_stub_no_error {
    ($name:ident, $vector:expr) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            core::arch::naked_asm!(
                "push 0",           // Dummy error code
                "push {}",          // Vector number
                "jmp {}",
                const $vector,
                sym exception_common,
            );
        }
    };
}

macro_rules! exception_stub_with_error {
    ($name:ident, $vector:expr) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            core::arch::naked_asm!(
                "push {}",          // Vector number (error code already pushed by CPU)
                "jmp {}",
                const $vector,
                sym exception_common,
            );
        }
    };
}

/// Common exception handler stub
#[unsafe(naked)]
unsafe extern "C" fn exception_common() {
    core::arch::naked_asm!(
        // Stack: [ss, rsp, rflags, cs, rip, error_code, vector]
        // Save all registers
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Call Rust handler
        // rdi = vector, rsi = error_code, rdx = frame pointer
        "mov rdi, [rsp + 120]",     // vector (15 regs * 8 = 120)
        "mov rsi, [rsp + 128]",     // error_code
        "lea rdx, [rsp + 136]",     // frame starts after error_code
        "call {}",

        // Restore registers
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",

        // Remove vector and error code
        "add rsp, 16",

        // Return from interrupt
        "iretq",
        sym exception_handler,
    );
}

// Define exception stubs
exception_stub_no_error!(exc_divide_error, 0);
exception_stub_no_error!(exc_debug, 1);
exception_stub_no_error!(exc_nmi, 2);
exception_stub_no_error!(exc_breakpoint, 3);
exception_stub_no_error!(exc_overflow, 4);
exception_stub_no_error!(exc_bound_range, 5);
exception_stub_no_error!(exc_invalid_opcode, 6);
exception_stub_no_error!(exc_device_not_available, 7);
exception_stub_with_error!(exc_double_fault, 8);
exception_stub_with_error!(exc_invalid_tss, 10);
exception_stub_with_error!(exc_segment_not_present, 11);
exception_stub_with_error!(exc_stack_segment, 12);
exception_stub_with_error!(exc_general_protection, 13);
exception_stub_with_error!(exc_page_fault, 14);
exception_stub_no_error!(exc_x87_fp, 16);
exception_stub_with_error!(exc_alignment_check, 17);
exception_stub_no_error!(exc_machine_check, 18);
exception_stub_no_error!(exc_simd_fp, 19);
exception_stub_no_error!(exc_virtualization, 20);
exception_stub_with_error!(exc_control_protection, 21);

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize IDT with exception handlers.
///
/// # Safety
/// Must be called after GDT is initialized.
pub unsafe fn init_idt() {
    if IDT_INITIALIZED {
        puts("[IDT] WARNING: already initialized\n");
        return;
    }

    // Set up exception handlers
    // Use IST1 for critical exceptions (double fault, NMI, machine check)

    IDT.set_handler(0, IdtEntry::interrupt_gate(exc_divide_error as u64, 0, 0));
    IDT.set_handler(1, IdtEntry::interrupt_gate(exc_debug as u64, 0, 0));
    IDT.set_handler(2, IdtEntry::interrupt_gate(exc_nmi as u64, 1, 0));  // IST1
    IDT.set_handler(3, IdtEntry::trap_gate(exc_breakpoint as u64, 0, 3)); // Allow from userspace
    IDT.set_handler(4, IdtEntry::interrupt_gate(exc_overflow as u64, 0, 0));
    IDT.set_handler(5, IdtEntry::interrupt_gate(exc_bound_range as u64, 0, 0));
    IDT.set_handler(6, IdtEntry::interrupt_gate(exc_invalid_opcode as u64, 0, 0));
    IDT.set_handler(7, IdtEntry::interrupt_gate(exc_device_not_available as u64, 0, 0));
    IDT.set_handler(8, IdtEntry::interrupt_gate(exc_double_fault as u64, 1, 0)); // IST1
    IDT.set_handler(10, IdtEntry::interrupt_gate(exc_invalid_tss as u64, 0, 0));
    IDT.set_handler(11, IdtEntry::interrupt_gate(exc_segment_not_present as u64, 0, 0));
    IDT.set_handler(12, IdtEntry::interrupt_gate(exc_stack_segment as u64, 0, 0));
    IDT.set_handler(13, IdtEntry::interrupt_gate(exc_general_protection as u64, 0, 0));
    IDT.set_handler(14, IdtEntry::interrupt_gate(exc_page_fault as u64, 0, 0));
    IDT.set_handler(16, IdtEntry::interrupt_gate(exc_x87_fp as u64, 0, 0));
    IDT.set_handler(17, IdtEntry::interrupt_gate(exc_alignment_check as u64, 0, 0));
    IDT.set_handler(18, IdtEntry::interrupt_gate(exc_machine_check as u64, 1, 0)); // IST1
    IDT.set_handler(19, IdtEntry::interrupt_gate(exc_simd_fp as u64, 0, 0));
    IDT.set_handler(20, IdtEntry::interrupt_gate(exc_virtualization as u64, 0, 0));
    IDT.set_handler(21, IdtEntry::interrupt_gate(exc_control_protection as u64, 0, 0));

    // Load IDT
    let idt_ptr = IdtPtr {
        limit: (core::mem::size_of::<Idt>() - 1) as u16,
        base: &IDT as *const Idt as u64,
    };

    core::arch::asm!(
        "lidt [{}]",
        in(reg) &idt_ptr,
        options(nostack, preserves_flags)
    );

    IDT_INITIALIZED = true;
    puts("[IDT] initialized (exceptions only)\n");
}

/// Set a custom interrupt handler for a vector.
///
/// # Safety
/// Handler must be a valid interrupt handler.
pub unsafe fn set_interrupt_handler(vector: u8, handler: u64, ist: u8, dpl: u8) {
    IDT.set_handler(vector, IdtEntry::interrupt_gate(handler, ist, dpl));
}

/// Enable interrupts
#[inline(always)]
pub fn enable_interrupts() {
    unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
}

/// Disable interrupts
#[inline(always)]
pub fn disable_interrupts() {
    unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
}

/// Check if interrupts are enabled
pub fn interrupts_enabled() -> bool {
    let rflags: u64;
    unsafe { core::arch::asm!("pushfq; pop {}", out(reg) rflags); }
    (rflags & 0x200) != 0
}
