//! Platform initialization orchestrator.
//!
//! Self-contained hardware init. No UEFI trust after entry.
//! Phases: Memory → GDT/TSS → IDT → PIC → Heap → TSC → DMA → PCI.
//! After this runs, the machine is SANE and drivers can do their work.
//!
//! Does NOT classify or initialize specific devices — that's the driver layer.

use crate::cpu::gdt::init_gdt;
use crate::cpu::idt::{init_idt, set_interrupt_handler};
use crate::cpu::pic::init_pic;
use crate::cpu::sse::enable_sse;
use crate::cpu::tsc::calibrate_tsc_pit;
use crate::dma::DmaRegion;
use crate::heap::init_heap;
use crate::memory::{
    fallback_allocator, global_registry_mut, init_global_registry, AllocateType, MemoryType,
    PhysicalAllocator,
};
use crate::paging::init_kernel_page_table;
use crate::pci::{offset, pci_cfg_read16, pci_cfg_write16, PciAddr};
use crate::process::scheduler::init_scheduler;
use crate::serial::{put_hex32, put_hex64, puts};
use crate::syscall::init_syscall;

// CONSTANTS

/// PCI command register bits
const CMD_MEM_SPACE: u16 = 1 << 1;
const CMD_BUS_MASTER: u16 = 1 << 2;

/// Stack sizes for CPU state
const KERNEL_STACK_SIZE: usize = 64 * 1024; // 64KB kernel stack
                                            // IST1 stack is now a static array in gdt.rs — no heap allocation needed.
const HEAP_SIZE: usize = 4 * 1024 * 1024; // 4MB initial heap
const DMA_SIZE: usize = 2 * 1024 * 1024; // 2MB DMA region

// TYPES

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
    /// Physical base address of the loaded PE image (page-aligned).
    /// All pages in [image_base, image_base + image_pages * 4096) are
    /// reserved from the buddy allocator so our .text/.data/.bss are
    /// never handed out as free memory.
    pub image_base: u64,
    /// Number of 4 KiB pages the PE image occupies (derived from SizeOfImage).
    pub image_pages: u64,
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

// SELF-CONTAINED ENTRY POINT

/// Take ownership of the machine. after this, UEFI is dead to us.
pub unsafe fn platform_init_selfcontained(
    config: SelfContainedConfig,
) -> Result<PlatformInit, InitError> {
    puts("[HWINIT] === TAKING OWNERSHIP ===\n");

    // phase 1: memory
    puts("[HWINIT] Phase 1: Memory\n");

    // exclude PE image from buddy or it'll scribble FreeNode into our .bss
    // (ask me how many hours THAT took to debug)
    init_global_registry(
        config.memory_map_ptr,
        config.memory_map_size,
        config.descriptor_size,
        config.descriptor_version,
        config.image_base,
        config.image_pages,
    );

    if config.image_pages > 0 {
        puts("[MEM] excluded PE image from buddy: ");
        put_hex64(config.image_base);
        puts(" (");
        put_hex32(config.image_pages as u32);
        puts(" pages)\n");
    }

    // reserve page-table pages. UEFI marks them BootServicesData ("free"),
    // but allocating over live PML4 entries is... educational.
    crate::paging::reserve_page_table_pages();

    // reserve the boot stack from the buddy allocator.
    // PID 0 runs on the original UEFI boot stack for its entire lifetime.
    // If the boot stack falls in a Conventional/LoaderData region the buddy
    // will happily hand those pages out, silently corrupting PID 0's live
    // stack frame with page-table entries or kernel-stack ISR frames.
    {
        let rsp: u64;
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, nomem));
        // 128 KiB below RSP (for deep call chains) + 32 KiB above (caller
        // frames and locals living above the current SP in run_desktop).
        let base = (rsp & !0xFFF).saturating_sub(128 * 1024);
        let top = (rsp & !0xFFF) + 32 * 1024;
        let pages = (top - base) / 4096;
        let reg = global_registry_mut();
        reg.reserve_range(base, pages);
        puts("[MEM] reserved boot stack: ");
        put_hex64(base);
        puts(" .. ");
        put_hex64(top);
        puts(" (");
        put_hex32(pages as u32);
        puts(" pages)\n");
    }

    let registry = global_registry_mut();

    // phase 2: cpu
    puts("[HWINIT] Phase 2: CPU state\n");

    // Allocate kernel stack
    let kernel_stack_pages = KERNEL_STACK_SIZE.div_ceil(4096) as u64;
    let kernel_stack_base = registry
        .allocate_pages(
            AllocateType::AnyPages,
            MemoryType::LoaderData,
            kernel_stack_pages,
        )
        .map_err(|_| InitError::NoFreeMemory)?;
    let kernel_stack_top = kernel_stack_base + KERNEL_STACK_SIZE as u64;

    // Load our GDT with TSS
    // IST1 lives in BSS. one less thing to allocate.
    init_gdt(kernel_stack_top);

    init_idt();

    enable_sse();

    // phase 3: interrupts
    puts("[HWINIT] Phase 3: PIC\n");

    init_pic();

    // phase 4: heap
    puts("[HWINIT] Phase 4: Heap\n");

    // init_heap allocates from registry itself
    if let Err(e) = init_heap(HEAP_SIZE) {
        puts("[HWINIT] ERROR: heap init failed: ");
        puts(e);
        puts("\n");
        return Err(InitError::NoFreeMemory);
    }

    // phase 5: tsc
    puts("[HWINIT] Phase 5: TSC\n");

    let tsc_freq = calibrate_tsc_pit();
    if tsc_freq == 0 {
        return Err(InitError::TscCalibrationFailed);
    }

    crate::process::scheduler::set_tsc_frequency(tsc_freq);

    // phase 6: dma
    puts("[HWINIT] Phase 6: DMA\n");

    let dma_pages = DMA_SIZE.div_ceil(4096) as u64;
    let dma_phys = registry
        .allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedDma, dma_pages)
        .map_err(|_| InitError::NoFreeMemory)?;

    // zero DMA region. VirtIO checks avail->idx on enable and garbage there
    // permanently desyncs the driver. found that one the hard way.
    core::ptr::write_bytes(dma_phys as *mut u8, 0u8, DMA_SIZE);

    // identity-mapped: VA = PA = bus address
    let dma_region = DmaRegion::new(dma_phys as *mut u8, dma_phys, DMA_SIZE);

    // phase 7: pci
    puts("[HWINIT] Phase 7: PCI\n");

    enable_all_pci_devices();

    // phase 8: paging
    puts("[HWINIT] Phase 8: Paging\n");

    init_kernel_page_table();

    // phase 9: scheduler
    puts("[HWINIT] Phase 9: Scheduler\n");

    init_scheduler();

    // phase 10: syscalls
    puts("[HWINIT] Phase 10: Syscalls\n");

    init_syscall();

    // PIT @ ~100 Hz for preemptive scheduling
    {
        use crate::cpu::pio::outb;
        const PIT_DIVISOR: u32 = 11931; // ~100 Hz
        outb(0x43, 0x36); // channel 0, lo/hi, mode 3
        outb(0x40, (PIT_DIVISOR & 0xFF) as u8);
        outb(0x40, ((PIT_DIVISOR >> 8) & 0xFF) as u8);
    }

    // timer ISR → IDT vector 0x20 (PIC IRQ 0)
    extern "C" {
        fn irq_timer_isr();
    }
    set_interrupt_handler(0x20, irq_timer_isr as u64, 0, 0);

    // Enable IRQ 0 (PIT timer) via PIC.
    use crate::cpu::pic::enable_irq;
    enable_irq(0); // PIT
    use crate::cpu::idt::enable_interrupts;
    enable_interrupts(); // here we go

    // phase 11: filesystem
    puts("[HWINIT] Phase 11: HelixFS\n");

    {
        const ROOT_FS_SIZE: usize = 16 * 1024 * 1024; // 16 MB
        let root_fs_pages = (ROOT_FS_SIZE / 4096) as u64;
        let registry = global_registry_mut();
        let root_fs_base = registry
            .allocate_pages(
                AllocateType::AnyPages,
                MemoryType::LoaderData,
                root_fs_pages,
            )
            .map_err(|_| InitError::NoFreeMemory)?;

        // Zero the region.
        core::ptr::write_bytes(root_fs_base as *mut u8, 0, ROOT_FS_SIZE);

        match morpheus_helix::vfs::global::init_root_fs(root_fs_base as *mut u8, ROOT_FS_SIZE) {
            Ok(()) => puts("[HWINIT]   HelixFS mounted at /\n"),
            Err(_) => {
                puts("[HWINIT]   WARNING: root FS init failed\n");
                // Non-fatal — system continues without FS.
            }
        }
    }

    // Set initial kernel_syscall_rsp for PID 0.
    {
        extern "C" {
            static mut kernel_syscall_rsp: u64;
        }
        kernel_syscall_rsp = kernel_stack_top;
    }

    // DONE - Machine is sane
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

// LEGACY ENTRY POINT (External DMA/TSC)

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
    // Validate DMA region
    if config.dma_base.is_null() || config.dma_size < DmaRegion::MIN_SIZE {
        puts("[HWINIT] ERROR: invalid DMA region\n");
        return Err(InitError::InvalidDmaRegion);
    }

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

// PCI BUS MASTERING (platform responsibility - NOT device classification)

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

// PUBLIC API FOR DRIVERS

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
