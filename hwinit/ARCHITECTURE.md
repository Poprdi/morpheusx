# hwinit Architecture

## Overview

This document describes the internal architecture of the `hwinit` crate. Read [README.md](README.md) first for the high-level purpose and lifecycle.

---

## Module Hierarchy

```
hwinit
├── pci           # PCI subsystem
│   ├── config    # Configuration space access
│   ├── bar       # BAR decoding and sizing
│   ├── cap       # Capability chain walking
│   └── enum      # Device enumeration
├── cpu           # CPU primitives
│   ├── barriers  # Memory ordering (sfence/lfence/mfence)
│   ├── cache     # Cache management (clflush)
│   ├── tsc       # Time stamp counter
│   └── delay     # Timing delays
├── io            # Low-level I/O
│   ├── mmio      # Memory-mapped I/O
│   └── pio       # Port I/O
├── dma           # DMA infrastructure
│   ├── region    # DMA region abstraction
│   ├── alloc     # DMA allocation (future)
│   └── iommu     # IOMMU management (future)
└── platform      # Orchestration
    └── init      # Platform initialization sequence
```

---

## Layer Dependencies

```
┌─────────────────────────────────────────────────────────────────┐
│                        platform                                 │
│                    (orchestration)                              │
├─────────────────────────────────────────────────────────────────┤
│     pci         │       dma        │       (future)            │
│  (enumeration)  │   (allocation)   │   (iommu, acpi, etc)      │
├─────────────────────────────────────────────────────────────────┤
│                         cpu                                     │
│            (barriers, cache, tsc, delay)                        │
├─────────────────────────────────────────────────────────────────┤
│                          io                                     │
│                    (mmio, pio)                                  │
├─────────────────────────────────────────────────────────────────┤
│                         asm                                     │
│              (raw assembly primitives)                          │
└─────────────────────────────────────────────────────────────────┘
```

Lower layers never depend on higher layers. This is not negotiable.

---

## PCI Subsystem

### Configuration Access

Two mechanisms supported:

1. **Legacy (CF8/CFC)** — Port I/O based
   - Address port: 0xCF8
   - Data port: 0xCFC
   - Supports registers 0x00-0xFF only
   - Works on all x86 hardware

2. **ECAM (Memory-Mapped)** — PCIe extended config
   - Base address from ACPI MCFG table
   - Supports registers 0x000-0xFFF
   - Required for PCIe extended capabilities

### BAR Handling

BAR sizing algorithm:
1. Save original BAR value
2. Write 0xFFFFFFFF
3. Read back (reveals size mask)
4. Restore original value
5. Invert and add 1 to get size

64-bit BARs span two consecutive BAR slots.

### Capability Walking

PCI capabilities form a linked list:
- STATUS register bit 4 indicates presence
- CAP_PTR (offset 0x34) points to first capability
- Each capability has ID and next pointer

We walk this chain to find:
- MSI / MSI-X capabilities
- Power management
- VirtIO-specific capabilities
- PCIe extended capabilities

---

## CPU Primitives

### Memory Barriers

| Barrier | Instruction | Use Case |
|---------|-------------|----------|
| sfence  | SFENCE      | After writing descriptors, before notifying device |
| lfence  | LFENCE      | After reading device index, before reading data |
| mfence  | MFENCE      | Full serialization (expensive, use sparingly) |

These prevent CPU reordering that would break DMA protocols.

### Cache Management

DMA coherency depends on memory type:
- **UC (Uncached)**: No cache involvement, hardware coherent
- **WC (Write-Combining)**: Writes may be combined, reads bypass cache
- **WB (Write-Back)**: Full caching, requires explicit flush

For WB-mapped DMA buffers:
- `clflush` before device reads our data
- `clflush` after device writes data we need to read

### TSC

Time Stamp Counter provides cycle-accurate timing:
- `RDTSC` — Fast (~40 cycles) but weakly ordered
- `CPUID; RDTSC` — Serialized (~200 cycles) but precise

TSC frequency must be calibrated at boot (from CPUID or UEFI timing).

---

## DMA Infrastructure

### DmaRegion

Core abstraction representing DMA-capable memory:

```
struct DmaRegion {
    cpu_ptr: *mut u8,    // What we use to read/write
    bus_addr: u64,       // What we tell the device
    size: usize,         // How big
}
```

### Allocation Policy

DMA memory requirements:
1. **Addressable** — Must be within device's addressing capability
   - 32-bit DMA masters: below 4GB
   - 64-bit DMA masters: anywhere (but we use below 4GB for simplicity)

2. **Contiguous** — Device sees physical addresses, not virtual
   - Identity mapping: bus_addr == physical_addr

3. **Aligned** — Device requirements vary
   - Descriptors: often 16-byte aligned
   - Buffers: often page-aligned for convenience

### IOMMU (Future)

When IOMMU is present (Intel VT-d, AMD IOMMU):
- Devices can be given restricted memory access
- Translation can remap non-contiguous pages
- We'll establish identity domain for simplicity initially

---

## Platform Initialization Sequence

The `platform_init()` function executes this sequence:

```
1. CALIBRATE TSC
   ├── Read TSC frequency from CPUID (if available)
   └── Or measure against known time source

2. ENUMERATE PCI
   ├── Scan all buses (0-255)
   ├── For each bus, scan devices (0-31)
   ├── For each device, scan functions (0-7)
   ├── Read vendor/device ID
   ├── If valid device found:
   │   ├── Read class code
   │   ├── Decode BARs
   │   ├── Walk capability chain
   │   └── Record in device list
   └── Apply device type classification

3. PREPARE DEVICES
   ├── For each device:
   │   ├── Enable memory space (COMMAND bit 1)
   │   ├── Enable bus mastering (COMMAND bit 2)
   │   └── Disable legacy INTx if MSI available
   └── Build PreparedDevice structures

4. ALLOCATE DMA
   ├── Request DMA-capable memory (below 4GB)
   ├── Verify identity mapping (bus == physical)
   └── Create DmaRegion

5. CONFIGURE IOMMU (if present)
   ├── Detect IOMMU via ACPI/PCI
   ├── Establish identity domain
   └── Grant access to allocated DMA regions

6. RETURN MANIFEST
   └── PlatformInit { devices, tsc_freq, dma_region }
```

---

## Error Handling

Errors during platform init are generally fatal:
- No PCI devices found → panic (something is very wrong)
- DMA allocation fails → panic (can't proceed without DMA)
- TSC not available → panic (can't do timing)

We don't attempt recovery. The machine is not in a usable state if these fail.

---

## Testing Strategy

### Unit Tests (when code exists)
- PCI address calculation
- BAR decoding logic
- Capability chain walking (mock data)

### Integration Tests
- QEMU with known device configurations
- Verify correct device discovery
- Verify BAR values match expected

### Hardware Tests
- Real Intel NIC (I218-V from T450s)
- Verify bus mastering actually works
- Verify DMA actually completes
- This is the only test that matters

---

## Non-Goals

Explicitly out of scope:

1. **Hot-plug** — We don't support devices appearing/disappearing
2. **Power management** — No suspend/resume
3. **Multiple IOMMU domains** — Single identity domain only
4. **NUMA awareness** — Allocate from any node
5. **Driver unload** — Drivers are eternal once loaded

These may be added in future phases if needed.

---

*Last updated: 2026-01-16*
*Phase: 2 — Skeleton & Documentation (NO CODE)*
