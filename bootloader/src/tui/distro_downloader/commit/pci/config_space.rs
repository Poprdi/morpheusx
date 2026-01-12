//! Low-level PCI configuration space access utilities.

/// PCI config space I/O ports
const PCI_CONFIG_ADDR: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Read 32-bit value from PCI config space.
pub fn pci_read32(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
    let addr: u32 = (1 << 31) // Enable bit
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);

    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") PCI_CONFIG_ADDR,
            in("eax") addr,
            options(nomem, nostack)
        );
        let value: u32;
        core::arch::asm!(
            "in eax, dx",
            in("dx") PCI_CONFIG_DATA,
            out("eax") value,
            options(nomem, nostack)
        );
        value
    }
}

/// Read 16-bit value from PCI config space.
pub fn pci_read16(bus: u8, device: u8, func: u8, offset: u8) -> u16 {
    let val32 = pci_read32(bus, device, func, offset & 0xFC);
    ((val32 >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

/// Read 8-bit value from PCI config space.
pub fn pci_read8(bus: u8, device: u8, func: u8, offset: u8) -> u8 {
    let val32 = pci_read32(bus, device, func, offset & 0xFC);
    ((val32 >> ((offset & 3) * 8)) & 0xFF) as u8
}

/// Read BAR address, handling 32-bit and 64-bit BARs.
pub fn read_bar(bus: u8, device: u8, func: u8, bar_index: u8) -> u64 {
    let bar_offset = 0x10 + bar_index * 4;
    let bar_val = pci_read32(bus, device, func, bar_offset);

    if bar_val & 1 == 0 {
        // Memory BAR
        let base = (bar_val & 0xFFFFFFF0) as u64;
        if (bar_val >> 1) & 3 == 2 {
            // 64-bit BAR - read upper 32 bits from next BAR
            let bar_hi = pci_read32(bus, device, func, bar_offset + 4);
            base | ((bar_hi as u64) << 32)
        } else {
            base
        }
    } else {
        // I/O BAR
        (bar_val & 0xFFFFFFFC) as u64
    }
}
