//! HID class USB requests + interface descriptor parser.
//!
//! These live in morpheus-xhci because `enumerate.rs` consumes them while
//! walking the bus. The HID *report* parsers (keyboard scancode translation,
//! mouse delta math) stay in `hwinit/src/usb/hid/` since they're true
//! class-semantic code, not USB-control mechanics.

use crate::controller::{XhciController, XhciError};
use crate::dma;
use crate::pack_setup;

// USB HID class/subclass/protocol constants (HID spec 1.11 §4.2)
pub const USB_CLASS_HID: u8 = 0x03;
pub const USB_SUBCLASS_BOOT: u8 = 0x01;
pub const USB_PROTOCOL_KEYBOARD: u8 = 0x01;
pub const USB_PROTOCOL_MOUSE: u8 = 0x02;

/// HID interface descriptor extracted from config.
#[derive(Debug, Clone, Copy)]
pub struct HIDInterface {
    pub interface_num: u8,
    pub protocol: u8,
    pub ep_in: u8,
    pub ep_out: u8,
    pub max_packet_in: u16,
    /// wDescriptorLength of the report descriptor (HID descriptor offset 7-8);
    /// 0 if no HID descriptor was found in the interface.
    pub report_desc_len: u16,
}

/// One Data,Variable field located in a HID report: where it sits and how wide.
/// `bit_size == 0` means the field is absent.
#[derive(Debug, Clone, Copy, Default)]
pub struct HidField {
    pub bit_offset: u16,
    pub bit_size: u8,
    pub signed: bool,
}

/// Decoded bit layout of a mouse's motion report. Either the spec-fixed boot
/// layout or the result of parsing the device's report descriptor — both feed
/// the same decoder, so any mouse is handled uniformly.
#[derive(Debug, Clone, Copy)]
pub struct MouseLayout {
    /// 0 = reports have no report-ID prefix byte.
    pub report_id: u8,
    pub buttons: HidField,
    pub x: HidField,
    pub y: HidField,
    pub wheel: HidField,
}

/// HID 1.11 §B.2 boot-protocol mouse report: byte0 buttons, byte1 X(i8),
/// byte2 Y(i8), byte3 wheel(i8, de-facto extension).
pub const BOOT_MOUSE_LAYOUT: MouseLayout = MouseLayout {
    report_id: 0,
    buttons: HidField {
        bit_offset: 0,
        bit_size: 8,
        signed: false,
    },
    x: HidField {
        bit_offset: 8,
        bit_size: 8,
        signed: true,
    },
    y: HidField {
        bit_offset: 16,
        bit_size: 8,
        signed: true,
    },
    wheel: HidField {
        bit_offset: 24,
        bit_size: 8,
        signed: true,
    },
};

impl XhciController {
    /// Fetch the HID report descriptor into the DMA buffer.
    ///
    /// # Safety
    /// The controller must be initialized with valid MMIO and DMA mappings and
    /// the caller must hold exclusive access; `self.slot_id` must be addressed.
    pub unsafe fn get_hid_report_descriptor(
        &mut self,
        interface_num: u8,
        len: u16,
    ) -> Result<*const u8, XhciError> {
        let desc_buf = self.dma_base + dma::OFF_DESC as u64;
        // bmRequestType=0x81 (D2H, Standard, Interface), GET_DESCRIPTOR,
        // wValue=0x2200 (Report descriptor), wIndex=interface.
        let param = pack_setup(
            0x81,
            0x06,
            0x2200 | interface_num as u16,
            interface_num as u16,
            len,
        );
        self.control_in(param, desc_buf, len)?;
        Ok(desc_buf as *const u8)
    }

    /// Set the HID idle duration (all endpoints, duration=0 = infinite).
    ///
    /// # Safety
    /// The controller must be initialized with valid MMIO and DMA mappings and
    /// the caller must hold exclusive access; `self.slot_id` must be addressed.
    pub unsafe fn set_hid_idle(&mut self, interface_num: u8) -> Result<(), XhciError> {
        let param = pack_setup(0x21, 0x0A, 0, interface_num as u16, 0);
        self.control_nodata(param)
    }

    /// SET_PROTOCOL(0 = boot) on a HID interface.
    ///
    /// The HID 1.11 spec says boot-subclass devices default to boot protocol
    /// at power-on, but real keyboards routinely come up in report protocol
    /// anyway (firmware bugs, "modern" defaults, etc.). Linux's usbkbd driver
    /// always sends this; we do too. Failures are non-fatal — for a properly
    /// behaved boot-subclass device this is just a no-op.
    ///
    /// # Safety
    /// The controller must be initialized with valid MMIO and DMA mappings and
    /// the caller must hold exclusive access; `self.slot_id` must be addressed.
    pub unsafe fn set_hid_protocol_boot(&mut self, interface_num: u8) -> Result<(), XhciError> {
        self.set_hid_protocol(interface_num, 0)
    }

    /// SET_PROTOCOL on a HID interface. `protocol` 0 = boot, 1 = report.
    ///
    /// # Safety
    /// The controller must be initialized with valid MMIO and DMA mappings and
    /// the caller must hold exclusive access; `self.slot_id` must be addressed.
    pub unsafe fn set_hid_protocol(
        &mut self,
        interface_num: u8,
        protocol: u8,
    ) -> Result<(), XhciError> {
        // bmRequestType=0x21 (H2D, Class, Interface), bRequest=0x0B (SET_PROTOCOL),
        // wValue=protocol, wIndex=interface_num.
        let param = pack_setup(0x21, 0x0B, protocol as u16, interface_num as u16, 0);
        self.control_nodata(param)
    }

    // GET_PROTOCOL (returns 0 = boot, 1 = report) is parked for now. I had tried
    // using it to decide boot-vs-report decoding, but real hardware lies (ofcourse) they
    // answer "boot" via GET_PROTOCOL yet emit report-format data so the mouse
    // path for now unconditionally forces report protocol and decodes from the report
    // descriptor. I left in in commented out because we might need it in future
    // versions of the driver.
    //
    // pub unsafe fn get_hid_protocol(&mut self, interface_num: u8) -> Result<u8, XhciError> {
    //     // bmRequestType=0xA1 (D2H, Class, Interface), bRequest=0x03 (GET_PROTOCOL).
    //     let buf = self.dma_base + dma::OFF_DESC as u64;
    //     let param = pack_setup(0xA1, 0x03, 0, interface_num as u16, 1);
    //     self.control_in(param, buf, 1)?;
    //     Ok(core::ptr::read_volatile(buf as *const u8))
    // }

    /// Find the first HID boot interface from the already-fetched config descriptor.
    /// Returns a HIDInterface if a boot-class keyboard or mouse is found.
    ///
    /// Many keyboards expose two HID interfaces: interface 0 = boot-protocol
    /// keyboard, interface 1 = vendor-defined extras (media keys, fn-key
    /// extensions). We must commit to the first match and not let a later
    /// non-boot HID interface clobber our finding — the previous version
    /// kept `in_hid_boot` as a per-iface flag and tested it in the final
    /// return, which dropped the captured iface-0 match the moment a
    /// non-boot iface-1 followed.
    ///
    /// # Safety
    /// `desc_ptr` must point to a readable configuration descriptor buffer whose
    /// declared total length stays within the mapped DMA region.
    pub unsafe fn find_hid_interface(&self, desc_ptr: *const u8) -> Option<HIDInterface> {
        let d = desc_ptr;
        let total = u16::from_le_bytes([
            core::ptr::read_volatile(d.add(2)),
            core::ptr::read_volatile(d.add(3)),
        ]) as usize;

        let limit = total.min(512);
        let mut off = 0usize;

        let mut iface_num: u8 = 0;
        let mut proto: u8 = 0;
        let mut ep_in: u8 = 0;
        let mut ep_out: u8 = 0;
        let mut mp_in: u16 = 64;
        let mut report_desc_len: u16 = 0;
        let mut in_hid_boot = false;

        while off + 2 <= limit {
            let blen = core::ptr::read_volatile(d.add(off)) as usize;
            let btype = core::ptr::read_volatile(d.add(off + 1));
            if blen < 2 || off + blen > limit {
                break;
            }

            if btype == 4 && blen >= 9 {
                // New interface boundary. If we already captured an IN
                // endpoint for an earlier boot HID iface, lock that match
                // in — any subsequent non-boot iface would otherwise flip
                // `in_hid_boot` to false and silently kill our result.
                if ep_in != 0 {
                    break;
                }

                let cls = core::ptr::read_volatile(d.add(off + 5));
                let subcls = core::ptr::read_volatile(d.add(off + 6));
                let p = core::ptr::read_volatile(d.add(off + 7));
                in_hid_boot = cls == USB_CLASS_HID
                    && subcls == USB_SUBCLASS_BOOT
                    && (p == USB_PROTOCOL_KEYBOARD || p == USB_PROTOCOL_MOUSE);
                if in_hid_boot {
                    iface_num = core::ptr::read_volatile(d.add(off + 2));
                    proto = p;
                    ep_in = 0;
                    ep_out = 0;
                }
            }

            // HID descriptor (0x21): wDescriptorLength of the report descriptor
            // is at offset 7-8 (first class descriptor, always the report one).
            if btype == 0x21 && blen >= 9 && in_hid_boot {
                report_desc_len = u16::from_le_bytes([
                    core::ptr::read_volatile(d.add(off + 7)),
                    core::ptr::read_volatile(d.add(off + 8)),
                ]);
            }

            if btype == 5 && blen >= 7 && in_hid_boot {
                // Endpoint descriptor
                let addr = core::ptr::read_volatile(d.add(off + 2));
                let attr = core::ptr::read_volatile(d.add(off + 3));
                let mpkt = u16::from_le_bytes([
                    core::ptr::read_volatile(d.add(off + 4)),
                    core::ptr::read_volatile(d.add(off + 5)),
                ]) & 0x7FF;
                // Only care about interrupt endpoints
                if attr & 0x03 == 0x03 {
                    if addr & 0x80 != 0 {
                        ep_in = addr & 0x7F;
                        mp_in = mpkt;
                    } else {
                        ep_out = addr & 0x7F;
                    }
                }
            }

            off += blen;
        }

        if ep_in != 0 {
            Some(HIDInterface {
                interface_num: iface_num,
                protocol: proto,
                ep_in,
                ep_out,
                max_packet_in: mp_in,
                report_desc_len,
            })
        } else {
            None
        }
    }
}
