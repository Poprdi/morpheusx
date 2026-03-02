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
//! | 3     | 0x18     | User data (64)     |
//! | 4     | 0x20     | User code (64)     |
//! | 5     | 0x28     | TSS (16 bytes)     |

use crate::serial::puts;
use core::mem::size_of;

// SEGMENT SELECTORS

/// Kernel code segment selector
pub const KERNEL_CS: u16 = 0x08;
/// Kernel data segment selector
pub const KERNEL_DS: u16 = 0x10;
/// User data segment selector (ring 3) — at 0x18 for SYSRET compatibility
pub const USER_DS: u16 = 0x18 | 3; // RPL=3
/// User code segment selector (ring 3) — at 0x20 for SYSRET compatibility
pub const USER_CS: u16 = 0x20 | 3; // RPL=3
/// TSS segment selector
pub const TSS_SEL: u16 = 0x28;

// GDT ENTRY

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

// TASK STATE SEGMENT

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

// GDT TABLE

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

// GLOBAL STATE

// STATIC IST1 STACK — baked into .bss, never allocated from the heap.
//
// The double-fault handler points at this stack via IST[0].  Because it lives
// in the binary image, it is valid from the very first instruction and stays
// valid regardless of what the physical memory allocator does.  This is the
// ONLY thing that prevents a double fault from cascading into a triple fault
// (which causes an unconditional CPU/QEMU reset with no BSOD).
//
// If the double-fault handler itself were to fault (e.g., heap-allocated stack
// gets corrupted), the CPU switches to this stack for the double fault, the
// handler runs, and our BSOD is displayed instead of an invisible reset.
//
// Size: 32 KiB — generous enough for the BSOD rendering path.

const IST1_STATIC_STACK_SIZE: usize = 32 * 1024;

/// 16-byte aligned wrapper so RSP stays aligned after the CPU pushes the
/// exception frame (which is 40 or 48 bytes — both leave RSP 16-aligned).
#[repr(C, align(16))]
struct StaticStack([u8; IST1_STATIC_STACK_SIZE]);

/// The actual stack bytes.  Zeroed by the BSS loader; no runtime init needed.
static mut IST1_STATIC_STACK: StaticStack = StaticStack([0; IST1_STATIC_STACK_SIZE]);

/// Public accessor: returns a pointer to the TOP of the static IST1 stack
/// (stacks grow downward on x86-64).  Written into TSS.IST[0] once at boot.
pub fn ist1_static_stack_top() -> u64 {
    // SAFETY: read-only pointer arithmetic on a static array.
    unsafe { IST1_STATIC_STACK.0.as_ptr().add(IST1_STATIC_STACK_SIZE) as u64 }
}

/// Our GDT (static, page-aligned for safety)
static mut GDT: Gdt = Gdt {
    entries: [
        GdtEntry::null(),    // 0x00: Null
        GdtEntry::code64(0), // 0x08: Kernel code
        GdtEntry::data64(0), // 0x10: Kernel data
        GdtEntry::data64(3), // 0x18: User data  (SYSRET SS = STAR[63:48]+8)
        GdtEntry::code64(3), // 0x20: User code  (SYSRET CS = STAR[63:48]+16)
    ],
    tss_desc: TssDescriptor::empty(), // Placeholder, updated at init
};

/// Our TSS
static mut TSS: Tss = Tss::new();

/// GDT initialized flag
static mut GDT_INITIALIZED: bool = false;

// INITIALIZATION

/// Initialize GDT and TSS.
///
/// # Arguments
/// - `kernel_stack`: Stack pointer for ring 0 (RSP0 — used for interrupts
///   arriving from user mode when no IST is configured on that vector).
///
/// IST[0] (IST1) is always set from `IST1_STATIC_STACK` — it is NEVER
/// taken from the heap.  This guarantees the double-fault handler can always
/// run even when the allocator or heap is in an invalid state, preventing the
/// CPU from silently triple-faulting and resetting.
///
/// # Safety
/// - Must be called once
/// - `kernel_stack` must be a valid, mapped stack top
pub unsafe fn init_gdt(kernel_stack: u64) {
    if GDT_INITIALIZED {
        puts("[GDT] WARNING: already initialized\n");
        return;
    }

    // Set up TSS
    TSS.rsp0 = kernel_stack;
    // IST1 — static stack baked into BSS.  Always valid; see comment above.
    TSS.ist[0] = ist1_static_stack_top();

    // Update TSS descriptor in GDT
    let tss_addr = &TSS as *const Tss as u64;
    GDT.tss_desc = TssDescriptor::new(tss_addr, size_of::<Tss>() as u16);

    // Load GDT
    let gdt_ptr = GdtPtr {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: &GDT as *const Gdt as u64,
    };

    load_gdt(&gdt_ptr);

    // Reload segment registers
    reload_segments();

    // Load TSS
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
