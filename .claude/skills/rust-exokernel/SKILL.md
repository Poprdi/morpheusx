---
name: rust-exokernel
description: |
  Develop bare-metal Rust code for MorpheusX exokernel. Enforces no_std conventions,
  minimal unsafe code, proper error handling, and exokernel design principles.
author: MorpheusX Architecture Team
version: 2026.1
---

## Inputs Required From User
- Component being modified (core, helix, hwinit, bootloader, display, network)
- Type of change (new driver, feature, bug fix, refactor)
- Whether the code touches unsafe blocks
- Architecture constraints (interrupt context, boot phase, etc.)

## Process

### Step 1: Validate NoStd Compliance
1. Check that core/helix/hwinit code has `#![no_std]` attribute
2. Verify no `std`, `alloc`, or `println!` imports in core components
3. Ensure `core::fmt` or custom formatting is used instead
4. Check that error handling uses `Result`/`Option` without unwrap

### Step 2: Unsafe Code Discipline
1. Every `unsafe` block must have a `SAFETY` comment explaining:
   - What invariants must hold
   - Why this can't be done safely
   - What prevents UB
2. Prefer `unsafe fn` with SAFETY doc comment over bare unsafe blocks
3. No unsafe in interrupt handlers unless absolutely necessary
4. Audit all pointer arithmetic for bounds

### Step 3: Memory Management Patterns
- Use custom allocators (buddy allocator already in tree)
- Pre-allocate DMA regions at boot (2 MB)
- Heap: 4 MB primary + overflow
- All allocations must propagate errors, never panic

### Step 4: Error Handling
```rust
// GOOD: Proper error propagation
fn allocate_frame(&mut self) -> Result<PhysAddr, MemoryError> {
    self.buddy_allocator.allocate(order).ok_or(MemoryError::OutOfMemory)
}

// BAD: Unwrap in core code
let ptr = self.allocator.alloc().unwrap();
```

### Step 5: Synchronization
- Spinlocks for critical sections (no sleeping in interrupts)
- Proper memory barriers (`core::sync::atomic::fence`)
- RCU-style read-copy-update for scheduler

## Quality Checklist
- [ ] `cargo fmt` passes
- [ ] `cargo clippy --target x86_64-unknown-uefi` passes or justified allows
- [ ] All unsafe blocks have SAFETY comments
- [ ] No `unwrap()`/`expect()` in core code
- [ ] All allocations return `Result` with error paths
- [ ] No `std`/`alloc` imports in no_std crates
- [ ] Interrupt handlers are `#[inline(never)]` with proper calling conventions
- [ ] Memory barriers on volatile accesses
- [ ] Pointer arithmetic validated for overflow

## References
- The Rustonomicon (unsafe code)
- `no_std` Rust book
- MorpheusX CONTRIBUTING.md