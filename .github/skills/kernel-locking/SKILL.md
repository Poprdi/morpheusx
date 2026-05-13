---
name: kernel-locking
description: 'Kernel locking discipline: spinlocks vs mutexes, IRQ-safe critical sections, lock ordering, no-sleep contexts, deadlock avoidance, lockdep-style analysis. Use when adding a Mutex/SpinLock, choosing between spin and sleep locks, fixing AB-BA deadlocks, debugging IRQ-context "scheduling while atomic", per-CPU locks, RCU-style read-mostly access, lock acquisition ordering, raw_spinlock vs spinlock, irqsave/irqrestore. Based on Linux Documentation/locking/.'
argument-hint: "lock to add or deadlock to debug"
---

# Kernel Locking

## When to Use
- Adding a new lock to a shared data structure
- Choosing between spinlock, mutex, RwLock, atomic, or RCU/seqlock
- Debugging deadlocks (especially AB-BA)
- Fixing "function called from atomic context can sleep" bugs
- Auditing IRQ-context code paths

## Lock Type Decision Tree

```
Will the critical section EVER be entered from interrupt context?
├── YES: must use spinlock (or atomic) — sleeping locks deadlock the IRQ
│         └── Disable local IRQs while holding (irqsave/irqrestore)
│             OR use a spinlock primitive that does it for you
└── NO:  can the critical section block / call alloc?
          ├── YES: use Mutex (sleeping lock OK, only kernel-thread context)
          └── NO:  use spinlock anyway (cheaper than mutex if uncontended)
```

## Spinlock Rules (the Linux discipline)

1. **Hold time MUST be short and bounded.** No allocations, no I/O, no calls into unknown subsystems.
2. **Never call code that may sleep while holding a spinlock.** This includes mutex_lock, kmalloc with non-atomic flags, copy_to_user.
3. **If the lock is also taken from IRQ context, you MUST disable interrupts on the holding CPU**, otherwise the IRQ tries to take the lock and deadlocks the same CPU.
4. **Acquire order is a global property.** If two locks are ever taken together, every site must take them in the same order.

```rust
// Lock taken from both task and IRQ context — must save/restore IRQ state
let flags = lock.lock_irqsave();
// critical section: short, bounded, no sleeping calls
lock.unlock_irqrestore(flags);
```

## Mutex Rules

- Only from preemptible (task) context. NEVER from interrupt or softirq.
- Owner semantics: the task that locked must unlock.
- Can sleep while contended — fine to call any blocking function inside.
- Use `try_lock` if you must check from a context that might be atomic.

## Atomics — When NO Lock Suffices

For single-word counters, flags, or sequence numbers, prefer `AtomicU32`/`AtomicU64` over a lock + `u32`. Picking the right ordering is the trick:

| Need | Ordering |
|------|----------|
| Independent counter, no other data depends on its value | `Relaxed` |
| Publishing data: writer stores ptr, readers must see initialized data | Writer `Release`, reader `Acquire` |
| Two-way synchronization (e.g. AP_ONLINE_COUNT signaling readiness) | `SeqCst` |
| Lock-like (taken/released, exclusion) | Use a real lock or `compare_exchange` with `Acquire`/`Release` |

**Default to `SeqCst` only when in doubt and reads are rare.** It is correct but expensive on weak-memory architectures (not x86 — but be portable).

## RCU / Read-Mostly Patterns

For data that is read constantly and written rarely (config tables, device lists, per-CPU stat hot paths):

- Linux uses RCU
- Bare-metal Rust kernels often use `arc_swap`-style or sequence locks (`SeqLock`)
- Pattern: writer publishes a new immutable snapshot; readers grab a pointer with `Acquire` and operate on the immutable copy

Avoid rolling your own lock-free algorithm unless you have a proof. The graveyard of lock-free queues is large.

## AB-BA Deadlock — The Classic

```
CPU A: lock(X); lock(Y); ...   // takes X, waits on Y
CPU B: lock(Y); lock(X); ...   // takes Y, waits on X  --> deadlock
```

Prevention:

1. **Document a global lock order.** Put it at the top of the module. Every nested-lock site obeys it.
2. **Smaller scope wins**: only hold one lock at a time when possible.
3. **`try_lock` and back off** if you must violate order temporarily.
4. **Use a lockdep equivalent**: instrument `lock` with a per-CPU "currently held" stack and assert ordering in debug builds.

## Per-CPU Data — The "No Lock" Trick

Data accessed only by one CPU needs no lock — but you must guarantee that property:

1. **Disable preemption** while accessing per-CPU data, or
2. **Disable IRQs** if the same data is touched from IRQ context on this CPU
3. **Cache-line align** the per-CPU struct (`#[repr(C, align(64))]`) to prevent false sharing

This is exactly what `PerCpu` does in `hwinit/src/cpu/per_cpu.rs`.

## Common Failure Modes

| Symptom | Cause |
|---------|-------|
| Hard hang, one CPU spinning forever | Spinlock held when IRQ fired and IRQ tried to take it; missing `_irqsave` |
| Sporadic data corruption under load | Wrong atomic ordering (Relaxed where Release/Acquire was needed) |
| Hang only with multiple cores online | AB-BA deadlock; you got lucky on UP |
| Triple fault during interrupt | Acquired a Mutex from interrupt context |
| Watchdog timeout in scheduler | Long critical section, or sleeping while holding a spinlock |

## Procedure

1. Before adding a lock: check if the data can be made per-CPU instead (zero contention).
2. Identify EVERY context that touches the data: task, softirq, hardirq, NMI?
3. Pick the lock type from the decision tree above.
4. Document the lock's acquisition order if it's ever held with another lock.
5. In debug builds, add `assert!(!in_interrupt())` before mutex acquisition.

## References
- `Documentation/locking/locktypes.rst`
- `Documentation/locking/spinlocks.rst`
- `Documentation/memory-barriers.txt`
