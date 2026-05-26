//! USB mass-storage block driver — Bulk-Only Transport (BOT) over xHCI.
//!
//! Finds the first USB mass storage device on the xHCI bus, initialises it
//! via Bulk-Only Transport, and exposes SCSI READ(10) through `BlockDriver`.
//! Read-only; write returns `Unsupported`.
//!
//! As of Phase 2 step 2.1 the xHCI controller (TRB rings, BIOS handoff,
//! enumeration, port reset, descriptor fetch) lives in `morpheus_xhci`.
//! This module owns only the class-specific BOT/SCSI glue.
//!
//! Previously this file was ~1800 lines of inlined xHCI logic — duplicate of
//! `hwinit/src/usb/` — that drifted from the canonical version every time
//! the HID path got a real-hardware bug-fix. The two-driver pain (same
//! physical controller, two instances live sequentially) motivated
//! [`XhciController::quiesce`] in `morpheus-xhci`.

use crate::block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
use morpheus_xhci::dma;
use morpheus_xhci::regs::*;
use morpheus_xhci::rings::{vr32, vw32};
use morpheus_xhci::{XhciController, XhciError};

const CBW_SIG: u32 = 0x4342_5355;
const CSW_SIG: u32 = 0x5342_5355;

const SCSI_TEST_UNIT_READY: u8 = 0x00;
const SCSI_REQUEST_SENSE: u8 = 0x03;
const SCSI_READ_CAPACITY_10: u8 = 0x25;
const SCSI_READ_10: u8 = 0x28;

const USB_CLASS_MASS_STORAGE: u8 = 0x08;
const USB_SUBCLASS_SCSI: u8 = 0x06;
const USB_PROTOCOL_BOT: u8 = 0x50;

/// USB mass-storage configuration.
#[derive(Debug, Clone)]
pub struct UsbMsdConfig {
    /// TSC frequency for timeout calculations.
    pub tsc_freq: u64,
    /// Optional DMA bounce buffer base (physical). Unused by the new
    /// implementation — `morpheus-xhci` owns its own static DMA region — but
    /// preserved in the type so callers (block_probe.rs) don't need to change.
    pub dma_phys: u64,
    pub dma_size: usize,
}

/// USB mass-storage init errors. Kept fine-grained because the bootloader's
/// storage probe matches on specific variants when deciding whether to retry
/// or skip. The variants whose error condition no longer occurs after the
/// Phase 2 step 2.1 extraction (e.g. `ControllerScratchpadUnsupported`) are
/// retained as never-constructed so existing match arms keep compiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum UsbMsdInitError {
    InvalidConfig,
    ControllerInitFailed,
    ControllerProbeFailed,
    ControllerResetFailed,
    ControllerScratchpadUnsupported,
    ControllerStartFailed,
    HubUnsupported,
    PortResetFailed,
    PortResetTimeout,
    PortResetHotCmdTimeout,
    PortResetHotSettleTimeout,
    PortResetWarmTimeout,
    PortResetNoLink,
    EnableSlotFailed,
    AddressDeviceFailed,
    DeviceDescriptorFailed,
    ConfigDescriptorFailed,
    MassStorageProtocolUnsupported,
    NoBotMassStorageInterface,
    ActivePortsNoConnectedDevice,
    SetConfigurationFailed,
    ConfigureEndpointsFailed,
    DeviceEnumerationFailed,
    TransportInitFailed,
    NoMedia,
    CommandTimeout,
    IoError,
    NotImplemented,
}

impl core::fmt::Display for UsbMsdInitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::InvalidConfig => "invalid USB MSD config",
            Self::ControllerInitFailed => "xHCI controller init failed",
            Self::ControllerProbeFailed => "xHCI probe failed (dead BAR)",
            Self::ControllerResetFailed => "xHCI reset failed",
            Self::ControllerScratchpadUnsupported => "xHCI scratchpad unsupported",
            Self::ControllerStartFailed => "xHCI start failed",
            Self::HubUnsupported => "USB hub traversal not implemented (storage path)",
            Self::PortResetFailed => "USB port reset failed",
            Self::PortResetTimeout => "USB port reset timed out",
            Self::PortResetHotCmdTimeout => "USB hot reset command did not complete",
            Self::PortResetHotSettleTimeout => "USB hot reset settled wrong",
            Self::PortResetWarmTimeout => "USB warm reset did not complete",
            Self::PortResetNoLink => "USB port reset: no link",
            Self::EnableSlotFailed => "USB enable-slot failed",
            Self::AddressDeviceFailed => "USB address-device failed",
            Self::DeviceDescriptorFailed => "USB device descriptor fetch failed",
            Self::ConfigDescriptorFailed => "USB config descriptor fetch failed",
            Self::MassStorageProtocolUnsupported => "USB mass-storage protocol != BOT",
            Self::NoBotMassStorageInterface => "USB config has no BOT MSD interface",
            Self::ActivePortsNoConnectedDevice => "USB ports active, no device connected",
            Self::SetConfigurationFailed => "USB SET_CONFIGURATION failed",
            Self::ConfigureEndpointsFailed => "USB Configure Endpoint failed",
            Self::DeviceEnumerationFailed => "USB enumeration failed",
            Self::TransportInitFailed => "USB BOT transport init failed",
            Self::NoMedia => "no USB mass-storage device found",
            Self::CommandTimeout => "USB command timeout",
            Self::IoError => "USB I/O error",
            Self::NotImplemented => "not implemented",
        };
        f.write_str(s)
    }
}

impl From<XhciError> for UsbMsdInitError {
    fn from(e: XhciError) -> Self {
        match e {
            XhciError::ProbeFailed => Self::ControllerProbeFailed,
            XhciError::ResetFailed => Self::ControllerResetFailed,
            XhciError::StartFailed => Self::ControllerStartFailed,
            XhciError::ScratchpadUnsupported => Self::ControllerScratchpadUnsupported,
            XhciError::PortResetTimeout => Self::PortResetTimeout,
            XhciError::PortResetNoLink => Self::PortResetNoLink,
            XhciError::PortResetNoCCS => Self::PortResetFailed,
            XhciError::EnableSlotFailed => Self::EnableSlotFailed,
            XhciError::AddressDeviceFailed => Self::AddressDeviceFailed,
            XhciError::ConfigureEndpointsFailed => Self::ConfigureEndpointsFailed,
            XhciError::CommandTimeout => Self::CommandTimeout,
            XhciError::IoError => Self::IoError,
            XhciError::NoMedia => Self::NoMedia,
            XhciError::NotSupported => Self::NotImplemented,
        }
    }
}

pub struct UsbMsdDriver {
    controller: XhciController,
    info: BlockDeviceInfo,
    last_completion: Option<BlockCompletion>,
    bot_tag: u32,
}

#[inline(always)]
fn dbg(s: &str) {
    morpheus_hal_x86_64::serial::puts(s);
}

impl UsbMsdDriver {
    /// Initialise xHCI controller, enumerate first USB mass-storage device,
    /// run SCSI READ CAPACITY. Returns a ready-to-read driver or an error.
    pub unsafe fn new(mmio_base: u64, config: UsbMsdConfig) -> Result<Self, UsbMsdInitError> {
        if mmio_base == 0 || config.tsc_freq == 0 {
            return Err(UsbMsdInitError::InvalidConfig);
        }

        dbg("[USB-MSD] bringing up xHCI controller\n");
        let controller =
            XhciController::new(mmio_base, config.tsc_freq).map_err(UsbMsdInitError::from)?;
        dbg("[USB-MSD] controller up; enumerating ports\n");

        let mut drv = Self {
            controller,
            info: BlockDeviceInfo {
                sector_size: 512,
                total_sectors: 0,
                max_sectors_per_request: (dma::DATA_BUF_SIZE as u32) / 512,
                read_only: false,
            },
            last_completion: None,
            bot_tag: 1,
        };

        drv.enumerate_and_configure()?;
        dbg("[USB-MSD] enumeration OK; running SCSI init\n");
        drv.scsi_init()?;
        dbg("[USB-MSD] driver ready\n");
        Ok(drv)
    }

    /// Walk root ports, reset the first one that returns a BOT mass-storage
    /// device, enable a slot, address it, configure bulk endpoints. Hub
    /// enumeration is intentionally NOT attempted here — the HID Phase-9 path
    /// has that, but for storage we only support root-port devices.
    unsafe fn enumerate_and_configure(&mut self) -> Result<(), UsbMsdInitError> {
        let port_count = self.controller.max_ports;

        for port in 0..port_count {
            // Speed check via PORTSC — skip ports with no link.
            let portsc = morpheus_hal_x86_64::asm::mmio::read32(self.controller.portsc(port));
            if portsc & PORTSC_CCS == 0 {
                continue;
            }

            let speed = match self.controller.port_reset(port) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // enable slot → address → fetch descriptors
            if self.controller.enable_slot().is_err() {
                continue;
            }
            if self
                .controller
                .address_device(port, speed, 0, 0, 0)
                .is_err()
            {
                continue;
            }
            let dev_desc = match self.controller.get_device_descriptor() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let dev_class = core::ptr::read_volatile(dev_desc.add(4));
            // Most MSD devices use per-interface class (dev_class == 0). We
            // accept any class here and decide based on the interface descriptor.
            let _ = dev_class;

            // 9-byte head, then full pull
            let cfg_short = match self.controller.get_config_descriptor(9) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let total_len = u16::from_le_bytes([
                core::ptr::read_volatile(cfg_short.add(2)),
                core::ptr::read_volatile(cfg_short.add(3)),
            ]);
            let cfg_full = match self.controller.get_config_descriptor(total_len.min(512)) {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Look for BOT mass-storage interface (cls=08 sub=06 proto=50)
            // and capture its bulk endpoints.
            let parsed = match self.controller.parse_config(cfg_full) {
                Some(v) => v,
                None => continue,
            };
            let (cfg_val, ep_in, ep_out, mp_in, mp_out) = parsed;

            // SET_CONFIGURATION, then configure bulk endpoints.
            if self.controller.set_configuration(cfg_val).is_err() {
                return Err(UsbMsdInitError::SetConfigurationFailed);
            }
            let dci_in = (ep_in & 0x7F) * 2 + 1;
            let dci_out = (ep_out & 0x7F) * 2;
            if self
                .controller
                .configure_endpoints(dci_in, dci_out, mp_in, mp_out)
                .is_err()
            {
                return Err(UsbMsdInitError::ConfigureEndpointsFailed);
            }
            self.controller.dci_bulk_in = dci_in;
            self.controller.dci_bulk_out = dci_out;
            return Ok(());
        }

        Err(UsbMsdInitError::NoMedia)
    }

    /// Issue TEST_UNIT_READY → READ_CAPACITY(10) to confirm the device is
    /// online and learn its sector count.
    unsafe fn scsi_init(&mut self) -> Result<(), UsbMsdInitError> {
        // TEST_UNIT_READY (6-byte CDB, zero-filled tail).
        let tur = [SCSI_TEST_UNIT_READY, 0, 0, 0, 0, 0];
        // Some media require one pass of REQUEST SENSE to clear UNIT
        // ATTENTION after enumeration — try TUR; if it fails, request sense
        // and retry once before giving up.
        if self.bot_command(&tur, 0, false).is_err() {
            let rs = [SCSI_REQUEST_SENSE, 0, 0, 0, 18, 0];
            let _ = self.bot_command(&rs, 18, true);
            self.bot_command(&tur, 0, false)?;
        }

        // READ_CAPACITY(10) — 8-byte response: last_lba (BE) + block_size (BE).
        let rc = [SCSI_READ_CAPACITY_10, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        self.bot_command(&rc, 8, true)?;
        let data = self.controller.dma_base + dma::OFF_DATA as u64;
        let last_lba_be = vr32(data);
        let blk_be = vr32(data + 4);
        let last_lba = u32::from_be(last_lba_be) as u64;
        let blk_size = u32::from_be(blk_be);
        self.info.sector_size = blk_size.max(512);
        self.info.total_sectors = last_lba + 1;
        self.info.max_sectors_per_request = (dma::DATA_BUF_SIZE as u32) / self.info.sector_size;
        Ok(())
    }

    /// SCSI READ(10) via BOT — reads `count` sectors at `lba` into OFF_DATA.
    unsafe fn scsi_read_sectors(&mut self, lba: u64, count: u32) -> Result<(), UsbMsdInitError> {
        let byte_count = count * self.info.sector_size;
        let mut cmd = [0u8; 10];
        cmd[0] = SCSI_READ_10;
        cmd[2] = (lba >> 24) as u8;
        cmd[3] = (lba >> 16) as u8;
        cmd[4] = (lba >> 8) as u8;
        cmd[5] = lba as u8;
        cmd[7] = (count >> 8) as u8;
        cmd[8] = count as u8;
        self.bot_command(&cmd, byte_count, true)?;
        Ok(())
    }

    /// Send a BOT command. Data lands at OFF_DATA. Returns transferred bytes.
    ///
    /// CBW (31 bytes) → optional data stage → CSW (13 bytes). Tags must
    /// match between CBW and CSW; tag mismatch is treated as a stalled
    /// transport (`IoError`).
    unsafe fn bot_command(
        &mut self,
        scsi_cb: &[u8],
        data_len: u32,
        data_in: bool,
    ) -> Result<u32, UsbMsdInitError> {
        let tag = self.bot_tag;
        self.bot_tag = self.bot_tag.wrapping_add(1);

        let c = &mut self.controller;
        let cbw = c.dma_base + dma::OFF_CBW as u64;
        core::ptr::write_bytes(cbw as *mut u8, 0, 31);
        vw32(cbw, CBW_SIG);
        vw32(cbw + 4, tag);
        vw32(cbw + 8, data_len);
        let flags: u8 = if data_in && data_len > 0 { 0x80 } else { 0x00 };
        core::ptr::write_volatile((cbw + 12) as *mut u8, flags);
        core::ptr::write_volatile((cbw + 14) as *mut u8, scsi_cb.len().min(16) as u8);
        for (i, &b) in scsi_cb.iter().take(16).enumerate() {
            core::ptr::write_volatile((cbw + 15 + i as u64) as *mut u8, b);
        }

        // ── send CBW on bulk-out ──
        c.bout.enqueue(cbw, 31, TRB_NORMAL | TRB_IOC);
        c.ring_xfer_doorbell(c.dci_bulk_out as u32);
        c.wait_xfer(c.slot_id, c.dci_bulk_out as u32, 5000)?;

        // ── data phase ──
        let mut transferred = 0u32;
        if data_len > 0 {
            let buf = c.dma_base + dma::OFF_DATA as u64;
            if data_in {
                c.bin.enqueue(buf, data_len, TRB_NORMAL | TRB_IOC | TRB_ISP);
                c.ring_xfer_doorbell(c.dci_bulk_in as u32);
                let residue = c.wait_xfer(c.slot_id, c.dci_bulk_in as u32, 10000)?;
                transferred = data_len.saturating_sub(residue);
            } else {
                c.bout.enqueue(buf, data_len, TRB_NORMAL | TRB_IOC);
                c.ring_xfer_doorbell(c.dci_bulk_out as u32);
                let residue = c.wait_xfer(c.slot_id, c.dci_bulk_out as u32, 10000)?;
                transferred = data_len.saturating_sub(residue);
            }
        }

        // ── receive CSW on bulk-in ──
        let csw = c.dma_base + dma::OFF_CSW as u64;
        core::ptr::write_bytes(csw as *mut u8, 0, 13);
        c.bin.enqueue(csw, 13, TRB_NORMAL | TRB_IOC);
        c.ring_xfer_doorbell(c.dci_bulk_in as u32);
        c.wait_xfer(c.slot_id, c.dci_bulk_in as u32, 5000)?;

        let sig = vr32(csw);
        let csw_tag = vr32(csw + 4);
        let status = core::ptr::read_volatile((csw + 12) as *const u8);
        if sig != CSW_SIG || csw_tag != tag || status != 0 {
            return Err(UsbMsdInitError::IoError);
        }

        Ok(transferred)
    }
}

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
                (self.controller.dma_base + dma::OFF_DATA as u64) as *const u8,
                buffer_phys as *mut u8,
                byte_count as usize,
            );
        }

        self.last_completion = Some(BlockCompletion {
            request_id,
            status: 0,
            bytes_transferred: byte_count as u32,
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

impl Drop for UsbMsdDriver {
    fn drop(&mut self) {
        // Quiesce the controller before dropping so the physical xHCI is
        // left in a clean state. In practice this driver lives for the
        // duration of the kernel run (held in `bootloader::BLOCK_DEVICE`),
        // but the explicit quiesce keeps the API contract honest.
        unsafe {
            self.controller.quiesce();
        }
    }
}
