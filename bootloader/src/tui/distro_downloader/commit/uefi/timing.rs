//! TSC (Time Stamp Counter) calibration using UEFI services.

/// Calibrate TSC frequency using UEFI Stall service.
///
/// Must be called BEFORE ExitBootServices.
pub fn calibrate_tsc_with_stall(bs: &crate::BootServices) -> u64 {
    let start_tsc = read_tsc();

    // UEFI Stall takes microseconds - stall for 10ms (10,000 us)
    let _ = (bs.stall)(10_000);

    let end_tsc = read_tsc();

    // Calculate ticks for 10ms
    let ticks_10ms = end_tsc.saturating_sub(start_tsc);

    // Extrapolate to 1 second (multiply by 100)
    let tsc_freq = ticks_10ms.saturating_mul(100);

    // Sanity check: expect 1-10 GHz range
    if tsc_freq < 1_000_000_000 || tsc_freq > 10_000_000_000 {
        // Fallback to 2.5 GHz if result seems wrong
        2_500_000_000
    } else {
        tsc_freq
    }
}

/// Read TSC (Time Stamp Counter).
fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack)
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}
