# hwinit Extraction Inventory

## Purpose

This document catalogs every piece of code being extracted from the `network/` crate into the new `hwinit` crate. It exists so that future contributors understand *what* was moved, *why* it was moved, and *what assumptions* the network stack can make after the refactor.

If you're wondering why DMA setup isn't in your driver anymore: read this.

---

## Summary

| Category        | Files | Lines (approx) | Destination           |
|-----------------|-------|----------------|-----------------------|
| PCI ASM         | 5     | 1,809          | `hwinit/asm/pci/`     |
| Core ASM        | 6     | 605            | `hwinit/asm/cpu/`, `hwinit/asm/` |
| PCI Rust        | 3     | 494            | `hwinit/src/pci/`     |
| Core Rust       | 6     | 340            | `hwinit/src/cpu/`     |
| Generic DMA     | 1     | 182            | `hwinit/src/dma/`     |
| Partial Refactor| 4     | ~395 (extracted)| Various removals     |
| **Total**       | **25**| **~3,825**     |                       |

---

## Category 1: PCI ASM (1,809 lines)

These files implement PCI configuration space access at the assembly level. They are not network-specific. They are not driver-specific. They are how you talk to the PCI bus, period.

### Files (move entirely)

| Source File                            | Lines | Destination                         |
|----------------------------------------|-------|-------------------------------------|
| `network/asm/pci/legacy.s`             | 387   | `hwinit/asm/pci/legacy.s`           |
| `network/asm/pci/bar.s`                | 402   | `hwinit/asm/pci/bar.s`              |
| `network/asm/pci/capability.s`         | 415   | `hwinit/asm/pci/capability.s`       |
| `network/asm/pci/ecam.s`               | 184   | `hwinit/asm/pci/ecam.s`             |
| `network/asm/pci/virtio_cap.s`         | 421   | `hwinit/asm/pci/virtio_cap.s`       |

### Why these are platform-level

- **legacy.s**: CF8/CFC port I/O for PCI Type 1 configuration mechanism. This is how you read/write PCI config space on *any* x86 machine. Not network. Not block. Just PCI.

- **bar.s**: BAR sizing algorithm (write 0xFFFFFFFF, read back, decode). Needed to know how big MMIO regions are. Every PCI driver needs this, or none of them should implement it.

- **capability.s**: PCI capability chain walking. MSI, MSI-X, power management, VirtIO capabilities — all found by walking this chain. Generic infrastructure.

- **ecam.s**: PCIe Enhanced Configuration Access Mechanism. Memory-mapped config space access for extended registers (above 256 bytes). Platform topology feature.

- **virtio_cap.s**: Yes, this says "virtio" but it's capability discovery infrastructure that happens to understand VirtIO capability types. Block devices use the same mechanism.

### Functions exposed

```asm
; From legacy.s
asm_pci_cfg_read8, asm_pci_cfg_read16, asm_pci_cfg_read32
asm_pci_cfg_write8, asm_pci_cfg_write16, asm_pci_cfg_write32
asm_pci_make_addr

; From bar.s
asm_pci_bar_read, asm_pci_bar_read64
asm_pci_bar_size, asm_pci_bar_size64
asm_pci_bar_type, asm_pci_bar_is_io, asm_pci_bar_is_64bit
asm_pci_bar_base, asm_pci_bar_base64

; From capability.s
asm_pci_has_capabilities, asm_pci_get_cap_ptr
asm_pci_find_cap, asm_pci_read_cap_id, asm_pci_read_cap_next

; From ecam.s
asm_pcie_ecam_read32, asm_pcie_ecam_write32, asm_pcie_calc_ecam_addr

; From virtio_cap.s
asm_pci_find_virtio_cap, asm_virtio_pci_parse_cap
asm_virtio_pci_read_bar, asm_virtio_pci_probe_caps
```

---

## Category 2: Core ASM (605 lines)

CPU primitives that every bare-metal component needs. These aren't "network" primitives. They're "talking to x86 hardware" primitives.

### Files (move entirely)

| Source File                            | Lines | Destination                         |
|----------------------------------------|-------|-------------------------------------|
| `network/asm/core/barriers.s`          | 74    | `hwinit/asm/cpu/barriers.s`         |
| `network/asm/core/cache.s`             | 97    | `hwinit/asm/cpu/cache.s`            |
| `network/asm/core/delay.s`             | 113   | `hwinit/asm/cpu/delay.s`            |
| `network/asm/core/mmio.s`              | 125   | `hwinit/asm/mmio.s`                 |
| `network/asm/core/pio.s`               | 129   | `hwinit/asm/pio.s`                  |
| `network/asm/core/tsc.s`               | 67    | `hwinit/asm/cpu/tsc.s`              |

### Why these are platform-level

- **barriers.s**: SFENCE, LFENCE, MFENCE. Memory ordering guarantees for DMA. *Every* DMA-capable driver needs these. Zero network-specific content.

- **cache.s**: CLFLUSH, CLFLUSHOPT. Cache line management for DMA coherency. Block devices need this. Graphics needs this. Not network-specific.

- **delay.s**: TSC-based timing delays. Hardware often needs "wait 10μs after reset" type sequencing. Universal requirement.

- **mmio.s**: Memory-mapped I/O read/write with volatile semantics. How you talk to device registers. Period.

- **pio.s**: Port I/O instructions (IN/OUT). Legacy hardware access. PCI CF8/CFC uses this.

- **tsc.s**: RDTSC and RDTSC+serialization. Cycle-accurate timing for timeouts, delays, performance measurement.

### Functions exposed

```asm
; Barriers
asm_bar_sfence, asm_bar_lfence, asm_bar_mfence

; Cache
asm_cache_clflush, asm_cache_clflushopt, asm_cache_flush_range

; Delay
asm_delay_tsc, asm_delay_us, asm_delay_ms

; MMIO
asm_mmio_read8, asm_mmio_read16, asm_mmio_read32
asm_mmio_write8, asm_mmio_write16, asm_mmio_write32

; PIO
asm_pio_read8, asm_pio_read16, asm_pio_read32
asm_pio_write8, asm_pio_write16, asm_pio_write32

; TSC
asm_tsc_read, asm_tsc_read_serialized
```

---

## Category 3: PCI Rust (494 lines)

Rust bindings and abstractions over the PCI ASM layer. Thin wrappers providing safe(ish) access to PCI configuration space.

### Files (move entirely)

| Source File                            | Lines | Destination                         |
|----------------------------------------|-------|-------------------------------------|
| `network/src/pci/mod.rs`               | 18    | `hwinit/src/pci/mod.rs`             |
| `network/src/pci/config.rs`            | 122   | `hwinit/src/pci/config.rs`          |
| `network/src/pci/capability.rs`        | 354   | `hwinit/src/pci/capability.rs`      |

### Why these are platform-level

- **config.rs**: `PciAddr` struct, `pci_cfg_read*`, `pci_cfg_write*`, standard offset constants. This is "how to address and access PCI devices" — pure platform infrastructure.

- **capability.rs**: Capability chain walking, VirtIO capability parsing. Used by both VirtIO-net and VirtIO-blk. Not network-specific.

- **mod.rs**: Re-exports. Follows the above.

### Types/Functions exposed

```rust
// From config.rs
pub struct PciAddr { bus, device, function }
pub fn pci_cfg_read8(addr: PciAddr, offset: u8) -> u8
pub fn pci_cfg_read16(addr: PciAddr, offset: u8) -> u16
pub fn pci_cfg_read32(addr: PciAddr, offset: u8) -> u32
pub fn pci_cfg_write8(addr: PciAddr, offset: u8, value: u8)
pub fn pci_cfg_write16(addr: PciAddr, offset: u8, value: u16)
pub fn pci_cfg_write32(addr: PciAddr, offset: u8, value: u32)
pub mod offset { VENDOR_ID, DEVICE_ID, COMMAND, STATUS, ... }

// From capability.rs
pub struct VirtioCapInfo { ... }
pub struct VirtioPciCaps { ... }
pub const VIRTIO_PCI_CAP_*
```

---

## Category 4: Core Rust (340 lines)

Rust wrappers for CPU primitives. Thin, unsafe-wrapping convenience functions.

### Files (move entirely)

| Source File                                  | Lines | Destination                         |
|----------------------------------------------|-------|-------------------------------------|
| `network/src/asm/core/mod.rs`                | 9     | `hwinit/src/cpu/mod.rs`             |
| `network/src/asm/core/barriers.rs`           | 57    | `hwinit/src/cpu/barriers.rs`        |
| `network/src/asm/core/cache.rs`              | 40    | `hwinit/src/cpu/cache.rs`           |
| `network/src/asm/core/mmio.rs`               | 96    | `hwinit/src/cpu/mmio.rs`            |
| `network/src/asm/core/pio.rs`                | 88    | `hwinit/src/cpu/pio.rs`             |
| `network/src/asm/core/tsc.rs`                | 50    | `hwinit/src/cpu/tsc.rs`             |

### Why these are platform-level

Same reasoning as Core ASM — these are Rust bindings for CPU/platform primitives. They wrap the assembly. They're not network logic.

### Functions exposed

```rust
// barriers.rs
pub fn sfence(), pub fn lfence(), pub fn mfence()

// cache.rs  
pub unsafe fn clflush(addr: *const u8)
pub unsafe fn clflushopt(addr: *const u8)
pub unsafe fn flush_range(addr: *const u8, len: usize)

// mmio.rs
pub unsafe fn read8(addr: u64) -> u8   // etc.
pub unsafe fn write8(addr: u64, val: u8) // etc.

// pio.rs
pub unsafe fn inb(port: u16) -> u8  // etc.
pub unsafe fn outb(port: u16, val: u8) // etc.

// tsc.rs
pub fn read_tsc() -> u64
pub fn read_tsc_serialized() -> u64
```

---

## Category 5: Generic DMA (182 lines)

DMA region definition and layout abstraction. This is "where do descriptors and buffers live in memory" — infrastructure, not protocol.

### Files (move entirely)

| Source File                            | Lines | Destination                         |
|----------------------------------------|-------|-------------------------------------|
| `network/src/dma/region.rs`            | 182   | `hwinit/src/dma/region.rs`          |

### Why this is platform-level

`DmaRegion` is a container for:
- CPU pointer to DMA-capable memory
- Corresponding bus address
- Layout constants (offsets for descriptors, buffers)

This is **not** network-specific. VirtIO-blk uses identical structures. NVMe would too. The layout constants currently in this file are network-specific and will be split:

- **Generic `DmaRegion` struct** → `hwinit/src/dma/region.rs`
- **Network-specific layout constants** → remain in `network/src/dma/` as driver config

### What moves

```rust
// To hwinit
pub struct DmaRegion {
    cpu_ptr: *mut u8,
    bus_addr: u64,
    size: usize,
}

impl DmaRegion {
    pub const MIN_SIZE: usize = 2 * 1024 * 1024;
    pub unsafe fn new(cpu_ptr: *mut u8, bus_addr: u64, size: usize) -> Self;
    pub fn cpu_base(&self) -> *mut u8;
    pub fn bus_base(&self) -> u64;
    pub fn size(&self) -> usize;
}
```

### What stays in network

```rust
// Network-specific layout (stays in network crate)
pub const RX_DESC_OFFSET: usize = 0x0000;
pub const TX_DESC_OFFSET: usize = 0x0800;
// ... queue-specific offsets
```

---

## Category 6: Partial Refactors (395 lines extracted)

These files contain *mixed* responsibilities. Some code is platform-level (PCI scanning, bus mastering), some is driver-level (vendor ID matching, driver instantiation).

### Files affected

| Source File                              | Total Lines | Extract | Keep | Notes                           |
|------------------------------------------|-------------|---------|------|---------------------------------|
| `network/src/boot/probe.rs`              | 319         | ~100    | ~219 | PCI scan loops, enable_device   |
| `network/src/boot/block_probe.rs`        | 426         | ~180    | ~246 | PCI scan loops, enable_pci_device |
| `network/src/pci/mod.rs`                 | 18          | 18      | 0    | Entire file moves               |
| `network/src/dma/mod.rs`                 | 17          | ~8      | ~9   | Generic re-exports              |
| `network/src/boot/handoff.rs`            | 581         | ~20     | ~561 | DMA validation constants        |

### probe.rs — What gets extracted

```rust
// REMOVE: PCI bus scanning (lines 122-173)
// This triple-nested loop scanning bus/device/function belongs in hwinit.
// Network should receive a list of pre-discovered, pre-enabled devices.

fn find_virtio_nic() -> Option<(PciAddr, u64)> {
    for bus in 0..=255u8 {        // ← Platform concern
        for device in 0..32u8 {    // ← Platform concern
            for function in 0..8u8 {  // ← Platform concern
                // ... vendor ID checks, BAR reads
            }
        }
    }
}

// REMOVE: enable_device calls (line 199-200, 222-223)
// Bus mastering enablement is platform init, not driver logic.
enable_device(info.pci_addr);
pci_cfg_write16(pci_addr, offset::COMMAND, cmd | 0x06);
```

### block_probe.rs — What gets extracted

```rust
// REMOVE: PCI bus scanning (lines 131-196)
// Identical pattern to probe.rs. Platform scans once, passes device list.

fn find_ahci_controller() -> Option<AhciInfo> {
    for bus in 0..=255u8 { ... }  // ← Platform concern
}

fn find_virtio_blk() -> Option<(PciAddr, u64)> {
    for bus in 0..=255u8 { ... }  // ← Platform concern
}

// REMOVE: enable_pci_device calls
// Same story: platform enables, driver consumes.
```

### handoff.rs — What gets extracted

```rust
// MOVE to hwinit: DMA validation constants (lines 26-29)
pub const MIN_DMA_SIZE: u64 = 2 * 1024 * 1024;
// DMA policy is platform policy, not network policy.
```

---

## What the Network Crate Loses

After this refactor, `network/` will **not** contain:

1. **PCI enumeration** — No bus/device/function scanning
2. **PCI config access** — No direct CF8/CFC or ECAM reads
3. **Bus mastering enablement** — No `COMMAND |= 0x06` writes
4. **DMA allocation policy** — No "allocate below 4GB" logic
5. **CPU primitives** — No SFENCE/LFENCE/TSC bindings
6. **MMIO/PIO primitives** — No direct port/memory access wrappers
7. **Cache management** — No CLFLUSH invocations for DMA prep

---

## What the Network Crate May Assume

After `hwinit` has run, network drivers may assume:

1. **Devices are pre-discovered** — Receive `PreparedDevice` structs with:
   - PCI address (bus/device/function)
   - MMIO base address (already decoded from BARs)
   - Device type/variant

2. **Bus mastering is enabled** — COMMAND register already has memory space and bus master bits set

3. **DMA is legal** — Memory regions are:
   - Allocated below 4GB (or IOMMU is configured for remapping)
   - Identity-mapped (bus address = physical address)
   - Cache-coherent or properly flushed

4. **IOMMU is configured** — If present, identity domain established

5. **Timing is calibrated** — TSC frequency known and validated

6. **Platform is sane** — No need for "defensive" PCI probing or "just in case" bus master re-enablement

---

## Verification Checklist

Before declaring Phase 1 complete, verify:

- [ ] All files listed above exist at the specified paths
- [ ] Line counts are approximately correct (±10%)
- [ ] No files are missing from inventory
- [ ] Categories correctly classify platform vs. driver responsibility
- [ ] "What network loses" list is complete
- [ ] "What network may assume" captures all preconditions

---

*Last updated: 2026-01-16*
*Phase: 1 — Documentation & Inventory*
