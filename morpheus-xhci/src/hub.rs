//! USB Hub class support — required to reach devices that aren't on a root port.
//!
//! Spec references:
//! - USB 2.0 spec chapter 11 (hub class request encoding, port status/change
//!   bits, port feature numbers)
//! - xHCI 1.2 §4.20, §6.2.2.1 (slot-context Hub bit, Number of Ports, TT
//!   Think Time, parent-hub routing fields)

use crate::controller::{XhciController, XhciError};
use crate::dma;
use crate::pack_setup;
use crate::regs::*;
use crate::rings::{vr32, vw32};

pub const USB_CLASS_HUB: u8 = 0x09;

// Hub descriptor type (wValue high byte of class-specific GET_DESCRIPTOR).
const HUB_DESC_NORMAL: u16 = 0x29 << 8;

// Port feature numbers (USB 2.0 §11.24.2.7.1)
const PORT_FEAT_RESET: u16 = 4;
const PORT_FEAT_POWER: u16 = 8;
const PORT_FEAT_C_CONNECTION: u16 = 16;
const PORT_FEAT_C_RESET: u16 = 20;

// wPortStatus bits in the low half of the 32-bit GET_PORT_STATUS result.
pub const PORT_STAT_CONNECTION: u32 = 1 << 0;
pub const PORT_STAT_ENABLE: u32 = 1 << 1;
pub const PORT_STAT_RESET: u32 = 1 << 4;
pub const PORT_STAT_POWER: u32 = 1 << 8;
pub const PORT_STAT_LOW_SPEED: u32 = 1 << 9;
pub const PORT_STAT_HIGH_SPEED: u32 = 1 << 10;

// wPortChange bits in the high half (offset +16).
pub const PORT_CHG_C_CONNECTION: u32 = 1 << 16;
pub const PORT_CHG_C_RESET: u32 = 1 << 20;

/// Parsed hub descriptor — only the fields enumeration needs.
#[derive(Debug, Clone, Copy)]
pub struct HubInfo {
    pub num_ports: u8,
    /// 0..=3 from wHubCharacteristics bits 5:6, stored in slot context DW2[17:16].
    pub tt_think_time: u8,
    /// Power-on to power-good delay in milliseconds (bPwrOn2PwrGood × 2).
    pub pwr_on_2_pwr_good_ms: u16,
}

impl XhciController {
    /// Class-specific GET_DESCRIPTOR(HUB). Issued on EP0 of the slot currently
    /// pointed to by `self.slot_id`.
    pub unsafe fn get_hub_descriptor(&mut self) -> Result<HubInfo, XhciError> {
        let buf = self.dma_base + dma::OFF_DESC as u64;
        let slot_id = self.slot_id;
        // 0xA0 = D2H, Class, Device; 0x06 = GET_DESCRIPTOR.
        let setup = pack_setup(0xA0, 0x06, HUB_DESC_NORMAL, 0, 16);
        self.ep0.enqueue(setup, 8, TRB_SETUP | TRB_IDT | TRB_TRT_IN);
        self.ep0.enqueue(buf, 16, TRB_DATA | TRB_ISP | TRB_DIR_IN);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;

        let num_ports = core::ptr::read_volatile((buf + 2) as *const u8);
        let chars_lo = core::ptr::read_volatile((buf + 3) as *const u8);
        let pwr_on = core::ptr::read_volatile((buf + 5) as *const u8);

        Ok(HubInfo {
            num_ports,
            tt_think_time: (chars_lo >> 5) & 0x3,
            pwr_on_2_pwr_good_ms: (pwr_on as u16).saturating_mul(2),
        })
    }

    pub unsafe fn hub_port_power_on(&mut self, port: u8) -> Result<(), XhciError> {
        self.hub_port_set_feature(port, PORT_FEAT_POWER)
    }

    /// SET_FEATURE(PORT_RESET) + poll for C_PORT_RESET. Clears the change bit
    /// on completion. Returns the detected speed (1=FS, 2=LS, 3=HS).
    pub unsafe fn hub_port_reset(&mut self, port: u8) -> Result<u8, XhciError> {
        self.hub_port_set_feature(port, PORT_FEAT_RESET)?;

        let start = morpheus_x86_asm::tsc::read_tsc();
        let timeout = self.tsc_freq; // 1 second
        loop {
            let status = self.hub_port_get_status(port)?;
            if status & PORT_CHG_C_RESET != 0 {
                self.hub_port_clear_feature(port, PORT_FEAT_C_RESET)?;
                let speed = if status & PORT_STAT_LOW_SPEED != 0 {
                    2 // LS
                } else if status & PORT_STAT_HIGH_SPEED != 0 {
                    3 // HS
                } else {
                    1 // FS
                };
                return Ok(speed);
            }
            if morpheus_x86_asm::tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(XhciError::PortResetTimeout);
            }
            core::hint::spin_loop();
        }
    }

    /// GET_PORT_STATUS — returns `wPortStatus | (wPortChange << 16)`.
    pub unsafe fn hub_port_get_status(&mut self, port: u8) -> Result<u32, XhciError> {
        let buf = self.dma_base + dma::OFF_DESC as u64;
        let slot_id = self.slot_id;
        // 0xA3 = D2H, Class, Other; 0x00 = GET_STATUS.
        let setup = pack_setup(0xA3, 0x00, 0, port as u16, 4);
        self.ep0.enqueue(setup, 8, TRB_SETUP | TRB_IDT | TRB_TRT_IN);
        self.ep0.enqueue(buf, 4, TRB_DATA | TRB_ISP | TRB_DIR_IN);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;
        Ok(vr32(buf))
    }

    pub unsafe fn hub_port_clear_connection_change(&mut self, port: u8) -> Result<(), XhciError> {
        self.hub_port_clear_feature(port, PORT_FEAT_C_CONNECTION)
    }

    unsafe fn hub_port_set_feature(&mut self, port: u8, feature: u16) -> Result<(), XhciError> {
        let slot_id = self.slot_id;
        // 0x23 = H2D, Class, Other; 0x03 = SET_FEATURE.
        let setup = pack_setup(0x23, 0x03, feature, port as u16, 0);
        self.ep0.enqueue(setup, 8, TRB_SETUP | TRB_IDT);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC | TRB_DIR_IN);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;
        Ok(())
    }

    unsafe fn hub_port_clear_feature(&mut self, port: u8, feature: u16) -> Result<(), XhciError> {
        let slot_id = self.slot_id;
        // 0x23 = H2D, Class, Other; 0x01 = CLEAR_FEATURE.
        let setup = pack_setup(0x23, 0x01, feature, port as u16, 0);
        self.ep0.enqueue(setup, 8, TRB_SETUP | TRB_IDT);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC | TRB_DIR_IN);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;
        Ok(())
    }

    /// Update the slot context to mark `self.slot_id` as a USB hub. Must run
    /// after `address_device` (so the output context already has the device's
    /// current state). We copy the output slot context into the input context,
    /// set Hub=1 / Number of Ports / TT Think Time, and issue CONFIGURE_ENDPOINT
    /// with the slot-context-update flag (A0=1, no endpoint changes).
    ///
    /// xHCI §6.2.2.1: DW0 bit 26 = Hub, DW1 [31:24] = Number of Ports,
    /// DW2 [17:16] = TT Think Time.
    pub unsafe fn configure_hub_slot(
        &mut self,
        num_ports: u8,
        multi_tt: bool,
        ttt: u8,
    ) -> Result<(), XhciError> {
        let cs = self.ctx_size as u64;
        let in_ctx = self.dma_base + dma::OFF_IN_CTX as u64;
        let out_ctx = self.dma_base + dma::slot_out_ctx_offset(self.slot_id) as u64;

        // Wipe input control context + slot context + EP0 context area
        core::ptr::write_bytes(in_ctx as *mut u8, 0, (3 * cs) as usize);

        // Add Context flag A0 (bit 0 of DW1 in the input control context) =
        // "update slot context only". No endpoint adds.
        vw32(in_ctx + 4, 0x01);

        // Copy current slot context (DW0..DW3) from output → input, then patch
        // the hub-specific bits.
        let in_slot = in_ctx + cs;
        let mut d0 = vr32(out_ctx);
        d0 |= 1u32 << 26; // Hub
        if multi_tt {
            d0 |= 1u32 << 25; // MTT
        }
        vw32(in_slot, d0);

        let d1 = (vr32(out_ctx + 4) & 0x00FF_FFFF) | ((num_ports as u32) << 24);
        vw32(in_slot + 4, d1);

        let d2 = (vr32(out_ctx + 8) & !(0x3u32 << 16)) | (((ttt as u32) & 0x3) << 16);
        vw32(in_slot + 8, d2);

        vw32(in_slot + 12, vr32(out_ctx + 12));

        let ctrl = TRB_CONFIGURE_EP | ((self.slot_id as u32) << 24);
        self.cmd_ring.enqueue(in_ctx, 0, ctrl);
        self.ring_cmd_doorbell();
        self.wait_cmd(2000)?;
        Ok(())
    }
}
