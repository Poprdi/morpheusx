//! PCI shim. Real scanner is `morpheus_hal_x86_64::pci`; this re-export
//! point keeps `morpheus_nic::device::pci::*` callers working.

/// Well-known PCIe ECAM base addresses.
pub mod ecam_bases {
    /// QEMU Q35 (also OVMF).
    pub const QEMU_Q35: usize = 0xB000_0000;
    pub const QEMU_I440FX: usize = 0xE000_0000;
    pub const INTEL_TYPICAL: usize = 0xE000_0000;
}

/// Approximate TSC-spin delay assuming ~2.5 GHz. Calibrate for precision.
#[cfg(target_arch = "x86_64")]
pub fn tsc_delay_us(us: u32) {
    const TSC_MHZ: u64 = 2500;
    let cycles = (us as u64) * TSC_MHZ;
    let start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    while morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(start) < cycles {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn tsc_delay_us(_us: u32) {}
