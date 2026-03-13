//! SDHCI block device driver scaffold.
//!
//! This module defines the BlockDriver-facing contract for the upcoming
//! ASM-backed SDHCI implementation. Hardware-touching logic will be added in
//! assembly primitives; Rust keeps orchestration/state/error surfaces stable.

use crate::driver::block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
use crate::asm::core::mmio;
use crate::asm::core::tsc;

extern "win64" {
    fn asm_sdhci_read_caps(mmio_base: u64) -> u32;
    fn asm_sdhci_card_present(mmio_base: u64) -> u32;
    fn asm_sdhci_controller_reset(mmio_base: u64, tsc_freq: u64) -> u32;
    fn asm_sdhci_basic_power_clock(mmio_base: u64) -> u32;
    fn asm_sdhci_read_block_pio(mmio_base: u64, lba: u64, dst: u64, tsc_freq: u64) -> u32;
}

/// PCI base class / subclass / prog-if for SD Host Controller.
pub const PCI_CLASS_SDHCI: u32 = 0x080501;

/// SDHCI configuration.
#[derive(Debug, Clone)]
pub struct SdhciConfig {
    /// TSC frequency for timeout calculations.
    pub tsc_freq: u64,
    /// Optional DMA bounce buffer base (physical).
    pub dma_phys: u64,
    /// Optional DMA bounce buffer size.
    pub dma_size: usize,
}

/// SDHCI initialization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdhciInitError {
    InvalidConfig,
    ControllerResetFailed,
    NoCardPresent,
    VoltageSwitchFailed,
    ClockSetupFailed,
    CommandTimeout,
    DataTimeout,
    IoError,
    NotImplemented,
}

impl core::fmt::Display for SdhciInitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidConfig => write!(f, "Invalid SDHCI configuration"),
            Self::ControllerResetFailed => write!(f, "SDHCI controller reset failed"),
            Self::NoCardPresent => write!(f, "No SD card present"),
            Self::VoltageSwitchFailed => write!(f, "SDHCI voltage switch failed"),
            Self::ClockSetupFailed => write!(f, "SDHCI clock setup failed"),
            Self::CommandTimeout => write!(f, "SDHCI command timeout"),
            Self::DataTimeout => write!(f, "SDHCI data timeout"),
            Self::IoError => write!(f, "SDHCI I/O error"),
            Self::NotImplemented => write!(f, "SDHCI driver not implemented yet"),
        }
    }
}

/// SDHCI driver state (scaffold).
pub struct SdhciDriver {
    mmio_base: u64,
    tsc_freq: u64,
    _caps: u32,
    info: BlockDeviceInfo,
    high_capacity: bool,
    rca: u16,
    last_completion: Option<BlockCompletion>,
}

const REG_ARGUMENT: u64 = 0x08;
const REG_COMMAND: u64 = 0x0E;
const REG_RESPONSE0: u64 = 0x10;
const REG_PRESENT_STATE: u64 = 0x24;
const REG_INT_STATUS: u64 = 0x30;

const PRESENT_CMD_INHIBIT: u32 = 1 << 0;
const PRESENT_DAT_INHIBIT: u32 = 1 << 1;

const INT_CMD_COMPLETE: u32 = 1 << 0;
const INT_ERROR: u32 = 1 << 15;

const CMD_RESP_NONE: u16 = 0x00;
const CMD_RESP_LONG: u16 = 0x01;
const CMD_RESP_SHORT: u16 = 0x02;
const CMD_RESP_SHORT_BUSY: u16 = 0x03;
const CMD_CRC: u16 = 0x08;
const CMD_INDEX: u16 = 0x10;

const ACMD41_OCR: u32 = 0x40FF_8000;

impl SdhciDriver {
    #[inline(always)]
    unsafe fn reg32(&self, off: u64) -> u64 {
        self.mmio_base + off
    }

    fn timeout_ticks(&self, ms: u64) -> u64 {
        self.tsc_freq.saturating_mul(ms) / 1000
    }

    unsafe fn wait_not_inhibit(&self, timeout_ms: u64) -> Result<(), SdhciInitError> {
        let start = tsc::read_tsc();
        let timeout = self.timeout_ticks(timeout_ms);
        loop {
            let ps = mmio::read32(self.reg32(REG_PRESENT_STATE));
            if (ps & (PRESENT_CMD_INHIBIT | PRESENT_DAT_INHIBIT)) == 0 {
                return Ok(());
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(SdhciInitError::CommandTimeout);
            }
            core::hint::spin_loop();
        }
    }

    unsafe fn clear_ints(&self) {
        mmio::write32(self.reg32(REG_INT_STATUS), 0xFFFF_FFFF);
    }

    unsafe fn send_cmd(
        &self,
        index: u8,
        arg: u32,
        flags: u16,
        timeout_ms: u64,
    ) -> Result<u32, SdhciInitError> {
        self.wait_not_inhibit(timeout_ms)?;
        self.clear_ints();

        mmio::write32(self.reg32(REG_ARGUMENT), arg);
        let cmd = ((index as u16) << 8) | flags;
        mmio::write16(self.reg32(REG_COMMAND), cmd);

        let start = tsc::read_tsc();
        let timeout = self.timeout_ticks(timeout_ms);
        loop {
            let st = mmio::read32(self.reg32(REG_INT_STATUS));
            if (st & INT_ERROR) != 0 {
                self.clear_ints();
                return Err(SdhciInitError::IoError);
            }
            if (st & INT_CMD_COMPLETE) != 0 {
                mmio::write32(self.reg32(REG_INT_STATUS), INT_CMD_COMPLETE);
                return Ok(mmio::read32(self.reg32(REG_RESPONSE0)));
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                self.clear_ints();
                return Err(SdhciInitError::CommandTimeout);
            }
            core::hint::spin_loop();
        }
    }

    unsafe fn init_card(&mut self) -> Result<(), SdhciInitError> {
        // CMD0: idle
        let _ = self.send_cmd(0, 0, CMD_RESP_NONE, 100)?;

        // CMD8: interface condition; if it fails, assume legacy SDSC.
        let mut supports_v2 = false;
        if let Ok(r7) = self.send_cmd(8, 0x0000_01AA, CMD_RESP_SHORT | CMD_CRC | CMD_INDEX, 100)
        {
            supports_v2 = (r7 & 0xFFF) == 0x1AA;
        }

        // ACMD41 loop until card powers up.
        let mut ocr: u32;
        let hcs = if supports_v2 { 1u32 << 30 } else { 0 };
        let start = tsc::read_tsc();
        let timeout = self.timeout_ticks(2000);
        loop {
            let _ = self.send_cmd(55, 0, CMD_RESP_SHORT | CMD_CRC | CMD_INDEX, 100)?;
            ocr = self.send_cmd(41, ACMD41_OCR | hcs, CMD_RESP_SHORT, 100)?;
            if (ocr & (1u32 << 31)) != 0 {
                break;
            }
            if tsc::read_tsc().wrapping_sub(start) > timeout {
                return Err(SdhciInitError::CommandTimeout);
            }
        }
        self.high_capacity = (ocr & (1u32 << 30)) != 0;

        // CMD2: CID
        let _ = self.send_cmd(2, 0, CMD_RESP_LONG | CMD_CRC, 200)?;

        // CMD3: RCA assign
        let rca_resp = self.send_cmd(3, 0, CMD_RESP_SHORT | CMD_CRC | CMD_INDEX, 200)?;
        self.rca = (rca_resp >> 16) as u16;
        if self.rca == 0 {
            return Err(SdhciInitError::IoError);
        }

        // CMD7: select card
        let _ = self.send_cmd(
            7,
            (self.rca as u32) << 16,
            CMD_RESP_SHORT_BUSY | CMD_CRC | CMD_INDEX,
            200,
        )?;

        // CMD16: set block length for SDSC cards.
        if !self.high_capacity {
            let _ = self.send_cmd(16, 512, CMD_RESP_SHORT | CMD_CRC | CMD_INDEX, 100)?;
        }

        Ok(())
    }

    /// Create a new SDHCI driver instance.
    ///
    /// Phase-1 scaffold returns NotImplemented until ASM primitives land.
    pub unsafe fn new(mmio_base: u64, config: SdhciConfig) -> Result<Self, SdhciInitError> {
        if mmio_base == 0 || config.tsc_freq == 0 {
            return Err(SdhciInitError::InvalidConfig);
        }

        if asm_sdhci_controller_reset(mmio_base, config.tsc_freq) != 0 {
            return Err(SdhciInitError::ControllerResetFailed);
        }

        if asm_sdhci_basic_power_clock(mmio_base) != 0 {
            return Err(SdhciInitError::ClockSetupFailed);
        }

        if asm_sdhci_card_present(mmio_base) == 0 {
            return Err(SdhciInitError::NoCardPresent);
        }

        let caps = asm_sdhci_read_caps(mmio_base);

        let mut this = Self {
            mmio_base,
            tsc_freq: config.tsc_freq,
            _caps: caps,
            info: BlockDeviceInfo {
            total_sectors: u32::MAX as u64,
            sector_size: 512,
            max_sectors_per_request: 128,
            read_only: true,
            },
            high_capacity: true,
            rca: 0,
            last_completion: None,
        };

        this.init_card()?;
        let _ = config;
        Ok(this)
    }
}

impl BlockDriverInit for SdhciDriver {
    type Error = SdhciInitError;
    type Config = SdhciConfig;

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

impl BlockDriver for SdhciDriver {
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

        let end_sector = sector
            .checked_add(num_sectors as u64)
            .ok_or(BlockError::InvalidSector)?;
        if end_sector > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }

        let mut curr_sector = sector;
        let mut curr_dst = buffer_phys;
        for _ in 0..num_sectors {
            let arg = if self.high_capacity {
                curr_sector
            } else {
                curr_sector
                    .checked_mul(self.info.sector_size as u64)
                    .ok_or(BlockError::InvalidSector)?
            };
            let rc = unsafe {
                asm_sdhci_read_block_pio(self.mmio_base, arg, curr_dst, self.tsc_freq)
            };
            if rc != 0 {
                return Err(match rc {
                    1 | 2 | 3 | 4 => BlockError::Timeout,
                    _ => BlockError::IoError,
                });
            }

            curr_sector = curr_sector.wrapping_add(1);
            curr_dst = curr_dst.wrapping_add(self.info.sector_size as u64);
        }

        self.last_completion = Some(BlockCompletion {
            request_id,
            status: 0,
            bytes_transferred: num_sectors * self.info.sector_size,
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
