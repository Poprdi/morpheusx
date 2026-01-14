//! Intel e1000e ASM bindings.
//!
//! Complete bindings for Intel e1000e device initialization and data path.
//!
//! All hardware access is done via these ASM functions. The Rust driver
//! layer only handles orchestration and logic.
//!
//! # Reference
//! Intel 82579 Datasheet, NETWORK_IMPL_GUIDE.md §2, §4

// ═══════════════════════════════════════════════════════════════════════════
// Result Types
// ═══════════════════════════════════════════════════════════════════════════

/// Result from RX poll operation.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RxPollResult {
    /// Packet length in bytes.
    pub length: u16,
    /// Status byte (DD, EOP, etc.).
    pub status: u8,
    /// Error byte (CE, SE, etc.).
    pub errors: u8,
}

impl RxPollResult {
    /// Descriptor done bit.
    pub const STA_DD: u8 = 1 << 0;
    /// End of packet bit.
    pub const STA_EOP: u8 = 1 << 1;

    /// CRC error bit.
    pub const ERR_CE: u8 = 1 << 0;
    /// Symbol error bit.
    pub const ERR_SE: u8 = 1 << 1;
    /// Sequence error bit.
    pub const ERR_SEQ: u8 = 1 << 2;
    /// RX data error bit.
    pub const ERR_RXE: u8 = 1 << 5;
    /// All error bits mask.
    pub const ERR_MASK: u8 = Self::ERR_CE | Self::ERR_SE | Self::ERR_SEQ | Self::ERR_RXE;

    /// Check if descriptor is done.
    #[inline]
    pub fn is_done(&self) -> bool {
        self.status & Self::STA_DD != 0
    }

    /// Check if end of packet.
    #[inline]
    pub fn is_eop(&self) -> bool {
        self.status & Self::STA_EOP != 0
    }

    /// Check if packet has errors.
    #[inline]
    pub fn has_errors(&self) -> bool {
        self.errors & Self::ERR_MASK != 0
    }
}

/// Link status result.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LinkStatusResult {
    /// Link is up (0/1).
    pub link_up: u8,
    /// Full duplex mode (0/1).
    pub full_duplex: u8,
    /// Speed: 0=10Mbps, 1=100Mbps, 2=1000Mbps.
    pub speed: u8,
}

impl LinkStatusResult {
    /// Speed value for 10 Mbps.
    pub const SPEED_10: u8 = 0;
    /// Speed value for 100 Mbps.
    pub const SPEED_100: u8 = 1;
    /// Speed value for 1000 Mbps.
    pub const SPEED_1000: u8 = 2;

    /// Check if link is up.
    #[inline]
    pub fn is_link_up(&self) -> bool {
        self.link_up != 0
    }

    /// Check if full duplex.
    #[inline]
    pub fn is_full_duplex(&self) -> bool {
        self.full_duplex != 0
    }

    /// Get speed in Mbps.
    #[inline]
    pub fn speed_mbps(&self) -> u32 {
        match self.speed {
            Self::SPEED_10 => 10,
            Self::SPEED_100 => 100,
            Self::SPEED_1000 => 1000,
            _ => 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Initialization Functions
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Reset the device and wait for completion.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `tsc_freq`: TSC frequency in ticks per second
    ///
    /// # Returns
    /// - 0: Success
    /// - 1: Timeout (reset did not complete in 100ms)
    ///
    /// # Safety
    /// `mmio_base` must be a valid, mapped MMIO address.
    pub fn asm_intel_reset(mmio_base: u64, tsc_freq: u64) -> u32;

    /// Read the STATUS register.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_read_status(mmio_base: u64) -> u32;

    /// Read MAC address from RAL/RAH registers.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `mac_out`: Pointer to 6-byte buffer for MAC address
    ///
    /// # Returns
    /// - 0: Success
    /// - 1: MAC address invalid (all zeros or all ones)
    ///
    /// # Safety
    /// Both pointers must be valid.
    pub fn asm_intel_read_mac(mmio_base: u64, mac_out: *mut [u8; 6]) -> u32;

    /// Write MAC address to RAL/RAH registers.
    ///
    /// # Safety
    /// Both pointers must be valid.
    pub fn asm_intel_write_mac(mmio_base: u64, mac: *const [u8; 6]);

    /// Clear multicast table array (128 entries).
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_clear_mta(mmio_base: u64);

    /// Disable all interrupts.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_disable_interrupts(mmio_base: u64);

    /// Set up RX descriptor ring.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `ring_bus_addr`: Bus address of descriptor ring
    /// - `ring_len_bytes`: Ring length in bytes
    ///
    /// # Safety
    /// All addresses must be valid.
    pub fn asm_intel_setup_rx_ring(mmio_base: u64, ring_bus_addr: u64, ring_len_bytes: u32);

    /// Set up TX descriptor ring.
    ///
    /// # Safety
    /// All addresses must be valid.
    pub fn asm_intel_setup_tx_ring(mmio_base: u64, ring_bus_addr: u64, ring_len_bytes: u32);

    /// Enable receiver with standard configuration.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_enable_rx(mmio_base: u64);

    /// Enable transmitter with standard configuration.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_enable_tx(mmio_base: u64);

    /// Force link up via CTRL register.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_set_link_up(mmio_base: u64);

    /// Generic register read.
    ///
    /// # Safety
    /// `mmio_base` and offset must be valid.
    pub fn asm_intel_read_reg(mmio_base: u64, offset: u32) -> u32;

    /// Generic register write.
    ///
    /// # Safety
    /// `mmio_base` and offset must be valid.
    pub fn asm_intel_write_reg(mmio_base: u64, offset: u32, value: u32);
}

// ═══════════════════════════════════════════════════════════════════════════
// TX Functions
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Initialize a TX descriptor to zero.
    ///
    /// # Safety
    /// `desc_ptr` must point to a valid 16-byte descriptor.
    pub fn asm_intel_tx_init_desc(desc_ptr: *mut u8);

    /// Submit a packet for transmission.
    ///
    /// Sets EOP, IFCS, RS command bits. Includes sfence.
    ///
    /// # Arguments
    /// - `desc_ptr`: Pointer to 16-byte descriptor
    /// - `buffer_bus_addr`: Bus address of packet buffer
    /// - `length`: Packet length in bytes
    ///
    /// # Safety
    /// All pointers must be valid.
    pub fn asm_intel_tx_submit(desc_ptr: *mut u8, buffer_bus_addr: u64, length: u32);

    /// Poll a TX descriptor for completion.
    ///
    /// # Returns
    /// - 1: Descriptor done (DD bit set)
    /// - 0: Not done
    ///
    /// # Safety
    /// `desc_ptr` must be valid.
    pub fn asm_intel_tx_poll(desc_ptr: *const u8) -> u32;

    /// Update TDT register (includes sfence).
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_tx_update_tail(mmio_base: u64, tail: u32);

    /// Read TDH register (head pointer).
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_tx_read_head(mmio_base: u64) -> u32;

    /// Clear DD bit in descriptor for reuse.
    ///
    /// # Safety
    /// `desc_ptr` must be valid.
    pub fn asm_intel_tx_clear_desc(desc_ptr: *mut u8);
}

// ═══════════════════════════════════════════════════════════════════════════
// RX Functions
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Initialize an RX descriptor with buffer address.
    ///
    /// # Safety
    /// `desc_ptr` must point to a valid 16-byte descriptor.
    pub fn asm_intel_rx_init_desc(desc_ptr: *mut u8, buffer_bus_addr: u64);

    /// Poll an RX descriptor for received packet.
    ///
    /// # Arguments
    /// - `desc_ptr`: Pointer to 16-byte descriptor
    /// - `result`: Pointer to RxPollResult struct
    ///
    /// # Returns
    /// - 1: Packet received (result populated)
    /// - 0: No packet
    ///
    /// # Safety
    /// All pointers must be valid.
    pub fn asm_intel_rx_poll(desc_ptr: *const u8, result: *mut RxPollResult) -> u32;

    /// Update RDT register (includes sfence).
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_rx_update_tail(mmio_base: u64, tail: u32);

    /// Read RDH register (head pointer).
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_rx_read_head(mmio_base: u64) -> u32;

    /// Clear descriptor status for reuse.
    ///
    /// # Safety
    /// `desc_ptr` must be valid.
    pub fn asm_intel_rx_clear_desc(desc_ptr: *mut u8);

    /// Get packet length from descriptor.
    ///
    /// # Safety
    /// `desc_ptr` must be valid.
    pub fn asm_intel_rx_get_length(desc_ptr: *const u8) -> u16;

    /// Check if descriptor has errors.
    ///
    /// # Returns
    /// - 0: No errors
    /// - Non-zero: Error bits
    ///
    /// # Safety
    /// `desc_ptr` must be valid.
    pub fn asm_intel_rx_check_errors(desc_ptr: *const u8) -> u32;
}

// ═══════════════════════════════════════════════════════════════════════════
// PHY Functions
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Read PHY register via MDIC.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `reg`: PHY register address (0-31)
    /// - `tsc_freq`: TSC frequency for timeout
    ///
    /// # Returns
    /// - Register value (16-bit) on success
    /// - 0xFFFFFFFF on error or timeout
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_phy_read(mmio_base: u64, reg: u32, tsc_freq: u64) -> u32;

    /// Write PHY register via MDIC.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `reg`: PHY register address (0-31)
    /// - `value`: Value to write (16-bit)
    /// - `tsc_freq`: TSC frequency for timeout
    ///
    /// # Returns
    /// - 0: Success
    /// - 1: Error or timeout
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_phy_write(mmio_base: u64, reg: u32, value: u32, tsc_freq: u64) -> u32;

    /// Get link status from STATUS register (fast path).
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `result`: Pointer to LinkStatusResult struct
    ///
    /// # Returns
    /// - 1: Link up
    /// - 0: Link down
    ///
    /// # Safety
    /// Both pointers must be valid.
    pub fn asm_intel_link_status(mmio_base: u64, result: *mut LinkStatusResult) -> u32;

    /// Wait for link up with timeout.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `timeout_us`: Timeout in microseconds
    /// - `tsc_freq`: TSC frequency
    ///
    /// # Returns
    /// - 0: Link came up
    /// - 1: Timeout
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_wait_link(mmio_base: u64, timeout_us: u64, tsc_freq: u64) -> u32;
}

// ═══════════════════════════════════════════════════════════════════════════
// I218/PCH LPT ULP (Ultra Low Power) Functions
// These are CRITICAL for real I218 hardware (ThinkPad T450s, etc.)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Disable Ultra Low Power (ULP) mode on I218 PHY.
    ///
    /// This is CRITICAL for I218-LM/V (ThinkPad T450s, etc.) - the PHY may be
    /// in ULP mode after BIOS handoff and won't respond to MDIC until ULP is
    /// disabled.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `tsc_freq`: TSC frequency for timeout
    ///
    /// # Returns
    /// - 0: Success
    /// - 1: Timeout or error
    ///
    /// # Reference
    /// Linux kernel e1000_disable_ulp_lpt_lp() in ich8lan.c
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_disable_ulp(mmio_base: u64, tsc_freq: u64) -> u32;

    /// Power cycle PHY via LANPHYPC control bits.
    ///
    /// Used when PHY is completely unresponsive. Toggles the LANPHYPC signal
    /// to force a power cycle of the PHY hardware.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `tsc_freq`: TSC frequency for timeout
    ///
    /// # Returns
    /// - 0: Success
    /// - 1: Timeout
    ///
    /// # Reference
    /// Linux kernel e1000_toggle_lanphypc_pch_lpt() in ich8lan.c
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_toggle_lanphypc(mmio_base: u64, tsc_freq: u64) -> u32;

    /// Check if PHY responds to MDIC reads.
    ///
    /// Reads PHY_ID1 to verify PHY is accessible. Returns success if the
    /// read returns a valid (non-0xFFFF) value.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `tsc_freq`: TSC frequency for timeout
    ///
    /// # Returns
    /// - 1: PHY is accessible
    /// - 0: PHY is not accessible
    ///
    /// # Reference
    /// Linux kernel e1000_phy_is_accessible_pchlan() in ich8lan.c
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_phy_is_accessible(mmio_base: u64, tsc_freq: u64) -> u32;

    /// Acquire hardware semaphore (EXTCNF_CTRL.SWFLAG).
    ///
    /// Must be called before PHY or NVM access on ICH8/ICH9/ICH10/PCH.
    /// This prevents concurrent access from multiple software components.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `tsc_freq`: TSC frequency for timeout
    ///
    /// # Returns
    /// - 0: Semaphore acquired successfully
    /// - 1: Timeout (semaphore not acquired)
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_acquire_swflag(mmio_base: u64, tsc_freq: u64) -> u32;

    /// Release hardware semaphore (EXTCNF_CTRL.SWFLAG).
    ///
    /// Must be called after completing PHY or NVM access.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_release_swflag(mmio_base: u64);

    /// Force SMBus mode for PHY access.
    ///
    /// Some I218 variants require SMBus mode for PHY access. Used as
    /// a fallback if MDIC reads fail.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_force_smbus_mode(mmio_base: u64);

    /// Clear SMBus mode for PHY access.
    ///
    /// Restores normal MDIO mode after SMBus mode was forced.
    ///
    /// # Safety
    /// `mmio_base` must be valid.
    pub fn asm_intel_clear_smbus_mode(mmio_base: u64);
}

// ═══════════════════════════════════════════════════════════════════════════
// Safe Wrappers
// ═══════════════════════════════════════════════════════════════════════════

/// Reset the device.
///
/// # Returns
/// - `Ok(())`: Reset successful
/// - `Err(())`: Reset timeout
#[inline]
pub fn reset(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_reset(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Read MAC address.
///
/// # Returns
/// - `Some([u8; 6])`: Valid MAC address
/// - `None`: MAC address invalid
#[inline]
pub fn read_mac(mmio_base: u64) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let result = unsafe { asm_intel_read_mac(mmio_base, &mut mac) };
    if result == 0 {
        Some(mac)
    } else {
        None
    }
}

/// Write MAC address.
#[inline]
pub fn write_mac(mmio_base: u64, mac: &[u8; 6]) {
    unsafe { asm_intel_write_mac(mmio_base, mac) };
}

/// Get link status.
#[inline]
pub fn get_link_status(mmio_base: u64) -> LinkStatusResult {
    let mut result = LinkStatusResult::default();
    unsafe { asm_intel_link_status(mmio_base, &mut result) };
    result
}

/// Wait for link with timeout.
///
/// # Returns
/// - `Ok(())`: Link came up
/// - `Err(())`: Timeout
#[inline]
pub fn wait_for_link(mmio_base: u64, timeout_us: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_wait_link(mmio_base, timeout_us, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Read PHY register.
///
/// # Returns
/// - `Some(u16)`: Register value
/// - `None`: Error or timeout
#[inline]
pub fn phy_read(mmio_base: u64, reg: u32, tsc_freq: u64) -> Option<u16> {
    let result = unsafe { asm_intel_phy_read(mmio_base, reg, tsc_freq) };
    if result != 0xFFFFFFFF {
        Some(result as u16)
    } else {
        None
    }
}

/// Write PHY register.
///
/// # Returns
/// - `Ok(())`: Success
/// - `Err(())`: Error or timeout
#[inline]
pub fn phy_write(mmio_base: u64, reg: u32, value: u16, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_phy_write(mmio_base, reg, value as u32, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// I218/PCH LPT ULP Safe Wrappers
// ═══════════════════════════════════════════════════════════════════════════

/// Disable Ultra Low Power (ULP) mode on I218 PHY.
///
/// CRITICAL for I218-LM/V on ThinkPad T450s, etc.
///
/// # Returns
/// - `Ok(())`: ULP disabled successfully
/// - `Err(())`: Timeout or error
#[inline]
pub fn disable_ulp(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_disable_ulp(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Power cycle PHY via LANPHYPC control bits.
///
/// Used when PHY is completely unresponsive.
///
/// # Returns
/// - `Ok(())`: PHY power cycled successfully
/// - `Err(())`: Timeout
#[inline]
pub fn toggle_lanphypc(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_toggle_lanphypc(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Check if PHY responds to MDIC reads.
///
/// # Returns
/// `true` if PHY is accessible, `false` otherwise.
#[inline]
pub fn phy_is_accessible(mmio_base: u64, tsc_freq: u64) -> bool {
    let result = unsafe { asm_intel_phy_is_accessible(mmio_base, tsc_freq) };
    result != 0
}

/// Acquire hardware semaphore for PHY/NVM access.
///
/// # Returns
/// - `Ok(())`: Semaphore acquired
/// - `Err(())`: Timeout
#[inline]
pub fn acquire_swflag(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_acquire_swflag(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Release hardware semaphore.
#[inline]
pub fn release_swflag(mmio_base: u64) {
    unsafe { asm_intel_release_swflag(mmio_base) };
}

/// Force SMBus mode for PHY access.
#[inline]
pub fn force_smbus_mode(mmio_base: u64) {
    unsafe { asm_intel_force_smbus_mode(mmio_base) };
}

/// Clear SMBus mode for PHY access.
#[inline]
pub fn clear_smbus_mode(mmio_base: u64) {
    unsafe { asm_intel_clear_smbus_mode(mmio_base) };
}
