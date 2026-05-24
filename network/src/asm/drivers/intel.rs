//! Intel e1000e asm bindings. Rust does orchestration; all MMIO lives in asm/.
//! Datasheet 82579 + NETWORK_IMPL_GUIDE.md §2, §4.

/// RX descriptor poll result.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RxPollResult {
    pub length: u16,
    pub status: u8,
    pub errors: u8,
}

impl RxPollResult {
    pub const STA_DD: u8 = 1 << 0;
    pub const STA_EOP: u8 = 1 << 1;

    pub const ERR_CE: u8 = 1 << 0;
    pub const ERR_SE: u8 = 1 << 1;
    pub const ERR_SEQ: u8 = 1 << 2;
    pub const ERR_RXE: u8 = 1 << 5;
    pub const ERR_MASK: u8 = Self::ERR_CE | Self::ERR_SE | Self::ERR_SEQ | Self::ERR_RXE;

    #[inline]
    pub fn is_done(&self) -> bool {
        self.status & Self::STA_DD != 0
    }

    #[inline]
    pub fn is_eop(&self) -> bool {
        self.status & Self::STA_EOP != 0
    }

    #[inline]
    pub fn has_errors(&self) -> bool {
        self.errors & Self::ERR_MASK != 0
    }
}

/// Link status snapshot. `speed`: 0=10, 1=100, 2=1000 Mbps.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LinkStatusResult {
    pub link_up: u8,
    pub full_duplex: u8,
    pub speed: u8,
}

impl LinkStatusResult {
    pub const SPEED_10: u8 = 0;
    pub const SPEED_100: u8 = 1;
    pub const SPEED_1000: u8 = 2;

    #[inline]
    pub fn is_link_up(&self) -> bool {
        self.link_up != 0
    }

    #[inline]
    pub fn is_full_duplex(&self) -> bool {
        self.full_duplex != 0
    }

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

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Returns 0 on success, 1 on 100ms timeout.
    pub fn asm_intel_reset(mmio_base: u64, tsc_freq: u64) -> u32;
    pub fn asm_intel_read_status(mmio_base: u64) -> u32;
    /// Returns 1 if MAC is all-zero/all-ones (invalid).
    pub fn asm_intel_read_mac(mmio_base: u64, mac_out: *mut [u8; 6]) -> u32;
    pub fn asm_intel_write_mac(mmio_base: u64, mac: *const [u8; 6]);
    pub fn asm_intel_clear_mta(mmio_base: u64);
    pub fn asm_intel_disable_interrupts(mmio_base: u64);
    pub fn asm_intel_setup_rx_ring(mmio_base: u64, ring_bus_addr: u64, ring_len_bytes: u32);
    pub fn asm_intel_setup_tx_ring(mmio_base: u64, ring_bus_addr: u64, ring_len_bytes: u32);
    pub fn asm_intel_enable_rx(mmio_base: u64);
    pub fn asm_intel_enable_tx(mmio_base: u64);
    pub fn asm_intel_set_link_up(mmio_base: u64);
    pub fn asm_intel_read_reg(mmio_base: u64, offset: u32) -> u32;
    pub fn asm_intel_write_reg(mmio_base: u64, offset: u32, value: u32);
}

#[cfg(target_arch = "x86_64")]
extern "win64" {
    pub fn asm_intel_tx_init_desc(desc_ptr: *mut u8);
    /// Sets EOP|IFCS|RS, ends with sfence.
    pub fn asm_intel_tx_submit(desc_ptr: *mut u8, buffer_bus_addr: u64, length: u32);
    /// Returns 1 if DD set.
    pub fn asm_intel_tx_poll(desc_ptr: *const u8) -> u32;
    /// TDT write; includes sfence.
    pub fn asm_intel_tx_update_tail(mmio_base: u64, tail: u32);
    pub fn asm_intel_tx_read_head(mmio_base: u64) -> u32;
    pub fn asm_intel_tx_clear_desc(desc_ptr: *mut u8);
}

#[cfg(target_arch = "x86_64")]
extern "win64" {
    pub fn asm_intel_rx_init_desc(desc_ptr: *mut u8, buffer_bus_addr: u64);
    /// Returns 1 with `result` populated when a packet is present.
    pub fn asm_intel_rx_poll(desc_ptr: *const u8, result: *mut RxPollResult) -> u32;
    /// RDT write; includes sfence.
    pub fn asm_intel_rx_update_tail(mmio_base: u64, tail: u32);
    pub fn asm_intel_rx_read_head(mmio_base: u64) -> u32;
    pub fn asm_intel_rx_clear_desc(desc_ptr: *mut u8);
    pub fn asm_intel_rx_get_length(desc_ptr: *const u8) -> u16;
    pub fn asm_intel_rx_check_errors(desc_ptr: *const u8) -> u32;
}

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// MDIC read; 0xFFFFFFFF on error/timeout.
    pub fn asm_intel_phy_read(mmio_base: u64, reg: u32, tsc_freq: u64) -> u32;
    /// MDIC write; 0 ok, 1 error/timeout.
    pub fn asm_intel_phy_write(mmio_base: u64, reg: u32, value: u32, tsc_freq: u64) -> u32;
    /// Fast STATUS-register snapshot. Returns 1 if link up.
    pub fn asm_intel_link_status(mmio_base: u64, result: *mut LinkStatusResult) -> u32;
    /// 0 when link comes up, 1 on timeout.
    pub fn asm_intel_wait_link(mmio_base: u64, timeout_us: u64, tsc_freq: u64) -> u32;
}

// I218/PCH LPT ULP. CRITICAL on real I218-LM/V (ThinkPad T450s): BIOS leaves
// the PHY in Ultra-Low-Power and MDIC silently fails until we wake it. See
// Linux e1000e/ich8lan.c for the dance.
#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// e1000_disable_ulp_lpt_lp (ich8lan.c). 0 ok, 1 timeout.
    pub fn asm_intel_disable_ulp(mmio_base: u64, tsc_freq: u64) -> u32;
    pub fn asm_intel_toggle_lanphypc(mmio_base: u64, tsc_freq: u64) -> u32;
    /// Reads PHY_ID1; nonzero result means alive. e1000_phy_is_accessible_pchlan.
    pub fn asm_intel_phy_is_accessible(mmio_base: u64, tsc_freq: u64) -> u32;
    /// EXTCNF_CTRL.SWFLAG. Required gate for PHY/NVM access on ICH8+/PCH.
    pub fn asm_intel_acquire_swflag(mmio_base: u64, tsc_freq: u64) -> u32;
    pub fn asm_intel_release_swflag(mmio_base: u64);
    /// I218 sometimes only answers via SMBus.
    pub fn asm_intel_force_smbus_mode(mmio_base: u64);
    pub fn asm_intel_clear_smbus_mode(mmio_base: u64);
}

#[inline]
pub fn reset(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_reset(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

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

#[inline]
pub fn write_mac(mmio_base: u64, mac: &[u8; 6]) {
    unsafe { asm_intel_write_mac(mmio_base, mac) };
}

#[inline]
pub fn get_link_status(mmio_base: u64) -> LinkStatusResult {
    let mut result = LinkStatusResult::default();
    unsafe { asm_intel_link_status(mmio_base, &mut result) };
    result
}

#[inline]
pub fn wait_for_link(mmio_base: u64, timeout_us: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_wait_link(mmio_base, timeout_us, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

#[inline]
pub fn phy_read(mmio_base: u64, reg: u32, tsc_freq: u64) -> Option<u16> {
    let result = unsafe { asm_intel_phy_read(mmio_base, reg, tsc_freq) };
    if result != 0xFFFFFFFF {
        Some(result as u16)
    } else {
        None
    }
}

#[inline]
pub fn phy_write(mmio_base: u64, reg: u32, value: u16, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_phy_write(mmio_base, reg, value as u32, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Wake the I218 PHY from ULP. T450s etc. won't talk MDIC without this.
#[inline]
pub fn disable_ulp(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_disable_ulp(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Last resort when the PHY is unresponsive.
#[inline]
pub fn toggle_lanphypc(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_toggle_lanphypc(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

#[inline]
pub fn phy_is_accessible(mmio_base: u64, tsc_freq: u64) -> bool {
    let result = unsafe { asm_intel_phy_is_accessible(mmio_base, tsc_freq) };
    result != 0
}

/// Acquire EXTCNF_CTRL.SWFLAG. Required before PHY/NVM access on ICH8+/PCH.
#[inline]
pub fn acquire_swflag(mmio_base: u64, tsc_freq: u64) -> Result<(), ()> {
    let result = unsafe { asm_intel_acquire_swflag(mmio_base, tsc_freq) };
    if result == 0 {
        Ok(())
    } else {
        Err(())
    }
}

#[inline]
pub fn release_swflag(mmio_base: u64) {
    unsafe { asm_intel_release_swflag(mmio_base) };
}

#[inline]
pub fn force_smbus_mode(mmio_base: u64) {
    unsafe { asm_intel_force_smbus_mode(mmio_base) };
}

#[inline]
pub fn clear_smbus_mode(mmio_base: u64) {
    unsafe { asm_intel_clear_smbus_mode(mmio_base) };
}
