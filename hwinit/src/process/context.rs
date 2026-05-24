//! `CpuContext` is `repr(C)` to match offsets used by `context_switch.s`:
//! rax..r15 @ 0x00..0x78, rip/rflags/rsp/cs/ss @ 0x78..0xA0. Size = 0xA0.
//! `FpuState` is the 512 B FXSAVE area; saved separately by the timer ISR.

/// FXSAVE area (AMD64 Vol 1 §11.5.6). 16-byte alignment required.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct FpuState {
    pub data: [u8; 512],
}

impl FpuState {
    /// FCW=0x037F (mask all x87 exc, 64-bit prec), MXCSR=0x1F80 (mask all
    /// SSE exc, RN), XMMs zeroed.
    pub const fn new() -> Self {
        let mut data = [0u8; 512];
        data[0] = 0x7F;
        data[1] = 0x03;
        data[24] = 0x80;
        data[25] = 0x1F;
        Self { data }
    }
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct CpuContext {
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
    pub rip: u64,
    pub rflags: u64,
    pub rsp: u64,
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
            rflags: 0x202, // IF=1, bit 1 always set
            rsp: 0,
            cs: 0,
            ss: 0,
        }
    }

    pub fn new_kernel_thread(
        entry_fn: u64,
        kernel_stack_top: u64,
        kernel_cs: u64,
        kernel_ss: u64,
    ) -> Self {
        Self {
            rip: entry_fn,
            rsp: kernel_stack_top,
            rflags: 0x202,
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

// Keep in sync with context_switch.s.
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
