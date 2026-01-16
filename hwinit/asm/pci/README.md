# PCI ASM Layer

## Purpose

Low-level x86_64 assembly implementations for PCI configuration space access and related operations.

## Files (to be populated in Phase 3)

| File | Description |
|------|-------------|
| `legacy.s` | CF8/CFC port I/O based config access |
| `bar.s` | BAR read, write, sizing operations |
| `capability.s` | PCI capability chain walking |
| `ecam.s` | PCIe ECAM memory-mapped config access |

## ABI

All functions use **Microsoft x64 calling convention** (win64):
- Arguments: RCX, RDX, R8, R9, then stack
- Return: RAX
- Caller-saved: RAX, RCX, RDX, R8, R9, R10, R11
- Callee-saved: RBX, RBP, RDI, RSI, R12-R15

This matches UEFI calling convention and Rust's `extern "win64"`.

## Why ASM?

These operations require:
- Precise instruction ordering (no compiler reordering)
- Volatile semantics (no optimization away)
- Exact port I/O or MMIO sequences

Inline assembly in Rust can achieve this, but separate ASM files:
- Are easier to audit for correctness
- Produce predictable machine code
- Avoid subtle compiler behavior differences

---

*Phase: 2 — Documentation only*
