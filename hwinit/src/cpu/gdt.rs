//! Global Descriptor Table (GDT) Management
//!
//! Sets up the GDT for long mode operation. UEFI leaves us in long mode
//! with a valid GDT, but we take ownership and set our own.
//!
//! # Segment Layout
//!
//! | Index | Selector | Description        |
//! |-------|----------|--------------------|
//! | 0     | 0x00     | Null descriptor    |
//! | 1     | 0x08     | Kernel code (64)   |
//! | 2     | 0x10     | Kernel data (64)   |
//! | 3     | 0x18     | User code (64)     |
//! | 4     | 0x20     | User data (64)     |
//! | 5     | 0x28     | TSS (16 bytes)     |

use core::mem::size_of;
use crate::serial::puts;

// ═══════════════════════════════════════════════════════════════════════════
// SEGMENT SELECTORS
// ═══════════════════════════════════════════════════════════════════════════

/// Kernel code segment selector
pub const KERNEL_CS: u16 = 0x08;
/// Kernel data segment selector
pub const KERNEL_DS: u16 = 0x10;
/// User code segment selector (ring 3)
pub const USER_CS: u16 = 0x18 | 3; // RPL=3
/// User data segment selector (ring 3)
pub const USER_DS: u16 = 0x20 | 3; // RPL=3
/// TSS segment selector
pub const TSS_SEL: u16 = 0x28;

// ═══════════════════════════════════════════════════════════════════════════
// GDT ENTRY
// ═══════════════════════════════════════════════════════════════════════════

/// GDT entry (8 bytes for normal, 16 bytes for TSS in long mode)
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl GdtEntry {
    /// Null descriptor
    pub const fn null() -> Self {
        Self {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
        }
    }

    /// 64-bit code segment
    pub const fn code64(ring: u8) -> Self {
        let dpl = (ring & 3) << 5;
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            // Present | DPL | Code | Executable | Readable
            access: 0x80 | dpl | 0x18 | 0x02,
            // Long mode | Granularity
            granularity: 0x20 | 0x0F,
            base_high: 0,
        }
    }

    /// 64-bit data segment
    pub const fn data64(ring: u8) -> Self {
        let dpl = (ring & 3) << 5;
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            // Present | DPL | Data | Writable
            access: 0x80 | dpl | 0x10 | 0x02,
            // Granularity
            granularity: 0x0F,
            base_high: 0,
        }
    }
}

/// TSS descriptor (16 bytes in long mode)
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct TssDescriptor {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
    base_upper: u32,
    reserved: u32,
}

impl TssDescriptor {
    /// Create TSS descriptor from TSS address
    pub const fn new(tss_addr: u64, tss_size: u16) -> Self {
        Self {
            limit_low: if tss_size > 0 { tss_size - 1 } else { 0 },
            base_low: tss_addr as u16,
            base_mid: (tss_addr >> 16) as u8,
            // Present | TSS Available (0x9)
            access: 0x80 | 0x09,
            granularity: 0,
            base_high: (tss_addr >> 24) as u8,
            base_upper: (tss_addr >> 32) as u32,
            reserved: 0,
        }
    }

    /// Create an empty/invalid descriptor for static initialization
    pub const fn empty() -> Self {
        Self {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
            base_upper: 0,
            reserved: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TASK STATE SEGMENT
// ═══════════════════════════════════════════════════════════════════════════

/// Task State Segment for long mode
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct Tss {
    reserved0: u32,
    /// RSP for ring 0 (kernel stack for interrupts from user mode)
    pub rsp0: u64,
    /// RSP for ring 1 (unused)
    pub rsp1: u64,
    /// RSP for ring 2 (unused)
    pub rsp2: u64,
    reserved1: u64,
    /// Interrupt stack table pointers
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    /// I/O permission bitmap offset
    pub iopb_offset: u16,
}

impl Tss {
    pub const fn new() -> Self {
        Self {
            reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iopb_offset: size_of::<Self>() as u16,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GDT TABLE
// ═══════════════════════════════════════════════════════════════════════════

/// Number of normal GDT entries (before TSS)
const GDT_ENTRY_COUNT: usize = 5;

/// Full GDT with TSS
#[repr(C, align(16))]
pub struct Gdt {
    entries: [GdtEntry; GDT_ENTRY_COUNT],
    tss_desc: TssDescriptor,
}

/// GDT pointer for lgdt instruction
#[repr(C, packed)]
pub struct GdtPtr {
    pub limit: u16,
    pub base: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Our GDT (static, page-aligned for safety)
static mut GDT: Gdt = Gdt {
    entries: [
        GdtEntry::null(),       // 0x00: Null
        GdtEntry::code64(0),    // 0x08: Kernel code
        GdtEntry::data64(0),    // 0x10: Kernel data
        GdtEntry::code64(3),    // 0x18: User code
        GdtEntry::data64(3),    // 0x20: User data
    ],
    tss_desc: TssDescriptor::empty(), // Placeholder, updated at init
};

/// Our TSS
static mut TSS: Tss = Tss::new();

/// GDT initialized flag
static mut GDT_INITIALIZED: bool = false;

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize GDT and TSS.
///
/// # Arguments
/// - `kernel_stack`: Stack pointer for ring 0 (used for interrupts from user mode)
/// - `ist1_stack`: Interrupt stack 1 (for NMI, double fault, etc.)
///
/// # Safety
/// - Must be called once
/// - Stacks must be valid and large enough
pub unsafe fn init_gdt(kernel_stack: u64, ist1_stack: u64) {
    if GDT_INITIALIZED {
        puts("[GDT] WARNING: already initialized\n");
        return;
    }

    puts("[GDT] setting up TSS...\n");

    // Set up TSS
    TSS.rsp0 = kernel_stack;
    TSS.ist[0] = ist1_stack; // IST1 for critical exceptions

    // Update TSS descriptor in GDT
    let tss_addr = &TSS as *const Tss as u64;
    GDT.tss_desc = TssDescriptor::new(tss_addr, size_of::<Tss>() as u16);

    puts("[GDT] loading GDT...\n");

    // Load GDT
    let gdt_ptr = GdtPtr {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: &GDT as *const Gdt as u64,
    };

    // Print GDT info for debug
    crate::serial::puts("[GDT] limit=0x");
    crate::serial::put_hex32(gdt_ptr.limit as u32);
    crate::serial::puts(" base=0x");
    crate::serial::put_hex64(gdt_ptr.base);
    crate::serial::puts("\n");

    load_gdt(&gdt_ptr);
    puts("[GDT] lgdt done\n");

    // Reload segment registers
    puts("[GDT] reloading segments (CS=0x08, DS=0x10)...\n");
    reload_segments();
    puts("[GDT] segments reloaded\n");

    // Load TSS
    puts("[GDT] loading TSS...\n");
    load_tss(TSS_SEL);

    GDT_INITIALIZED = true;
    puts("[GDT] initialized\n");
}

/// Load GDT via lgdt instruction
#[inline(always)]
unsafe fn load_gdt(ptr: &GdtPtr) {
    core::arch::asm!(
        "lgdt [{0}]",
        in(reg) ptr,
        options(nostack, preserves_flags)
    );
}

/// Reload segment registers after GDT change
#[inline(never)]
unsafe fn reload_segments() {
    // Far return to reload CS, then set data segments
    // retfq pops: RIP from [RSP], then CS from [RSP+8]
    // So we need: push CS first, then push RIP
    let code_sel: u64 = KERNEL_CS as u64;
    let data_sel: u64 = KERNEL_DS as u64;
    
    core::arch::asm!(
        // Push CS (will be at RSP+8 after next push)
        "push {code}",
        // Push return address (will be at RSP)
        "lea {tmp}, [rip + 2f]",
        "push {tmp}",
        // Far return: pops RIP from RSP, CS from RSP+8
        "retfq",
        "2:",
        // Now reload data segments
        "mov ds, {data:x}",
        "mov es, {data:x}",
        "mov fs, {data:x}",
        "mov gs, {data:x}",
        "mov ss, {data:x}",
        code = in(reg) code_sel,
        data = in(reg) data_sel,
        tmp = lateout(reg) _,
        options(preserves_flags)
    );
}

/// Load TSS via ltr instruction
#[inline(always)]
unsafe fn load_tss(selector: u16) {
    core::arch::asm!(
        "ltr {0:x}",
        in(reg) selector,
        options(nostack, preserves_flags)
    );
}

/// Update kernel stack in TSS (for context switches)
///
/// # Safety
/// Stack must be valid.
pub unsafe fn set_kernel_stack(stack: u64) {
    TSS.rsp0 = stack;
}

/// Get current kernel stack from TSS
pub fn get_kernel_stack() -> u64 {
    unsafe { TSS.rsp0 }
}
