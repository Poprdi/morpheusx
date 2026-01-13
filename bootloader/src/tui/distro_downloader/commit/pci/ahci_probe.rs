//! AHCI controller PCI probing for real hardware.
//!
//! Detects AHCI SATA controllers and retrieves the ABAR (AHCI Base Address Register).

extern crate alloc;

use super::config_space::{pci_read16, pci_read32, read_bar};
use crate::boot::network_boot::BlkProbeResult;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_LIGHTGREEN};

/// PCI class code for AHCI
const PCI_CLASS_STORAGE: u8 = 0x01;
const PCI_SUBCLASS_SATA: u8 = 0x06;
const PCI_PROGIF_AHCI: u8 = 0x01;

/// Probe for AHCI controller on PCI bus with debug output.
///
/// Scans PCI bus 0 for AHCI controllers (class 01:06:01).
pub fn probe_ahci_with_debug(screen: &mut Screen, log_y: &mut usize) -> BlkProbeResult {
    // Scan PCI bus 0 for AHCI controller
    for device in 0..32u8 {
        let id = pci_read32(0, device, 0, 0);

        if id == 0xFFFFFFFF || id == 0 {
            continue;
        }

        // Read class code (offset 0x08: rev_id, prog_if, subclass, class)
        let class_reg = pci_read32(0, device, 0, 0x08);
        let class_code = ((class_reg >> 24) & 0xFF) as u8;
        let subclass = ((class_reg >> 16) & 0xFF) as u8;
        let prog_if = ((class_reg >> 8) & 0xFF) as u8;

        // Check for AHCI (Mass Storage / SATA / AHCI mode)
        if class_code == PCI_CLASS_STORAGE
            && subclass == PCI_SUBCLASS_SATA
            && prog_if == PCI_PROGIF_AHCI
        {
            let vendor = (id & 0xFFFF) as u16;
            let dev_id = ((id >> 16) & 0xFFFF) as u16;

            screen.put_str_at(
                9,
                *log_y,
                &alloc::format!(
                    "PCI 0:{:02}:0 - AHCI Controller ({:04X}:{:04X})",
                    device,
                    vendor,
                    dev_id
                ),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            *log_y += 1;

            // Read ABAR from BAR5 (AHCI spec: BAR5 = ABAR)
            let abar = read_bar(0, device, 0, 5);

            if abar != 0 {
                screen.put_str_at(
                    11,
                    *log_y,
                    &alloc::format!("ABAR: {:#x}", abar),
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                *log_y += 1;

                return BlkProbeResult::ahci(abar, 0, device, 0);
            }
        }
    }

    BlkProbeResult::zeroed()
}
