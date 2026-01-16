# hwinit — Hardware Initialization Crate

## What This Crate Is

`hwinit` is the foundational hardware initialization layer for MorpheusX. It runs **once**, **early**, and establishes the invariants that all device drivers depend on.

Think of it as the "make the machine sane" layer. After `hwinit` completes, drivers are allowed to exist.

---

## What This Crate Is Responsible For

### PCI Subsystem
- PCI bus enumeration (all buses, devices, functions)
- Configuration space access (Legacy CF8/CFC and PCIe ECAM)
- BAR decoding and sizing
- Capability chain walking
- Bus mastering enablement
- Memory space / I/O space enable

### CPU Primitives
- Memory barriers (SFENCE, LFENCE, MFENCE)
- Cache management (CLFLUSH, CLFLUSHOPT)
- TSC access and calibration
- Timing delays (TSC-based microsecond/millisecond waits)

### Low-Level I/O
- MMIO read/write (volatile, properly ordered)
- Port I/O (IN/OUT instructions)

### DMA Infrastructure
- DMA region abstraction (CPU pointer ↔ bus address mapping)
- Allocation policy (below 4GB for 32-bit DMA masters)
- IOMMU detection and configuration (future)
- Identity mapping verification

### Platform Orchestration
- Single entry point that performs all initialization in correct order
- Returns a manifest of prepared devices to higher layers
- Establishes invariants that drivers may rely on

---

## What This Crate Does NOT Do

### No Device-Specific Logic
- No VirtIO queue setup
- No Intel e1000e register programming  
- No AHCI port initialization
- No NVMe command queue creation

### No Protocol Logic
- No Ethernet frame handling
- No SCSI/ATA command translation
- No packet parsing

### No Driver Behavior
- No RX/TX processing
- No interrupt handling (beyond basic MSI enumeration)
- No performance tuning
- No runtime device management

### No Speculative Abstractions
- No "maybe we'll need this later" code
- No unused flexibility
- No premature optimization

If a driver needs to do something device-specific, that belongs in the driver. If the driver needs to call back into `hwinit` for platform services, the architecture is wrong.

---

## Lifecycle

```
┌─────────────────────────────────────────────────────────────────┐
│                        BOOT SEQUENCE                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   UEFI Boot                                                     │
│       │                                                         │
│       ▼                                                         │
│   ExitBootServices()                                            │
│       │                                                         │
│       ▼                                                         │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │                    hwinit runs                          │   │
│   │                                                         │   │
│   │   1. Calibrate TSC                                      │   │
│   │   2. Enumerate PCI buses                                │   │
│   │   3. Decode BARs for discovered devices                 │   │
│   │   4. Enable bus mastering on relevant devices           │   │
│   │   5. Allocate DMA regions (below 4GB)                   │   │
│   │   6. Detect IOMMU, configure identity domain            │   │
│   │   7. Build PreparedDevice manifest                      │   │
│   │   8. Return to caller                                   │   │
│   └─────────────────────────────────────────────────────────┘   │
│       │                                                         │
│       ▼                                                         │
│   Driver initialization                                         │
│       │                                                         │
│       │   Drivers receive PreparedDevice structs                │
│       │   Drivers assume:                                       │
│       │     • Bus mastering enabled                             │
│       │     • MMIO accessible                                   │
│       │     • DMA legal                                         │
│       │     • No PCI scanning needed                            │
│       │                                                         │
│       ▼                                                         │
│   Normal operation                                              │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### When hwinit Runs
- **Once** per boot
- **After** ExitBootServices (we own the hardware)
- **Before** any driver initialization

### When hwinit Does NOT Run
- Never re-runs during normal operation
- No "reinitialize" or "reset" paths (for now)
- Drivers must not call back into hwinit's init sequence

---

## Interaction with Network Crate

### Before Refactor (Current State)

```
network/
├── asm/
│   ├── pci/        ← PCI config access (WRONG LOCATION)
│   └── core/       ← CPU primitives (WRONG LOCATION)
├── src/
│   ├── pci/        ← PCI Rust bindings (WRONG LOCATION)
│   ├── asm/core/   ← CPU Rust bindings (WRONG LOCATION)
│   ├── dma/        ← DMA region (WRONG LOCATION)
│   └── boot/
│       ├── probe.rs      ← PCI scanning (WRONG LOCATION)
│       └── block_probe.rs ← More PCI scanning (WRONG LOCATION)
```

Network stack does its own PCI enumeration, enables bus mastering, allocates DMA... and QEMU lets it work. Real hardware does not.

### After Refactor (Target State)

```
hwinit/                              network/
├── asm/                             ├── src/
│   ├── pci/                         │   ├── driver/
│   │   ├── legacy.s                 │   │   ├── virtio.rs
│   │   ├── bar.s                    │   │   └── intel.rs
│   │   ├── capability.s             │   ├── stack/
│   │   └── ecam.s                   │   └── ...
│   ├── cpu/                         │
│   │   ├── barriers.s               │ (no PCI, no DMA alloc,
│   │   ├── cache.s                  │  no CPU primitives)
│   │   ├── delay.s                  │
│   │   └── tsc.s                    │
│   ├── mmio.s                       │
│   └── pio.s                        │
├── src/
│   ├── pci/
│   ├── cpu/
│   ├── dma/
│   └── platform.rs
```

### API Contract

`hwinit` exposes:

```rust
/// Result of platform initialization
pub struct PlatformInit {
    /// Discovered and prepared network devices
    pub net_devices: &'static [PreparedNetDevice],
    /// Discovered and prepared block devices  
    pub blk_devices: &'static [PreparedBlkDevice],
    /// Calibrated TSC frequency
    pub tsc_freq: u64,
    /// Pre-allocated DMA region
    pub dma_region: DmaRegion,
}

/// A network device ready for driver initialization
pub struct PreparedNetDevice {
    pub pci_addr: PciAddr,
    pub mmio_base: u64,
    pub device_type: NetDeviceType,
    pub mac_addr: [u8; 6],  // if discoverable
}

/// A block device ready for driver initialization
pub struct PreparedBlkDevice {
    pub pci_addr: PciAddr,
    pub mmio_base: u64,
    pub device_type: BlkDeviceType,
}
```

Network stack calls:
```rust
// Receive prepared devices - no scanning, no enabling
let init = hwinit::platform_init()?;
for dev in init.net_devices {
    match dev.device_type {
        NetDeviceType::VirtIO => VirtioNetDriver::new(dev, &init.dma_region),
        NetDeviceType::IntelE1000e => E1000eDriver::new(dev, &init.dma_region),
    }
}
```

---

## Directory Structure

```
hwinit/
├── Cargo.toml              # (Phase 3)
├── build.rs                # (Phase 3) 
├── README.md               # This file
├── ARCHITECTURE.md         # Detailed architecture docs
├── asm/
│   ├── pci/
│   │   ├── README.md       # PCI ASM documentation
│   │   ├── legacy.s        # (Phase 3)
│   │   ├── bar.s           # (Phase 3)
│   │   ├── capability.s    # (Phase 3)
│   │   └── ecam.s          # (Phase 3)
│   ├── cpu/
│   │   ├── README.md       # CPU ASM documentation
│   │   ├── barriers.s      # (Phase 3)
│   │   ├── cache.s         # (Phase 3)
│   │   ├── delay.s         # (Phase 3)
│   │   └── tsc.s           # (Phase 3)
│   ├── mmio.s              # (Phase 3)
│   └── pio.s               # (Phase 3)
└── src/
    ├── lib.rs              # (Phase 3)
    ├── platform.rs         # (Phase 3)
    ├── pci/
    │   ├── mod.rs          # (Phase 3)
    │   ├── config.rs       # (Phase 3)
    │   └── capability.rs   # (Phase 3)
    ├── cpu/
    │   ├── mod.rs          # (Phase 3)
    │   ├── barriers.rs     # (Phase 3)
    │   ├── cache.rs        # (Phase 3)
    │   ├── mmio.rs         # (Phase 3)
    │   ├── pio.rs          # (Phase 3)
    │   └── tsc.rs          # (Phase 3)
    └── dma/
        ├── mod.rs          # (Phase 3)
        └── region.rs       # (Phase 3)
```

---

## Design Principles

1. **Run once, establish invariants** — No re-initialization, no "maybe fix it later"

2. **Explicit over implicit** — If a device needs bus mastering, we enable it explicitly in one place

3. **Fail early, fail loud** — Invalid hardware state detected during init should panic, not paper over

4. **No backwards dependencies** — hwinit depends on nothing runtime; drivers depend on hwinit

5. **No temporary bridges** — No "keep the old code path for now" compromises

6. **Real hardware is the truth** — QEMU success is necessary but not sufficient

---

## References

- [HWINIT_EXTRACTION_INVENTORY.md](../docs/md/HWINIT_EXTRACTION_INVENTORY.md) — Complete file inventory
- [HWINIT_REFACTORING_PLAN.md](../docs/md/HWINIT_REFACTORING_PLAN.md) — Original audit and plan
- [ARCHITECTURE_V3.md](../docs/md/Architecture&Design/) — System architecture

---

*Last updated: 2026-01-16*
*Phase: 2 — Skeleton & Documentation (NO CODE)*
