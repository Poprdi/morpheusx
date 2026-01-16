# Hardware Initialization Refactoring Plan

## Executive Summary

This document provides a **line-by-line audit** of the `network/` crate identifying all code that violates separation of concerns. The network stack currently contains **4,175+ lines** of generic hardware initialization code that must be moved to a new `hwinit` crate.

**Root Cause of Real Hardware Failures**: The network stack is doing hardware initialization (PCI enumeration, bus mastering, DMA setup) inline with driver initialization. QEMU forgives timing/ordering issues; real Intel I218 silicon does not.

---

## Table of Contents

1. [Files to Move Entirely](#1-files-to-move-entirely)
2. [Files to Partially Refactor](#2-files-to-partially-refactor)
3. [Line-by-Line Extraction Guide](#3-line-by-line-extraction-guide)
4. [hwinit Crate Implementation Plan](#4-hwinit-crate-implementation-plan)
5. [API Contract Specification](#5-api-contract-specification)
6. [Migration Execution Steps](#6-migration-execution-steps)
7. [Testing Strategy](#7-testing-strategy)

---

## 1. Files to Move Entirely

### 1.1 PCI ASM Layer (1,809 lines total)

| File | Lines | Move To |
|------|-------|---------|
| `network/asm/pci/legacy.s` | 387 | `hwinit/asm/pci/legacy.s` |
| `network/asm/pci/bar.s` | 402 | `hwinit/asm/pci/bar.s` |
| `network/asm/pci/capability.s` | 415 | `hwinit/asm/pci/capability.s` |
| `network/asm/pci/ecam.s` | 184 | `hwinit/asm/pci/ecam.s` |
| `network/asm/pci/virtio_cap.s` | 421 | `hwinit/asm/pci/virtio_cap.s` |

#### `network/asm/pci/legacy.s` - Lines 1-387 (MOVE ALL)
```asm
; Functions to move:
; - asm_pci_make_addr (line ~55-75)
; - asm_pci_cfg_read32 (line ~85-120)
; - asm_pci_cfg_write32 (line ~125-160)
; - asm_pci_cfg_read16 (line ~165-205)
; - asm_pci_cfg_write16 (line ~210-250)
; - asm_pci_cfg_read8 (line ~255-295)
; - asm_pci_cfg_write8 (line ~300-340)
; Constants: PCI_CONFIG_ADDR (0x0CF8), PCI_CONFIG_DATA (0x0CFC)
```

#### `network/asm/pci/bar.s` - Lines 1-402 (MOVE ALL)
```asm
; Functions to move:
; - asm_pci_read_bar32 - Read BAR value
; - asm_pci_write_bar32 - Write BAR value
; - asm_pci_probe_bar_size - Probe BAR size by writing 0xFFFFFFFF
; - asm_pci_is_bar_64bit - Check if BAR is 64-bit
; - asm_pci_is_bar_mmio - Check if BAR is MMIO (not I/O)
; - asm_pci_read_bar64 - Read 64-bit BAR value
```

#### `network/asm/pci/capability.s` - Lines 1-415 (MOVE ALL)
```asm
; Functions to move:
; - asm_pci_has_capabilities - Check STATUS.CAP_LIST bit
; - asm_pci_get_cap_ptr - Read CAP_PTR register
; - asm_pci_find_cap - Walk capability chain to find cap by ID
; - asm_pci_read_cap_id - Read capability ID at offset
; - asm_pci_read_cap_next - Read next capability pointer
```

#### `network/asm/pci/ecam.s` - Lines 1-184 (MOVE ALL)
```asm
; Functions to move:
; - asm_pcie_ecam_read32 - PCIe ECAM config read
; - asm_pcie_ecam_write32 - PCIe ECAM config write
; - asm_pcie_calc_ecam_addr - Calculate ECAM address
; Note: ECAM base discovery will be added in hwinit
```

#### `network/asm/pci/virtio_cap.s` - Lines 1-421 (MOVE ALL)
```asm
; Functions to move:
; - asm_pci_find_virtio_cap - Find VirtIO capability by cfg_type
; - asm_virtio_pci_parse_cap - Parse VirtIO capability structure
; - asm_virtio_pci_read_bar - Read BAR for VirtIO
; - asm_virtio_pci_probe_caps - Probe all VirtIO capabilities
; Note: VirtIO-specific but capability chain walking is generic
```

---

### 1.2 Core ASM Primitives (605 lines total)

| File | Lines | Move To |
|------|-------|---------|
| `network/asm/core/barriers.s` | 74 | `hwinit/asm/cpu/barriers.s` |
| `network/asm/core/cache.s` | 97 | `hwinit/asm/cpu/cache.s` |
| `network/asm/core/delay.s` | 113 | `hwinit/asm/cpu/delay.s` |
| `network/asm/core/mmio.s` | 125 | `hwinit/asm/mmio.s` |
| `network/asm/core/pio.s` | 129 | `hwinit/asm/pio.s` |
| `network/asm/core/tsc.s` | 67 | `hwinit/asm/cpu/tsc.s` |

#### `network/asm/core/barriers.s` - Lines 1-74 (MOVE ALL)
```asm
; Functions to move:
; - asm_bar_sfence (line ~38-41) - SFENCE instruction
; - asm_bar_lfence (line ~50-53) - LFENCE instruction  
; - asm_bar_mfence (line ~62-65) - MFENCE instruction
; Why generic: Memory barriers are used by ALL DMA devices
```

#### `network/asm/core/cache.s` - Lines 1-97 (MOVE ALL)
```asm
; Functions to move:
; - asm_cache_clflush (line ~40-43) - Flush single cache line
; - asm_cache_clflushopt (line ~55-58) - Optimized cache flush
; - asm_cache_flush_range (line ~70-95) - Flush range of memory
; Why generic: Cache coherence needed for ALL DMA devices
```

#### `network/asm/core/delay.s` - Lines 1-113 (MOVE ALL)
```asm
; Functions to move:
; - asm_delay_tsc - TSC-based delay
; - asm_delay_us - Microsecond delay
; - asm_delay_ms - Millisecond delay
; Why generic: Timing delays needed by all hardware init
```

#### `network/asm/core/mmio.s` - Lines 1-125 (MOVE ALL)
```asm
; Functions to move:
; - asm_mmio_read8/16/32 (lines ~45, 80, 32-45)
; - asm_mmio_write8/16/32 (lines ~55, 90, 50-65)
; Why generic: MMIO access needed by ALL MMIO devices
```

#### `network/asm/core/pio.s` - Lines 1-129 (MOVE ALL)
```asm
; Functions to move:
; - asm_pio_read8/16/32 - IN instructions
; - asm_pio_write8/16/32 - OUT instructions
; Why generic: Port I/O needed for PCI CF8/CFC, legacy devices
```

#### `network/asm/core/tsc.s` - Lines 1-67 (MOVE ALL)
```asm
; Functions to move:
; - asm_tsc_read - RDTSC instruction
; - asm_tsc_read_serialized - CPUID + RDTSC for serialization
; Why generic: TSC timing used by all drivers for timeouts
```

---

### 1.3 PCI Rust Wrappers (494 lines total)

| File | Lines | Move To |
|------|-------|---------|
| `network/src/pci/mod.rs` | 18 | `hwinit/src/pci/mod.rs` |
| `network/src/pci/config.rs` | 122 | `hwinit/src/pci/config.rs` |
| `network/src/pci/capability.rs` | 354 | `hwinit/src/pci/capability.rs` |

#### `network/src/pci/config.rs` - Lines 1-122 (MOVE ALL)

**Lines 1-27: ASM Bindings**
```rust
extern "win64" {
    fn asm_pci_cfg_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8;
    fn asm_pci_cfg_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16;
    fn asm_pci_cfg_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32;
    fn asm_pci_cfg_write8(bus: u8, device: u8, function: u8, offset: u8, value: u8);
    fn asm_pci_cfg_write16(bus: u8, device: u8, function: u8, offset: u8, value: u16);
    fn asm_pci_cfg_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32);
}
```

**Lines 33-48: PciAddr struct**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciAddr {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciAddr {
    pub const fn new(bus: u8, device: u8, function: u8) -> Self { ... }
}
```

**Lines 51-84: Config space access functions**
```rust
pub fn pci_cfg_read8(addr: PciAddr, offset: u8) -> u8 { ... }
pub fn pci_cfg_read16(addr: PciAddr, offset: u8) -> u16 { ... }
pub fn pci_cfg_read32(addr: PciAddr, offset: u8) -> u32 { ... }
pub fn pci_cfg_write8(addr: PciAddr, offset: u8, value: u8) { ... }
pub fn pci_cfg_write16(addr: PciAddr, offset: u8, value: u16) { ... }
pub fn pci_cfg_write32(addr: PciAddr, offset: u8, value: u32) { ... }
```

**Lines 90-120: PCI standard offsets**
```rust
pub mod offset {
    pub const VENDOR_ID: u8 = 0x00;
    pub const DEVICE_ID: u8 = 0x02;
    pub const COMMAND: u8 = 0x04;
    // ... all 20+ offset constants
}

pub mod status {
    pub const CAP_LIST: u16 = 1 << 4;
}
```

#### `network/src/pci/capability.rs` - Lines 1-354 (MOVE ALL)

**Lines 1-44: ASM bindings for capability walking**
```rust
extern "win64" {
    fn asm_pci_has_capabilities(...) -> u32;
    fn asm_pci_get_cap_ptr(...) -> u32;
    fn asm_pci_find_cap(...) -> u32;
    fn asm_pci_find_virtio_cap(...) -> u32;
    fn asm_virtio_pci_parse_cap(...) -> u32;
    fn asm_virtio_pci_read_bar(...) -> u64;
    fn asm_virtio_pci_probe_caps(...) -> u32;
}
```

**Lines 46-66: PCI capability ID constants**
```rust
pub const PCI_CAP_ID_VNDR: u8 = 0x09;
pub const VIRTIO_PCI_CAP_COMMON: u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
pub const VIRTIO_PCI_CAP_ISR: u8 = 3;
pub const VIRTIO_PCI_CAP_DEVICE: u8 = 4;
pub const VIRTIO_PCI_CAP_PCI_CFG: u8 = 5;
```

**Lines 68-90: VirtioCapInfo struct (24 bytes)**
```rust
#[repr(C)]
pub struct VirtioCapInfo {
    pub cfg_type: u8,
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
    pub notify_off_multiplier: u32,
    pub cap_offset: u8,
    ...
}
```

**Lines 92-150: VirtioPciCaps struct and impl**
```rust
pub struct VirtioPciCaps {
    pub common: Option<VirtioCapInfo>,
    pub notify: Option<VirtioCapInfo>,
    ...
}
impl VirtioPciCaps {
    pub fn has_required(&self) -> bool { ... }
    pub fn common_cfg_addr(&self) -> Option<u64> { ... }
    ...
}
```

**Lines 152-354: Public API functions**
```rust
pub fn has_capabilities(addr: PciAddr) -> bool { ... }
pub fn get_cap_ptr(addr: PciAddr) -> Option<u8> { ... }
pub fn find_cap(addr: PciAddr, cap_id: u8) -> Option<u8> { ... }
pub fn find_virtio_cap(addr: PciAddr, cfg_type: u8) -> Option<u8> { ... }
pub fn parse_virtio_cap(addr: PciAddr, cap_offset: u8) -> Option<VirtioCapInfo> { ... }
pub fn probe_virtio_caps(addr: PciAddr) -> VirtioPciCaps { ... }
```

---

### 1.4 Core Rust Wrappers (340 lines total)

| File | Lines | Move To |
|------|-------|---------|
| `network/src/asm/core/mod.rs` | 9 | `hwinit/src/cpu/mod.rs` |
| `network/src/asm/core/barriers.rs` | 57 | `hwinit/src/cpu/barriers.rs` |
| `network/src/asm/core/cache.rs` | 40 | `hwinit/src/cpu/cache.rs` |
| `network/src/asm/core/mmio.rs` | 96 | `hwinit/src/mmio.rs` |
| `network/src/asm/core/pio.rs` | 88 | `hwinit/src/pio.rs` |
| `network/src/asm/core/tsc.rs` | 50 | `hwinit/src/cpu/tsc.rs` |

#### `network/src/asm/core/barriers.rs` - Lines 1-57 (MOVE ALL)
```rust
// Lines 7-11: ASM bindings
extern "win64" {
    fn asm_bar_sfence();
    fn asm_bar_lfence();
    fn asm_bar_mfence();
}

// Lines 17-22: sfence()
pub fn sfence() { unsafe { asm_bar_sfence(); } }

// Lines 28-33: lfence()
pub fn lfence() { unsafe { asm_bar_lfence(); } }

// Lines 39-44: mfence()
pub fn mfence() { unsafe { asm_bar_mfence(); } }

// Lines 48-57: Stubs for non-x86_64
```

#### `network/src/asm/core/cache.rs` - Lines 1-40 (MOVE ALL)
```rust
// Lines 7-10: ASM bindings
extern "win64" {
    fn asm_cache_clflush(addr: u64);
    fn asm_cache_flush_range(addr: u64, len: u64);
}

// Lines 17-20: clflush()
pub unsafe fn clflush(addr: *const u8) { ... }

// Lines 26-30: flush_range()
pub unsafe fn flush_range(addr: *const u8, len: usize) { ... }

// Lines 35-40: Stubs for non-x86_64
```

#### `network/src/asm/core/mmio.rs` - Lines 1-96 (MOVE ALL)
```rust
// Lines 12-19: ASM bindings
extern "win64" {
    fn asm_mmio_read8(addr: u64) -> u8;
    fn asm_mmio_write8(addr: u64, value: u8);
    fn asm_mmio_read16(addr: u64) -> u16;
    fn asm_mmio_write16(addr: u64, value: u16);
    fn asm_mmio_read32(addr: u64) -> u32;
    fn asm_mmio_write32(addr: u64, value: u32);
}

// Lines 26-68: read8/write8/read16/write16/read32/write32
// Lines 72-96: Stubs for non-x86_64
```

#### `network/src/asm/core/pio.rs` - Lines 1-88 (MOVE ALL)
```rust
// Lines 10-17: ASM bindings
extern "win64" {
    fn asm_pio_read8(port: u16) -> u8;
    fn asm_pio_write8(port: u16, value: u8);
    fn asm_pio_read16(port: u16) -> u16;
    fn asm_pio_write16(port: u16, value: u16);
    fn asm_pio_read32(port: u16) -> u32;
    fn asm_pio_write32(port: u16, value: u32);
}

// Lines 24-60: inb/outb/inw/outw/inl/outl
// Lines 64-88: Stubs for non-x86_64
```

#### `network/src/asm/core/tsc.rs` - Lines 1-50 (MOVE ALL)
```rust
// Lines 10-14: ASM bindings
extern "win64" {
    fn asm_tsc_read() -> u64;
    fn asm_tsc_read_serialized() -> u64;
}

// Lines 21-25: read_tsc()
pub fn read_tsc() -> u64 { unsafe { asm_tsc_read() } }

// Lines 32-36: read_tsc_serialized()
pub fn read_tsc_serialized() -> u64 { unsafe { asm_tsc_read_serialized() } }

// Lines 42-50: Stubs for non-x86_64
```

---

## 2. Files to Partially Refactor

### 2.1 `network/src/boot/probe.rs` (319 lines)

| Lines | Type | Action |
|-------|------|--------|
| 1-30 | Imports | Update to use `hwinit::pci::*` |
| 33-45 | Constants (vendor/device IDs) | KEEP - network device specific |
| 48-74 | ProbeError enum | KEEP - network specific errors |
| 76-90 | DetectedNic enum | KEEP - network device info |
| 92-100 | ProbeResult enum | KEEP - network specific |
| **101-120** | `scan_for_nic()` | **REMOVE** - move to hwinit |
| **123-177** | `find_virtio_nic()` | **REMOVE** - PCI enumeration |
| 182-195 | `probe_and_create_driver()` signature | KEEP |
| **196-200** | Intel enable_device call | **REMOVE** - done by hwinit |
| 202-222 | Intel driver creation | KEEP |
| **225-227** | VirtIO enable device | **REMOVE** - done by hwinit |
| 229-245 | VirtIO driver creation | KEEP |
| 250-280 | `create_intel_driver()` | KEEP - factory function |
| 285-310 | `create_virtio_driver()` | KEEP - factory function |
| 312-319 | `detect_nic_type()` | **REMOVE** - move to hwinit |

**Specific Lines to Remove:**

```rust
// DELETE Lines 101-120
pub fn scan_for_nic() -> Option<DetectedNic> {
    if let Some(info) = find_intel_nic() { ... }
    if let Some((pci_addr, mmio_base)) = find_virtio_nic() { ... }
    None
}

// DELETE Lines 123-177
fn find_virtio_nic() -> Option<(PciAddr, u64)> {
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                // 55 lines of PCI enumeration
            }
        }
    }
    None
}

// DELETE Lines 196-200 (inside probe_and_create_driver)
// Enable device (bus mastering, memory space)
enable_device(info.pci_addr);

// DELETE Lines 225-227 (inside probe_and_create_driver)
let cmd = pci_cfg_read16(pci_addr, offset::COMMAND);
crate::pci::config::pci_cfg_write16(pci_addr, offset::COMMAND, cmd | 0x06);

// DELETE Lines 312-319
pub fn detect_nic_type() -> (NicType, Option<u64>, Option<PciAddr>) { ... }
```

---

### 2.2 `network/src/boot/block_probe.rs` (426 lines)

| Lines | Type | Action |
|-------|------|--------|
| 1-27 | Imports | Update to use `hwinit::pci::*` |
| 30-40 | Constants | KEEP - AHCI/VirtIO device IDs |
| 43-68 | BlockProbeError | KEEP |
| 70-95 | DetectedBlockDevice/AhciInfo | KEEP |
| 97-110 | BlockProbeResult | KEEP |
| **112-130** | `scan_for_block_device()` | **REMOVE** |
| **133-200** | `find_ahci_controller()` | **REMOVE** - PCI scan |
| **203-260** | `find_virtio_blk()` | **REMOVE** - PCI scan |
| 265-305 | BlockDmaConfig struct | KEEP - DMA layout config |
| **310-315** | `enable_pci_device()` | **REMOVE** - duplicate |
| 320-400 | `probe_and_create_block_driver()` | Refactor - remove enable calls |
| 405-426 | `detect_block_device_type()` | **REMOVE** |

**Specific Lines to Remove:**

```rust
// DELETE Lines 112-130
pub fn scan_for_block_device() -> Option<DetectedBlockDevice> { ... }

// DELETE Lines 133-200 (68 lines)
pub fn find_ahci_controller() -> Option<AhciInfo> {
    for bus in 0..=255u8 { ... }  // Full PCI enumeration
}

// DELETE Lines 203-260 (58 lines)
fn find_virtio_blk() -> Option<(PciAddr, u64)> {
    for bus in 0..=255u8 { ... }  // Full PCI enumeration
}

// DELETE Lines 310-315
fn enable_pci_device(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    pci_cfg_write16(addr, offset::COMMAND, cmd | 0x06);
}

// MODIFY Lines 329-330 - Remove enable call
// enable_pci_device(info.pci_addr);  // DELETE THIS LINE

// MODIFY Lines 354-355 - Remove enable call
// enable_pci_device(pci_addr);  // DELETE THIS LINE
```

---

### 2.3 `network/src/driver/intel/mod.rs`

**Lines to Remove:**

```rust
// DELETE Lines 155-178: read_bar_size() function
pub fn read_bar_size(addr: PciAddr, bar_index: u8) -> u32 {
    // 23 lines of BAR probing - this is generic PCI
}

// DELETE Lines 182-190: enable_device() function
pub fn enable_device(addr: PciAddr) {
    use crate::pci::config::pci_cfg_write16;
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    let new_cmd = cmd | 0x06;
    pci_cfg_write16(addr, offset::COMMAND, new_cmd);
}
```

---

### 2.4 `network/src/dma/region.rs` (182 lines)

**Lines to MOVE to hwinit (generic DMA region):**

```rust
// MOVE Lines 18-27: Core DmaRegion struct
pub struct DmaRegion {
    pub cpu_ptr: *mut u8,
    pub bus_addr: u64,
    pub size: usize,
}

// MOVE Lines 29-35: Generic constants
impl DmaRegion {
    pub const MIN_SIZE: usize = 2 * 1024 * 1024;
    pub const DEFAULT_QUEUE_SIZE: usize = 32;
    pub const DEFAULT_BUFFER_SIZE: usize = 2048;
}

// MOVE Lines 60-80: Generic methods
pub unsafe fn new(cpu_ptr: *mut u8, bus_addr: u64, size: usize) -> Self { ... }
pub fn cpu_base(&self) -> *mut u8 { ... }
pub fn bus_base(&self) -> u64 { ... }
pub fn size(&self) -> usize { ... }
```

**Lines to KEEP in network (VirtIO-specific layout):**
```rust
// KEEP Lines 42-57: VirtIO-specific offsets
pub const RX_DESC_OFFSET: usize = 0x0000;
pub const RX_AVAIL_OFFSET: usize = 0x0200;
pub const RX_USED_OFFSET: usize = 0x0400;
pub const TX_DESC_OFFSET: usize = 0x0800;
// ... etc

// KEEP Lines 82-182: VirtIO-specific accessor methods
pub fn rx_desc_cpu(&self) -> *mut u8 { ... }
pub fn rx_desc_bus(&self) -> u64 { ... }
// ... etc
```

---

### 2.5 `network/src/boot/handoff.rs` (582 lines)

**Lines to MOVE to hwinit:**

```rust
// MOVE Lines 26-28: Generic DMA constants
pub const MIN_DMA_SIZE: u64 = 2 * 1024 * 1024;
pub const MIN_STACK_SIZE: u64 = 64 * 1024;

// MOVE Lines 33-36: TSC frequency bounds
pub const MIN_TSC_FREQ: u64 = 1_000_000_000;
pub const MAX_TSC_FREQ: u64 = 10_000_000_000;

// MOVE Lines 92-97: Generic DMA validation errors
DmaRegionTooSmall,
DmaCpuPtrNull,
DmaBusAddrZero,
```

**Lines to KEEP in network:**
```rust
// KEEP Lines 40-65: NIC and block device type constants
pub const NIC_TYPE_NONE: u8 = 0;
pub const NIC_TYPE_VIRTIO: u8 = 1;
pub const NIC_TYPE_INTEL: u8 = 2;
pub const BLK_TYPE_NONE: u8 = 0;
pub const BLK_TYPE_VIRTIO: u8 = 1;
pub const BLK_TYPE_AHCI: u8 = 3;

// KEEP Lines 130-582: BootHandoff struct and validation
// This is network/boot specific handoff structure
```

---

## 3. Line-by-Line Extraction Guide

### Total Lines to Extract from network/

| Category | ASM Lines | Rust Lines | Total |
|----------|-----------|------------|-------|
| PCI (asm/pci/) | 1,809 | 0 | 1,809 |
| Core (asm/core/) | 605 | 0 | 605 |
| PCI (src/pci/) | 0 | 494 | 494 |
| Core (src/asm/core/) | 0 | 340 | 340 |
| boot/probe.rs | 0 | ~100 | 100 |
| boot/block_probe.rs | 0 | ~180 | 180 |
| driver/intel/mod.rs | 0 | ~35 | 35 |
| dma/region.rs | 0 | ~60 | 60 |
| boot/handoff.rs | 0 | ~20 | 20 |
| **TOTAL** | **2,414** | **1,229** | **3,643** |

---

## 4. hwinit Crate Implementation Plan

### 4.1 Directory Structure

```
hwinit/
├── Cargo.toml
├── build.rs                    # ASM compilation (copy from network/)
├── asm/
│   ├── cpu/
│   │   ├── barriers.s          # sfence, lfence, mfence
│   │   ├── cache.s             # clflush, flush_range
│   │   ├── delay.s             # TSC delays
│   │   └── tsc.s               # RDTSC
│   ├── pci/
│   │   ├── legacy.s            # CF8/CFC access
│   │   ├── bar.s               # BAR probing
│   │   ├── capability.s        # Capability chain
│   │   ├── ecam.s              # PCIe ECAM
│   │   └── virtio_cap.s        # VirtIO capability parsing
│   ├── mmio.s                  # MMIO read/write
│   ├── pio.s                   # Port I/O
│   └── dma/
│       └── iommu.s             # VT-d setup (NEW)
└── src/
    ├── lib.rs
    ├── cpu/
    │   ├── mod.rs
    │   ├── barriers.rs
    │   ├── cache.rs
    │   └── tsc.rs
    ├── pci/
    │   ├── mod.rs
    │   ├── config.rs           # PciAddr, pci_cfg_read/write
    │   ├── capability.rs       # Capability chain walking
    │   ├── bar.rs              # BAR probing
    │   └── enumerate.rs        # NEW: Full PCI enumeration
    ├── dma/
    │   ├── mod.rs
    │   ├── region.rs           # Generic DmaRegion
    │   ├── allocator.rs        # NEW: DMA allocator
    │   └── iommu.rs            # NEW: IOMMU/VT-d
    ├── mmio.rs
    ├── pio.rs
    └── platform.rs             # NEW: Platform init orchestrator
```

### 4.2 New Files to Create

#### `hwinit/src/pci/enumerate.rs` (NEW - ~200 lines)
```rust
//! PCI bus enumeration.
//! 
//! Scans entire PCI bus and builds device inventory.

use super::config::{PciAddr, pci_cfg_read16, pci_cfg_read32, offset};

/// Discovered PCI device
#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub addr: PciAddr,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u32,
    pub bars: [PciBar; 6],
    pub capabilities_ptr: Option<u8>,
}

/// BAR information
#[derive(Debug, Clone, Copy, Default)]
pub struct PciBar {
    pub base: u64,
    pub size: u64,
    pub is_mmio: bool,
    pub is_64bit: bool,
    pub is_prefetchable: bool,
}

/// Enumerate all PCI devices
pub fn enumerate_pci_bus() -> impl Iterator<Item = PciDevice> {
    PciEnumerator::new()
}

/// Enable bus mastering and memory space for device
pub fn enable_device(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    pci_cfg_write16(addr, offset::COMMAND, cmd | 0x07);  // MEM + IO + BusMaster
}

struct PciEnumerator { ... }
impl Iterator for PciEnumerator { ... }
```

#### `hwinit/src/dma/allocator.rs` (NEW - ~150 lines)
```rust
//! DMA memory allocator.
//!
//! Allocates DMA-capable memory with proper constraints:
//! - Identity mapped (physical == virtual)
//! - Below 4GB (for 32-bit DMA addressing)
//! - Properly aligned
//! - Cache coherent (UC/WC or with flush API)

use super::region::DmaRegion;

/// DMA allocation constraints
pub struct DmaConstraints {
    /// Maximum physical address (default: 4GB)
    pub max_addr: u64,
    /// Minimum alignment (default: 4KB)
    pub alignment: usize,
    /// Memory type (UC, WC, WB)
    pub memory_type: MemoryType,
}

#[derive(Debug, Clone, Copy)]
pub enum MemoryType {
    /// Uncached - no cache coherence needed
    Uncached,
    /// Write-combining - good for sequential writes
    WriteCombining,
    /// Write-back - requires cache flush for DMA
    WriteBack,
}

/// Allocate DMA region with constraints
pub unsafe fn allocate_dma_region(
    size: usize,
    constraints: &DmaConstraints,
) -> Option<DmaRegion> {
    // Implementation will use platform-specific allocation
    // For UEFI: AllocatePages with below-4GB constraint
    // For bare-metal: Use pre-reserved memory region
    todo!()
}

/// Validate that a region meets DMA requirements
pub fn validate_dma_region(region: &DmaRegion) -> Result<(), DmaValidationError> {
    // Check identity mapping
    // Check address < 4GB
    // Check alignment
    todo!()
}
```

#### `hwinit/src/dma/iommu.rs` (NEW - ~200 lines)
```rust
//! IOMMU (VT-d) detection and configuration.
//!
//! When IOMMU is active, DMA addresses != physical addresses.
//! This module detects IOMMU presence and configures identity mapping.

/// IOMMU detection result
pub enum IommuState {
    /// No IOMMU present - DMA uses physical addresses
    NotPresent,
    /// IOMMU present but disabled in BIOS
    Disabled,
    /// IOMMU active - need to configure mappings
    Active { base: u64 },
}

/// Detect IOMMU presence via ACPI DMAR table
pub fn detect_iommu() -> IommuState {
    // Parse ACPI tables to find DMAR
    // Check if VT-d is enabled
    todo!()
}

/// Configure identity mapping for DMA region
pub unsafe fn configure_identity_mapping(region: &DmaRegion) -> Result<(), IommuError> {
    // Program IOMMU page tables
    todo!()
}
```

#### `hwinit/src/platform.rs` (NEW - ~300 lines)
```rust
//! Platform initialization orchestrator.
//!
//! This is the main entry point for hardware initialization.
//! Performs all generic setup before any device-specific drivers run.

use crate::pci::{enumerate_pci_bus, PciDevice, enable_device};
use crate::dma::{DmaRegion, DmaConstraints, allocate_dma_region};
use crate::cpu::tsc;

/// Initialized platform state
pub struct Platform {
    /// TSC frequency (ticks per second)
    pub tsc_freq: u64,
    /// All discovered PCI devices
    pub pci_devices: alloc::vec::Vec<PciDevice>,
    /// Pre-allocated DMA regions
    pub dma_regions: alloc::vec::Vec<DmaRegion>,
}

/// Platform initialization error
#[derive(Debug)]
pub enum PlatformError {
    TscCalibrationFailed,
    NoDmaMemory,
    IommuConfigFailed,
}

/// Initialize platform hardware.
///
/// This function performs:
/// 1. TSC calibration
/// 2. PCI bus enumeration
/// 3. Enable bus mastering on all devices
/// 4. DMA region allocation
/// 5. IOMMU configuration (if present)
/// 6. Cache coherence setup
///
/// # Safety
/// Must be called before any device drivers.
/// Must be called after ExitBootServices (for UEFI platforms).
pub unsafe fn init_platform() -> Result<Platform, PlatformError> {
    // Step 1: Calibrate TSC
    let tsc_freq = tsc::calibrate()?;
    
    // Step 2: Enumerate PCI
    let devices: Vec<_> = enumerate_pci_bus().collect();
    
    // Step 3: Enable all devices (bus master + mem space)
    for device in &devices {
        enable_device(device.addr);
    }
    
    // Step 4: Allocate DMA regions
    let constraints = DmaConstraints {
        max_addr: 0xFFFF_FFFF,  // Below 4GB
        alignment: 4096,        // Page aligned
        memory_type: MemoryType::Uncached,
    };
    
    let mut dma_regions = Vec::new();
    // Allocate per-device DMA regions...
    
    // Step 5: IOMMU
    if let IommuState::Active { base } = detect_iommu() {
        for region in &dma_regions {
            configure_identity_mapping(region)?;
        }
    }
    
    Ok(Platform {
        tsc_freq,
        pci_devices: devices,
        dma_regions,
    })
}

/// Get device handle by vendor/device ID
impl Platform {
    pub fn find_device(&self, vendor: u16, device: u16) -> Option<&PciDevice> {
        self.pci_devices.iter().find(|d| 
            d.vendor_id == vendor && d.device_id == device
        )
    }
    
    pub fn find_network_devices(&self) -> impl Iterator<Item = &PciDevice> {
        self.pci_devices.iter().filter(|d| {
            // Class 0x02 = Network controller
            (d.class_code >> 24) == 0x02
        })
    }
    
    pub fn find_storage_devices(&self) -> impl Iterator<Item = &PciDevice> {
        self.pci_devices.iter().filter(|d| {
            // Class 0x01 = Mass storage
            (d.class_code >> 24) == 0x01
        })
    }
}
```

### 4.3 Cargo.toml

```toml
[package]
name = "morpheus-hwinit"
version = "1.0.0"
edition = "2021"
description = "Hardware initialization for MorpheusX bootloader"
license = "MIT OR Apache-2.0"

[lib]
name = "morpheus_hwinit"
path = "src/lib.rs"

[dependencies]
spin = "0.9"

[build-dependencies]
cc = "1.0"

[features]
default = []
# Enable IOMMU support
iommu = []
# Enable PCIe ECAM support (vs legacy CF8/CFC only)
pcie = []
```

---

## 5. API Contract Specification

### 5.1 What hwinit Guarantees

After `init_platform()` returns successfully:

| Guarantee | Implementation |
|-----------|----------------|
| TSC calibrated | `Platform.tsc_freq` is accurate within 0.1% |
| PCI enumerated | All devices discovered with vendor/device IDs |
| Bus mastering enabled | All devices have COMMAND.BME set |
| Memory space enabled | All devices have COMMAND.MSE set |
| BARs probed | All BAR addresses and sizes known |
| DMA below 4GB | All DMA regions have `bus_addr < 0x1_0000_0000` |
| Identity mapped | `cpu_ptr` as u64 == `bus_addr` for all DMA regions |
| Cache coherent | Either UC memory or flush API provided |
| IOMMU configured | If active, identity mappings in place |

### 5.2 What Network Stack Assumes

The network stack will receive:

```rust
/// Pre-initialized device info from hwinit
pub struct InitializedNic {
    /// PCI device (already enabled)
    pub pci: hwinit::pci::PciDevice,
    /// MMIO base address
    pub mmio_base: u64,
    /// Pre-allocated DMA region
    pub dma: hwinit::dma::DmaRegion,
}

// Network stack's new init signature:
impl E1000eDriver {
    pub fn new(nic: &InitializedNic, tsc_freq: u64) -> Result<Self, E1000eError> {
        // NO PCI access needed
        // NO bus master enable needed
        // NO DMA allocation needed
        // ONLY Intel-specific register programming
    }
}
```

---

## 6. Migration Execution Steps

### Phase 1: Create hwinit Skeleton (Day 1)

```bash
# Create crate
mkdir -p hwinit/src hwinit/asm

# Create Cargo.toml
cat > hwinit/Cargo.toml << 'EOF'
[package]
name = "morpheus-hwinit"
version = "1.0.0"
edition = "2021"
[lib]
name = "morpheus_hwinit"
[build-dependencies]
cc = "1.0"
EOF

# Copy ASM files
cp -r network/asm/pci hwinit/asm/
cp -r network/asm/core hwinit/asm/cpu
mv hwinit/asm/cpu/mmio.s hwinit/asm/
mv hwinit/asm/cpu/pio.s hwinit/asm/

# Copy build.rs and adapt
cp network/build.rs hwinit/build.rs
# Edit to point to new asm locations
```

### Phase 2: Move Rust Wrappers (Day 1-2)

```bash
# Create src structure
mkdir -p hwinit/src/{cpu,pci,dma}

# Move PCI module
cp network/src/pci/*.rs hwinit/src/pci/

# Move core wrappers
cp network/src/asm/core/barriers.rs hwinit/src/cpu/
cp network/src/asm/core/cache.rs hwinit/src/cpu/
cp network/src/asm/core/tsc.rs hwinit/src/cpu/
cp network/src/asm/core/mmio.rs hwinit/src/
cp network/src/asm/core/pio.rs hwinit/src/
```

### Phase 3: Build and Test hwinit (Day 2)

```bash
# Add to workspace
echo 'members = ["hwinit", ...]' >> Cargo.toml

# Build
cargo build -p morpheus-hwinit --target x86_64-unknown-uefi

# Fix any issues
```

### Phase 4: Update network to use hwinit (Day 3-4)

```rust
// network/Cargo.toml
[dependencies]
morpheus-hwinit = { path = "../hwinit" }

// network/src/lib.rs
// Remove: pub mod pci;
// Remove: pub mod asm::core;

// Update all imports:
// Old: use crate::pci::config::PciAddr;
// New: use morpheus_hwinit::pci::PciAddr;

// Old: use crate::asm::core::barriers::sfence;
// New: use morpheus_hwinit::cpu::barriers::sfence;
```

### Phase 5: Remove Duplicated PCI Enumeration (Day 4-5)

```rust
// network/src/boot/probe.rs
// Delete scan_for_nic(), find_virtio_nic()
// Modify probe_and_create_driver() to take pre-initialized device

// Old:
pub unsafe fn probe_and_create_driver(dma: &DmaRegion, tsc_freq: u64) -> Result<ProbeResult, ProbeError> {
    let detected = scan_for_nic().ok_or(ProbeError::NoDevice)?;
    enable_device(info.pci_addr);
    ...
}

// New:
pub unsafe fn create_driver_from_device(
    device: &hwinit::pci::PciDevice,
    dma: &hwinit::dma::DmaRegion,
    tsc_freq: u64,
) -> Result<ProbeResult, ProbeError> {
    // No enumeration, no enable - device already ready
    ...
}
```

### Phase 6: Implement Platform Init (Day 5-6)

```rust
// hwinit/src/platform.rs
// Implement init_platform()
// Test with QEMU first
// Then test on real ThinkPad T450s
```

### Phase 7: Clean Up (Day 7)

```bash
# Delete moved files from network/
rm -rf network/src/pci
rm -rf network/asm/pci
rm -rf network/src/asm/core
rm -rf network/asm/core

# Update network/src/lib.rs
# Update network/src/asm/mod.rs
# Full build and test
```

---

## 7. Testing Strategy

### 7.1 Unit Tests

```rust
// hwinit/tests/pci_test.rs
#[test]
fn test_pci_addr_creation() {
    let addr = PciAddr::new(0, 31, 0);
    assert_eq!(addr.bus, 0);
    assert_eq!(addr.device, 31);
    assert_eq!(addr.function, 0);
}

#[test]
fn test_config_space_address() {
    // Test CF8 address calculation
}
```

### 7.2 Integration Tests (QEMU)

```bash
# Test PCI enumeration
qemu-system-x86_64 -machine q35 -device virtio-net-pci ...
# Should enumerate VirtIO NIC

# Test with Intel emulation
qemu-system-x86_64 -device e1000e ...
# Should enumerate Intel NIC
```

### 7.3 Real Hardware Tests (ThinkPad T450s)

| Test | Pass Criteria |
|------|---------------|
| PCI enumeration | Intel I218 found at correct BDF |
| Bus master enable | COMMAND register shows 0x07 |
| BAR probing | MMIO base matches lspci output |
| DMA below 4GB | All dma.bus_addr < 0x1_0000_0000 |
| DHCP works | IP address obtained in <30s |

---

## Summary

This refactoring extracts **3,643 lines** of generic hardware code from the network stack into a dedicated `hwinit` crate. The network stack will be reduced to **only device-specific driver code**.

**Before:** Network stack does PCI enumeration, bus mastering, DMA allocation inline.
**After:** Network stack receives pre-initialized device handles from hwinit.

This fixes the root cause of "works in QEMU, fails on real hardware" bugs by ensuring:
1. Proper initialization order (platform before drivers)
2. DMA constraints enforced (below 4GB, identity mapped)
3. Cache coherence configured before any DMA
4. Bus mastering enabled before driver touches device
