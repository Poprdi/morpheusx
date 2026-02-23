//! CPU Context — saved register state for a process.
//!
//! `CpuContext` is laid out in `#[repr(C)]` so that the assembly context-switch
//! stub (`context_switch.s`) can push/pop registers at known, fixed offsets.
//!
//! ## Field order (matches ASM stub offsets)
//!
//! ```text
//! Offset  Register
//!  0x00   rax
//!  0x08   rbx
//!  0x10   rcx
//!  0x18   rdx
//!  0x20   rsi
//!  0x28   rdi
//!  0x30   rbp
//!  0x38   r8
//!  0x40   r9
//!  0x48   r10
//!  0x50   r11
//!  0x58   r12
//!  0x60   r13
//!  0x68   r14
//!  0x70   r15
//!  0x78   rip
//!  0x80   rflags
//!  0x88   rsp
//!  0x90   cs
//!  0x98   ss
//! ```
//!
//! Total size: 0xA0 = 160 bytes.

/// Full CPU register state saved at a context switch or interrupt boundary.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct CpuContext {
    // General-purpose registers
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
    // Control flow
    pub rip: u64,
    pub rflags: u64,
    pub rsp: u64,
    // Segment selectors (stored as u64 to keep alignment simple)
    pub cs: u64,
    pub ss: u64,
}

impl CpuContext {
    pub const fn empty() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: 0x202, // IF=1, reserved bit 1 always set
            rsp: 0,
            cs: 0,
            ss: 0,
        }
    }

    /// Construct a context that will begin execution at `entry_fn` on the
    /// given kernel stack, with the kernel code/data selectors.
    ///
    /// `kernel_cs` / `kernel_ss` come from your GDT (e.g. `KERNEL_CS` / `KERNEL_DS`
    /// from `hwinit::cpu::gdt`).
    pub fn new_kernel_thread(
        entry_fn: u64,
        kernel_stack_top: u64,
        kernel_cs: u64,
        kernel_ss: u64,
    ) -> Self {
        Self {
            rip: entry_fn,
            rsp: kernel_stack_top,
            rflags: 0x202, // IF=1 (interrupts enabled), reserved bit
            cs: kernel_cs,
            ss: kernel_ss,
            ..Self::empty()
        }
    }
}

impl core::fmt::Debug for CpuContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "CpuContext {{ rip={:#x} rsp={:#x} rflags={:#x} rax={:#x} rbx={:#x} rcx={:#x} }}",
            self.rip, self.rsp, self.rflags, self.rax, self.rbx, self.rcx
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// COMPILE-TIME OFFSET ASSERTIONS (keep in sync with context_switch.s)
// ═══════════════════════════════════════════════════════════════════════════

const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(CpuContext, rax) == 0x00);
    assert!(offset_of!(CpuContext, rbx) == 0x08);
    assert!(offset_of!(CpuContext, rcx) == 0x10);
    assert!(offset_of!(CpuContext, rdx) == 0x18);
    assert!(offset_of!(CpuContext, rsi) == 0x20);
    assert!(offset_of!(CpuContext, rdi) == 0x28);
    assert!(offset_of!(CpuContext, rbp) == 0x30);
    assert!(offset_of!(CpuContext, r8) == 0x38);
    assert!(offset_of!(CpuContext, r9) == 0x40);
    assert!(offset_of!(CpuContext, r10) == 0x48);
    assert!(offset_of!(CpuContext, r11) == 0x50);
    assert!(offset_of!(CpuContext, r12) == 0x58);
    assert!(offset_of!(CpuContext, r13) == 0x60);
    assert!(offset_of!(CpuContext, r14) == 0x68);
    assert!(offset_of!(CpuContext, r15) == 0x70);
    assert!(offset_of!(CpuContext, rip) == 0x78);
    assert!(offset_of!(CpuContext, rflags) == 0x80);
    assert!(offset_of!(CpuContext, rsp) == 0x88);
    assert!(offset_of!(CpuContext, cs) == 0x90);
    assert!(offset_of!(CpuContext, ss) == 0x98);
    assert!(core::mem::size_of::<CpuContext>() == 0xA0);
};
