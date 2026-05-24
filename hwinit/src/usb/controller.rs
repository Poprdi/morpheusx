//! xHCI controller bring-up: probe, soft-restart, BIOS handoff, port management.

use crate::cpu::mmio;
use crate::cpu::tsc;
use crate::usb::asm;
use crate::usb::dma;
use crate::usb::dma::{EVT_RING_LEN, XFER_RING_LEN};
use crate::usb::regs::*;
use crate::usb::rings::{vw32, vw64, CmdRing, EvtRing, XferRing};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhciError {
    ProbeFailed,
    ResetFailed,
    StartFailed,
    ScratchpadUnsupported,
    PortResetTimeout,
    PortResetNoLink,
    PortResetNoCCS,
    EnableSlotFailed,
    AddressDeviceFailed,
    ConfigureEndpointsFailed,
    CommandTimeout,
    IoError,
    NoMedia,
    NotSupported,
}

impl core::fmt::Display for XhciError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ProbeFailed => write!(f, "xhci probe failed (dead BAR)"),
            Self::ResetFailed => write!(f, "xhci reset failed"),
            Self::StartFailed => write!(f, "xhci start failed"),
            Self::ScratchpadUnsupported => write!(f, "xhci scratchpad unsupported"),
            Self::PortResetTimeout => write!(f, "port reset timeout"),
            Self::PortResetNoLink => write!(f, "port reset no link"),
            Self::PortResetNoCCS => write!(f, "port reset no CCS"),
            Self::EnableSlotFailed => write!(f, "enable slot failed"),
            Self::AddressDeviceFailed => write!(f, "address device failed"),
            Self::ConfigureEndpointsFailed => write!(f, "configure endpoints failed"),
            Self::CommandTimeout => write!(f, "command timeout"),
            Self::IoError => write!(f, "I/O error"),
            Self::NoMedia => write!(f, "no media"),
            Self::NotSupported => write!(f, "not supported"),
        }
    }
}

pub struct XhciController {
    pub mmio_base: u64,
    pub op_base: u64,
    pub rt_base: u64,
    pub db_base: u64,
    pub tsc_freq: u64,
    pub max_ports: u8,
    pub ctx_size: u8,
    pub dma_base: u64,
    pub slot_id: u8,
    pub dci_bulk_in: u8,
    pub dci_bulk_out: u8,

    pub cmd_ring: CmdRing,
    pub evt_ring: EvtRing,
    pub ep0: XferRing,
    pub bout: XferRing,
    pub bin: XferRing,
}

impl XhciController {
    pub unsafe fn new(mmio_base: u64, tsc_freq: u64) -> Result<Self, XhciError> {
        if mmio_base == 0 || tsc_freq == 0 {
            return Err(XhciError::ProbeFailed);
        }

        // probe controller
        let probe = asm::asm_usb_host_probe(mmio_base);
        if probe == 0 {
            return Err(XhciError::ProbeFailed);
        }
        let cap_len = (probe & 0xFF) as u64;
        let op_base = mmio_base + cap_len;

        let hcsparams1 = mmio::read32(mmio_base + CAP_HCSPARAMS1);
        let hcsparams2 = mmio::read32(mmio_base + CAP_HCSPARAMS2);
        let hccparams1 = mmio::read32(mmio_base + CAP_HCCPARAMS1);
        let db_off = mmio::read32(mmio_base + CAP_DBOFF) & !0x03;
        let rts_off = mmio::read32(mmio_base + CAP_RTSOFF) & !0x1F;

        let max_slots = (hcsparams1 & 0xFF) as u8;
        let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;
        let ctx_size: u8 = if hccparams1 & (1 << 2) != 0 { 64 } else { 32 };
        let scratch_hi = ((hcsparams2 >> 21) & 0x1F) as u16;
        let scratch_lo = ((hcsparams2 >> 27) & 0x1F) as u16;
        let n_scratch = ((scratch_hi << 5) | scratch_lo) as usize;

        let rt_base = mmio_base + rts_off as u64;
        let db_base = mmio_base + db_off as u64;

        let dma_base = dma::dma_base();
        dma::dma_zero(dma_base);

        // BIOS/SMM handoff
        let _ = asm::asm_xhci_bios_handoff(mmio_base, hccparams1 as u64, tsc_freq);
        Self::tsc_delay(tsc_freq, 10);

        // Check if firmware left any port with link state
        let mut linked = 0u32;
        for p in 0..max_ports {
            let ps = mmio::read32(op_base + PORT_REG_BASE + (p as u64) * PORT_REG_STRIDE);
            let speed = (ps >> PORTSC_SPEED_SHIFT) & 0xF;
            if (ps & (PORTSC_CCS | PORTSC_PED)) != 0 || speed != 0 {
                linked += 1;
            }
        }

        // Soft restart if all ports are dead
        if linked == 0 {
            let rc = asm::asm_xhci_controller_soft_restart(op_base, tsc_freq);
            if rc != 0 {
                return Err(XhciError::ResetFailed);
            }
        }
        Self::tsc_delay(tsc_freq, 50);

        // Halt the controller before reconfiguring DCBAAP / CRCR / ERST.
        // xHCI spec §5.4.5/§5.4.6: writes to those registers are silently
        // ignored by the xHC while the controller is running. UEFI typically
        // hands off with USBCMD.R/S = 1 (and the conditional soft-restart
        // above is skipped when ports already have link state), so without
        // this halt the ring-pointer writes below would be no-ops on real
        // hardware and ENABLE_SLOT would time out with no completion event.
        // HCRST is NOT asserted — connected port state is preserved.
        {
            let cmd = mmio::read32(op_base + OP_USBCMD);
            if cmd & CMD_RS != 0 {
                mmio::write32(op_base + OP_USBCMD, cmd & !CMD_RS);
            }
            let h_start = tsc::read_tsc();
            let h_timeout = tsc_freq; // 1 second
            loop {
                let sts = mmio::read32(op_base + OP_USBSTS);
                if sts & STS_HCH != 0 {
                    break;
                }
                if tsc::read_tsc().wrapping_sub(h_start) > h_timeout {
                    return Err(XhciError::ResetFailed);
                }
                core::hint::spin_loop();
            }
        }

        // scratchpad buffers
        if n_scratch > dma::MAX_SCRATCH {
            return Err(XhciError::ScratchpadUnsupported);
        }
        if n_scratch > 0 {
            let arr = dma_base + dma::OFF_SCRATCH_ARR as u64;
            for i in 0..n_scratch {
                let pg = dma_base + (dma::OFF_SCRATCH_PG + i * 4096) as u64;
                vw64(arr + (i as u64) * 8, pg);
            }
            vw64(dma_base + dma::OFF_DCBAA as u64, arr);
        }

        // Configure register — enable the device slots we want to use.
        // xHCI spec §4.3.3: MaxSlotsEnabled must be programmed before any
        // ENABLE_SLOT command. UEFI may have left a non-zero value here, but
        // we don't rely on that — write it ourselves so behaviour matches
        // across firmwares and reboots. Bits [7:0] = MaxSlotsEn; preserve the
        // upper bits (U3E, CIE, etc.) in case firmware set them.
        {
            let cur = mmio::read32(op_base + OP_CONFIG);
            mmio::write32(op_base + OP_CONFIG, (cur & !0xFF) | (max_slots as u32));
        }

        // DCBAAP
        let dcbaa = dma_base + dma::OFF_DCBAA as u64;
        mmio::write32(op_base + OP_DCBAAP, dcbaa as u32);
        mmio::write32(op_base + OP_DCBAAP + 4, (dcbaa >> 32) as u32);

        // command ring
        let cr = dma_base + dma::OFF_CMD_RING as u64;
        mmio::write32(op_base + OP_CRCR, (cr as u32 & !0x3F) | 1);
        mmio::write32(op_base + OP_CRCR + 4, (cr >> 32) as u32);

        // event ring
        let er = dma_base + dma::OFF_EVT_RING as u64;
        let erst = dma_base + dma::OFF_ERST as u64;
        vw32(erst, er as u32);
        vw32(erst + 4, (er >> 32) as u32);
        vw32(erst + 8, EVT_RING_LEN as u32);
        vw32(erst + 12, 0);

        mmio::write32(rt_base + RT_IR0_ERSTSZ, 1);
        mmio::write32(rt_base + RT_IR0_ERDP, (er as u32 & !0xF) | 0x08);
        mmio::write32(rt_base + RT_IR0_ERDP + 4, (er >> 32) as u32);
        mmio::write32(rt_base + RT_IR0_ERSTBA, erst as u32);
        mmio::write32(rt_base + RT_IR0_ERSTBA + 4, (erst >> 32) as u32);

        // IMAN.IE
        let iman = mmio::read32(rt_base + RT_IR0_IMAN);
        mmio::write32(rt_base + RT_IR0_IMAN, iman | 0x02);

        // start controller
        mmio::write32(op_base + OP_USBCMD, CMD_RS | CMD_INTE);

        // wait HCH to clear
        let start = tsc::read_tsc();
        let timeout = tsc_freq;
        loop {
            let sts = mmio::read32(op_base + OP_USBSTS);
            if sts & STS_HCH == 0 {
                break;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(XhciError::StartFailed);
            }
            core::hint::spin_loop();
        }

        // clear stale change bits on all ports
        for p in 0..max_ports {
            let addr = op_base + PORT_REG_BASE + (p as u64) * PORT_REG_STRIDE;
            let ps = mmio::read32(addr);
            let clr = ps & PORTSC_RW1C;
            if clr != 0 {
                Self::portsc_write(addr, ps, clr, 0);
            }
        }

        Self::tsc_delay(tsc_freq, 200);

        // Single-line diagnostic dump — gives the operator everything needed
        // to debug a non-responding controller without reading scrollback.
        // Fields: HCCPARAMS1 / USBCMD / USBSTS / CRCR.lo / DCBAAP.lo /
        //         dma_base / max_slots / max_ports.
        {
            use crate::serial::{puts, puts_dec_u8, puts_hex_u32, puts_hex_u64};
            let cmd_now = mmio::read32(op_base + OP_USBCMD);
            let sts_now = mmio::read32(op_base + OP_USBSTS);
            let crcr_lo = mmio::read32(op_base + OP_CRCR);
            let dcb_lo = mmio::read32(op_base + OP_DCBAAP);
            puts("[USB-DBG] hccp1=");
            puts_hex_u32(hccparams1);
            puts(" cmd=");
            puts_hex_u32(cmd_now);
            puts(" sts=");
            puts_hex_u32(sts_now);
            puts(" crcr=");
            puts_hex_u32(crcr_lo);
            puts(" dcb=");
            puts_hex_u32(dcb_lo);
            puts(" dma=");
            puts_hex_u64(dma_base);
            puts(" slots=");
            puts_dec_u8(max_slots);
            puts(" ports=");
            puts_dec_u8(max_ports);
            puts("\n");
        }

        Ok(Self {
            mmio_base,
            op_base,
            rt_base,
            db_base,
            tsc_freq,
            max_ports,
            ctx_size,
            dma_base,
            slot_id: 0,
            dci_bulk_in: 0,
            dci_bulk_out: 0,
            cmd_ring: CmdRing::new(dma_base + dma::OFF_CMD_RING as u64),
            evt_ring: EvtRing::new(dma_base + dma::OFF_EVT_RING as u64),
            ep0: XferRing::new(dma_base + dma::OFF_XFER_EP0 as u64, XFER_RING_LEN),
            bout: XferRing::new(dma_base + dma::OFF_XFER_BOUT as u64, XFER_RING_LEN),
            bin: XferRing::new(dma_base + dma::OFF_XFER_BIN as u64, XFER_RING_LEN),
        })
    }

    #[inline(always)]
    pub fn portsc(&self, port: u8) -> u64 {
        self.op_base + PORT_REG_BASE + (port as u64) * PORT_REG_STRIDE
    }

    /// Write PORTSC preserving RO+RWS bits.
    unsafe fn portsc_write(addr: u64, current: u32, set: u32, clear: u32) {
        const RO: u32 = (1 << 0) | (1 << 3) | (0xF << 10) | (1 << 30);
        const RWS: u32 = (0xF << 5) | (1 << 9) | (0x3 << 14) | (0x7 << 25);
        let v = (current & (RO | RWS)) & !clear | set;
        mmio::write32(addr, v);
    }

    #[inline(always)]
    pub(crate) unsafe fn tsc_delay(tsc_freq: u64, ms: u64) {
        if ms == 0 {
            return;
        }
        let ticks = tsc_freq / 1000 * ms;
        let start = tsc::read_tsc();
        while tsc::read_tsc().wrapping_sub(start) < ticks {
            core::hint::spin_loop();
        }
    }

    /// Instance wrapper around `tsc_delay` for callers that already hold a
    /// controller reference (mostly the hub class code in `hub.rs`).
    #[inline(always)]
    pub unsafe fn delay_ms(&self, ms: u64) {
        Self::tsc_delay(self.tsc_freq, ms);
    }

    /// Single-line snapshot of the controller's run-state registers.
    /// Tag is a short identifier so multiple dumps in a boot can be distinguished.
    pub unsafe fn dump_state(&self, tag: &str) {
        use crate::serial::{puts, puts_hex_u32};
        let cmd = mmio::read32(self.op_base + OP_USBCMD);
        let sts = mmio::read32(self.op_base + OP_USBSTS);
        let crcr = mmio::read32(self.op_base + OP_CRCR);
        let dcb = mmio::read32(self.op_base + OP_DCBAAP);
        puts("[USB-DBG] ");
        puts(tag);
        puts(" cmd=");
        puts_hex_u32(cmd);
        puts(" sts=");
        puts_hex_u32(sts);
        puts(" crcr=");
        puts_hex_u32(crcr);
        puts(" dcb=");
        puts_hex_u32(dcb);
        puts("\n");
    }

    /// Ring the command doorbell.
    #[inline(always)]
    pub unsafe fn ring_cmd_doorbell(&self) {
        mmio::write32(self.db_base, 0);
    }

    /// Ring a transfer doorbell for slot/ep.
    #[inline(always)]
    pub unsafe fn ring_xfer_doorbell(&self, ep_dci: u32) {
        mmio::write32(self.db_base + (self.slot_id as u64) * 4, ep_dci);
    }

    /// Wait for a command completion event. Returns (slot_id, completion_code).
    ///
    /// Drains every event TRB it sees, returning only when one matches
    /// `TRB_CMD_COMPLETE`. This is required on real silicon, where port reset
    /// posts multiple `Port Status Change Event` TRBs that arrive in the
    /// ring ahead of the command completion — without draining them, we
    /// would spin on the first PSCEC forever.
    pub unsafe fn wait_cmd(&mut self, timeout_ms: u64) -> Result<(u8, u8), XhciError> {
        let start = tsc::read_tsc();
        let timeout = self.tsc_freq.saturating_mul(timeout_ms) / 1000;
        loop {
            if let Some((_, status, ctrl)) = self.evt_ring.peek() {
                let ty = ctrl & Self::TYPE_MASK;
                self.evt_ring.advance();
                if ty == TRB_CMD_COMPLETE {
                    self.update_erdp();
                    let cc = (status >> 24) as u8;
                    let sid = (ctrl >> 24) as u8;
                    if cc != 1 {
                        return Err(XhciError::IoError);
                    }
                    return Ok((sid, cc));
                }
                // non-target event (PSCEC, transfer, etc.) — drained, keep looking
                continue;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(XhciError::CommandTimeout);
            }
            core::hint::spin_loop();
        }
    }

    /// Wait for a transfer event matching the slot/ep. Returns remaining byte count.
    ///
    /// Same draining discipline as [`wait_cmd`] — advances past every event,
    /// returns only on a `TRB_TRANSFER_EVENT` for the requested slot.
    pub unsafe fn wait_xfer(
        &mut self,
        slot_id: u8,
        ep_dci: u32,
        timeout_ms: u64,
    ) -> Result<u32, XhciError> {
        let _ = ep_dci;
        let start = tsc::read_tsc();
        let timeout = self.tsc_freq.saturating_mul(timeout_ms) / 1000;
        loop {
            if let Some((_, status, ctrl)) = self.evt_ring.peek() {
                let ty = ctrl & Self::TYPE_MASK;
                let sid = (ctrl >> 24) as u8;
                self.evt_ring.advance();
                if ty == TRB_TRANSFER_EVENT && sid == slot_id {
                    self.update_erdp();
                    let cc = (status >> 24) as u8;
                    if cc != 1 && cc != 13 {
                        return Err(XhciError::IoError);
                    }
                    return Ok(status & 0x00FF_FFFF);
                }
                // unrelated event — drained, keep looking
                continue;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(XhciError::CommandTimeout);
            }
            core::hint::spin_loop();
        }
    }

    /// Update ERDP after processing events.
    pub unsafe fn update_erdp(&mut self) {
        let new_erdp = self.evt_ring.base + (self.evt_ring.deq as u64) * 16;
        mmio::write32(self.rt_base + RT_IR0_ERDP, (new_erdp as u32 & !0xF) | 0x08);
        mmio::write32(self.rt_base + RT_IR0_ERDP + 4, (new_erdp >> 32) as u32);
    }

    const TYPE_MASK: u32 = 0x3F << 10;

    /// Reset a port. Returns detected link speed (1=FS, 2=LS, 3=HS, 4=SS).
    pub unsafe fn port_reset(&self, port: u8) -> Result<u8, XhciError> {
        let addr = self.portsc(port);

        // ensure port power
        let ps = mmio::read32(addr);
        if ps & PORTSC_PP == 0 {
            Self::portsc_write(addr, ps, PORTSC_PP, 0);
            Self::tsc_delay(self.tsc_freq, 10);
        }

        // check for any link indicators
        let pre = mmio::read32(addr);
        let pre_speed = ((pre >> PORTSC_SPEED_SHIFT) & 0xF) as u8;
        if (pre & PORTSC_CCS) == 0 && (pre & PORTSC_PED) == 0 && pre_speed == 0 {
            return Err(XhciError::PortResetNoLink);
        }

        // warm reset for stuck SS links
        let pls = pre & PORTSC_PLS_MASK;
        if pre_speed >= 4 && (pls == PLS_U3 || pls == PLS_RECOVERY || pls == PLS_RESUME) {
            Self::portsc_write(addr, pre, PORTSC_LWS | PLS_U0, PORTSC_PLS_MASK);
        }

        // assert PR
        Self::portsc_write(addr, mmio::read32(addr), PORTSC_PR, 0);

        // wait for reset to complete
        let start = tsc::read_tsc();
        let timeout = self.tsc_freq / 5;
        loop {
            let psn = mmio::read32(addr);
            let clr = psn & PORTSC_RW1C;
            if clr != 0 {
                Self::portsc_write(addr, psn, clr, 0);
            }
            if psn & PORTSC_PR == 0 || (psn & PORTSC_PRC) != 0 {
                break;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(XhciError::PortResetTimeout);
            }
            core::hint::spin_loop();
        }

        // settle: wait for PED or CCS+speed
        let start2 = tsc::read_tsc();
        loop {
            let psn = mmio::read32(addr);
            let clr = psn & PORTSC_RW1C;
            if clr != 0 {
                Self::portsc_write(addr, psn, clr, 0);
            }
            let speed = ((psn >> PORTSC_SPEED_SHIFT) & 0xF) as u8;
            if psn & PORTSC_PED != 0 || ((psn & PORTSC_CCS) != 0 && speed != 0) {
                return Ok(speed);
            }
            if tsc::read_tsc().wrapping_sub(start2) > timeout {
                return Err(XhciError::PortResetTimeout);
            }
            core::hint::spin_loop();
        }
    }
}
