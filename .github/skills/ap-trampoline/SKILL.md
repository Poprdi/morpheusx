---
name: ap-trampoline
description: 'AP trampoline authoring, debugging, and layout. Use when writing or fixing asm/cpu/ap_trampoline.s, real-mode to protected-mode to long-mode AP entry sequence, GDT/CR3/stack handoff from trampoline data block, TRAMPOLINE_DATA_OFFSET layout, TD_CR3/TD_STACK/TD_ENTRY64/TD_READY fields, trampoline binary included via include_bytes!, AP triple-fault on real hardware, page 0x8000 setup, AP_TRAMPOLINE_PHYS.'
argument-hint: "Trampoline task or symptom (e.g. 'AP triple-faults after SIPI')"
---

# AP Trampoline

## When to Use
- Writing or modifying `asm/cpu/ap_trampoline.s`
- Debugging AP triple-faults after SIPI (especially when QEMU works, real hardware dies)
- Changing the trampoline data block layout (`TRAMPOLINE_DATA_OFFSET`, `TD_*` offsets)
- Updating `build.rs` trampoline assembly step
- AP boot hangs at the `TD_READY` poll

## Key Files
- `hwinit/asm/cpu/ap_trampoline.s` — the trampoline source
- `hwinit/src/cpu/ap_boot.rs` — BSP-side setup (`setup_trampoline`, `boot_single_ap`)
- `hwinit/build.rs` — assembles the trampoline flat binary into `OUT_DIR/ap_trampoline.bin`

## Trampoline Data Block Contract

The data block lives at `AP_TRAMPOLINE_PHYS + 0xF00` (= `0x8F00`).
The BSP writes before firing SIPI; the trampoline reads in real/protected mode.

| Offset | Size | Field | Written by | Read by |
|--------|------|-------|------------|---------|
| +0x00 | 8 | `TD_CR3` | BSP | trampoline (32-bit) |
| +0x08 | 8 | `TD_ENTRY64` | BSP | trampoline (64-bit jmp) |
| +0x10 | 8 | `TD_STACK` | BSP | trampoline (RSP setup) |
| +0x18 | 4 | `TD_CORE_IDX` | BSP | `ap_rust_entry` arg 0 |
| +0x1C | 4 | `TD_LAPIC_ID` | BSP | `ap_rust_entry` arg 1 |
| +0x20 | 10 | `TD_GDT_PTR` | BSP | trampoline LGDT |
| +0x30 | 4 | `TD_READY` | BSP (0), AP (1) | BSP poll in `boot_single_ap` |

If you add fields: keep 8-byte alignment, update both the `.s` and `ap_boot.rs` constants.

## Real-Mode → Protected → Long Mode Sequence

1. **Real mode**: CPU starts at `0x8000:0000`, 16-bit. Load a flat 32-bit GDT (from `TD_GDT_PTR`), enable PE in CR0.
2. **32-bit protected**: Far-jump to flush CS. Load `TD_CR3` into CR0 — **must be ≤ 4 GB** (the check is in `setup_trampoline`). Enable PAE in CR4. Set IA32_EFER.LME. Enable paging (CR0.PG). 
3. **64-bit long mode**: Far-jump with 64-bit code selector. Load `TD_STACK` into RSP. Call `TD_ENTRY64` with `(TD_CORE_IDX, TD_LAPIC_ID)` in `edi`/`esi`.

## Common Failure Modes

| Symptom | Likely Cause |
|---------|-------------|
| Triple-fault immediately after SIPI | CR3 > 4 GB, or GDT not accessible from trampoline |
| AP hangs, never sets `TD_READY` | Stack pointer wrong (stack_top vs stack_base confusion) |
| Works in QEMU, dies on real hardware | Cache not flushed before CPU reads trampoline data; add `WBINVD` or ensure WB mapping |
| SIPI fires, AP starts, crashes in Rust | `TD_ENTRY64` address wrong; `ap_rust_entry` calling convention mismatch |
| Second SIPI causes double-init | Normal — Intel MP spec requires two SIPIs for reliability |

## Procedure

1. Read the current `ap_trampoline.s` and note section labels and data offsets.
2. Cross-check `TRAMPOLINE_DATA_OFFSET` and `TD_*` constants against the `.s` layout.
3. Make changes, then verify `build.rs` still assembles it as a flat binary (no ELF header).
4. Confirm the assembled binary is ≤ `0xF00` bytes (data block starts there).
5. Test with `AP_TRAMPOLINE_BIN.len()` assertion in `setup_trampoline`.
