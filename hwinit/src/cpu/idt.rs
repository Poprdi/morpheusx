//! IDT. 0-31 = exceptions, 32-47 = PIC IRQs, 48+ = ours.
//! Double fault on IST1 so we get a stack even when the kernel stack is toast.

use crate::cpu::gdt::KERNEL_CS;
use crate::serial::{newline, put_hex32, put_hex64, put_hex8, puts};

// IDT ENTRY

/// IDT entry (16 bytes in long mode)
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,       // IST index (0 = no IST)
    type_attr: u8, // Type and attributes
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

// IDT POINTER

/// IDT pointer for lidt instruction
#[repr(C, packed)]
pub struct IdtPtr {
    pub limit: u16,
    pub base: u64,
}

// EXCEPTION FRAME

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

// IDT TABLE

// CRASH DIAGNOSTICS — rich crash info for the BSoD

/// Saved general-purpose registers — layout matches push order in `exception_common`.
///
/// Push order: rax, rbx, rcx, rdx, rsi, rdi, rbp, r8..r15.
/// Stack grows down → r15 at lowest address.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SavedRegs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
}

/// Rich crash diagnostic — built entirely on the kernel stack, **zero allocation**.
///
/// Contains everything a developer needs to diagnose what went wrong:
/// exception identity, all CPU registers, process context, human-readable
/// explanation, and a best-effort kernel-mode stack backtrace.
#[repr(C)]
pub struct CrashInfo {
    // exception identity
    /// CPU exception vector (0–31) or interrupt number.
    pub vector: u64,
    /// Hardware error code pushed by the CPU (0 if none).
    pub error_code: u64,
    /// Human-readable exception name (e.g. "Page Fault").
    pub exception_name: &'static str,

    // cpu exception frame (pushed by hardware)
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,

    // control registers
    /// CR2: faulting linear address (meaningful only for #PF, vector 14).
    pub cr2: u64,
    /// CR3: page-table root physical address.
    pub cr3: u64,

    // general-purpose registers at the instant of the fault
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,

    // process context (lock-free, best-effort read)
    /// PID of the process executing when the fault occurred.
    pub pid: u32,
    /// Process name, NUL-terminated.  "[kernel]" for PID 0 / unknown.
    pub process_name: [u8; 32],
    /// True if the fault originated in user mode (CPL 3).
    pub is_user_mode: bool,

    // stack backtrace (kernel-mode only, best-effort rbp walk)
    /// Return addresses from the RBP frame chain (most recent first).
    pub backtrace: [u64; 16],
    /// Number of valid entries in `backtrace`.
    pub backtrace_depth: u8,

    // human-readable diagnostic
    /// One-line explanation of what went wrong.
    pub explanation: [u8; 256],
    /// Length of the explanation text.
    pub explanation_len: u16,
}

/// Crash hook callback — receives a reference to the rich crash diagnostics.
///
/// # Safety
/// Called from exception context with interrupts disabled.  Must not
/// allocate or acquire any lock.  The reference points to a stack-local
/// struct inside the exception handler frame.
pub type CrashHookFn = unsafe fn(&CrashInfo);

static mut CRASH_HOOK: Option<CrashHookFn> = None;

/// Register the crash screen callback (typically called during boot init).
///
/// # Safety
/// Must be called during single-threaded init.
pub unsafe fn set_crash_hook(hook: CrashHookFn) {
    CRASH_HOOK = Some(hook);
}

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

// GLOBAL STATE

/// Our IDT
static mut IDT: Idt = Idt::new();

/// IDT initialized flag
static mut IDT_INITIALIZED: bool = false;

// EXCEPTION HANDLERS

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

// crash diagnostic helpers (zero-alloc)

/// Tiny stack-only buffer writer for building diagnostic strings.
struct BufWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> BufWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn len(&self) -> usize {
        self.pos
    }
    fn push(&mut self, s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(self.buf.len() - self.pos);
        self.buf[self.pos..self.pos + n].copy_from_slice(&b[..n]);
        self.pos += n;
    }
    fn hex64(&mut self, val: u64) {
        const H: &[u8; 16] = b"0123456789ABCDEF";
        self.push("0x");
        for i in (0..16).rev() {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = H[((val >> (i * 4)) & 0xF) as usize];
                self.pos += 1;
            }
        }
    }
}

/// Build a human-readable explanation of the exception.
fn build_explanation(vec: u64, ec: u64, cr2: u64, user: bool, buf: &mut [u8; 256]) -> u16 {
    let mut w = BufWriter::new(buf);
    match vec {
        0 => w.push("Division by zero"),
        1 => w.push("Hardware debug trap"),
        2 => w.push("Non-maskable interrupt"),
        3 => w.push("Breakpoint (INT3)"),
        4 => w.push("Arithmetic overflow (INTO)"),
        5 => w.push("Array index out of bounds (BOUND)"),
        6 => {
            w.push("Invalid CPU instruction");
            if user {
                w.push(" \u{2014} bad user code or corrupted binary");
            } else {
                w.push(" \u{2014} kernel bug");
            }
        }
        7 => w.push("FPU/SSE instruction but device not available"),
        8 => w.push("Double fault: exception while handling another exception"),
        10 => w.push("Invalid Task State Segment"),
        11 => w.push("Segment not present in descriptor table"),
        12 => w.push("Stack segment fault: stack overflow or bad SS"),
        13 => {
            w.push("General protection fault");
            if user {
                w.push(" \u{2014} process tried a privileged operation");
            }
        }
        14 => {
            w.push("Attempted to ");
            if ec & 16 != 0 {
                w.push("execute");
            } else if ec & 2 != 0 {
                w.push("write to");
            } else {
                w.push("read from");
            }
            if ec & 1 != 0 {
                w.push(" protected");
            } else {
                w.push(" unmapped");
            }
            w.push(" memory at ");
            w.hex64(cr2);
            if user {
                w.push(" from user mode");
            } else {
                w.push(" from kernel");
            }
        }
        16 => w.push("x87 floating-point exception"),
        17 => w.push("Unaligned memory access (alignment check)"),
        18 => w.push("Machine check: uncorrectable hardware error"),
        19 => w.push("SIMD floating-point exception"),
        20 => w.push("Virtualization exception"),
        21 => w.push("Control-flow integrity violation (CET)"),
        _ => w.push("Unknown exception"),
    }
    w.len() as u16
}

/// Walk the RBP frame chain (kernel-mode only) to collect return addresses.
unsafe fn walk_stack(rbp: u64, out: &mut [u64; 16]) -> u8 {
    let mut fp = rbp;
    let mut depth: u8 = 0;
    for _ in 0..16u8 {
        // Sanity: aligned, non-null, in a plausible kernel range
        if fp == 0 || !fp.is_multiple_of(8) || !(0x1000..=0x0000_7FFF_FFFF_FFFF).contains(&fp) {
            break;
        }
        let ret = core::ptr::read_volatile((fp + 8) as *const u64);
        let prev = core::ptr::read_volatile(fp as *const u64);
        if ret == 0 {
            break;
        }
        out[depth as usize] = ret;
        depth += 1;
        if prev <= fp {
            break;
        } // prevent loops
        fp = prev;
    }
    depth
}

/// Unified exception handler — builds a rich [`CrashInfo`] and invokes the BSoD hook.
///
/// Called from the `exception_common` ASM stub for ALL CPU exceptions (vectors 0–21).
/// Arguments arrive via MS x64 ABI: rcx=vector, rdx=error_code, r8=&frame, r9=&saved.
#[no_mangle]
pub extern "C" fn exception_handler(
    vector: u64,
    error_code: u64,
    frame: &ExceptionFrame,
    saved: &SavedRegs,
) {
    // serial dump (always, before anything that could re-fault)
    let exc_name = if (vector as usize) < EXCEPTION_NAMES.len() {
        EXCEPTION_NAMES[vector as usize]
    } else {
        "Unknown"
    };

    puts("\n!!! EXCEPTION ");
    put_hex8(vector as u8);
    puts(": ");
    puts(exc_name);
    newline();
    puts("  Error: ");
    put_hex64(error_code);
    newline();
    puts("  RIP:   ");
    put_hex64(frame.rip);
    newline();
    puts("  RSP:   ");
    put_hex64(frame.rsp);
    newline();
    puts("  CS:    ");
    put_hex64(frame.cs);
    newline();

    // Control registers
    let (cr2, cr3): (u64, u64);
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2);
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    if vector == 14 {
        puts("  CR2:   ");
        put_hex64(cr2);
        newline();
    }
    puts("  CR3:   ");
    put_hex64(cr3);
    newline();

    // Process context (lock-free read — safe even pre-scheduler-init)
    let pid = crate::process::SCHEDULER.current_pid();
    let mut proc_name = [0u8; 32];
    unsafe {
        if let Some(p) = crate::process::SCHEDULER.process_by_pid(pid) {
            proc_name = p.name;
        } else {
            proc_name[..8].copy_from_slice(b"[kernel]");
        }
    }
    let is_user_mode = (frame.cs & 3) == 3;

    puts("  PID:   ");
    put_hex32(pid);
    puts("  Name: ");
    let name_end = proc_name.iter().position(|&b| b == 0).unwrap_or(32);
    if let Ok(s) = core::str::from_utf8(&proc_name[..name_end]) {
        puts(s);
    }
    puts(if is_user_mode {
        " [USER]\n"
    } else {
        " [KERNEL]\n"
    });

    // GPR dump
    puts("  RAX: ");
    put_hex64(saved.rax);
    puts("  RBX: ");
    put_hex64(saved.rbx);
    newline();
    puts("  RCX: ");
    put_hex64(saved.rcx);
    puts("  RDX: ");
    put_hex64(saved.rdx);
    newline();
    puts("  RSI: ");
    put_hex64(saved.rsi);
    puts("  RDI: ");
    put_hex64(saved.rdi);
    newline();
    puts("  RBP: ");
    put_hex64(saved.rbp);
    puts("  R8:  ");
    put_hex64(saved.r8);
    newline();
    puts("  R9:  ");
    put_hex64(saved.r9);
    puts("  R10: ");
    put_hex64(saved.r10);
    newline();
    puts("  R11: ");
    put_hex64(saved.r11);
    puts("  R12: ");
    put_hex64(saved.r12);
    newline();
    puts("  R13: ");
    put_hex64(saved.r13);
    puts("  R14: ");
    put_hex64(saved.r14);
    newline();
    puts("  R15: ");
    put_hex64(saved.r15);
    newline();

    // Explanation
    let mut explanation = [0u8; 256];
    let explanation_len =
        build_explanation(vector, error_code, cr2, is_user_mode, &mut explanation);
    if explanation_len > 0 {
        puts("  WHY:   ");
        if let Ok(s) = core::str::from_utf8(&explanation[..explanation_len as usize]) {
            puts(s);
        }
        newline();
    }

    // Page fault diagnostic: full page table walk from CR3 for CR2
    if vector == 14 {
        unsafe {
            puts("  ── PT walk for CR2 ──\n");
            let va = cr2;
            let pml4_idx = ((va >> 39) & 0x1FF) as usize;
            let pdpt_idx = ((va >> 30) & 0x1FF) as usize;
            let pd_idx = ((va >> 21) & 0x1FF) as usize;
            let pt_idx = ((va >> 12) & 0x1FF) as usize;

            puts("  indices: PML4[");
            put_hex32(pml4_idx as u32);
            puts("] PDPT[");
            put_hex32(pdpt_idx as u32);
            puts("] PD[");
            put_hex32(pd_idx as u32);
            puts("] PT[");
            put_hex32(pt_idx as u32);
            puts("]\n");

            let pml4_base = cr3 & 0x000F_FFFF_FFFF_F000;
            let pml4_entry = *((pml4_base + pml4_idx as u64 * 8) as *const u64);
            puts("  PML4e: ");
            put_hex64(pml4_entry);
            if pml4_entry & 1 == 0 {
                puts(" NOT PRESENT\n");
            } else {
                puts(if pml4_entry & 4 != 0 { " U" } else { " S" });
                newline();

                let pdpt_base = pml4_entry & 0x000F_FFFF_FFFF_F000;
                let pdpt_entry = *((pdpt_base + pdpt_idx as u64 * 8) as *const u64);
                puts("  PDPTe: ");
                put_hex64(pdpt_entry);
                if pdpt_entry & 1 == 0 {
                    puts(" NOT PRESENT\n");
                } else {
                    puts(if pdpt_entry & 4 != 0 { " U" } else { " S" });
                    if pdpt_entry & 0x80 != 0 {
                        puts(" HUGE\n");
                    } else {
                        newline();

                        let pd_base = pdpt_entry & 0x000F_FFFF_FFFF_F000;
                        let pd_entry = *((pd_base + pd_idx as u64 * 8) as *const u64);
                        puts("  PDe:   ");
                        put_hex64(pd_entry);
                        if pd_entry & 1 == 0 {
                            puts(" NOT PRESENT\n");
                        } else {
                            puts(if pd_entry & 4 != 0 { " U" } else { " S" });
                            if pd_entry & 0x80 != 0 {
                                puts(" HUGE\n");
                            } else {
                                newline();

                                let pt_base = pd_entry & 0x000F_FFFF_FFFF_F000;
                                let pt_entry = *((pt_base + pt_idx as u64 * 8) as *const u64);
                                puts("  PTe:   ");
                                put_hex64(pt_entry);
                                if pt_entry & 1 == 0 {
                                    puts(" NOT PRESENT\n");
                                } else {
                                    puts(if pt_entry & 4 != 0 { " U" } else { " S" });
                                    puts(if pt_entry & 2 != 0 { " W" } else { " R" });
                                    puts(" phys=");
                                    put_hex64(pt_entry & 0x000F_FFFF_FFFF_F000);
                                    newline();
                                }
                            }
                        }
                    }
                }
            }
            puts("  ── end PT walk ──\n");
        }
    }

    // Backtrace (kernel only — user pages may be unmapped)
    let mut backtrace = [0u64; 16];
    let backtrace_depth = if !is_user_mode {
        unsafe { walk_stack(saved.rbp, &mut backtrace) }
    } else {
        0
    };
    if backtrace_depth > 0 {
        puts("  Backtrace:\n");
        for (i, &addr) in backtrace.iter().enumerate().take(backtrace_depth as usize) {
            puts("    #");
            put_hex8(i as u8);
            puts(" ");
            put_hex64(addr);
            newline();
        }
    }

    // build crashinfo & invoke bsod hook
    let info = CrashInfo {
        vector,
        error_code,
        exception_name: exc_name,
        rip: frame.rip,
        cs: frame.cs,
        rflags: frame.rflags,
        rsp: frame.rsp,
        ss: frame.ss,
        cr2,
        cr3,
        rax: saved.rax,
        rbx: saved.rbx,
        rcx: saved.rcx,
        rdx: saved.rdx,
        rsi: saved.rsi,
        rdi: saved.rdi,
        rbp: saved.rbp,
        r8: saved.r8,
        r9: saved.r9,
        r10: saved.r10,
        r11: saved.r11,
        r12: saved.r12,
        r13: saved.r13,
        r14: saved.r14,
        r15: saved.r15,
        pid,
        process_name: proc_name,
        is_user_mode,
        backtrace,
        backtrace_depth,
        explanation,
        explanation_len,
    };

    if is_user_mode {
        let code = if vector == 14 {
            -11
        } else {
            -128 - (vector as i32)
        };
        puts("[EXC] user fault -> terminating PID ");
        put_hex32(pid);
        puts("\n");
        unsafe {
            crate::process::scheduler::exit_process(code);
        }
    }

    unsafe {
        if let Some(hook) = CRASH_HOOK {
            hook(&info);
        }
    }

    puts("!!! SYSTEM HALTED\n");
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}

// ASSEMBLY STUBS

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

        // Call Rust handler (MS x64 ABI: rcx, rdx, r8, r9)
        // rcx = vector, rdx = error_code, r8 = &ExceptionFrame, r9 = &SavedRegs
        "mov rcx, [rsp + 120]",     // vector (15 regs * 8 = 120)
        "mov rdx, [rsp + 128]",     // error_code
        "lea r8, [rsp + 136]",      // frame starts after error_code
        "mov r9, rsp",              // saved GPRs (r15..rax on stack)
        "sub rsp, 32",              // shadow space (MS x64 ABI)
        "call {}",
        "add rsp, 32",

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

// INITIALIZATION

/// Initialize IDT with exception handlers.
///
/// # Safety
/// Must be called after GDT is initialized.
pub unsafe fn init_idt() {
    if IDT_INITIALIZED {
        crate::serial::log_warn("IDT", 740, "already initialized");
        return;
    }

    // Set up exception handlers
    // Use IST1 for critical exceptions (double fault, NMI, machine check)

    IDT.set_handler(0, IdtEntry::interrupt_gate(exc_divide_error as u64, 0, 0));
    IDT.set_handler(1, IdtEntry::interrupt_gate(exc_debug as u64, 0, 0));
    IDT.set_handler(2, IdtEntry::interrupt_gate(exc_nmi as u64, 1, 0)); // IST1
    IDT.set_handler(3, IdtEntry::trap_gate(exc_breakpoint as u64, 0, 3)); // Allow from userspace
    IDT.set_handler(4, IdtEntry::interrupt_gate(exc_overflow as u64, 0, 0));
    IDT.set_handler(5, IdtEntry::interrupt_gate(exc_bound_range as u64, 0, 0));
    IDT.set_handler(6, IdtEntry::interrupt_gate(exc_invalid_opcode as u64, 0, 0));
    IDT.set_handler(
        7,
        IdtEntry::interrupt_gate(exc_device_not_available as u64, 0, 0),
    );
    IDT.set_handler(8, IdtEntry::interrupt_gate(exc_double_fault as u64, 1, 0)); // IST1
    IDT.set_handler(10, IdtEntry::interrupt_gate(exc_invalid_tss as u64, 0, 0));
    IDT.set_handler(
        11,
        IdtEntry::interrupt_gate(exc_segment_not_present as u64, 0, 0),
    );
    IDT.set_handler(12, IdtEntry::interrupt_gate(exc_stack_segment as u64, 0, 0));
    IDT.set_handler(
        13,
        IdtEntry::interrupt_gate(exc_general_protection as u64, 0, 0),
    );
    IDT.set_handler(14, IdtEntry::interrupt_gate(exc_page_fault as u64, 0, 0));
    IDT.set_handler(16, IdtEntry::interrupt_gate(exc_x87_fp as u64, 0, 0));
    IDT.set_handler(
        17,
        IdtEntry::interrupt_gate(exc_alignment_check as u64, 0, 0),
    );
    IDT.set_handler(18, IdtEntry::interrupt_gate(exc_machine_check as u64, 1, 0)); // IST1
    IDT.set_handler(19, IdtEntry::interrupt_gate(exc_simd_fp as u64, 0, 0));
    IDT.set_handler(
        20,
        IdtEntry::interrupt_gate(exc_virtualization as u64, 0, 0),
    );
    IDT.set_handler(
        21,
        IdtEntry::interrupt_gate(exc_control_protection as u64, 0, 0),
    );

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
    crate::serial::log_ok("IDT", 741, "exception vectors installed");
}

/// Set a custom interrupt handler for a vector.
///
/// # Safety
/// Handler must be a valid interrupt handler.
pub unsafe fn set_interrupt_handler(vector: u8, handler: u64, ist: u8, dpl: u8) {
    IDT.set_handler(vector, IdtEntry::interrupt_gate(handler, ist, dpl));
}

/// Load the BSP's IDT on an AP core.
///
/// The IDT is a shared static — all cores use the same interrupt handlers.
/// Each AP just needs to `lidt` the same pointer the BSP already uses.
///
/// # Safety
/// Must be called after BSP's `init_idt()`.  CLI'd.
pub unsafe fn load_idt_for_ap() {
    let idt_ptr = IdtPtr {
        limit: (core::mem::size_of::<Idt>() - 1) as u16,
        base: &IDT as *const Idt as u64,
    };
    core::arch::asm!(
        "lidt [{}]",
        in(reg) &idt_ptr,
        options(nostack, preserves_flags)
    );
    // intentionally quiet on AP path to avoid N-core boot spam.
}

/// Enable interrupts
#[inline(always)]
pub fn enable_interrupts() {
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
    }
}

/// Disable interrupts
#[inline(always)]
pub fn disable_interrupts() {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }
}

/// Check if interrupts are enabled
pub fn interrupts_enabled() -> bool {
    let rflags: u64;
    unsafe {
        core::arch::asm!("pushfq; pop {}", out(reg) rflags);
    }
    (rflags & 0x200) != 0
}
