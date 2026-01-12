//! Intel e1000e PHY management.
//!
//! Rust orchestration layer for PHY operations.
//! All hardware access is via ASM bindings.
//!
//! # Reference
//! Intel 82579 Datasheet, Section 8 (PHY)

use crate::asm::drivers::intel::{
    asm_intel_link_status, asm_intel_phy_read, asm_intel_phy_write, asm_intel_wait_link,
    LinkStatusResult,
};

use super::regs;

// ═══════════════════════════════════════════════════════════════════════════
// LINK STATUS
// ═══════════════════════════════════════════════════════════════════════════

/// Link speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSpeed {
    /// 10 Mbps.
    Speed10,
    /// 100 Mbps.
    Speed100,
    /// 1000 Mbps (1 Gbps).
    Speed1000,
    /// Unknown speed.
    Unknown,
}

impl LinkSpeed {
    /// Get speed in Mbps.
    pub fn mbps(&self) -> u32 {
        match self {
            LinkSpeed::Speed10 => 10,
            LinkSpeed::Speed100 => 100,
            LinkSpeed::Speed1000 => 1000,
            LinkSpeed::Unknown => 0,
        }
    }
}

/// Link status information.
#[derive(Debug, Clone, Copy)]
pub struct LinkStatus {
    /// Link is up.
    pub link_up: bool,
    /// Full duplex mode.
    pub full_duplex: bool,
    /// Link speed.
    pub speed: LinkSpeed,
}

impl Default for LinkStatus {
    fn default() -> Self {
        Self {
            link_up: false,
            full_duplex: false,
            speed: LinkSpeed::Unknown,
        }
    }
}

impl From<LinkStatusResult> for LinkStatus {
    fn from(result: LinkStatusResult) -> Self {
        let speed = match result.speed {
            0 => LinkSpeed::Speed10,
            1 => LinkSpeed::Speed100,
            2 => LinkSpeed::Speed1000,
            _ => LinkSpeed::Unknown,
        };

        Self {
            link_up: result.link_up != 0,
            full_duplex: result.full_duplex != 0,
            speed,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PHY MANAGER
// ═══════════════════════════════════════════════════════════════════════════

/// PHY manager for an e1000e device.
pub struct PhyManager {
    /// MMIO base address.
    mmio_base: u64,
    /// TSC frequency for timeouts.
    tsc_freq: u64,
    /// Cached link status.
    cached_status: LinkStatus,
}

impl PhyManager {
    /// Create a new PHY manager.
    pub fn new(mmio_base: u64, tsc_freq: u64) -> Self {
        Self {
            mmio_base,
            tsc_freq,
            cached_status: LinkStatus::default(),
        }
    }

    /// Get current link status (fast path via STATUS register).
    ///
    /// This is a quick check that doesn't access the PHY directly.
    pub fn link_status(&mut self) -> LinkStatus {
        let mut result = LinkStatusResult::default();
        unsafe {
            asm_intel_link_status(self.mmio_base, &mut result);
        }
        self.cached_status = LinkStatus::from(result);
        self.cached_status
    }

    /// Check if link is up.
    #[inline]
    pub fn is_link_up(&mut self) -> bool {
        self.link_status().link_up
    }

    /// Get cached link status (no hardware access).
    #[inline]
    pub fn cached_link_status(&self) -> LinkStatus {
        self.cached_status
    }

    /// Wait for link to come up.
    ///
    /// # Arguments
    /// - `timeout_us`: Timeout in microseconds
    ///
    /// # Returns
    /// - `Ok(LinkStatus)`: Link came up
    /// - `Err(())`: Timeout
    pub fn wait_for_link(&mut self, timeout_us: u64) -> Result<LinkStatus, ()> {
        let result = unsafe { asm_intel_wait_link(self.mmio_base, timeout_us, self.tsc_freq) };

        if result == 0 {
            // Link came up - get status
            Ok(self.link_status())
        } else {
            Err(())
        }
    }

    /// Read a PHY register.
    ///
    /// # Arguments
    /// - `reg`: PHY register address (0-31)
    ///
    /// # Returns
    /// - `Some(value)`: Register value
    /// - `None`: Error or timeout
    pub fn read_reg(&self, reg: u32) -> Option<u16> {
        let result = unsafe { asm_intel_phy_read(self.mmio_base, reg, self.tsc_freq) };
        if result != 0xFFFFFFFF {
            Some(result as u16)
        } else {
            None
        }
    }

    /// Write a PHY register.
    ///
    /// # Arguments
    /// - `reg`: PHY register address (0-31)
    /// - `value`: Value to write
    ///
    /// # Returns
    /// - `Ok(())`: Success
    /// - `Err(())`: Error or timeout
    pub fn write_reg(&self, reg: u32, value: u16) -> Result<(), ()> {
        let result =
            unsafe { asm_intel_phy_write(self.mmio_base, reg, value as u32, self.tsc_freq) };
        if result == 0 {
            Ok(())
        } else {
            Err(())
        }
    }

    /// Read PHY identifier.
    ///
    /// # Returns
    /// - `Some((id1, id2))`: PHY ID registers
    /// - `None`: Error reading PHY
    pub fn read_phy_id(&self) -> Option<(u16, u16)> {
        let id1 = self.read_reg(regs::PHY_PHYID1)?;
        let id2 = self.read_reg(regs::PHY_PHYID2)?;
        Some((id1, id2))
    }

    /// Read BMSR (Basic Mode Status Register).
    ///
    /// This includes link status and auto-negotiation complete bits.
    pub fn read_bmsr(&self) -> Option<u16> {
        self.read_reg(regs::PHY_BMSR)
    }

    /// Check if auto-negotiation is complete.
    pub fn is_autoneg_complete(&self) -> bool {
        self.read_bmsr()
            .map(|bmsr| bmsr & regs::BMSR_ANEGCOMPLETE != 0)
            .unwrap_or(false)
    }

    /// Restart auto-negotiation.
    pub fn restart_autoneg(&self) -> Result<(), ()> {
        // Read current BMCR
        let bmcr = self.read_reg(regs::PHY_BMCR).ok_or(())?;

        // Set auto-negotiation enable and restart bits
        let new_bmcr = bmcr | regs::BMCR_ANENABLE | regs::BMCR_ANRESTART;

        self.write_reg(regs::PHY_BMCR, new_bmcr)
    }

    /// Software reset the PHY.
    pub fn reset(&self) -> Result<(), ()> {
        // Set reset bit in BMCR
        self.write_reg(regs::PHY_BMCR, regs::BMCR_RESET)?;

        // The PHY should clear the reset bit when done
        // We don't wait here - caller should poll or use wait_for_link
        Ok(())
    }
}

// Safety: PhyManager only contains raw values, no references
unsafe impl Send for PhyManager {}
