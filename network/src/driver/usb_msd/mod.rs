//! USB mass-storage block driver — xHCI host + BOT transport + SCSI read.
//!
//! Finds the first USB mass storage device on the xHCI bus, initialises it
//! via Bulk-Only Transport, and exposes SCSI READ(10) through BlockDriver.
//! Read-only; write returns Unsupported.

use crate::asm::core::mmio;
use crate::asm::core::tsc;
use crate::driver::block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};

// ═══════════════════════════════════════════════════════════════════════════
// ASM externs
// ═══════════════════════════════════════════════════════════════════════════

extern "win64" {
    /// Reads CAPLENGTH + HCIVERSION.  0 = dead controller.
    /// Low byte = CAPLENGTH, bits 31:16 = HCIVERSION.
    fn asm_usb_host_probe(mmio_base: u64) -> u32;
    /// Stop + HCRST + wait CNR.  0 = ok, 1/2/3 = timeout at halt/reset/cnr.
    fn asm_xhci_controller_reset(op_base: u64, tsc_freq: u64) -> u32;
    /// Walk extended caps, claim ownership from BIOS/SMM.  0 = ok.
    fn asm_xhci_bios_handoff(mmio_base: u64, hccparams1: u64, tsc_freq: u64) -> u32;
}

// ═══════════════════════════════════════════════════════════════════════════
// xHCI register offsets (operational, from op_base)
// ═══════════════════════════════════════════════════════════════════════════

const OP_USBCMD: u64 = 0x00;
const OP_USBSTS: u64 = 0x04;
const OP_CRCR: u64 = 0x18;
const OP_DCBAAP: u64 = 0x30;
const OP_CONFIG: u64 = 0x38;

const PORT_REG_BASE: u64 = 0x400;
const PORT_REG_STRIDE: u64 = 0x10;

// interrupter 0 offsets from rt_base
const IR0_IMAN: u64 = 0x20;
const IR0_IMOD: u64 = 0x24;
const IR0_ERSTSZ: u64 = 0x28;
const IR0_ERSTBA: u64 = 0x30;
const IR0_ERDP: u64 = 0x38;

// USBCMD / USBSTS bits
const CMD_RS: u32 = 1 << 0;
const CMD_INTE: u32 = 1 << 2;
const STS_HCH: u32 = 1 << 0;

// PORTSC bits
const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_PP: u32 = 1 << 9;
const PORTSC_PRC: u32 = 1 << 21;
// RW1C mask: bits 17-23 must be written 0 to preserve, 1 to clear
const PORTSC_RW1C: u32 = 0x00FE_0000;
const PORTSC_SPEED_SHIFT: u32 = 10;

// ═══════════════════════════════════════════════════════════════════════════
// TRB types (pre-shifted to bits 15:10)
// ═══════════════════════════════════════════════════════════════════════════

const TRB_NORMAL: u32 = 1 << 10;
const TRB_SETUP: u32 = 2 << 10;
const TRB_DATA: u32 = 3 << 10;
const TRB_STATUS: u32 = 4 << 10;
const TRB_LINK: u32 = 6 << 10;
const TRB_ENABLE_SLOT: u32 = 9 << 10;
const TRB_DISABLE_SLOT: u32 = 10 << 10;
const TRB_ADDRESS_DEV: u32 = 11 << 10;
const TRB_CONFIGURE_EP: u32 = 12 << 10;
const TRB_XFER_EVENT: u32 = 32 << 10;
const TRB_CMD_COMPLETE: u32 = 33 << 10;

// TRB control bits
const TRB_TC: u32 = 1 << 1;
const TRB_ISP: u32 = 1 << 2;
const TRB_IOC: u32 = 1 << 5;
const TRB_IDT: u32 = 1 << 6;
const TRB_DIR_IN: u32 = 1 << 16;
const TRB_TRT_IN: u32 = 3 << 16;

const TRB_TYPE_MASK: u32 = 0x3F << 10;

// ═══════════════════════════════════════════════════════════════════════════
// USB / SCSI / BOT constants
// ═══════════════════════════════════════════════════════════════════════════

const USB_CLASS_MASS_STORAGE: u8 = 0x08;
const USB_SUBCLASS_SCSI: u8 = 0x06;
const USB_PROTOCOL_BOT: u8 = 0x50;

const CBW_SIG: u32 = 0x4342_5355;
const CSW_SIG: u32 = 0x5342_5355;
const SCSI_TEST_UNIT_READY: u8 = 0x00;
const SCSI_READ_CAPACITY_10: u8 = 0x25;
const SCSI_READ_10: u8 = 0x28;

// ═══════════════════════════════════════════════════════════════════════════
// DMA region layout — all offsets 64-byte aligned inside a 64KB-aligned buf
// ═══════════════════════════════════════════════════════════════════════════

const DMA_SIZE: usize = 65536;
const CMD_RING_LEN: u8 = 32;
const EVT_RING_LEN: u8 = 32;
const XFER_RING_LEN: u8 = 16;

const OFF_DCBAA: usize = 0x0000; // 2KB
const OFF_CMD_RING: usize = 0x1000; // 512B
const OFF_EVT_RING: usize = 0x1200; // 512B
const OFF_ERST: usize = 0x1400; // 16B
const OFF_OUT_CTX: usize = 0x2000; // 2KB (supports CSZ=1)
const OFF_IN_CTX: usize = 0x3000; // 2.5KB
const OFF_XFER_EP0: usize = 0x4000; // 256B
const OFF_XFER_BOUT: usize = 0x4100; // 256B
const OFF_XFER_BIN: usize = 0x4200; // 256B
const OFF_CBW: usize = 0x4400; // 64B
const OFF_CSW: usize = 0x4440; // 64B
const OFF_DESC: usize = 0x4480; // 256B
const OFF_DATA: usize = 0x5000; // 4KB sector bounce buffer (one page, no 64KB boundary crossing)
const DATA_BUF_SIZE: usize = 4096;
const OFF_SCRATCH_ARR: usize = 0x7000; // 64B
const OFF_SCRATCH_PG: usize = 0x8000; // up to 8 × 4KB pages
const MAX_SCRATCH: usize = 8;

#[repr(C, align(4096))]
struct XhciDma([u8; DMA_SIZE]);

static mut XHCI_DMA: XhciDma = XhciDma([0u8; DMA_SIZE]);

// ═══════════════════════════════════════════════════════════════════════════
// Public types (kept wire-compatible with scaffold)
// ═══════════════════════════════════════════════════════════════════════════

/// USB mass-storage configuration.
#[derive(Debug, Clone)]
pub struct UsbMsdConfig {
    /// TSC frequency for timeout calculations.
    pub tsc_freq: u64,
    /// Optional DMA bounce buffer base (physical).
    pub dma_phys: u64,
    /// Optional DMA bounce buffer size.
    pub dma_size: usize,
}

/// USB mass-storage init errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbMsdInitError {
    InvalidConfig,
    ControllerInitFailed,
    DeviceEnumerationFailed,
    TransportInitFailed,
    NoMedia,
    CommandTimeout,
    IoError,
    NotImplemented,
}

impl core::fmt::Display for UsbMsdInitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidConfig => write!(f, "Invalid USB MSD configuration"),
            Self::ControllerInitFailed => write!(f, "USB xHCI controller init failed"),
            Self::DeviceEnumerationFailed => write!(f, "USB device enumeration failed"),
            Self::TransportInitFailed => write!(f, "USB BOT transport init failed"),
            Self::NoMedia => write!(f, "No USB mass-storage device found"),
            Self::CommandTimeout => write!(f, "USB command timeout"),
            Self::IoError => write!(f, "USB I/O error"),
            Self::NotImplemented => write!(f, "USB mass-storage driver not implemented yet"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Transfer ring identifier
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
enum Ring {
    Ep0,
    BulkOut,
    BulkIn,
}

// ═══════════════════════════════════════════════════════════════════════════
// Driver state
// ═══════════════════════════════════════════════════════════════════════════

pub struct UsbMsdDriver {
    // xHCI register bases
    op_base: u64,
    rt_base: u64,
    db_base: u64,
    tsc_freq: u64,
    max_ports: u8,
    ctx_size: u8, // 32 or 64

    dma_base: u64,

    // ring producer/consumer state
    cmd_enq: u8,
    cmd_cycle: u8,
    evt_deq: u8,
    evt_cycle: u8,
    ep0_enq: u8,
    ep0_cycle: u8,
    bout_enq: u8,
    bout_cycle: u8,
    bin_enq: u8,
    bin_cycle: u8,

    // USB device
    slot_id: u8,
    dci_bulk_in: u8,
    dci_bulk_out: u8,

    info: BlockDeviceInfo,
    last_completion: Option<BlockCompletion>,
    bot_tag: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// Volatile helpers (DMA RAM, NOT mmio)
// ═══════════════════════════════════════════════════════════════════════════

#[inline(always)]
unsafe fn vr32(a: u64) -> u32 {
    core::ptr::read_volatile(a as *const u32)
}
#[inline(always)]
unsafe fn vw32(a: u64, v: u32) {
    core::ptr::write_volatile(a as *mut u32, v);
}
#[inline(always)]
unsafe fn vw64(a: u64, v: u64) {
    vw32(a, v as u32);
    vw32(a + 4, (v >> 32) as u32);
}

/// Write a TRB at `base + idx*16`. Control word (with cycle bit) written last.
#[inline(always)]
unsafe fn write_trb(base: u64, idx: usize, param: u64, status: u32, ctrl: u32) {
    let a = base + (idx as u64) * 16;
    vw32(a, param as u32);
    vw32(a + 4, (param >> 32) as u32);
    vw32(a + 8, status);
    vw32(a + 12, ctrl);
}

// busy-wait delay using TSC. ms=0 is a no-op.
#[inline(always)]
unsafe fn tsc_delay(tsc_freq: u64, ms: u64) {
    if ms == 0 {
        return;
    }
    let ticks = tsc_freq / 1000 * ms;
    let start = tsc::read_tsc();
    while tsc::read_tsc().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Implementation
// ═══════════════════════════════════════════════════════════════════════════

impl UsbMsdDriver {
    // ─── public entry point ──────────────────────────────────────────────

    /// Initialise xHCI controller, enumerate first USB mass-storage device,
    /// run SCSI READ CAPACITY.  Returns a ready-to-read driver or an error.
    pub unsafe fn new(mmio_base: u64, config: UsbMsdConfig) -> Result<Self, UsbMsdInitError> {
        if mmio_base == 0 || config.tsc_freq == 0 {
            return Err(UsbMsdInitError::InvalidConfig);
        }

        let mut drv = Self::init_controller(mmio_base, config.tsc_freq)?;
        drv.enumerate_and_configure()?;
        drv.scsi_init()?;
        Ok(drv)
    }

    // ─── phase 1: controller bring-up (brutal edition) ─────────────────

    unsafe fn init_controller(mmio_base: u64, tsc_freq: u64) -> Result<Self, UsbMsdInitError> {
        // probe: low byte = CAPLENGTH, high half = HCIVERSION. 0 = dead.
        let probe = asm_usb_host_probe(mmio_base);
        if probe == 0 {
            return Err(UsbMsdInitError::ControllerInitFailed);
        }
        let cap_len = (probe & 0xFF) as u64;
        let op_base = mmio_base + cap_len;

        let hcsparams1 = mmio::read32(mmio_base + 0x04);
        let hcsparams2 = mmio::read32(mmio_base + 0x08);
        let hccparams1 = mmio::read32(mmio_base + 0x10);
        let db_off = mmio::read32(mmio_base + 0x14) & !0x03;
        let rts_off = mmio::read32(mmio_base + 0x18) & !0x1F;

        let max_slots = (hcsparams1 & 0xFF) as u8;
        let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;
        let ctx_size: u8 = if hccparams1 & (1 << 2) != 0 { 64 } else { 32 };
        let scratch_hi = ((hcsparams2 >> 21) & 0x1F) as u16;
        let scratch_lo = ((hcsparams2 >> 27) & 0x1F) as u16;
        let n_scratch = ((scratch_hi << 5) | scratch_lo) as usize;

        let rt_base = mmio_base + rts_off as u64;
        let db_base = mmio_base + db_off as u64;

        // static DMA buffer, identity-mapped in UEFI
        let dma_base = core::ptr::addr_of_mut!(XHCI_DMA) as u64;
        core::ptr::write_bytes(dma_base as *mut u8, 0, DMA_SIZE);

        // ── rip controller from BIOS/UEFI/SMM ──
        asm_xhci_bios_handoff(mmio_base, hccparams1 as u64, tsc_freq);
        tsc_delay(tsc_freq, 10);

        // ── controller reset with 3 attempts because hardware lies ──
        let mut reset_ok = false;
        for attempt in 0..3u32 {
            let rc = asm_xhci_controller_reset(op_base, tsc_freq);
            if rc == 0 {
                reset_ok = true;
                break;
            }
            // increasing backoff: 100ms, 200ms, 300ms
            tsc_delay(tsc_freq, 100 * (attempt as u64 + 1));
        }
        if !reset_ok {
            return Err(UsbMsdInitError::ControllerInitFailed);
        }

        // post-reset settle
        tsc_delay(tsc_freq, 50);

        // MaxSlotsEn
        mmio::write32(op_base + OP_CONFIG, max_slots.min(16) as u32);

        // scratchpad buffers (controller refuses to start without them)
        if n_scratch > MAX_SCRATCH {
            return Err(UsbMsdInitError::ControllerInitFailed);
        }
        if n_scratch > 0 {
            let arr = dma_base + OFF_SCRATCH_ARR as u64;
            for i in 0..n_scratch {
                let pg = dma_base + (OFF_SCRATCH_PG + i * 4096) as u64;
                vw64(arr + (i as u64) * 8, pg);
            }
            // DCBAA[0] = scratchpad buffer array
            vw64(dma_base + OFF_DCBAA as u64, arr);
        }

        // DCBAAP
        let dcbaa = dma_base + OFF_DCBAA as u64;
        mmio::write32(op_base + OP_DCBAAP, dcbaa as u32);
        mmio::write32(op_base + OP_DCBAAP + 4, (dcbaa >> 32) as u32);

        // command ring → CRCR (RCS = 1)
        let cr = dma_base + OFF_CMD_RING as u64;
        mmio::write32(op_base + OP_CRCR, (cr as u32 & !0x3F) | 1);
        mmio::write32(op_base + OP_CRCR + 4, (cr >> 32) as u32);

        // event ring: ERST entry, then registers
        let er = dma_base + OFF_EVT_RING as u64;
        let erst = dma_base + OFF_ERST as u64;
        vw32(erst, er as u32);
        vw32(erst + 4, (er >> 32) as u32);
        vw32(erst + 8, EVT_RING_LEN as u32);
        vw32(erst + 12, 0);

        mmio::write32(rt_base + IR0_ERSTSZ, 1);
        // ERDP must be written before ERSTBA per spec
        mmio::write32(rt_base + IR0_ERDP, (er as u32 & !0xF) | 0x08);
        mmio::write32(rt_base + IR0_ERDP + 4, (er >> 32) as u32);
        mmio::write32(rt_base + IR0_ERSTBA, erst as u32);
        mmio::write32(rt_base + IR0_ERSTBA + 4, (erst >> 32) as u32);

        // IMAN.IE — some controllers gate event generation on this
        let iman = mmio::read32(rt_base + IR0_IMAN);
        mmio::write32(rt_base + IR0_IMAN, iman | 0x02);

        // start controller: RS=1, INTE=1
        mmio::write32(op_base + OP_USBCMD, CMD_RS | CMD_INTE);

        // wait HCH to clear (controller running) — 1 second
        let start = tsc::read_tsc();
        let timeout = tsc_freq;
        loop {
            if mmio::read32(op_base + OP_USBSTS) & STS_HCH == 0 {
                break;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(UsbMsdInitError::ControllerInitFailed);
            }
            core::hint::spin_loop();
        }

        // ── port power cycle: turn off, wait, turn on, wait ──
        for p in 0..max_ports {
            let addr = op_base + PORT_REG_BASE + (p as u64) * PORT_REG_STRIDE;
            let ps = mmio::read32(addr);
            mmio::write32(addr, (ps & !PORTSC_RW1C) & !PORTSC_PP);
        }
        tsc_delay(tsc_freq, 50);
        for p in 0..max_ports {
            let addr = op_base + PORT_REG_BASE + (p as u64) * PORT_REG_STRIDE;
            let ps = mmio::read32(addr);
            mmio::write32(addr, (ps & !PORTSC_RW1C) | PORTSC_PP);
        }
        // let devices settle after power-on
        tsc_delay(tsc_freq, 200);

        Ok(Self {
            op_base,
            rt_base,
            db_base,
            tsc_freq,
            max_ports,
            ctx_size,
            dma_base,
            cmd_enq: 0,
            cmd_cycle: 1,
            evt_deq: 0,
            evt_cycle: 1,
            ep0_enq: 0,
            ep0_cycle: 1,
            bout_enq: 0,
            bout_cycle: 1,
            bin_enq: 0,
            bin_cycle: 1,
            slot_id: 0,
            dci_bulk_in: 0,
            dci_bulk_out: 0,
            info: BlockDeviceInfo {
                total_sectors: 0,
                sector_size: 512,
                max_sectors_per_request: 8,
                read_only: true,
            },
            last_completion: None,
            bot_tag: 1,
        })
    }

    // ─── phase 2: USB device enumeration ─────────────────────────────────

    unsafe fn enumerate_and_configure(&mut self) -> Result<(), UsbMsdInitError> {
        for port in 0..self.max_ports {
            let ps = mmio::read32(self.portsc(port));
            if ps & PORTSC_CCS == 0 {
                continue;
            }
            if ps & PORTSC_PP == 0 {
                continue;
            }

            // settle delay before touching connected device
            tsc_delay(self.tsc_freq, 100);
            self.reset_transfer_state();

            match self.try_port(port) {
                Ok(()) => return Ok(()),
                Err(_) => continue,
            }
        }
        Err(UsbMsdInitError::NoMedia)
    }

    unsafe fn try_port(&mut self, port: u8) -> Result<(), UsbMsdInitError> {
        let speed = self.port_reset(port)?;
        let slot = self.cmd_enable_slot()?;
        self.slot_id = slot;

        // wire output context into DCBAA
        let out_ctx = self.dma_base + OFF_OUT_CTX as u64;
        vw64(
            self.dma_base + OFF_DCBAA as u64 + (slot as u64) * 8,
            out_ctx,
        );

        self.cmd_address_device(port, speed)?;

        // GET_DESCRIPTOR device (18 bytes)
        let _dev_desc = self.control_in(0x80, 0x06, 0x0100, 0, 18)?;

        // GET_DESCRIPTOR configuration (up to 255 bytes)
        let _cfg = self.control_in(0x80, 0x06, 0x0200, 0, 255)?;

        // parse for mass-storage interface + bulk endpoints
        let (cfg_val, ep_in, ep_out, mpkt_in, mpkt_out) =
            self.parse_config_desc().ok_or(UsbMsdInitError::DeviceEnumerationFailed)?;

        // SET_CONFIGURATION
        self.control_nodata(0x00, 0x09, cfg_val as u16, 0)?;

        // compute DCIs
        let dci_in = ((ep_in & 0x0F) * 2) + ((ep_in >> 7) & 1);
        let dci_out = ((ep_out & 0x0F) * 2) + ((ep_out >> 7) & 1);
        self.dci_bulk_in = dci_in;
        self.dci_bulk_out = dci_out;

        // configure endpoint command
        self.cmd_configure_eps(dci_in, dci_out, mpkt_in, mpkt_out)?;

        Ok(())
    }

    // ─── phase 3: SCSI bring-up ──────────────────────────────────────────

    unsafe fn scsi_init(&mut self) -> Result<(), UsbMsdInitError> {
        // TEST UNIT READY — absorb unit-attention condition, ignore errors
        let _ = self.bot_command(&[SCSI_TEST_UNIT_READY, 0, 0, 0, 0, 0], 0, false);

        // READ CAPACITY(10) → 8 bytes response
        let cap_cmd = [SCSI_READ_CAPACITY_10, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        self.bot_command(&cap_cmd, 8, true)?;

        let data = self.dma_base + OFF_DATA as u64;
        let last_lba = u32::from_be(vr32(data)) as u64;
        let blk_size = u32::from_be(vr32(data + 4));

        self.info.total_sectors = last_lba + 1;
        self.info.sector_size = if blk_size == 0 { 512 } else { blk_size };
        // max request limited by bounce buffer (4KB page, no 64KB boundary crossing)
        let max_req = DATA_BUF_SIZE as u32 / self.info.sector_size;
        self.info.max_sectors_per_request = max_req.max(1);

        Ok(())
    }

    // ─── port management ─────────────────────────────────────────────────

    #[inline(always)]
    fn portsc(&self, port: u8) -> u64 {
        self.op_base + PORT_REG_BASE + (port as u64) * PORT_REG_STRIDE
    }

    unsafe fn port_reset(&self, port: u8) -> Result<u8, UsbMsdInitError> {
        let addr = self.portsc(port);
        // preserve non-RW1C bits, set PR
        let ps = mmio::read32(addr);
        mmio::write32(addr, (ps & !PORTSC_RW1C) | PORTSC_PR);

        let start = tsc::read_tsc();
        let timeout = self.tsc_freq / 2;
        loop {
            let ps = mmio::read32(addr);
            if ps & PORTSC_PRC != 0 {
                // clear PRC
                mmio::write32(addr, (ps & !PORTSC_RW1C) | PORTSC_PRC);
                if ps & PORTSC_PED != 0 {
                    let speed = ((ps >> PORTSC_SPEED_SHIFT) & 0xF) as u8;
                    return Ok(speed);
                }
                return Err(UsbMsdInitError::DeviceEnumerationFailed);
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(UsbMsdInitError::CommandTimeout);
            }
            core::hint::spin_loop();
        }
    }

    // ─── command ring ────────────────────────────────────────────────────

    unsafe fn cmd_enqueue(&mut self, param: u64, status: u32, ctrl: u32) {
        let base = self.dma_base + OFF_CMD_RING as u64;
        let c = (ctrl & !1) | (self.cmd_cycle as u32);
        write_trb(base, self.cmd_enq as usize, param, status, c);
        self.cmd_enq += 1;
        if self.cmd_enq >= CMD_RING_LEN - 1 {
            // link TRB wrapping back to start
            let link_ctrl = TRB_LINK | TRB_TC | (self.cmd_cycle as u32);
            write_trb(base, self.cmd_enq as usize, base, 0, link_ctrl);
            self.cmd_enq = 0;
            self.cmd_cycle ^= 1;
        }
    }

    #[inline(always)]
    unsafe fn ring_cmd_doorbell(&self) {
        mmio::write32(self.db_base, 0);
    }

    /// Wait for a Command Completion Event.  Returns (completion_code, slot_id).
    unsafe fn wait_cmd(&mut self, timeout_ms: u64) -> Result<(u8, u8), UsbMsdInitError> {
        let (_, status, ctrl) = self.wait_event(TRB_CMD_COMPLETE, timeout_ms)?;
        let cc = (status >> 24) as u8;
        let sid = (ctrl >> 24) as u8;
        if cc != 1 {
            return Err(UsbMsdInitError::IoError);
        }
        Ok((cc, sid))
    }

    unsafe fn cmd_enable_slot(&mut self) -> Result<u8, UsbMsdInitError> {
        self.cmd_enqueue(0, 0, TRB_ENABLE_SLOT);
        self.ring_cmd_doorbell();
        let (_, slot) = self.wait_cmd(2000)?;
        if slot == 0 {
            return Err(UsbMsdInitError::DeviceEnumerationFailed);
        }
        Ok(slot)
    }

    unsafe fn cmd_address_device(
        &mut self,
        port: u8,
        speed: u8,
    ) -> Result<(), UsbMsdInitError> {
        let cs = self.ctx_size as u64;
        let in_ctx = self.dma_base + OFF_IN_CTX as u64;

        // zero input context
        core::ptr::write_bytes(in_ctx as *mut u8, 0, (33 * cs) as usize);

        // input control context: add slot (A0) + EP0 (A1)
        vw32(in_ctx + 4, 0x03);

        // slot context at index 1
        let slot_ctx = in_ctx + cs;
        let max_pkt_ep0 = Self::ep0_max_packet(speed);
        // dword 0: speed | context_entries=1
        vw32(slot_ctx, ((speed as u32) << 20) | (1u32 << 26));
        // dword 1: root hub port (1-based)
        vw32(slot_ctx + 4, (port as u32 + 1) << 16);

        // EP0 context at index 2
        let ep0 = in_ctx + 2 * cs;
        // dword 1: CErr=3, EP type=4 (Control), max packet size
        vw32(ep0 + 4, (3u32 << 1) | (4u32 << 3) | ((max_pkt_ep0 as u32) << 16));
        // dword 2-3: TR dequeue pointer | DCS=1
        let ring_phys = self.dma_base + OFF_XFER_EP0 as u64;
        vw32(ep0 + 8, (ring_phys as u32 & !0xF) | 1);
        vw32(ep0 + 12, (ring_phys >> 32) as u32);
        // dword 4: average TRB length
        vw32(ep0 + 16, 8);

        // address device command
        let ctrl = TRB_ADDRESS_DEV | ((self.slot_id as u32) << 24);
        self.cmd_enqueue(in_ctx, 0, ctrl);
        self.ring_cmd_doorbell();
        self.wait_cmd(2000)?;
        Ok(())
    }

    unsafe fn cmd_configure_eps(
        &mut self,
        dci_in: u8,
        dci_out: u8,
        mpkt_in: u16,
        mpkt_out: u16,
    ) -> Result<(), UsbMsdInitError> {
        let cs = self.ctx_size as u64;
        let in_ctx = self.dma_base + OFF_IN_CTX as u64;
        let max_dci = dci_in.max(dci_out);

        // zero input context
        core::ptr::write_bytes(in_ctx as *mut u8, 0, ((max_dci as u64 + 2) * cs) as usize);

        // input control context: A0 (slot) | A(dci_in) | A(dci_out)
        let add_flags = (1u32 << 0) | (1u32 << dci_in) | (1u32 << dci_out);
        vw32(in_ctx + 4, add_flags);

        // slot context: update context entries
        let slot_ctx = in_ctx + cs;
        // read the current speed from output context
        let out_slot = self.dma_base + OFF_OUT_CTX as u64;
        let d0 = vr32(out_slot);
        let speed_bits = d0 & (0xF << 20);
        vw32(slot_ctx, speed_bits | ((max_dci as u32) << 26));
        // root hub port from output context
        vw32(slot_ctx + 4, vr32(out_slot + 4));

        // bulk-in endpoint context
        let ep_in = in_ctx + ((dci_in as u64) + 1) * cs;
        // EP type 6 = Bulk IN, CErr=3
        vw32(ep_in + 4, (3u32 << 1) | (6u32 << 3) | ((mpkt_in as u32) << 16));
        let ring_in = self.dma_base + OFF_XFER_BIN as u64;
        vw32(ep_in + 8, (ring_in as u32 & !0xF) | 1);
        vw32(ep_in + 12, (ring_in >> 32) as u32);
        vw32(ep_in + 16, 1024); // average TRB length

        // bulk-out endpoint context
        let ep_out = in_ctx + ((dci_out as u64) + 1) * cs;
        // EP type 2 = Bulk OUT, CErr=3
        vw32(ep_out + 4, (3u32 << 1) | (2u32 << 3) | ((mpkt_out as u32) << 16));
        let ring_out = self.dma_base + OFF_XFER_BOUT as u64;
        vw32(ep_out + 8, (ring_out as u32 & !0xF) | 1);
        vw32(ep_out + 12, (ring_out >> 32) as u32);
        vw32(ep_out + 16, 1024);

        let ctrl = TRB_CONFIGURE_EP | ((self.slot_id as u32) << 24);
        self.cmd_enqueue(in_ctx, 0, ctrl);
        self.ring_cmd_doorbell();
        self.wait_cmd(2000)?;
        Ok(())
    }

    // ─── control transfers (EP0) ─────────────────────────────────────────

    /// IN control transfer: GET_DESCRIPTOR etc.
    /// Returns slice into the descriptor buffer.
    unsafe fn control_in(
        &mut self,
        req_type: u8,
        request: u8,
        value: u16,
        index: u16,
        len: u16,
    ) -> Result<&[u8], UsbMsdInitError> {
        let setup = (req_type as u64)
            | ((request as u64) << 8)
            | ((value as u64) << 16)
            | ((index as u64) << 32)
            | ((len as u64) << 48);

        let desc_buf = self.dma_base + OFF_DESC as u64;

        // setup stage: IDT=1, TRT=IN
        self.xfer_enqueue(Ring::Ep0, setup, 8, TRB_SETUP | TRB_IDT | TRB_TRT_IN);
        // data stage: DIR=IN
        self.xfer_enqueue(
            Ring::Ep0,
            desc_buf,
            len as u32,
            TRB_DATA | TRB_DIR_IN | TRB_ISP,
        );
        // status stage: DIR=OUT (no DIR_IN), IOC
        self.xfer_enqueue(Ring::Ep0, 0, 0, TRB_STATUS | TRB_IOC);

        // ring EP0 doorbell (DCI=1)
        mmio::write32(self.db_base + (self.slot_id as u64) * 4, 1);
        self.wait_xfer(5000)?;

        Ok(core::slice::from_raw_parts(
            desc_buf as *const u8,
            len as usize,
        ))
    }

    /// No-data control transfer: SET_CONFIGURATION etc.
    unsafe fn control_nodata(
        &mut self,
        req_type: u8,
        request: u8,
        value: u16,
        index: u16,
    ) -> Result<(), UsbMsdInitError> {
        let setup = (req_type as u64)
            | ((request as u64) << 8)
            | ((value as u64) << 16)
            | ((index as u64) << 32);

        // setup stage: no data
        self.xfer_enqueue(Ring::Ep0, setup, 8, TRB_SETUP | TRB_IDT);
        // status stage: DIR=IN, IOC
        self.xfer_enqueue(Ring::Ep0, 0, 0, TRB_STATUS | TRB_IOC | TRB_DIR_IN);

        mmio::write32(self.db_base + (self.slot_id as u64) * 4, 1);
        self.wait_xfer(5000)?;
        Ok(())
    }

    // ─── transfer ring enqueue ───────────────────────────────────────────

    unsafe fn xfer_enqueue(&mut self, ring: Ring, param: u64, status: u32, ctrl: u32) {
        let (off, enq, cycle) = match ring {
            Ring::Ep0 => (OFF_XFER_EP0, &mut self.ep0_enq, &mut self.ep0_cycle),
            Ring::BulkOut => (OFF_XFER_BOUT, &mut self.bout_enq, &mut self.bout_cycle),
            Ring::BulkIn => (OFF_XFER_BIN, &mut self.bin_enq, &mut self.bin_cycle),
        };
        let base = self.dma_base + off as u64;
        let c = (ctrl & !1) | (*cycle as u32);
        write_trb(base, *enq as usize, param, status, c);
        *enq += 1;
        if *enq >= XFER_RING_LEN - 1 {
            let link = TRB_LINK | TRB_TC | (*cycle as u32);
            write_trb(base, *enq as usize, base, 0, link);
            *enq = 0;
            *cycle ^= 1;
        }
    }

    /// Wait for a Transfer Event. Returns remaining byte count.
    unsafe fn wait_xfer(&mut self, timeout_ms: u64) -> Result<u32, UsbMsdInitError> {
        let (_, status, _) = self.wait_event(TRB_XFER_EVENT, timeout_ms)?;
        let cc = (status >> 24) as u8;
        // 1 = success, 13 = short packet (ok for mass storage / descriptors)
        if cc != 1 && cc != 13 {
            return Err(UsbMsdInitError::IoError);
        }
        // remaining bytes in bits 23:0
        Ok(status & 0x00FF_FFFF)
    }

    // ─── event ring ──────────────────────────────────────────────────────

    unsafe fn wait_event(
        &mut self,
        expected: u32,
        timeout_ms: u64,
    ) -> Result<(u64, u32, u32), UsbMsdInitError> {
        let start = tsc::read_tsc();
        let timeout = self.tsc_freq.saturating_mul(timeout_ms) / 1000;
        let base = self.dma_base + OFF_EVT_RING as u64;

        loop {
            let a = base + (self.evt_deq as u64) * 16;
            let ctrl = vr32(a + 12);
            if (ctrl & 1) == self.evt_cycle as u32 {
                let p_lo = vr32(a) as u64;
                let p_hi = vr32(a + 4) as u64;
                let param = p_lo | (p_hi << 32);
                let status = vr32(a + 8);

                self.evt_deq += 1;
                if self.evt_deq >= EVT_RING_LEN {
                    self.evt_deq = 0;
                    self.evt_cycle ^= 1;
                }
                // update ERDP, clear EHB
                let new_erdp = base + (self.evt_deq as u64) * 16;
                mmio::write32(
                    self.rt_base + IR0_ERDP,
                    (new_erdp as u32 & !0xF) | 0x08,
                );
                mmio::write32(self.rt_base + IR0_ERDP + 4, (new_erdp >> 32) as u32);

                if (ctrl & TRB_TYPE_MASK) == expected {
                    return Ok((param, status, ctrl));
                }
                // skip unexpected events (port status change, etc.)
                continue;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(UsbMsdInitError::CommandTimeout);
            }
            core::hint::spin_loop();
        }
    }

    // ─── BOT (Bulk-Only Transport) ───────────────────────────────────────

    /// Send a SCSI command via BOT.
    /// `scsi_cb` = command block (6-16 bytes).
    /// `data_len` = expected data transfer length (0 = no data phase).
    /// `data_in` = true for device→host data.
    /// Data lands at OFF_DATA.
    unsafe fn bot_command(
        &mut self,
        scsi_cb: &[u8],
        data_len: u32,
        data_in: bool,
    ) -> Result<u32, UsbMsdInitError> {
        let tag = self.bot_tag;
        self.bot_tag = self.bot_tag.wrapping_add(1);

        // ── build CBW at OFF_CBW ──
        let cbw = self.dma_base + OFF_CBW as u64;
        core::ptr::write_bytes(cbw as *mut u8, 0, 31);
        vw32(cbw, CBW_SIG);
        vw32(cbw + 4, tag);
        vw32(cbw + 8, data_len);
        // flags: 0x80 = data-in, 0x00 = data-out/no-data
        let flags: u8 = if data_in && data_len > 0 { 0x80 } else { 0x00 };
        core::ptr::write_volatile((cbw + 12) as *mut u8, flags);
        // LUN = 0, CB length
        core::ptr::write_volatile((cbw + 14) as *mut u8, scsi_cb.len().min(16) as u8);
        // copy SCSI CDB
        for (i, &b) in scsi_cb.iter().take(16).enumerate() {
            core::ptr::write_volatile((cbw + 15 + i as u64) as *mut u8, b);
        }

        // ── send CBW via bulk-out ──
        self.xfer_enqueue(Ring::BulkOut, cbw, 31, TRB_NORMAL | TRB_IOC);
        mmio::write32(
            self.db_base + (self.slot_id as u64) * 4,
            self.dci_bulk_out as u32,
        );
        self.wait_xfer(5000)?;

        // ── data phase (if any) ──
        let mut transferred = 0u32;
        if data_len > 0 {
            let data_buf = self.dma_base + OFF_DATA as u64;
            if data_in {
                self.xfer_enqueue(Ring::BulkIn, data_buf, data_len, TRB_NORMAL | TRB_IOC | TRB_ISP);
                mmio::write32(
                    self.db_base + (self.slot_id as u64) * 4,
                    self.dci_bulk_in as u32,
                );
            } else {
                self.xfer_enqueue(Ring::BulkOut, data_buf, data_len, TRB_NORMAL | TRB_IOC);
                mmio::write32(
                    self.db_base + (self.slot_id as u64) * 4,
                    self.dci_bulk_out as u32,
                );
            }
            let residue = self.wait_xfer(10000)?;
            transferred = data_len.saturating_sub(residue);
        }

        // ── receive CSW via bulk-in ──
        let csw = self.dma_base + OFF_CSW as u64;
        core::ptr::write_bytes(csw as *mut u8, 0, 13);
        self.xfer_enqueue(Ring::BulkIn, csw, 13, TRB_NORMAL | TRB_IOC);
        mmio::write32(
            self.db_base + (self.slot_id as u64) * 4,
            self.dci_bulk_in as u32,
        );
        self.wait_xfer(5000)?;

        // verify CSW
        let csw_sig = vr32(csw);
        let csw_tag = vr32(csw + 4);
        let csw_status = core::ptr::read_volatile((csw + 12) as *const u8);
        if csw_sig != CSW_SIG || csw_tag != tag || csw_status != 0 {
            return Err(UsbMsdInitError::IoError);
        }

        Ok(transferred)
    }

    /// SCSI READ(10) via BOT — reads `count` sectors at `lba` into OFF_DATA.
    unsafe fn scsi_read_sectors(
        &mut self,
        lba: u64,
        count: u32,
    ) -> Result<(), UsbMsdInitError> {
        let byte_count = count * self.info.sector_size;
        let mut cmd = [0u8; 10];
        cmd[0] = SCSI_READ_10;
        // LBA big-endian at offset 2
        cmd[2] = (lba >> 24) as u8;
        cmd[3] = (lba >> 16) as u8;
        cmd[4] = (lba >> 8) as u8;
        cmd[5] = lba as u8;
        // transfer length (blocks) big-endian at offset 7
        cmd[7] = (count >> 8) as u8;
        cmd[8] = count as u8;

        self.bot_command(&cmd, byte_count, true)?;
        Ok(())
    }

    // ─── descriptor parsing ──────────────────────────────────────────────

    /// Parse the configuration descriptor at OFF_DESC for a mass-storage
    /// BOT interface.  Returns (config_val, ep_in_addr, ep_out_addr, max_pkt_in, max_pkt_out).
    unsafe fn parse_config_desc(&self) -> Option<(u8, u8, u8, u16, u16)> {
        let d = self.dma_base + OFF_DESC as u64;
        let total = u16::from_le_bytes([
            core::ptr::read_volatile((d + 2) as *const u8),
            core::ptr::read_volatile((d + 3) as *const u8),
        ]) as usize;
        let cfg_val = core::ptr::read_volatile((d + 5) as *const u8);
        let limit = total.min(255);

        let mut off = 0usize;
        let mut in_msc = false;
        let mut ep_in: u8 = 0;
        let mut ep_out: u8 = 0;
        let mut mp_in: u16 = 0;
        let mut mp_out: u16 = 0;

        while off + 2 <= limit {
            let blen = core::ptr::read_volatile((d + off as u64) as *const u8) as usize;
            let btype = core::ptr::read_volatile((d + off as u64 + 1) as *const u8);
            if blen < 2 {
                break;
            }
            if off + blen > limit {
                break;
            }
            if btype == 4 && blen >= 9 {
                // interface descriptor
                let cls = core::ptr::read_volatile((d + off as u64 + 5) as *const u8);
                let sub = core::ptr::read_volatile((d + off as u64 + 6) as *const u8);
                let proto = core::ptr::read_volatile((d + off as u64 + 7) as *const u8);
                in_msc =
                    cls == USB_CLASS_MASS_STORAGE && sub == USB_SUBCLASS_SCSI && proto == USB_PROTOCOL_BOT;
            }
            if btype == 5 && blen >= 7 && in_msc {
                // endpoint descriptor
                let addr = core::ptr::read_volatile((d + off as u64 + 2) as *const u8);
                let attr = core::ptr::read_volatile((d + off as u64 + 3) as *const u8);
                let mpkt = u16::from_le_bytes([
                    core::ptr::read_volatile((d + off as u64 + 4) as *const u8),
                    core::ptr::read_volatile((d + off as u64 + 5) as *const u8),
                ]);
                if attr & 0x03 == 0x02 {
                    // bulk
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

    // ─── helpers ─────────────────────────────────────────────────────────

    fn ep0_max_packet(speed: u8) -> u16 {
        match speed {
            4 => 512, // SS
            3 => 64,  // HS
            2 => 8,   // LS
            _ => 64,  // FS / default
        }
    }

    /// Zero transfer ring buffers and reset ring indices for a fresh enumeration attempt.
    unsafe fn reset_transfer_state(&mut self) {
        // zero ring memory
        core::ptr::write_bytes((self.dma_base + OFF_XFER_EP0 as u64) as *mut u8, 0, 256);
        core::ptr::write_bytes((self.dma_base + OFF_XFER_BOUT as u64) as *mut u8, 0, 256);
        core::ptr::write_bytes((self.dma_base + OFF_XFER_BIN as u64) as *mut u8, 0, 256);
        // zero contexts
        core::ptr::write_bytes((self.dma_base + OFF_OUT_CTX as u64) as *mut u8, 0, 2048);
        core::ptr::write_bytes((self.dma_base + OFF_IN_CTX as u64) as *mut u8, 0, 2560);
        // reset indices
        self.ep0_enq = 0;
        self.ep0_cycle = 1;
        self.bout_enq = 0;
        self.bout_cycle = 1;
        self.bin_enq = 0;
        self.bin_cycle = 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BlockDriverInit
// ═══════════════════════════════════════════════════════════════════════════

impl BlockDriverInit for UsbMsdDriver {
    type Error = UsbMsdInitError;
    type Config = UsbMsdConfig;

    fn supported_vendors() -> &'static [u16] {
        &[]
    }

    fn supported_devices() -> &'static [u16] {
        &[]
    }

    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error> {
        Self::new(mmio_base, config)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BlockDriver
// ═══════════════════════════════════════════════════════════════════════════

impl BlockDriver for UsbMsdDriver {
    fn info(&self) -> BlockDeviceInfo {
        self.info
    }

    fn can_submit(&self) -> bool {
        self.last_completion.is_none()
    }

    fn submit_read(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError> {
        if self.last_completion.is_some() {
            return Err(BlockError::QueueFull);
        }
        if num_sectors == 0 || num_sectors > self.info.max_sectors_per_request {
            return Err(BlockError::RequestTooLarge);
        }
        if buffer_phys == 0 {
            return Err(BlockError::InvalidSector);
        }
        let end = sector
            .checked_add(num_sectors as u64)
            .ok_or(BlockError::InvalidSector)?;
        if end > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }

        let byte_count = num_sectors as u64 * self.info.sector_size as u64;
        unsafe {
            self.scsi_read_sectors(sector, num_sectors)
                .map_err(|_| BlockError::IoError)?;

            // copy bounce buffer → caller's buffer
            core::ptr::copy_nonoverlapping(
                (self.dma_base + OFF_DATA as u64) as *const u8,
                buffer_phys as *mut u8,
                byte_count as usize,
            );
        }

        self.last_completion = Some(BlockCompletion {
            request_id,
            status: 0,
            bytes_transferred: (byte_count as u32),
        });
        Ok(())
    }

    fn submit_write(
        &mut self,
        _sector: u64,
        _buffer_phys: u64,
        _num_sectors: u32,
        _request_id: u32,
    ) -> Result<(), BlockError> {
        Err(BlockError::Unsupported)
    }

    fn poll_completion(&mut self) -> Option<BlockCompletion> {
        self.last_completion.take()
    }

    fn notify(&mut self) {}
}
