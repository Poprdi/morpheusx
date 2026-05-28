//! Phase 1-9 bring-up: memory → GDT/TSS → IDT → PIC → heap → TSC → DMA → PCI →
//! paging → USB. After this returns, UEFI is dead to us.
//!
//! Portable phases 10/11/11b live in `morpheus_kernel::init`; the bootloader
//! drives 10.5 reclaim and phase 12 SMP via HAL methods directly.
//!
//! Phase 9 USB hooks: bootloader wires fn-pointers for TSC publish, HID input
//! init, xHCI MSI-X, and xHCI runtime polling before calling init. Missing
//! hooks are no-ops (single-controller polling-only mode still works).

use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use morpheus_hal_api::Hal;
use morpheus_xhci::enumerate::{enumerate_and_bind_inputs, UsbInputDevice};
use morpheus_xhci::XhciController;

use crate::cpu::gdt::init_gdt;
use crate::cpu::idt::init_idt;
use crate::cpu::pic::init_pic;
use crate::cpu::sse::enable_sse;
use crate::cpu::tsc::calibrate_tsc_pit;
use crate::cpu::{apic, per_cpu};
use crate::dma::DmaRegion;
use crate::heap::init_heap;
use crate::memory::{
    fallback_allocator, global_registry_mut, init_global_registry, AllocateType, MemoryType,
    PhysicalAllocator,
};
use crate::paging::{collect_page_table_pages, init_kernel_page_table};
use crate::pci::{
    dump, offset, pci_cfg_read16, pci_cfg_read32, pci_cfg_read8, pci_cfg_write16, PciAddr,
};
use crate::serial::{checkpoint, log_error, log_info, log_ok, log_warn};
use crate::HalImpl;

const CMD_MEM_SPACE: u16 = 1 << 1;
const CMD_BUS_MASTER: u16 = 1 << 2;

const KERNEL_STACK_SIZE: usize = 64 * 1024;
// IST1 stack lives in BSS (see gdt.rs) — no heap dependency.
const HEAP_SIZE: usize = 4 * 1024 * 1024;
const DMA_SIZE: usize = 2 * 1024 * 1024;

/// Lets `morpheus-xhci` log through our UART without depending on this crate.
fn xhci_log_sink(
    level: morpheus_xhci::logger::LogLevel,
    component: &'static str,
    code: u16,
    msg: &'static str,
) {
    match level {
        morpheus_xhci::logger::LogLevel::Info => crate::serial::log_info(component, code, msg),
        morpheus_xhci::logger::LogLevel::Ok => crate::serial::log_ok(component, code, msg),
        morpheus_xhci::logger::LogLevel::Warn => crate::serial::log_warn(component, code, msg),
        morpheus_xhci::logger::LogLevel::Error => crate::serial::log_error(component, code, msg),
    }
}

/// `static mut` rather than `spin::Once<T>` — local sync primitives lack a
/// value-bearing Once. Caller-enforced single-threaded init.
static mut HAL: Option<HalImpl> = None;
static HAL_INIT_DONE: AtomicBool = AtomicBool::new(false);

/// Idempotent.
///
/// # Safety
/// Single-threaded; before any other CPU is up.
// SAFETY: single-threaded boot/init context; static accessed before APs start, no aliasing.
#[allow(static_mut_refs)]
pub unsafe fn init() -> &'static dyn Hal {
    if !HAL_INIT_DONE.load(Ordering::Acquire) {
        HAL = Some(HalImpl::new());
        morpheus_xhci::logger::install(xhci_log_sink);
        HAL_INIT_DONE.store(true, Ordering::Release);
    }
    HAL.as_ref().expect("HAL not initialized") as &'static dyn Hal
}

pub struct SelfContainedConfig {
    pub memory_map_ptr: *const u8,
    pub memory_map_size: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
    /// Buddy excludes this so .text/.data/.bss don't get freed.
    pub image_base: u64,
    pub image_pages: u64,
    /// Boot stack — also excluded.
    pub stack_base: u64,
    pub stack_pages: u64,
    /// 0 = unavailable.
    pub acpi_rsdp_phys: u64,
}

pub struct PlatformInit {
    pub tsc_freq: u64,
    pub dma_region: DmaRegion,
    pub allocator: PhysicalAllocator,
    /// Seeds the per-CPU `kernel_syscall_rsp` slot in `morpheus_kernel::init`.
    pub kernel_stack_top: u64,
    /// Forwarded so the bootloader can drive phase 12 MADT / AP bring-up.
    pub acpi_rsdp_phys: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    InvalidDmaRegion,
    TscCalibrationFailed,
    NoFreeMemory,
    MemoryRegistryFailed,
}

// Phase 9 USB fn-pointer hooks. Bodies live in morpheus-xhci / morpheus-kernel;
// this module keeps an upward-dep-free shape via these pointers.

/// `morpheus_kernel::schedular::set_tsc_frequency`.
type TscFreqPublishHook = unsafe fn(u64);
/// `input::init()` — HID input event ringbuf setup.
type InputInitHook = unsafe fn();
/// `usb::msi::wire_msix(pci_addr, rt_base)`.
type XhciMsixHook = unsafe fn(PciAddr, u64);
/// `usb::runtime::install_runtime(controller, keyboard)`. Hook owns the controller after the call.
type XhciRuntimeHook = unsafe fn(XhciController, Option<UsbInputDevice>);

static TSC_FREQ_PUBLISH_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static INPUT_INIT_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static XHCI_MSIX_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static XHCI_RUNTIME_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Last writer wins.
pub fn set_tsc_freq_publish_hook(f: TscFreqPublishHook) {
    TSC_FREQ_PUBLISH_HOOK.store(f as *mut (), Ordering::Release);
}

/// Last writer wins.
pub fn set_input_init_hook(f: InputInitHook) {
    INPUT_INIT_HOOK.store(f as *mut (), Ordering::Release);
}

/// Last writer wins.
pub fn set_xhci_msix_hook(f: XhciMsixHook) {
    XHCI_MSIX_HOOK.store(f as *mut (), Ordering::Release);
}

/// Last writer wins.
pub fn set_xhci_runtime_hook(f: XhciRuntimeHook) {
    XHCI_RUNTIME_HOOK.store(f as *mut (), Ordering::Release);
}

/// # Safety
/// Call exactly once on the BSP, pre-scheduler, single-threaded, immediately
/// after `ExitBootServices`. `config` must describe the real UEFI memory map
/// and framebuffer. Takes over interrupts, paging, and CR0; nothing else may be
/// touching that hardware.
pub unsafe fn platform_init_selfcontained(
    config: SelfContainedConfig,
) -> Result<PlatformInit, InitError> {
    // Belt + suspenders in case entry skipped enter_baremetal.
    // SAFETY: BSP, pre-scheduler, single-threaded.
    core::arch::asm!("cli", options(nomem, nostack));

    // Clear CR0.WP forever. UEFI leaves some PTEs R/W=0; with WP=1 even ring-0
    // #PFs trying to write FreeNode into them.
    {
        let cr0: u64;
        // SAFETY: read CR0 — no memory effect.
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        if cr0 & (1u64 << 16) != 0 {
            // SAFETY: BSP, pre-paging; UEFI mappings tolerate WP clear.
            core::arch::asm!(
                "mov cr0, {}",
                in(reg) cr0 & !(1u64 << 16),
                options(nomem, nostack),
            );
            // Some CPUs cache WP in TLB entries — force full flush.
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
        }
    }

    // Enable EFER.NXE on the BSP. Our page-table presets set the NX bit (63) on
    // data pages (USER_RW / KERNEL_RW / intermediate table entries). With NXE=0
    // that bit is RESERVED, so the first access to any such PTE faults with a
    // reserved-bit #PF (error bit 3) — e.g. /bin/init's first stack write. UEFI
    // happens to leave NXE set on some firmware but not all; don't depend on it.
    // The AP trampoline already forces NXE for the same reason — mirror it here.
    // SAFETY: BSP, single-threaded; RMW of IA32_EFER preserves SCE/LME/LMA.
    {
        const IA32_EFER: u32 = 0xC000_0080;
        let (mut lo, hi): (u32, u32);
        core::arch::asm!(
            "rdmsr",
            in("ecx") IA32_EFER,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
        lo |= 1 << 11; // NXE
        core::arch::asm!(
            "wrmsr",
            in("ecx") IA32_EFER,
            in("eax") lo,
            in("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }

    log_info("BOOT", 100, "taking ownership of platform init");

    log_info("BOOT", 101, "phase 1/13: memory");

    // Collect live PT/GDT/IDT/stack pages BEFORE buddy init. The buddy writes a
    // 16-byte FreeNode at every free block base (incl. spare splits); landing
    // that on a live PT rewrites the first two PTEs → #PF/#GP next TLB miss.
    let (mut hw_holes, mut hw_count) = collect_page_table_pages();

    {
        let mut buf = [0u8; 10];
        // SAFETY: SGDT writes 10 bytes to `buf`. We hold the only reference.
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
        // SAFETY: SIDT writes 10 bytes to `buf`. We hold the only reference.
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

    // Bootloader-reported stack bounds preferred. Past RSP-guess missed the
    // bottom and let the buddy plant FreeNodes in live LoaderData pages;
    // OVMF's 0xAF scrub then read like list corruption when nodes were walked.
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
        // SAFETY: read RSP; no memory effect.
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

    // Insertion sort — hw_count < 100 typical.
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

    // Guard MUST drop before next relock — SpinLock recursion deadlocks instantly.
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

    // Per-CPU init must follow GDT and precede anything touching gs:[offset]
    // (scheduler, interrupts, syscalls).
    checkpoint("phase2-lapic-probe");
    {
        // Firmware can relocate LAPIC; trust MSR 0x1B over the spec default.
        let actual_base = apic::probe_lapic_base();

        // SAFETY: BSP, single-threaded; LAPIC MMIO identity-mapped by UEFI.
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

    // GLOBAL_REGISTRY unlocked — phase 2 dropped the guard.
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
    // the TSC → broken delay_us + scheduler timing on real silicon.
    {
        let max_ext: u32;
        // SAFETY: CPUID leaf 0x80000000; no memory effect; rbx preserved.
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
            // SAFETY: CPUID leaf 0x80000007; no memory effect; rbx preserved.
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
    // Publish calibrated frequency via the registered hook.
    let tsc_pub_raw = TSC_FREQ_PUBLISH_HOOK.load(Ordering::Acquire);
    if !tsc_pub_raw.is_null() {
        // SAFETY: registered fn pointer, matching ABI.
        let f: TscFreqPublishHook = unsafe { core::mem::transmute(tsc_pub_raw) };
        unsafe {
            f(tsc_freq);
        }
    }

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

    // VirtIO reads avail->idx on enable; uninitialized garbage there permanently
    // desyncs the driver.
    // SAFETY: dma_phys identity-mapped, DMA_SIZE bytes owned.
    core::ptr::write_bytes(dma_phys as *mut u8, 0u8, DMA_SIZE);
    checkpoint("phase6-done");

    // Identity-mapped: VA = PA = bus addr.
    // SAFETY: page-aligned, identity-mapped, owned.
    let dma_region = unsafe { DmaRegion::new(dma_phys as *mut u8, dma_phys, DMA_SIZE) };

    log_info("BOOT", 107, "phase 7/13: pci");
    checkpoint("phase7-pci-begin");

    enable_all_pci_devices();
    checkpoint("phase7-done");

    log_info("BOOT", 108, "phase 8/13: paging");
    checkpoint("phase8-paging-begin");

    init_kernel_page_table();
    checkpoint("phase8-paging-done");

    // Paging up — remap LAPIC MMIO as UC and bring the BSP LAPIC fully online.
    checkpoint("phase8-lapic-init");
    apic::init_bsp();
    checkpoint("phase8-done");

    // USB input enumerates pre-scheduler so user processes never see a
    // half-built input subsystem.
    log_info("BOOT", 109, "phase 9/13: USB input init");
    checkpoint("phase9-usb-begin");

    // Dump every USB host before the xHCI-only scan — exposes EHCI/UHCI/OHCI
    // controllers we don't (yet) drive.
    // SAFETY: PCI config space is always accessible.
    unsafe {
        dump::dump_usb_controllers();
    }

    {
        // HID ringbuf init (kernel-side; bootloader wires the hook).
        let input_init_raw = INPUT_INIT_HOOK.load(Ordering::Acquire);
        if !input_init_raw.is_null() {
            // SAFETY: registered fn pointer, matching ABI.
            let f: InputInitHook = unsafe { core::mem::transmute(input_init_raw) };
            unsafe {
                f();
            }
        }

        // PCI class 0x0C / subclass 0x03 = Serial Bus / USB; prog_if 0x30 = xHCI (only one we drive).
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

                        // SAFETY: BAR0 masked to MMIO base; tsc_freq != 0 by check above.
                        match unsafe { XhciController::new(base_addr as u64, tsc_freq) } {
                            Ok(mut controller) => {
                                // MSI-X wired to a stub ISR; polling remains
                                // the authoritative event path.
                                let msix_raw = XHCI_MSIX_HOOK.load(Ordering::Acquire);
                                if !msix_raw.is_null() {
                                    // SAFETY: registered fn pointer, matching ABI.
                                    let f: XhciMsixHook = unsafe { core::mem::transmute(msix_raw) };
                                    unsafe {
                                        f(addr, controller.rt_base);
                                    }
                                }
                                let kbd_for_runtime =
                                    match unsafe { enumerate_and_bind_inputs(&mut controller) } {
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
                                        },
                                        Err(e) => {
                                            log_warn("USB", 920, "USB enumeration failed");
                                            let _ = e;
                                            None
                                        },
                                    };
                                // Hand the xHC to the runtime polling module so
                                // the input loop can fetch reports later. Without
                                // this, `controller` drops and the keyboard is
                                // unreachable. No-hook = headless/polling-disabled config.
                                let rt_raw = XHCI_RUNTIME_HOOK.load(Ordering::Acquire);
                                if !rt_raw.is_null() {
                                    // SAFETY: registered fn pointer, matching ABI.
                                    let f: XhciRuntimeHook =
                                        unsafe { core::mem::transmute(rt_raw) };
                                    unsafe {
                                        f(controller, kbd_for_runtime);
                                    }
                                }
                            },
                            Err(e) => {
                                log_warn("USB", 921, "xHCI controller init failed");
                                let _ = e;
                            },
                        }
                    }
                }

                let _ = prog_if;
            }
        }

        if usb_init_count == 0 {
            log_info("USB", 930, "no USB input devices; PS/2 remains primary");
        } else {
            log_ok("USB", 931, "USB input initialization complete");
        }
    }
    checkpoint("phase9-usb-done");

    // End phases 1-9. Bootloader calls `install_hal`, then `morpheus_kernel::init`
    // (phases 10/11/11b), then 10.5 reclaim + phase 12 SMP via HAL methods.

    log_ok(
        "BOOT",
        199,
        "phase 1-9 complete; handing off to kernel late-init",
    );

    let allocator = fallback_allocator();

    Ok(PlatformInit {
        tsc_freq,
        dma_region,
        allocator,
        kernel_stack_top,
        acpi_rsdp_phys: config.acpi_rsdp_phys,
    })
}

/// Enable MEM + bus mastering on every non-bridge function.
///
/// # Safety
/// PCI config space accessible (always on x86_64).
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
/// triggered IOMMU faults + stray DMA from shadow BARs on real silicon.
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
