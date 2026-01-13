//! Post-ExitBootServices initialization.
//!
//! This module handles the transition from UEFI boot services to bare-metal
//! operation. After ExitBootServices(), no UEFI services are available.
//!
//! # Initialization Sequence
//! 1. Validate BootHandoff structure
//! 2. Initialize DMA region layout
//! 3. Initialize NIC driver (VirtIO/Intel/Realtek)
//! 4. Initialize block device driver (if present)
//! 5. Return control to caller for main loop entry
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §7.6

use super::handoff::{BootHandoff, HandoffError, BLK_TYPE_VIRTIO, NIC_TYPE_INTEL, NIC_TYPE_VIRTIO};
use crate::dma::DmaRegion;
use crate::driver::virtio::VirtioConfig;
use crate::types::VirtqueueState;

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// Post-EBS initialization error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    /// Handoff validation failed
    HandoffInvalid(HandoffError),
    /// Unsupported NIC type
    UnsupportedNic(u8),
    /// NIC initialization failed
    NicInitFailed,
    /// Unsupported block device type
    UnsupportedBlockDevice(u8),
    /// Block device initialization failed
    BlockDeviceInitFailed,
    /// DMA region setup failed
    DmaSetupFailed,
}

impl From<HandoffError> for InitError {
    fn from(e: HandoffError) -> Self {
        InitError::HandoffInvalid(e)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZED NIC ENUM
// ═══════════════════════════════════════════════════════════════════════════

/// Represents an initialized network device type.
///
/// The actual driver state is stored separately for VirtIO (in InitResult fields).
/// For Intel, we store parameters needed for deferred initialization.
#[derive(Debug, Clone, Copy)]
pub enum InitializedNicType {
    /// VirtIO network device (state in InitResult.nic_config, rx_queue, tx_queue)
    VirtIO,
    /// Intel e1000e network device
    Intel {
        mmio_base: u64,
        tsc_freq: u64,
    },
    /// No NIC
    None,
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION RESULT
// ═══════════════════════════════════════════════════════════════════════════

/// Result of post-EBS initialization.
///
/// Contains all the initialized components ready for the main loop.
pub struct InitResult {
    /// Validated handoff reference
    pub handoff: &'static BootHandoff,
    /// DMA region layout
    pub dma: DmaRegion,
    /// Initialized NIC type
    pub nic_type: InitializedNicType,
    /// VirtIO NIC configuration (if NIC is VirtIO)
    pub nic_config: Option<VirtioConfig>,
    /// RX queue state (initialized) - VirtIO only
    pub rx_queue: Option<VirtqueueState>,
    /// TX queue state (initialized) - VirtIO only
    pub tx_queue: Option<VirtqueueState>,
    /// MAC address
    pub mac_address: [u8; 6],
    /// Block device config (if present and VirtIO)
    pub blk_config: Option<VirtioConfig>,
}

// ═══════════════════════════════════════════════════════════════════════════
// TIMEOUT CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// Timeout configuration derived from TSC frequency.
#[derive(Debug, Clone, Copy)]
pub struct TimeoutConfig {
    /// TSC ticks per millisecond
    ticks_per_ms: u64,
}

impl TimeoutConfig {
    /// Create from TSC frequency.
    pub fn new(tsc_freq: u64) -> Self {
        Self {
            ticks_per_ms: tsc_freq / 1_000,
        }
    }

    /// DHCP timeout (30 seconds)
    #[inline]
    pub fn dhcp(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// DNS query timeout (5 seconds)
    #[inline]
    pub fn dns(&self) -> u64 {
        5_000 * self.ticks_per_ms
    }

    /// TCP connect timeout (30 seconds)
    #[inline]
    pub fn tcp_connect(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// TCP close timeout (10 seconds)
    #[inline]
    pub fn tcp_close(&self) -> u64 {
        10_000 * self.ticks_per_ms
    }

    /// HTTP send timeout (30 seconds)
    #[inline]
    pub fn http_send(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// HTTP receive timeout (60 seconds for slow connections)
    #[inline]
    pub fn http_receive(&self) -> u64 {
        60_000 * self.ticks_per_ms
    }

    /// HTTP idle timeout between chunks (30 seconds)
    #[inline]
    pub fn http_idle(&self) -> u64 {
        30_000 * self.ticks_per_ms
    }

    /// Main loop iteration warning threshold (5ms)
    #[inline]
    pub fn loop_warning(&self) -> u64 {
        5 * self.ticks_per_ms
    }

    /// Device reset timeout (100ms)
    #[inline]
    pub fn device_reset(&self) -> u64 {
        100 * self.ticks_per_ms
    }

    /// Convert milliseconds to ticks
    #[inline]
    pub fn ms_to_ticks(&self, ms: u64) -> u64 {
        ms * self.ticks_per_ms
    }

    /// Convert ticks to milliseconds
    #[inline]
    pub fn ticks_to_ms(&self, ticks: u64) -> u64 {
        ticks / self.ticks_per_ms
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// POST-EBS INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Perform post-ExitBootServices initialization.
///
/// # Arguments
/// - `handoff`: Pointer to BootHandoff structure (must be valid)
///
/// # Returns
/// - `Ok(InitResult)` with initialized components
/// - `Err(InitError)` if initialization fails
///
/// # Safety
/// - `handoff` must point to a valid, populated BootHandoff structure
/// - Must be called after ExitBootServices()
/// - Must be called on the pre-allocated stack
/// - Must not be called more than once
#[cfg(target_arch = "x86_64")]
pub unsafe fn post_ebs_init(handoff: &'static BootHandoff) -> Result<InitResult, InitError> {
    // ═══════════════════════════════════════════════════════════════════════
    // STEP 1: VALIDATE HANDOFF
    // ═══════════════════════════════════════════════════════════════════════
    handoff.validate()?;

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 2: INITIALIZE DMA REGION LAYOUT
    // ═══════════════════════════════════════════════════════════════════════
    let (dma_cpu, dma_bus, dma_size) = handoff.dma_region();
    let dma = DmaRegion::new(dma_cpu, dma_bus, dma_size as usize);

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 3: INITIALIZE NIC DRIVER
    // ═══════════════════════════════════════════════════════════════════════
    let (nic_type, nic_config, rx_queue, tx_queue, mac_address) = match handoff.nic_type {
        NIC_TYPE_VIRTIO => {
            let (config, rx, tx, mac) = init_virtio_nic(handoff, &dma)?;
            (
                InitializedNicType::VirtIO,
                Some(config),
                Some(rx),
                Some(tx),
                mac,
            )
        }
        NIC_TYPE_INTEL => {
            // Intel NIC - just record parameters for deferred initialization
            // The actual driver creation happens when creating the HTTP client
            let mac = handoff.mac_address;
            (
                InitializedNicType::Intel {
                    mmio_base: handoff.nic_mmio_base,
                    tsc_freq: handoff.tsc_freq,
                },
                None,
                None,
                None,
                mac,
            )
        }
        other => {
            return Err(InitError::UnsupportedNic(other));
        }
    };

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 4: INITIALIZE BLOCK DEVICE (if present)
    // ═══════════════════════════════════════════════════════════════════════
    let blk_config = if handoff.has_block_device() {
        match handoff.blk_type {
            BLK_TYPE_VIRTIO => {
                // Will be implemented in Chunk 7
                None
            }
            _ => None,
        }
    } else {
        None
    };

    Ok(InitResult {
        handoff,
        dma,
        nic_type,
        nic_config,
        rx_queue,
        tx_queue,
        mac_address,
        blk_config,
    })
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn post_ebs_init(_handoff: &'static BootHandoff) -> Result<InitResult, InitError> {
    Err(InitError::UnsupportedNic(0))
}

/// Initialize VirtIO NIC.
#[cfg(target_arch = "x86_64")]
unsafe fn init_virtio_nic(
    handoff: &BootHandoff,
    dma: &DmaRegion,
) -> Result<(VirtioConfig, VirtqueueState, VirtqueueState, [u8; 6]), InitError> {
    use crate::driver::virtio::init::virtio_net_init;

    // Create VirtIO config from handoff and DMA region
    let config = VirtioConfig {
        dma_cpu_base: dma.cpu_base(),
        dma_bus_base: dma.bus_base(),
        dma_size: dma.size(),
        queue_size: 32, // Standard queue size
        buffer_size: 2048,
    };

    // Initialize device
    let (features, rx_queue, tx_queue, mac) =
        virtio_net_init(handoff.nic_mmio_base, &config).map_err(|_| InitError::NicInitFailed)?;

    // Update config with negotiated features (for reference)
    let config = VirtioConfig {
        dma_cpu_base: dma.cpu_base(),
        dma_bus_base: dma.bus_base(),
        dma_size: dma.size(),
        queue_size: 32,
        buffer_size: 2048,
    };
    let _ = features; // Used for feature tracking if needed

    Ok((config, rx_queue, tx_queue, mac))
}

// ═══════════════════════════════════════════════════════════════════════════
// ENTRY POINT (called from ASM trampoline)
// ═══════════════════════════════════════════════════════════════════════════

/// Post-EBS entry point for ASM trampoline.
///
/// This is the Rust entry point called after:
/// 1. ExitBootServices() has been called
/// 2. Stack has been switched to pre-allocated stack
/// 3. Interrupts are disabled
///
/// # Arguments
/// - `handoff_ptr`: Pointer to BootHandoff structure
///
/// # Returns
/// Never returns (enters main loop or panics)
///
/// # Safety
/// Must only be called from the ASM trampoline with correct setup.
#[no_mangle]
#[cfg(target_arch = "x86_64")]
pub unsafe extern "C" fn _post_ebs_entry(handoff_ptr: *const BootHandoff) -> ! {
    // Convert to static reference (valid for program lifetime)
    let handoff = &*handoff_ptr;

    // Initialize
    match post_ebs_init(handoff) {
        Ok(_result) => {
            // Main loop entry point will be implemented in Chunk 9
            // For now, halt
            loop {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
        Err(_e) => {
            // Fatal error - halt
            loop {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe extern "C" fn _post_ebs_entry(_handoff_ptr: *const BootHandoff) -> ! {
    loop {}
}
