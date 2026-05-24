/// HID (Human Interface Device) USB interface implementation
///
/// Provides HID interface descriptors, report descriptor parsing,
/// and bridges input events to the same InputKey type used by the PS/2 path.
pub mod keyboard;
pub mod mouse;

use crate::usb::control::pack_setup;
use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
use crate::usb::regs::*;

// USB HID class/subclass/protocol constants (HID spec 1.11 §4.2)
pub const USB_CLASS_HID: u8 = 0x03;
pub const USB_SUBCLASS_BOOT: u8 = 0x01;
pub const USB_PROTOCOL_KEYBOARD: u8 = 0x01;
pub const USB_PROTOCOL_MOUSE: u8 = 0x02;

/// HID interface descriptor extracted from config.
#[derive(Debug)]
pub struct HIDInterface {
    pub interface_num: u8,
    pub protocol: u8,
    pub ep_in: u8,
    pub ep_out: u8,
    pub max_packet_in: u16,
}

impl XhciController {
    /// Fetch HID report descriptor via GET_DESCRIPTOR (type 0x22).
    pub unsafe fn get_hid_report_descriptor(
        &mut self,
        interface_num: u8,
        len: u16,
    ) -> Result<*const u8, XhciError> {
        let desc_buf = self.dma_base + dma::OFF_DESC as u64;
        let slot_id = self.slot_id;
        let param = pack_setup(
            0x81,
            0x06,
            0x2200 | interface_num as u16,
            interface_num as u16,
            len,
        );
        self.ep0.enqueue(param, 8, TRB_SETUP | TRB_IDT | TRB_TRT_IN);
        self.ep0
            .enqueue(desc_buf, len as u32, TRB_DATA | TRB_ISP | TRB_DIR_IN);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;
        Ok(desc_buf as *const u8)
    }

    /// Set the HID idle duration (all endpoints, duration=0 = infinite).
    pub unsafe fn set_hid_idle(&mut self, interface_num: u8) -> Result<(), XhciError> {
        let slot_id = self.slot_id;
        let param = pack_setup(0x21, 0x0A, 0, interface_num as u16, 0);
        self.ep0.enqueue(param, 8, TRB_SETUP | TRB_IDT);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC | TRB_DIR_IN);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 2000)?;
        Ok(())
    }

    /// Find the first HID boot interface from the already-fetched config descriptor.
    /// Returns a HIDInterface if a boot-class keyboard or mouse is found.
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
        let mut in_hid_boot = false;

        while off + 2 <= limit {
            let blen = core::ptr::read_volatile(d.add(off)) as usize;
            let btype = core::ptr::read_volatile(d.add(off + 1));
            if blen < 2 || off + blen > limit {
                break;
            }

            if btype == 4 && blen >= 9 {
                // Interface descriptor
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

        if in_hid_boot && ep_in != 0 {
            Some(HIDInterface {
                interface_num: iface_num,
                protocol: proto,
                ep_in,
                ep_out,
                max_packet_in: mp_in,
            })
        } else {
            None
        }
    }
}
