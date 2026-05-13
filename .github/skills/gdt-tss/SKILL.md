---
name: gdt-tss
description: 'GDT and TSS management for bare-metal x86_64. Use when implementing per-AP GDT init, TSS setup for ring-0 stack (RSP0), GDT segment selectors (KERNEL_CS, USER_CS, USER_DS, TSS_SEL), 64-bit TSS descriptor (16-byte entry), LGDT instruction, init_gdt_for_ap, GdtEntry, Task State Segment RSP0 for interrupt stack, gdt.rs, selector values, DPL, ring transition descriptors, SYSRET compatibility selector layout.'
argument-hint: "GDT/TSS task or selector bug to fix"
---

# GDT / TSS

## When to Use
- Setting up per-AP GDT and TSS in `init_gdt_for_ap`
- Fixing selector value mismatches (SYSRET requires specific ordering)
- Adding a new segment descriptor
- Updating RSP0 in the TSS (e.g., after stack reallocation for a core)
- Debugging `#GP` or `#TS` faults caused by invalid selectors

## Key Files
- `hwinit/src/cpu/gdt.rs` — `GdtEntry`, selector constants, `init_gdt_for_ap`
- `hwinit/src/cpu/per_cpu.rs` — `PERCPU_TSS_PTR` (GS-relative TSS pointer for RSP0 updates)

## Segment Layout

The GDT layout is fixed — SYSRET hardcodes the selector arithmetic. Do not reorder.

| Index | Selector | Ring | Type | Notes |
|-------|----------|------|------|-------|
| 0 | 0x00 | — | Null | Required |
| 1 | 0x08 | 0 | Code 64 | Kernel code (SYSCALL CS) |
| 2 | 0x10 | 0 | Data 64 | Kernel data |
| 3 | 0x18 | 3 | Data 64 | User data (SYSRET base; `\| 3` for RPL) |
| 4 | 0x20 | 3 | Code 64 | User code (SYSRET CS = 0x18 + 16) |
| 5 | 0x28 | — | TSS | 16-byte entry — occupies slots 5 and 6 |

**SYSRET selector constraint**: SYSRET loads SS = STAR[63:48] + 8, CS = STAR[63:48] + 16.
`STAR[63:48]` must be set to 0x18 (user data) so SS = 0x20 and CS = 0x28 — except our user code is at 0x20. Confirm STAR encoding in `syscall_init` matches this layout.

## TSS in Long Mode

The TSS descriptor is 16 bytes (two GDT slots). The TSS itself needs:
- `RSP0` (bytes 4–11): kernel stack for privilege-level 0 interrupts from ring 3
- `IST1`–`IST7` (optional): alternate stacks for NMI, DF, etc.

Update `RSP0` in the TSS whenever the kernel stack changes:
```rust
// via PerCpu.tss_ptr (GS-accessible)
let tss = per_cpu::current().tss_ptr as *mut Tss;
(*tss).rsp0 = new_kernel_rsp;
```

## Per-AP GDT Init

Each AP needs its own GDT and TSS — you cannot share them because:
- TSS stores per-core `RSP0` (different stack per core)
- LGDT loads a base address that must remain valid for the lifetime of the core

Allocation must happen **before** the SIPI fires. The BSP allocates the GDT/TSS in `boot_single_ap` (or equivalent), writes the pointer into the trampoline, and the AP loads it in `init_gdt_for_ap`.

## 64-bit TSS Descriptor Encoding

```
bits 15:0   = limit[15:0]
bits 31:16  = base[15:0]
bits 39:32  = base[23:16]
bits 40     = type bit 0 (must be 1 for available TSS = 0x9)
bits 43:41  = type bits 3:1
bits 44     = 0 (system segment)
bits 46:45  = DPL (0 for kernel TSS)
bits 47     = present = 1
bits 55:48  = limit[19:16] + granularity
bits 63:56  = base[31:24]
[+4 bytes]  = base[63:32]
[+4 bytes]  = reserved (0)
```

`type = 0x9` = 64-bit available TSS.

## Common Failure Modes

| Symptom | Cause |
|---------|-------|
| `#TS` on first interrupt from ring 3 | TSS not loaded (LTR not called), or RSP0 = 0 |
| `#GP` on SYSRET | STAR[63:48] mismatches GDT user data selector |
| AP crashes immediately after LGDT | GDT page is not mapped or was allocated above 4 GB in 32-bit trampoline context |
| TSS base read as wrong address | TSS descriptor base field not encoded correctly (packed split-field) |

## Procedure

1. Read `gdt.rs` in full — confirm current selector values before adding anything.
2. New segment? Add at the end (after TSS) to avoid disturbing the SYSRET-critical slots.
3. Per-AP GDT: allocate static or per-core GDT array; fill with `GdtEntry` constructors.
4. Call LTR with `TSS_SEL` after LGDT on every core.
5. Store TSS pointer in `PerCpu.tss_ptr` for runtime RSP0 updates.
