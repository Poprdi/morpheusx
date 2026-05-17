//! Control transfers (EP0): setup → optional data → status.

use crate::usb::regs::*;
use crate::usb::rings::{write_trb, vr32, vw32};

/// USB setup packet packed into a 64-bit value for IDT TRBs.
///   bits  7:0  = bmRequestType
///   bits 15:8  = bRequest
///   bits 31:16 = wValue
///   bits 47:32 = wIndex
///   bits 63:48 = wLength
#[inline(always)]
pub fn pack_setup(req_type: u8, req: u8, val: u16, idx: u16, len: u16) -> u64 {
    (req_type as u64)
        | ((req as u64) << 8)
        | ((val as u64) << 16)
        | ((idx as u64) << 32)
        | ((len as u64) << 48)
}

/// Control transfer helper — owns EP0 ring state.
pub struct ControlXfer<'a> {
    pub ep0: &'a mut crate::usb::rings::XferRing,
    pub slot_id: u8,
}

impl<'a> ControlXfer<'a> {
    /// Submit a SETUP TRB on EP0.
    #[inline(always)]
    pub unsafe fn setup(&mut self, param: u64, len: u32) {
        let ctrl = TRB_SETUP | TRB_IDT | TRB_TRT_IN;
        self.ep0.enqueue(param, len, ctrl);
    }

    /// Submit a data-stage TRB on EP0.
    #[inline(always)]
    pub unsafe fn data(&mut self, param: u64, len: u32, dir_in: bool) {
        let ctrl = TRB_DATA | TRB_ISP | if dir_in { TRB_DIR_IN } else { 0 };
        self.ep0.enqueue(param, len, ctrl);
    }

    /// Submit a status-stage TRB on EP0. dir_in=true → host→device status (no data).
    #[inline(always)]
    pub unsafe fn status(&mut self, dir_in: bool) {
        let ctrl = TRB_STATUS | TRB_IOC | if dir_in { TRB_DIR_IN } else { 0 };
        self.ep0.enqueue(0, 0, ctrl);
    }

    /// IN control transfer: GET_DESCRIPTOR etc.
    /// Returns slice into `desc_buf`. `wait_xfer_fn` must poll the event ring until TRB_TRANSFER_EVENT.
    #[inline(always)]
    pub unsafe fn control_in<
        F: FnMut(u8, u32) -> Result<u32, crate::usb::controller::XhciError>,
    >(
        &mut self,
        req_type: u8,
        request: u8,
        value: u16,
        index: u16,
        len: u16,
        desc_buf: u64,
        wait_xfer_fn: &mut F,
    ) -> Result<(), crate::usb::controller::XhciError> {
        let param = pack_setup(req_type, request, value, index, len);
        self.setup(param, 8);
        self.data(desc_buf, len as u32, true);
        self.status(false);
        wait_xfer_fn(self.slot_id, 1)?; // doorbell DCI=1 for EP0
        let _ = wait_xfer_fn(self.slot_id, 0)?; // data
        let _ = wait_xfer_fn(self.slot_id, 0)?; // status
        Ok(())
    }

    /// No-data control transfer: SET_ADDRESS, SET_CONFIGURATION etc.
    #[inline(always)]
    pub unsafe fn control_nodata<
        F: FnMut(u8, u32) -> Result<u32, crate::usb::controller::XhciError>,
    >(
        &mut self,
        req_type: u8,
        request: u8,
        value: u16,
        index: u16,
        wait_xfer_fn: &mut F,
    ) -> Result<(), crate::usb::controller::XhciError> {
        let param = pack_setup(req_type, request, value, index, 0);
        self.setup(param, 8);
        self.status(true);
        wait_xfer_fn(self.slot_id, 1)?;
        let _ = wait_xfer_fn(self.slot_id, 0)?;
        Ok(())
    }
}