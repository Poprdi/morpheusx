//! Long-mode GDT/TSS. Layout: 0x08 KCS, 0x10 KDS, 0x18 UDS, 0x20 UCS,
//! 0x28 TSS. UDS-before-UCS order is mandatory for SYSRET (STAR[63:48]+8/+16).

use core::mem::size_of;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
/// SYSRET requires UDS at STAR[63:48]+8.
pub const USER_DS: u16 = 0x18 | 3;
/// SYSRET requires UCS at STAR[63:48]+16.
pub const USER_CS: u16 = 0x20 | 3;
pub const TSS_SEL: u16 = 0x28;

/// 8-byte GDT entry (16-byte TSS descriptor handled separately).
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

    pub const fn code64(ring: u8) -> Self {
        let dpl = (ring & 3) << 5;
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            access: 0x80 | dpl | 0x18 | 0x02, // P|DPL|Code|Exec|Read
            granularity: 0x20 | 0x0F,         // L (long mode) | G
            base_high: 0,
        }
    }

    pub const fn data64(ring: u8) -> Self {
        let dpl = (ring & 3) << 5;
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            access: 0x80 | dpl | 0x10 | 0x02, // P|DPL|Data|Writable
            granularity: 0x0F,
            base_high: 0,
        }
    }
}

/// 16-byte long-mode TSS descriptor.
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
    pub const fn new(tss_addr: u64, tss_size: u16) -> Self {
        Self {
            limit_low: if tss_size > 0 { tss_size - 1 } else { 0 },
            base_low: tss_addr as u16,
            base_mid: (tss_addr >> 16) as u8,
            access: 0x80 | 0x09, // P | type=Available 64-bit TSS
            granularity: 0,
            base_high: (tss_addr >> 24) as u8,
            base_upper: (tss_addr >> 32) as u32,
            reserved: 0,
        }
    }

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

/// Long-mode TSS.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct Tss {
    reserved0: u32,
    /// Ring 0 stack used on user->kernel interrupt entry (no IST).
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    reserved1: u64,
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
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

const GDT_ENTRY_COUNT: usize = 5;

#[repr(C, align(16))]
pub struct Gdt {
    entries: [GdtEntry; GDT_ENTRY_COUNT],
    tss_desc: TssDescriptor,
}

#[repr(C, packed)]
pub struct GdtPtr {
    pub limit: u16,
    pub base: u64,
}

// Per-core IST1 stacks live in .bss so the #DF handler stays runnable even
// if the heap is wrecked. Without this any double fault triple-faults =>
// silent CPU/QEMU reset, no BSOD.
const IST1_STATIC_STACK_SIZE: usize = 32 * 1024;

/// 16-byte aligned so RSP stays aligned after the 40/48-byte CPU exception push.
#[repr(C, align(16))]
struct StaticStack([u8; IST1_STATIC_STACK_SIZE]);

/// Per-core to prevent cross-CPU #DF frame corruption. BSS-zeroed.
static mut IST1_STACKS: [StaticStack; MAX_CPUS] =
    [const { StaticStack([0; IST1_STATIC_STACK_SIZE]) }; MAX_CPUS];

/// Top of per-core IST1 stack (x86_64 stacks grow down).
pub fn ist1_stack_top_for_core(core_idx: usize) -> u64 {
    let idx = if core_idx < MAX_CPUS { core_idx } else { 0 };
    // SAFETY: read-only pointer arithmetic on a static array.
    unsafe { IST1_STACKS[idx].0.as_ptr().add(IST1_STATIC_STACK_SIZE) as u64 }
}

pub fn ist1_static_stack_top() -> u64 {
    ist1_stack_top_for_core(0)
}

static mut GDT: Gdt = Gdt {
    entries: [
        GdtEntry::null(),
        GdtEntry::code64(0),
        GdtEntry::data64(0),
        GdtEntry::data64(3),
        GdtEntry::code64(3),
    ],
    tss_desc: TssDescriptor::empty(),
};

static mut TSS: Tss = Tss::new();
static mut GDT_INITIALIZED: bool = false;

/// Load GDT/TSS. IST[0] uses the BSS-resident stack so #DF can never
/// triple-fault from a bad heap.
///
/// # Safety
/// Once per CPU. `kernel_stack` must be a mapped stack top.
pub unsafe fn init_gdt(kernel_stack: u64) {
    if GDT_INITIALIZED {
        crate::serial::log_warn("GDT", 710, "already initialized");
        return;
    }

    TSS.rsp0 = kernel_stack;
    TSS.ist[0] = ist1_static_stack_top();

    let tss_addr = &TSS as *const Tss as u64;
    GDT.tss_desc = TssDescriptor::new(tss_addr, size_of::<Tss>() as u16);

    let gdt_ptr = GdtPtr {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: &GDT as *const Gdt as u64,
    };

    load_gdt(&gdt_ptr);
    reload_segments();
    load_tss(TSS_SEL);

    GDT_INITIALIZED = true;
    crate::serial::log_ok("GDT", 711, "gdt+tss initialized");
}

#[inline(always)]
unsafe fn load_gdt(ptr: &GdtPtr) {
    core::arch::asm!(
        "lgdt [{0}]",
        in(reg) ptr,
        options(nostack, preserves_flags)
    );
}

/// Reload CS via retfq, then data segments.
#[inline(never)]
unsafe fn reload_segments() {
    let code_sel: u64 = KERNEL_CS as u64;
    let data_sel: u64 = KERNEL_DS as u64;

    core::arch::asm!(
        "push {code}",
        "lea {tmp}, [rip + 2f]",
        "push {tmp}",
        "retfq",
        "2:",
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

#[inline(always)]
unsafe fn load_tss(selector: u16) {
    core::arch::asm!(
        "ltr {0:x}",
        in(reg) selector,
        options(nostack, preserves_flags)
    );
}

/// Update TSS.rsp0 for context switch.
///
/// # Safety
/// `stack` must be a valid mapped stack top.
pub unsafe fn set_kernel_stack(stack: u64) {
    TSS.rsp0 = stack;
}

pub fn get_kernel_stack() -> u64 {
    unsafe { TSS.rsp0 }
}

// Per-AP GDT+TSS: same layout, distinct RSP0/IST. Static array, no heap.

use super::per_cpu::MAX_CPUS;

static mut AP_TSS: [Tss; MAX_CPUS] = [const { Tss::new() }; MAX_CPUS];
static mut AP_GDT: [Gdt; MAX_CPUS] = [const {
    Gdt {
        entries: [
            GdtEntry::null(),
            GdtEntry::code64(0),
            GdtEntry::data64(0),
            GdtEntry::data64(3),
            GdtEntry::code64(3),
        ],
        tss_desc: TssDescriptor::empty(),
    }
}; MAX_CPUS];

/// Load per-AP GDT/TSS.
///
/// # Safety
/// Once per AP, on its own stack, IF=0.
pub unsafe fn init_gdt_for_ap(stack_top: u64, core_idx: u32) {
    let idx = core_idx as usize;
    assert!(idx < MAX_CPUS);

    AP_TSS[idx].rsp0 = stack_top;
    AP_TSS[idx].ist[0] = ist1_stack_top_for_core(idx);

    let tss_addr = &AP_TSS[idx] as *const Tss as u64;
    AP_GDT[idx].tss_desc = TssDescriptor::new(tss_addr, size_of::<Tss>() as u16);

    let gdt_ptr = GdtPtr {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: &AP_GDT[idx] as *const Gdt as u64,
    };

    load_gdt(&gdt_ptr);
    reload_segments();
    load_tss(TSS_SEL);

    let _ = (core_idx, stack_top);
}

/// Per-core RSP0 update for context switch.
///
/// # Safety
/// `core_idx` < MAX_CPUS.
pub unsafe fn set_kernel_stack_for_core(core_idx: u32, stack: u64) {
    if core_idx == 0 {
        TSS.rsp0 = stack;
    } else {
        let idx = core_idx as usize;
        if idx < MAX_CPUS {
            AP_TSS[idx].rsp0 = stack;
        }
    }
}
