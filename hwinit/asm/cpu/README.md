# CPU ASM Layer

## Purpose

Low-level x86_64 assembly implementations for CPU primitives: memory barriers, cache management, timing.

## Files (to be populated in Phase 3)

| File | Description |
|------|-------------|
| `barriers.s` | SFENCE, LFENCE, MFENCE memory barriers |
| `cache.s` | CLFLUSH, CLFLUSHOPT cache line operations |
| `delay.s` | TSC-based timing delays |
| `tsc.s` | RDTSC and serialized RDTSC |

## ABI

All functions use **Microsoft x64 calling convention** (win64).

## Critical for DMA

Memory barriers are not optional for DMA correctness:

```
CPU writes descriptor → SFENCE → notify device
device writes data → LFENCE → CPU reads data
```

Without barriers, CPU may reorder operations such that:
- Device sees stale descriptor data
- CPU reads data before device has written it

QEMU often masks these bugs. Real hardware does not.

## Cache Coherency

DMA buffers mapped as Write-Back require explicit cache management:
- Flush before device reads (ensure device sees our writes)
- Invalidate/flush before CPU reads (ensure we see device writes)

UC/WC mappings handle this in hardware but have performance implications.

---

*Phase: 2 — Documentation only*
