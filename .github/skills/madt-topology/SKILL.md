---
name: madt-topology
description: 'ACPI MADT parsing for SMP topology. Use when extracting LAPIC IDs from MADT, parsing Local APIC or x2APIC entries, NMI source records, IOAPIC entries, RSDP/RSDT/XSDT traversal, checksum validation, multicore CPU discovery, acpi.rs, start_aps_from_list, MADT signature APIC, LAPIC address field in MADT fixed header, enabled flag in MADT entries.'
argument-hint: "MADT parsing task or topology bug"
---

# MADT Topology

## When to Use
- Adding new MADT entry types (e.g. x2APIC override, NMI source)
- Debugging wrong core count on firmware that lies about CPUID topology
- Tracing RSDP → RSDT/XSDT → MADT walk failures
- Extracting the MADT LAPIC address override (firmware can remap the LAPIC base)
- `start_aps_from_list` receiving wrong or empty LAPIC ID list

## Key Files
- `hwinit/src/cpu/acpi.rs` — RSDP scan, MADT walk, LAPIC ID extraction
- `hwinit/src/cpu/ap_boot.rs` — `start_aps_from_list` consumer

## MADT Entry Types

| Type | Struct | What to extract |
|------|--------|-----------------|
| 0x00 | Processor Local APIC | `apic_id` if `flags & 1` (enabled) |
| 0x01 | IOAPIC | base address, GSI base |
| 0x02 | Interrupt Source Override | IRQ → GSI remapping |
| 0x04 | NMI Source | pin, flags |
| 0x05 | LAPIC NMI | lint pin |
| 0x09 | Processor Local x2APIC | `x2apic_id` if `flags & 1` |

Prefer type 0x09 entries when present — type 0x00 APIC IDs are truncated to 8 bits.
If both are present for the same ACPI UID, use 0x09.

## RSDP Location Strategy

1. Scan EBDA (first 1 KiB at physical address from `[0x40E] << 4`).
2. Scan BIOS ROM `0xE0000`–`0xFFFFF` in 16-byte steps.
3. UEFI path: read from EFI config table (ACPI 2.0 GUID) — already done before ExitBootServices.

`RSDP_SIG = b"RSD PTR "` (8 bytes, note trailing space).

## Checksum Validation

Every ACPI table has a checksum byte such that the byte-sum of the entire table = 0 (mod 256).
RSDP has two checksums: base 20-byte sum, and extended 36-byte sum for ACPI 2.0.
Skip tables with invalid checksums — firmware sometimes has corrupt entries.

## Common Failure Modes

| Symptom | Cause |
|---------|-------|
| Only BSP comes up, MADT list empty | MADT walk not finding entries; check RSDP search range |
| Core count off by one | BSP's own LAPIC ID included in the list; filter by BSP LAPIC ID |
| Wrong LAPIC IDs on NUMA systems | Only reading type 0x00, missing type 0x09 x2APIC entries |
| MADT table at address > 4 GB | RSDT uses 32-bit pointers; use XSDT path for ACPI 2.0 |
| Checksum valid but table garbage | ACPI revision < 2 RSDP, but trying to read 64-bit XSDT field |

## Parsing Discipline

All pointers are physical addresses, identity-mapped. No allocation. All reads via raw pointer casts — use `read_unaligned` because ACPI structs are `packed`.

```rust
// Always use read_unaligned on packed structs
let apic_id = unsafe { core::ptr::read_unaligned(&entry.apic_id) };
```

Failing to use `read_unaligned` on `#[repr(C, packed)]` fields is UB and will misread on unaligned accesses.

## Procedure

1. Read `acpi.rs` fully — understand the current traversal before extending it.
2. For new entry types: add a variant to the match on the entry type byte.
3. Always check the `flags & 1` enabled bit before adding a LAPIC to the list.
4. Filter out the BSP LAPIC ID from the AP list (compare against `apic::read_lapic_id()`).
5. Respect `MAX_CPUS` — do not collect more entries than the array can hold.
