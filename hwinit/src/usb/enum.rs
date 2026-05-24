//! xHCI device enumeration: slot, address, endpoint configuration, descriptor parsing.

use crate::usb::control::pack_setup;
use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
use crate::usb::regs::*;
use crate::usb::rings::{vr32, vw32, vw64};

/// USB device speed mapping (PORTSC[13:10]).
pub fn ep0_max_packet(speed: u8) -> u16 {
    match speed {
        4 => 512, // SS
        3 => 64,  // HS
        2 => 8,   // LS
        _ => 64,  // FS
    }
}

impl XhciController {
    /// Allocate a device slot.
    pub unsafe fn enable_slot(&mut self) -> Result<u8, XhciError> {
        self.cmd_ring.enqueue(0, 0, TRB_ENABLE_SLOT);
        self.ring_cmd_doorbell();
        let (slot, _) = match self.wait_cmd(2000) {
            Ok(v) => v,
            Err(XhciError::CommandTimeout) => {
                // No completion event in 2 s. Controller never processed the
                // command — usually means CRCR wasn't accepted, MaxSlotsEn=0,
                // bus-mastering disabled, or the doorbell isn't being seen.
                crate::serial::log_warn("USB", 251, "enable_slot: no completion event (timeout)");
                self.dump_state("enable_slot.timeout");
                return Err(XhciError::CommandTimeout);
            }
            Err(e) => {
                // Got a completion event but with a non-success code (CC != 1).
                // Most likely "No Slots Available" (CC=12) — MaxSlotsEnabled issue.
                crate::serial::log_warn("USB", 252, "enable_slot: command returned error CC");
                self.dump_state("enable_slot.cc_err");
                return Err(e);
            }
        };
        if slot == 0 {
            // Completion event was successful but reported slot 0 — controller
            // bug or context-size mismatch (slot ID in wrong bit position).
            crate::serial::log_warn("USB", 253, "enable_slot: success event but slot id = 0");
            return Err(XhciError::EnableSlotFailed);
        }
        self.slot_id = slot;

        // wire output context into DCBAA
        let out_ctx = self.dma_base + dma::OFF_OUT_CTX as u64;
        vw64(
            self.dma_base + dma::OFF_DCBAA as u64 + (slot as u64) * 8,
            out_ctx,
        );
        Ok(slot)
    }

    /// Address the device whose slot is currently in `self.slot_id`.
    ///
    /// `root_port` is the 0-based root-hub port the device's link traverses
    /// (the slot context stores it as 1-based in DW1[23:16]).
    /// `route` is the 20-bit xHCI route string — 0 for devices directly on a
    /// root port, populated for devices behind hubs.
    /// `parent_hub_slot` / `parent_hub_port` identify the immediately upstream
    /// HS hub when this is a LS/FS device (used by the controller's TT routing).
    /// They are 0 for devices that don't need TT forwarding.
    pub unsafe fn address_device(
        &mut self,
        root_port: u8,
        speed: u8,
        route: u32,
        parent_hub_slot: u8,
        parent_hub_port: u8,
    ) -> Result<(), XhciError> {
        let cs = self.ctx_size as u64;
        let in_ctx = self.dma_base + dma::OFF_IN_CTX as u64;

        core::ptr::write_bytes(in_ctx as *mut u8, 0, (33 * cs) as usize);
        vw32(in_ctx + 4, 0x03); // add slot + EP0

        let slot_ctx = in_ctx + cs;
        let max_pkt = ep0_max_packet(speed);
        // Slot Context DW0:
        //   [19:0]  Route String
        //   [23:20] Speed
        //   [26]    Hub   — must be 0 for non-hub devices (set later via
        //                  configure_hub_slot if device turns out to be a hub)
        //   [31:27] Context Entries — must be >=1 (0 is reserved/invalid)
        let d0 = (route & 0x000F_FFFF) | ((speed as u32) << 20) | (1u32 << 27);
        vw32(slot_ctx, d0);
        // DW1 [23:16] Root Hub Port Number (1-based)
        vw32(slot_ctx + 4, ((root_port as u32 + 1) & 0xFF) << 16);
        // DW2: TT routing for LS/FS devices behind a HS hub.
        //   [7:0]  Parent Hub Slot ID
        //   [15:8] Parent Port Number
        // (TT Think Time lives in [17:16] but is only meaningful for hubs;
        // configure_hub_slot patches it in if this device turns out to be one.)
        let d2 = if parent_hub_slot != 0 && speed < 3 {
            (parent_hub_slot as u32) | ((parent_hub_port as u32) << 8)
        } else {
            0
        };
        vw32(slot_ctx + 8, d2);

        let ep0 = in_ctx + 2 * cs;
        vw32(
            ep0 + 4,
            (3u32 << 1) | (4u32 << 3) | ((max_pkt as u32) << 16),
        );
        // CRITICAL: each slot's EP0 context dequeue pointer must reflect the
        // CURRENT position of the shared EP0 transfer ring (with the current
        // cycle bit), not the ring base with cycle=1. Otherwise the controller
        // tries to read stale TRBs left over from previous devices.
        let ring_base = self.dma_base + dma::OFF_XFER_EP0 as u64;
        let cur_pos = ring_base + (self.ep0.enq as u64) * 16;
        let dcs = self.ep0.cycle as u32 & 1;
        vw32(ep0 + 8, (cur_pos as u32 & !0xF) | dcs);
        vw32(ep0 + 12, (cur_pos >> 32) as u32);
        vw32(ep0 + 16, 8);

        let ctrl = TRB_ADDRESS_DEV | ((self.slot_id as u32) << 24);
        self.cmd_ring.enqueue(in_ctx, 0, ctrl);
        self.ring_cmd_doorbell();
        self.wait_cmd(2000)?;
        Ok(())
    }

    /// Configure bulk-in and bulk-out endpoints.
    pub unsafe fn configure_endpoints(
        &mut self,
        dci_in: u8,
        dci_out: u8,
        mpkt_in: u16,
        mpkt_out: u16,
    ) -> Result<(), XhciError> {
        let cs = self.ctx_size as u64;
        let in_ctx = self.dma_base + dma::OFF_IN_CTX as u64;
        let max_dci = dci_in.max(dci_out);

        core::ptr::write_bytes(in_ctx as *mut u8, 0, ((max_dci as u64 + 2) * cs) as usize);

        let add_flags = (1u32 << 0) | (1u32 << dci_in) | (1u32 << dci_out);
        vw32(in_ctx + 4, add_flags);

        let out_slot = self.dma_base + dma::OFF_OUT_CTX as u64;
        let d0 = vr32(out_slot);
        vw32(in_ctx + cs, (d0 & (0xF << 20)) | ((max_dci as u32) << 26));
        vw32(in_ctx + cs + 4, vr32(out_slot + 4));

        let ep_in = in_ctx + ((dci_in as u64) + 1) * cs;
        vw32(
            ep_in + 4,
            (3u32 << 1) | (6u32 << 3) | ((mpkt_in as u32) << 16),
        );
        let ring_in = self.dma_base + dma::OFF_XFER_BIN as u64;
        vw32(ep_in + 8, (ring_in as u32 & !0xF) | 1);
        vw32(ep_in + 12, (ring_in >> 32) as u32);
        vw32(ep_in + 16, 1024);

        let ep_out = in_ctx + ((dci_out as u64) + 1) * cs;
        vw32(
            ep_out + 4,
            (3u32 << 1) | (2u32 << 3) | ((mpkt_out as u32) << 16),
        );
        let ring_out = self.dma_base + dma::OFF_XFER_BOUT as u64;
        vw32(ep_out + 8, (ring_out as u32 & !0xF) | 1);
        vw32(ep_out + 12, (ring_out >> 32) as u32);
        vw32(ep_out + 16, 1024);

        let ctrl = TRB_CONFIGURE_EP | ((self.slot_id as u32) << 24);
        self.cmd_ring.enqueue(in_ctx, 0, ctrl);
        self.ring_cmd_doorbell();
        self.wait_cmd(2000)?;
        Ok(())
    }

    /// Fetch device descriptor (18 bytes). Returns pointer into DMA buffer.
    pub unsafe fn get_device_descriptor(&mut self) -> Result<*const u8, XhciError> {
        let desc_buf = self.dma_base + dma::OFF_DESC as u64;
        let slot_id = self.slot_id;
        let param = pack_setup(0x80, 0x06, 0x0100, 0, 18);
        self.ep0.enqueue(param, 8, TRB_SETUP | TRB_IDT | TRB_TRT_IN);
        self.ep0
            .enqueue(desc_buf, 18, TRB_DATA | TRB_ISP | TRB_DIR_IN);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;
        Ok(desc_buf as *const u8)
    }

    /// Fetch configuration descriptor.
    pub unsafe fn get_config_descriptor(&mut self, len: u16) -> Result<*const u8, XhciError> {
        let desc_buf = self.dma_base + dma::OFF_DESC as u64;
        let slot_id = self.slot_id;
        let param = pack_setup(0x80, 0x06, 0x0200, 0, len);
        self.ep0.enqueue(param, 8, TRB_SETUP | TRB_IDT | TRB_TRT_IN);
        self.ep0
            .enqueue(desc_buf, len as u32, TRB_DATA | TRB_ISP | TRB_DIR_IN);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;
        Ok(desc_buf as *const u8)
    }

    /// Set the active configuration.
    pub unsafe fn set_configuration(&mut self, cfg_val: u8) -> Result<(), XhciError> {
        let slot_id = self.slot_id;
        let param = pack_setup(0x00, 0x09, cfg_val as u16, 0, 0);
        self.ep0.enqueue(param, 8, TRB_SETUP | TRB_IDT);
        self.ep0.enqueue(0, 0, TRB_STATUS | TRB_IOC | TRB_DIR_IN);
        self.ring_xfer_doorbell(1);
        self.wait_xfer(slot_id, 1, 5000)?;
        Ok(())
    }

    /// Parse configuration descriptor. Returns (cfg_val, ep_in, ep_out, mpkt_in, mpkt_out).
    pub unsafe fn parse_config(&self, desc_ptr: *const u8) -> Option<(u8, u8, u8, u16, u16)> {
        let d = desc_ptr;
        let total = u16::from_le_bytes([
            core::ptr::read_volatile(d.add(2)),
            core::ptr::read_volatile(d.add(3)),
        ]) as usize;
        let cfg_val = core::ptr::read_volatile(d.add(5));

        let limit = total.min(255);

        let mut off = 0usize;
        let mut ep_in: u8 = 0;
        let mut ep_out: u8 = 0;
        let mut mp_in: u16 = 0;
        let mut mp_out: u16 = 0;
        let mut in_bot_msc = false;

        while off + 2 <= limit {
            let blen = core::ptr::read_volatile(d.add(off)) as usize;
            let btype = core::ptr::read_volatile(d.add(off + 1));
            if blen < 2 || off + blen > limit {
                break;
            }
            if btype == 4 && blen >= 9 {
                let cls = core::ptr::read_volatile(d.add(off + 5));
                let sub = core::ptr::read_volatile(d.add(off + 6));
                let proto = core::ptr::read_volatile(d.add(off + 7));
                in_bot_msc = cls == 0x08 && sub == 0x06 && proto == 0x50;
            }
            if btype == 5 && blen >= 7 && in_bot_msc {
                let addr = core::ptr::read_volatile(d.add(off + 2));
                let attr = core::ptr::read_volatile(d.add(off + 3));
                let mpkt = u16::from_le_bytes([
                    core::ptr::read_volatile(d.add(off + 4)),
                    core::ptr::read_volatile(d.add(off + 5)),
                ]);
                if attr & 0x03 == 0x02 {
                    if addr & 0x80 != 0 {
                        ep_in = addr;
                        mp_in = mpkt;
                    } else {
                        ep_out = addr;
                        mp_out = mpkt;
                    }
                }
            }
            off += blen;
        }

        if ep_in != 0 && ep_out != 0 {
            Some((cfg_val, ep_in, ep_out, mp_in, mp_out))
        } else {
            None
        }
    }

    /// Reset all transfer rings and contexts for a fresh enumeration attempt.
    pub unsafe fn reset_transfer_state(&mut self) {
        core::ptr::write_bytes(
            (self.dma_base + dma::OFF_XFER_EP0 as u64) as *mut u8,
            0,
            256,
        );
        core::ptr::write_bytes(
            (self.dma_base + dma::OFF_XFER_BOUT as u64) as *mut u8,
            0,
            256,
        );
        core::ptr::write_bytes(
            (self.dma_base + dma::OFF_XFER_BIN as u64) as *mut u8,
            0,
            256,
        );
        core::ptr::write_bytes(
            (self.dma_base + dma::OFF_OUT_CTX as u64) as *mut u8,
            0,
            2048,
        );
        core::ptr::write_bytes((self.dma_base + dma::OFF_IN_CTX as u64) as *mut u8, 0, 2560);
        self.ep0.reset();
        self.bout.reset();
        self.bin.reset();
    }
}
