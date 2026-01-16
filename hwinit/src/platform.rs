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
//! │  7. Allocate DMA region (DMA legal)                          │
//! │  8. Enable bus mastering on PCI devices                      │
//! │                                                              │
//! │  Result: Machine is SANE. Drivers just do driver work.       │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Non-responsibilities
//!
//! This module does NOT:
//! - Classify devices (that's driver layer)
//! - Initialize specific hardware (that's driver layer)
//! - Know about virtio, e1000, AHCI, etc. (that's driver layer)
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
//! //
//! // Driver layer can now scan PCI and claim devices.
//! ```

use crate::dma::DmaRegion;
use crate::memory::{
    PhysicalAllocator, fallback_allocator,
    init_global_registry, global_registry_mut, MemoryType, AllocateType,
};
use crate::cpu::tsc::calibrate_tsc_pit;
use crate::cpu::gdt::init_gdt;
use crate::cpu::idt::init_idt;
use crate::cpu::pic::init_pic;
use crate::heap::init_heap;
use crate::pci::{pci_cfg_read16, pci_cfg_write16, PciAddr, offset};
use crate::serial::{puts, put_hex32, put_hex64, newline};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// PCI command register bits
const CMD_MEM_SPACE: u16 = 1 << 1;
const CMD_BUS_MASTER: u16 = 1 << 2;

/// Stack sizes for CPU state
const KERNEL_STACK_SIZE: usize = 64 * 1024;  // 64KB kernel stack
const IST1_STACK_SIZE: usize = 16 * 1024;    // 16KB IST1 for critical exceptions
const HEAP_SIZE: usize = 4 * 1024 * 1024;    // 4MB initial heap
const DMA_SIZE: usize = 2 * 1024 * 1024;     // 2MB DMA region

// ═══════════════════════════════════════════════════════════════════════════
// TYPES
// ═══════════════════════════════════════════════════════════════════════════

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

/// Platform configuration input (legacy - externally allocated).
pub struct PlatformConfig {
    pub dma_base: *mut u8,
    pub dma_bus: u64,
    pub dma_size: usize,
    pub tsc_freq: u64,
}

/// Platform initialization result.
///
/// Contains only platform resources. No device information.
/// Drivers are responsible for their own device enumeration.
pub struct PlatformInit {
    /// TSC frequency in Hz
    pub tsc_freq: u64,
    /// DMA region (identity-mapped, safe for device DMA)
    pub dma_region: DmaRegion,
    /// Physical allocator for additional allocations
    pub allocator: PhysicalAllocator,
}

/// Initialization error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    InvalidDmaRegion,
    TscCalibrationFailed,
    NoFreeMemory,
    MemoryRegistryFailed,
}

// ═══════════════════════════════════════════════════════════════════════════
// SELF-CONTAINED ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════════

/// Self-contained platform initialization.
///
/// After this returns, the machine is SANE:
/// - CPU state is ours (GDT/IDT/TSS)
/// - Memory is ours (registry, heap)
/// - Interrupts are sane (PIC remapped)
/// - Bus mastering enabled on PCI devices
/// - DMA is legal (identity-mapped region allocated)
///
/// Drivers can now scan PCI and do their own device detection.
///
/// # Safety
/// - Must be called IMMEDIATELY after ExitBootServices
/// - Memory map must be valid
/// - Must be called exactly once
pub unsafe fn platform_init_selfcontained(
    config: SelfContainedConfig
) -> Result<PlatformInit, InitError> {
    puts("[HWINIT] ═══════════════════════════════════════════════\n");
    puts("[HWINIT] FULL PLATFORM INIT - TAKING OWNERSHIP\n");
    puts("[HWINIT] ═══════════════════════════════════════════════\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 1: MEMORY - Parse UEFI map, we own physical memory now
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
    puts("[HWINIT]   calling init_gdt...\n");
    init_gdt(kernel_stack_top, ist1_stack_top);
    puts("[HWINIT]   GDT loaded\n");

    // Load our IDT with exception handlers
    init_idt();
    puts("[HWINIT]   IDT loaded\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 3: INTERRUPTS - PIC remapped, sane vectors
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 3: Interrupt controller\n");

    init_pic();
    puts("[HWINIT]   PIC remapped (IRQ0-7 -> 0x20, IRQ8-15 -> 0x28)\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 4: HEAP - GlobalAlloc works after this
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 4: Heap allocator\n");

    // init_heap allocates from registry itself
    if let Err(e) = init_heap(HEAP_SIZE) {
        puts("[HWINIT] ERROR: heap init failed: ");
        puts(e);
        puts("\n");
        return Err(InitError::NoFreeMemory);
    }
    puts("[HWINIT]   heap initialized (");
    put_hex32((HEAP_SIZE / 1024) as u32);
    puts(" KB)\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 5: TSC - Calibrate for timing
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 5: TSC calibration\n");

    let tsc_freq = calibrate_tsc_pit();
    if tsc_freq == 0 {
        return Err(InitError::TscCalibrationFailed);
    }
    puts("[HWINIT]   TSC: ");
    put_hex64(tsc_freq);
    puts(" Hz (");
    put_hex32((tsc_freq / 1_000_000) as u32);
    puts(" MHz)\n");

    // ─────────────────────────────────────────────────────────────────────
    // PHASE 6: DMA - Allocate identity-mapped region for device DMA
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 6: DMA region\n");

    let dma_pages = ((DMA_SIZE + 4095) / 4096) as u64;
    let dma_phys = registry.allocate_pages(
        AllocateType::MaxAddress(0x100000000), // Under 4GB for 32-bit DMA
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
    // PHASE 7: PCI - Enable bus mastering on all devices
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] Phase 7: PCI bus mastering\n");

    let devices_enabled = enable_all_pci_devices();
    puts("[HWINIT]   enabled ");
    put_hex32(devices_enabled as u32);
    puts(" devices\n");

    // ─────────────────────────────────────────────────────────────────────
    // DONE - Machine is sane
    // ─────────────────────────────────────────────────────────────────────
    puts("[HWINIT] ═══════════════════════════════════════════════\n");
    puts("[HWINIT] PLATFORM READY - Drivers may proceed\n");
    puts("[HWINIT] ═══════════════════════════════════════════════\n");

    // Legacy allocator for backward compatibility
    let allocator = fallback_allocator();

    Ok(PlatformInit {
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

    // Enable bus mastering on all devices
    enable_all_pci_devices();

    // Legacy mode: no allocator (caller managed memory)
    let allocator = fallback_allocator();

    Ok(PlatformInit {
        tsc_freq: config.tsc_freq,
        dma_region,
        allocator,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// PCI BUS MASTERING (platform responsibility - NOT device classification)
// ═══════════════════════════════════════════════════════════════════════════

/// Enable memory space and bus mastering on ALL PCI devices.
///
/// This is platform responsibility - making devices capable of DMA.
/// Device classification and driver binding is NOT our job.
unsafe fn enable_all_pci_devices() -> usize {
    let mut count = 0usize;

    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let addr = PciAddr::new(bus, device, 0);
            let vendor = pci_cfg_read16(addr, offset::VENDOR_ID);
            
            if vendor == 0xFFFF || vendor == 0x0000 {
                continue;
            }

            // Enable this device
            enable_bus_mastering(addr);
            count += 1;

            // Check for multi-function
            let header_type = pci_cfg_read16(addr, offset::HEADER_TYPE) as u8;
            if (header_type & 0x80) != 0 {
                for function in 1..8u8 {
                    let faddr = PciAddr::new(bus, device, function);
                    let v = pci_cfg_read16(faddr, offset::VENDOR_ID);
                    if v != 0xFFFF && v != 0x0000 {
                        enable_bus_mastering(faddr);
                        count += 1;
                    }
                }
            }
        }
    }

    count
}

/// Enable memory space and bus mastering on a device.
fn enable_bus_mastering(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    let new_cmd = cmd | CMD_MEM_SPACE | CMD_BUS_MASTER;
    if cmd != new_cmd {
        pci_cfg_write16(addr, offset::COMMAND, new_cmd);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC API FOR DRIVERS
// ═══════════════════════════════════════════════════════════════════════════

impl PlatformInit {
    /// Get DMA region for device transfers.
    pub fn dma(&self) -> &DmaRegion {
        &self.dma_region
    }

    /// Get TSC frequency for timing.
    pub fn tsc_freq(&self) -> u64 {
        self.tsc_freq
    }
}
