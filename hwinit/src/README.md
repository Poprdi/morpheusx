# hwinit Source

## Purpose

Rust bindings and abstractions over the ASM primitives, plus platform orchestration.

## Modules (to be populated in Phase 3)

| Module | Description |
|--------|-------------|
| `pci/` | PCI configuration access, BAR decoding, capability walking |
| `cpu/` | Memory barriers, cache management, TSC, timing |
| `dma/` | DMA region abstraction, allocation policy |
| `platform.rs` | Platform initialization orchestrator |
| `lib.rs` | Crate root, public API |

## Design Constraints

1. **Thin wrappers** — Rust code should be minimal logic over ASM
2. **No allocations in core paths** — Static or caller-provided buffers only
3. **No panics in init** — Return errors, let caller decide to panic
4. **No dependencies on network** — hwinit is foundational, not dependent

## Public API (conceptual)

```rust
// Entry point
pub fn platform_init() -> Result<PlatformInit, InitError>;

// Prepared device descriptors
pub struct PreparedNetDevice { ... }
pub struct PreparedBlkDevice { ... }

// DMA region (ownership transferred to drivers)
pub struct DmaRegion { ... }

// Low-level primitives (for drivers that need them)
pub mod cpu {
    pub fn sfence();
    pub fn lfence();
    pub fn mfence();
    pub fn read_tsc() -> u64;
    // ...
}
```

---

*Phase: 2 — Documentation only*
