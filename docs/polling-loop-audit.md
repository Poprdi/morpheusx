# Polling Loop Audit Report

**Codebase:** MorpheusX exokernel (bare-metal x86_64 Rust, `no_std`)
**Date:** 2026-05-24
**Branch:** `hardware-bringup`
**Scope:** `hwinit/`, `core/`, `helix/`, `network/`, `bootloader/`, `display/`

---

## Summary

- **Total polling/busy-wait loops found:** ~85
- **Critical/architectural concerns:** 3
- **High-impact runtime polling:** 4 (AHCI cmd, xHCI event ring, network main loop, USB MSC event ring)
- **Medium-impact init polling:** ~30 (TSC-bounded device init/reset waits)
- **Low-impact / correct (HLT-based or CAS):** ~25
- **Already interrupt-driven:** scheduler tick, AP idle, syscall stdin/input read, kernel input forward loop

Headline result: **the scheduler is fully interrupt-driven** (LAPIC vector `0x20` → `irq_timer_isr` → `scheduler_tick`). The big remaining polling targets are **AHCI command completion** (intentionally polled — design choice), the **xHCI event ring** (no ISR wired despite `IMAN.IE = 1`), and the **network state-machine main loop** (acceptable as the top-level driver).

---

## Critical Findings

### C1. PIT calibration spin has no timeout
`hwinit/src/cpu/apic.rs:298-301` — LAPIC timer calibration polls PIT channel-2 gate bit (`port 0x61 & 0x20`) in a loop with no timeout and no iteration cap. If the PIT is broken, masked, or absent (some modern boards remove channel 2), the BSP hangs forever during boot.

```rust
crate::serial::checkpoint("lapic-pit-spin");
while crate::cpu::pio::inb(0x61) & 0x20 == 0 {
    core::hint::spin_loop();
}
```

The serial checkpoint is the only diagnostic — there is no recovery. This is the single most dangerous loop in the kernel for real-hardware bring-up.

### C2. xHCI is wired for interrupts but no ISR exists
Both `hwinit/src/usb/controller.rs:201-203` and `network/src/driver/usb_msd/mod.rs:671` set `IMAN.IE = 0x02` on Interrupter 0, and `ERSTBA`/`ERDP`/`ERSTSZ` are all configured properly. But **no IDT vector is installed** for the xHCI MSI/MSI-X line, so the controller silently raises interrupts that are never delivered to a CPU handler. Events are caught only because the same code polls the event ring (`evt_ring.peek()`) on every `wait_cmd`/`wait_xfer` call. The infrastructure is one ISR registration away from being interrupt-driven.

### C3. AHCI deliberately disables interrupts and polls forever
`network/src/driver/ahci/mod.rs:374-378` has an explicit comment:

```rust
// STEP 3: Disable interrupts (we use polling)
asm_ahci_disable_interrupts(abar);
```

…followed by per-port `asm_ahci_port_disable_interrupts` at line 491. Every command (read/write/IDENTIFY/flush) then busy-waits on `PXIS` in assembly (`network/asm/drivers/ahci/cmd.s:308-445`) — for `flush` the timeout is **30 seconds**. There is no interrupt path at all, and the assembly poll lacks `pause`/`spin_loop` hints. This is design choice, not omission, but it caps throughput and burns power.

---

## Detailed Loop Inventory

### Group A — Scheduler / process management (interrupt-driven, OK)

| # | Location | Pattern | Impact | Assessment |
|---|---|---|---|---|
| A1 | `hwinit/src/process/schedular/tick.rs:27-30` | `loop { sti; hlt; cli }` (AP idle) | runs forever per AP | ✓ Correct — woken by `scheduler_tick` IRQ (vector `0x20`) |
| A2 | `hwinit/src/process/schedular/tick.rs:37-39` | `loop { hlt }` (AP shutdown quiesce) | end-of-life | ✓ Correct |
| A3 | `hwinit/src/process/schedular/wait.rs:15-32` | CAS on `EARLIEST_DEADLINE` | rare contention | ✓ Lock-free, bounded |
| A4 | `hwinit/src/process/schedular/wait.rs:43-88` | `wait_for_child` `sti;hlt;cli` loop | blocked waiter | ✓ Correct |
| A5 | `hwinit/src/process/schedular/lifecycle.rs:156-159` | `loop { hlt }` (exit_process) | post-exit | ✓ Correct |
| A6 | `hwinit/src/cpu/ap_boot.rs:389-392` | `loop { hlt }` (AP post-online) | one per AP | ✓ Correct |
| A7 | `hwinit/src/syscall/handler/core.rs:166-187` | Composited input read `sti;hlt;cli` | per syscall | ✓ Correct |
| A8 | `hwinit/src/syscall/handler/core.rs:191-210` | stdin read `sti;hlt;cli` | per syscall | ✓ Correct |
| A9 | `hwinit/src/input.rs:243-271` | Mouse event drain (`break` on `None`) | per tick from ISR | ✓ Bounded drain |

### Group B — Synchronization primitives (CAS / spin)

| # | Location | Pattern | Impact | Assessment |
|---|---|---|---|---|
| B1 | `hwinit/src/sync.rs:42-61` | `SpinLock::lock` CAS | every lock | ✓ Disables IF, no ISR contention |
| B2 | `hwinit/src/sync.rs:144-152` | `RawSpinLock::lock` | every lock | ✓ Short crit sections only |
| B3 | `hwinit/src/sync.rs:204-221` | `IsrSafeRawSpinLock::lock` | ISR-safe | ✓ Per-core saved IF |
| B4 | `hwinit/src/sync.rs:275-299` | `Once::call_once` wait | init only | ✓ Bounded by initializer |
| B5 | `hwinit/src/serial.rs:57-81` | `BOOT_LOG_LEN` CAS | every log line | ✓ Bounded by core count |
| B6 | `hwinit/src/syscall/handler/sync.rs:51-60` | Futex deadline CAS | futex ops | ✓ Lock-free |

### Group C — UART / serial output (polling, bounded)

| # | Location | Pattern | Impact | Assessment |
|---|---|---|---|---|
| C-S1 | `hwinit/src/serial.rs:99-121` | UART TX-empty poll, 100-iter cap | every byte | ⚠ Hot path; cap is loose |
| C-S2 | `network/src/lib.rs:170-194` | Same UART poll, 100-iter cap | every byte | ⚠ Duplicate of C-S1 |
| C-S3 | `network/src/mainloop/serial.rs:16-31` | Same pattern | every byte | ⚠ Triplicate |

Three near-identical UART polling implementations — candidate for consolidation. Replacing with FIFO + threshold IRQ on COM1 vector would eliminate the per-byte poll.

### Group D — APIC / CPU init (TSC-bounded except PIT)

| # | Location | Pattern | Timeout | Assessment |
|---|---|---|---|---|
| D1 | `hwinit/src/cpu/apic.rs:298-301` | **PIT gate poll** | **NONE** | **🔥 Critical — see C1** |
| D2 | `hwinit/src/cpu/apic.rs:415-432` | ICR delivery-status poll | 10 000 iter | ✓ Bounded |
| D3 | `hwinit/src/cpu/apic.rs:439-478` | `delay_us` busy-wait + watchdog | TSC | ✓ Bounded |
| D4 | `hwinit/src/cpu/per_cpu.rs:389-407` | AP shutdown quiesce wait | TSC + fallback | ✓ Dual timeout |
| D5 | `hwinit/src/cpu/reset.rs:16-23` | KBC input-empty wait | 100 000 iter | ✓ Bounded |
| D6 | `hwinit/src/cpu/reset.rs:122-137` | Keypress / shutdown wait | TSC (10 s) | ✓ Bounded |

### Group E — xHCI controller (event ring polling)

| # | Location | Pattern | Timeout | Assessment |
|---|---|---|---|---|
| E1 | `hwinit/src/usb/controller.rs:141-150` | Controller halt (`STS_HCH` set) | 1 s | ✓ Init only |
| E2 | `hwinit/src/usb/controller.rs:211-220` | Controller start (`STS_HCH` clear) | 1 s | ✓ Init only |
| E3 | `hwinit/src/usb/controller.rs:297-306` | `tsc_delay` ms busy-wait | parametric | ✓ Calibration helper |
| E4 | `hwinit/src/usb/controller.rs:358-378` | **`wait_cmd` event-ring drain** | 2–5 s | ⚠ See C2 — runtime path |
| E5 | `hwinit/src/usb/controller.rs:394-414` | **`wait_xfer` event-ring drain** | 5–10 s | ⚠ See C2 — runtime path |
| E6 | `hwinit/src/usb/controller.rs:456-469` | Port reset `PR` clear | 200 ms | ✓ Init only |
| E7 | `hwinit/src/usb/controller.rs:473-487` | Port link settle (`PED`/`CCS`) | 200 ms | ✓ Init only |

### Group F — USB MSC (`network/src/driver/usb_msd/mod.rs`)

Mirror of Group E plus extras for runtime mass-storage I/O. All TSC-bounded.

| # | Location | Pattern | Timeout |
|---|---|---|---|
| F1 | `680-695` | Controller start wait | 1 s |
| F2 | `944-955` | USB 3.0 speed poll | 100 ms |
| F3 | `1080-1089` | Link state → U0 | 100 ms |
| F4 | `1101-1111` | Hot reset command phase | 125 ms |
| F5 | `1133-1161` | Hot reset settle | 200 ms |
| F6 | `1176-1197` | Warm reset settle | 200 ms |
| F7 | `1212-1222` | Late speed recovery | 100 ms |
| F8 | `798-872` | **Enumeration scan rounds** | 8 s outer |
| F9 | `1485-1515` | **Event-ring drain (unified)** | 2–10 s |
| F10 | `335-344` | `tsc_delay` busy-wait helper | parametric |

### Group G — AHCI (polling by design)

| # | Location | Pattern | Timeout | Assessment |
|---|---|---|---|---|
| G1 | `network/src/driver/ahci/mod.rs:222-232` | BIOS handoff phase 1 (BOS) | 25 ms | ✓ Init |
| G2 | `network/src/driver/ahci/mod.rs:237-246` | BIOS handoff phase 2 (BB) | 2 s | ✓ Init, T450s-specific |
| G3 | `network/src/driver/ahci/mod.rs:374-378` | **Disable interrupts** | n/a | 🔥 See C3 |
| G4 | `network/src/driver/ahci/mod.rs:428-444` | Port detect (per port) | 200 ms | ✓ Bounded but expensive (200 ms × 32 = 6.4 s worst case) |
| G5 | `network/src/driver/ahci/mod.rs:495-504` | Link settle | 50 ms | ✓ Init |
| G6 | `network/asm/drivers/ahci/cmd.s:308-445` | **`asm_ahci_poll_cmd` PXIS poll** | 30 s for flush | 🔥 Runtime hot path; no `pause` hint |
| G7 | `network/src/driver/ahci/mod.rs:491` | Per-port disable IRQ | n/a | 🔥 Cements polling |

### Group H — VirtIO / Intel NIC / SDHCI

| # | Location | Pattern | Timeout | Assessment |
|---|---|---|---|---|
| H1 | `network/src/driver/virtio/tx.rs:100-114` | TX completion drain | bounded by ring | ✓ |
| H2 | `network/src/driver/virtio_blk.rs:255-260` | VirtIO reset wait | 1 M iter | ✓ |
| H3 | `network/src/driver/intel/init.rs:192-204` | e1000e RX/TX quiesce | 10 ms | ✓ Init |
| H4 | `network/src/driver/intel/init.rs:222-232` | GIO master disable | 10 ms | ✓ Init |
| H5 | `network/src/driver/intel/init.rs:254-264` | EEPROM auto-read | 500 ms | ✓ Init |
| H6 | `network/src/driver/intel/init.rs:436-437,519,596,627,634,651` | Various PHY/ULP delays | parametric | ✓ Init |
| H7 | `network/src/driver/sdhci/mod.rs:110-119` | Inhibit clear | parametric | ✓ Runtime, short |
| H8 | `network/src/driver/sdhci/mod.rs:142-157` | Command complete | parametric | ⚠ Could be IRQ-driven |
| H9 | `network/src/driver/sdhci/mod.rs:175-179` | ACMD41 power-up | 2 s | ✓ Init |

### Group I — Block I/O wrappers

| # | Location | Pattern | Timeout | Assessment |
|---|---|---|---|---|
| I1 | `network/src/driver/block_io_adapter.rs:122-142` | Request-id completion poll | parametric | ⚠ Runtime |
| I2 | `network/src/driver/unified_block_io.rs:156-176` | Same | parametric | ⚠ Runtime |
| I3 | `network/src/driver/unified_block_io.rs:397-414` | Generic adapter variant | parametric | ⚠ Runtime |
| I4 | `network/src/driver/unified_block_io.rs:194,233,431,465` | Pre-op completion drain | unbounded but breaks on `None` | ✓ Drain |
| I5 | `network/src/mainloop/disk_writer.rs:133-156` | Per-block write wait | ~1 s | ⚠ Runtime |
| I6 | `network/src/mainloop/states/manifest.rs:486-497` | Manifest write wait | 500 ms | ✓ One-shot |

### Group J — Network state machine (`network/src/`)

| # | Location | Pattern | Timeout | Assessment |
|---|---|---|---|---|
| J1 | `network/src/mainloop/orchestrator.rs:144-191` | **Top-level state-machine loop** | until Done/Failed | ⚠ Spins on `Continue`; acceptable as driver but `hlt`-on-idle would save power |
| J2 | `network/src/client/native.rs:195-203` | Wait for IP/DHCP | parametric | ✓ 1 ms backoff |
| J3 | `network/src/client/native.rs:246-264` | DNS resolution | 5 s | ✓ 1 ms backoff |
| J4 | `network/src/client/native.rs:337-352` | TCP connect | parametric | ✓ 1 ms backoff |
| J5 | `network/src/client/native.rs:367-380` | TCP send | parametric | ✓ 100 µs backoff |
| J6 | `network/src/client/native.rs:390-407` | TCP recv | parametric | ✓ 1 ms backoff |
| J7 | `network/src/mainloop/states/done.rs:53-86` | Reboot wait → `hlt` | bounded + HLT | ✓ Terminal |
| J8 | `network/src/device/pci.rs:252-254` | `tsc_delay_us` primitive | parametric | ✓ Helper |

### Group K — Bootloader / TUI

| # | Location | Pattern | Timeout | Assessment |
|---|---|---|---|---|
| K1 | `bootloader/src/boot.rs:1070-1107` | **Kernel input forward loop** (`sti;hlt;cli` when idle) | infinite | ✓ Correct interrupt-driven idle |
| K2 | `bootloader/src/boot.rs:1145-1150` | `halt_forever` | fatal | ✓ |
| K3 | `bootloader/src/main.rs:132-136` | Panic halt | fatal | ✓ |
| K4 | `bootloader/src/tui/input.rs:480-493` | `wait_kbd_byte` bounded spin | `max_spins` | ✓ Init |
| K5 | `bootloader/src/tui/input.rs:496-502` | Drain all PS/2 | bounded | ✓ Init |
| K6 | `bootloader/src/tui/input.rs:543-567` | **`wait_for_key` (menu)** | infinite outer + 4096 inner | ⚠ Polling backoff; could use `hlt` like K1 |
| K7 | `bootloader/src/tui/input.rs:586-601` | `poll_key_with_delay` (16 ms frame, HLT) | TSC | ✓ HLT-paced |
| K8 | `bootloader/src/tui/mouse.rs:188-231` | 4 mouse-init waits | bounded | ✓ Init |
| K9 | `bootloader/src/tui/main_menu.rs:270-306` | Menu render loop | infinite (until action) | ✓ Delegates to K7 |
| K10 | `bootloader/src/tui/rain.rs:156-172` | Animation pacing (30 ms HLT) | TSC | ✓ HLT-paced |

### Group L — Filesystem walks (bounded iterators, not polling)

| # | Location | Pattern | Assessment |
|---|---|---|---|
| L1 | `core/log/mod.rs:374-389, 422-505` | Log record iteration | ✓ Bounded by CRC |
| L2 | `core/log/mod.rs:316-327` | Block-spanning payload read | ✓ Bounded |
| L3 | `core/src/fs/fat32_ops/file_ops.rs:320-348` | FAT chain walk | ✓ Bounded by EOC; pathological for highly fragmented files |
| L4 | `core/src/disk/gpt_ops/scan.rs:60-81` | GPT entry iter | ✓ |
| L5 | `core/src/iso/writer.rs:180-236` | ISO chunk write | ✓ |
| L6 | `helix/src/log/mod.rs:446-537` | Segment scan (recovery) | ✓ Bounded by segment count |
| L7 | `helix/src/log/mod.rs:490-530` | Inner record iter | ✓ |
| L8 | `helix/src/log/mod.rs:342-356` | Payload block read | ✓ |
| L9 | `helix/src/ops/read.rs:84-158` | Read-at-LSN walk | ✓ |

These are algorithmic loops over data, not polling. Listed for completeness.

### Group M — Display

| # | Location | Pattern | Assessment |
|---|---|---|---|
| M1 | `display/src/console.rs:151-154` | Tab-stop expansion | ✓ Bounded |

No framebuffer or VSYNC polling — the display subsystem is fully passive.

---

## Architectural Blockers

1. **xHCI MSI/MSI-X never enabled.** Despite `IMAN.IE = 1` and complete event-ring setup, the driver never:
   - Probes MSI/MSI-X capability in PCI config
   - Calls `set_interrupt_handler(vec, xhci_isr, ...)`
   - Configures `MSI Message Address/Data` to point at a free IDT vector
   The polling path in `wait_cmd`/`wait_xfer` must be retained as a fallback (and on real Intel silicon, `PSCEC` events arrive interleaved with command completions — the drain logic is already correct), but adding an ISR would let other work proceed while waiting and would eliminate the 8-second enumeration scan loop (Group F8).

2. **AHCI explicitly disables interrupts.** This is a deliberate decision (commented in source). The `asm_ahci_poll_cmd` assembly busy-loops on `PXIS` with no `pause`, which is wasteful even when polling is desired. Migrating to MSI-X would require: re-enable `GHC.IE`, re-enable `PXIE` per port, install ISR, and route completion through a queue read by the requesting thread.

3. **Three duplicated UART polling routines.** `hwinit/src/serial.rs`, `network/src/lib.rs`, and `network/src/mainloop/serial.rs` all reimplement the same `LSR & 0x20` poll. Until consolidated, any IRQ-based serial transition has to be made three times.

4. **HID runtime polling is missing entirely.** `hwinit/src/usb/hid/keyboard.rs:201` and `mouse.rs:88` have `// TODO: Implement proper interrupt transfer handling`. There is no driver loop that polls the interrupt endpoint after enumeration — USB HID is enumerated but not driven. Adding a periodic poll (driven by the scheduler tick) or, better, an xHCI ISR + transfer-event dispatcher is required before USB HID actually produces input events.

5. **No HPET driver.** Only the LAPIC timer is used. HPET would give a higher-resolution wall clock without polling the TSC.

---

## Recommended Refactoring Priority

### 1. Add PIT calibration timeout (low effort, removes a boot-time hang)
File: `hwinit/src/cpu/apic.rs:298-301`. Add a TSC-deadline (e.g., 100 ms) around the PIT spin, and fall back to a cached/CPUID-derived LAPIC frequency if it trips. This converts a hard hang into a recoverable warning.

```rust
let pit_deadline = tsc::read_tsc().wrapping_add(tsc_freq / 10); // 100 ms
while crate::cpu::pio::inb(0x61) & 0x20 == 0 {
    if tsc::read_tsc() >= pit_deadline {
        log_warn("LAPIC", 999, "PIT calibration timed out");
        return;
    }
    core::hint::spin_loop();
}
```

### 2. Wire xHCI MSI-X to an event-ring ISR (highest impact for USB)
Single ISR drains the event ring into a per-slot completion table; `wait_cmd`/`wait_xfer` then read from the table instead of polling MMIO. This kills the 8-second enumeration scan window (Group F8) and lets the kernel `hlt` while USB I/O is in flight. The work needed:
- Add MSI/MSI-X capability parser to `hwinit/src/pci/capability.rs` (if not already present)
- Allocate a free IDT vector, `set_interrupt_handler(vec, xhci_isr, 0, 0)`
- Write MSI message addr/data so the controller targets that vector
- Keep the existing event-ring drain code; just call it from the ISR plus from `wait_*` as a fallback

### 3. Consolidate the three UART polling routines
Move all callers to `hwinit::serial::putc_raw`. Then a future "enable COM1 receive IRQ" change is a single edit.

### 4. Implement HID interrupt-transfer dispatch
Until this lands, no USB keyboard or mouse will produce input. The minimum viable path is: schedule a periodic interrupt-IN transfer from a kernel thread tied to the scheduler tick. The interrupt-driven version (item 2) makes this trivial.

### 5. AHCI MSI-X (optional)
Re-enabling AHCI interrupts is a larger architectural change because the current `BlockDriver::flush` path expects synchronous completion. Worth doing once item 2 proves the pattern.

---

## Interrupt Infrastructure Gaps

**Present and working:**
- IDT with all 22 CPU exception vectors (`hwinit/src/cpu/idt.rs:836-873`)
- LAPIC timer at vector `0x20` → `irq_timer_isr` → `scheduler_tick` (`hwinit/src/platform.rs:557`)
- AP IPIs (used for shutdown)
- 8259 PIC is fully masked after switch to LAPIC

**Missing / unconfigured:**
- No IOAPIC routing for any device line interrupt
- No MSI/MSI-X enablement for any PCI device (xHCI, AHCI, VirtIO, e1000e)
- No HPET driver
- No PS/2 keyboard/mouse IRQ (vector 0x21/0x2C) — bootloader TUI polls
- No COM1 receive IRQ — serial is TX-only and polled
- No xHCI ISR (despite `IMAN.IE` being set)
- No AHCI ISR (intentionally disabled)
- No VirtIO MSI-X (RX/TX/cfg vectors not programmed)

---

## Proof-of-Concept Suggestion

**Quick win: xHCI MSI-X with a stub ISR that just records events.**

1. In `hwinit/src/pci/capability.rs`, add `find_msix(bdf) -> Option<MsixCapability>` that walks the capability list looking for ID `0x11`.
2. In `XhciController::init`, after `IMAN.IE` is set:
   ```rust
   if let Some(msix) = pci::find_msix(self.bdf) {
       let vec = 0x40; // pick a free vector
       idt::set_interrupt_handler(vec, xhci_isr as u64, 0, 0);
       msix.program_entry(0, lapic_addr(), vec, /*masked=*/false);
       msix.enable();
   }
   ```
3. Define `xhci_isr`:
   ```rust
   #[no_mangle]
   pub unsafe extern "C" fn xhci_isr() {
       XHCI_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
       // Don't actually drain here yet — let polling continue.
       // Just acknowledge:
       let iman = mmio::read32(rt_base + RT_IR0_IMAN);
       mmio::write32(rt_base + RT_IR0_IMAN, iman | 0x01); // W1C IP bit
       lapic_eoi();
   }
   ```
4. Boot with QEMU's xHCI (`-device qemu-xhci`) and inspect `XHCI_EVENT_COUNT` from a debug syscall. Once events are observed firing, evolve the ISR to wake a per-slot waitqueue and have `wait_cmd`/`wait_xfer` `hlt` instead of polling.

This template — `pci::find_msix` → `set_interrupt_handler` → MSI program → minimal ISR — generalizes to VirtIO, e1000e, and (eventually) AHCI.

---

## Answers to the Audit Questions

1. **Single biggest CPU consumer right now:** `asm_ahci_poll_cmd` (`network/asm/drivers/ahci/cmd.s`) — pure spin on `PXIS` with no `pause` hint, on every block I/O, up to 30-second timeout on flush.
2. **Three highest-impact refactor candidates:** (a) xHCI MSI-X ISR, (b) AHCI MSI-X / interrupt re-enable, (c) PIT calibration timeout fix.
3. **What interrupt infrastructure is missing:** MSI/MSI-X capability parsing, IOAPIC routing, any device-line ISR registration. `set_interrupt_handler` exists and is wired only for `0x20` (timer).
4. **Is the 100 Hz scheduler tick polling or timer-based?** **Timer-based.** LAPIC timer programmed in periodic mode at `TIMER_VECTOR = 0x20` (`hwinit/src/cpu/apic.rs:331`); `platform.rs:557` installs `irq_timer_isr` at that vector; the ISR calls `scheduler_tick` (`hwinit/src/process/schedular/tick.rs:69`).
5. **Do all devices that support interrupts have them enabled?** No. **None do.** xHCI has the event-ring side configured but no ISR; AHCI explicitly disables; VirtIO/Intel-NIC/SDHCI have no MSI-X programming.
6. **Polling workarounds for hardware quirks:** Yes —
   - `wait_cmd`/`wait_xfer` drain PSCEC events that real Intel xHCI silicon interleaves with completions (documented in project memory as `usb_event_ring_drain_invariant`).
   - AHCI BIOS handoff phase 2 (`network/src/driver/ahci/mod.rs:237-246`) tolerates a 2-second BIOS busy hang on Intel PCH.
   - USB MSC late-speed-recovery (`network/src/driver/usb_msd/mod.rs:1212-1222`) re-polls speed bits after some SS controllers report them late.
7. **Critical path that must remain polling-based:** UART TX in `serial::putc_raw` during panic / early boot — interrupts may not be available. Spinlock CAS loops. The scheduler tick itself runs in interrupt context and is non-blocking by design. PIT calibration must spin because no other time source is available at that moment (but should still be bounded).
