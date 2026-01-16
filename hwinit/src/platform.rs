//! Platform initialization orchestrator.
//!
//! Self-contained hardware init. No UEFI trust after entry.
//! After this runs, the machine is SANE and drivers can do their work.
//!
//! # What This Does
//!
//! ```text
//! UEFI hands off memory map
//!        │
//!        ▼
//! ┌──────────────────────────────────────────────────────────────┐
//! │  platform_init_selfcontained()                               │
//! │                                                              │
//! │  1. Initialize Memory Registry (we own memory now)           │
//! │  2. Set up GDT/TSS (proper long mode segments)               │
//! │  3. Set up IDT (exception handlers ready)                    │
//! │  4. Remap PIC (IRQs won't collide with exceptions)           │
//! │  5. Initialize Heap (GlobalAlloc works)                      │
//! │  6. Calibrate TSC (timing works)                             │
//! │  7. Scan PCI, enable bus mastering (devices ready)           │
//! │  8. Allocate DMA region (DMA legal)                          │
//! │                                                              │
//! │  Result: Machine is SANE. Drivers just do driver work.       │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // After ExitBootServices, call once:
//! let platform = unsafe { platform_init_selfcontained(SelfContainedConfig {
//!     memory_map_ptr: map_ptr,
//!     memory_map_size: map_size,
//!     descriptor_size: desc_size,
//!     descriptor_version: desc_version,
//! })? };
//!
//! // Now safe to use:
//! // - Box, Vec, any heap allocation
//! // - Spinlocks (interrupt-safe)
//! // - DMA transfers
//! // - Device MMIO
//! ```

use crate::dma::DmaRegion;
use crate::memory::{
    PhysicalAllocator, parse_uefi_memory_map, fallback_allocator,
    init_global_registry, global_registry_mut, MemoryType, AllocateType,
};
use crate::cpu::tsc::calibrate_tsc_pit;
use crate::cpu::gdt::init_gdt;
use crate::cpu::idt::init_idt;
use crate::cpu::pic::init_pic;
use crate::heap::init_heap;
use crate::pci::{pci_cfg_read16, pci_cfg_read32, pci_cfg_write16, PciAddr, offset};
use crate::serial::{puts, put_hex32, put_hex64, put_hex8, newline};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// VirtIO vendor ID
const VIRTIO_VENDOR: u16 = 0x1AF4;
/// VirtIO net device IDs
const VIRTIO_NET_LEGACY: u16 = 0x1000;
const VIRTIO_NET_MODERN: u16 = 0x1041;
/// VirtIO block device IDs  
const VIRTIO_BLK_LEGACY: u16 = 0x1001;
const VIRTIO_BLK_MODERN: u16 = 0x1042;

/// Intel vendor ID
const INTEL_VENDOR: u16 = 0x8086;
/// Intel e1000e device IDs (common ones)
const INTEL_I217_LM: u16 = 0x153A;
const INTEL_I218_LM: u16 = 0x155A;
const INTEL_I218_V: u16 = 0x1559;
const INTEL_I219_LM: u16 = 0x156F;
const INTEL_I219_V: u16 = 0x1570;
const INTEL_82579LM: u16 = 0x1502;
const INTEL_82579V: u16 = 0x1503;

/// AHCI class code (mass storage / SATA / AHCI)
const CLASS_AHCI: u32 = 0x010601;

/// PCI command register bits
const CMD_IO_SPACE: u16 = 1 << 0;
const CMD_MEM_SPACE: u16 = 1 << 1;
const CMD_BUS_MASTER: u16 = 1 << 2;

/// Max devices to track
const MAX_NET_DEVICES: usize = 4;
const MAX_BLK_DEVICES: usize = 4;

// ═══════════════════════════════════════════════════════════════════════════
// TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Network device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetDeviceType {
    VirtIO,
    IntelE1000e,
}

/// Block device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlkDeviceType {
    VirtIO,
    Ahci,
}

/// Prepared network device ready for driver init.
#[derive(Debug, Clone, Copy)]
pub struct PreparedNetDevice {
    pub pci_addr: PciAddr,
    pub mmio_base: u64,
    pub device_type: NetDeviceType,
    pub device_id: u16,
}

/// Prepared block device ready for driver init.
#[derive(Debug, Clone, Copy)]
pub struct PreparedBlkDevice {
    pub pci_addr: PciAddr,
    pub mmio_base: u64,
    pub device_type: BlkDeviceType,
    pub device_id: u16,
}

/// Platform configuration input (legacy - externally allocated).
pub struct PlatformConfig {
    pub dma_base: *mut u8,
    pub dma_bus: u64,
    pub dma_size: usize,
    pub tsc_freq: u64,
}

/// Self-contained platform configuration.
/// Pass just the memory map - we do everything else.
pub struct SelfContainedConfig {
    /// Pointer to UEFI memory map (from ExitBootServices)
    pub memory_map_ptr: *const u8,
    /// Size of memory map in bytes
    pub memory_map_size: usize,
    /// Size of each descriptor entry
    pub descriptor_size: usize,
    /// Descriptor version (from UEFI)
    pub descriptor_version: u32,
}

/// Platform initialization result.
pub struct PlatformInit {
    pub net_devices: [Option<PreparedNetDevice>; MAX_NET_DEVICES],
    pub blk_devices: [Option<PreparedBlkDevice>; MAX_BLK_DEVICES],
    pub tsc_freq: u64,
    pub dma_region: DmaRegion,
    /// Physical allocator for additional allocations
    pub allocator: PhysicalAllocator,
}

/// Initialization error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    InvalidDmaRegion,
    NoDevicesFound,
    BarDecodeFailed,
    TscCalibrationFailed,
    NoFreeMemory,
    MemoryRegistryFailed,
    GdtInitFailed,
    IdtInitFailed,
    PicInitFailed,
    HeapInitFailed,
}

// ═══════════════════════════════════════════════════════════════════════════
// SELF-CONTAINED ENTRY POINT (RECOMMENDED)
// ═══════════════════════════════════════════════════════════════════════════

/// Stack sizes for CPU state
const KERNEL_STACK_SIZE: usize = 64 * 1024;  // 64KB kernel stack
const IST1_STACK_SIZE: usize = 16 * 1024;    // 16KB IST1 for critical exceptions
const HEAP_SIZE: usize = 4 * 1024 * 1024;    // 4MB initial heap
const DMA_SIZE: usize = 2 * 1024 * 1024;     // 2MB DMA region

/// Self-contained platform initialization.
///
/// After this returns, the machine is SANE:
/// - CPU state is ours (GDT/IDT/TSS)
/// - Memory is ours (registry, heap)
/// - Interrupts are sane (PIC remapped)
/// - Devices are ready (bus mastering enabled)
/// - DMA is legal (identity-mapped region allocated)
///
/// Drivers can now just do driver work.
///
/// # Safety
/// - Must be called IMMEDIATELY after ExitBootServices
/// - Memory map must be valid
/// - Must be called exactly once
/// - After this, UEFI boot services are GONE
pub unsafe fn platform_init_selfcontained(config: SelfContainedConfig) -> Result<PlatformInit, InitError> {
    puts("[HWINIT] ═══════════════════════════════════════════════\n");
    puts("[HWINIT] FULL PLATFORM INIT - TAKING OWNERSHIP\n");
    puts("[HWINIT] ═══════════════════════════════════════════════\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 1: MEMORY - Become the memory authority
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 1: Memory ownership\n");

    init_global_registry(
        config.memory_map_ptr,
        config.memory_map_size,
        config.descriptor_size,
        config.descriptor_version,
    );
    puts("[HWINIT]   memory registry initialized\n");

    let registry = global_registry_mut();
    let total_mb = registry.total_memory() / (1024 * 1024);
    let free_mb = registry.free_memory() / (1024 * 1024);
    puts("[HWINIT]   total: ");
    put_hex32(total_mb as u32);
    puts(" MB, free: ");
    put_hex32(free_mb as u32);
    puts(" MB\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 2: CPU STATE - Our GDT, our IDT, our rules
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 2: CPU state\n");

    // Allocate kernel stack
    let kernel_stack_pages = ((KERNEL_STACK_SIZE + 4095) / 4096) as u64;
    let kernel_stack_base = registry.allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LoaderData,
        kernel_stack_pages,
    ).map_err(|_| InitError::NoFreeMemory)?;
    let kernel_stack_top = kernel_stack_base + KERNEL_STACK_SIZE as u64;

    // Allocate IST1 stack (for NMI, double fault, machine check)
    let ist1_stack_pages = ((IST1_STACK_SIZE + 4095) / 4096) as u64;
    let ist1_stack_base = registry.allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LoaderData,
        ist1_stack_pages,
    ).map_err(|_| InitError::NoFreeMemory)?;
    let ist1_stack_top = ist1_stack_base + IST1_STACK_SIZE as u64;

    puts("[HWINIT]   kernel stack: ");
    put_hex64(kernel_stack_base);
    puts(" - ");
    put_hex64(kernel_stack_top);
    newline();

    puts("[HWINIT]   IST1 stack: ");
    put_hex64(ist1_stack_base);
    puts(" - ");
    put_hex64(ist1_stack_top);
    newline();

    // Load our GDT with TSS
    init_gdt(kernel_stack_top, ist1_stack_top);
    puts("[HWINIT]   GDT loaded (kernel CS=0x08, DS=0x10)\n");

    // Load our IDT with exception handlers
    init_idt();
    puts("[HWINIT]   IDT loaded (exceptions 0-21 handled)\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 3: INTERRUPTS - PIC remapped, sane vectors
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 3: Interrupt controller\n");

    init_pic();
    puts("[HWINIT]   PIC remapped (IRQ0-7 -> 0x20, IRQ8-15 -> 0x28)\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 4: HEAP - GlobalAlloc works now
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 4: Heap allocator\n");

    init_heap(HEAP_SIZE).map_err(|_| InitError::HeapInitFailed)?;
    puts("[HWINIT]   heap initialized (");
    put_hex32((HEAP_SIZE / 1024) as u32);
    puts(" KB)\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 5: TIMING - TSC calibrated via PIT
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 5: TSC calibration\n");

    let tsc_freq = calibrate_tsc_pit();
    if tsc_freq == 0 {
        puts("[HWINIT]   ERROR: TSC calibration failed\n");
        return Err(InitError::TscCalibrationFailed);
    }
    puts("[HWINIT]   TSC: ");
    put_hex64(tsc_freq);
    puts(" Hz (");
    put_hex32((tsc_freq / 1_000_000) as u32);
    puts(" MHz)\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 6: DMA - Allocate identity-mapped DMA region
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 6: DMA region\n");

    // DMA needs to be below 4GB for device compatibility
    let dma_pages = (DMA_SIZE / 4096) as u64;
    let dma_phys = registry.allocate_pages(
        AllocateType::MaxAddress(0xFFFF_FFFF), // Below 4GB
        MemoryType::LoaderData,
        dma_pages,
    ).map_err(|_| InitError::NoFreeMemory)?;

    puts("[HWINIT]   DMA: ");
    put_hex64(dma_phys);
    puts(" (");
    put_hex32((DMA_SIZE / 1024) as u32);
    puts(" KB)\n");

    // Identity-mapped: CPU address = bus address = physical address
    let dma_region = DmaRegion::new(dma_phys as *mut u8, dma_phys, DMA_SIZE);

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 7: PCI - Scan devices, enable bus mastering
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 7: PCI enumeration\n");

    let (net_devices, blk_devices) = scan_pci_devices()?;

    // ─────────────────────────────────────────────────────────────────────
    // DONE - Machine is sane
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] ═══════════════════════════════════════════════\n");
    puts("[HWINIT] PLATFORM READY - Drivers may proceed\n");
    puts("[HWINIT] ═══════════════════════════════════════════════\n");

    // Legacy allocator for backward compatibility
    let allocator = fallback_allocator();

    Ok(PlatformInit {
        net_devices,
        blk_devices,
        tsc_freq,
        dma_region,
        allocator,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY ENTRY POINT (External DMA/TSC)
// ═══════════════════════════════════════════════════════════════════════════

/// Platform initialization entry point (legacy - external DMA/TSC).
///
/// Caller provides pre-allocated DMA and pre-calibrated TSC.
/// Use `platform_init_selfcontained` for fully autonomous init.
///
/// # Safety
/// - Must be called after ExitBootServices
/// - DMA region must be valid and identity-mapped
/// - Must be called exactly once
pub unsafe fn platform_init(config: PlatformConfig) -> Result<PlatformInit, InitError> {
    puts("[HWINIT] platform_init (legacy) start\n");

    // Validate DMA region
    if config.dma_base.is_null() || config.dma_size < DmaRegion::MIN_SIZE {
        puts("[HWINIT] ERROR: invalid DMA region\n");
        return Err(InitError::InvalidDmaRegion);
    }

    puts("[HWINIT] DMA base=");
    put_hex64(config.dma_base as u64);
    puts(" size=");
    put_hex32(config.dma_size as u32);
    newline();

    let dma_region = DmaRegion::new(config.dma_base, config.dma_bus, config.dma_size);

    // Use shared PCI scan
    let (net_devices, blk_devices) = scan_pci_devices()?;

    // Legacy mode: no allocator (caller managed memory)
    let allocator = fallback_allocator();

    Ok(PlatformInit {
        net_devices,
        blk_devices,
        tsc_freq: config.tsc_freq,
        dma_region,
        allocator,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// PCI SCANNING (shared by both entry points)
// ═══════════════════════════════════════════════════════════════════════════

/// Scan PCI bus and return discovered devices.
unsafe fn scan_pci_devices() -> Result<(
    [Option<PreparedNetDevice>; MAX_NET_DEVICES],
    [Option<PreparedBlkDevice>; MAX_BLK_DEVICES],
), InitError> {
    let mut net_devices = [None; MAX_NET_DEVICES];
    let mut blk_devices = [None; MAX_BLK_DEVICES];
    let mut net_count = 0usize;
    let mut blk_count = 0usize;

    puts("[HWINIT] scanning PCI bus...\n");

    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let addr = PciAddr::new(bus, device, 0);
            let vendor = pci_cfg_read16(addr, offset::VENDOR_ID);
            
            if vendor == 0xFFFF || vendor == 0x0000 {
                continue;
            }

            let device_id = pci_cfg_read16(addr, offset::DEVICE_ID);
            let header_type = pci_cfg_read16(addr, offset::HEADER_TYPE) as u8;
            let multi_func = (header_type & 0x80) != 0;
            let max_func = if multi_func { 8 } else { 1 };

            for function in 0..max_func {
                let addr = PciAddr::new(bus, device, function);
                
                if function > 0 {
                    let v = pci_cfg_read16(addr, offset::VENDOR_ID);
                    if v == 0xFFFF || v == 0x0000 {
                        continue;
                    }
                }

                let dev_id = if function == 0 { device_id } else {
                    pci_cfg_read16(addr, offset::DEVICE_ID)
                };

                // Check for network devices
                if net_count < MAX_NET_DEVICES {
                    if let Some(net_dev) = classify_net_device(addr, vendor, dev_id) {
                        puts("[HWINIT] NET ");
                        put_hex8(bus);
                        puts(":");
                        put_hex8(device);
                        puts(".");
                        put_hex8(function);
                        puts(" vid=");
                        put_hex32(vendor as u32);
                        puts(" did=");
                        put_hex32(dev_id as u32);
                        puts(" bar0=");
                        put_hex64(net_dev.mmio_base);
                        newline();
                        enable_device(addr);
                        net_devices[net_count] = Some(net_dev);
                        net_count += 1;
                    }
                }

                // Check for block devices
                if blk_count < MAX_BLK_DEVICES {
                    if let Some(blk_dev) = classify_blk_device(addr, vendor, dev_id) {
                        puts("[HWINIT] BLK ");
                        put_hex8(bus);
                        puts(":");
                        put_hex8(device);
                        puts(".");
                        put_hex8(function);
                        puts(" vid=");
                        put_hex32(vendor as u32);
                        puts(" did=");
                        put_hex32(dev_id as u32);
                        puts(" bar0=");
                        put_hex64(blk_dev.mmio_base);
                        newline();
                        enable_device(addr);
                        blk_devices[blk_count] = Some(blk_dev);
                        blk_count += 1;
                    }
                }
            }
        }
    }

    puts("[HWINIT] scan complete: net=");
    put_hex32(net_count as u32);
    puts(" blk=");
    put_hex32(blk_count as u32);
    newline();

    Ok((net_devices, blk_devices))
}

// ═══════════════════════════════════════════════════════════════════════════
// DEVICE CLASSIFICATION
// ═══════════════════════════════════════════════════════════════════════════

fn classify_net_device(addr: PciAddr, vendor: u16, device_id: u16) -> Option<PreparedNetDevice> {
    let device_type = match (vendor, device_id) {
        (VIRTIO_VENDOR, VIRTIO_NET_LEGACY | VIRTIO_NET_MODERN) => NetDeviceType::VirtIO,
        (INTEL_VENDOR, id) if is_intel_e1000e(id) => NetDeviceType::IntelE1000e,
        _ => return None,
    };

    let mmio_base = read_bar0(addr)?;

    Some(PreparedNetDevice {
        pci_addr: addr,
        mmio_base,
        device_type,
        device_id,
    })
}

fn classify_blk_device(addr: PciAddr, vendor: u16, device_id: u16) -> Option<PreparedBlkDevice> {
    let device_type = match (vendor, device_id) {
        (VIRTIO_VENDOR, VIRTIO_BLK_LEGACY | VIRTIO_BLK_MODERN) => BlkDeviceType::VirtIO,
        _ => {
            // Check for AHCI by class code
            let class = pci_cfg_read32(addr, offset::CLASS_CODE) >> 8;
            if class == CLASS_AHCI {
                BlkDeviceType::Ahci
            } else {
                return None;
            }
        }
    };

    let mmio_base = read_bar0(addr)?;

    Some(PreparedBlkDevice {
        pci_addr: addr,
        mmio_base,
        device_type,
        device_id,
    })
}

fn is_intel_e1000e(device_id: u16) -> bool {
    matches!(device_id,
        INTEL_I217_LM | INTEL_I218_LM | INTEL_I218_V |
        INTEL_I219_LM | INTEL_I219_V | INTEL_82579LM | INTEL_82579V |
        0x153B | 0x15A0 | 0x15A1 | 0x15A2 | 0x15A3 |
        0x15B7 | 0x15B8 | 0x15B9 | 0x15D7 | 0x15D8 | 0x15E3 |
        0x15F9 | 0x15FA | 0x15FB | 0x15FC | 0x1A1E | 0x1A1F
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// DEVICE ENABLEMENT
// ═══════════════════════════════════════════════════════════════════════════

/// Enable memory space and bus mastering on device.
fn enable_device(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    let new_cmd = cmd | CMD_MEM_SPACE | CMD_BUS_MASTER;
    if cmd != new_cmd {
        pci_cfg_write16(addr, offset::COMMAND, new_cmd);
        puts("[HWINIT]   cmd ");
        put_hex32(cmd as u32);
        puts(" -> ");
        put_hex32(new_cmd as u32);
        newline();
    }
}

/// Read BAR0 and decode MMIO base address.
fn read_bar0(addr: PciAddr) -> Option<u64> {
    let bar0 = pci_cfg_read32(addr, offset::BAR0);
    
    if bar0 == 0 || bar0 == 0xFFFFFFFF {
        return None;
    }

    // Check if memory BAR (bit 0 = 0)
    if (bar0 & 1) != 0 {
        return None; // I/O BAR, not MMIO
    }

    let bar_type = (bar0 >> 1) & 3;
    
    match bar_type {
        0 => {
            // 32-bit BAR
            Some((bar0 & 0xFFFFFFF0) as u64)
        }
        2 => {
            // 64-bit BAR
            let bar1 = pci_cfg_read32(addr, offset::BAR1);
            let base = ((bar1 as u64) << 32) | ((bar0 & 0xFFFFFFF0) as u64);
            Some(base)
        }
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// IMPL
// ═══════════════════════════════════════════════════════════════════════════

impl PlatformInit {
    /// Get first network device of given type.
    pub fn find_net(&self, dtype: NetDeviceType) -> Option<&PreparedNetDevice> {
        self.net_devices.iter().flatten().find(|d| d.device_type == dtype)
    }

    /// Get first block device of given type.
    pub fn find_blk(&self, dtype: BlkDeviceType) -> Option<&PreparedBlkDevice> {
        self.blk_devices.iter().flatten().find(|d| d.device_type == dtype)
    }

    /// Count of network devices found.
    pub fn net_count(&self) -> usize {
        self.net_devices.iter().filter(|d| d.is_some()).count()
    }

    /// Count of block devices found.
    pub fn blk_count(&self) -> usize {
        self.blk_devices.iter().filter(|d| d.is_some()).count()
    }
}
