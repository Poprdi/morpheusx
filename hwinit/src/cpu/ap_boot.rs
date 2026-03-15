//! AP (Application Processor) boot orchestration.
//!
//! The BSP calls `start_aps()` or `start_aps_from_list()` after its own
//! init is complete.  For each AP we:
//!   1.  Copy the real-mode trampoline to page 0x8000 (validated first)
//!   2.  Fill the trampoline data area (CR3, GDT, stack, entry point)
//!   3.  Send INIT IPI → wait 10ms → SIPI × 2
//!   4.  Wait for the AP to signal readiness via `AP_ONLINE_COUNT`
//!
//! The trampoline code lives in `asm/cpu/ap_trampoline.s`, assembled as
//! flat binary by build.rs and included here via `include_bytes!`.

use crate::cpu::apic;
use crate::cpu::gdt;
use crate::cpu::per_cpu::{self, MAX_CPUS};
use crate::memory::{global_registry_mut, AllocateType, MemoryType, PAGE_SIZE};
use crate::serial::{log_error, log_info, log_ok, log_warn};
use core::sync::atomic::Ordering;
/// Physical address where the AP trampoline is copied.
/// Must be page-aligned, below 1 MiB, and not in use by anything else.
/// 0x8000 is the traditional choice (page 8).
const AP_TRAMPOLINE_PHYS: u64 = 0x8000;

/// SIPI start page = AP_TRAMPOLINE_PHYS / 0x1000
const AP_TRAMPOLINE_PAGE: u8 = (AP_TRAMPOLINE_PHYS / 0x1000) as u8;

/// AP kernel stack size (64 KiB per core).
const AP_STACK_SIZE: u64 = 64 * 1024;

// Brute-force LAPIC probing is a fallback path. Keep it bounded so one bad
// topology guess doesn't make boot look dead on real hardware.
const AP_WAIT_STEP_US: u64 = 50;
const AP_WAIT_TIMEOUT_US: u64 = 20_000;
const AP_BRUTE_SCAN_BUDGET_US: u64 = 3_000_000;
const AP_MADT_SCAN_BUDGET_US: u64 = 1_000_000;

// These must match the .data section layout at the end of ap_trampoline.s.lapic_id
// The trampoline data block starts at AP_TRAMPOLINE_PHYS + TRAMPOLINE_DATA_OFFSET.
const TRAMPOLINE_DATA_OFFSET: u64 = 0xF00; // within the 4K page
const TD_CR3: u64 = TRAMPOLINE_DATA_OFFSET + 0x00;
const TD_ENTRY64: u64 = TRAMPOLINE_DATA_OFFSET + 0x08;
const TD_STACK: u64 = TRAMPOLINE_DATA_OFFSET + 0x10;
const TD_CORE_IDX: u64 = TRAMPOLINE_DATA_OFFSET + 0x18;
const TD_LAPIC_ID: u64 = TRAMPOLINE_DATA_OFFSET + 0x1C;
const TD_GDT_PTR: u64 = TRAMPOLINE_DATA_OFFSET + 0x20; // 10 bytes: limit(2) + base(8)
const TD_READY: u64 = TRAMPOLINE_DATA_OFFSET + 0x30;

/// The flat-binary AP trampoline assembled by build.rs.
/// May be empty if the trampoline ASM file doesn't exist yet — in that case
/// `start_aps` is a no-op.
#[cfg(feature = "smp")]
static AP_TRAMPOLINE_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ap_trampoline.bin"));

#[cfg(not(feature = "smp"))]
static AP_TRAMPOLINE_BIN: &[u8] = &[];

// ── Trampoline setup ─────────────────────────────────────────────────────

/// Validate and prepare the trampoline page at 0x8000.
///
/// Reserves the physical page from the buddy allocator (validates it's
/// actually available), copies the trampoline binary, fills shared data
/// (CR3, GDT, entry point).
///
/// Returns false if the trampoline page is unavailable (reserved by firmware).
unsafe fn setup_trampoline() -> bool {
    // validate the trampoline page is usable before stomping it.
    // on exotic firmware 0x8000 might be reserved/MMIO.
    match global_registry_mut().allocate_pages(
        AllocateType::Address(AP_TRAMPOLINE_PHYS),
        MemoryType::LoaderData,
        1,
    ) {
        Ok(_) => {}
        Err(_) => {
            log_error("AP", 500, "trampoline page 0x8000 unavailable in memory map");
            return false;
        }
    }

    // copy trampoline code to low memory
    let trampoline_len = AP_TRAMPOLINE_BIN.len().min(0xF00);
    let dest = AP_TRAMPOLINE_PHYS as *mut u8;
    core::ptr::copy_nonoverlapping(AP_TRAMPOLINE_BIN.as_ptr(), dest, trampoline_len);
    // zero the data area
    core::ptr::write_bytes(dest.add(trampoline_len), 0, 0x1000 - trampoline_len);

    // read current CR3 for the trampoline to use
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    let kernel_cr3 = cr3 & 0x000F_FFFF_FFFF_F000;

    // The 32-bit protected mode trampoline loads CR3 via `mov eax, [0x8F00]`
    // which only reads the low 32 bits.  If the kernel page tables are above
    // 4 GB, the AP would load a truncated CR3 and triple-fault immediately.
    if kernel_cr3 > 0xFFFF_FFFF {
        log_error("AP", 514, "kernel CR3 above 4GB; AP trampoline cannot load it in 32-bit mode");
        return false;
    }

    // read current GDT for APs to load (temporary, until per-core GDT)
    let mut gdt_buf = [0u8; 10];
    core::arch::asm!("sgdt [{}]", in(reg) gdt_buf.as_mut_ptr(), options(nostack));

    // fill shared trampoline data
    let gdt_ptr_dest = (AP_TRAMPOLINE_PHYS + TD_GDT_PTR) as *mut u8;
    core::ptr::copy_nonoverlapping(gdt_buf.as_ptr(), gdt_ptr_dest, 10);
    *((AP_TRAMPOLINE_PHYS + TD_CR3) as *mut u64) = kernel_cr3;
    *((AP_TRAMPOLINE_PHYS + TD_ENTRY64) as *mut u64) = ap_rust_entry as u64;

    true
}

/// Boot a single AP via INIT+SIPI and wait for it to come online.
///
/// Returns true if the AP responded within the timeout.
/// On failure, frees the allocated stack — no leak.
unsafe fn boot_single_ap(core_idx: u32, lapic_id: u32) -> bool {
    // allocate a kernel stack for this AP
    let stack_pages = AP_STACK_SIZE / PAGE_SIZE;
    let stack_base = match global_registry_mut().allocate_pages(
        AllocateType::AnyPages,
        MemoryType::AllocatedStack,
        stack_pages,
    ) {
        Ok(base) => base,
        Err(_) => {
            log_error("AP", 501, "stack allocation failed");
            return false;
        }
    };
    let stack_top = stack_base + AP_STACK_SIZE;

    // fill per-AP trampoline data
    *((AP_TRAMPOLINE_PHYS + TD_STACK) as *mut u64) = stack_top;
    *((AP_TRAMPOLINE_PHYS + TD_CORE_IDX) as *mut u32) = core_idx;
    *((AP_TRAMPOLINE_PHYS + TD_LAPIC_ID) as *mut u32) = lapic_id;
    core::sync::atomic::fence(Ordering::SeqCst);
    *((AP_TRAMPOLINE_PHYS + TD_READY) as *mut u32) = 0;

    let before = per_cpu::AP_ONLINE_COUNT.load(Ordering::SeqCst);

    // INIT IPI
    apic::send_init_ipi(lapic_id);
    apic::delay_us(10_000); // 10ms

    // SIPI #1
    apic::send_sipi(lapic_id, AP_TRAMPOLINE_PAGE);
    apic::delay_us(10_000); // 10ms

    // SIPI #2 (per Intel spec, send twice for reliability)
    apic::send_sipi(lapic_id, AP_TRAMPOLINE_PAGE);
    apic::delay_us(10_000); // 10ms

    // wait for AP to come online (bounded; fallback path must stay snappy)
    let mut waited_us = 0u64;
    while per_cpu::AP_ONLINE_COUNT.load(Ordering::SeqCst) <= before {
        apic::delay_us(AP_WAIT_STEP_US);
        waited_us += AP_WAIT_STEP_US;
        if waited_us >= AP_WAIT_TIMEOUT_US {
            break;
        }
    }

    if per_cpu::AP_ONLINE_COUNT.load(Ordering::SeqCst) > before {
        true
    } else {
        // don't leak 64KB per ghost core
        let _ = global_registry_mut().free_pages(stack_base, stack_pages);
        false
    }
}

// ── Public API ───────────────────────────────────────────────────────────

/// Start APs from an explicit list of LAPIC IDs (from ACPI MADT).
///
/// This is the preferred path — no brute-force, no ghost timeouts,
/// no wasted stacks.
///
/// # Safety
/// BSP must have completed full platform init (GDT, IDT, paging, LAPIC,
/// memory registry, scheduler).
pub unsafe fn start_aps_from_list(ap_lapic_ids: &[u32]) {
    if AP_TRAMPOLINE_BIN.is_empty() {
        log_warn("AP", 503, "no trampoline binary; smp disabled");
        return;
    }
    if ap_lapic_ids.is_empty() {
        return;
    }

    log_info("AP", 504, "starting AP bring-up from MADT list");

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
            // xAPIC destination field is 8-bit. without x2APIC mode this target is unreachable.
            log_warn("AP", 513, "skipping MADT x2APIC ID in xAPIC mode");
            continue;
        }

        if budget_used_us >= AP_MADT_SCAN_BUDGET_US {
            log_warn("AP", 512, "MADT AP bring-up budget exhausted; continuing with discovered cores");
            break;
        }

        if boot_single_ap(core_idx, lapic_id) {
            core_idx += 1;
        }

        // same per-attempt upper bound used by brute-force path.
        budget_used_us += 10_400 + AP_WAIT_TIMEOUT_US;
    }

    log_ok("AP", 505, "MADT AP bring-up pass complete");
}

/// Start APs by brute-force LAPIC ID enumeration (CPUID fallback).
///
/// Iterates LAPIC IDs 0..255, skipping the BSP.  Ghost IDs that don't
/// respond are skipped without wasting a core slot or leaking stack memory.
///
/// # Safety
/// BSP must have completed full platform init.
pub unsafe fn start_aps() {
    if AP_TRAMPOLINE_BIN.is_empty() {
        log_warn("AP", 506, "no trampoline binary; smp disabled");
        return;
    }

    let total_cpus = per_cpu::cpu_count();
    if total_cpus <= 1 {
        log_info("AP", 507, "single-core detected; no AP startup needed");
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
        // found enough cores already
        if online >= total_cpus - 1 {
            break;
        }
        if core_idx as usize >= MAX_CPUS {
            break;
        }

        if scan_budget_used_us >= AP_BRUTE_SCAN_BUDGET_US {
            log_warn("AP", 510, "brute-force AP scan budget exhausted; continuing with discovered cores");
            break;
        }

        if (lapic_id & 0x1F) == 0 {
            log_info("AP", 511, "AP scan progress checkpoint");
        }

        if boot_single_ap(core_idx, lapic_id) {
            // only consume a core slot on success
            core_idx += 1;
            online += 1;
        }

        // upper bound for one probe: INIT delay + SIPI delays + wait timeout
        scan_budget_used_us += 10_400 + AP_WAIT_TIMEOUT_US;
    }

    log_ok("AP", 509, "brute-force AP bring-up pass complete");
}

// ── AP Rust entry point ──────────────────────────────────────────────────

/// Called by the AP trampoline after transitioning to 64-bit long mode.
///
/// At entry:
/// - RSP = per-AP kernel stack (allocated by BSP)
/// - interrupts disabled
/// - CR3 = kernel page tables
/// - GDT = BSP's GDT (temporary — we'll load per-core GDT)
///
/// The AP must:
/// 1. Load its own GDT + TSS
/// 2. Load the shared IDT
/// 3. Initialize per-CPU data (GS-base)
/// 4. Enable LAPIC + timer
/// 5. Enter the idle loop (scheduler will assign work)
#[no_mangle]
pub unsafe extern "sysv64" fn ap_rust_entry(core_idx: u32, lapic_id: u32) -> ! {
    // 1. Set up per-core GDT + TSS
    // Read the exact stack_top from the trampoline data area.
    // The BSP wrote the precise value to TD_STACK; rounding RSP up
    // can overshoot if RSP is near a page boundary.
    let stack_top = *((AP_TRAMPOLINE_PHYS + TD_STACK) as *const u64);
    gdt::init_gdt_for_ap(stack_top, core_idx);

    // 2. Load the shared IDT
    crate::cpu::idt::load_idt_for_ap();

    // 3. Initialize per-CPU data — use probed base, not the default constant
    per_cpu::init_ap(core_idx, lapic_id, apic::lapic_base());

    // 4. Enable SSE on this core
    crate::cpu::sse::enable_sse();

    // 5. Set up SYSCALL MSRs for this core
    extern "C" {
        fn syscall_init();
    }
    syscall_init();

    // 6. Initialize LAPIC + timer on this core
    apic::init_ap();
    apic::setup_timer(100); // 100 Hz, same as BSP

    // 7. Signal we're online (done in init_ap via per_cpu::init_ap)

    // 8. Enable interrupts and enter AP idle loop
    core::arch::asm!("sti", options(nostack, nomem));

    // AP idle loop — the LAPIC timer will fire and call scheduler_tick,
    // which will pick a process for this core to run.
    loop {
        core::arch::asm!("hlt", options(nostack, nomem));
    }
}
