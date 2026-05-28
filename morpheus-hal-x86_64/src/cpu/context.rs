//! x86_64 concrete layout behind the opaque `CpuContext`/`FpuState`. Layout
//! MUST match `asm/cpu/context_switch.s`: rax..r15 @ 0x00..0x78, rip/rflags/rsp/cs/ss
//! @ 0x78..0xA0, total 0xA0.

use morpheus_hal_api::{CpuContext, FpuState};

use crate::cpu::gdt::{KERNEL_CS, KERNEL_DS, USER_CS, USER_DS};

/// Crate-private register file; offsets locked to `context_switch.s` via the
/// const assertions below.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub(crate) struct X86Context {
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

// Layout lock — build breaks if anything drifts.
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(X86Context, rax) == 0x00);
    assert!(offset_of!(X86Context, rbx) == 0x08);
    assert!(offset_of!(X86Context, rcx) == 0x10);
    assert!(offset_of!(X86Context, rdx) == 0x18);
    assert!(offset_of!(X86Context, rsi) == 0x20);
    assert!(offset_of!(X86Context, rdi) == 0x28);
    assert!(offset_of!(X86Context, rbp) == 0x30);
    assert!(offset_of!(X86Context, r8) == 0x38);
    assert!(offset_of!(X86Context, r9) == 0x40);
    assert!(offset_of!(X86Context, r10) == 0x48);
    assert!(offset_of!(X86Context, r11) == 0x50);
    assert!(offset_of!(X86Context, r12) == 0x58);
    assert!(offset_of!(X86Context, r13) == 0x60);
    assert!(offset_of!(X86Context, r14) == 0x68);
    assert!(offset_of!(X86Context, r15) == 0x70);
    assert!(offset_of!(X86Context, rip) == 0x78);
    assert!(offset_of!(X86Context, rflags) == 0x80);
    assert!(offset_of!(X86Context, rsp) == 0x88);
    assert!(offset_of!(X86Context, cs) == 0x90);
    assert!(offset_of!(X86Context, ss) == 0x98);
    assert!(core::mem::size_of::<X86Context>() == 0xA0);
    // Opaque blob must hold the concrete register file.
    assert!(core::mem::size_of::<X86Context>() <= core::mem::size_of::<CpuContext>());
    // 16-byte alignment for stack ABI + SSE.
    assert!(core::mem::align_of::<X86Context>() <= core::mem::align_of::<CpuContext>());
};

impl X86Context {
    #[inline]
    pub(crate) fn from_opaque(c: &CpuContext) -> &Self {
        // SAFETY: size + align asserted above.
        unsafe { &*(c as *const CpuContext as *const Self) }
    }

    #[inline]
    pub(crate) fn from_opaque_mut(c: &mut CpuContext) -> &mut Self {
        // SAFETY: size + align asserted above.
        unsafe { &mut *(c as *mut CpuContext as *mut Self) }
    }
}

// `impl Cpu for HalImpl` lives in `hal_impl.rs` and delegates to these primitives.

#[inline]
pub(crate) fn ctx_init_kernel(ctx: &mut CpuContext, entry: u64, stack_top: u64) {
    let x = X86Context::from_opaque_mut(ctx);
    *x = X86Context {
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
        rip: entry,
        // IF=1, reserved bit 1.
        rflags: 0x202,
        rsp: stack_top,
        cs: KERNEL_CS as u64,
        ss: KERNEL_DS as u64,
    };
}

#[inline]
pub(crate) fn ctx_init_user(
    ctx: &mut CpuContext,
    entry_va: u64,
    user_stack_top: u64,
    args: &[u64; 6],
) {
    let x = X86Context::from_opaque_mut(ctx);
    *x = X86Context {
        rax: 0,
        rbx: 0,
        rcx: args[3],
        rdx: args[2],
        rsi: args[1],
        rdi: args[0],
        rbp: 0,
        r8: args[4],
        r9: args[5],
        r10: 0,
        r11: 0,
        r12: 0,
        r13: 0,
        r14: 0,
        r15: 0,
        rip: entry_va,
        // IF=1, reserved bit 1.
        rflags: 0x202,
        rsp: user_stack_top,
        cs: USER_CS as u64,
        ss: USER_DS as u64,
    };
}

#[inline]
pub(crate) fn ctx_set_ip(ctx: &mut CpuContext, ip: u64) {
    X86Context::from_opaque_mut(ctx).rip = ip;
}

#[inline]
pub(crate) fn ctx_set_sp(ctx: &mut CpuContext, sp: u64) {
    X86Context::from_opaque_mut(ctx).rsp = sp;
}

#[inline]
pub(crate) fn ctx_set_arg(ctx: &mut CpuContext, n: u8, val: u64) {
    let x = X86Context::from_opaque_mut(ctx);
    match n {
        0 => x.rdi = val,
        1 => x.rsi = val,
        2 => x.rdx = val,
        3 => x.rcx = val,
        4 => x.r8 = val,
        5 => x.r9 = val,
        // Out-of-range: silent no-op per trait contract.
        _ => {},
    }
}

#[inline]
pub(crate) fn ctx_set_return(ctx: &mut CpuContext, val: u64) {
    X86Context::from_opaque_mut(ctx).rax = val;
}

#[inline]
pub(crate) fn ctx_get_return(ctx: &CpuContext) -> u64 {
    X86Context::from_opaque(ctx).rax
}

#[inline]
pub(crate) fn ctx_get_sp(ctx: &CpuContext) -> u64 {
    X86Context::from_opaque(ctx).rsp
}

#[inline]
pub(crate) fn ctx_is_user_mode(ctx: &CpuContext) -> bool {
    // CPL bits [1:0] of CS; CPL=3 ⇒ user.
    (X86Context::from_opaque(ctx).cs & 3) == 3
}

#[inline]
pub(crate) fn ctx_set_user_mode(ctx: &mut CpuContext, user: bool) {
    let x = X86Context::from_opaque_mut(ctx);
    if user {
        // Mirrors legacy `cur.context.ss |= 3`.
        x.ss |= 3;
    } else {
        x.ss &= !0b11;
    }
}

// 512-byte FXSAVE area; matches `fxsave [rbx]` in context_switch.s.

/// AMD64 Vol 1 §11.5.6 offsets.
mod fxsave_offsets {
    pub const FCW: usize = 0x00;
    pub const MXCSR: usize = 0x18;
}

#[inline]
pub(crate) fn fpu_init(fpu: &mut FpuState) {
    // SAFETY: `FpuState` is `repr(C, align(16))`, 512 B (FXSAVE area size).
    let bytes: &mut [u8; 512] = unsafe { &mut *(fpu as *mut FpuState as *mut [u8; 512]) };

    bytes.fill(0);

    // FCW=0x037F: mask all x87, 64-bit precision, round-to-nearest.
    bytes[fxsave_offsets::FCW] = 0x7F;
    bytes[fxsave_offsets::FCW + 1] = 0x03;

    // MXCSR=0x1F80: mask all SSE, round-to-nearest, FZ/DAZ off.
    bytes[fxsave_offsets::MXCSR] = 0x80;
    bytes[fxsave_offsets::MXCSR + 1] = 0x1F;
}
