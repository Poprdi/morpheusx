//! Hardware RNG via RDRAND (Intel SDM Vol 1 §7.3.17). Ivy Bridge+; CF=1 on
//! success. Probed once through CPUID.01H:ECX[30] so a CPU without RDRAND
//! reports no entropy instead of taking #UD.

use core::sync::atomic::{AtomicU8, Ordering};

// 0 = unprobed, 1 = present, 2 = absent.
static RDRAND_STATE: AtomicU8 = AtomicU8::new(0);

fn rdrand_supported() -> bool {
    match RDRAND_STATE.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let ecx: u32;
            // SAFETY: CPUID is a pure ID instruction. rbx is preserved because
            // LLVM reserves it in some PIC modes (mirrors cpu::HalImpl::cpuid).
            unsafe {
                core::arch::asm!(
                    "push rbx",
                    "cpuid",
                    "pop rbx",
                    inout("eax") 1u32 => _,
                    out("ecx") ecx,
                    out("edx") _,
                    options(nostack),
                );
            }
            let present = ecx & (1 << 30) != 0; // CPUID.01H:ECX.RDRAND[bit 30]
            RDRAND_STATE.store(if present { 1 } else { 2 }, Ordering::Relaxed);
            present
        },
    }
}

/// One 64-bit RDRAND word, retried up to the Intel-recommended 10 times before
/// declaring the DRNG starved (SDM Vol 1 §7.3.17.1). `None` if RDRAND is absent
/// or starved.
#[inline]
pub fn hw_random() -> Option<u64> {
    if !rdrand_supported() {
        return None;
    }
    for _ in 0..10 {
        let val: u64;
        let ok: u8;
        // SAFETY: RDRAND has no memory operands; it writes the dst reg and CF.
        unsafe {
            core::arch::asm!(
                "rdrand {v}",
                "setc {c}",
                v = out(reg) val,
                c = out(reg_byte) ok,
                options(nomem, nostack),
            );
        }
        if ok != 0 {
            return Some(val);
        }
    }
    None
}
