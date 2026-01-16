# DMA Module

## Purpose

DMA region abstraction and allocation policy.

## Files (to be populated in Phase 3)

| File | Description |
|------|-------------|
| `mod.rs` | Module root, re-exports |
| `region.rs` | DmaRegion struct and accessors |

## Core Type

```rust
/// DMA-capable memory region.
pub struct DmaRegion {
    /// CPU-accessible pointer
    cpu_ptr: *mut u8,
    /// Device-visible bus address
    bus_addr: u64,
    /// Total size in bytes
    size: usize,
}
```

## Allocation Policy

DMA memory must be:

1. **Below 4GB** — Most devices have 32-bit DMA addressing limitations. Even 64-bit capable devices work fine with low memory.

2. **Identity mapped** — `bus_addr == physical_addr`. We don't use IOMMU remapping initially.

3. **Contiguous** — Physically contiguous for simplicity. Devices see physical addresses.

4. **Properly aligned** — At least page-aligned, often cache-line aligned for performance.

## Layout

The DMA region contains driver-specific layouts. Generic region just provides base addresses:

```
┌──────────────────────────────────────────────┐
│               DMA Region                     │
├──────────────────────────────────────────────┤
│  Driver-specific allocation within region    │
│  (descriptors, rings, buffers)               │
│                                              │
│  hwinit provides the region                  │
│  driver decides internal layout              │
└──────────────────────────────────────────────┘
```

---

*Phase: 2 — Documentation only*
