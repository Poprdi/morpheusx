/// HID (Human Interface Device) USB interface implementation
/// 
/// Provides HID interface descriptors, report descriptor parsing,
/// and bridges input events to the same InputKey type used by the PS/2 path.

pub mod keyboard;
pub mod mouse;

use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
use crate::usb::regs::*;

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
        let desc_buf = self.dma_base + dma::OFF_DESCRIPTOR as u64;
        let ctrl = &mut self.ep0;

        create::usb::controller::ControlXfer {
            ep0: ctrl,
            slot_id: self.slot_id,
        }
        .control_in(
            0x81,
            0x06,
            (0x22 << 8) as u16,
            interface_num as u16,
            len,
            desc_buf as *mut u8,
            |sid, dci| {
                self.ring_doorbell(dci);
                self.wait_xfer(sid, dci, 5000)
            },
        )?;

        Ok(desc_buf as *const u8)
    }

    /// Set the HID idle duration (all endpoints, duration=0 = infinite).
    pub unsafe fn set_hid_idle(&mut self, interface_num: u8) -> Result<(), XhciError> {
        let ctrl = &mut self.ep0;

        create::usb::controller::ControlXfer {
            ep0: ctrl,
            slot_id: self.slot_id,
        }
        .control_no_data(
            0x21,
            0x0A,
            0,
            interface_num as u16,
            |sid, dci| {
                self.ring_doorbell(dci);
                self.wait_xfer(sid, dci, 2000)
            },
        )?;

        Ok(())
    }

    /// Find the first HID boot interface from the already-fetched config descriptor.
    /// Returns (ep_in, ep_out, ep_dci_in, ep_dci_out, protocol).
    pub unsafe fn find_hid_interface(
        &self,
        desc_ptr: *const u8,
    ) -> Option<(u8, u8, u8, u8, u8)> {
        let desc = desc_ptr as u64;
        let total = u16::from_le_bytes([desc_ptr as u8, (desc_ptr as u8) + 1]) as usize;

        let mut off = 0usize;
        let mut limit = total.min(255);

        while off + 2 <= limit {
            let len = desc_ptr.add(off) as u8 as usize;
            if len == 0 || off + len > limit {
                break;
            }

            let btype = desc_ptr.add(off + 1) as u8;

            if btype == 4 && len >= 9 {
                let cls = desc_ptr.add(off + 5) as u8;
                let subcls = desc_ptr.add(off + 6) as u8;
                let proto = desc_ptr.add(off + 7) as u8;

                if cls == USB_CLASS_HID && subcls == USB_SUBCLASS_BOOT && proto == USB_PROTOCOL_KEYBOARD || proto == USB_PROTOCOL_MOUSE {
                    let ep_in = desc_ptr.add(off + 8) as u8 & 0x7F;
                    let ep_out = if (desc_ptr.add(off + 8) as u8 & 0x80) == 0 {
                        desc_ptr.add(off + 8) as u8
                    } else {
                        0
                    };

                    let dci_in = (ep_in as u8) * 2;
                    let dci_out = if ep_out != 0 { ep_out as u8 * 2 + 1 } else { 0 };

                    return Some((ep_in, ep_out, dci_in, dci_out, proto));
                }
            }

            off += len;
        }

        None
    }
}