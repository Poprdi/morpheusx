//! Network initialization configuration.
//!
//! Configuration options for the network initialization sequence.

/// ECAM base address for QEMU Q35 machine type.
pub const ECAM_BASE_QEMU_Q35: usize = 0xB000_0000;

/// ECAM base address for QEMU i440FX machine type.
pub const ECAM_BASE_QEMU_I440FX: usize = 0xE000_0000;

/// Network initialization configuration.
#[derive(Debug, Clone)]
pub struct InitConfig {
    /// Timeout for DHCP in milliseconds.
    pub dhcp_timeout_ms: u64,
    /// Use static DMA pool (fallback if discovery fails).
    pub use_static_dma: bool,
    /// Image base address for DMA cave discovery.
    pub image_base: Option<usize>,
    /// Image end address for DMA cave discovery.
    pub image_end: Option<usize>,
    /// ECAM base address for PCIe config access.
    /// If None, uses legacy I/O ports on x86.
    pub ecam_base: Option<usize>,
    /// Retry count for transient failures.
    pub retry_count: u8,
    /// Delay between retries in milliseconds.
    pub retry_delay_ms: u64,
}

impl Default for InitConfig {
    fn default() -> Self {
        Self {
            dhcp_timeout_ms: 30_000, // 30 seconds
            use_static_dma: true,     // Use static as fallback
            image_base: None,
            image_end: None,
            ecam_base: Some(ECAM_BASE_QEMU_Q35), // Default to Q35
            retry_count: 3,
            retry_delay_ms: 1_000,
        }
    }
}

impl InitConfig {
    /// Create config with PE image addresses for DMA cave discovery.
    pub fn with_image_bounds(image_base: usize, image_end: usize) -> Self {
        Self {
            image_base: Some(image_base),
            image_end: Some(image_end),
            ..Default::default()
        }
    }

    /// Create config for QEMU/VirtIO testing.
    pub fn for_qemu() -> Self {
        Self {
            dhcp_timeout_ms: 10_000, // Faster for testing
            use_static_dma: true,
            image_base: None,
            image_end: None,
            ecam_base: Some(ECAM_BASE_QEMU_Q35),
            retry_count: 2,
            retry_delay_ms: 500,
        }
    }

    /// Create config using legacy I/O port PCI access (x86 only).
    #[cfg(target_arch = "x86_64")]
    pub fn legacy_io() -> Self {
        Self {
            ecam_base: None,
            ..Default::default()
        }
    }

    /// Set ECAM base address.
    pub fn ecam(mut self, base: usize) -> Self {
        self.ecam_base = Some(base);
        self
    }

    /// Set DHCP timeout.
    pub fn dhcp_timeout(mut self, timeout_ms: u64) -> Self {
        self.dhcp_timeout_ms = timeout_ms;
        self
    }

    /// Set retry configuration.
    pub fn retries(mut self, count: u8, delay_ms: u64) -> Self {
        self.retry_count = count;
        self.retry_delay_ms = delay_ms;
        self
    }
}
