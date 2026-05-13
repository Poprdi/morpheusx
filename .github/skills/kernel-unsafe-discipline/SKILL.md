---
name: kernel-unsafe-discipline
description: 'Rust unsafe block and unsafe fn discipline for kernel code. Use when writing or reviewing unsafe blocks, raw pointer dereferences, MMIO/PIO wrappers, transmute, FFI to assembly, ptr::read_unaligned on packed structs, NonNull invariants, sound Safe/Unsafe API boundaries, SAFETY: comments, # Safety doc sections, justifying why unsafe is correct. Based on Rust-for-Linux coding guidelines and Rustonomicon.'
argument-hint: "unsafe block to write/audit"
---

# Kernel Unsafe Discipline

## When to Use
- Writing any new `unsafe` block or `unsafe fn`
- Reviewing existing unsafe code for soundness
- Wrapping MMIO/PIO/MSR primitives in safe APIs
- Transmuting between repr(C) types or `&[u8]` slices
- Calling `extern "C"` assembly stubs from Rust

## The Two Rules (Rust-for-Linux)

1. **Every `unsafe` block needs a `// SAFETY:` comment** above it explaining why the call cannot trigger UB. No exceptions.
2. **Every `unsafe fn` needs a `# Safety` doc section** stating the contract callers must uphold.

```rust
/// Writes `val` to MMIO register at `addr`.
///
/// # Safety
/// `addr` must be a valid MMIO address mapped uncacheable, and the
/// device must be in a state where this register is writable.
pub unsafe fn mmio_write32(addr: *mut u32, val: u32) {
    // SAFETY: caller guarantees `addr` is valid mapped MMIO per the contract above.
    unsafe { core::ptr::write_volatile(addr, val) }
}
```

## Soundness Boundary Pattern (R4L abstractions/bindings model)

Build pyramids: a thin `unsafe` core wrapped by a safe API. Drivers and leaf code consume only the safe layer.

```
+---------------------------------+
| Safe API (no unsafe at callsite)|  <- consumed by drivers
+---------------------------------+
| Audited unsafe abstractions     |  <- one place to review
+---------------------------------+
| Raw FFI / asm / volatile ops    |  <- minimal, leaf-only
+---------------------------------+
```

If a safe function can cause UB by being called with arbitrary safe inputs, it is **unsound** — mark it `unsafe fn` and document the contract.

## Unsafe Block Scope

Keep blocks minimal. The compiler doesn't care, but reviewers do:

```rust
// BAD: 30-line unsafe block, hides everything
unsafe {
    let a = ...;
    let b = compute_safely(a);
    let c = ...;
    write_volatile(reg, c);
}

// GOOD: only the actually-unsafe operation is in the block
let a = unsafe { read_volatile(src) };
let b = compute_safely(a);
let c = transform(b);
// SAFETY: `reg` is the LAPIC EOI register, mapped at init.
unsafe { write_volatile(reg, c) };
```

## Forbidden Patterns

- `unsafe` block with no SAFETY comment → reject in review
- `// SAFETY: trust me` or `// SAFETY: it works` → reject
- Wrapping in safe `fn` to suppress unsafe-counts without removing UB potential → unsound
- `mem::transmute` when `as` cast or `from_bits`/`into_bits` works → use the narrower op
- `&*ptr` to construct `&T` from raw pointer without proving lifetime + aliasing → audit aliasing rules

## Packed Struct Reads (ACPI, MADT, PCI config)

`#[repr(C, packed)]` fields are unaligned. Plain field access constructs a misaligned reference = UB.

```rust
// WRONG: misaligned reference, UB on some architectures
let id = entry.apic_id;

// RIGHT: read by value through unaligned pointer access
let id = unsafe { core::ptr::read_unaligned(&raw const entry.apic_id) };
```

Use `&raw const` / `&raw mut` (stable since 1.82) instead of `&` / `&mut` to packed fields.

## Volatile = Always for MMIO

Plain reads/writes through a `*mut u32` MMIO pointer can be elided, reordered, fused, or duplicated by the optimizer. Always use `read_volatile` / `write_volatile`. A `volatile` op is not a memory barrier — pair with `barriers::*` if you need ordering across multiple registers.

## SAFETY Comment Anatomy

A good SAFETY comment names the invariants being relied on:

```
// SAFETY: <pointer> is non-null because <reason>;
//         <pointer> is aligned for T because <reason>;
//         the referenced memory is initialized because <reason>;
//         no other references alias this region because <reason>.
```

You don't need to enumerate all four every time — name the ones that aren't obvious.

## References
- Rust-for-Linux coding guidelines: https://docs.kernel.org/rust/coding-guidelines.html
- Rustonomicon: https://doc.rust-lang.org/nomicon/
- Linux kernel `Documentation/rust/general-information.rst`
