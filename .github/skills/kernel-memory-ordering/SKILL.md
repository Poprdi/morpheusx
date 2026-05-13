---
name: kernel-memory-ordering
description: 'Memory ordering, barriers, and the C/Rust memory model in kernel code. Use when adding fences, choosing atomic ordering (Relaxed/Acquire/Release/AcqRel/SeqCst), pairing publish/subscribe between cores, MMIO ordering vs CPU memory ordering, sfence/lfence/mfence, compiler barriers vs CPU barriers, AP synchronization, write-combining, store buffer flushing, dependency ordering. Based on Linux Documentation/memory-barriers.txt.'
argument-hint: "ordering question or barrier to add"
---

# Kernel Memory Ordering

## When to Use
- Choosing `Ordering::*` for an `Atomic*` operation
- Adding `core::sync::atomic::fence(...)` calls
- Coordinating data publication between BSP and APs
- MMIO writes that must complete before triggering a device action
- Suspecting a "works on x86, breaks on ARM" data race
- Reviewing AP_ONLINE_COUNT / handoff sequences

## Core Principle

The CPU and compiler will both reorder memory operations unless told not to. Single-threaded code is unaffected because they preserve program-order *as observed by that thread*. The trouble starts when another agent (another CPU, a device, an interrupt handler) observes those operations.

x86 has a strong memory model (TSO — total store order). ARM/RISC-V are weakly ordered. **Code that is correct only on x86 is broken kernel code.** Write portably ordered code even if you only ship x86 today.

## Ordering Cheat Sheet

| Ordering | Use case | What it guarantees |
|----------|----------|---------------------|
| `Relaxed` | Independent counters (statistics, hit counts) | Atomicity only — no ordering |
| `Acquire` (load) | Reading a flag that gates other reads | All later reads/writes happen after this load |
| `Release` (store) | Writing a flag that publishes earlier writes | All earlier reads/writes happen before this store |
| `AcqRel` | RMW that both reads and publishes (lock acquire) | Combination |
| `SeqCst` | Need a single total order across all SeqCst ops | Strongest, slowest, simplest to reason about |

## The Publish / Subscribe Pattern

Producer writes data, then sets a flag. Consumer reads the flag, then reads the data:

```rust
// Producer (BSP):
unsafe { *trampoline_data = config; }      // (1) write payload
READY.store(true, Ordering::Release);       // (2) publish

// Consumer (AP):
while !READY.load(Ordering::Acquire) {}     // (3) subscribe
let v = unsafe { *trampoline_data };        // (4) read payload — guaranteed to see (1)
```

`Release` on the store + `Acquire` on the load establishes a happens-before edge from (1) to (4). Without these, the consumer can see `READY=true` but stale payload data.

`SeqCst` works too but is overkill — use it only when you need a single total order across multiple flags.

## When SeqCst is Right

```rust
// Two flags whose relative order matters across cores:
A.store(true, Ordering::SeqCst);
let b = B.load(Ordering::SeqCst);
// On any other core seeing B=true must also see A=true (some scenarios).
```

If you cannot articulate why you need SeqCst, you probably want Acquire/Release.

## Fences

`core::sync::atomic::fence(Ordering::SeqCst)` is a CPU memory barrier. Use when you have a sequence of plain (non-atomic) reads/writes that need ordering against atomic ops.

`core::sync::atomic::compiler_fence(...)` only blocks compiler reordering. Use for signal/interrupt handler boundaries on a single CPU where the CPU can't reorder anyway (e.g., SWAPGS pairing).

## MMIO is NOT Memory

MMIO writes can sit in the CPU's write buffer or PCI write-posting queue. To guarantee the device sees a write before another action:

1. Use `write_volatile` (already required for MMIO)
2. Follow with a read from the same device (a "posted-write flush" — most common pattern), OR
3. Insert an `sfence` if write ordering between two device registers matters

```rust
// Trigger DMA, then wait for completion:
unsafe { write_volatile(ctrl_reg, START); }
let _ = unsafe { read_volatile(ctrl_reg); }; // posted-write flush
```

The DMA APIs in `hwinit/src/dma/` should encapsulate this — drivers should not roll their own MMIO ordering.

## x86 Specifics (Don't Rely On These for Portability)

- All loads have implicit acquire semantics (TSO)
- All stores have implicit release semantics
- `lock`-prefixed RMW is full barrier (SeqCst)
- `mfence` is the only "everything before everything after" barrier
- `sfence` orders only stores, `lfence` only loads
- Non-temporal stores (movnt*) bypass cache and can be reordered against everything — always pair with `sfence`

A correct x86 program written with C11/Rust memory model orderings remains correct on ARM. The reverse is not true.

## Dependency Ordering

A load that produces a value used in the address of another load creates an *address dependency*. On most architectures (excluding old Alpha) this implies ordering between the two loads:

```rust
let p = PTR.load(Ordering::Acquire);  // explicit Acquire
let v = unsafe { *p };                 // address-dependent — sees init data
```

`Acquire` here is conservative-correct. RCU traditionally relies on dependency ordering (`READ_ONCE` + the implicit address dep). In Rust, prefer explicit `Acquire`.

## Common Failure Modes

| Symptom | Cause |
|---------|-------|
| Works in QEMU, fails on real hardware under load | Missing barriers; weak-memory hardware exposes a race x86-TSO hides |
| AP sees garbage in trampoline data block | No fence between BSP writes and SIPI; SIPI is not a barrier across CPUs |
| Device reads stale ring buffer descriptor | No `wmb`-equivalent before ringing the doorbell |
| Sporadic counter goes backward | Used `Relaxed` where multi-step Acquire/Release was needed |
| Lockless queue corrupts on contention | Hand-rolled lock-free without proof — switch to a verified primitive |

## Procedure

1. Identify the producer-consumer pair: who writes, who reads, what data is published?
2. The publisher uses `Release`; the subscriber uses `Acquire`.
3. For pure independent counters, `Relaxed` is fine.
4. For complex multi-flag protocols, default to `SeqCst` — get it correct first, optimize after measurement.
5. Comment every non-trivial barrier explaining what it pairs with (Linux kernel rule: every memory barrier needs a comment).

## References
- `Documentation/memory-barriers.txt`
- C11/Rust memory model: https://en.cppreference.com/w/cpp/atomic/memory_order
- "Memory Models" by McKenney (LWN series)
