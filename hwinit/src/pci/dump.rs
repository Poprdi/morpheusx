//! PCI USB host controller inventory dump.
//!
//! Phase-0 visibility for real-hardware bring-up. Walks the entire PCI bus
//! (all 256 buses × 32 devices × multi-function awareness) and prints every
//! controller it finds with class 0x0C / subclass 0x03 (USB) to the boot log,
//! including the prog_if byte so the operator can tell xHCI from EHCI/UHCI/OHCI.
//!
//! Output is rendered via `serial::puts`, which the bootloader mirrors to the
//! framebuffer once the live putc hook is installed — so this works on boards
//! with no serial console.

use super::config::{offset, pci_cfg_read16, pci_cfg_read32, pci_cfg_read8, PciAddr};
use crate::serial::puts;

const USB_CLASS: u8 = 0x0C;
const USB_SUBCLASS: u8 = 0x03;

const PROG_IF_UHCI: u8 = 0x00;
const PROG_IF_OHCI: u8 = 0x10;
const PROG_IF_EHCI: u8 = 0x20;
const PROG_IF_XHCI: u8 = 0x30;

fn type_label(prog_if: u8) -> &'static str {
    match prog_if {
        PROG_IF_UHCI => "UHCI (USB 1.1)",
        PROG_IF_OHCI => "OHCI (USB 1.1)",
        PROG_IF_EHCI => "EHCI (USB 2.0)",
        PROG_IF_XHCI => "xHCI (USB 2.0/3.x)",
        0x80 => "unspecified host",
        0xFE => "USB device (not host)",
        _ => "unknown prog_if",
    }
}

fn put_str(buf: &mut [u8], mut off: usize, s: &str) -> usize {
    for b in s.bytes() {
        if off < buf.len() {
            buf[off] = b;
            off += 1;
        }
    }
    off
}

fn put_hex_nybble(buf: &mut [u8], off: usize, nyb: u8) -> usize {
    if off >= buf.len() {
        return off;
    }
    buf[off] = if nyb < 10 { b'0' + nyb } else { b'a' + (nyb - 10) };
    off + 1
}

fn put_hex_u8(buf: &mut [u8], mut off: usize, val: u8) -> usize {
    off = put_hex_nybble(buf, off, (val >> 4) & 0x0F);
    put_hex_nybble(buf, off, val & 0x0F)
}

fn put_hex_u16(buf: &mut [u8], mut off: usize, val: u16) -> usize {
    off = put_hex_u8(buf, off, (val >> 8) as u8);
    put_hex_u8(buf, off, (val & 0xFF) as u8)
}

fn put_hex_u32(buf: &mut [u8], mut off: usize, val: u32) -> usize {
    off = put_hex_u16(buf, off, (val >> 16) as u16);
    put_hex_u16(buf, off, (val & 0xFFFF) as u16)
}

fn put_dec_u16(buf: &mut [u8], mut off: usize, mut val: u16) -> usize {
    if val == 0 {
        if off < buf.len() {
            buf[off] = b'0';
            return off + 1;
        }
        return off;
    }
    let mut tmp = [0u8; 5];
    let mut i = 0usize;
    while val > 0 {
        tmp[i] = b'0' + (val % 10) as u8;
        i += 1;
        val /= 10;
    }
    while i > 0 {
        i -= 1;
        if off < buf.len() {
            buf[off] = tmp[i];
            off += 1;
        }
    }
    off
}

fn flush(buf: &[u8], len: usize) {
    let cap = len.min(buf.len());
    if let Ok(s) = core::str::from_utf8(&buf[..cap]) {
        puts(s);
    }
}

fn dump_one(
    addr: PciAddr,
    vendor: u16,
    device: u16,
    class: u8,
    subclass: u8,
    prog_if: u8,
    bar0: u32,
) {
    let mut buf = [0u8; 128];
    let mut o = 0usize;
    o = put_str(&mut buf, o, "[USB-PCI] ");
    o = put_hex_u8(&mut buf, o, addr.bus);
    o = put_str(&mut buf, o, ":");
    o = put_hex_u8(&mut buf, o, addr.device);
    o = put_str(&mut buf, o, ".");
    o = put_hex_nybble(&mut buf, o, addr.function & 0x0F);
    o = put_str(&mut buf, o, "  ");
    o = put_hex_u16(&mut buf, o, vendor);
    o = put_str(&mut buf, o, ":");
    o = put_hex_u16(&mut buf, o, device);
    o = put_str(&mut buf, o, "  cls=");
    o = put_hex_u8(&mut buf, o, class);
    o = put_str(&mut buf, o, ".");
    o = put_hex_u8(&mut buf, o, subclass);
    o = put_str(&mut buf, o, ".");
    o = put_hex_u8(&mut buf, o, prog_if);
    o = put_str(&mut buf, o, "  BAR0=");
    o = put_hex_u32(&mut buf, o, bar0);
    o = put_str(&mut buf, o, "  ");
    o = put_str(&mut buf, o, type_label(prog_if));
    o = put_str(&mut buf, o, "\n");
    flush(&buf, o);
}

fn dump_summary(total: u16, xhci: u16, ehci: u16, uhci: u16, ohci: u16, other: u16) {
    let mut buf = [0u8; 128];
    let mut o = 0usize;
    o = put_str(&mut buf, o, "[USB-PCI] total=");
    o = put_dec_u16(&mut buf, o, total);
    o = put_str(&mut buf, o, "  xHCI=");
    o = put_dec_u16(&mut buf, o, xhci);
    o = put_str(&mut buf, o, "  EHCI=");
    o = put_dec_u16(&mut buf, o, ehci);
    o = put_str(&mut buf, o, "  UHCI=");
    o = put_dec_u16(&mut buf, o, uhci);
    o = put_str(&mut buf, o, "  OHCI=");
    o = put_dec_u16(&mut buf, o, ohci);
    o = put_str(&mut buf, o, "  other=");
    o = put_dec_u16(&mut buf, o, other);
    o = put_str(&mut buf, o, "\n");
    flush(&buf, o);
}

/// Walk the entire PCI bus and print every USB host controller found.
///
/// Designed for real-hardware bring-up where the runtime PCI scan in
/// `platform_init_selfcontained` only finds xHCI controllers and silently
/// ignores EHCI/UHCI/OHCI. This dump shows the ground truth.
///
/// # Safety
/// Reads PCI configuration space via I/O ports (0xCF8 / 0xCFC). Safe to call
/// once Phase 7 (PCI bus mastering enable) has completed; can also be called
/// earlier — config-space reads do not depend on bus-mastering state.
pub unsafe fn dump_usb_controllers() {
    puts("[USB-PCI] ==== USB host controller inventory ====\n");

    let mut total: u16 = 0;
    let mut count_uhci: u16 = 0;
    let mut count_ohci: u16 = 0;
    let mut count_ehci: u16 = 0;
    let mut count_xhci: u16 = 0;
    let mut count_other: u16 = 0;

    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let addr0 = PciAddr::new(bus, device, 0);
            let vendor0 = pci_cfg_read16(addr0, offset::VENDOR_ID);
            if vendor0 == 0xFFFF || vendor0 == 0x0000 {
                continue;
            }

            // Bit 7 of HEADER_TYPE indicates a multi-function device. Only walk
            // function 1..=7 if multi-function — otherwise reading them can
            // return aliased values on some chipsets.
            let header_type = pci_cfg_read8(addr0, offset::HEADER_TYPE);
            let max_fn: u8 = if (header_type & 0x80) != 0 { 8 } else { 1 };

            for function in 0..max_fn {
                let addr = PciAddr::new(bus, device, function);
                let vendor = pci_cfg_read16(addr, offset::VENDOR_ID);
                if vendor == 0xFFFF || vendor == 0x0000 {
                    continue;
                }

                let class = pci_cfg_read8(addr, 0x0B);
                let subclass = pci_cfg_read8(addr, 0x0A);
                if class != USB_CLASS || subclass != USB_SUBCLASS {
                    continue;
                }

                let prog_if = pci_cfg_read8(addr, 0x09);
                let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
                let bar0 = pci_cfg_read32(addr, offset::BAR0);

                dump_one(addr, vendor, device_id, class, subclass, prog_if, bar0);

                total = total.saturating_add(1);
                match prog_if {
                    PROG_IF_UHCI => count_uhci = count_uhci.saturating_add(1),
                    PROG_IF_OHCI => count_ohci = count_ohci.saturating_add(1),
                    PROG_IF_EHCI => count_ehci = count_ehci.saturating_add(1),
                    PROG_IF_XHCI => count_xhci = count_xhci.saturating_add(1),
                    _ => count_other = count_other.saturating_add(1),
                }
            }
        }
    }

    dump_summary(total, count_xhci, count_ehci, count_uhci, count_ohci, count_other);
    puts("[USB-PCI] ==== end inventory ====\n");
}
