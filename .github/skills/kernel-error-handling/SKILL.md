---
name: kernel-error-handling
description: 'Errors-as-values discipline for kernel code: no panics in kernel paths, bounded error codes, Result over Option for fallible ops, errno-style codes, fail-fast at init vs degrade-gracefully at runtime, no unwrap/expect in production paths, free-on-error resource cleanup, ? operator and early return, log_error with bounded codes. Based on Linux kernel error handling patterns and Rust-for-Linux Error type.'
argument-hint: "error path to design or audit"
---

# Kernel Error Handling

## When to Use
- Designing the return type of a new kernel function
- Auditing a path for `unwrap()`, `expect()`, `panic!()` removal
- Resource-allocation paths that must free on partial failure (the goto-fail pattern)
- Adding new bounded error codes for `log_error`/`log_warn`
- Deciding between fail-fast (panic at init) and graceful degradation

## The Iron Rule

**A kernel panic is a system death sentence. There is no recovery.**

This means:

1. `panic!`, `unwrap()`, `expect()`, `unreachable!()`, `todo!()` are reserved for paths where the only correct response is "stop the machine."
2. If a recoverable error exists, it must propagate as a `Result`.
3. Out-of-memory is recoverable. Treat it as such — never `Box::new` without handling allocation failure.

## Where Panic IS Acceptable

- BSP boot init, before the system is functional and there is no userland to corrupt
- Detected memory corruption (e.g., GS-base self-pointer mismatch — a wrong PerCpu means we already lost)
- ABI-contract violations caught by `debug_assert_offsets()` in dev builds
- Type-system invariants that the compiler can't express (e.g., a `const { assert!(...) }` proof)

Outside these: return `Result`.

## Result vs Option

- `Result<T, E>` for "this operation can fail and the caller needs to know why"
- `Option<T>` for "this lookup may legitimately not find anything"

`Option` is not an error. Don't use `Option` to encode failure modes — `None` carries no diagnostic info.

## Bounded Error Codes

Logs must be parseable and bounded. Don't `format!` strings into logs in hot paths — allocate-free integer error codes that map to a registry:

```rust
log_error("AP", 501, "stack allocation failed");
//         ^      ^      ^
//         tag    code   short literal description
```

Per the existing pattern in `ap_boot.rs`:
- `tag` is a 2-4 char subsystem
- `code` is a unique numeric ID per subsystem (grep for collisions before adding)
- description is a `&'static str` literal — never a `format!` result

## Resource Cleanup on Error (the goto-fail pattern in Rust)

C uses `goto fail;` for cleanup. In Rust, the patterns are:

### Pattern A: RAII Drop guards (preferred)
```rust
let _stack = StackGuard::alloc(AP_STACK_SIZE)?;  // Drop frees on early return
let _gdt = GdtGuard::alloc()?;
configure_ap(&_stack, &_gdt)?;
core::mem::forget(_stack);  // commit on success
core::mem::forget(_gdt);
```

### Pattern B: Explicit cleanup in the failure arm
```rust
let stack = alloc_stack()?;
match configure(stack) {
    Ok(v) => v,
    Err(e) => {
        free_stack(stack);  // explicit cleanup, no leak
        return Err(e);
    }
}
```

`ap_boot.rs::boot_single_ap` uses Pattern B for the AP stack — when AP fails to come online, the stack is freed. Don't leak 64 KiB per ghost core.

## The `?` Operator

Use `?` for clean propagation. It's the Rust answer to `goto fail`:

```rust
fn init_ap(idx: u32) -> Result<(), ApInitError> {
    let stack = alloc_stack()?;
    let gdt = alloc_gdt()?;
    configure(stack, gdt)?;
    Ok(())
}
```

If you need cleanup on early return, the `?` shortcut requires Drop guards or you need explicit match arms.

## Error Type Design

Prefer enums with concrete variants over stringly-typed errors:

```rust
#[derive(Debug, Clone, Copy)]
pub enum ApInitError {
    StackAllocFailed,
    TrampolinePageUnavailable,
    Cr3Above4Gb,
    SipiTimeout,
    LapicIdOutOfRange,
}
```

Each variant maps cleanly to a log code. Errors are `Copy` when possible — they're metadata, not heavy values.

## What NOT to Do

| Pattern | Why it's wrong |
|---------|----------------|
| `addr.unwrap()` in driver code | Hides the failure mode; panics on first bad input |
| `expect("should never happen")` | Famous last words; assumptions break |
| `format!("error: {}", x)` in log | Allocates in error path; alloc may also fail |
| Returning `()` and logging on error | Caller has no way to react |
| Returning `Option` for fallible IO | Loses the error reason |
| `panic!("OOM")` for runtime allocation | Convert to `Result<T, AllocError>` |

## Init vs Runtime

| Phase | Failure response |
|-------|------------------|
| Pre-userland init (BSP setup, GDT, IDT, paging) | Panic on failure — no recovery possible |
| AP bring-up | Log and skip the AP — boot continues with fewer cores |
| Driver probe | Return Err; subsystem disables that device |
| Runtime (any time after init complete) | Always Result; never panic |

## References
- Linux kernel `Documentation/process/coding-style.rst` (return values, `goto` cleanup pattern)
- Rust-for-Linux `kernel::error::Error` (errno-style)
- `Documentation/core-api/printk-formats.rst` for log discipline
