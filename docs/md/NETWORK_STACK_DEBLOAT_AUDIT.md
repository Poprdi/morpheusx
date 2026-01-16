# Network Stack Debloat Audit

## Overview

This document identifies all **generic hardware initialization code** that currently resides in the `network/` crate but should be moved to a dedicated `hwinit` crate. The network stack should **only** contain device-specific driver logic and expect all generic hardware to be in a sane, initialized state.

**Design Principle**: The network stack should receive a fully initialized platform with:
- PCI bus enumerated
- Bus mastering enabled
- DMA regions allocated and identity-mapped
- Cache coherence configured
- Memory barriers available

---

## Summary of Violations

| Category | Files Affected | Severity | Lines of Code |
|----------|----------------|----------|---------------|
| PCI Enumeration & Config | 8 files | **HIGH** | ~800 lines |
| Bus Mastering Enable | 3 files | **HIGH** | ~30 lines |
| DMA Region Management | 5 files | **MEDIUM** | ~450 lines |
| Generic ASM (barriers, MMIO, PIO) | 8 files | **MEDIUM** | ~600 lines |
| Memory Allocation | 1 file | **LOW** | ~165 lines |
| TSC/Timing | 2 files | **MEDIUM** | ~150 lines |

**Total estimated generic code to extract: ~2,200 lines**

---

## Category 1: PCI Enumeration & Configuration (HIGH PRIORITY)

### Files to Move Entirely to `hwinit`

#### `network/src/pci/` (entire module)
**Path**: `network/src/pci/mod.rs`, `config.rs`, `capability.rs`
**Lines**: ~500

**What it contains**:
- `PciAddr` struct - generic PCI BDF addressing
- `pci_cfg_read8/16/32` - PCI config space access wrappers
- `pci_cfg_write8/16/32` - PCI config space write wrappers
- `offset::*` - PCI standard register offsets (VENDOR_ID, DEVICE_ID, BAR0, etc.)
- `status::*` - PCI status register bits
- VirtIO capability parsing (`VirtioCapInfo`, `VirtioPciCaps`)

**Why it's generic**: PCI configuration space access is not network-specific. Every PCI device (NIC, AHCI, NVMe, GPU) uses the same CF8/CFC port I/O mechanism.

```rust
// Currently in network/src/pci/config.rs
pub fn pci_cfg_read32(addr: PciAddr, offset: u8) -> u32 {
    unsafe { asm_pci_cfg_read32(addr.bus, addr.device, addr.function, offset) }
}
```

#### `network/asm/pci/` (entire directory)
**Path**: `network/asm/pci/legacy.s`, `bar.s`, `capability.s`, `ecam.s`, `virtio_cap.s`
**Lines**: ~800

**What it contains**:
- `asm_pci_cfg_read32/16/8` - Legacy CF8/CFC PCI config access
- `asm_pci_cfg_write32/16/8` - Legacy PCI config writes
- `asm_pci_make_addr` - Build PCI address from BDF
- BAR probing and sizing
- Capability list walking

**Why it's generic**: This is fundamental PCI bus access, not networking.

---

### Files with PCI Code to Extract

#### `network/src/boot/probe.rs`
**Lines to extract**: ~150

**Generic code present**:
```rust
// PCI bus scanning - GENERIC
fn find_virtio_nic() -> Option<(PciAddr, u64)> {
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                // Full PCI enumeration...
```

```rust
// Device enable - GENERIC
enable_device(info.pci_addr);  // Sets bus master + memory space
```

**Network-specific code to KEEP**:
- VirtIO vendor/device ID matching
- Intel vendor/device ID matching
- Driver instantiation

#### `network/src/boot/block_probe.rs`
**Lines to extract**: ~200

**Generic code present**:
```rust
// Duplicate PCI scanning logic
fn find_ahci_controller() -> Option<AhciInfo>
fn find_virtio_blk() -> Option<(PciAddr, u64)>

// Duplicate enable function
fn enable_pci_device(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    pci_cfg_write16(addr, offset::COMMAND, cmd | 0x06);
}
```

#### `network/src/driver/intel/mod.rs`
**Lines to extract**: ~50

**Generic code present**:
```rust
// PCI BAR sizing - GENERIC (not Intel-specific)
pub fn read_bar_size(addr: PciAddr, bar_index: u8) -> u32

// Bus master enable - GENERIC
pub fn enable_device(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    let new_cmd = cmd | 0x06;
    pci_cfg_write16(addr, offset::COMMAND, new_cmd);
}
```

---

## Category 2: Bus Mastering Enable (HIGH PRIORITY)

### Duplicated Code Locations

| File | Function | Lines |
|------|----------|-------|
| `network/src/boot/probe.rs:199` | `enable_device()` call | 1 |
| `network/src/boot/probe.rs:226-227` | Inline bus master enable | 2 |
| `network/src/boot/block_probe.rs:310-315` | `enable_pci_device()` | 5 |
| `network/src/boot/block_probe.rs:330` | `enable_pci_device()` call | 1 |
| `network/src/boot/block_probe.rs:355` | `enable_pci_device()` call | 1 |
| `network/src/driver/intel/mod.rs:182-190` | `enable_device()` | 8 |

**Problem**: Bus mastering enable is done **inside** the driver layer when it should be done **before** any driver sees the device.

**Correct Flow**:
```
hwinit (Phase 1):
  - Enumerate PCI bus
  - For each device: enable bus mastering + memory space
  - Allocate DMA regions
  - Configure IOMMU (if present)

network (Phase 2):
  - Receive pre-initialized device handle
  - Assume bus mastering already enabled
  - Configure device-specific registers
```

---

## Category 3: DMA Region Management (MEDIUM PRIORITY)

### Files to Move to `hwinit`

#### `network/src/dma/region.rs`
**Lines**: 183
**What it contains**:
- `DmaRegion` struct - generic DMA memory layout
- Offset constants for descriptor tables, buffers
- CPU/bus address translation

**Why it's partially generic**: The concept of a DMA region with CPU and bus addresses is generic. However, the specific **layout** (VirtIO descriptor ring offsets) is driver-specific.

**Recommendation**: Split into:
- `hwinit::DmaRegion` - Generic: cpu_ptr, bus_addr, size, allocation
- `network::VirtioDmaLayout` - VirtIO-specific offsets

#### `network/src/dma/buffer.rs` and `pool.rs`
**Lines**: ~350 combined
**Status**: **Keep in network stack** - these are buffer management abstractions that are network-specific (packet buffers, ownership tracking).

#### `network/src/boot/handoff.rs`
**Lines to extract**: ~100

**Generic code present**:
```rust
// DMA region validation - GENERIC
pub const MIN_DMA_SIZE: u64 = 2 * 1024 * 1024;

// DMA validation errors - GENERIC
DmaRegionTooSmall,
DmaCpuPtrNull,
DmaBusAddrZero,
```

---

## Category 4: Generic ASM Primitives (MEDIUM PRIORITY)

### Files to Move Entirely to `hwinit`

#### `network/asm/core/` (entire directory)
**Files**:
- `barriers.s` - sfence, lfence, mfence
- `cache.s` - clflush, clflushopt, cache_flush_range
- `mmio.s` - MMIO read/write 8/16/32
- `pio.s` - Port I/O in/out 8/16/32
- `tsc.s` - Read TSC, TSC frequency calibration
- `delay.s` - TSC-based delays

**Lines**: ~400 ASM + ~200 Rust wrappers

**Why it's generic**: These are fundamental x86-64 CPU primitives used by ALL drivers, not just network:
- Memory barriers for any DMA device
- Cache management for any DMA device
- MMIO access for any MMIO device
- Port I/O for legacy devices, PCI config

**Current location**: `network/src/asm/core/`
**Rust wrappers**: `network/src/asm/core/*.rs`

---

## Category 5: Memory Allocation (LOW PRIORITY)

#### `network/src/alloc_heap.rs`
**Lines**: 165

**What it contains**:
- Global allocator using `linked_list_allocator`
- Static 1MB heap buffer
- `init_heap()` function

**Analysis**: This is post-EBS heap allocation. While not network-specific, it's currently only needed by the network stack. Could remain here or move to a `runtime` crate.

**Recommendation**: Low priority - can stay for now.

---

## Category 6: TSC/Timing (MEDIUM PRIORITY)

#### `network/src/time/` module
#### `network/src/boot/handoff.rs` (TSC calibration)

**Generic code**:
- `TscCalibration` struct
- `read_tsc_raw()` function
- `has_invariant_tsc()` function

**Why it's generic**: TSC timing is used by ALL drivers for timeouts, not just network.

---

## Proposed `hwinit` Crate Structure

```
hwinit/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── pci/
│   │   ├── mod.rs          # PciAddr, PciDevice, PciEnumerator
│   │   ├── config.rs       # Config space access
│   │   ├── bar.rs          # BAR probing
│   │   └── capability.rs   # Capability chain walking
│   ├── dma/
│   │   ├── mod.rs          # DmaRegion, DmaAllocator
│   │   └── iommu.rs        # IOMMU setup (VT-d)
│   ├── cpu/
│   │   ├── mod.rs
│   │   ├── barriers.rs     # sfence, lfence, mfence
│   │   ├── cache.rs        # clflush, cache_flush_range
│   │   └── tsc.rs          # TSC read, calibration
│   ├── mmio.rs             # MMIO read/write
│   └── pio.rs              # Port I/O
└── asm/
    ├── pci/
    │   ├── legacy.s        # CF8/CFC access
    │   └── ecam.s          # PCIe ECAM access
    ├── core/
    │   ├── barriers.s
    │   ├── cache.s
    │   ├── mmio.s
    │   ├── pio.s
    │   └── tsc.s
    └── dma/
        └── iommu.s         # VT-d setup
```

---

## API Contract After Debloat

### What `hwinit` Provides to Network Stack

```rust
/// Initialized PCI device handle
pub struct PciDeviceHandle {
    pub pci_addr: PciAddr,
    pub mmio_base: u64,
    pub mmio_size: usize,
    pub device_type: DeviceType,
    // Bus mastering: ALREADY ENABLED
    // Memory space: ALREADY ENABLED
}

/// Pre-allocated DMA region
pub struct DmaAllocation {
    pub cpu_base: *mut u8,
    pub bus_base: u64,  // Physical/device-visible address
    pub size: usize,
    // Identity-mapped: GUARANTEED
    // Below 4GB: GUARANTEED (or IOMMU configured)
    // Cache coherent: CONFIGURED (UC/WC or flush API provided)
}

/// Platform initialization result
pub struct HwInitResult {
    pub tsc_freq: u64,
    pub devices: Vec<PciDeviceHandle>,
    pub dma_regions: Vec<DmaAllocation>,
}
```

### What Network Stack Assumes

```rust
// Network driver init signature AFTER debloat
impl IntelE1000eDriver {
    pub fn new(
        device: &PciDeviceHandle,   // Pre-enabled device
        dma: &DmaAllocation,        // Pre-allocated DMA
        tsc_freq: u64,              // Pre-calibrated
    ) -> Result<Self, E1000eError> {
        // NO PCI enumeration here
        // NO bus master enable here
        // NO DMA allocation here
        // ONLY Intel e1000e register programming
    }
}
```

---

## Migration Plan

### Phase 1: Create `hwinit` crate skeleton
1. Create `hwinit/Cargo.toml`
2. Move `network/asm/core/` → `hwinit/asm/core/`
3. Move `network/asm/pci/` → `hwinit/asm/pci/`
4. Move `network/src/pci/` → `hwinit/src/pci/`
5. Create Rust wrappers in `hwinit`

### Phase 2: Update network stack dependencies
1. Add `hwinit` as dependency
2. Replace `crate::pci::*` with `hwinit::pci::*`
3. Replace `crate::asm::core::*` with `hwinit::cpu::*`
4. Update imports in all affected files

### Phase 3: Refactor probe/init logic
1. Move PCI enumeration to `hwinit`
2. Move bus master enable to `hwinit`
3. Create `HwInitResult` → `NetworkStack` handoff
4. Remove duplicated `enable_device` functions

### Phase 4: Implement proper DMA setup
1. Add IOMMU detection to `hwinit`
2. Add identity-mapping verification
3. Add cache coherence configuration
4. Add DMA address validation (< 4GB or IOMMU)

---

## Files Summary

### Files to Move Entirely
| Source | Destination |
|--------|-------------|
| `network/src/pci/*` | `hwinit/src/pci/*` |
| `network/asm/pci/*` | `hwinit/asm/pci/*` |
| `network/asm/core/*` | `hwinit/asm/core/*` |
| `network/src/asm/core/*` | `hwinit/src/cpu/*` |

### Files to Partially Refactor
| File | Action |
|------|--------|
| `network/src/boot/probe.rs` | Remove PCI scan, keep driver matching |
| `network/src/boot/block_probe.rs` | Remove PCI scan, keep driver matching |
| `network/src/boot/handoff.rs` | Move TSC calibration to hwinit |
| `network/src/driver/intel/mod.rs` | Remove `enable_device`, `read_bar_size` |
| `network/src/dma/region.rs` | Split generic/specific |

### Files to Keep As-Is
| File | Reason |
|------|--------|
| `network/src/driver/intel/*.rs` | Device-specific register programming |
| `network/src/driver/virtio/*.rs` | Device-specific VirtIO implementation |
| `network/src/dma/buffer.rs` | Network-specific buffer management |
| `network/src/dma/pool.rs` | Network-specific pool management |
| `network/src/stack/*` | smoltcp integration |
| `network/src/mainloop/*` | Network polling loop |

---

## Critical Hardware Issues This Fixes

Moving this code to `hwinit` allows proper implementation of:

1. **DMA Below 4GB**: `hwinit` can enforce allocation below 4GB or configure IOMMU
2. **Identity Mapping**: `hwinit` can verify physical = virtual for DMA buffers
3. **Cache Coherence**: `hwinit` can configure memory as UC/WC or provide flush API
4. **Bus Master Enable BEFORE Driver**: Proper ordering guaranteed
5. **IOMMU Configuration**: Central place to detect and configure VT-d

These are the exact issues causing "works in QEMU, fails on real hardware" bugs.
