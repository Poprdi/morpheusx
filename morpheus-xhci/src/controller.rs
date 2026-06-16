//! xHCI controller bring-up: probe, soft-restart, BIOS handoff, port management.

use crate::asm;
use crate::dma;
use crate::dma::{EVT_RING_LEN, XFER_RING_LEN};
use crate::regs::*;
use crate::rings::{vw32, vw64, CmdRing, EvtRing, XferRing};
use morpheus_x86_asm::mmio;
use morpheus_x86_asm::tsc;

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
    pub mouse_ring: XferRing,
    // Last non-success completion code from wait_cmd/wait_xfer. Load-bearing:
    // control_in/control_nodata read this (== 6 STALL) to drive EP0 stall
    // recovery + retry. Not just diagnostics — do not remove.
    pub last_cc: u8,
}

impl XhciController {
    /// # Safety
    /// `mmio_base` must be the valid, mapped MMIO base of an xHCI controller
    /// and the caller must have exclusive control of that controller.
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
        // `slot_out_ctx_offset(20)` would silently index past the array.
        let enabled = (max_slots as u32).min(dma::MAX_OUT_CTX_SLOTS as u32);
        {
            let cur = mmio::read32(op_base + OP_CONFIG);
            mmio::write32(op_base + OP_CONFIG, (cur & !0xFF) | enabled);
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

        let mut this = Self {
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
            mouse_ring: XferRing::new(dma_base + dma::OFF_XFER_MOUSE as u64, XFER_RING_LEN),
            last_cc: 0,
        };

        // Release any slots a prior controller owner left bound to a port.
        // Because bring-up deliberately skips HCRST (above), the xHC retains
        // the slot→port bindings UEFI established while enumerating USB input
        // before ExitBootServices. With those intact, the kernel's fresh
        // enumeration lands on a NEW slot id and tries to address a port that
        // a stale firmware slot still owns — which the controller rejects
        // (CC_TRB_ERROR on QEMU's xHC; a stale-context hazard on real silicon).
        // Disabling them frees the ports without touching PORTSC, so connected
        // link state survives for the later storage probe.
        this.disable_inherited_slots(enabled as u8);

        Ok(this)
    }

    /// Disable every device slot the Config register enables, releasing any
    /// slot→port bindings inherited from a prior controller owner (UEFI
    /// firmware, or an earlier kernel controller instance that did not assert
    /// HCRST on handoff). xHCI permits at most one Device Slot per root-hub
    /// port, so a leftover firmware slot must be freed before the kernel can
    /// address the same device on a fresh slot.
    ///
    /// Best-effort: a DISABLE_SLOT for a slot that was never allocated returns
    /// CC_SLOT_NOT_ENABLED, which is expected and ignored. The command never
    /// reads or writes PORTSC, so port power and link state are preserved.
    ///
    /// # Safety
    /// The command/event rings must be live (controller started) and the caller
    /// must hold exclusive access.
    unsafe fn disable_inherited_slots(&mut self, max_slot: u8) {
        for slot in 1..=max_slot {
            self.cmd_ring
                .enqueue(0, 0, TRB_DISABLE_SLOT | ((slot as u32) << 24));
            self.ring_cmd_doorbell();
            // Each DISABLE_SLOT posts a completion event promptly (success or
            // CC_SLOT_NOT_ENABLED); a 500 ms cap guards against a wedged ring
            // without stalling boot across the (≤16) slots.
            let _ = self.wait_cmd(500);
        }
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

    /// Quiesce the controller so it can be dropped and re-`new`'d safely.
    ///
    /// Phase 9 of `hwinit` brings up the xHC for HID enumeration and then
    /// drops the `XhciController`. The bootloader's Stage D2 storage probe
    /// then constructs a *fresh* `XhciController` against the same physical
    /// device. Without an explicit handoff step, real hardware may keep the
    /// controller running with stale DCBAAP/CRCR/ERST pointers from the
    /// first instance — leading to silent ring-pointer mismatches on the
    /// second bring-up.
    ///
    /// Sequence (xHCI 1.2 §4.2 stop sequence):
    /// 1. Set CRCR.CS=1 (Command Stop). Wait for CRR=0.
    /// 2. Drain the event ring; ack the interrupter (IP=1 W1C) and write a
    ///    fresh ERDP so it doesn't fill the segment.
    /// 3. Clear CMD_RS; wait for STS_HCH (controller halted).
    ///
    /// HCRST is intentionally NOT asserted — port state from UEFI / Phase 9
    /// must persist into Stage D2 (see [[usb-xhci-controller]]).
    ///
    /// # Safety
    /// The controller's MMIO base addresses must be valid and mapped, and the
    /// caller must hold exclusive access to the controller.
    pub unsafe fn quiesce(&mut self) {
        let timeout = self.tsc_freq; // 1 second cap on every wait
        let cs_bit: u64 = 1 << 1; // CRCR.CS (Command Stop)
        let crr_bit: u64 = 1 << 3; // CRCR.CRR (Command Ring Running)

        // ── 1. Stop the command ring ──
        // CRCR is 64-bit register; only the low dword carries CS/CRR.
        let cur_lo = mmio::read32(self.op_base + OP_CRCR);
        mmio::write32(self.op_base + OP_CRCR, cur_lo | cs_bit as u32);
        let start = tsc::read_tsc();
        loop {
            let lo = mmio::read32(self.op_base + OP_CRCR);
            if (lo as u64 & crr_bit) == 0 {
                break;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                break; // best-effort — proceed with halt anyway
            }
            core::hint::spin_loop();
        }

        // ── 2. Drain the event ring + ack interrupter ──
        // Walk every queued event so the controller doesn't consider the
        // ring full on the next bring-up.
        while self.evt_ring.peek().is_some() {
            self.evt_ring.advance();
        }
        self.update_erdp();
        // W1C the IP (Interrupt Pending) bit on IR0; preserve IE.
        let iman = mmio::read32(self.rt_base + RT_IR0_IMAN);
        mmio::write32(self.rt_base + RT_IR0_IMAN, iman | 0x01);

        // ── 3. Halt the controller ──
        let cmd = mmio::read32(self.op_base + OP_USBCMD);
        mmio::write32(self.op_base + OP_USBCMD, cmd & !CMD_RS);
        let start = tsc::read_tsc();
        loop {
            let sts = mmio::read32(self.op_base + OP_USBSTS);
            if sts & STS_HCH != 0 {
                break;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                break;
            }
            core::hint::spin_loop();
        }
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
    ///
    /// # Safety
    /// `self.tsc_freq` must reflect the calibrated TSC frequency; reads the TSC.
    #[inline(always)]
    pub unsafe fn delay_ms(&self, ms: u64) {
        Self::tsc_delay(self.tsc_freq, ms);
    }

    #[inline(always)]
    /// Issue a `RESET_ENDPOINT` command (xHCI TRB type 14).
    ///
    /// Required to clear a Halted endpoint — caused on real hardware by the
    /// controller hitting CErr USB errors during polling (timeouts, STALLs,
    /// LS-on-HS TT glitches). The endpoint stays Halted until reset; new
    /// doorbell rings are ignored. After this completes the endpoint is in
    /// Stopped state, and the TR Dequeue Pointer is left wherever the
    /// controller halted it. Pair with `set_tr_dequeue_pointer` to restart
    /// from a known position.
    ///
    /// # Safety
    /// The controller's MMIO/command-ring state must be valid and the caller
    /// must hold exclusive access; `slot_id`/`ep_dci` must name a real endpoint.
    pub unsafe fn reset_endpoint(&mut self, slot_id: u8, ep_dci: u32) -> Result<(), XhciError> {
        const TRB_RESET_ENDPOINT: u32 = 14u32 << 10;
        let ctrl = TRB_RESET_ENDPOINT | ((ep_dci & 0x1F) << 16) | ((slot_id as u32) << 24);
        self.cmd_ring.enqueue(0, 0, ctrl);
        self.ring_cmd_doorbell();
        self.wait_cmd(2000)?;
        Ok(())
    }

    /// Issue a `SET_TR_DEQUEUE_POINTER` command (xHCI TRB type 16).
    ///
    /// Tells the controller where to read the next TRB for the given
    /// endpoint. The low bit of `deq_ptr` is DCS (Dequeue Cycle State);
    /// bits 3:1 are reserved; bits [63:4] are the 16-byte-aligned ring address.
    ///
    /// # Safety
    /// The controller's MMIO/command-ring state must be valid and the caller
    /// must hold exclusive access; `deq_ptr` must point into the endpoint's ring.
    pub unsafe fn set_tr_dequeue_pointer(
        &mut self,
        slot_id: u8,
        ep_dci: u32,
        deq_ptr: u64,
    ) -> Result<(), XhciError> {
        const TRB_SET_TR_DEQ: u32 = 16u32 << 10;
        let ctrl = TRB_SET_TR_DEQ | ((ep_dci & 0x1F) << 16) | ((slot_id as u32) << 24);
        self.cmd_ring.enqueue(deq_ptr, 0, ctrl);
        self.ring_cmd_doorbell();
        self.wait_cmd(2000)?;
        Ok(())
    }

    /// # Safety
    /// `self.db_base` must be the valid, mapped doorbell array base.
    pub unsafe fn ring_cmd_doorbell(&self) {
        mmio::write32(self.db_base, 0);
    }

    /// # Safety
    /// `self.db_base`/`self.slot_id` must address a valid doorbell register.
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
    ///
    /// # Safety
    /// The controller's event ring and MMIO base must be valid; the caller must
    /// hold exclusive access while the ring is drained.
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
                        self.last_cc = cc;
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
    ///
    /// # Safety
    /// The controller's event ring and MMIO base must be valid; the caller must
    /// hold exclusive access while the ring is drained.
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
                        self.last_cc = cc;
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

    /// # Safety
    /// The controller's event ring and MMIO base must be valid; the caller must
    /// hold exclusive access while the ring is drained.
    pub unsafe fn poll_xfer_event(&mut self) -> Option<(u8, u32, u32)> {
        while let Some((_, status, ctrl)) = self.evt_ring.peek() {
            let ty = ctrl & Self::TYPE_MASK;
            let sid = (ctrl >> 24) as u8;
            let dci = (ctrl >> 16) & 0x1F;
            self.evt_ring.advance();
            self.update_erdp();
            if ty == TRB_TRANSFER_EVENT {
                return Some((sid, dci, status & 0x00FF_FFFF));
            }
        }
        None
    }

    /// # Safety
    /// `self.rt_base` must be the valid, mapped runtime register base and
    /// `self.evt_ring` must describe the live event ring.
    pub unsafe fn update_erdp(&mut self) {
        let new_erdp = self.evt_ring.base + (self.evt_ring.deq as u64) * 16;
        mmio::write32(self.rt_base + RT_IR0_ERDP, (new_erdp as u32 & !0xF) | 0x08);
        mmio::write32(self.rt_base + RT_IR0_ERDP + 4, (new_erdp >> 32) as u32);
    }

    const TYPE_MASK: u32 = 0x3F << 10;

    /// Reset a port. Returns detected link speed (1=FS, 2=LS, 3=HS, 4=SS).
    ///
    /// # Safety
    /// The controller's MMIO base must be valid and mapped; `port` must be a
    /// real root-hub port index.
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
                // 10 ms delay is should be sufficient for the device to recover from reset and be ready for the next command, 
                // Dont remove it or you will debug the same issue as i did for hours :) 
                Self::tsc_delay(self.tsc_freq, 10);
                return Ok(speed);
            }
            if tsc::read_tsc().wrapping_sub(start2) > timeout {
                return Err(XhciError::PortResetTimeout);
            }
            core::hint::spin_loop();
        }
    }
}
