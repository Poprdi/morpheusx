//! AP bring-up: copy real-mode trampoline to 0x8000, INIT IPI + SIPI x2,
//! poll AP_ONLINE_COUNT. Trampoline binary baked in via build.rs.

// The optional `smp` feature gates the baked-in trampoline; it is supplied by
// the build wiring, not declared in this crate's Cargo.toml.
#![allow(unexpected_cfgs)]

use crate::cpu::apic;
use crate::cpu::gdt;
use crate::cpu::per_cpu::{self, MAX_CPUS};
use crate::memory::{global_registry_mut, AllocateType, MemoryType, PAGE_SIZE};
use crate::serial::{log_error, log_warn};
use core::sync::atomic::{AtomicBool, Ordering};
/// Trampoline physical address. Page-aligned, <1 MiB. 0x8000 is traditional.
const AP_TRAMPOLINE_PHYS: u64 = 0x8000;
const AP_TRAMPOLINE_PAGE: u8 = (AP_TRAMPOLINE_PHYS / 0x1000) as u8;
const AP_STACK_SIZE: u64 = 64 * 1024;

// Bounded brute-force LAPIC fallback; one bad topology must not deadlock boot.
const AP_WAIT_STEP_US: u64 = 50;
const AP_WAIT_TIMEOUT_US: u64 = 20_000;
const AP_BRUTE_SCAN_BUDGET_US: u64 = 3_000_000;
const AP_MADT_SCAN_BUDGET_US: u64 = 1_000_000;

static AP_SCHEDULER_RELEASED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApBootError {
    StackAllocFailed,
    OnlineTimeout,
}

struct ApStack {
    base: u64,
    pages: u64,
    top: u64,
}

// Layout MUST match ap_trampoline.s .data block.
const TRAMPOLINE_DATA_OFFSET: u64 = 0xF00;
const TD_CR3: u64 = TRAMPOLINE_DATA_OFFSET;
const TD_ENTRY64: u64 = TRAMPOLINE_DATA_OFFSET + 0x08;
const TD_STACK: u64 = TRAMPOLINE_DATA_OFFSET + 0x10;
const TD_CORE_IDX: u64 = TRAMPOLINE_DATA_OFFSET + 0x18;
const TD_LAPIC_ID: u64 = TRAMPOLINE_DATA_OFFSET + 0x1C;
const TD_GDT_PTR: u64 = TRAMPOLINE_DATA_OFFSET + 0x20; // limit(2) + base(8)
const TD_READY: u64 = TRAMPOLINE_DATA_OFFSET + 0x30;

/// Flat-binary trampoline from build.rs. Empty => `start_aps` is a no-op.
#[cfg(feature = "smp")]
static AP_TRAMPOLINE_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ap_trampoline.bin"));

#[cfg(not(feature = "smp"))]
static AP_TRAMPOLINE_BIN: &[u8] = &[];

/// Reserve 0x8000 from the registry, copy trampoline, fill handoff data
/// (CR3, GDT, entry). False = page unavailable (firmware reserved).
unsafe fn setup_trampoline() -> bool {
    // Some firmware marks 0x8000 reserved/MMIO; validate before stomping.
    match global_registry_mut().allocate_pages(
        AllocateType::Address(AP_TRAMPOLINE_PHYS),
        MemoryType::LoaderData,
        1,
    ) {
        Ok(_) => {},
        Err(_) => {
            log_error(
                "AP",
                500,
                "trampoline page 0x8000 unavailable in memory map",
            );
            return false;
        },
    }

    let trampoline_len = AP_TRAMPOLINE_BIN.len();
    let dest = AP_TRAMPOLINE_PHYS as *mut u8;
    core::ptr::copy_nonoverlapping(AP_TRAMPOLINE_BIN.as_ptr(), dest, trampoline_len);
    core::ptr::write_bytes(dest.add(trampoline_len), 0, 0x1000 - trampoline_len);

    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    let kernel_cr3 = cr3 & 0x000F_FFFF_FFFF_F000;

    // 32-bit trampoline reads CR3 from [0x8F00] (low 32 bits only).
    // CR3 >4 GiB => truncated load => instant triple-fault.
    if kernel_cr3 > 0xFFFF_FFFF {
        log_error(
            "AP",
            514,
            "kernel CR3 above 4GB; AP trampoline cannot load it in 32-bit mode",
        );
        return false;
    }

    // Snapshot BSP GDT (APs swap to per-core GDT later).
    let mut gdt_buf = [0u8; 10];
    core::arch::asm!("sgdt [{}]", in(reg) gdt_buf.as_mut_ptr(), options(nostack));

    let gdt_ptr_dest = (AP_TRAMPOLINE_PHYS + TD_GDT_PTR) as *mut u8;
    core::ptr::copy_nonoverlapping(gdt_buf.as_ptr(), gdt_ptr_dest, 10);
    core::ptr::write_volatile((AP_TRAMPOLINE_PHYS + TD_CR3) as *mut u64, kernel_cr3);
    core::ptr::write_volatile(
        (AP_TRAMPOLINE_PHYS + TD_ENTRY64) as *mut u64,
        ap_rust_entry as usize as u64,
    );

    true
}

/// INIT+SIPI a single AP and wait for online ack. Frees stack on timeout.
unsafe fn boot_single_ap(core_idx: u32, lapic_id: u32) -> bool {
    let stack = match allocate_ap_stack() {
        Ok(stack) => stack,
        Err(ApBootError::StackAllocFailed) => {
            log_error("AP", 501, "stack allocation failed");
            return false;
        },
        Err(_) => return false,
    };

    write_trampoline_handoff(&stack, core_idx, lapic_id);

    let before = per_cpu::AP_ONLINE_COUNT.load(Ordering::SeqCst);
    send_init_sipi_sequence(lapic_id);

    match wait_ap_online(before) {
        Ok(()) => true,
        Err(ApBootError::OnlineTimeout) => {
            let _ = global_registry_mut().free_pages(stack.base, stack.pages);
            false
        },
        Err(_) => false,
    }
}

unsafe fn allocate_ap_stack() -> Result<ApStack, ApBootError> {
    let pages = AP_STACK_SIZE / PAGE_SIZE;
    let base = global_registry_mut()
        .allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedStack, pages)
        .map_err(|_| ApBootError::StackAllocFailed)?;

    Ok(ApStack {
        base,
        pages,
        top: base + AP_STACK_SIZE,
    })
}

unsafe fn write_trampoline_handoff(stack: &ApStack, core_idx: u32, lapic_id: u32) {
    core::ptr::write_volatile((AP_TRAMPOLINE_PHYS + TD_READY) as *mut u32, 0);
    core::ptr::write_volatile((AP_TRAMPOLINE_PHYS + TD_STACK) as *mut u64, stack.top);
    core::ptr::write_volatile((AP_TRAMPOLINE_PHYS + TD_CORE_IDX) as *mut u32, core_idx);
    core::ptr::write_volatile((AP_TRAMPOLINE_PHYS + TD_LAPIC_ID) as *mut u32, lapic_id);
    core::sync::atomic::fence(Ordering::SeqCst);
}

unsafe fn send_init_sipi_sequence(lapic_id: u32) {
    apic::send_init_assert(lapic_id);
    apic::delay_us(10_000);

    apic::send_sipi(lapic_id, AP_TRAMPOLINE_PAGE);
    apic::delay_us(10_000);

    apic::send_sipi(lapic_id, AP_TRAMPOLINE_PAGE);
    apic::delay_us(10_000);
}

unsafe fn wait_ap_online(before: u32) -> Result<(), ApBootError> {
    let mut waited_us = 0u64;
    while per_cpu::AP_ONLINE_COUNT.load(Ordering::Acquire) <= before {
        apic::delay_us(AP_WAIT_STEP_US);
        waited_us += AP_WAIT_STEP_US;
        if waited_us >= AP_WAIT_TIMEOUT_US {
            return Err(ApBootError::OnlineTimeout);
        }
    }

    Ok(())
}

fn park_until_scheduler_release() {
    while !AP_SCHEDULER_RELEASED.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
}

/// Release parked APs into LAPIC-timer scheduling. APs are brought up early
/// but held IF=0 until BSP finishes driver init.
pub fn release_parked_aps() {
    let _ = AP_SCHEDULER_RELEASED.swap(true, Ordering::Release);
}

/// Preferred AP bring-up path: use MADT-discovered LAPIC IDs.
///
/// # Safety
/// BSP must have completed platform init (GDT, IDT, paging, LAPIC, registry, sched).
pub unsafe fn start_aps_from_list(ap_lapic_ids: &[u32]) {
    if AP_TRAMPOLINE_BIN.is_empty() {
        log_warn("AP", 503, "no trampoline binary; smp disabled");
        return;
    }
    if ap_lapic_ids.is_empty() {
        return;
    }

    if !setup_trampoline() {
        return;
    }

    let mut core_idx: u32 = 1; // 0 = BSP
    let mut budget_used_us: u64 = 0;
    let x2_mode = apic::is_x2apic_mode();
    for &lapic_id in ap_lapic_ids {
        if core_idx as usize >= MAX_CPUS {
            break;
        }

        if !x2_mode && lapic_id > 0xFF {
            // xAPIC destination is 8-bit; this ID is unreachable.
            log_warn("AP", 513, "skipping MADT x2APIC ID in xAPIC mode");
            continue;
        }

        if budget_used_us >= AP_MADT_SCAN_BUDGET_US {
            log_warn(
                "AP",
                512,
                "MADT AP bring-up budget exhausted; continuing with discovered cores",
            );
            break;
        }

        if boot_single_ap(core_idx, lapic_id) {
            core_idx += 1;
        }

        budget_used_us += 10_400 + AP_WAIT_TIMEOUT_US;
    }
}

/// CPUID fallback: brute-force LAPIC IDs 0..255. Ghosts free their stacks.
///
/// # Safety
/// BSP must have completed platform init.
pub unsafe fn start_aps() {
    if AP_TRAMPOLINE_BIN.is_empty() {
        log_warn("AP", 506, "no trampoline binary; smp disabled");
        return;
    }

    let total_cpus = per_cpu::cpu_count();
    if total_cpus <= 1 {
        return;
    }

    let bsp_lapic_id = apic::read_lapic_id();
    log_warn("AP", 508, "starting AP bring-up via brute-force LAPIC scan");

    if !setup_trampoline() {
        return;
    }

    let mut core_idx: u32 = 1; // 0 = BSP
    let mut online: u32 = 0;
    let mut scan_budget_used_us: u64 = 0;
    for lapic_id in 0u32..256 {
        if lapic_id == bsp_lapic_id {
            continue;
        }
        if online >= total_cpus - 1 {
            break;
        }
        if core_idx as usize >= MAX_CPUS {
            break;
        }

        if scan_budget_used_us >= AP_BRUTE_SCAN_BUDGET_US {
            log_warn(
                "AP",
                510,
                "brute-force AP scan budget exhausted; continuing with discovered cores",
            );
            break;
        }

        if boot_single_ap(core_idx, lapic_id) {
            core_idx += 1;
            online += 1;
        }

        // Upper bound: INIT delay + SIPI delays + wait timeout.
        scan_budget_used_us += 10_400 + AP_WAIT_TIMEOUT_US;
    }
}

/// AP entry from trampoline. Long mode, IF=0, kernel CR3, BSP GDT.
/// Loads per-core GDT/IDT, brings up LAPIC, then parks until released.
///
/// # Safety
/// Called only by the AP trampoline in long mode with a valid kernel CR3, the
/// BSP GDT loaded, and IF=0. `core_idx`/`lapic_id` must be the values written
/// into the trampoline handoff block for this AP. Must not be called from Rust.
#[no_mangle]
pub unsafe extern "sysv64" fn ap_rust_entry(core_idx: u32, lapic_id: u32) -> ! {
    let stack_top = core::ptr::read_volatile((AP_TRAMPOLINE_PHYS + TD_STACK) as *const u64);
    gdt::init_gdt_for_ap(stack_top, core_idx);
    crate::cpu::idt::load_idt_for_ap();
    per_cpu::init_ap(core_idx, lapic_id, apic::lapic_base(), stack_top);
    crate::cpu::sse::enable_sse();
    extern "C" {
        fn syscall_init();
    }
    syscall_init();
    apic::init_ap();

    core::ptr::write_volatile((AP_TRAMPOLINE_PHYS + TD_READY) as *mut u32, 1);
    crate::cpu::per_cpu::AP_ONLINE_COUNT.fetch_add(1, Ordering::Release);

    park_until_scheduler_release();
    apic::setup_timer(100);
    core::arch::asm!("sti", options(nostack, nomem));

    loop {
        core::arch::asm!("hlt", options(nostack, nomem));
    }
}
