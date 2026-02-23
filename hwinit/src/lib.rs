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
#![allow(static_mut_refs)]
#![allow(unexpected_cfgs)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::fn_to_numeric_cast)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::new_without_default)]

pub mod cpu;
pub mod dma;
pub mod elf;
pub mod heap;
pub mod memory;
pub mod paging;
pub mod pci;
pub mod platform;
pub mod process;
pub mod serial;
pub mod stdin;
pub mod sync;
pub mod syscall;

// ═══════════════════════════════════════════════════════════════════════════
// CPU RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use cpu::gdt::{init_gdt, KERNEL_CS, KERNEL_DS, USER_CS, USER_DS};
pub use cpu::idt::{
    disable_interrupts, enable_interrupts, init_idt, interrupts_enabled, set_crash_hook,
    CrashHookFn, CrashInfo,
};
pub use cpu::pic::{disable_irq, enable_irq, init_pic, send_eoi, PIC1_VECTOR_OFFSET};
pub use cpu::tsc::calibrate_tsc_pit;
pub use cpu::{barriers, cache, mmio, pio, tsc};

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use memory::{
    fallback_allocator,
    global_registry,
    global_registry_mut,
    init_global_registry,
    is_registry_initialized,

    parse_uefi_memory_map,
    AllocateType,
    E820Entry,

    // E820
    E820Type,
    MemoryAttribute,
    MemoryDescriptor,
    MemoryError,

    MemoryRegion,
    // The registry
    MemoryRegistry,
    // Types
    MemoryType,
    // Legacy compatibility
    PhysicalAllocator,
    PhysicalMemoryMap,
    PAGE_SHIFT,

    // Constants
    PAGE_SIZE,
};

// ═══════════════════════════════════════════════════════════════════════════
// HEAP RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use heap::{heap_stats, init_heap, init_heap_with_buffer, is_heap_initialized, HeapAllocator};

// ═══════════════════════════════════════════════════════════════════════════
// SYNC RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use sync::{
    without_interrupts, InterruptGuard, Lazy, Once, RawSpinLock, SpinLock, SpinLockGuard,
};

// ═══════════════════════════════════════════════════════════════════════════
// DMA RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use dma::DmaRegion;

// ═══════════════════════════════════════════════════════════════════════════
// PCI RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════════
// PAGING RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use paging::{
    init_kernel_page_table, is_paging_initialized, kensure_4k, kernel_page_table,
    kernel_page_table_mut, kmap_2m, kmap_4k, kmap_mmio, kmark_uncacheable, kunmap_4k,
    kvirt_to_phys, MappedPageSize, PageFlags, PageTable, PageTableEntry, PageTableManager,
    VirtAddr,
};

// ═══════════════════════════════════════════════════════════════════════════
// PCI RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use pci::{pci_cfg_read16, pci_cfg_read32, pci_cfg_read8, PciAddr};
pub use pci::{pci_cfg_write16, pci_cfg_write32, pci_cfg_write8};

// ═══════════════════════════════════════════════════════════════════════════
// PROCESS RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use process::{
    block_sleep,
    exit_process,
    init_scheduler,
    scheduler_tick,
    set_tsc_frequency,
    spawn_kernel_thread,
    tsc_frequency,
    wait_for_child,
    BlockReason,
    // CPU context
    CpuContext,
    // Process descriptor
    Process,
    ProcessInfo,
    ProcessState,
    // Scheduler
    Scheduler,
    // Signals
    Signal,
    SignalSet,
    MAX_PROCESSES,
    SCHEDULER,
};

// ELF loader
pub use elf::{load_elf64, validate_elf64, ElfError, ElfImage};

// User process spawning
pub use process::scheduler::spawn_user_process;

// ═══════════════════════════════════════════════════════════════════════════
// SYSCALL RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use syscall::{
    init_syscall,
    // Core syscalls (0-9)
    SYS_ALLOC,
    SYS_EXIT,
    SYS_FREE,
    SYS_GETPID,
    SYS_KILL,
    SYS_READ,
    SYS_SLEEP,
    SYS_WAIT,
    SYS_WRITE,
    SYS_YIELD,
    // HelixFS syscalls (10-21)
    SYS_CLOSE,
    SYS_MKDIR,
    SYS_OPEN,
    SYS_READDIR,
    SYS_RENAME,
    SYS_SEEK,
    SYS_SNAPSHOT,
    SYS_STAT,
    SYS_SYNC,
    SYS_TRUNCATE,
    SYS_UNLINK,
    SYS_VERSIONS,
    // System / process / memory (22-31)
    SYS_CHDIR,
    SYS_CLOCK,
    SYS_DUP,
    SYS_GETCWD,
    SYS_GETPPID,
    SYS_MMAP,
    SYS_MUNMAP,
    SYS_SPAWN,
    SYS_SYSINFO,
    SYS_SYSLOG,
    // Networking stubs (32-41)
    SYS_NIC_INFO,
    SYS_NIC_TX,
    SYS_NIC_RX,
    SYS_NIC_LINK,
    SYS_NIC_MAC,
    SYS_NIC_REFILL,
    SYS_NET_RSVD38,
    SYS_NET_RSVD39,
    SYS_NET_RSVD40,
    SYS_NET_RSVD41,
    // Device / mount stubs (42-45)
    SYS_IOCTL,
    SYS_MOUNT,
    SYS_POLL,
    SYS_UMOUNT,
    // Persistence / introspection (46-51)
    SYS_PERSIST_DEL,
    SYS_PERSIST_GET,
    SYS_PERSIST_INFO,
    SYS_PERSIST_LIST,
    SYS_PERSIST_PUT,
    SYS_PE_INFO,
    // Hardware primitives (52-62)
    SYS_PORT_IN,
    SYS_PORT_OUT,
    SYS_PCI_CFG_READ,
    SYS_PCI_CFG_WRITE,
    SYS_DMA_ALLOC,
    SYS_DMA_FREE,
    SYS_MAP_PHYS,
    SYS_VIRT_TO_PHYS,
    SYS_IRQ_ATTACH,
    SYS_IRQ_ACK,
    SYS_CACHE_FLUSH,
    // Display (63-64)
    SYS_FB_INFO,
    SYS_FB_MAP,
    // Process management (65-68)
    SYS_PS,
    SYS_SIGACTION,
    SYS_SETPRIORITY,
    SYS_GETPRIORITY,
    // CPU features / diagnostics (69-72)
    SYS_CPUID,
    SYS_RDTSC,
    SYS_BOOT_LOG,
    SYS_MEMMAP,
};

// Syscall handler registration APIs — used by the bootloader to wire
// hardware backends that hwinit cannot depend on directly.
pub use syscall::handler::{register_framebuffer, register_nic, FbInfo, NicOps};

// ═══════════════════════════════════════════════════════════════════════════
// PLATFORM INIT RE-EXPORTS
// ═══════════════════════════════════════════════════════════════════════════

pub use platform::{
    // Legacy entry (external DMA/TSC)
    platform_init,
    // Self-contained entry (recommended)
    platform_init_selfcontained,
    // Common types (platform only - no device types)
    InitError,
    PlatformConfig,

    PlatformInit,
    SelfContainedConfig,
};
