---
name: msr-setup
description: 'MSR programming for CPU bring-up. Use when configuring IA32_EFER, IA32_STAR, IA32_LSTAR, IA32_CSTAR, IA32_SYSCALL_MASK, IA32_GS_BASE, IA32_KERNEL_GS_BASE, IA32_FS_BASE, SYSCALL MSR layout, syscall_init, SFMASK, STAR ring transition selectors, LSTAR syscall handler address, per-AP MSR init, rdmsr/wrmsr inline asm, MSR programming order, EFER.LME/SCE/NXE, MSR-based LAPIC access in x2APIC mode.'
argument-hint: "MSR to configure or SYSCALL path bug"
---

# MSR Setup

## When to Use
- Implementing or fixing `syscall_init` (STAR/LSTAR/SFMASK)
- Configuring `IA32_GS_BASE` / `IA32_KERNEL_GS_BASE` during AP init
- Setting `IA32_EFER` (LME/SCE/NXE bits)
- Adding per-core MSR writes to `ap_rust_entry`
- MSR-based LAPIC access (x2APIC mode uses MSRs 0x800–0x8FF)

## Key MSRs

| MSR | Address | Purpose |
|-----|---------|---------|
| `IA32_EFER` | 0xC000_0080 | SCE (SYSCALL enable), LME, NXE |
| `IA32_STAR` | 0xC000_0081 | SYSCALL/SYSRET CS/SS selectors |
| `IA32_LSTAR` | 0xC000_0082 | SYSCALL handler RIP (64-bit) |
| `IA32_CSTAR` | 0xC000_0083 | SYSCALL handler RIP (compat, unused) |
| `IA32_SYSCALL_MASK` | 0xC000_0084 | RFLAGS bits to clear on SYSCALL |
| `IA32_FS_BASE` | 0xC000_0100 | FS base (thread-local storage) |
| `IA32_GS_BASE` | 0xC000_0101 | GS base (kernel PerCpu pointer) |
| `IA32_KERNEL_GS_BASE` | 0xC000_0102 | Shadow GS (user value, swapped by SWAPGS) |
| `IA32_APIC_BASE` | 0x0000_001B | LAPIC base + x2APIC enable bit |

## STAR Layout

```
STAR[63:48] = SYSRET CS/SS  (CS = this + 16, SS = this + 8)
STAR[47:32] = SYSCALL CS/SS (CS = this,       SS = this + 8)
STAR[31:0]  = reserved (0)
```

For the GDT layout in `gdt.rs` (null=0x00, kcode=0x08, kdata=0x10, udata=0x18, ucode=0x20):
```
SYSCALL CS = 0x08 (kernel code)
SYSRET CS  = 0x18 (user data — CPU adds 8 and 16 to get SS and CS)
STAR = (0x18u64 << 48) | (0x08u64 << 32)
```

## SFMASK (IA32_SYSCALL_MASK)

Bits set here are cleared in RFLAGS on SYSCALL entry.
Minimum: clear `IF` (bit 9) to disable interrupts, `TF` (bit 8) to prevent singlestep into kernel.
Typical: `0x47700` = IF + TF + DF + AC + NT cleared.

## Per-Core Init Order in `ap_rust_entry`

MSRs must be written **after** GDT is loaded, **before** interrupts are enabled:

1. `gdt::init_gdt_for_ap(stack_top, core_idx)` — loads GDT, TSS
2. `idt::load_idt_for_ap()` — loads IDT
3. `per_cpu::init_ap(...)` — writes `IA32_GS_BASE` + `IA32_KERNEL_GS_BASE`
4. `sse::enable_sse()` — CR4 changes
5. `syscall_init()` — writes STAR, LSTAR, SFMASK, EFER.SCE
6. `apic::init_ap()` — LAPIC enable (may use x2APIC MSRs)
7. `AP_ONLINE_COUNT.fetch_add(1, SeqCst)` — signal online
8. `sti` — enable interrupts

Do not reorder 3 and 5 — `syscall_init` may read GS-base.

## rdmsr / wrmsr Inline ASM

```rust
#[inline(always)]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32; let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo, out("edx") hi,
        options(nostack, nomem),
    );
    ((hi as u64) << 32) | lo as u64
}

#[inline(always)]
unsafe fn wrmsr(msr: u32, val: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") val as u32,
        in("edx") (val >> 32) as u32,
        options(nostack, nomem),
    );
}
```

## Common Failure Modes

| Symptom | Cause |
|---------|-------|
| `#GP` on SYSCALL | EFER.SCE not set; STAR selectors don't match loaded GDT |
| LSTAR points to wrong address | Linked address differs from runtime; check with `ap_rust_entry as u64` pattern |
| SWAPGS corrupts user GS | `IA32_KERNEL_GS_BASE` not initialized to user value before ring-3 entry |
| x2APIC LAPIC reads return garbage | Reading via MMIO when x2APIC is enabled — must use MSRs 0x800–0x8FF |

## Procedure

1. Verify GDT layout in `gdt.rs` before computing STAR value.
2. Write all per-core MSRs in `syscall_init` (called from `ap_rust_entry`).
3. `IA32_GS_BASE` / `IA32_KERNEL_GS_BASE` are written in `per_cpu::init_ap` — do not duplicate.
4. After changes: test SYSCALL round-trip; verify SWAPGS works at both syscall entry and interrupt entry.
