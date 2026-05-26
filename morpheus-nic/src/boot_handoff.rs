//! BootHandoff: UEFI → bare-metal ABI. `#[repr(C)]`, populated pre-EBS.

use core::fmt;

/// "MORPHEUS" little-endian.
pub const HANDOFF_MAGIC: u64 = 0x5355_4548_5052_4F4D;

pub const HANDOFF_VERSION: u32 = 1;

pub const MIN_DMA_SIZE: u64 = 2 * 1024 * 1024;

pub const MIN_STACK_SIZE: u64 = 64 * 1024;

pub const MIN_TSC_FREQ: u64 = 1_000_000_000;

pub const MAX_TSC_FREQ: u64 = 10_000_000_000;

pub const NIC_TYPE_NONE: u8 = 0;
pub const NIC_TYPE_VIRTIO: u8 = 1;
/// Intel e1000/i210/i225.
pub const NIC_TYPE_INTEL: u8 = 2;
pub const NIC_TYPE_REALTEK: u8 = 3;
pub const NIC_TYPE_BROADCOM: u8 = 4;

pub const BLK_TYPE_NONE: u8 = 0;
pub const BLK_TYPE_VIRTIO: u8 = 1;
pub const BLK_TYPE_NVME: u8 = 2;
pub const BLK_TYPE_AHCI: u8 = 3;

pub const TRANSPORT_MMIO: u8 = 0;
pub const TRANSPORT_PCI_MODERN: u8 = 1;
pub const TRANSPORT_PCI_LEGACY: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    InvalidMagic,
    UnsupportedVersion,
    SizeMismatch,
    InvalidTscFreq,
    DmaRegionTooSmall,
    DmaCpuPtrNull,
    DmaBusAddrZero,
    StackTooSmall,
    StackTopNull,
    NoNic,
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

/// UEFI → bare-metal ABI. All pointers must survive ExitBootServices; DMA
/// region must come from PCI I/O Protocol for IOMMU correctness.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct BootHandoff {
    pub magic: u64,
    pub version: u32,
    pub size: u32,

    pub nic_mmio_base: u64,
    pub nic_pci_bus: u8,
    pub nic_pci_device: u8,
    pub nic_pci_function: u8,
    /// 0=None 1=VirtIO 2=Intel 3=Realtek 4=Broadcom.
    pub nic_type: u8,
    pub mac_address: [u8; 6],
    pub _nic_pad: [u8; 2],

    /// Legacy MMIO base; PCI Modern uses `blk_common_cfg` instead.
    pub blk_mmio_base: u64,
    pub blk_pci_bus: u8,
    pub blk_pci_device: u8,
    pub blk_pci_function: u8,
    /// 0=None 1=VirtIO-blk 2=NVMe 3=AHCI.
    pub blk_type: u8,
    pub blk_sector_size: u32,
    pub blk_total_sectors: u64,

    pub dma_cpu_ptr: u64,
    /// May differ from CPU addr with IOMMU.
    pub dma_bus_addr: u64,
    pub dma_size: u64,

    /// MUST be calibrated at boot via UEFI Stall(). Never hardcode.
    pub tsc_freq: u64,

    /// Highest address; stack grows down.
    pub stack_top: u64,
    pub stack_size: u64,

    pub framebuffer_base: u64,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_stride: u32,
    /// 0=Rgbx 1=Bgrx.
    pub framebuffer_format: u32,

    /// UEFI memory map snapshot copied before EBS.
    pub memory_map_ptr: u64,
    pub memory_map_size: u32,
    pub memory_map_desc_size: u32,

    /// 0=MMIO 1=PCI Modern 2=PCI Legacy.
    pub nic_transport_type: u8,
    pub _transport_pad: [u8; 3],
    /// From VIRTIO_PCI_CAP_NOTIFY.
    pub nic_notify_off_multiplier: u32,
    pub nic_common_cfg: u64,
    pub nic_notify_cfg: u64,
    pub nic_isr_cfg: u64,
    pub nic_device_cfg: u64,

    /// 0=MMIO 1=PCI Modern 2=PCI Legacy.
    pub blk_transport_type: u8,
    pub _blk_transport_pad: [u8; 3],
    pub blk_notify_off_multiplier: u32,
    pub blk_common_cfg: u64,
    pub blk_notify_cfg: u64,
    pub blk_isr_cfg: u64,
    pub blk_device_cfg: u64,

    pub _reserved: [u8; 8],
}

const _: () = assert!(core::mem::size_of::<BootHandoff>() == 256);

impl BootHandoff {
    pub const MAGIC: u64 = HANDOFF_MAGIC;

    pub const VERSION: u32 = HANDOFF_VERSION;

    pub const SIZE: u32 = 256;

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
            nic_transport_type: 0,
            _transport_pad: [0; 3],
            nic_notify_off_multiplier: 0,
            nic_common_cfg: 0,
            nic_notify_cfg: 0,
            nic_isr_cfg: 0,
            nic_device_cfg: 0,
            blk_transport_type: 0,
            _blk_transport_pad: [0; 3],
            blk_notify_off_multiplier: 0,
            blk_common_cfg: 0,
            blk_notify_cfg: 0,
            blk_isr_cfg: 0,
            blk_device_cfg: 0,
            _reserved: [0; 8],
        }
    }

    pub fn validate(&self) -> Result<(), HandoffError> {
        if self.magic != HANDOFF_MAGIC {
            return Err(HandoffError::InvalidMagic);
        }
        if self.version != HANDOFF_VERSION {
            return Err(HandoffError::UnsupportedVersion);
        }
        if self.size != Self::SIZE {
            return Err(HandoffError::SizeMismatch);
        }
        if self.tsc_freq < MIN_TSC_FREQ || self.tsc_freq > MAX_TSC_FREQ {
            return Err(HandoffError::InvalidTscFreq);
        }
        if self.dma_size < MIN_DMA_SIZE {
            return Err(HandoffError::DmaRegionTooSmall);
        }
        if self.dma_cpu_ptr == 0 {
            return Err(HandoffError::DmaCpuPtrNull);
        }
        if self.dma_bus_addr == 0 {
            return Err(HandoffError::DmaBusAddrZero);
        }
        if self.stack_size < MIN_STACK_SIZE {
            return Err(HandoffError::StackTooSmall);
        }
        if self.stack_top == 0 {
            return Err(HandoffError::StackTopNull);
        }
        if self.nic_type == NIC_TYPE_NONE {
            return Err(HandoffError::NoNic);
        }
        if self.nic_mmio_base == 0 {
            return Err(HandoffError::NicMmioZero);
        }

        Ok(())
    }

    /// Network-only path: NIC required, block device optional.
    pub fn validate_network_only(&self) -> Result<(), HandoffError> {
        self.validate()
    }

    pub fn has_block_device(&self) -> bool {
        // PCI Modern: blk_mmio_base=0, blk_common_cfg set. Legacy: blk_mmio_base set.
        self.blk_type != BLK_TYPE_NONE && (self.blk_mmio_base != 0 || self.blk_common_cfg != 0)
    }

    pub fn has_framebuffer(&self) -> bool {
        self.framebuffer_base != 0 && self.framebuffer_width > 0 && self.framebuffer_height > 0
    }

    /// # Safety
    /// Caller must have validated the handoff.
    pub unsafe fn dma_region(&self) -> (*mut u8, u64, u64) {
        (
            self.dma_cpu_ptr as *mut u8,
            self.dma_bus_addr,
            self.dma_size,
        )
    }

    #[inline]
    pub fn ms_to_ticks(&self, ms: u64) -> u64 {
        ms * self.tsc_freq / 1_000
    }

    #[inline]
    pub fn ticks_to_ms(&self, ticks: u64) -> u64 {
        ticks * 1_000 / self.tsc_freq
    }

    #[inline]
    pub fn us_to_ticks(&self, us: u64) -> u64 {
        us * self.tsc_freq / 1_000_000
    }

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

#[derive(Debug, Clone, Copy)]
pub struct TscCalibration {
    pub frequency: u64,
    pub invariant: bool,
}

/// CPUID 0x80000007 EDX[8]: invariant TSC (constant rate across P/C-states).
#[cfg(target_arch = "x86_64")]
pub fn has_invariant_tsc() -> bool {
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
