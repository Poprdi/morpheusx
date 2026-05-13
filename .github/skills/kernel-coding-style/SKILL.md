---
name: kernel-coding-style
description: 'Kernel-grade coding style for Rust and asm. Use when writing or reviewing kernel code for naming, function size, indentation, comment policy, no-clever-tricks rule, line length, ordering of items in a file, public API surface minimization, type-state encoding to make illegal states unrepresentable, newtype wrappers for unit safety, dead code removal, no speculative abstractions. Synthesizes Linux kernel coding-style.rst, Rust-for-Linux guidelines, and Redox conventions.'
argument-hint: "code to style-review or refactor"
---

# Kernel Coding Style

## When to Use
- Writing any new kernel-side code
- Reviewing a PR for style adherence before merging
- Refactoring code that "works but reads badly"
- Deciding whether a helper function is justified
- Naming a new type or constant

## Function Length

- **Aim for one screen.** If a function doesn't fit on a 24-line terminal, it probably does too much.
- **One purpose per function.** If you describe it with "and", split it.
- **Exception**: state machines and giant match dispatchers can be long if each arm is short.

## Naming

- `snake_case` for functions, methods, modules, locals
- `CamelCase` for types, traits, enum variants
- `SCREAMING_SNAKE_CASE` for constants and statics
- **No abbreviations** unless universally known (`addr`, `len`, `ptr`, `idx` OK; `cnstr` not OK)
- Hardware names match Intel SDM / spec capitalization (`LAPIC`, `MADT`, `RSP0`)
- Newtype wrappers prevent unit confusion: `pub struct LapicId(u32)` not raw `u32`

## Comments

The Linux kernel rule:

> Generally, you want your comments to tell WHAT your code does, not HOW.

In Rust kernel code (per Rust-for-Linux):

- Comments only when:
  - The hardware does something non-obvious
  - A sequencing requirement will silently corrupt state if reordered
  - You're working around a real hardware bug
- **No "what the code already says"** — `// increment counter` above `count += 1` is noise
- **`SAFETY:` comment for every unsafe block** — non-negotiable
- **`# Safety` doc section for every `unsafe fn`**
- Tone: matter-of-fact. Bitter is allowed if the situation warrants ("Yes, this has to be done before RX or the NIC silently eats packets.")

## Doc vs Inline Comment

| Marker | Purpose |
|--------|---------|
| `//` | Implementation detail, for the maintainer reading the source |
| `///` / `//!` | API documentation, for the consumer of the function/module |

Don't use `//` to "document" — use `///`. Don't use `///` to comment on internals — use `//`.

## Make Illegal States Unrepresentable

Prefer compile-time correctness over runtime checks:

```rust
// BAD: runtime check, easy to forget at one callsite
fn send_ipi(dest: u32, vector: u8) {
    assert!(vector >= 32 && vector < 256, "reserved vector");
    ...
}

// GOOD: compile-time enforcement via newtype with private constructor
pub struct UserVector(u8);
impl UserVector {
    pub const fn new(v: u8) -> Option<Self> {
        if v >= 32 { Some(Self(v)) } else { None }
    }
}
fn send_ipi(dest: LapicId, vector: UserVector) { ... }
```

Type-state pattern for state machines:

```rust
pub struct ApBooting;
pub struct ApRunning;
pub struct Ap<S> { id: LapicId, _state: PhantomData<S> }
impl Ap<ApBooting> {
    pub fn complete(self) -> Ap<ApRunning> { ... }
}
// ApRunning::complete doesn't exist — can't double-complete
```

## Public API Surface

- `pub` is opt-in. Default to private.
- A `pub fn` is a contract — once exported, it's hard to remove.
- Module-internal helpers stay private; cross-module helpers go in a dedicated `internal` module if shared.
- Re-exports through `pub use` only when the type is part of the documented API.

## File Layout (Rust)

A consistent ordering makes files navigable:

```rust
//! Module-level doc

#![no_std]                    // crate attributes (lib.rs only)

use ...;                      // imports

// Constants and statics
const FOO: u32 = ...;
static BAR: AtomicU32 = ...;

// Types
pub struct Foo { ... }
pub enum Bar { ... }

// impls in declaration order
impl Foo { ... }
impl Bar { ... }

// Free functions: public first, then private
pub fn init() { ... }
fn helper() { ... }

#[cfg(test)]
mod tests { ... }
```

## Imports (Rust-for-Linux convention)

Vertical layout for nested imports — easier to merge:

```rust
use crate::{
    cpu::{
        apic,
        gdt,
        per_cpu,
    },
    memory::PAGE_SIZE,
    serial::log_error,
};
```

## No Clever Tricks

The Linux kernel rule (paraphrased): *if it isn't obvious to a reader, the cleverness costs more than it saves.*

- Bitwise hacks for arithmetic should be commented with the equivalent expression
- Inline asm beyond the trivial needs context
- Macro magic that hides control flow is rejected
- "Micro-optimization" without a benchmark is just obfuscation

## Dead Code

- No `// TODO: implement later` stubs without an issue link
- No commented-out code — git remembers it
- No `unused` items — gate behind feature flags or delete
- No "we might need this" abstractions — YAGNI

## Constant Assertions

Encode invariants the type system can't:

```rust
const _: () = assert!(core::mem::size_of::<PerCpu>() <= 4096);
const _: () = assert!(PERCPU_LAPIC_BASE == 0x38);
```

These fire at compile time. Use them for ABI offsets, struct sizes, alignment.

## Magic Numbers

- Single-use raw numbers in spec-defined places (CPUID leaf 1, MSR 0x1B) are OK if commented or self-documenting via context
- Anything used twice → `const`
- Anything in a public API → `pub const` with a doc comment

```rust
const IA32_APIC_BASE_MSR: u32 = 0x1B;  // good
let x = msr_read(0x1B);                // bad
```

## Refactoring Checklist

Before opening a PR:

- [ ] Does each function do one thing?
- [ ] Are public items minimal?
- [ ] Every `unsafe` block has a `SAFETY:` comment?
- [ ] Every `unsafe fn` has a `# Safety` doc?
- [ ] No `unwrap`/`expect`/`panic` outside of init paths or invariant checks?
- [ ] No commented-out code?
- [ ] No clever tricks unexplained?
- [ ] `cargo fmt` clean? `cargo clippy` clean (or warnings explained)?

## References
- `Documentation/process/coding-style.rst`
- `Documentation/rust/coding-guidelines.rst`
- The Rust API Guidelines: https://rust-lang.github.io/api-guidelines/
