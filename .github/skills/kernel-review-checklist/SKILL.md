---
name: kernel-review-checklist
description: 'Pre-merge code review checklist for kernel patches. Use before merging or submitting any kernel change. Covers build cleanliness, safety, locking races, IRQ context safety, error path resource cleanup, real-hardware vs QEMU validation, documentation, ABI stability, no panics in runtime paths, atomic ordering pairing, dead-code removal, test coverage. Synthesizes Linux Documentation/process/submit-checklist.rst plus Rust kernel patterns.'
argument-hint: "patch or PR to review"
---

# Kernel Review Checklist

Run this before claiming a patch is "done." Each item is a real failure mode that has bitten kernel developers.

## When to Use
- Before opening a PR
- Before merging your own commit
- When reviewing someone else's kernel-side change
- When a patch touched anything in `hwinit/`, `core/`, or kernel-side `bootloader/`

## Build & Static Analysis

- [ ] `cargo build --release` clean — no warnings
- [ ] `cargo clippy -- -D warnings` clean (or each remaining warning has a `// allow(...)` with reason)
- [ ] `cargo fmt --check` clean
- [ ] No unused imports, dead code, or `#[allow(dead_code)]` without an issue link
- [ ] Conditional compilation flags compile in all combinations (e.g., `--features smp` and `--no-default-features`)

## Safety (the unsafe gauntlet)

- [ ] Every `unsafe` block has a `// SAFETY:` comment explaining why UB cannot occur
- [ ] Every `unsafe fn` has a `# Safety` doc section stating the caller's contract
- [ ] No `transmute` where a narrower op (`as`, `from_bits`) works
- [ ] All MMIO accesses use `read_volatile` / `write_volatile`
- [ ] All `#[repr(C, packed)]` field accesses use `&raw const` + `read_unaligned` (no plain `&field`)
- [ ] No raw pointer dereferences without proving non-null + aligned + initialized + non-aliasing

## Error Handling

- [ ] No new `unwrap()` / `expect()` / `panic!()` outside init paths
- [ ] Every fallible operation returns `Result`, not `Option` (Option is for "not found", not "failed")
- [ ] Resource allocation failures are handled — no `Box::new` without OOM consideration
- [ ] On error: every allocated resource is freed (no leaked pages, stacks, IRQ vectors)
- [ ] Error log codes don't collide with existing ones in the same subsystem (grep first)
- [ ] No `format!`/`String` allocation in error paths

## Locking & Concurrency

- [ ] Every shared mutable static is `Atomic*`, behind a lock, or `unsafe` with documented synchronization
- [ ] Every lock acquisition documents what data it protects
- [ ] If two locks are ever held together: lock order is documented and consistent across all sites
- [ ] No sleeping calls (mutex, alloc, blocking I/O) while holding a spinlock
- [ ] If the same lock is taken from IRQ context: holders disable IRQs (`irqsave`) on the holding CPU
- [ ] Every memory barrier / fence has a comment explaining what it pairs with
- [ ] Atomic ordering choices are deliberate: `Relaxed` only for independent counters, `Acquire`/`Release` for publish/subscribe, `SeqCst` only when justified

## IRQ / Context Safety

- [ ] No allocation in IRQ context (no `Vec::push`, no `Box::new`)
- [ ] No printk/log with `format!` in IRQ context (bounded `&'static str` + integer code only)
- [ ] No taking sleeping locks (Mutex) in IRQ context
- [ ] EOI is sent before returning from interrupt handler
- [ ] If touching `PerCpu` from IRQ: GS-base is valid (post-init only)

## ABI Stability

- [ ] If you reordered any field in `PerCpu`: every `PERCPU_*` constant is updated AND every `gs:[offset]` in `hwinit/asm/` is updated
- [ ] If you changed any GDT selector: `gdt::*` constants are updated AND `STAR` MSR computation in `syscall_init` is updated
- [ ] If you changed the AP trampoline data layout: `TD_*` constants in `ap_boot.rs` match the `.s` file
- [ ] `debug_assert_offsets()` includes the new fields and runs on boot

## Multi-Core Correctness

- [ ] Code that runs on APs has been audited for assumptions only true on the BSP
- [ ] No "first core to reach this point wins" without an explicit atomic exchange
- [ ] AP_ONLINE_COUNT is incremented exactly once per AP, after init is complete
- [ ] Per-CPU data is `#[repr(C, align(64))]` to prevent false sharing
- [ ] If a counter is per-CPU: there's no global aggregation that races

## Documentation

- [ ] Every new public item has a `///` doc comment
- [ ] New module has a `//!` module-level doc explaining purpose and usage
- [ ] Hardware-spec citations (Intel SDM, ACPI spec, PCI spec) included when relevant
- [ ] If the change has subtle ordering: the ordering is documented in a comment
- [ ] If the change touches user-visible behavior: relevant docs in `docs/` updated

## Testing

- [ ] Built and booted on QEMU
- [ ] **Built and booted on at least one piece of real hardware** (the only truth)
- [ ] If touching SMP path: tested with `-smp 4` minimum on QEMU
- [ ] If touching error paths: tested by injecting failures (e.g., make `allocate_pages` return Err)
- [ ] Unit tests for any pure-data logic (frame parsing, ID encoding, etc.)
- [ ] No new flaky tests introduced

## Real-Hardware Litmus Tests

QEMU is lenient. Real hardware is not. Specifically:

- Cache coherency: QEMU has perfect coherency; real hardware needs WBINVD or proper write-back mapping
- IPI delivery: QEMU is fast; real hardware can drop or delay IPIs
- LAPIC base: QEMU keeps it at default; firmware can remap on real hardware
- Memory map: QEMU gives you contiguous low memory; real BIOS reserves random regions
- Topology: QEMU reports what you tell `-smp`; real CPUID can lie (especially in VMs)

If your patch touches any of these areas: **boot on real hardware before merging.**

## Final Sanity

- [ ] If this commit was reverted, would the system still boot? (i.e., not load-bearing on something else uncommitted)
- [ ] If a junior developer modified this code 6 months from now without context, would the failure modes be obvious?
- [ ] Have you described WHY the change is needed in the commit message, not just WHAT?

## References
- `Documentation/process/submit-checklist.rst`
- `Documentation/process/submitting-patches.rst`
- `Documentation/process/coding-style.rst`
- `Documentation/rust/coding-guidelines.rst`
