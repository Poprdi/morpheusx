//! Control transfers (EP0): setup → optional data → status. Re-exports
//! `pack_setup` from `morpheus-xhci` for back-compat.

pub use morpheus_xhci::pack_setup;

use morpheus_xhci::regs::*;
use morpheus_xhci::rings::XferRing;

pub struct ControlXfer<'a> {
    pub ep0: &'a mut XferRing,
    pub slot_id: u8,
}

impl<'a> ControlXfer<'a> {
    #[inline(always)]
    pub unsafe fn setup(&mut self, param: u64, len: u32) {
        let ctrl = TRB_SETUP | TRB_IDT | TRB_TRT_IN;
        self.ep0.enqueue(param, len, ctrl);
    }

    #[inline(always)]
    pub unsafe fn data(&mut self, param: u64, len: u32, dir_in: bool) {
        let ctrl = TRB_DATA | TRB_ISP | if dir_in { TRB_DIR_IN } else { 0 };
        self.ep0.enqueue(param, len, ctrl);
    }

    /// `dir_in=true` = host->device status (no data).
    #[inline(always)]
    pub unsafe fn status(&mut self, dir_in: bool) {
        let ctrl = TRB_STATUS | TRB_IOC | if dir_in { TRB_DIR_IN } else { 0 };
        self.ep0.enqueue(0, 0, ctrl);
    }

    /// IN control transfer (GET_DESCRIPTOR etc). `wait_xfer_fn` must poll
    /// the event ring until TRB_TRANSFER_EVENT.
    #[inline(always)]
    pub unsafe fn control_in<F: FnMut(u8, u32) -> Result<u32, morpheus_xhci::XhciError>>(
        &mut self,
        req_type: u8,
        request: u8,
        value: u16,
        index: u16,
        len: u16,
        desc_buf: u64,
        wait_xfer_fn: &mut F,
    ) -> Result<(), morpheus_xhci::XhciError> {
        let param = pack_setup(req_type, request, value, index, len);
        self.setup(param, 8);
        self.data(desc_buf, len as u32, true);
        self.status(false);
        wait_xfer_fn(self.slot_id, 1)?; // EP0 doorbell DCI=1
        let _ = wait_xfer_fn(self.slot_id, 0)?; // data
        let _ = wait_xfer_fn(self.slot_id, 0)?; // status
        Ok(())
    }

    /// SET_ADDRESS, SET_CONFIGURATION etc.
    #[inline(always)]
    pub unsafe fn control_nodata<F: FnMut(u8, u32) -> Result<u32, morpheus_xhci::XhciError>>(
        &mut self,
        req_type: u8,
        request: u8,
        value: u16,
        index: u16,
        wait_xfer_fn: &mut F,
    ) -> Result<(), morpheus_xhci::XhciError> {
        let param = pack_setup(req_type, request, value, index, 0);
        self.setup(param, 8);
        self.status(true);
        wait_xfer_fn(self.slot_id, 1)?;
        let _ = wait_xfer_fn(self.slot_id, 0)?;
        Ok(())
    }
}
