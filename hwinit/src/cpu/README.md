# CPU Module

## Purpose

Rust bindings for CPU primitives: barriers, cache, timing.

## Files (to be populated in Phase 3)

| File | Description |
|------|-------------|
| `mod.rs` | Module root, re-exports |
| `barriers.rs` | sfence, lfence, mfence wrappers |
| `cache.rs` | clflush, flush_range wrappers |
| `mmio.rs` | MMIO read/write wrappers |
| `pio.rs` | Port I/O wrappers (inb, outb, etc.) |
| `tsc.rs` | TSC read functions |

## Usage Patterns

### DMA Write Path (CPU → Device)

```rust
// 1. Write descriptor
ptr::write_volatile(desc_ptr, descriptor);

// 2. Ensure write is visible to device
cpu::sfence();

// 3. Notify device
mmio::write32(notify_addr, queue_idx);
```

### DMA Read Path (Device → CPU)

```rust
// 1. Device has written data

// 2. Read device's completion index
let idx = ptr::read_volatile(used_idx_ptr);

// 3. Ensure we don't read data before index
cpu::lfence();

// 4. Read the actual data
let data = ptr::read_volatile(buffer_ptr);
```

---

*Phase: 2 — Documentation only*
