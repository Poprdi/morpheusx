---
name: interrupt-driven-refactor
description: |
  Convert MorpheusX polling loops (event-ring drain, MMIO status spin, completion
  poll) into MSI/MSI-X interrupt-driven paths with HLT-based wait. Preserves the
  existing drain invariant and quirks; polling stays as a fallback.
author: MorpheusX Architecture Team
version: 2026.1
---

## When to use

A driver currently busy-waits on a device-state register or event ring and you want
to convert it to interrupt-driven completion. Typical targets in this codebase:

- `hwinit/src/usb/controller.rs` — `wait_cmd` / `wait_xfer`
- `network/src/driver/usb_msd/mod.rs` — event-ring drain
- `network/src/driver/ahci/*` — `PXIS` poll
- `network/src/driver/virtio*` — used-ring poll
- `network/src/driver/intel/*` — RX/TX descriptor poll
- `network/src/driver/sdhci/mod.rs` — command-complete poll

Not for: spinlock CAS, panic-path UART, PIT calibration (timing-bound).

## Companion skills

- `pci-msi-programming` — for the capability-walk and MSI-X table programming
- `x86-64-lowlevel` — IDT, LAPIC EOI, `sti;hlt;cli` semantics
- `hardware-abstraction` — volatile MMIO, DMA, barriers

## The five steps

### Step 1 — Reserve an IDT vector

Pick a free vector ≥ 0x40 (0x20–0x2F belong to LAPIC timer + IPIs). Add a named
constant alongside `TIMER_VECTOR` in `hwinit/src/cpu/apic.rs`:

```rust
pub const XHCI_VECTOR: u8 = 0x40;
pub const AHCI_VECTOR: u8 = 0x41;
// ...
```

Then in `hwinit/src/platform.rs` (next to the existing timer hookup):

```rust
set_interrupt_handler(XHCI_VECTOR, xhci_isr as u64, 0, 0);
```

### Step 2 — Program MSI-X via the pci-msi-programming skill

```rust
if let Some(cap) = pci::find_msix(bdf) {
    cap.program_entry(0, lapic_phys_addr(), XHCI_VECTOR, /*masked=*/false);
    cap.enable();
} else if let Some(cap) = pci::find_msi(bdf) {
    cap.program(lapic_phys_addr(), XHCI_VECTOR);
    cap.enable();
} else {
    // No MSI capability — fall back to polling-only mode. Don't fail.
}
```

Keep the polling fallback path. Devices on legacy buses or virtualized environments
without MSI must still work.

### Step 3 — Build a completion table

The ISR cannot block, allocate, or call into anything that takes a sleeping lock.
It only drains hardware state into a pre-allocated table. The waiter reads from
the table.

```rust
pub struct XhciCompletion {
    ready: AtomicBool,
    status: UnsafeCell<u32>,
    ctrl: UnsafeCell<u32>,
}

static CMD_SLOT: XhciCompletion = XhciCompletion::new();
static XFER_SLOTS: [XhciCompletion; MAX_SLOTS * 31] = [...];

unsafe fn drain_events_into(table: &XhciTable) {
    while let Some((_, status, ctrl)) = self.evt_ring.peek() {
        let ty = ctrl & TYPE_MASK;
        self.evt_ring.advance();
        self.update_erdp();
        match ty {
            TRB_CMD_COMPLETE => CMD_SLOT.publish(status, ctrl),
            TRB_TRANSFER_EVENT => {
                let key = ((ctrl >> 24) as usize) * 31
                        + (((ctrl >> 16) & 0x1F) as usize);
                XFER_SLOTS[key].publish(status, ctrl);
            }
            TRB_PSCEC => { /* drain — see usb_event_ring_drain_invariant memory */ }
            _ => {}
        }
    }
}
```

`publish` writes payload then sets `ready` with `Release`; consumer reads with `Acquire`.
Never allocate in the ISR. Never call into the scheduler from the ISR.

### Step 4 — Write the ISR

```rust
#[no_mangle]
pub unsafe extern "C" fn xhci_isr() {
    // 1. Acknowledge at the device first (clear IP bit, EHB on ERDP write).
    let iman = mmio::read32(rt_base + RT_IR0_IMAN);
    mmio::write32(rt_base + RT_IR0_IMAN, iman | 0x01); // W1C IP

    // 2. Drain into the completion table. drain_events_into bumps ERDP.
    XHCI.lock_isr_safe().drain_events_into(&XHCI_TABLE);

    // 3. EOI the LAPIC last so a re-entrant interrupt does not preempt ack.
    crate::cpu::apic::lapic_eoi();
}
```

Use `IsrSafeRawSpinLock` (saves IF per-core) for the controller lock the ISR shares
with thread context. Plain `SpinLock` deadlocks if the ISR fires while the same
core holds the lock.

### Step 5 — Convert the waiter to HLT-on-wait

The old polling waiter:

```rust
loop {
    if let Some(c) = self.evt_ring.peek() { ... return; }
    if deadline_exceeded() { return Err(_); }
    core::hint::spin_loop();
}
```

becomes:

```rust
loop {
    // Drain in-thread too — catches anything posted between Step 4's drain and
    // our enable of interrupts below. Same invariant as the audit's
    // usb_event_ring_drain_invariant memory.
    self.drain_events_into(&XHCI_TABLE);
    if let Some(c) = XHCI_TABLE.take_cmd() { return Ok(c); }
    if deadline_exceeded() { return Err(XhciError::CommandTimeout); }
    // Re-enable IF, halt, disable IF — wakes on the next interrupt of any source.
    core::arch::asm!("sti; hlt; cli", options(nostack, nomem));
}
```

`hlt` wakes on *any* interrupt (timer, IPI, our device), so the loop retries fast
on spurious wakes. Cost is one extra table check per wake, not a full poll.

## Critical invariants

1. **The drain invariant.** `wait_*` must advance past *every* event, not just
   matching ones. Intel xHCI silicon posts `PSCEC` events interleaved with
   completions; dropping them breaks port enumeration on real hardware. (See
   memory `usb_event_ring_drain_invariant` and `usb_xhci_real_hw_quirks`.)
2. **EOI ordering.** Acknowledge at the device first (clear IP), then drain, then
   `lapic_eoi`. Reversing this loses edge-triggered MSIs.
3. **ERDP discipline.** Every time you advance the event-ring dequeue pointer,
   write `ERDP` with the EHB bit (0x08) set so the controller re-arms the
   interrupt. (Same as the existing `update_erdp` helper.)
4. **No allocation in the ISR.** No `Vec`, no `Box`, no `Mutex` that could sleep.
   Pre-allocate the completion table at init.
5. **`IsrSafeRawSpinLock` for shared state.** A plain `SpinLock` deadlocks if the
   ISR pre-empts the same core mid-critical-section.
6. **Keep polling as a fallback.** If MSI/MSI-X programming fails (no capability,
   firmware quirk), fall back to the original busy-wait. Don't fail driver init.

## Verification checklist

- [ ] ISR fires (counter > 0 after one device operation) — proves MSI delivery
- [ ] Polling fallback path still works when MSI is disabled at init
- [ ] Average CPU during the driver's hot path drops (TSC-active percentage)
- [ ] Existing integration tests still pass under QEMU
- [ ] Real-hardware boot reaches the post-USB checkpoint without a triple-fault
      (look at serial output for the existing `checkpoint(...)` markers)
- [ ] No new `unsafe` block without a `// SAFETY:` comment
- [ ] `cargo clippy --target x86_64-unknown-uefi` clean

## Common mistakes

- **Forgetting the in-thread drain in the waiter.** Without it, an event that
  arrives between the ISR's last drain and the waiter's `hlt` is missed for one
  timer tick (10 ms at 100 Hz).
- **Using `Ordering::Relaxed` on the `ready` flag.** The consumer must observe
  the status/ctrl writes that the ISR did before it set `ready`. Use `Release` on
  set, `Acquire` on read.
- **Holding a plain spinlock across `hlt`.** Always release locks before
  `sti; hlt; cli`.
- **EOI before device ack.** Loses subsequent interrupts.
- **Using line interrupts via IOAPIC instead of MSI/MSI-X.** This codebase has no
  IOAPIC routing. MSI/MSI-X is the path.

## References

- `docs/polling-loop-audit.md` — the audit this skill was built from
- `docs/interrupt-refactor-plan.md` — phased rollout
- xHCI Specification 1.2 §4.17 (Interrupters), §5.5.2 (Interrupter Register Set)
- AMD64 Architecture Manual Vol 2, §16.4 (Local APIC)
- Project memory: `usb_event_ring_drain_invariant`, `usb_xhci_real_hw_quirks`
