---
name: lapic-ipi
description: 'LAPIC and x2APIC driver work. Use when implementing or debugging INIT assert/deassert IPI, SIPI construction, x2APIC mode detection, xAPIC vs x2APIC destination field width, LAPIC base MSR probing, LAPIC timer calibration via PIT, EOI, spurious vector, send_init_assert, send_sipi, ICR register writes, apic.rs, LAPIC_ICR_LO, LAPIC_ICR_HI, IA32_APIC_BASE MSR, X2APIC_ENABLED, is_x2apic_mode.'
argument-hint: "LAPIC/IPI task or bug to fix"
---

# LAPIC / IPI

## When to Use
- Writing or debugging `send_init_assert`, `send_sipi`, `init_ap`, `setup_timer` in `apic.rs`
- Switching between xAPIC and x2APIC mode
- LAPIC base probing from IA32_APIC_BASE MSR
- Timer calibration (PIT-based init count)
- IPI delivery failures on real hardware (not QEMU)

## Key Files
- `hwinit/src/cpu/apic.rs` — full LAPIC driver
- `hwinit/src/cpu/ap_boot.rs` — callers: `apic::send_init_assert`, `apic::send_sipi`, `apic::delay_us`

## xAPIC vs x2APIC

| Property | xAPIC | x2APIC |
|----------|-------|--------|
| Access | MMIO at LAPIC base | MSR 0x800–0x8FF |
| LAPIC ID | 8-bit (bits 31:24 of reg 0x020) | 32-bit (MSR 0x802) |
| ICR | Two 32-bit MMIO writes (HI then LO) | Single 64-bit MSR 0x830 write |
| IPI destination | 8-bit field in ICR[HI] | Full 32-bit |
| Mode flag | `X2APIC_ENABLED` static | `is_x2apic_mode()` |

**Critical**: xAPIC destination field is 8 bits. Any LAPIC ID > 0xFF requires x2APIC mode.
`start_aps_from_list` already guards this — do not remove that check.

## INIT/SIPI Sequence (Intel SDM Vol.3 §8.4.4)

```
1. INIT assert:   ICR = dest | LEVEL_ASSERT | TRIGGER_LEVEL | ICR_INIT
2. Wait 10ms
3. INIT deassert: ICR = 0    | LEVEL_DEASSERT | TRIGGER_LEVEL | ICR_INIT  (xAPIC only)
4. SIPI #1:       ICR = dest | ICR_STARTUP | page
5. Wait 10ms (or 200µs per strict spec — 10ms is conservative, use it)
6. SIPI #2:       ICR = dest | ICR_STARTUP | page
```

SIPI vector = physical page number of trampoline (e.g. `0x08` for `0x8000`).
AP ignores SIPI if already running — sending twice is idempotent and required.

## ICR Write Order (xAPIC)

Write `ICR_HI` (destination) **before** `ICR_LO` (command). Writing `ICR_LO` triggers the IPI.
Reverse order = IPI fires to whatever garbage was last in `ICR_HI`.

## LAPIC Base Probing

```rust
// IA32_APIC_BASE MSR 0x1B
// bits 51:12 = physical base (right-shift 12 to get page, left-shift back for address)
// bit 11 = APIC global enable
// bit 10 = x2APIC enable
// bit 8  = BSP flag
let msr = rdmsr(0x1B);
let base = msr & 0x000F_FFFF_FFFF_F000;
```

## Timer Calibration

BSP calibrates once with PIT, stores in `LAPIC_TIMER_INIT_COUNT`.
APs read that value and skip calibration entirely — avoids the PIT race.
If `LAPIC_TIMER_INIT_COUNT == 0`, the AP arrived before BSP finished calibration; spin briefly.

## Common Failure Modes

| Symptom | Cause |
|---------|-------|
| IPI never delivered | ICR_HI written after ICR_LO |
| IPI goes to wrong core | xAPIC mode but LAPIC ID > 0xFF |
| Timer fires once then stops | Periodic bit not set in LVT_TIMER |
| Spurious IRQ 0xFF floods | SVR spurious vector field not set; enable SVR_ENABLE |
| LAPIC reads return all-zeros | LAPIC base wrong; firmware remapped it |

## Procedure

1. Read `apic.rs` in full before touching IPI sequences.
2. Confirm `X2APIC_ENABLED` is set correctly during `init_bsp()`.
3. For any new IPI type: write HI before LO (xAPIC), or single MSR write (x2APIC).
4. After changes, verify with a real boot — QEMU delivers IPIs too leniently.
