---
description: 'Harden and modularize AP bringup and CPU init sequence. Use when: AP bringup succeeds on QEMU but fails or corrupts state on real hardware, need to audit boot ordering, refactor AP trampoline or per-CPU initialization, fix "works until scheduler starts" instability. Establishes diagnosis procedures, modularization boundaries, and validates real-hardware correctness.'
argument-hint: "specific symptom (e.g. 'AP triple-faults at syscall_init', 'scheduler crashes after AP4 comes online')"
---

# Harden and Modularize AP Bringup

## Problem Statement

The AP bringup sequence **works on QEMU but fails/corrupts state on real hardware**. Symptoms include:

- AP triple-faults after SIPI
- AP comes online but scheduler crashes
- System locks up after N cores are online
- Per-CPU data corruption (GS-base not set, offsets wrong)
- Boot hangs instead of timing out

The existing code in `hwinit/src/cpu/ap_boot.rs` has:

1. **Coupling**: trampoline setup, data block fill, and boot sequencing all live in one function
2. **Missing validation**: no checks that per-CPU state is coherent after AP init
3. **Ordering assumptions**: code assumes BSP init paths happen before APs, but doesn't enforce it
4. **Race windows**: AP_ONLINE_COUNT increment timing is ambiguous with scheduler activation

## Goals

1. **Diagnose real-hardware failure mode** — understand what breaks where
2. **Modularize**: separate concerns (trampoline, GDT/TSS, LAPIC, MSR, PerCpu init)
3. **Enforce ordering**: explicit state machine for BSP → AP handoff
4. **Harden**: add validation, assertions, bounded errors
5. **Real-hardware-first**: boot and validate on actual hardware before merging

## Scope

**In scope**:
- `hwinit/src/cpu/ap_boot.rs` — orchestration
- `hwinit/src/cpu/gdt.rs` — per-AP GDT/TSS setup
- `hwinit/src/cpu/per_cpu.rs` — PerCpu init, AP_ONLINE_COUNT semantics
- `hwinit/asm/cpu/ap_trampoline.s` — real-mode→LM transition
- `hwinit/build.rs` — trampoline assembly, binary validation
- BSP init sequence (kernel entry → scheduler)

**Out of scope**:
- Scheduler modifications
- Interrupt delivery optimization
- ACPI topology discovery (use what `start_aps_from_list` gets)

## Diagnosis Procedure

Start here. Don't code yet.

### Step 1: Reproduce on Real Hardware

1. Boot MorpheusX with 4+ CPU cores on real hardware (ThinkPad, Xeon, whatever is available)
2. Note the exact failure point:
   - Does serial log show APs coming online?
   - Which core # first fails?
   - Does the crash happen at a fixed point or random?
3. Add detailed logging to `ap_rust_entry`:
   ```rust
   log_ok("AP", 520, "ap_rust_entry: entering");
   log_ok("AP", 521, "gdt_init done");
   log_ok("AP", 522, "idt_load done");
   // ... one log per major step
   ```
4. Identify the exact line/function where the AP dies.

### Step 2: Cross-Reference Against Symptoms

Use the skills:

- **kernel-unsafe-discipline**: Are all `unsafe` blocks justified? Check if AP copies from trampoline safely.
- **kernel-memory-ordering**: Is AP_ONLINE_COUNT increment synchronized correctly? Is TD_STACK write visible to AP?
- **per-cpu-layout**: Are PERCPU_* offsets correct? Compare `PerCpu` struct layout against `gs:[offset]` in asm.
- **gdt-tss**: Is per-AP GDT/TSS allocated before SIPI? Is RSP0 in TSS set correctly?
- **kernel-locking**: Is there a race between AP coming online and BSP starting scheduler?

### Step 3: Root Cause Categories

Map symptom to likely cause:

| Symptom | Likely Cause | Diagnosis |
|---------|--------------|-----------|
| Triple-fault after SIPI | CR3 wrong, GDT not accessible, paging broken | Add logging before SIPI; check `setup_trampoline` CR3 path |
| AP hangs at TD_READY poll | Stack ptr wrong, AP crashes silently | Verify stack allocation in `boot_single_ap` |
| Crash in `syscall_init` | STAR selector mismatch with GDT, LSTAR points to BSP code | Audit GDT slot ordering vs STAR constants |
| Scheduler hangs after AP3 | Per-CPU offset mismatch, GS-base wrong | Run `debug_assert_offsets()` and check GS-base MSR write |
| Data corruption | PerCpu read/write races, no synchronization | Check AP_ONLINE_COUNT timing — is it set before scheduler reads? |

## Modularization Proposal

Split `ap_boot.rs` into clearer stages:

### Stage 0: BSP Validation (new function)
```rust
unsafe fn validate_bsp_preconditions() -> Result<(), ApBootError> {
    // Assert: GDT loaded, IDT loaded, paging on, LAPIC online
    // Assert: memory registry ready
    // Assert: scheduler NOT started yet
    // Return error if any contract violated
}
```

### Stage 1: Trampoline Prep (refactor from `setup_trampoline`)
```rust
unsafe fn prepare_trampoline_once() -> Result<TrampolineHandle, ApBootError> {
    // Reserve 0x8000
    // Copy trampoline binary
    // Zero data block
    // Return handle so we don't repeat this for every AP
    // (problem now: every AP boot re-does this work)
}
```

### Stage 2: Per-AP Resource Allocation (new function)
```rust
struct ApResources {
    stack_base: u64,
    gdt: &'static mut [GdtEntry; GDT_SIZE],
    tss: &'static mut Tss,
}

unsafe fn allocate_ap_resources(core_idx: u32) -> Result<ApResources, ApBootError> {
    // Allocate stack (no SIPI yet)
    // Allocate per-AP GDT
    // Allocate per-AP TSS
    // Fill GDT+TSS
    // Return — AP cannot run yet
}
```

### Stage 3: Pre-SIPI Data Handoff (new function)
```rust
unsafe fn write_trampoline_handoff(resources: &ApResources, lapic_id: u32, core_idx: u32) -> Result<(), ApBootError> {
    // Fill TD_STACK, TD_GDT_PTR, TD_ENTRY64, TD_CORE_IDX, TD_LAPIC_ID
    // Fence (Acquire): ensure all writes reach memory
    // Return
}
```

### Stage 4: INIT/SIPI Sequence (extract to new function)
```rust
unsafe fn send_init_sipi_sequence(lapic_id: u32) -> Result<(), ApBootError> {
    // INIT assert, wait, SIPI 1, wait, SIPI 2, wait
    // Bounded timeouts
    // Return
}
```

### Stage 5: AP Readiness Poll (extract to new function)
```rust
unsafe fn wait_ap_online(core_idx: u32, timeout_us: u64) -> Result<(), ApBootError> {
    // Poll AP_ONLINE_COUNT with timeout
    // Return early if timeout
    // Return error code (not just bool) so caller can log which AP failed
}
```

Each stage is now reviewable, testable, and has a single responsibility.

## Validation Checklist

Before merging any changes:

- [ ] **Real hardware**: boots with all cores online on real hardware
- [ ] **Diagnostic logs**: every major step in AP init is logged with unique code
- [ ] **Modularization**: each function ≤ 50 lines, one purpose per function
- [ ] **Error handling**: every fallible operation returns `Result`, no `unwrap`
- [ ] **Memory safety**: run through `kernel-unsafe-discipline` skill checklist
- [ ] **Ordering**: all `Atomic*` operations audited for `Acquire`/`Release` pairing (see `kernel-memory-ordering`)
- [ ] **ABI**: `debug_assert_offsets` passes, every PerCpu change updates PERCPU_* constants
- [ ] **Locking**: no new spinlock deadlock vectors (check against `kernel-locking` skill)
- [ ] **Code style**: pass `cargo fmt`, `cargo clippy -D warnings`, no dead code

## Success Criteria

- [ ] Boot on real 4-core or 8-core system without hang / corruption
- [ ] Scheduler runs on all cores
- [ ] No spurious crashes after APs are online
- [ ] Serial log is clean (no error codes during normal boot)
- [ ] All cores remain online for ≥ 10 seconds (stress-test stability)

## Recommended Reading

Before starting code:

1. **ap_boot.rs**: read the entire file top-to-bottom
2. **ap-trampoline skill**: understand CR3, stack, GDT handoff
3. **per-cpu-layout skill**: verify your understanding of PerCpu offsets
4. **kernel-memory-ordering skill**: reason through AP_ONLINE_COUNT synchronization
5. **kernel-review-checklist skill**: use as final validation gate

## Procedure

1. Use Step 1 (Reproduce) to nail down the real-hardware failure
2. Use Step 2 (Cross-Reference) to pick the most likely root cause
3. Design modularization per the proposal above (don't code; draw boxes)
4. Implement Stage 0 validation (minimal, just asserts)
5. Refactor into Stages 1–5 without changing behavior (structural only)
6. Add logging per Step 1 diagnostics
7. Test on real hardware; iterate until stable
8. Run full validation checklist
9. Ensure all commits are self-contained and reviewable
