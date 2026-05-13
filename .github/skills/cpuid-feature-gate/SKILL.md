---
name: cpuid-feature-gate
description: 'CPUID-gated CPU feature enablement. Use when enabling SSE, SSE2, AVX, XSAVE, FSGSBASE, SMEP, SMAP, CET, UMIP, PKE, or any CPUID-detected feature, sse.rs, CPUID leaf 1 ECX/EDX, CPUID leaf 7 EBX/ECX/EDX, CR4 bits, XCR0 setup for XSAVE, feature detection at BSP and AP init, enable_sse, CPU feature flags, x86_64 extended features.'
argument-hint: "Feature to enable or CPUID detection task"
---

# CPUID Feature Gate

## When to Use
- Enabling a new CPU feature (SSE/AVX/SMEP/SMAP/CET/FSGSBASE/UMIP)
- Adding CPUID detection for a feature before gating code on it
- Enabling the feature on APs (`ap_rust_entry` calls `enable_sse` — extend pattern there)
- `XCR0` configuration for XSAVE-based FPU state

## Key Files
- `hwinit/src/cpu/sse.rs` — `enable_sse`, current SSE enablement
- `hwinit/src/cpu/ap_boot.rs` — `ap_rust_entry` calls `enable_sse` on each AP

## CPUID Leaves

| Leaf | Register | Feature |
|------|----------|---------|
| 1 | EDX bit 25 | SSE |
| 1 | EDX bit 26 | SSE2 |
| 1 | ECX bit 0  | SSE3 |
| 1 | ECX bit 19 | SSE4.1 |
| 1 | ECX bit 20 | SSE4.2 |
| 1 | ECX bit 26 | XSAVE |
| 1 | ECX bit 28 | AVX |
| 7/0 | EBX bit 5  | AVX2 |
| 7/0 | EBX bit 7  | SMEP |
| 7/0 | EBX bit 20 | SMAP |
| 7/0 | EBX bit 16 | FSGSBASE |
| 7/0 | ECX bit 2  | UMIP |
| 7/0 | ECX bit 7  | CET_SS |

Leaf 7 requires ECX=0 subleaf.

## Enabling Features

**SSE**:
```
CR0: clear EM (bit 2), set MP (bit 1)
CR4: set OSFXSR (bit 9), OSXMMEXCPT (bit 10)
```

**SMEP/SMAP**:
```
CR4: set SMEP (bit 20), SMAP (bit 21)
SMAP: clear AC flag in RFLAGS before accessing user memory (CLAC/STAC)
```

**XSAVE/AVX**:
```
CR4: set OSXSAVE (bit 18)
XCR0: set bit 0 (x87), bit 1 (SSE), bit 2 (AVX) via XSETBV
```

**FSGSBASE**:
```
CR4: set FSGSBASE (bit 16)
Enables RDFSBASE/WRFSBASE/RDGSBASE/WRGSBASE in user mode
```

## Enablement Pattern

All features must be:
1. Detected via CPUID before attempting to enable.
2. Enabled on **every core** — BSP and every AP.
3. Called from `ap_rust_entry` for AP path.

```rust
pub unsafe fn enable_smep_smap() {
    // CPUID leaf 7 subleaf 0
    let ebx: u32;
    core::arch::asm!(
        "cpuid",
        inout("eax") 7u32 => _,
        in("ecx") 0u32,
        lateout("ebx") ebx,
        lateout("ecx") _,
        lateout("edx") _,
    );
    let mut cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4);
    if ebx & (1 << 7) != 0 { cr4 |= 1 << 20; } // SMEP
    if ebx & (1 << 20) != 0 { cr4 |= 1 << 21; } // SMAP
    core::arch::asm!("mov cr4, {}", in(reg) cr4);
}
```

## XCR0 / XSAVE State Components

Set only the state components your FPU save path handles. Enabling AVX in XCR0 without saving YMM state in context switches = corrupted AVX registers across context switches.

| XCR0 bit | State | Size added to XSAVE area |
|----------|-------|--------------------------|
| 0 | x87 FPU | 160 bytes |
| 1 | SSE (XMM) | 256 bytes |
| 2 | AVX (YMM hi) | 256 bytes |
| 5,6 | AVX-512 opmask + ZMM_Hi256 | 64 + 512 bytes |

Only enable bits for state your FPU save buffer actually has space for.

## Procedure

1. Check `sse.rs` for the existing pattern before adding new feature enablement.
2. Add CPUID check gating the CR4/XCR0 write.
3. Call the new enable function from both BSP init path and `ap_rust_entry`.
4. Confirm `debug_assert_offsets` still passes (CR4 changes can affect GS-base behavior).
