//! BootHandoff structure.
//!
//! Data passed from UEFI boot phase to bare-metal phase.
//! Populated before ExitBootServices, consumed after.
//!
//! # Layout
//! All fields are explicitly sized and aligned for ABI stability.
//! The structure is `#[repr(C)]` to ensure predictable memory layout
//! across Rust/ASM boundary.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §7.2

use core::fmt;

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Magic number: "MORPHEUS" in ASCII (little-endian)
pub const HANDOFF_MAGIC: u64 = 0x5355_4548_5052_4F4D;

/// Current structure version
pub const HANDOFF_VERSION: u32 = 1;

/// Minimum DMA region size (2MB)
pub const MIN_DMA_SIZE: u64 = 2 * 1024 * 1024;

/// Minimum stack size (64KB)
pub const MIN_STACK_SIZE: u64 = 64 * 1024;

/// Minimum TSC frequency (1 GHz - sanity check)
pub const MIN_TSC_FREQ: u64 = 1_000_000_000;

/// Maximum TSC frequency (10 GHz - sanity check)
pub const MAX_TSC_FREQ: u64 = 10_000_000_000;

// ═══════════════════════════════════════════════════════════════════════════
// NIC TYPE CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// No NIC detected
pub const NIC_TYPE_NONE: u8 = 0;
/// VirtIO-net device
pub const NIC_TYPE_VIRTIO: u8 = 1;
/// Intel e1000/i210/i225
pub const NIC_TYPE_INTEL: u8 = 2;
/// Realtek RTL8168/8111
pub const NIC_TYPE_REALTEK: u8 = 3;
/// Broadcom BCM57xx
pub const NIC_TYPE_BROADCOM: u8 = 4;

// ═══════════════════════════════════════════════════════════════════════════
// BLOCK DEVICE TYPE CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// No block device
pub const BLK_TYPE_NONE: u8 = 0;
/// VirtIO-blk device
pub const BLK_TYPE_VIRTIO: u8 = 1;
/// NVMe device (future)
pub const BLK_TYPE_NVME: u8 = 2;
/// AHCI/SATA device (future)
pub const BLK_TYPE_AHCI: u8 = 3;

// ═══════════════════════════════════════════════════════════════════════════
// TRANSPORT TYPE CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// VirtIO MMIO transport
pub const TRANSPORT_MMIO: u8 = 0;
/// VirtIO PCI Modern transport (capability-based)
pub const TRANSPORT_PCI_MODERN: u8 = 1;
/// VirtIO PCI Legacy transport (I/O ports)
pub const TRANSPORT_PCI_LEGACY: u8 = 2;

// ═══════════════════════════════════════════════════════════════════════════
// HANDOFF ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// Errors during handoff validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    /// Magic number mismatch
    InvalidMagic,
    /// Unsupported version
    UnsupportedVersion,
    /// Structure size mismatch
    SizeMismatch,
    /// TSC frequency not calibrated or out of range
    InvalidTscFreq,
    /// DMA region too small
    DmaRegionTooSmall,
    /// DMA CPU pointer is null
    DmaCpuPtrNull,
    /// DMA bus address is zero
    DmaBusAddrZero,
    /// Stack too small
    StackTooSmall,
    /// Stack top is null
    StackTopNull,
    /// No NIC configured
    NoNic,
    /// NIC MMIO base is zero
    NicMmioZero,
}

impl fmt::Display for HandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "invalid magic number"),
            Self::UnsupportedVersion => write!(f, "unsupported handoff version"),
            Self::SizeMismatch => write!(f, "structure size mismatch"),
            Self::InvalidTscFreq => write!(f, "TSC frequency not calibrated or out of range"),
            Self::DmaRegionTooSmall => write!(f, "DMA region too small (need >= 2MB)"),
            Self::DmaCpuPtrNull => write!(f, "DMA CPU pointer is null"),
            Self::DmaBusAddrZero => write!(f, "DMA bus address is zero"),
            Self::StackTooSmall => write!(f, "stack too small (need >= 64KB)"),
            Self::StackTopNull => write!(f, "stack top is null"),
            Self::NoNic => write!(f, "no NIC configured"),
            Self::NicMmioZero => write!(f, "NIC MMIO base is zero"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BOOT HANDOFF STRUCTURE
// ═══════════════════════════════════════════════════════════════════════════

/// Data passed from UEFI boot phase to bare-metal phase.
///
/// This structure is populated before ExitBootServices and consumed after.
/// All pointers must remain valid post-EBS.
///
/// # Safety
/// - All pointers must point to memory that survives ExitBootServices
/// - DMA region must be allocated via PCI I/O Protocol for IOMMU compatibility
/// - Structure must be placed in EfiLoaderData or EfiBootServicesData memory
#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct BootHandoff {
    // ═══════════════════════════════════════════════════════════════════════
    // HEADER (16 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Magic number for validation: "MORPHEUS" = 0x5355_4548_5052_4F4D
    pub magic: u64,
    
    /// Structure version (currently 1)
    pub version: u32,
    
    /// Structure size in bytes (for forward compatibility)
    pub size: u32,
    
    // ═══════════════════════════════════════════════════════════════════════
    // NIC INFORMATION (24 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// VirtIO/NIC MMIO base address (from PCI BAR)
    pub nic_mmio_base: u64,
    
    /// PCI bus number
    pub nic_pci_bus: u8,
    
    /// PCI device number
    pub nic_pci_device: u8,
    
    /// PCI function number
    pub nic_pci_function: u8,
    
    /// NIC type: 0=None, 1=VirtIO, 2=Intel, 3=Realtek, 4=Broadcom
    pub nic_type: u8,
    
    /// MAC address (6 bytes, may be zeros if not yet read)
    pub mac_address: [u8; 6],
    
    /// Padding for alignment
    pub _nic_pad: [u8; 2],
    
    // ═══════════════════════════════════════════════════════════════════════
    // BLOCK DEVICE INFORMATION (24 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Block device MMIO base address (legacy) or common_cfg (PCI Modern)
    pub blk_mmio_base: u64,
    
    /// Block device PCI bus number
    pub blk_pci_bus: u8,
    
    /// Block device PCI device number
    pub blk_pci_device: u8,
    
    /// Block device PCI function number
    pub blk_pci_function: u8,
    
    /// Block device type: 0=None, 1=VirtIO-blk, 2=NVMe, 3=AHCI
    pub blk_type: u8,
    
    /// Block device sector size (typically 512)
    pub blk_sector_size: u32,
    
    /// Block device total sectors
    pub blk_total_sectors: u64,
    
    // ═══════════════════════════════════════════════════════════════════════
    // DMA REGION (24 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// CPU pointer for software access
    pub dma_cpu_ptr: u64,
    
    /// Bus address for device DMA (may differ from CPU addr with IOMMU)
    pub dma_bus_addr: u64,
    
    /// Region size in bytes (minimum 2MB)
    pub dma_size: u64,
    
    // ═══════════════════════════════════════════════════════════════════════
    // TIMING (8 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Calibrated TSC frequency (ticks per second)
    /// MUST be calibrated at boot using UEFI Stall(). NO HARDCODED VALUES.
    pub tsc_freq: u64,
    
    // ═══════════════════════════════════════════════════════════════════════
    // STACK (16 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Top of stack (highest address, stack grows down)
    pub stack_top: u64,
    
    /// Stack size in bytes (minimum 64KB)
    pub stack_size: u64,
    
    // ═══════════════════════════════════════════════════════════════════════
    // FRAMEBUFFER / DEBUG (24 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Framebuffer base address for debug output (0 if unavailable)
    pub framebuffer_base: u64,
    
    /// Framebuffer width in pixels
    pub framebuffer_width: u32,
    
    /// Framebuffer height in pixels
    pub framebuffer_height: u32,
    
    /// Framebuffer stride (bytes per row)
    pub framebuffer_stride: u32,
    
    /// Framebuffer pixel format: 0=BGR, 1=RGB
    pub framebuffer_format: u32,
    
    // ═══════════════════════════════════════════════════════════════════════
    // MEMORY MAP INFO (16 bytes)
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Pointer to UEFI memory map (copied before EBS)
    pub memory_map_ptr: u64,
    
    /// Memory map size in bytes
    pub memory_map_size: u32,
    
    /// Memory map descriptor size
    pub memory_map_desc_size: u32,
    
    // ═══════════════════════════════════════════════════════════════════════
    // PCI MODERN TRANSPORT INFO (48 bytes) - for VirtIO PCI Modern NIC
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Transport type: 0=MMIO, 1=PCI Modern, 2=PCI Legacy
    pub nic_transport_type: u8,
    
    /// Padding
    pub _transport_pad: [u8; 3],
    
    /// Notify offset multiplier (from VIRTIO_PCI_CAP_NOTIFY)
    pub nic_notify_off_multiplier: u32,
    
    /// Common cfg address (BAR base + cap offset)
    pub nic_common_cfg: u64,
    
    /// Notify cfg address (BAR base + cap offset)
    pub nic_notify_cfg: u64,
    
    /// ISR cfg address (BAR base + cap offset)
    pub nic_isr_cfg: u64,
    
    /// Device cfg address (BAR base + cap offset)
    pub nic_device_cfg: u64,
    
    // ═══════════════════════════════════════════════════════════════════════
    // PCI MODERN TRANSPORT INFO (48 bytes) - for VirtIO PCI Modern BLK
    // ═══════════════════════════════════════════════════════════════════════
    
    /// Transport type: 0=MMIO, 1=PCI Modern, 2=PCI Legacy
    pub blk_transport_type: u8,
    
    /// Padding
    pub _blk_transport_pad: [u8; 3],
    
    /// Notify offset multiplier (from VIRTIO_PCI_CAP_NOTIFY)
    pub blk_notify_off_multiplier: u32,
    
    /// Common cfg address (BAR base + cap offset) - same as blk_mmio_base for PCI Modern
    pub blk_common_cfg: u64,
    
    /// Notify cfg address (BAR base + cap offset)
    pub blk_notify_cfg: u64,
    
    /// ISR cfg address (BAR base + cap offset)
    pub blk_isr_cfg: u64,
    
    /// Device cfg address (BAR base + cap offset)
    pub blk_device_cfg: u64,
    
    // ═══════════════════════════════════════════════════════════════════════
    // RESERVED (8 bytes for future expansion)
    // ═══════════════════════════════════════════════════════════════════════
    
    pub _reserved: [u8; 8],
}

// Compile-time size check (200 original + 40 blk PCI Modern = 240 fields, aligned to 64 = 256)
const _: () = assert!(core::mem::size_of::<BootHandoff>() == 256);

impl BootHandoff {
    /// Magic number constant
    pub const MAGIC: u64 = HANDOFF_MAGIC;
    
    /// Current version constant
    pub const VERSION: u32 = HANDOFF_VERSION;
    
    /// Expected structure size
    pub const SIZE: u32 = 256;
    
    /// Create a zeroed handoff structure with magic and version set.
    pub const fn new() -> Self {
        Self {
            magic: HANDOFF_MAGIC,
            version: HANDOFF_VERSION,
            size: Self::SIZE,
            nic_mmio_base: 0,
            nic_pci_bus: 0,
            nic_pci_device: 0,
            nic_pci_function: 0,
            nic_type: NIC_TYPE_NONE,
            mac_address: [0; 6],
            _nic_pad: [0; 2],
            blk_mmio_base: 0,
            blk_pci_bus: 0,
            blk_pci_device: 0,
            blk_pci_function: 0,
            blk_type: BLK_TYPE_NONE,
            blk_sector_size: 0,
            blk_total_sectors: 0,
            dma_cpu_ptr: 0,
            dma_bus_addr: 0,
            dma_size: 0,
            tsc_freq: 0,
            stack_top: 0,
            stack_size: 0,
            framebuffer_base: 0,
            framebuffer_width: 0,
            framebuffer_height: 0,
            framebuffer_stride: 0,
            framebuffer_format: 0,
            memory_map_ptr: 0,
            memory_map_size: 0,
            memory_map_desc_size: 0,
            // PCI Modern transport fields (NIC)
            nic_transport_type: 0,  // 0 = MMIO
            _transport_pad: [0; 3],
            nic_notify_off_multiplier: 0,
            nic_common_cfg: 0,
            nic_notify_cfg: 0,
            nic_isr_cfg: 0,
            nic_device_cfg: 0,
            // PCI Modern transport fields (BLK)
            blk_transport_type: 0,  // 0 = MMIO
            _blk_transport_pad: [0; 3],
            blk_notify_off_multiplier: 0,
            blk_common_cfg: 0,
            blk_notify_cfg: 0,
            blk_isr_cfg: 0,
            blk_device_cfg: 0,
            _reserved: [0; 8],
        }
    }
    
    /// Validate the handoff structure.
    ///
    /// # Returns
    /// - `Ok(())` if valid
    /// - `Err(HandoffError)` describing the first validation failure
    pub fn validate(&self) -> Result<(), HandoffError> {
        // Header validation
        if self.magic != HANDOFF_MAGIC {
            return Err(HandoffError::InvalidMagic);
        }
        if self.version != HANDOFF_VERSION {
            return Err(HandoffError::UnsupportedVersion);
        }
        if self.size != Self::SIZE {
            return Err(HandoffError::SizeMismatch);
        }
        
        // TSC validation (required)
        if self.tsc_freq < MIN_TSC_FREQ || self.tsc_freq > MAX_TSC_FREQ {
            return Err(HandoffError::InvalidTscFreq);
        }
        
        // DMA validation (required)
        if self.dma_size < MIN_DMA_SIZE {
            return Err(HandoffError::DmaRegionTooSmall);
        }
        if self.dma_cpu_ptr == 0 {
            return Err(HandoffError::DmaCpuPtrNull);
        }
        if self.dma_bus_addr == 0 {
            return Err(HandoffError::DmaBusAddrZero);
        }
        
        // Stack validation (required)
        if self.stack_size < MIN_STACK_SIZE {
            return Err(HandoffError::StackTooSmall);
        }
        if self.stack_top == 0 {
            return Err(HandoffError::StackTopNull);
        }
        
        // NIC validation (required for network boot)
        if self.nic_type == NIC_TYPE_NONE {
            return Err(HandoffError::NoNic);
        }
        if self.nic_mmio_base == 0 {
            return Err(HandoffError::NicMmioZero);
        }
        
        Ok(())
    }
    
    /// Validate for network-only operation (block device optional).
    pub fn validate_network_only(&self) -> Result<(), HandoffError> {
        // Same as validate() - NIC is required
        self.validate()
    }
    
    /// Check if block device is configured.
    pub fn has_block_device(&self) -> bool {
        // For PCI Modern, blk_mmio_base is 0 but blk_common_cfg is set
        // For Legacy MMIO, blk_mmio_base is set
        self.blk_type != BLK_TYPE_NONE && 
            (self.blk_mmio_base != 0 || self.blk_common_cfg != 0)
    }
    
    /// Check if framebuffer is available.
    pub fn has_framebuffer(&self) -> bool {
        self.framebuffer_base != 0 && self.framebuffer_width > 0 && self.framebuffer_height > 0
    }
    
    /// Get DMA region as raw pointer and size.
    ///
    /// # Safety
    /// The returned pointer is only valid if the handoff has been validated.
    pub unsafe fn dma_region(&self) -> (*mut u8, u64, u64) {
        (self.dma_cpu_ptr as *mut u8, self.dma_bus_addr, self.dma_size)
    }
    
    /// Convert milliseconds to TSC ticks.
    #[inline]
    pub fn ms_to_ticks(&self, ms: u64) -> u64 {
        ms * self.tsc_freq / 1_000
    }
    
    /// Convert TSC ticks to milliseconds.
    #[inline]
    pub fn ticks_to_ms(&self, ticks: u64) -> u64 {
        ticks * 1_000 / self.tsc_freq
    }
    
    /// Convert microseconds to TSC ticks.
    #[inline]
    pub fn us_to_ticks(&self, us: u64) -> u64 {
        us * self.tsc_freq / 1_000_000
    }
    
    /// Convert TSC ticks to microseconds.
    #[inline]
    pub fn ticks_to_us(&self, ticks: u64) -> u64 {
        ticks * 1_000_000 / self.tsc_freq
    }
}

impl Default for BootHandoff {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for BootHandoff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BootHandoff")
            .field("magic", &format_args!("{:#x}", self.magic))
            .field("version", &self.version)
            .field("tsc_freq", &format_args!("{} Hz", self.tsc_freq))
            .field("nic_type", &self.nic_type)
            .field("nic_mmio", &format_args!("{:#x}", self.nic_mmio_base))
            .field("blk_type", &self.blk_type)
            .field("blk_mmio", &format_args!("{:#x}", self.blk_mmio_base))
            .field("dma_cpu", &format_args!("{:#x}", self.dma_cpu_ptr))
            .field("dma_bus", &format_args!("{:#x}", self.dma_bus_addr))
            .field("dma_size", &format_args!("{:#x}", self.dma_size))
            .field("stack_top", &format_args!("{:#x}", self.stack_top))
            .field("stack_size", &format_args!("{:#x}", self.stack_size))
            .finish()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TSC CALIBRATION HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// TSC calibration result.
#[derive(Debug, Clone, Copy)]
pub struct TscCalibration {
    /// Ticks per second
    pub frequency: u64,
    /// Whether invariant TSC is available
    pub invariant: bool,
}

/// Check if CPU has invariant TSC via CPUID.
///
/// Invariant TSC means the TSC runs at constant rate regardless of
/// CPU frequency changes (power management, turbo boost, etc.)
///
/// # Safety
/// Uses CPUID instruction which is always available on x86_64.
#[cfg(target_arch = "x86_64")]
pub fn has_invariant_tsc() -> bool {
    // CPUID leaf 0x80000007, EDX bit 8
    let result: u32;
    unsafe {
        core::arch::asm!(
            // Save rbx since LLVM uses it internally
            "push rbx",
            "mov eax, 0x80000007",
            "cpuid",
            "mov {0:e}, edx",
            "pop rbx",
            out(reg) result,
            out("eax") _,
            out("ecx") _,
            out("edx") _,
            options(nostack)
        );
    }
    (result & (1 << 8)) != 0
}

#[cfg(not(target_arch = "x86_64"))]
pub fn has_invariant_tsc() -> bool {
    false
}

/// Read TSC value directly (used for calibration).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn read_tsc_raw() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack)
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn read_tsc_raw() -> u64 {
    0
}
