---
name: per-cpu-layout
description: 'Per-CPU struct and GS-base ABI. Use when adding or reordering fields in PerCpu, updating PERCPU_* offset constants, fixing gs:[offset] mismatches in context_switch.s or syscall.s, SWAPGS discipline, IA32_GS_BASE / IA32_KERNEL_GS_BASE MSR setup, init_ap, GS-base initialization for APs, per_cpu.rs ABI contract, debug_assert_offsets, false sharing on PerCpu, cache-line alignment.'
argument-hint: "PerCpu field change or GS-base bug to fix"
---

# Per-CPU Layout

## When to Use
- Adding, removing, or reordering fields in `PerCpu` in `per_cpu.rs`
- Updating `PERCPU_*` offset constants after struct changes
- Fixing crashes caused by stale `gs:[offset]` reads in `context_switch.s` or `syscall.s`
- Debugging SWAPGS ordering bugs at ring transitions
- Setting up GS-base for a new AP in `init_ap`

## Key Files
- `hwinit/src/cpu/per_cpu.rs` — `PerCpu` struct, `PERCPU_*` offsets, `init_ap`, `AP_ONLINE_COUNT`
- Assembly that uses `gs:[offset]` — search with: `grep -r "gs:\[" hwinit/asm/`

## ABI Contract

The first `0x48` bytes of `PerCpu` are the hot-path ABI. **Assembly reads these by numeric offset — not by name.**

| Offset | Field | Size | Used by |
|--------|-------|------|---------|
| 0x00 | `self_ptr` | 8 | sanity |
| 0x08 | `cpu_id` | 4 | context switch |
| 0x0C | `current_pid` | 4 | context switch |
| 0x10 | `next_cr3` | 8 | context switch |
| 0x18 | `current_fpu_ptr` | 8 | FPU save/restore |
| 0x20 | `kernel_syscall_rsp` | 8 | SYSCALL entry |
| 0x28 | `user_rsp_scratch` | 8 | SYSCALL entry |
| 0x30 | `tss_ptr` | 8 | RSP0 update |
| 0x38 | `lapic_base` | 8 | timer ISR |
| 0x40 | `tick_count` | 8 | scheduler |

## Adding a Field

1. Add the field **after** offset `0x48` unless it belongs in the hot path.
2. If it must be in the hot path: reorder the struct, update ALL `PERCPU_*` constants.
3. Run `debug_assert_offsets()` — it panics at boot if any offset is wrong.
4. Grep assembly: `grep -rn "gs:\[0x" hwinit/asm/` and `grep -rn "PERCPU_" hwinit/asm/`.
5. Update every assembly constant that shifted.

## SWAPGS Discipline

- **SYSCALL entry (ring 3 → 0)**: `SWAPGS` immediately, then `mov rsp, gs:[PERCPU_KERNEL_RSP]`.
- **SYSCALL return (ring 0 → 3)**: `SWAPGS` immediately before `SYSRETQ`.
- **Interrupt entry from ring 3**: check CPL in saved CS; if ring 3, `SWAPGS`.
- **Interrupt entry from ring 0**: **do NOT** `SWAPGS` — GS already points to PerCpu.
- **NMI**: NMI can interrupt at any CPL including mid-SWAPGS. Use `swapgs_unsafe_fixup` or the standard NMI double-frame pattern.

## GS-Base MSR Setup

```
IA32_GS_BASE       (0xC000_0101): kernel GS — points to PerCpu in kernel mode
IA32_KERNEL_GS_BASE (0xC000_0102): user GS  — SWAPGS exchanges these two
```

On `init_ap`:
1. Write `PerCpu` address to `IA32_GS_BASE`.
2. Write `0` (or user-mode value) to `IA32_KERNEL_GS_BASE`.
3. Set `self_ptr = &percpu as u64` so the self-pointer is valid.

## `#[repr(C, align(64))]`

`PerCpu` is cache-line aligned. Do not remove this — two cores sharing a cache line on `PerCpu` fields causes false sharing and destroys scheduler perf on real hardware. If `PerCpu` grows beyond 64 bytes, bump alignment to `128`.

## Procedure

1. Read `per_cpu.rs` fully before touching the struct.
2. Make the struct change.
3. Recompute all affected `PERCPU_*` constants by hand (offset of each field in `repr(C)` layout).
4. Add/update entries in `debug_assert_offsets`.
5. Search for assembly that hardcodes those offsets and patch them.
6. Boot — `debug_assert_offsets` panics loudly at BSP init if anything is wrong.
