---
name: pci-msi-programming
description: |
  Discover and program PCI MSI and MSI-X capabilities for x86_64 bare-metal
  drivers. Walks the PCI capability list, decodes MSI/MSI-X structures, and
  programs message address/data so a device's interrupts land on a chosen IDT
  vector via the LAPIC.
author: MorpheusX Architecture Team
version: 2026.1
---

## When to use

You need a PCI device (xHCI, AHCI, VirtIO, e1000e, NVMe, …) to deliver
interrupts to a CPU vector you control. This skill covers discovery and
programming; the `interrupt-driven-refactor` skill covers what to do with the
events once they arrive.

Do **not** use this skill for:

- Devices that have no MSI/MSI-X capability (legacy PIC line interrupts via
  IOAPIC routing — not implemented in this codebase)
- Cases where you only need to *probe* the device without enabling interrupts

## Background

x86 has three interrupt delivery paths:

1. **Legacy PIC line interrupts** — vectors `0x20–0x2F`. Already masked in this
   codebase. Don't use.
2. **MSI** — device writes a configured `data` value to a configured `address`
   (always the LAPIC's `0xFEEx_xxxx` range). Up to 32 vectors. Single
   address/data pair in the capability.
3. **MSI-X** — same idea but with a per-vector table living in BAR memory, up to
   2048 vectors. Modern devices prefer MSI-X.

We program MSI-X by preference, MSI as fallback, and leave legacy unused.

## Capability list walk

PCI capability list starts at config offset `0x34` *if* `STATUS.CAP_LIST` (bit
4) is set. Each entry is `(cap_id: u8, next: u8, body...)` aligned to 4 bytes.
Walk until `next == 0`. Cap IDs we care about: `0x05` (MSI), `0x11` (MSI-X).

```rust
const PCI_STATUS: u8 = 0x06;
const STATUS_CAP_LIST: u16 = 1 << 4;
const PCI_CAP_PTR: u8 = 0x34;
const CAP_ID_MSI: u8 = 0x05;
const CAP_ID_MSIX: u8 = 0x11;

pub unsafe fn walk_caps(bdf: Bdf) -> impl Iterator<Item = (u8, u8)> {
    let mut next = if pci::read16(bdf, PCI_STATUS) & STATUS_CAP_LIST != 0 {
        pci::read8(bdf, PCI_CAP_PTR) & 0xFC
    } else { 0 };
    core::iter::from_fn(move || {
        if next == 0 { return None; }
        let id = pci::read8(bdf, next);
        let nxt = pci::read8(bdf, next + 1) & 0xFC;
        let off = next;
        next = nxt;
        Some((id, off))
    })
}
```

## MSI capability layout

```
+0  cap_id (0x05) | next_ptr | message_control (16-bit)
+4  message_address_low
+8  message_address_high  (only if MC.64bit == 1)
+8/+C message_data (16-bit)
+C/+10 mask bits (only if MC.per_vector_mask == 1)
+10/+14 pending bits
```

`message_control` (offset +2):
- bit 0: MSI enable
- bits 1–3: multi-message capable (log2 of vectors requested)
- bits 4–6: multi-message enable (log2 of vectors granted by us)
- bit 7: 64-bit address capable
- bit 8: per-vector mask capable

Most drivers just want one vector — set multi-message enable to 0.

## MSI-X capability layout

```
+0  cap_id (0x11) | next_ptr | message_control (16-bit)
+4  table_offset_bir   (BIR in low 3 bits, offset in upper 29)
+8  pba_offset_bir
```

`message_control`:
- bits 0–10: table size minus 1
- bit 14: function mask (mask all)
- bit 15: MSI-X enable

The MSI-X table lives in the device's BAR at `BAR[BIR] + (offset & ~7)`. Each
entry is 16 bytes:

```
+0  message_address_low
+4  message_address_high
+8  message_data
+C  vector_control (bit 0 = masked)
```

## LAPIC message format

The MSI/MSI-X message address must target the local APIC. On x86 the encoding
is:

- `address_low` = `0xFEE0_0000 | (destination_apic_id << 12) | (RH << 3) | (DM << 2)`
- `address_high` = 0
- `data` (low 16 bits) = `(trigger << 15) | (level << 14) | (delivery_mode << 8) | vector`

For our use:
- `destination_apic_id` = BSP's APIC ID (we don't load-balance device IRQs)
- `RH` (redirection hint) = 0, `DM` (destination mode) = 0 (physical)
- `delivery_mode` = 0 (fixed), `trigger` = 0 (edge), `level` = 0
- `vector` = the IDT vector we picked (≥ 0x40)

```rust
pub fn lapic_msi_addr() -> u32 {
    let apic_id = crate::cpu::apic::current_apic_id();
    0xFEE0_0000 | ((apic_id as u32) << 12)
}

pub fn msi_data(vector: u8) -> u32 {
    vector as u32 // fixed delivery, edge, no level
}
```

## Programming pattern

```rust
pub unsafe fn enable_msix_single(bdf: Bdf, vector: u8) -> Result<(), MsiError> {
    let cap = find_msix(bdf).ok_or(MsiError::NoMsix)?;

    // 1. Disable & mask everything first.
    cap.set_function_mask(true);
    cap.set_enable(false);

    // 2. Map the table BAR into virtual memory if not already mapped.
    let table = cap.table_addr(bdf)?;

    // 3. Program entry 0.
    let entry = table.add(0);
    write_volatile(entry.add(0x00) as *mut u32, lapic_msi_addr());
    write_volatile(entry.add(0x04) as *mut u32, 0);
    write_volatile(entry.add(0x08) as *mut u32, msi_data(vector));
    write_volatile(entry.add(0x0C) as *mut u32, 0); // unmask

    // 4. Disable legacy INTx (PCI command bit 10 = 1 disables INTx).
    let cmd = pci::read16(bdf, 0x04);
    pci::write16(bdf, 0x04, cmd | (1 << 10));

    // 5. Enable, unmask function.
    cap.set_enable(true);
    cap.set_function_mask(false);

    Ok(())
}
```

## Critical rules

1. **Always disable legacy INTx** (PCI command bit 10) when enabling MSI/MSI-X.
   Otherwise the device may also raise INTx, which we don't route → spurious
   IRQs and wasted IOAPIC vectors.
2. **Mask before reconfigure.** Set function-mask bit before touching table
   entries; clear it after.
3. **MSI-X table BAR may need mapping.** It often lives in a 64-bit BAR — read
   both halves. The current `hwinit/src/paging` has identity mappings for low
   physical addresses; for high addresses, add an MMIO mapping.
4. **Don't enable MSI and MSI-X simultaneously.** Pick one. MSI-X wins if both
   present.
5. **APIC ID, not core index.** `current_apic_id()` reads the LAPIC ID register;
   it is not the same as the scheduler's core index.
6. **Edge-triggered, fixed delivery, physical destination.** This is the only
   combination this codebase supports today. Don't introduce level-triggered or
   logical destination modes without also updating the LAPIC config.
7. **Read-back to verify.** PCI config writes can silently fail on the target
   T450s if the device is in an odd power state — read back the message
   control register after enabling and assert the enable bit is set.

## Verification

- Dump the device's config space (`hwinit/src/pci/dump.rs`) and confirm the
  enable bit is set, table entry is non-zero, INTx-disable bit is set.
- In QEMU, `info pic` and `info irq` from the monitor show MSI delivery.
- On real hardware, the only signal is "events get delivered." Have the ISR
  bump a counter and read it from a debug syscall.

## Common mistakes

- Forgetting to disable INTx → device fires both, kernel sees spurious IRQs
  on unmapped legacy vectors.
- Writing the MSI-X table while function-mask is *clear* → race with hardware.
- Using `apic_id = 0` as a shortcut. BSP APIC ID is often 0 on QEMU but not
  guaranteed on real hardware.
- Mapping the table BAR through cached memory. MMIO must be uncached
  (or write-combining only where the device spec allows it).

## References

- `docs/polling-loop-audit.md` — why we need this
- `docs/interrupt-refactor-plan.md` — Phase 1 introduces this primitive
- PCI Local Bus Specification 3.0, §6.8 (MSI), §6.8.2 (MSI-X)
- Intel SDM Vol 3, §11.11 (Message Signaled Interrupts)
