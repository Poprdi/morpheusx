//! e1000e brutal-reset init. 82579 datasheet §14 and Linux ich8lan.c.
//!
//! 10 phases: mask IRQ -> disable RX/TX -> GIO master disable -> device reset ->
//! wait EEPROM -> cleanup -> PCH workarounds (ULP, PHY access, PHY wake) ->
//! validate MAC -> program rings -> re-enable. Every MMIO write is flushed
//! via STATUS read; every poll is TSC-bounded; interrupts stay masked (polled).

use crate::asm::{
    asm_intel_clear_mta, asm_intel_enable_rx, asm_intel_enable_tx, asm_intel_read_mac,
    asm_intel_reset, asm_intel_set_link_up, asm_intel_setup_rx_ring, asm_intel_setup_tx_ring,
    asm_intel_write_mac, disable_ulp, phy_is_accessible, phy_read, phy_write, toggle_lanphypc,
};
use crate::serial::{serial_print, serial_print_decimal, serial_println};
use crate::traits::MacAddress;
use morpheus_virtio::dma::DmaRegion;

use super::regs;
use super::rx::RxRing;
use super::tx::TxRing;

#[derive(Debug, Clone)]
pub struct E1000eConfig {
    pub rx_queue_size: u16,
    pub tx_queue_size: u16,
    pub buffer_size: usize,
    pub tsc_freq: u64,
    pub dma_cpu_base: *mut u8,
    pub dma_bus_base: u64,
}

impl E1000eConfig {
    /// # Safety
    /// DMA pointers must be valid for the lifetime of the driver.
    pub unsafe fn new(dma_cpu_base: *mut u8, dma_bus_base: u64, tsc_freq: u64) -> Self {
        Self {
            rx_queue_size: regs::DEFAULT_QUEUE_SIZE,
            tx_queue_size: regs::DEFAULT_QUEUE_SIZE,
            buffer_size: regs::DEFAULT_BUFFER_SIZE,
            tsc_freq,
            dma_cpu_base,
            dma_bus_base,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E1000eInitError {
    ResetTimeout,
    InvalidMac,
    MmioError,
    LinkTimeout,
    /// I218 ULP exit failed.
    UlpDisableFailed,
    PhyNotAccessible,
    SemaphoreTimeout,
}

pub struct E1000eInitResult {
    pub mac: MacAddress,
    pub rx_ring: RxRing,
    pub tx_ring: TxRing,
}

/// # Safety
/// `mmio_base` must be the device BAR0; config DMA region must be valid.
pub unsafe fn init_e1000e(
    mmio_base: u64,
    config: &E1000eConfig,
) -> Result<E1000eInitResult, E1000eInitError> {
    use morpheus_hal_x86_64::asm::mmio::{read32, write32};

    serial_println("  [e1000e] === BRUTAL RESET INIT ===");

    serial_println("  [e1000e] Phase 1: Mask/clear interrupts");

    write32(mmio_base + regs::IMC as u64, regs::INT_MASK_ALL);
    let _ = read32(mmio_base + regs::STATUS as u64);

    // ICR is read-to-clear.
    let _ = read32(mmio_base + regs::ICR as u64);

    serial_println("  [e1000e] Phase 2: Disable RX/TX, wait for quiescence");

    let rctl = read32(mmio_base + regs::RCTL as u64);
    write32(mmio_base + regs::RCTL as u64, rctl & !regs::RCTL_EN);
    let _ = read32(mmio_base + regs::STATUS as u64);

    let tctl = read32(mmio_base + regs::TCTL as u64);
    write32(mmio_base + regs::TCTL as u64, tctl & !regs::TCTL_EN);
    let _ = read32(mmio_base + regs::STATUS as u64);

    let quiesce_timeout = config.tsc_freq / 100;
    let quiesce_start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    loop {
        let rxdctl = read32(mmio_base + regs::RXDCTL as u64);
        let txdctl = read32(mmio_base + regs::TXDCTL as u64);
        if (rxdctl & regs::XDCTL_QUEUE_ENABLE == 0) && (txdctl & regs::XDCTL_QUEUE_ENABLE == 0) {
            break;
        }
        if morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(quiesce_start) > quiesce_timeout {
            serial_println("  [e1000e] WARN: RX/TX quiesce timeout (continuing)");
            break;
        }
        core::hint::spin_loop();
    }

    serial_println("  [e1000e] Phase 3: Disable bus mastering");

    let ctrl = read32(mmio_base + regs::CTRL as u64);
    write32(
        mmio_base + regs::CTRL as u64,
        ctrl | regs::CTRL_GIO_MASTER_DISABLE,
    );
    let _ = read32(mmio_base + regs::STATUS as u64);

    let gio_timeout = config.tsc_freq / 100;
    let gio_start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    loop {
        let status = read32(mmio_base + regs::STATUS as u64);
        if status & regs::STATUS_GIO_MASTER_EN == 0 {
            break;
        }
        if morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(gio_start) > gio_timeout {
            serial_println("  [e1000e] WARN: GIO master disable timeout");
            break;
        }
        core::hint::spin_loop();
    }

    serial_println("  [e1000e] Phase 4: Device reset (MANDATORY)");

    let reset_result = asm_intel_reset(mmio_base, config.tsc_freq);
    if reset_result != 0 {
        serial_println("  [e1000e] FATAL: Reset timeout");
        return Err(E1000eInitError::ResetTimeout);
    }

    serial_println("  [e1000e] Reset complete, waiting for EEPROM auto-read");

    // 500 ms is generous; some parts are slow.
    let eecd_timeout = config.tsc_freq / 2;
    let eecd_start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    loop {
        let eecd = read32(mmio_base + regs::EECD as u64);
        if eecd & regs::EECD_AUTO_RD != 0 {
            break;
        }
        if morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(eecd_start) > eecd_timeout {
            serial_println("  [e1000e] WARN: EEPROM auto-read timeout");
            break;
        }
        core::hint::spin_loop();
    }

    serial_println("  [e1000e] Phase 6: Post-reset cleanup");

    // Reset may re-arm interrupts; remask.
    write32(mmio_base + regs::IMC as u64, regs::INT_MASK_ALL);
    let _ = read32(mmio_base + regs::ICR as u64);
    let _ = read32(mmio_base + regs::STATUS as u64);

    // Zero all ring pointers — no stale DMA on resume from BIOS.
    write32(mmio_base + regs::RDBAL as u64, 0);
    write32(mmio_base + regs::RDBAH as u64, 0);
    write32(mmio_base + regs::RDLEN as u64, 0);
    write32(mmio_base + regs::RDH as u64, 0);
    write32(mmio_base + regs::RDT as u64, 0);
    write32(mmio_base + regs::TDBAL as u64, 0);
    write32(mmio_base + regs::TDBAH as u64, 0);
    write32(mmio_base + regs::TDLEN as u64, 0);
    write32(mmio_base + regs::TDH as u64, 0);
    write32(mmio_base + regs::TDT as u64, 0);
    let _ = read32(mmio_base + regs::STATUS as u64);

    // RAR[0] is rewritten in Phase 8 — leave it for now (EEPROM path can be fragile).

    let rctl = read32(mmio_base + regs::RCTL as u64);
    write32(mmio_base + regs::RCTL as u64, rctl & !regs::RCTL_LBM_MASK);

    asm_intel_clear_mta(mmio_base);

    let _ = read32(mmio_base + regs::STATUS as u64);

    serial_println("  [e1000e] Phase 7: I218/PCH workarounds");

    // TODO: gate on PCH device IDs once detection is plumbed through config.
    let _ulp_result = disable_ulp(mmio_base, config.tsc_freq);

    if !ensure_phy_accessible(mmio_base, config.tsc_freq) {
        serial_println("  [e1000e] FATAL: PHY not accessible");
        return Err(E1000eInitError::PhyNotAccessible);
    }

    wake_phy(mmio_base, config.tsc_freq);

    serial_println("  [e1000e] Phase 8: Read MAC address");

    let mut mac: MacAddress = [0u8; 6];
    let mac_result = asm_intel_read_mac(mmio_base, &mut mac);

    if mac_result != 0 {
        serial_println("  [e1000e] FATAL: MAC read failed");
        return Err(E1000eInitError::InvalidMac);
    }

    if mac == [0, 0, 0, 0, 0, 0] || mac == [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF] {
        serial_println("  [e1000e] FATAL: MAC invalid (all 0s or FFs)");
        return Err(E1000eInitError::InvalidMac);
    }

    asm_intel_write_mac(mmio_base, &mac);
    let _ = read32(mmio_base + regs::STATUS as u64);

    serial_println("  [e1000e] Phase 9: Setup descriptor rings");

    let rx_desc_cpu = config.dma_cpu_base.add(DmaRegion::RX_DESC_OFFSET);
    let rx_desc_bus = config.dma_bus_base + DmaRegion::RX_DESC_OFFSET as u64;
    let rx_buffer_cpu = config.dma_cpu_base.add(DmaRegion::RX_BUFFERS_OFFSET);
    let rx_buffer_bus = config.dma_bus_base + DmaRegion::RX_BUFFERS_OFFSET as u64;

    let rx_ring_len_bytes = (config.rx_queue_size as u32) * (regs::DESC_SIZE as u32);

    asm_intel_setup_rx_ring(mmio_base, rx_desc_bus, rx_ring_len_bytes);

    let mut rx_ring = RxRing::new(
        mmio_base,
        rx_desc_cpu,
        rx_desc_bus,
        rx_buffer_cpu,
        rx_buffer_bus,
        config.buffer_size,
        config.rx_queue_size,
    );

    rx_ring.init_descriptors();

    let tx_desc_cpu = config.dma_cpu_base.add(DmaRegion::TX_DESC_OFFSET);
    let tx_desc_bus = config.dma_bus_base + DmaRegion::TX_DESC_OFFSET as u64;
    let tx_buffer_cpu = config.dma_cpu_base.add(DmaRegion::TX_BUFFERS_OFFSET);
    let tx_buffer_bus = config.dma_bus_base + DmaRegion::TX_BUFFERS_OFFSET as u64;

    let tx_ring_len_bytes = (config.tx_queue_size as u32) * (regs::DESC_SIZE as u32);

    asm_intel_setup_tx_ring(mmio_base, tx_desc_bus, tx_ring_len_bytes);

    let mut tx_ring = TxRing::new(
        mmio_base,
        tx_desc_cpu,
        tx_desc_bus,
        tx_buffer_cpu,
        tx_buffer_bus,
        config.buffer_size,
        config.tx_queue_size,
    );

    tx_ring.init_descriptors();

    let _ = read32(mmio_base + regs::STATUS as u64);

    serial_println("  [e1000e] Phase 10: Enable RX/TX, set link up");

    let ctrl = read32(mmio_base + regs::CTRL as u64);
    write32(
        mmio_base + regs::CTRL as u64,
        ctrl & !regs::CTRL_GIO_MASTER_DISABLE,
    );
    let _ = read32(mmio_base + regs::STATUS as u64);

    asm_intel_enable_rx(mmio_base);
    let _ = read32(mmio_base + regs::STATUS as u64);

    rx_ring.update_tail();
    let _ = read32(mmio_base + regs::STATUS as u64);

    asm_intel_enable_tx(mmio_base);
    let _ = read32(mmio_base + regs::STATUS as u64);

    asm_intel_set_link_up(mmio_base);

    if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, config.tsc_freq) {
        let new_bmcr = bmcr | regs::BMCR_ANENABLE | regs::BMCR_ANRESTART;
        let _ = phy_write(mmio_base, regs::PHY_BMCR, new_bmcr, config.tsc_freq);
    }

    // 100 ms for PHY to start negotiating.
    let delay_start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    let delay_ticks = config.tsc_freq / 10;
    while morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(delay_start) < delay_ticks {
        core::hint::spin_loop();
    }

    // IMS stays 0 — polled I/O.

    serial_println("  [e1000e] === INIT COMPLETE (interrupts masked, polled mode) ===");

    Ok(E1000eInitResult {
        mac,
        rx_ring,
        tx_ring,
    })
}

/// Generate a locally-administered MAC address from a seed.
pub fn generate_fallback_mac(seed: u64) -> MacAddress {
    let mut mac = [0u8; 6];
    let bytes = seed.to_le_bytes();

    // Set locally-administered bit, clear multicast bit
    mac[0] = (bytes[0] & 0xFE) | 0x02;
    mac[1] = bytes[1];
    mac[2] = bytes[2];
    mac[3] = bytes[3];
    mac[4] = bytes[4];
    mac[5] = bytes[5];

    mac
}

/// Probe PHY, escalating recovery on failure. CRITICAL for I218-LM/V (T450s):
/// after ULP disable the PHY may still ignore MDIC. Escalates wait -> LANPHYPC
/// power-cycle -> forced SMBus mode.
///
/// # Safety
/// Called during init, MMIO must be valid.
unsafe fn ensure_phy_accessible(mmio_base: u64, tsc_freq: u64) -> bool {
    const MAX_ATTEMPTS: u32 = 3;

    for attempt in 0..MAX_ATTEMPTS {
        serial_print("    PHY check attempt ");
        serial_print_decimal(attempt);
        serial_println("...");

        if phy_is_accessible(mmio_base, tsc_freq) {
            serial_println("    PHY accessible!");
            return true;
        }

        serial_println("    PHY not responding, trying recovery...");

        match attempt {
            0 => {
                // Some I218 variants just need more time after ULP disable.
                serial_println("    Recovery: waiting 50ms...");
                let start = morpheus_hal_x86_64::asm::tsc::read_tsc();
                let delay = tsc_freq / 20; // 50ms
                while morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(start) < delay {
                    core::hint::spin_loop();
                }
            },
            1 => {
                // Power-cycle the PHY.
                serial_println("    Recovery: toggling LANPHYPC...");
                let _ = toggle_lanphypc(mmio_base, tsc_freq);
            },
            2 => {
                // Last resort: some I218 only answer over SMBus.
                serial_println("    Recovery: SMBus mode + LANPHYPC...");
                crate::asm::force_smbus_mode(mmio_base);
                let _ = toggle_lanphypc(mmio_base, tsc_freq);
                crate::asm::clear_smbus_mode(mmio_base);
            },
            _ => {},
        }
    }

    serial_println("    Final PHY check...");
    phy_is_accessible(mmio_base, tsc_freq)
}

/// Clear PDOWN, reset the PHY, restart auto-negotiation. BIOS may leave the PHY
/// in BMCR.PDOWN; post-EBS there is no ACPI/SMM to wake it, so do it by hand.
/// Real-hardware timing is mandatory (QEMU tolerates skipping it).
///
/// # Safety
/// Called during init, MMIO must be valid.
unsafe fn wake_phy(mmio_base: u64, tsc_freq: u64) {
    if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
        if bmcr & regs::BMCR_PDOWN != 0 {
            let new_bmcr = bmcr & !regs::BMCR_PDOWN;
            let _ = phy_write(mmio_base, regs::PHY_BMCR, new_bmcr, tsc_freq);
        }
    }

    // ISOLATE can also block operation.
    if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
        if bmcr & regs::BMCR_ISOLATE != 0 {
            let new_bmcr = bmcr & !regs::BMCR_ISOLATE;
            let _ = phy_write(mmio_base, regs::PHY_BMCR, new_bmcr, tsc_freq);
        }
    }

    // Datasheet: 50-100ms after PDOWN clear for PLL lock + analog settle.
    let start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    let delay_ticks = tsc_freq / 10; // 100ms (not 1ms!)
    while morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(start) < delay_ticks {
        core::hint::spin_loop();
    }

    // Reset to a clean baseline after the BIOS handoff.
    let _ = phy_write(mmio_base, regs::PHY_BMCR, regs::BMCR_RESET, tsc_freq);

    // PHY clears RESET when done; some never do, so bound the wait.
    let reset_start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    let reset_timeout = tsc_freq / 2; // 500ms
    loop {
        if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
            if bmcr & regs::BMCR_RESET == 0 {
                break;
            }
        }
        if morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(reset_start) >= reset_timeout {
            break;
        }
        core::hint::spin_loop();
    }

    let post_reset_start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    let post_reset_delay = tsc_freq / 100; // 10ms
    while morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(post_reset_start)
        < post_reset_delay
    {
        core::hint::spin_loop();
    }

    // Restart auto-neg; without it link may never come up.
    if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
        let new_bmcr = bmcr | regs::BMCR_ANENABLE | regs::BMCR_ANRESTART;
        let _ = phy_write(mmio_base, regs::PHY_BMCR, new_bmcr, tsc_freq);
    }

    let autoneg_start = morpheus_hal_x86_64::asm::tsc::read_tsc();
    let autoneg_delay = tsc_freq / 100; // 10ms
    while morpheus_hal_x86_64::asm::tsc::read_tsc().wrapping_sub(autoneg_start) < autoneg_delay {
        core::hint::spin_loop();
    }
}
