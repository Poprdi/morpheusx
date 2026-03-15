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
use crate::pci::{offset, pci_cfg_read8, pci_cfg_read16, pci_cfg_write16, PciAddr};
use crate::process::scheduler::init_scheduler;
use crate::serial::{checkpoint, log_error, log_info, log_ok, log_warn};
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
    /// Physical base of the boot stack allocated by the bootloader.
    /// The entire range [stack_base, stack_base + stack_pages * 4096)
    /// is excluded from the buddy allocator.
    pub stack_base: u64,
    /// Number of 4 KiB pages in the boot stack allocation.
    pub stack_pages: u64,
    /// Physical address of the ACPI RSDP from UEFI configuration table.
    /// 0 means unavailable.
    pub acpi_rsdp_phys: u64,
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
    // Belt-and-suspenders: ensure IF=0 before we touch any memory.
    // enter_baremetal already does `cli` after ExitBootServices, but if
    // anyone ever calls this entry from a different path, we're covered.
    core::arch::asm!("cli", options(nomem, nostack));

    // Clear CR0.WP (Write Protect) BEFORE any buddy operations.
    // UEFI marks page-table pages and some BootServicesCode as read-only
    // (R/W=0 in the PTE).  With WP=1, even Ring 0 code faults when
    // writing to those pages.  The buddy's `list_push` writes FreeNode
    // structs at physical addresses — if a split spare lands on one of
    // these read-only pages, we get #PF.  Clear WP once, early, forever.
    {
        let cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        if cr0 & (1u64 << 16) != 0 {
            core::arch::asm!(
                "mov cr0, {}",
                in(reg) cr0 & !(1u64 << 16),
                options(nomem, nostack),
            );
            // Flush TLB so no stale entries carry the old WP=1 permission
            // model.  Some real CPUs cache the WP bit in TLB entries.
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
        }
    }

    log_info("BOOT", 100, "taking ownership of platform init");

    // phase 1: memory
    log_info("BOOT", 101, "phase 1/12: memory");

    // ── Collect every page the CPU is actively using ──────────────
    //
    // The buddy allocator writes a 16-byte FreeNode header at the base
    // of each free block — including "spare" halves produced by
    // carve_block splits.  If ANY of those addresses is a live page-
    // table page, the write overwrites PTE entries 0-1, corrupting the
    // identity mapping.  Subsequent reads through the broken PTE return
    // garbage → #GP / #PF.
    //
    // Fix: collect PT/GDT/IDT/stack pages BEFORE populating the buddy,
    // and pass them as a sorted hole-punch array to import_uefi_map.
    // The buddy will never touch those addresses — not during initial
    // import, not during any later carve_block split.

    // 1) Page-table pages (PML4, PDPT, PD, PT)
    let (mut hw_holes, mut hw_count) = crate::paging::collect_page_table_pages();

    // 2) GDT page(s)
    {
        let mut buf = [0u8; 10];
        core::arch::asm!("sgdt [{}]", in(reg) buf.as_mut_ptr(), options(nostack));
        let limit = u16::from_le_bytes([buf[0], buf[1]]) as u64;
        let base = u64::from_le_bytes([
            buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
        ]);
        let page_start = base & !0xFFF;
        let page_end = (base + limit + 0xFFF) & !0xFFF;
        let mut p = page_start;
        while p < page_end && hw_count < hw_holes.len() {
            hw_holes[hw_count] = p;
            hw_count += 1;
            p += 4096;
        }
    }

    // 3) IDT page(s)
    {
        let mut buf = [0u8; 10];
        core::arch::asm!("sidt [{}]", in(reg) buf.as_mut_ptr(), options(nostack));
        let limit = u16::from_le_bytes([buf[0], buf[1]]) as u64;
        let base = u64::from_le_bytes([
            buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
        ]);
        let page_start = base & !0xFFF;
        let page_end = (base + limit + 0xFFF) & !0xFFF;
        let mut p = page_start;
        while p < page_end && hw_count < hw_holes.len() {
            hw_holes[hw_count] = p;
            hw_count += 1;
            p += 4096;
        }
    }

    // 4) Boot stack pages — use actual bounds from bootloader, not RSP guess.
    //    The bootloader passes the exact base + page count it allocated.
    //    Guessing from RSP missed the bottom of the stack and let the buddy
    //    write FreeNode headers into live LoaderData pages — OVMF's 0xAF
    //    scrub then looked like corruption when those nodes were walked.
    let boot_stack_base;
    let boot_stack_top;
    if config.stack_base != 0 && config.stack_pages != 0 {
        boot_stack_base = config.stack_base;
        boot_stack_top = config.stack_base + config.stack_pages * 4096;
        let mut p = boot_stack_base;
        while p < boot_stack_top && hw_count < hw_holes.len() {
            hw_holes[hw_count] = p;
            hw_count += 1;
            p += 4096;
        }
    } else {
        // fallback: RSP-based guess (generous margins)
        let rsp: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, nomem));
        }
        boot_stack_base = (rsp & !0xFFF).saturating_sub(256 * 1024);
        boot_stack_top = (rsp + 0xFFF) & !0xFFF;
        let mut p = boot_stack_base;
        while p < boot_stack_top && hw_count < hw_holes.len() {
            hw_holes[hw_count] = p;
            hw_count += 1;
            p += 4096;
        }
    }

    // Deduplicate and sort (insertion sort — hw_count is typically < 100).
    for i in 1..hw_count {
        let key = hw_holes[i];
        let mut j = i;
        while j > 0 && hw_holes[j - 1] > key {
            hw_holes[j] = hw_holes[j - 1];
            j -= 1;
        }
        hw_holes[j] = key;
    }
    // Deduplicate in-place.
    if hw_count > 1 {
        let mut w = 1usize;
        for r in 1..hw_count {
            if hw_holes[r] != hw_holes[w - 1] {
                hw_holes[w] = hw_holes[r];
                w += 1;
            }
        }
        hw_count = w;
    }

    // ── Populate the buddy allocator ─────────────────────────────
    init_global_registry(
        config.memory_map_ptr,
        config.memory_map_size,
        config.descriptor_size,
        config.descriptor_version,
        config.image_base,
        config.image_pages,
        &hw_holes[..hw_count],
    );

    // Validate free-list integrity after import.
    {
        let reg = global_registry_mut();
        let corrupt = reg.validate_free_lists();
        if corrupt > 0 {
            log_warn("MEM", 150, "free-list validation detected corruption; dumping map");
            reg.dump_map();
        }
    }

    // phase 2: cpu
    log_info("BOOT", 102, "phase 2/12: cpu state");
    checkpoint("phase2-begin");

    // Allocate kernel stack inside a narrow scope so GLOBAL_REGISTRY
    // is released before any subsequent init that also calls global_registry_mut().
    // holding a SpinLock guard across a callsite that re-acquires the same
    // lock is a guaranteed deadlock.
    let kernel_stack_top = {
        let mut registry = global_registry_mut();
        let kernel_stack_pages = KERNEL_STACK_SIZE.div_ceil(4096) as u64;
        let kernel_stack_base = registry
            .allocate_pages(
                AllocateType::AnyPages,
                MemoryType::LoaderData,
                kernel_stack_pages,
            )
            .map_err(|_| InitError::NoFreeMemory)?;
        kernel_stack_base + KERNEL_STACK_SIZE as u64
    }; // registry dropped here — GLOBAL_REGISTRY unlocked

    // Load our GDT with TSS
    // IST1 lives in BSS. one less thing to allocate.
    checkpoint("phase2-gdt");
    init_gdt(kernel_stack_top);

    checkpoint("phase2-idt");
    init_idt();

    checkpoint("phase2-sse");
    enable_sse();

    // Initialize BSP per-CPU data before anything touches gs:[offset].
    // Must happen after GDT (for segment state) but before scheduler and
    // interrupt handlers that rely on GS-relative per-CPU fields.
    checkpoint("phase2-lapic-probe");
    {
        use crate::cpu::{apic, per_cpu};

        // probe the actual LAPIC base from MSR 0x1B. firmware can relocate it.
        let actual_base = apic::probe_lapic_base();

        // LAPIC MMIO is identity-mapped by UEFI. safe to read before paging init.
        let bsp_lapic_id = unsafe { apic::read_lapic_id() };
        checkpoint("phase2-percpu-init");
        per_cpu::init_bsp(bsp_lapic_id, actual_base);
        checkpoint("phase2-percpu-done");
    }

    // phase 3: interrupts
    log_info("BOOT", 103, "phase 3/12: pic");
    checkpoint("phase3-pic");

    init_pic();
    checkpoint("phase3-done");

    // phase 4: heap
    log_info("BOOT", 104, "phase 4/12: heap");
    checkpoint("phase4-heap-begin");

    // init_heap allocates from registry itself.
    // GLOBAL_REGISTRY is NOT held here — the kernel stack was allocated
    // in a scoped block above and the guard was dropped before this callsite.
    if let Err(e) = init_heap(HEAP_SIZE) {
        log_error("HEAP", 401, e);
        return Err(InitError::NoFreeMemory);
    }
    checkpoint("phase4-heap-done");

    // phase 5: tsc
    log_info("BOOT", 105, "phase 5/12: tsc");
    checkpoint("phase5-tsc-calibrate");

    let tsc_freq = calibrate_tsc_pit();
    if tsc_freq == 0 {
        return Err(InitError::TscCalibrationFailed);
    }

    // Check for invariant TSC (CPUID.80000007H:EDX[8]).
    // Without it, CPU frequency scaling (P-states, C-states) causes TSC
    // drift — delay_us and scheduler timing will be inaccurate on real HW.
    {
        let max_ext: u32;
        core::arch::asm!(
            "push rbx",
            "mov eax, 0x80000000",
            "cpuid",
            "pop rbx",
            out("eax") max_ext,
            out("ecx") _,
            out("edx") _,
            options(nostack),
        );
        if max_ext >= 0x80000007 {
            let edx: u32;
            core::arch::asm!(
                "push rbx",
                "mov eax, 0x80000007",
                "cpuid",
                "pop rbx",
                out("edx") edx,
                out("eax") _,
                out("ecx") _,
                options(nostack),
            );
            if edx & (1 << 8) == 0 {
                log_warn("TSC", 551, "CPU lacks invariant TSC; timing may drift with P-state changes");
            }
        } else {
            log_warn("TSC", 552, "CPUID extended leaf 0x80000007 unavailable; cannot verify invariant TSC");
        }
    }

    checkpoint("phase5-done");
    crate::process::scheduler::set_tsc_frequency(tsc_freq);

    // phase 6: dma
    log_info("BOOT", 106, "phase 6/12: dma");
    checkpoint("phase6-dma-alloc");

    let dma_pages = DMA_SIZE.div_ceil(4096) as u64;
    let dma_phys = {
        let mut dma_reg = global_registry_mut();
        dma_reg
            .allocate_pages(AllocateType::MaxAddress(0xFFFF_FFFF), MemoryType::AllocatedDma, dma_pages)
            .map_err(|_| InitError::NoFreeMemory)?
    };

    // zero DMA region. VirtIO checks avail->idx on enable and garbage there
    // permanently desyncs the driver. found that one the hard way.
    core::ptr::write_bytes(dma_phys as *mut u8, 0u8, DMA_SIZE);
    checkpoint("phase6-done");

    // identity-mapped: VA = PA = bus address
    let dma_region = DmaRegion::new(dma_phys as *mut u8, dma_phys, DMA_SIZE);

    // phase 7: pci
    log_info("BOOT", 107, "phase 7/12: pci");
    checkpoint("phase7-pci-begin");

    enable_all_pci_devices();
    checkpoint("phase7-done");

    // phase 8: paging
    log_info("BOOT", 108, "phase 8/12: paging");
    checkpoint("phase8-paging-begin");

    init_kernel_page_table();
    checkpoint("phase8-paging-done");

    // Now that paging is initialized, map LAPIC MMIO as uncacheable
    // and fully enable the BSP's LAPIC hardware.
    checkpoint("phase8-lapic-init");
    crate::cpu::apic::init_bsp();
    checkpoint("phase8-done");

    // phase 9: scheduler
    log_info("BOOT", 109, "phase 9/12: scheduler");
    checkpoint("phase9-scheduler");

    init_scheduler();
    checkpoint("phase9-done");

    // phase 10: syscalls
    log_info("BOOT", 110, "phase 10/12: syscalls");
    checkpoint("phase10-syscall");

    init_syscall();
    checkpoint("phase10-syscall-done");

    // Disable PIC — all interrupts flow through LAPIC now.
    checkpoint("phase10-pic-disable");
    crate::cpu::apic::disable_pic8259();

    // LAPIC timer @ ~100 Hz for preemptive scheduling.
    // calibrates against PIT channel 2 (doesn't need PIC IRQ).
    checkpoint("phase10-lapic-timer");
    crate::cpu::apic::setup_timer(100);
    checkpoint("phase10-lapic-timer-done");

    // timer ISR → IDT vector 0x20 (same vector the PIT used, now LAPIC-sourced)
    extern "C" {
        fn irq_timer_isr();
    }
    set_interrupt_handler(0x20, irq_timer_isr as u64, 0, 0);

    use crate::cpu::idt::enable_interrupts;
    checkpoint("phase10-sti");
    enable_interrupts(); // here we go
    checkpoint("phase10-done");

    // phase 10.5: reclaim UEFI BootServices RAM
    //
    // BootServices{Code,Data} pages are legally free after ExitBootServices.
    // We deferred adding them to the buddy until now (well after GDT/IDT/PIC/
    // heap/TSC/paging/scheduler) to let UEFI's boot-time state wind down.
    //
    // CRITICAL: page-table pages live in BootServicesData.  We must NOT
    // write FreeNode into them — doing so corrupts the live PML4/PDPT/PD/PT
    // and the very next instruction that triggers a TLB miss will #GP or #PF.
    // Collect them first, sort, and pass as an exclusion set.
    log_info("BOOT", 111, "phase 10.5/12: reclaim boot services ram");
    checkpoint("phase10.5-reclaim-begin");
    // Keep this whole critical section non-preemptible. The timer IRQ is live
    // after phase 10; reclaim/reserve mutates allocator + paging metadata.
    crate::cpu::idt::disable_interrupts();
    {
        let (mut pt_pages, pt_count) = crate::paging::collect_page_table_pages();

        // Simple insertion sort — pt_count is typically < 50.
        for i in 1..pt_count {
            let key = pt_pages[i];
            let mut j = i;
            while j > 0 && pt_pages[j - 1] > key {
                pt_pages[j] = pt_pages[j - 1];
                j -= 1;
            }
            pt_pages[j] = key;
        }

        let mut reg = global_registry_mut();
        reg.reclaim_boot_services(&pt_pages[..pt_count]);
        reg.validate_free_lists();
    }
    checkpoint("phase10.5-reserve-pt");
    crate::paging::reserve_page_table_pages();
    // second validation: catch corruption introduced by carve_block splits
    {
        let reg = global_registry_mut();
        reg.validate_free_lists();
    }
    crate::cpu::idt::enable_interrupts();
    checkpoint("phase10.5-done");

    // phase 11: filesystem
    log_info("BOOT", 112, "phase 11/12: helixfs");
    checkpoint("phase11-fs-alloc");
    {
        const ROOT_FS_SIZE: usize = 16 * 1024 * 1024; // 16 MB
        let root_fs_pages = (ROOT_FS_SIZE / 4096) as u64;
        let root_fs_base = {
            let mut registry = global_registry_mut();
            registry
                .allocate_pages(
                    AllocateType::AnyPages,
                    MemoryType::LoaderData,
                    root_fs_pages,
                )
                .map_err(|_| InitError::NoFreeMemory)?
        };
        checkpoint("phase11-fs-zero");
        core::ptr::write_bytes(root_fs_base as *mut u8, 0, ROOT_FS_SIZE);
        checkpoint("phase11-fs-mount");
        match morpheus_helix::vfs::global::init_root_fs(root_fs_base as *mut u8, ROOT_FS_SIZE) {
            Ok(()) => log_ok("FS", 112, "bootstrap RAM helixfs mounted at /"),
            Err(_) => {
                log_warn("FS", 412, "root fs init failed; continuing without fs");
                // Non-fatal — system continues without FS.
            }
        }
    }
    checkpoint("phase11-done");

    // Set initial kernel_syscall_rsp for PID 0 via per-CPU data.
    // The syscall entry point reads this from gs:[0x20].
    {
        let pcpu = crate::cpu::per_cpu::current();
        pcpu.kernel_syscall_rsp = kernel_stack_top;
    }

    // phase 12: SMP — bring up Application Processors
    log_info("BOOT", 113, "phase 12/12: smp");
    checkpoint("phase12-smp-begin");

    {
        use crate::cpu::{acpi, ap_boot, apic, per_cpu};

        // UEFI gave us the authoritative ACPI root. no BIOS scavenger hunt.
        acpi::set_rsdp_phys(config.acpi_rsdp_phys);

        let bsp_lapic_id = apic::read_lapic_id();
        checkpoint("phase12-madt-scan");

        // try ACPI MADT first — gives us the exact set of enabled LAPIC IDs.
        // no brute-force, no ghost timeouts, no wasted stacks.
        let madt_result = acpi::discover_ap_lapic_ids(bsp_lapic_id);
        checkpoint("phase12-madt-done");

        if madt_result.count > 0 {
            per_cpu::set_cpu_count(madt_result.count as u32 + 1); // +1 for BSP
            checkpoint("phase12-start-aps-madt");
            crate::cpu::idt::disable_interrupts();
            ap_boot::start_aps_from_list(&madt_result.ids[..madt_result.count]);
            crate::cpu::idt::enable_interrupts();
        } else {
            // fallback: CPUID-based count + brute-force LAPIC ID enumeration.
            // works but slow on sparse topologies.
            let cpu_count = apic::detect_cpu_count();
            per_cpu::set_cpu_count(cpu_count);

            log_warn("SMP", 213, "using CPUID fallback topology scan");

            if cpu_count > 1 {
                checkpoint("phase12-start-aps-cpuid");
                crate::cpu::idt::disable_interrupts();
                ap_boot::start_aps();
                crate::cpu::idt::enable_interrupts();
            } else {
                log_info("SMP", 114, "single-core detected; no AP startup");
            }
        }

        checkpoint("phase12-smp-done");
        log_ok("SMP", 115, "cpu bring-up complete");
    }

    // DONE - Machine is sane
    log_ok("BOOT", 199, "platform ready; drivers may proceed");

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
        log_error("BOOT", 900, "invalid dma region");
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

            // Enable this device (if it's not a bridge)
            maybe_enable_bus_mastering(addr);
            count += 1;

            // Check for multi-function
            let header_type = pci_cfg_read16(addr, offset::HEADER_TYPE) as u8;
            if (header_type & 0x80) != 0 {
                for function in 1..8u8 {
                    let faddr = PciAddr::new(bus, device, function);
                    let v = pci_cfg_read16(faddr, offset::VENDOR_ID);
                    if v != 0xFFFF && v != 0x0000 {
                        maybe_enable_bus_mastering(faddr);
                        count += 1;
                    }
                }
            }
        }
    }

    count
}

/// Check class code and skip bridge devices. Blindly enabling bus mastering
/// on host bridges, PCI-PCI bridges, and ISA bridges can trigger IOMMU
/// faults or stray DMA from shadow BARs on real hardware.
fn maybe_enable_bus_mastering(addr: PciAddr) {
    let class = pci_cfg_read8(addr, 0x0B);  // base class at offset 0x0B
    if class == 0x06 {
        // 0x06 = Bridge device (host, ISA, PCI-PCI, CardBus, etc.)
        // Don't toggle bus mastering — the firmware configured these.
        return;
    }
    enable_bus_mastering(addr);
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
