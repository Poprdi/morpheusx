---
name: x86-64-lowlevel
description: |
  Implement x86_64 architecture-specific code: GDT, IDT, paging, syscalls.
  Follow AMD64 architecture manual conventions and use proper memory barriers.
author: MorpheusX Architecture Team
version: 2026.1
---

## Inputs Required
- Architecture feature (GDT/IDT/paging/syscall/MSR)
- Context (boot, kernel, interrupt)
- Whether inline ASM vs separate assembly is needed

## Process

### Step 1: Descriptor Tables (GDT/IDT)
```rust
// GDT entry for x86_64
#[repr(C, packed)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    flags: u4,
    limit_high: u4,
    base_high: u8,
}
```
- 8-byte GDT entries (64-bit mode)
- TSS descriptor with IST
- Proper DPL/RPL handling

### Step 2: Interrupt Descriptor Table
- IDT entries: 16 bytes each in x86_64
- Interrupt gate (D=1, P=1, type=14)
- Error code pushed for page fault, divide error
- IST for NMI, double fault, machine check

### Step 3: Paging
- 4-level paging (PML4 → PDP → PD → PT)
- 2MB pages for kernel mappings
- NX bit on user pages
- Proper CR3 reload sequence

### Step 4: Syscall/SYSRET
- Enable with `MSR_EFER.SCE`
- Set `MSR_STAR` for CS/SS selectors
- Set `MSR_LSTAR` for syscall entry
- Set `MSR_SYSCALL_FLAG_MASK`

### Step 5: Memory Barriers
```rust
// Required before locked instructions
core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

// For device memory (MMIO)
core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
```

## Critical Patterns

### Volatile MMIO
```rust
// GOOD: Volatile read with memory barrier
let value = core::ptr::read_volatile(addr as *const u32);
core::sync::atomic::fence(Ordering::SeqCst);

// BAD: Speculative read could be reordered
let value = *addr;
```

### Locked Instructions
```rust
// MUST fence before lock prefix on x86_64
core::sync::atomic::fence(Ordering::SeqCst);
let result = core::sync::atomic::atomic_xadd(&mut counter, 1);
```

## References
- AMD64 Architecture Manual (Vol 2: System Programming)
- Intel SDM Vol 3 (selected chapters)