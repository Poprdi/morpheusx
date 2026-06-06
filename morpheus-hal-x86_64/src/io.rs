//! Raw x86 port I/O. Exposed as plain fns (not a HAL trait method per LD25) so
//! the portable kernel installs them as a fn-pointer seam on x86 only; other
//! arches simply never install the hook and `SYS_PORT_IN/OUT` stay ENOSYS.

/// Read a byte/word/dword from `port`. `width` must be 1, 2, or 4; any other
/// value reads nothing and returns 0 (callers validate before this).
pub fn port_in(port: u16, width: u8) -> u32 {
    // SAFETY: `in` from an I/O port has no memory effect; width is bounded.
    unsafe {
        match width {
            1 => {
                let v: u8;
                core::arch::asm!("in al, dx", out("al") v, in("dx") port,
                    options(nomem, nostack, preserves_flags));
                v as u32
            },
            2 => {
                let v: u16;
                core::arch::asm!("in ax, dx", out("ax") v, in("dx") port,
                    options(nomem, nostack, preserves_flags));
                v as u32
            },
            4 => {
                let v: u32;
                core::arch::asm!("in eax, dx", out("eax") v, in("dx") port,
                    options(nomem, nostack, preserves_flags));
                v
            },
            _ => 0,
        }
    }
}

/// Write `value` (low `width` bytes) to `port`. `width` must be 1, 2, or 4.
pub fn port_out(port: u16, width: u8, value: u32) {
    // SAFETY: `out` to an I/O port has no memory effect; width is bounded.
    unsafe {
        match width {
            1 => core::arch::asm!("out dx, al", in("dx") port, in("al") value as u8,
                options(nomem, nostack, preserves_flags)),
            2 => core::arch::asm!("out dx, ax", in("dx") port, in("ax") value as u16,
                options(nomem, nostack, preserves_flags)),
            4 => core::arch::asm!("out dx, eax", in("dx") port, in("eax") value,
                options(nomem, nostack, preserves_flags)),
            _ => {},
        }
    }
}
