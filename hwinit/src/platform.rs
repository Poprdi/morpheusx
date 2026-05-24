//! Self-contained hardware init. Memory → GDT/TSS → IDT → PIC → Heap → TSC
//! → DMA → PCI → paging → USB → scheduler → syscalls → reclaim → FS → SMP.
//! Trusts UEFI for nothing past entry. Devices are the driver layer's problem.

use crate::cpu::gdt::init_gdt;
use crate::cpu::idt::{init_idt, set_interrupt_handler};
use crate::cpu::pic::init_pic;
use crate::cpu::sse::enable_sse;
use crate::cpu::tsc::calibrate_tsc_pit;
use crate::dma::DmaRegion;
use crate::heap::init_heap;
use crate::input;
use crate::memory::{
    fallback_allocator, global_registry_mut, init_global_registry, AllocateType, MemoryType,
    PhysicalAllocator,
};
use crate::paging::init_kernel_page_table;
use crate::pci::{offset, pci_cfg_read16, pci_cfg_read32, pci_cfg_read8, pci_cfg_write16, PciAddr};
use crate::process::scheduler::init_scheduler;
use crate::serial::{checkpoint, log_error, log_info, log_ok, log_warn};
use crate::syscall::init_syscall;
use crate::usb::controller::XhciController;
use crate::usb::enumerate::enumerate_and_bind_inputs;
const CMD_MEM_SPACE: u16 = 1 << 1;
const CMD_BUS_MASTER: u16 = 1 << 2;

const KERNEL_STACK_SIZE: usize = 64 * 1024;
// IST1 stack lives in BSS (see gdt.rs) — no heap dependency.
const HEAP_SIZE: usize = 4 * 1024 * 1024;
const DMA_SIZE: usize = 2 * 1024 * 1024;

/// Everything platform init needs. Caller hands over the UEFI map + bootloader
/// PE/stack bounds + ACPI RSDP. We pull our weight from there.
pub struct SelfContainedConfig {
    pub memory_map_ptr: *const u8,
    pub memory_map_size: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
    /// PE image range — buddy stays out so we don't free .text/.data/.bss.
    pub image_base: u64,
    pub image_pages: u64,
    /// Boot stack range — also kept out of the buddy.
    pub stack_base: u64,
    pub stack_pages: u64,
    /// 0 = unavailable.
    pub acpi_rsdp_phys: u64,
}

/// Legacy: caller pre-allocates DMA + TSC. New code uses `SelfContainedConfig`.
pub struct PlatformConfig {
    pub dma_base: *mut u8,
    pub dma_bus: u64,
    pub dma_size: usize,
    pub tsc_freq: u64,
}

pub struct PlatformInit {
    pub tsc_freq: u64,
    pub dma_region: DmaRegion,
    pub allocator: PhysicalAllocator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    InvalidDmaRegion,
    TscCalibrationFailed,
    NoFreeMemory,
    MemoryRegistryFailed,
}

/// Take ownership of the machine. After this, UEFI is dead to us.
pub unsafe fn platform_init_selfcontained(
    config: SelfContainedConfig,
) -> Result<PlatformInit, InitError> {
    // Belt + suspenders: IF=0 in case entry was reached without enter_baremetal.
    core::arch::asm!("cli", options(nomem, nostack));

    // Clear CR0.WP before any buddy work. UEFI leaves some pages R/W=0 in their
    // PTEs; with WP=1 even ring-0 #PFs writing FreeNode into them. Off forever.
    {
        let cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        if cr0 & (1u64 << 16) != 0 {
            core::arch::asm!(
                "mov cr0, {}",
                in(reg) cr0 & !(1u64 << 16),
                options(nomem, nostack),
            );
            // Some CPUs cache WP in TLB entries — full flush.
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
        }
    }

    log_info("BOOT", 100, "taking ownership of platform init");

    // phase 1: memory
    log_info("BOOT", 101, "phase 1/13: memory");

    // Collect live PT/GDT/IDT/stack pages into a hole-punch list before
    // populating the buddy. The buddy writes a 16-byte FreeNode at every
    // free block base (including spares from carve_block splits); landing
    // that on a live page-table page rewrites the first two PTEs → garbage
    // mapping → #PF/#GP on the next TLB miss.

    let (mut hw_holes, mut hw_count) = crate::paging::collect_page_table_pages();

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

    // Boot stack: use bootloader-reported bounds. RSP-guess missed the bottom
    // and let the buddy plant FreeNodes in live LoaderData pages; OVMF's 0xAF
    // scrub then looked like corruption when the nodes were walked.
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
        // Fallback: RSP-based guess with generous margins.
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

    // Insertion sort — hw_count < 100 in practice.
    for i in 1..hw_count {
        let key = hw_holes[i];
        let mut j = i;
        while j > 0 && hw_holes[j - 1] > key {
            hw_holes[j] = hw_holes[j - 1];
            j -= 1;
        }
        hw_holes[j] = key;
    }
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
            log_warn(
                "MEM",
                150,
                "free-list validation detected corruption; dumping map",
            );
            reg.dump_map();
        }
    }

    log_info("BOOT", 102, "phase 2/13: cpu state");
    checkpoint("phase2-begin");

    // Scoped so the registry guard drops before anything else relocks it —
    // holding SpinLock across a re-acquisition deadlocks instantly.
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
    };

    checkpoint("phase2-gdt");
    init_gdt(kernel_stack_top);

    checkpoint("phase2-idt");
    init_idt();

    checkpoint("phase2-sse");
    enable_sse();

    // Per-CPU init must follow GDT (segment state) and precede anything
    // that touches gs:[offset] — scheduler, interrupts, syscalls.
    checkpoint("phase2-lapic-probe");
    {
        use crate::cpu::{apic, per_cpu};

        // Firmware can relocate LAPIC; trust MSR 0x1B over the spec default.
        let actual_base = apic::probe_lapic_base();

        // LAPIC MMIO is identity-mapped by UEFI — safe pre-paging.
        let bsp_lapic_id = unsafe { apic::read_lapic_id() };
        checkpoint("phase2-percpu-init");
        per_cpu::init_bsp(bsp_lapic_id, actual_base);
        checkpoint("phase2-percpu-done");
    }

    log_info("BOOT", 103, "phase 3/13: pic");
    checkpoint("phase3-pic");

    init_pic();
    checkpoint("phase3-done");

    log_info("BOOT", 104, "phase 4/13: heap");
    checkpoint("phase4-heap-begin");

    // GLOBAL_REGISTRY is unlocked here — phase 2 dropped its guard.
    if let Err(e) = init_heap(HEAP_SIZE) {
        log_error("HEAP", 401, e);
        return Err(InitError::NoFreeMemory);
    }
    checkpoint("phase4-heap-done");

    log_info("BOOT", 105, "phase 5/13: tsc");
    checkpoint("phase5-tsc-calibrate");

    let tsc_freq = calibrate_tsc_pit();
    if tsc_freq == 0 {
        return Err(InitError::TscCalibrationFailed);
    }

    // CPUID.80000007H:EDX[8] = invariant TSC. Without it, P-state changes drift
    // the TSC and break delay_us + scheduler timing on real silicon.
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
                log_warn(
                    "TSC",
                    551,
                    "CPU lacks invariant TSC; timing may drift with P-state changes",
                );
            }
        } else {
            log_warn(
                "TSC",
                552,
                "CPUID extended leaf 0x80000007 unavailable; cannot verify invariant TSC",
            );
        }
    }

    checkpoint("phase5-done");
    crate::process::scheduler::set_tsc_frequency(tsc_freq);

    log_info("BOOT", 106, "phase 6/13: dma");
    checkpoint("phase6-dma-alloc");

    let dma_pages = DMA_SIZE.div_ceil(4096) as u64;
    let dma_phys = {
        let mut dma_reg = global_registry_mut();
        dma_reg
            .allocate_pages(
                AllocateType::MaxAddress(0xFFFF_FFFF),
                MemoryType::AllocatedDma,
                dma_pages,
            )
            .map_err(|_| InitError::NoFreeMemory)?
    };

    // Zero the DMA arena. VirtIO reads avail->idx on enable; uninitialized
    // memory there permanently desyncs the driver. Found that one the hard way.
    core::ptr::write_bytes(dma_phys as *mut u8, 0u8, DMA_SIZE);
    checkpoint("phase6-done");

    // Identity-mapped: VA = PA = bus address.
    let dma_region = DmaRegion::new(dma_phys as *mut u8, dma_phys, DMA_SIZE);

    log_info("BOOT", 107, "phase 7/13: pci");
    checkpoint("phase7-pci-begin");

    enable_all_pci_devices();
    checkpoint("phase7-done");

    log_info("BOOT", 108, "phase 8/13: paging");
    checkpoint("phase8-paging-begin");

    init_kernel_page_table();
    checkpoint("phase8-paging-done");

    // Paging is up — remap LAPIC MMIO as UC and bring the BSP LAPIC fully online.
    checkpoint("phase8-lapic-init");
    crate::cpu::apic::init_bsp();
    checkpoint("phase8-done");

    // USB input must enumerate before the scheduler so user processes never
    // see a half-built input subsystem. Synchronous, deterministic.
    log_info("BOOT", 109, "phase 9/13: USB input init");
    checkpoint("phase9-usb-begin");

    // Dump every USB host controller before the xHCI-only scan runs —
    // exposes the EHCI/UHCI/OHCI controllers we don't (yet) drive.
    unsafe {
        crate::pci::dump::dump_usb_controllers();
    }

    {
        use crate::usb;

        input::init();

        // Scan PCI for class 0x0C / subclass 0x03 (Serial Bus / USB).
        // prog_if 0x30 = xHCI; we only attempt that.
        let mut usb_init_count = 0usize;
        for bus in 0..=255u8 {
            for device in 0..32u8 {
                let addr = PciAddr::new(bus, device, 0);
                let class = pci_cfg_read8(addr, 0x0B);
                let subclass = pci_cfg_read8(addr, 0x0A);
                let prog_if = pci_cfg_read8(addr, 0x09);

                if class == 0x0C && subclass == 0x03 {
                    let base_addr = pci_cfg_read32(addr, offset::BAR0) & !0x0F;
                    if base_addr != 0 && base_addr != 0xFFFFFFFF {
                        let tsc_freq = calibrate_tsc_pit();
                        if tsc_freq == 0 {
                            log_warn("USB", 901, "TSC calibration failed for USB");
                            continue;
                        }

                        match unsafe { XhciController::new(base_addr as u64, tsc_freq) } {
                            Ok(mut controller) => {
                                let kbd_for_runtime = match unsafe {
                                    enumerate_and_bind_inputs(&mut controller)
                                } {
                                    Ok(result) => {
                                        if result.keyboard.is_some() {
                                            log_ok("USB", 910, "USB keyboard detected");
                                            usb_init_count += 1;
                                        }
                                        if result.mouse.is_some() {
                                            log_ok("USB", 911, "USB mouse detected");
                                            usb_init_count += 1;
                                        }
                                        if result.keyboard.is_none() && result.mouse.is_none() {
                                            log_info(
                                                "USB",
                                                912,
                                                "USB device found but no HID interface",
                                            );
                                        }
                                        result.keyboard
                                    }
                                    Err(e) => {
                                        log_warn("USB", 920, "USB enumeration failed");
                                        let _ = e;
                                        None
                                    }
                                };
                                // Hand the xHC to the runtime polling module
                                // so the input loop can fetch reports later.
                                // Without this, `controller` drops here and
                                // the enumerated keyboard becomes unreachable.
                                unsafe {
                                    crate::usb::runtime::install_runtime(
                                        controller,
                                        kbd_for_runtime,
                                    );
                                }
                            }
                            Err(e) => {
                                log_warn("USB", 921, "xHCI controller init failed");
                                let _ = e;
                            }
                        }
                    }
                }
            }
        }

        if usb_init_count == 0 {
            log_info("USB", 930, "no USB input devices; PS/2 remains primary");
        } else {
            log_ok("USB", 931, "USB input initialization complete");
        }
    }
    checkpoint("phase9-usb-done");

    log_info("BOOT", 110, "phase 10/13: scheduler");
    checkpoint("phase10-scheduler");

    init_scheduler();
    checkpoint("phase10-done");

    log_info("BOOT", 111, "phase 11/13: syscalls");
    checkpoint("phase10-syscall");

    init_syscall();
    checkpoint("phase10-syscall-done");

    // Interrupts now flow through LAPIC, not the 8259.
    checkpoint("phase10-pic-disable");
    crate::cpu::apic::disable_pic8259();

    // ~100Hz preemption tick. Calibrates against PIT ch2 (no PIC IRQ needed).
    checkpoint("phase10-lapic-timer");
    crate::cpu::apic::setup_timer(100);
    checkpoint("phase10-lapic-timer-done");

    // IDT 0x20 keeps its meaning — same vector, LAPIC-sourced now.
    extern "C" {
        fn irq_timer_isr();
    }
    set_interrupt_handler(0x20, irq_timer_isr as u64, 0, 0);

    use crate::cpu::idt::enable_interrupts;
    checkpoint("phase10-sti");
    enable_interrupts(); // here we go
    checkpoint("phase10-done");

    // Reclaim BootServices{Code,Data} pages into the buddy. Deferred this far
    // so UEFI runtime state is genuinely dead. Page-table pages live in
    // BootServicesData; we collect them as an exclusion set first — writing a
    // FreeNode over a live PML4 entry is a one-way trip to #PF.
    log_info("BOOT", 111, "phase 10.5/13: reclaim boot services ram");
    checkpoint("phase10.5-reclaim-begin");
    // Non-preemptible: timer IRQ is live and we mutate allocator + paging meta.
    crate::cpu::idt::disable_interrupts();
    {
        let (mut pt_pages, pt_count) = crate::paging::collect_page_table_pages();

        // Insertion sort — pt_count < 50.
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
    // Second pass catches any corruption from carve_block splits during reclaim.
    {
        let reg = global_registry_mut();
        reg.validate_free_lists();
    }
    crate::cpu::idt::enable_interrupts();
    checkpoint("phase10.5-done");

    log_info("BOOT", 112, "phase 11/13: helixfs");
    checkpoint("phase11-fs-alloc");
    {
        const ROOT_FS_SIZE: usize = 16 * 1024 * 1024;
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
            Err(_) => log_warn("FS", 412, "root fs init failed; continuing without fs"),
        }
    }
    checkpoint("phase11-done");

    // PID 0's syscall entry reads kernel_syscall_rsp from gs:[0x20].
    {
        let pcpu = crate::cpu::per_cpu::current();
        pcpu.kernel_syscall_rsp = kernel_stack_top;
    }

    log_info("BOOT", 113, "phase 12/13: smp");
    checkpoint("phase12-smp-begin");

    {
        use crate::cpu::{acpi, ap_boot, apic, per_cpu};

        // Authoritative ACPI root from UEFI — no BIOS scavenger hunt.
        acpi::set_rsdp_phys(config.acpi_rsdp_phys);

        let bsp_lapic_id = apic::read_lapic_id();
        checkpoint("phase12-madt-scan");

        // MADT gives the exact enabled-LAPIC set. No brute force, no ghost
        // timeouts, no wasted stacks.
        let madt_result = acpi::discover_ap_lapic_ids(bsp_lapic_id);
        checkpoint("phase12-madt-done");

        if madt_result.count > 0 {
            per_cpu::set_cpu_count(madt_result.count as u32 + 1);
            checkpoint("phase12-start-aps-madt");
            crate::cpu::idt::disable_interrupts();
            ap_boot::start_aps_from_list(&madt_result.ids[..madt_result.count]);
            crate::cpu::idt::enable_interrupts();
        } else {
            // Fallback: CPUID count + brute-force LAPIC enumeration. Works,
            // but slow on sparse topologies.
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

    log_ok("BOOT", 199, "platform ready; drivers may proceed");

    let allocator = fallback_allocator();

    Ok(PlatformInit {
        tsc_freq,
        dma_region,
        allocator,
    })
}

/// Legacy entry: caller pre-allocates DMA and pre-calibrates TSC.
/// Prefer `platform_init_selfcontained` for new code.
pub unsafe fn platform_init(config: PlatformConfig) -> Result<PlatformInit, InitError> {
    if config.dma_base.is_null() || config.dma_size < DmaRegion::MIN_SIZE {
        log_error("BOOT", 900, "invalid dma region");
        return Err(InitError::InvalidDmaRegion);
    }

    let dma_region = DmaRegion::new(config.dma_base, config.dma_bus, config.dma_size);

    enable_all_pci_devices();

    let allocator = fallback_allocator();

    Ok(PlatformInit {
        tsc_freq: config.tsc_freq,
        dma_region,
        allocator,
    })
}

/// Enable MEM + bus mastering on every non-bridge function. Driver binding
/// happens elsewhere.
unsafe fn enable_all_pci_devices() -> usize {
    let mut count = 0usize;

    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let addr = PciAddr::new(bus, device, 0);
            let vendor = pci_cfg_read16(addr, offset::VENDOR_ID);

            if vendor == 0xFFFF || vendor == 0x0000 {
                continue;
            }

            maybe_enable_bus_mastering(addr);
            count += 1;

            // Multi-function bit.
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

/// Skip class 0x06 (bridges). Toggling BM on host/PCI-PCI/ISA bridges has
/// triggered IOMMU faults and stray DMA from shadow BARs on real silicon.
fn maybe_enable_bus_mastering(addr: PciAddr) {
    let class = pci_cfg_read8(addr, 0x0B);
    if class == 0x06 {
        return;
    }
    enable_bus_mastering(addr);
}

fn enable_bus_mastering(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    let new_cmd = cmd | CMD_MEM_SPACE | CMD_BUS_MASTER;
    if cmd != new_cmd {
        pci_cfg_write16(addr, offset::COMMAND, new_cmd);
    }
}

impl PlatformInit {
    pub fn dma(&self) -> &DmaRegion {
        &self.dma_region
    }

    pub fn tsc_freq(&self) -> u64 {
        self.tsc_freq
    }
}
