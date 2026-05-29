//! AHCI 1.3.1 block driver. Targets Intel PCH SATA (Wildcat Point-LP, etc.)
//! and QEMU ich9-ahci. Polling-only; per-port CLB/FIS/CT DMA layout per spec §4.2.

pub mod init;
pub mod port;
pub mod regs;

use crate::block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
use morpheus_hal_x86_64::asm::tsc::read_tsc as read_tsc_raw;

pub use init::{AhciConfig, AhciInitError};

extern "win64" {
    fn asm_ahci_hba_reset(abar: u64, tsc_freq: u64) -> u32;
    fn asm_ahci_enable(abar: u64) -> u32;
    fn asm_ahci_read_cap(abar: u64) -> u32;
    fn asm_ahci_read_pi(abar: u64) -> u32;
    fn asm_ahci_read_version(abar: u64) -> u32;
    fn asm_ahci_disable_interrupts(abar: u64);
    fn asm_ahci_get_num_ports(abar: u64) -> u32;
    fn asm_ahci_get_num_cmd_slots(abar: u64) -> u32;
    fn asm_ahci_supports_64bit(abar: u64) -> u32;

    fn asm_ahci_port_detect(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_stop(abar: u64, port_num: u32, tsc_freq: u64) -> u32;
    fn asm_ahci_port_start(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_setup(abar: u64, port_num: u32, clb_phys: u64, fb_phys: u64) -> u32;
    fn asm_ahci_port_clear_errors(abar: u64, port_num: u32);
    fn asm_ahci_port_read_sig(abar: u64, port_num: u32) -> u32;
    #[allow(dead_code)]
    fn asm_ahci_port_read_tfd(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_read_ssts(abar: u64, port_num: u32) -> u32;
    #[allow(dead_code)]
    fn asm_ahci_port_read_is(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_clear_is(abar: u64, port_num: u32, bits: u32);
    fn asm_ahci_port_disable_interrupts(abar: u64, port_num: u32);

    #[allow(dead_code)]
    fn asm_ahci_setup_cmd_header(cmd_header_ptr: u64, flags: u32, ctba_phys: u64);
    #[allow(dead_code)]
    fn asm_ahci_build_h2d_fis(fis_ptr: u64, command: u8, lba: u64, sector_count: u16);
    #[allow(dead_code)]
    fn asm_ahci_build_prdt(prdt_ptr: u64, data_phys: u64, byte_count_minus_1: u32);
    #[allow(dead_code)]
    fn asm_ahci_issue_cmd(abar: u64, port_num: u32, slot_mask: u32);
    fn asm_ahci_poll_cmd(
        abar: u64,
        port_num: u32,
        slot_mask: u32,
        tsc_freq: u64,
        timeout_ms: u32,
    ) -> u32;
    fn asm_ahci_check_cmd_complete(abar: u64, port_num: u32, slot_mask: u32) -> u32;
    fn asm_ahci_read_prdbc(cmd_header_ptr: u64) -> u32;

    fn asm_ahci_identify_device(
        abar: u64,
        port_num: u32,
        identify_buf_phys: u64,
        cmd_header_ptr: u64,
        cmd_table_ptr: u64,
        cmd_table_phys: u64,
        tsc_freq: u64,
    ) -> u32;
    fn asm_ahci_get_identify_capacity(identify_buf_ptr: u64) -> u64;
    fn asm_ahci_get_identify_sector_size(identify_buf_ptr: u64) -> u32;

    fn asm_ahci_submit_read(
        abar: u64,
        port_num: u32,
        lba: u64,
        data_buf_phys: u64,
        num_sectors: u32,
        cmd_slot: u32,
        cmd_header_ptr: u64,
        cmd_table_ptr: u64,
        cmd_table_phys: u64,
    ) -> u32;
    fn asm_ahci_submit_write(
        abar: u64,
        port_num: u32,
        lba: u64,
        data_buf_phys: u64,
        num_sectors: u32,
        cmd_slot: u32,
        cmd_header_ptr: u64,
        cmd_table_ptr: u64,
        cmd_table_phys: u64,
    ) -> u32;
    fn asm_ahci_flush_cache(
        abar: u64,
        port_num: u32,
        cmd_slot: u32,
        cmd_header_ptr: u64,
        cmd_table_ptr: u64,
        cmd_table_phys: u64,
    ) -> u32;
}

pub const INTEL_VENDOR_ID: u16 = 0x8086;

/// Wildcat Point-LP — ThinkPad T450s reference target.
pub const AHCI_DEVICE_WPT_LP: u16 = 0x9C83;

pub const AHCI_DEVICE_IDS: &[u16] = &[
    0x2922, // ICH9 (QEMU ich9-ahci)
    0x9C83, // Wildcat Point-LP
    0x9C03, // Lynx Point-LP
    0x8C02, // 8 Series/C220
    0x8C03, // 8 Series/C220 Mobile
    0xA102, // 100 Series/C230
    0xA103, // 100 Series Mobile
    0xA282, // 200 Series
    0xA353, // Cannon Lake
    0x02D3, // Comet Lake
    0xA0D3, // Tiger Lake
];

pub const PCI_CLASS_SATA_AHCI: u32 = 0x010601;

pub const MAX_CMD_SLOTS: usize = 32;

pub const DET_NONE: u32 = 0;
pub const DET_PRESENT: u32 = 1;
pub const DET_PHY_COMM: u32 = 3;

pub const SIG_ATA: u32 = 0x00000101;
pub const SIG_ATAPI: u32 = 0xEB140101;

pub const ATA_STS_BSY: u8 = 1 << 7;
pub const ATA_STS_DRQ: u8 = 1 << 3;
pub const ATA_STS_ERR: u8 = 1 << 0;

/// AHCI §10.6 BIOS/OS handoff. On Intel PCH, skipping this and writing GHC.HR
/// stalls the bus forever because the firmware state machine never advances.
/// No-op when CAP2.BOH is clear (QEMU, non-Intel HBAs).
unsafe fn ahci_bios_handoff(abar: u64, tsc_freq: u64) {
    const AHCI_CAP2: u64 = 0x24;
    const AHCI_BOHC: u64 = 0x28;

    const CAP2_BOH: u32 = 1 << 0;
    const BOHC_OOS: u32 = 1 << 1;
    const BOHC_BOS: u32 = 1 << 0;
    const BOHC_BB: u32 = 1 << 4;

    let cap2 = core::ptr::read_volatile((abar + AHCI_CAP2) as *const u32);
    if cap2 & CAP2_BOH == 0 {
        return;
    }

    let bohc = core::ptr::read_volatile((abar + AHCI_BOHC) as *const u32);
    core::ptr::write_volatile((abar + AHCI_BOHC) as *mut u32, bohc | BOHC_OOS);

    // §10.6.4 step 5: 25 ms for BIOS to drop BOS. If it doesn't, HBA reset will sort it.
    let deadline_bos = read_tsc_raw().wrapping_add(tsc_freq / 40);
    loop {
        let b = core::ptr::read_volatile((abar + AHCI_BOHC) as *const u32);
        if b & BOHC_BOS == 0 {
            break;
        }
        if read_tsc_raw() >= deadline_bos {
            break;
        }
        core::hint::spin_loop();
    }

    // §10.6.4 step 7: up to 2 s for BB to clear — some BIOSes do post-handoff cleanup.
    let deadline_bb = read_tsc_raw().wrapping_add(tsc_freq.saturating_mul(2));
    loop {
        let b = core::ptr::read_volatile((abar + AHCI_BOHC) as *const u32);
        if b & BOHC_BB == 0 {
            break;
        }
        if read_tsc_raw() >= deadline_bb {
            break;
        }
        core::hint::spin_loop();
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
struct InFlightRequest {
    request_id: u32,
    slot: u8,
    active: bool,
}

#[allow(dead_code)]
pub struct AhciDriver {
    abar: u64,
    port_num: u32,
    tsc_freq: u64,
    info: BlockDeviceInfo,
    num_slots: u32,
    in_flight: [InFlightRequest; MAX_CMD_SLOTS],
    next_slot: u8,
    cmd_list_cpu: *mut u8,
    cmd_list_phys: u64,
    fis_cpu: *mut u8,
    fis_phys: u64,
    cmd_tables_cpu: *mut u8,
    cmd_tables_phys: u64,
    identify_cpu: *mut u8,
    identify_phys: u64,
}

impl AhciDriver {
    /// # Safety
    /// `abar` must be the device's BAR5 MMIO mapping; config DMA pointers must be valid.
    pub unsafe fn new(abar: u64, config: AhciConfig) -> Result<Self, AhciInitError> {
        Self::new_inner(abar, config, None)
    }

    /// # Safety
    /// Same as `new`; `port_num < 32`.
    pub unsafe fn new_on_port(
        abar: u64,
        config: AhciConfig,
        port_num: u32,
    ) -> Result<Self, AhciInitError> {
        Self::new_inner(abar, config, Some(port_num))
    }

    unsafe fn new_inner(
        abar: u64,
        config: AhciConfig,
        forced_port: Option<u32>,
    ) -> Result<Self, AhciInitError> {
        if config.cmd_list_cpu.is_null() || config.fis_cpu.is_null() {
            return Err(AhciInitError::InvalidConfig);
        }

        let tsc_freq = config.tsc_freq;

        // BIOS/OS handoff must precede any GHC access on Intel PCH.
        ahci_bios_handoff(abar, tsc_freq);

        if asm_ahci_hba_reset(abar, tsc_freq) != 0 {
            return Err(AhciInitError::ResetFailed);
        }
        asm_ahci_enable(abar);

        let _cap = asm_ahci_read_cap(abar);
        let num_slots = asm_ahci_get_num_cmd_slots(abar);
        let ports_impl = asm_ahci_read_pi(abar);

        let supports_64bit = asm_ahci_supports_64bit(abar) != 0;
        if !supports_64bit && (config.cmd_list_phys >> 32) != 0 {
            return Err(AhciInitError::No64BitSupport);
        }

        asm_ahci_disable_interrupts(abar);

        // Strict ATA sigs first, then anything that looked alive during the settle window.
        let mut strict_ports = [u32::MAX; 32];
        let mut strict_count = 0usize;
        let mut fallback_ports = [u32::MAX; 32];
        let mut fallback_count = 0usize;

        if let Some(port) = forced_port {
            if port >= 32 {
                return Err(AhciInitError::NoDeviceFound);
            }
            let mut impl_mask = ports_impl;
            if impl_mask == 0 {
                let n_ports = asm_ahci_get_num_ports(abar).min(32);
                impl_mask = if n_ports >= 32 {
                    u32::MAX
                } else {
                    (1u32 << n_ports) - 1
                };
            }
            if (impl_mask & (1 << port)) == 0 {
                return Err(AhciInitError::NoDeviceFound);
            }
            strict_ports[0] = port;
            strict_count = 1;
        } else {
            let mut impl_mask = ports_impl;
            if impl_mask == 0 {
                let n_ports = asm_ahci_get_num_ports(abar).min(32);
                impl_mask = if n_ports >= 32 {
                    u32::MAX
                } else {
                    (1u32 << n_ports) - 1
                };
            }

            let settle_ticks = (tsc_freq / 1000).saturating_mul(200);
            for port in 0..32u32 {
                if (impl_mask & (1 << port)) == 0 {
                    continue;
                }

                let start = read_tsc_raw();
                let mut strict = false;
                let mut fallback = false;

                while read_tsc_raw().wrapping_sub(start) < settle_ticks {
                    let det = asm_ahci_port_detect(abar, port);
                    let sig = asm_ahci_port_read_sig(abar, port);
                    let ssts = asm_ahci_port_read_ssts(abar, port);
                    let ipm = (ssts >> 8) & 0x0F;

                    if (det == DET_PHY_COMM || det == DET_PRESENT) && sig == SIG_ATA {
                        strict = true;
                        break;
                    }

                    if det != DET_NONE || sig != 0 || ipm != 0 {
                        fallback = true;
                    }

                    core::hint::spin_loop();
                }

                if strict && strict_count < strict_ports.len() {
                    strict_ports[strict_count] = port;
                    strict_count += 1;
                } else if fallback && fallback_count < fallback_ports.len() {
                    fallback_ports[fallback_count] = port;
                    fallback_count += 1;
                }
            }
        }

        if strict_count == 0 && fallback_count == 0 {
            return Err(AhciInitError::NoDeviceFound);
        }

        let mut candidate_ports = [u32::MAX; 32];
        let mut candidate_count = 0usize;
        #[allow(clippy::needless_range_loop)]
        for i in 0..strict_count {
            candidate_ports[candidate_count] = strict_ports[i];
            candidate_count += 1;
        }
        #[allow(clippy::needless_range_loop)]
        for i in 0..fallback_count {
            if candidate_count >= candidate_ports.len() {
                break;
            }
            candidate_ports[candidate_count] = fallback_ports[i];
            candidate_count += 1;
        }

        let mut last_err = AhciInitError::NoDeviceFound;

        #[allow(clippy::needless_range_loop)]
        for i in 0..candidate_count {
            let port_num = candidate_ports[i];

            let stop_result = asm_ahci_port_stop(abar, port_num, tsc_freq);
            if stop_result != 0 {
                last_err = AhciInitError::PortStopTimeout;
                continue;
            }

            asm_ahci_port_setup(abar, port_num, config.cmd_list_phys, config.fis_phys);
            asm_ahci_port_clear_errors(abar, port_num);
            asm_ahci_port_disable_interrupts(abar, port_num);
            asm_ahci_port_start(abar, port_num);

            // 50 ms settle for link/signature after engine start.
            let link_ticks = (tsc_freq / 1000).saturating_mul(50);
            let link_start = read_tsc_raw();
            while read_tsc_raw().wrapping_sub(link_start) < link_ticks {
                let det = asm_ahci_port_detect(abar, port_num);
                let sig = asm_ahci_port_read_sig(abar, port_num);
                if det != DET_NONE && (sig == SIG_ATA || sig == 0) {
                    break;
                }
                core::hint::spin_loop();
            }

            let identify_result = asm_ahci_identify_device(
                abar,
                port_num,
                config.identify_phys,
                config.cmd_list_cpu as u64,
                config.cmd_tables_cpu as u64,
                config.cmd_tables_phys,
                tsc_freq,
            );

            if identify_result != 0 {
                last_err = AhciInitError::IdentifyFailed;
                continue;
            }

            let total_sectors = asm_ahci_get_identify_capacity(config.identify_cpu as u64);
            let sector_size = asm_ahci_get_identify_sector_size(config.identify_cpu as u64);

            let info = BlockDeviceInfo {
                total_sectors,
                sector_size,
                max_sectors_per_request: 256,
                read_only: false,
            };

            return Ok(Self {
                abar,
                port_num,
                tsc_freq,
                info,
                num_slots,
                in_flight: [InFlightRequest::default(); MAX_CMD_SLOTS],
                next_slot: 0,
                cmd_list_cpu: config.cmd_list_cpu,
                cmd_list_phys: config.cmd_list_phys,
                fis_cpu: config.fis_cpu,
                fis_phys: config.fis_phys,
                cmd_tables_cpu: config.cmd_tables_cpu,
                cmd_tables_phys: config.cmd_tables_phys,
                identify_cpu: config.identify_cpu,
                identify_phys: config.identify_phys,
            });
        }

        Err(last_err)
    }

    fn cmd_header_ptr(&self, slot: u32) -> *mut u8 {
        // 32-byte command header per slot.
        unsafe { self.cmd_list_cpu.add((slot as usize) * 32) }
    }

    fn cmd_table_ptr(&self, slot: u32) -> *mut u8 {
        // 256-byte command table (CFIS + ACMD + PRDT) per slot.
        unsafe { self.cmd_tables_cpu.add((slot as usize) * 256) }
    }

    fn cmd_table_phys(&self, slot: u32) -> u64 {
        self.cmd_tables_phys + (slot as u64) * 256
    }

    fn alloc_slot(&mut self) -> Option<u32> {
        for _ in 0..self.num_slots {
            let slot = self.next_slot as u32;
            self.next_slot = ((self.next_slot as u32 + 1) % self.num_slots) as u8;

            if !self.in_flight[slot as usize].active {
                return Some(slot);
            }
        }
        None
    }

    pub fn link_up(&self) -> bool {
        let det = unsafe { asm_ahci_port_detect(self.abar, self.port_num) };
        det == DET_PHY_COMM
    }

    pub fn port(&self) -> u32 {
        self.port_num
    }

    pub fn version(&self) -> (u8, u8) {
        let vs = unsafe { asm_ahci_read_version(self.abar) };
        let major = ((vs >> 16) & 0xFF) as u8;
        let minor = ((vs >> 8) & 0xFF) as u8;
        (major, minor)
    }
}

impl BlockDriver for AhciDriver {
    fn info(&self) -> BlockDeviceInfo {
        self.info
    }

    fn can_submit(&self) -> bool {
        self.in_flight.iter().any(|s| !s.active)
    }

    fn submit_read(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError> {
        if sector + num_sectors as u64 > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }
        if num_sectors > self.info.max_sectors_per_request {
            return Err(BlockError::RequestTooLarge);
        }

        let slot = self.alloc_slot().ok_or(BlockError::QueueFull)?;

        let result = unsafe {
            asm_ahci_submit_read(
                self.abar,
                self.port_num,
                sector,
                buffer_phys,
                num_sectors,
                slot,
                self.cmd_header_ptr(slot) as u64,
                self.cmd_table_ptr(slot) as u64,
                self.cmd_table_phys(slot),
            )
        };

        if result != 0 {
            return Err(BlockError::DeviceError);
        }

        self.in_flight[slot as usize] = InFlightRequest {
            request_id,
            slot: slot as u8,
            active: true,
        };

        Ok(())
    }

    fn submit_write(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError> {
        if self.info.read_only {
            return Err(BlockError::ReadOnly);
        }

        if sector + num_sectors as u64 > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }
        if num_sectors > self.info.max_sectors_per_request {
            return Err(BlockError::RequestTooLarge);
        }

        let slot = self.alloc_slot().ok_or(BlockError::QueueFull)?;

        let result = unsafe {
            asm_ahci_submit_write(
                self.abar,
                self.port_num,
                sector,
                buffer_phys,
                num_sectors,
                slot,
                self.cmd_header_ptr(slot) as u64,
                self.cmd_table_ptr(slot) as u64,
                self.cmd_table_phys(slot),
            )
        };

        if result != 0 {
            return Err(BlockError::DeviceError);
        }

        self.in_flight[slot as usize] = InFlightRequest {
            request_id,
            slot: slot as u8,
            active: true,
        };

        Ok(())
    }

    fn poll_completion(&mut self) -> Option<BlockCompletion> {
        for slot in 0..self.num_slots as usize {
            if !self.in_flight[slot].active {
                continue;
            }

            let slot_mask = 1u32 << slot;
            let status =
                unsafe { asm_ahci_check_cmd_complete(self.abar, self.port_num, slot_mask) };

            if status == 0 {
                continue;
            }

            let request_id = self.in_flight[slot].request_id;

            let bytes_transferred =
                unsafe { asm_ahci_read_prdbc(self.cmd_header_ptr(slot as u32) as u64) };

            self.in_flight[slot].active = false;

            unsafe {
                asm_ahci_port_clear_is(self.abar, self.port_num, 0xFFFFFFFF);
            }

            // status: 1 = ok, anything else = error.
            let completion = BlockCompletion {
                request_id,
                status: if status == 1 { 0 } else { 1 },
                bytes_transferred,
            };

            return Some(completion);
        }

        None
    }

    fn notify(&mut self) {
        // AHCI issues immediately on slot bit write; no doorbell needed.
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let slot = self.alloc_slot().ok_or(BlockError::QueueFull)?;

        unsafe {
            let result = asm_ahci_flush_cache(
                self.abar,
                self.port_num,
                slot,
                self.cmd_header_ptr(slot) as u64,
                self.cmd_table_ptr(slot) as u64,
                self.cmd_table_phys(slot),
            );

            if result != 0 {
                return Err(BlockError::DeviceError);
            }

            let slot_mask = 1u32 << slot;
            // 30 s — FLUSH CACHE can stall on large dirty queues.
            let poll_result =
                asm_ahci_poll_cmd(self.abar, self.port_num, slot_mask, self.tsc_freq, 30000);

            if poll_result != 0 {
                return Err(BlockError::Timeout);
            }
        }

        Ok(())
    }
}

impl BlockDriverInit for AhciDriver {
    type Error = AhciInitError;
    type Config = AhciConfig;

    fn supported_vendors() -> &'static [u16] {
        &[INTEL_VENDOR_ID]
    }

    fn supported_devices() -> &'static [u16] {
        AHCI_DEVICE_IDS
    }

    unsafe fn create(abar: u64, config: Self::Config) -> Result<Self, Self::Error> {
        Self::new(abar, config)
    }
}

// SAFETY: raw pointers are owned, never aliased across threads.
unsafe impl Send for AhciDriver {}
