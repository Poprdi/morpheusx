---
applyTo: ['hwinit/src/cpu/**/*.rs', 'hwinit/asm/cpu/**/*.s']
---

# CPU Code — Real Hardware First

This instruction applies to all changes in `hwinit/src/cpu/` and CPU ASM.

## The Contract

Any change to CPU code **must be validated on real hardware before merging.** QEMU is a liar. It will let code slide that triple-faults on a real Xeon.

Symptoms that indicate you've broken something on real hardware that QEMU hides:

- AP triples-fault after SIPI
- System locks up after N cores come online
- Scheduler crashes when running on AP (works on BSP)
- Per-CPU data corruption (GS-base, stack, PerCpu fields read as garbage)
- Cache coherency bugs: one core writes, another doesn't see it (QEMU has perfect coherency)
- LAPIC base remapping: firmware moved it; code assumes default

## Before You Start

1. Know the real-hardware symptom your change might cause
2. Build a mental model of what can break in what you're touching
3. Read the relevant skill (see the list below)

## While You Code

- Every `unsafe` block needs a `// SAFETY:` comment — non-negotiable
- Every `Atomic*` operation needs deliberate ordering choice — document why
- Every new lock needs a reason (see `kernel-locking` skill)
- No `unwrap()` / `expect()` / `panic!()` outside init paths
- Memory barriers (fence, mfence, SeqCst) need a comment explaining what they pair with

## After You Code — Pre-Merge Gates

1. **Static checks pass**:
   - `cargo fmt --check` clean
   - `cargo clippy -- -D warnings` clean (or justified allow)
   - `cargo check` succeeds

2. **Real hardware validation**:
   - Boot on real hardware (4+ cores minimum)
   - Verify no crashes, hangs, or data corruption
   - Serial log shows clean boot (no ERR codes)
   - System remains stable for ≥30 seconds with all cores online

3. **Code review gates** (use skills):
   - Run through `kernel-unsafe-discipline` skill checklist
   - Run through `kernel-memory-ordering` skill checklist (if touching Atomic or barriers)
   - Run through `kernel-review-checklist` skill checklist

## Relevant Skills

Depending on what you're changing:

- **Any CPU code**: `kernel-coding-style`, `kernel-review-checklist`
- **AP bringup / GDT / MSR**: `/hardened-ap-bringup` prompt (major refactoring needed)
- **AP trampoline, GDT, TSS, per-CPU**: `ap-trampoline`, `gdt-tss`, `per-cpu-layout` skills
- **LAPIC, IPI, timers**: `lapic-ipi` skill
- **ACPI MADT, topology**: `madt-topology` skill
- **CPU features (SSE, AVX, SMEP)**: `cpuid-feature-gate` skill
- **MSR programming**: `msr-setup` skill
- **Locking or synchronization**: `kernel-locking`, `kernel-memory-ordering` skills
- **Unsafe blocks**: `kernel-unsafe-discipline` skill

## Known Issues to Avoid

- **CR3 above 4 GB**: 32-bit trampoline can only load 32-bit CR3
- **AP triple-fault on SIPI**: Usually CR3 wrong, GDT not accessible, or paging not set up
- **Per-CPU offset mismatch**: asm uses hardcoded `gs:[0x20]` but you moved the field; add to `debug_assert_offsets()`
- **x2APIC ID > 0xFF**: xAPIC destination field is 8-bit only; need x2APIC mode for large IDs
- **Cache coherency**: QEMU hides coherency bugs; real hardware exposes them
- **TD_READY not set**: AP either crashed before reaching `ap_rust_entry`, or never incremented `AP_ONLINE_COUNT`

## How to Debug Real-Hardware Failures

1. Add serial logging to every significant step in the affected code path
2. Use unique log codes (see `log_error("AP", CODE, "...")` pattern)
3. Boot on real hardware with verbose logging
4. Identify the last successful log before the crash
5. Audit the next function: unsafe blocks, memory ordering, lock contention
6. Reference the appropriate skill for that subsystem

## What NOT to Do

- Do not assume BSP behavior works for APs (GDT, IDT, MSR, GS-base all per-core)
- Do not assume QEMU coherency works on real hardware
- Do not add a "TODO: fix on real hardware" comment — either fix it or open an issue
- Do not merge with `#[allow(...)]` warnings without a strong reason documented
- Do not allocate DMA memory from interrupt context
- Do not hold a spinlock across an allocation or I/O operation

## Questions?

If you're uncertain about whether a change is safe for real hardware, invoke the relevant skill or ask an agent before coding.
