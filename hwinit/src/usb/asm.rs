//! ASM externs — xHCI host controller primitives in asm/drivers/usb/init.s.

extern "win64" {
    /// Probe xHCI at BAR0. Returns: low byte = CAPLENGTH, high half = HCIVERSION.
    /// EAX = 0 if controller is dead (reads back all-F).
    pub fn asm_usb_host_probe(mmio_base: u64) -> u32;

    /// Soft restart (stop + start, NO HCRST). Preserves UEFI port state.
    /// Returns: 0 = ok, 1 = halt timeout, 2 = start timeout.
    pub fn asm_xhci_controller_soft_restart(op_base: u64, tsc_freq: u64) -> u32;

    /// BIOS/SMM handoff via USBLEGSUP extended capability.
    /// Returns: 0 = ok (or no legacy cap), 1 = timeout waiting BIOS release.
    pub fn asm_xhci_bios_handoff(mmio_base: u64, hccparams1: u64, tsc_freq: u64) -> u32;
}
