//! Hardware Initialization Layer
//!
//! Self-contained platform initialization. After ExitBootServices, we are the
//! authority for memory, timers, and device access.
//!
//! # Architecture
//!
//! ```text
//! UEFI hands off:
//!   - Memory map (we import it, then own it)
//!   - Framebuffer (optional)
//!   - ACPI tables pointer
//!
//! We do everything else:
//!   - TSC calibration (PIT-based)
//!   - Memory management (our registry, mirrors UEFI services)
//!   - PCI enumeration
//!   - DMA allocation
//!   - E820 export for Linux
//! ```
//!
//! # Memory Registry
//!
//! ```ignore
//! use morpheus_hwinit::memory::{MemoryRegistry, AllocateType, MemoryType};
//!
//! // Initialize from UEFI map (call once after EBS)
//! unsafe {
//!     init_global_registry(map_ptr, map_size, desc_size, desc_version);
//! }
//!
//! // Now use like UEFI memory services
//! let registry = unsafe { global_registry_mut() };
//!
//! // Allocate pages
//! let dma_addr = registry.alloc_dma_pages(4)?;
//!
//! // Query memory
//! let total = registry.total_memory();
//! let free = registry.free_memory();
//!
//! // Export E820 for Linux
//! let count = registry.export_e820(&mut e820_buffer);
//! ```
//!
//! # Platform Init (Full Initialization)
//!
//! ```ignore
//! use morpheus_hwinit::{platform_init_selfcontained, SelfContainedConfig};
//!
//! let config = SelfContainedConfig {
//!     memory_map_ptr: map_ptr,
//!     memory_map_size: map_size,
//!     descriptor_size: desc_size,
//! };
//!
//! let platform = unsafe { platform_init_selfcontained(config)? };
//! ```
//!
//! # What This Crate Does
//!
//! - Memory services (mirrors UEFI: GetMemoryMap, AllocatePages, etc.)
//! - CPU state management (GDT, IDT, TSS)
//! - Interrupt controller setup (PIC remapping)
//! - Heap allocator (backed by MemoryRegistry)
//! - TSC calibration via PIT (no UEFI needed)
//! - PCI enumeration (bus/device/function scanning)
//! - BAR decoding and device classification
//! - Bus mastering enablement
//! - E820 export for Linux handoff
//! - Synchronization primitives (spinlocks, etc.)
//!
//! # What This Crate Does NOT Do
//!
//! - Device-specific register programming
//! - Protocol logic (Ethernet, SCSI, etc.)
//! - RX/TX processing

#![no_std]
#![allow(dead_code)]

pub mod cpu;
pub mod dma;
pub mod heap;
pub mod memory;
pub mod pci;
pub mod platform;
pub mod serial;
pub mod sync;

// ═══════════════════════════════════════════════════════════════════════════
// CPU RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use cpu::{barriers, cache, mmio, pio, tsc};
pub use cpu::tsc::calibrate_tsc_pit;
pub use cpu::gdt::{init_gdt, KERNEL_CS, KERNEL_DS};
pub use cpu::idt::{init_idt, enable_interrupts, disable_interrupts, interrupts_enabled};
pub use cpu::pic::{init_pic, enable_irq, disable_irq, send_eoi, PIC1_VECTOR_OFFSET};

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use memory::{
    // The registry
    MemoryRegistry,
    init_global_registry,
    global_registry,
    global_registry_mut,
    is_registry_initialized,

    // Types
    MemoryType,
    MemoryDescriptor,
    MemoryAttribute,
    AllocateType,
    MemoryError,

    // E820
    E820Type,
    E820Entry,

    // Constants
    PAGE_SIZE,
    PAGE_SHIFT,

    // Legacy compatibility
    PhysicalAllocator,
    PhysicalMemoryMap,
    MemoryRegion,
    parse_uefi_memory_map,
    fallback_allocator,
};

// ═══════════════════════════════════════════════════════════════════════════
// HEAP RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use heap::{HeapAllocator, init_heap, init_heap_with_buffer, is_heap_initialized, heap_stats};

// ═══════════════════════════════════════════════════════════════════════════
// SYNC RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use sync::{SpinLock, SpinLockGuard, RawSpinLock, Once, Lazy, InterruptGuard, without_interrupts};

// ═══════════════════════════════════════════════════════════════════════════
// DMA RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use dma::DmaRegion;

// ═══════════════════════════════════════════════════════════════════════════
// PCI RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use pci::{PciAddr, pci_cfg_read8, pci_cfg_read16, pci_cfg_read32};
pub use pci::{pci_cfg_write8, pci_cfg_write16, pci_cfg_write32};

// ═══════════════════════════════════════════════════════════════════════════
// PLATFORM INIT RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use platform::{
    // Self-contained entry (recommended)
    platform_init_selfcontained,
    SelfContainedConfig,

    // Legacy entry (external DMA/TSC)
    platform_init,
    PlatformConfig,

    // Common types
    InitError,
    NetDeviceType,
    BlkDeviceType,
    PlatformInit,
    PreparedNetDevice,
    PreparedBlkDevice,
};
