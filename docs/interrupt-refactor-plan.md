# Interrupt-Driven Refactor Plan

**Source:** `docs/polling-loop-audit.md`
**Goal:** Eliminate the three critical polling issues and lay infrastructure for incremental device-by-device migration from polling to MSI/MSI-X.
**Non-goal:** Rewriting drivers that are already polling-only by intent — those move only after the infrastructure exists.

Each phase is independently shippable, atomically reversible, and gated by a clear acceptance test.

---

## Phase 0 — Safety patches (1 PR, ~1 day)

Bound the loops that can hang the kernel today. No new infrastructure.

**Work**
1. `hwinit/src/cpu/apic.rs:298-301` — wrap the PIT spin in a 100 ms TSC deadline; on timeout, `log_warn` and fall through using a CPUID-derived or cached LAPIC frequency.
2. `hwinit/src/cpu/apic.rs` — extract the calibration body into a helper so the fallback path is single-sourced.

**Acceptance**
- Boot under QEMU with `-machine pcspk-audiodev=none -no-pit` (or equivalent) — kernel does not hang; warning is logged.
- Normal QEMU boot still calibrates within 10 ms of the previous baseline (compare `LAPIC_TIMER_INIT_COUNT`).

**Risk:** Low. Affects only the boot path; fallback path is a printf + continue.

---

## Phase 1 — PCI MSI/MSI-X capability layer (1 PR, ~2 days)

Add the missing primitive that every later phase depends on. No driver changes yet.

**Work**
1. `hwinit/src/pci/capability.rs` — add capability-list walker (read `STATUS.CAP_LIST`, follow chain at offset `0x34`, dedupe).
2. New file `hwinit/src/pci/msi.rs` — types and helpers:
   - `MsiCapability` (16/64-bit message addr, multi-message enable)
   - `MsixCapability` (table BAR/offset, PBA BAR/offset, table size, function mask)
   - `find_msi(bdf) -> Option<MsiCapability>`, `find_msix(bdf) -> Option<MsixCapability>`
   - `MsixCapability::program_entry(idx, vector, lapic_addr, masked)`, `mask_all`, `enable`
3. Wire `pci/mod.rs` to expose `find_msi`/`find_msix`.
4. Unit test on `lspci -vv` golden output if practical (target dev only).

**Acceptance**
- `pci::find_msix(xhci_bdf)` returns `Some(cap)` on QEMU `qemu-xhci` and on the target T450s xHCI controller (verified via `hwinit/src/pci/dump.rs` print).
- `program_entry` writes survive a read-back (verify in QEMU monitor).

**Risk:** Low — read/write to PCI config space is already well exercised.

---

## Phase 2 — xHCI MSI-X proof of concept (1 PR, ~2 days)

Wire one device through the new infrastructure. Keep polling as fallback.

**Work**
1. `hwinit/src/cpu/idt.rs` — reserve vector `0x40` for xHCI (constant `XHCI_VECTOR`).
2. `hwinit/src/usb/controller.rs`:
   - In `init`, after `IMAN.IE`: call `pci::find_msix`, allocate vector, `set_interrupt_handler(XHCI_VECTOR, xhci_isr, 0, 0)`, program MSI-X entry 0 with LAPIC physical address + vector + unmasked, `cap.enable()`.
3. New `xhci_isr` (minimal):
   ```rust
   #[no_mangle]
   pub unsafe extern "C" fn xhci_isr() {
       XHCI_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
       let iman = mmio::read32(rt_base + RT_IR0_IMAN);
       mmio::write32(rt_base + RT_IR0_IMAN, iman | 0x01); // W1C IP
       lapic_eoi();
   }
   ```
4. Leave `wait_cmd`/`wait_xfer` polling intact — this PR only proves events fire.

**Acceptance**
- Counter strictly > 0 after a USB enumeration round under QEMU.
- On real T450s, counter > 0 after attaching a HID device.
- No regression in existing USB enumeration tests.

**Risk:** Low. ISR is a stub; polling fallback preserves behavior.

---

## Phase 3 — xHCI completion table + HLT wait (1 PR, ~3 days)

Convert `wait_cmd` / `wait_xfer` to interrupt-driven without breaking the drain invariant.

**Work**
1. New `XhciCompletion` table keyed by `(slot_id, ep_dci)` and command-ring-pointer for command events. Each slot has an `AtomicBool` ready flag + `(status, ctrl)` payload.
2. Move the body of the existing drain loop into a `drain_events_into(&mut self, table)` helper. Call it from:
   - `xhci_isr` (preferred path)
   - `wait_cmd`/`wait_xfer` start (catches anything posted before HLT)
3. `wait_cmd`/`wait_xfer`:
   ```rust
   loop {
       self.drain_events_into(table);
       if let Some(c) = table.take_cmd() { return Ok(c); }
       if deadline_exceeded() { return Err(_); }
       core::arch::asm!("sti; hlt; cli", options(nostack, nomem));
   }
   ```
4. Preserve the existing PSCEC-drain semantics (per the `usb_event_ring_drain_invariant` memory).

**Acceptance**
- `wait_cmd`/`wait_xfer` no longer call `spin_loop()` in the steady-state path.
- Under QEMU, average CPU utilization during USB enumeration drops noticeably (measure via TSC-active percentage exposed by the scheduler).
- Real-hw enumeration still completes within the same wall-clock window as before.

**Risk:** Medium. Race between ISR draining and the in-flight thread checking the table needs careful ordering — use `AcqRel` on the ready flag, drain under `IsrSafeRawSpinLock`.

---

## Phase 4 — USB HID interrupt-transfer dispatch (1 PR, ~2 days)

Unblocks USB keyboard/mouse, which the audit identified as currently non-functional (`hwinit/src/usb/hid/keyboard.rs:201`, `mouse.rs:88`).

**Work**
1. After Phase 3 lands, add a kernel thread (or scheduler-tick callback) that:
   - For each enumerated HID device, posts an interrupt-IN transfer if none outstanding.
   - On completion (via the table from Phase 3), parses the 8-byte boot report and feeds `crate::input::push_*`.
2. Use existing `usb_hid_class` boot-protocol layout.

**Acceptance**
- Plugging a USB keyboard into QEMU produces keystrokes in the kernel input layer.
- Same on real T450s.

**Risk:** Low once Phase 3 exists.

---

## Phase 5 — UART consolidation + RX IRQ (optional, ~1 day)

Cleanup that becomes valuable once we have an ISR pattern proven.

**Work**
1. Delete `network/src/lib.rs:170-194` and `network/src/mainloop/serial.rs:16-31`; route both to `hwinit::serial::putc_raw`.
2. Optionally: enable COM1 RX IRQ (vector `0x24`), register handler, push into a `stdin` ring buffer. Lets debug shell receive input without polling.

**Acceptance:** existing serial tests pass; single owner of UART code.

**Risk:** Low.

---

## Phase 6 — AHCI MSI-X (separate effort, ~1 week)

Larger because `BlockDriver::flush` currently assumes synchronous completion. Tackle only after Phases 1–3 prove the pattern.

**Work outline (not yet detailed)**
1. Re-enable `GHC.IE` and per-port `PXIE` selectively.
2. Reuse Phase 1 MSI-X primitives to route AHCI vector.
3. Per-port completion queue; `asm_ahci_poll_cmd` becomes `wait_ahci_cmd` that consults the queue and HLTs.
4. Replace the unhinted assembly spin (`network/asm/drivers/ahci/cmd.s:308-445`) with a Rust path; keep assembly only for the actual MMIO accessors.

**Acceptance:** sequential block I/O throughput improves measurably; CPU utilization during flush drops to near zero.

**Risk:** Medium-high. Touches the hot block-I/O path; needs benchmark + crash-recovery validation.

---

## Phase 7 — VirtIO / e1000e MSI-X (incremental, as needed)

Same pattern as AHCI but per-device. Not prioritized; the network state machine is acceptable as-is until contention with USB/storage emerges.

---

## Out of scope (explicitly)

- IOAPIC routing for legacy line interrupts — MSI/MSI-X obviates it for the devices we care about.
- HPET driver — LAPIC timer is sufficient.
- Removing the network orchestrator's top-level `loop {}` — it's the state machine itself; an `sti;hlt;cli` on `StepResult::Continue` would be a one-line power win but isn't urgent.
- AP scheduler — already interrupt-driven (LAPIC vector `0x20`); no work needed.

---

## Cross-cutting acceptance gates

Every phase must clear, before merge:

- `cargo fmt`, `cargo clippy --target x86_64-unknown-uefi`, `cargo build --release --target x86_64-unknown-uefi`
- No new `unsafe` block without a `SAFETY:` comment
- Boots in QEMU with OVMF (regression: existing tests)
- For any phase that touches ISR code: boot once on the T450s and confirm no triple-fault (serial checkpoint after the new ISR fires)

## Sequencing

```
Phase 0 ──┐
Phase 1 ──┴─→ Phase 2 ──→ Phase 3 ──→ Phase 4
                                  └──→ Phase 6 (AHCI) ──→ Phase 7 (VirtIO/NIC)
Phase 5 (independent, any time)
```
