//! AHCI (Advanced Host Controller Interface) block device driver.
//!
//! Provides native SATA block I/O for real hardware (ThinkPad T450s).
//!
//! # Target Hardware
//!
//! Intel Wildcat Point-LP SATA Controller [AHCI Mode]
//! - PCI Vendor: 0x8086 (Intel)
//! - PCI Device: 0x9C83
//! - Class: 0x010601 (Mass Storage, SATA, AHCI)
//!
//! # Architecture
//!
//! The driver follows MorpheusX's ASM-first pattern:
//! - All hardware access (MMIO, DMA structures) via hand-written assembly
//! - Rust code handles orchestration, state tracking, error handling
//! - Fire-and-forget command submission with poll-based completion
//!
//! # DMA Memory Layout
//!
//! Per-port DMA structures (must be properly aligned):
//! - Command List: 1KB aligned, 32 × 32-byte command headers
//! - FIS Receive: 256-byte aligned, 256 bytes
//! - Command Tables: 128-byte aligned, one per command slot
//!
//! # Reference
//!
//! - AHCI 1.3.1 Specification
//! - Intel PCH Datasheet
//! - ATA/ATAPI-8 Command Set

pub mod init;
pub mod port;
pub mod regs;

use crate::driver::block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
use core::ptr;

// Re-exports
pub use init::{AhciConfig, AhciInitError};

// ═══════════════════════════════════════════════════════════════════════════
// ASM BINDINGS
// ═══════════════════════════════════════════════════════════════════════════

extern "win64" {
    // HBA initialization
    fn asm_ahci_hba_reset(abar: u64, tsc_freq: u64) -> u32;
    fn asm_ahci_enable(abar: u64) -> u32;
    fn asm_ahci_read_cap(abar: u64) -> u32;
    fn asm_ahci_read_pi(abar: u64) -> u32;
    fn asm_ahci_read_version(abar: u64) -> u32;
    fn asm_ahci_disable_interrupts(abar: u64);
    fn asm_ahci_get_num_ports(abar: u64) -> u32;
    fn asm_ahci_get_num_cmd_slots(abar: u64) -> u32;
    fn asm_ahci_supports_64bit(abar: u64) -> u32;

    // Port operations
    fn asm_ahci_port_detect(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_stop(abar: u64, port_num: u32, tsc_freq: u64) -> u32;
    fn asm_ahci_port_start(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_setup(abar: u64, port_num: u32, clb_phys: u64, fb_phys: u64) -> u32;
    fn asm_ahci_port_clear_errors(abar: u64, port_num: u32);
    fn asm_ahci_port_read_sig(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_read_tfd(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_read_ssts(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_read_is(abar: u64, port_num: u32) -> u32;
    fn asm_ahci_port_clear_is(abar: u64, port_num: u32, bits: u32);
    fn asm_ahci_port_disable_interrupts(abar: u64, port_num: u32);

    // Command operations
    fn asm_ahci_setup_cmd_header(cmd_header_ptr: u64, flags: u32, ctba_phys: u64);
    fn asm_ahci_build_h2d_fis(fis_ptr: u64, command: u8, lba: u64, sector_count: u16);
    fn asm_ahci_build_prdt(prdt_ptr: u64, data_phys: u64, byte_count_minus_1: u32);
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

    // IDENTIFY
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

    // I/O operations
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

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Intel PCI Vendor ID
pub const INTEL_VENDOR_ID: u16 = 0x8086;

/// Wildcat Point-LP SATA Controller (ThinkPad T450s)
pub const AHCI_DEVICE_WPT_LP: u16 = 0x9C83;

/// Other supported AHCI device IDs
pub const AHCI_DEVICE_IDS: &[u16] = &[
    0x9C83, // Wildcat Point-LP (T450s)
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

/// PCI Class code for SATA AHCI controller
pub const PCI_CLASS_SATA_AHCI: u32 = 0x010601;

/// Maximum command slots we support
pub const MAX_CMD_SLOTS: usize = 32;

/// Detection status values
pub const DET_NONE: u32 = 0;
pub const DET_PRESENT: u32 = 1;
pub const DET_PHY_COMM: u32 = 3;

/// Device signatures
pub const SIG_ATA: u32 = 0x00000101;
pub const SIG_ATAPI: u32 = 0xEB140101;

/// ATA status bits
pub const ATA_STS_BSY: u8 = 1 << 7;
pub const ATA_STS_DRQ: u8 = 1 << 3;
pub const ATA_STS_ERR: u8 = 1 << 0;

// ═══════════════════════════════════════════════════════════════════════════
// REQUEST TRACKING
// ═══════════════════════════════════════════════════════════════════════════

/// Track an in-flight request
#[derive(Debug, Clone, Copy, Default)]
struct InFlightRequest {
    /// Caller's request ID
    request_id: u32,
    /// Command slot used
    slot: u8,
    /// Is this slot in use?
    active: bool,
}

// ═══════════════════════════════════════════════════════════════════════════
// DRIVER
// ═══════════════════════════════════════════════════════════════════════════

/// AHCI block device driver for real hardware.
///
/// Supports Intel SATA controllers in AHCI mode.
pub struct AhciDriver {
    /// AHCI Base Address Register (BAR5 mapped)
    abar: u64,
    /// Active port number (first detected SATA device)
    port_num: u32,
    /// Calibrated TSC frequency
    tsc_freq: u64,
    /// Device information
    info: BlockDeviceInfo,
    /// Number of available command slots
    num_slots: u32,
    /// In-flight request tracking
    in_flight: [InFlightRequest; MAX_CMD_SLOTS],
    /// Next slot to use (round-robin)
    next_slot: u8,
    /// DMA: Command List base (CPU pointer)
    cmd_list_cpu: *mut u8,
    /// DMA: Command List base (physical)
    cmd_list_phys: u64,
    /// DMA: FIS Receive buffer (CPU pointer)
    fis_cpu: *mut u8,
    /// DMA: FIS Receive buffer (physical)
    fis_phys: u64,
    /// DMA: Command Tables base (CPU pointer)
    cmd_tables_cpu: *mut u8,
    /// DMA: Command Tables base (physical)
    cmd_tables_phys: u64,
    /// DMA: IDENTIFY buffer (CPU pointer, 512 bytes)
    identify_cpu: *mut u8,
    /// DMA: IDENTIFY buffer (physical)
    identify_phys: u64,
}

impl AhciDriver {
    /// Create and initialize AHCI driver.
    ///
    /// # Safety
    /// - `abar` must be valid AHCI MMIO address (from BAR5)
    /// - `config` must contain valid DMA memory pointers/addresses
    pub unsafe fn new(abar: u64, config: AhciConfig) -> Result<Self, AhciInitError> {
        // Validate config
        if config.cmd_list_cpu.is_null() || config.fis_cpu.is_null() {
            return Err(AhciInitError::InvalidConfig);
        }

        let tsc_freq = config.tsc_freq;

        // ═══════════════════════════════════════════════════════════════════
        // STEP 1: Enable AHCI mode
        // ═══════════════════════════════════════════════════════════════════
        asm_ahci_enable(abar);

        // ═══════════════════════════════════════════════════════════════════
        // STEP 2: Read capabilities
        // ═══════════════════════════════════════════════════════════════════
        let cap = asm_ahci_read_cap(abar);
        let num_slots = asm_ahci_get_num_cmd_slots(abar);
        let ports_impl = asm_ahci_read_pi(abar);

        // Check 64-bit support
        let supports_64bit = asm_ahci_supports_64bit(abar) != 0;
        if !supports_64bit && (config.cmd_list_phys >> 32) != 0 {
            return Err(AhciInitError::No64BitSupport);
        }

        // ═══════════════════════════════════════════════════════════════════
        // STEP 3: Disable interrupts (we use polling)
        // ═══════════════════════════════════════════════════════════════════
        asm_ahci_disable_interrupts(abar);

        // ═══════════════════════════════════════════════════════════════════
        // STEP 4: Find first port with an attached ATA device
        // ═══════════════════════════════════════════════════════════════════
        let mut active_port: Option<u32> = None;

        for port in 0..32u32 {
            if (ports_impl & (1 << port)) == 0 {
                continue; // Port not implemented
            }

            let det = asm_ahci_port_detect(abar, port);
            if det != DET_PHY_COMM {
                continue; // No device or not ready
            }

            // Check signature
            let sig = asm_ahci_port_read_sig(abar, port);
            if sig == SIG_ATA {
                active_port = Some(port);
                break;
            }
        }

        let port_num = active_port.ok_or(AhciInitError::NoDeviceFound)?;

        // ═══════════════════════════════════════════════════════════════════
        // STEP 5: Stop port and configure DMA structures
        // ═══════════════════════════════════════════════════════════════════
        let stop_result = asm_ahci_port_stop(abar, port_num, tsc_freq);
        if stop_result != 0 {
            return Err(AhciInitError::PortStopTimeout);
        }

        // Setup command list and FIS receive buffer
        asm_ahci_port_setup(abar, port_num, config.cmd_list_phys, config.fis_phys);

        // Clear any pending errors
        asm_ahci_port_clear_errors(abar, port_num);
        asm_ahci_port_disable_interrupts(abar, port_num);

        // ═══════════════════════════════════════════════════════════════════
        // STEP 6: Start port
        // ═══════════════════════════════════════════════════════════════════
        asm_ahci_port_start(abar, port_num);

        // ═══════════════════════════════════════════════════════════════════
        // STEP 7: Issue IDENTIFY DEVICE to get capacity
        // ═══════════════════════════════════════════════════════════════════
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
            return Err(AhciInitError::IdentifyFailed);
        }

        // Parse IDENTIFY data
        let total_sectors = asm_ahci_get_identify_capacity(config.identify_cpu as u64);
        let sector_size = asm_ahci_get_identify_sector_size(config.identify_cpu as u64);

        let info = BlockDeviceInfo {
            total_sectors,
            sector_size,
            max_sectors_per_request: 256, // Conservative for DMA
            read_only: false,
        };

        Ok(Self {
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
        })
    }

    /// Get command header pointer for a slot
    fn cmd_header_ptr(&self, slot: u32) -> *mut u8 {
        // Each command header is 32 bytes
        unsafe { self.cmd_list_cpu.add((slot as usize) * 32) }
    }

    /// Get command table pointer for a slot
    fn cmd_table_ptr(&self, slot: u32) -> *mut u8 {
        // Each command table is 256 bytes (128 header + PRDTs)
        unsafe { self.cmd_tables_cpu.add((slot as usize) * 256) }
    }

    /// Get command table physical address for a slot
    fn cmd_table_phys(&self, slot: u32) -> u64 {
        self.cmd_tables_phys + (slot as u64) * 256
    }

    /// Allocate a command slot
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

    /// Check link status
    pub fn link_up(&self) -> bool {
        let det = unsafe { asm_ahci_port_detect(self.abar, self.port_num) };
        det == DET_PHY_COMM
    }

    /// Get port number being used
    pub fn port(&self) -> u32 {
        self.port_num
    }

    /// Get AHCI version
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
        // Validate
        if sector + num_sectors as u64 > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }
        if num_sectors > self.info.max_sectors_per_request {
            return Err(BlockError::RequestTooLarge);
        }

        // Allocate slot
        let slot = self.alloc_slot().ok_or(BlockError::QueueFull)?;

        // Submit via ASM
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

        // Track in-flight
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
        // Check read-only (shouldn't happen for AHCI, but be safe)
        if self.info.read_only {
            return Err(BlockError::ReadOnly);
        }

        // Validate
        if sector + num_sectors as u64 > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }
        if num_sectors > self.info.max_sectors_per_request {
            return Err(BlockError::RequestTooLarge);
        }

        // Allocate slot
        let slot = self.alloc_slot().ok_or(BlockError::QueueFull)?;

        // Submit via ASM
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

        // Track in-flight
        self.in_flight[slot as usize] = InFlightRequest {
            request_id,
            slot: slot as u8,
            active: true,
        };

        Ok(())
    }

    fn poll_completion(&mut self) -> Option<BlockCompletion> {
        // Check each active slot
        for slot in 0..self.num_slots as usize {
            if !self.in_flight[slot].active {
                continue;
            }

            let slot_mask = 1u32 << slot;
            let status = unsafe {
                asm_ahci_check_cmd_complete(self.abar, self.port_num, slot_mask)
            };

            if status == 0 {
                // Still pending
                continue;
            }

            let request_id = self.in_flight[slot].request_id;
            
            // Read bytes transferred from command header
            let bytes_transferred = unsafe {
                asm_ahci_read_prdbc(self.cmd_header_ptr(slot as u32) as u64)
            };

            // Mark slot as free
            self.in_flight[slot].active = false;

            // Clear interrupt status for this completion
            unsafe {
                asm_ahci_port_clear_is(self.abar, self.port_num, 0xFFFFFFFF);
            }

            let completion = BlockCompletion {
                request_id,
                status: if status == 1 { 0 } else { 1 }, // 1=complete, 2=error
                bytes_transferred,
            };

            return Some(completion);
        }

        None
    }

    fn notify(&mut self) {
        // AHCI doesn't need explicit notify - commands are issued immediately
        // This is a no-op for AHCI
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        // Use slot 0 for flush (should be available after draining queue)
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

            // Poll for completion (30 second timeout for flush)
            let slot_mask = 1u32 << slot;
            let poll_result = asm_ahci_poll_cmd(
                self.abar,
                self.port_num,
                slot_mask,
                self.tsc_freq,
                30000, // 30 seconds
            );

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

// Safety: AhciDriver only contains raw pointers that are not shared
unsafe impl Send for AhciDriver {}
