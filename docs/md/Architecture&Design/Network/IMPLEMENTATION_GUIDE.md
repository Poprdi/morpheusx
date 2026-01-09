# MorpheusX Network Stack Implementation Guide

**Version**: 1.0  
**Status**: AUTHORITATIVE  
**Date**: January 2026  

---

## Document Hierarchy

This document provides concrete implementation guidance. It is subordinate to:

1. **NETWORK_STACK_AUDIT.md** — Authoritative corrections (Part 7 supersedes all)
2. **NETWORK_ASM_RUST_ABI_CONTRACT.md** — Frozen ABI specification
3. **NETWORK_STACK_REDESIGN.md** — Frozen architecture specification

Where conflicts exist, the hierarchy above determines precedence.

---

## Table of Contents

1. [Overview & Architecture](#1-overview--architecture)
2. [ASM Layer Specification](#2-asm-layer-specification)
3. [DMA & Buffer Management](#3-dma--buffer-management)
4. [VirtIO Driver Implementation](#4-virtio-driver-implementation)
5. [State Machines](#5-state-machines)
6. [Main Loop & Execution Model](#6-main-loop--execution-model)
7. [Boot Integration](#7-boot-integration)
8. [Driver Abstraction Layer](#8-driver-abstraction-layer)
9. [smoltcp Integration](#9-smoltcp-integration)
10. [Testing & Validation](#10-testing--validation)

---

# 1. Overview & Architecture

## 1.1 System Context

MorpheusX operates in a **post-ExitBootServices bare-metal environment**:

| Constraint | Implication |
|------------|-------------|
| No UEFI runtime services for networking | Must implement full stack |
| Single-core, no interrupts | Poll-driven execution only |
| No heap allocator (optional) | Pre-allocated buffers |
| No threads, no async runtime | Cooperative state machines |
| Direct hardware access | ASM layer for MMIO/PIO |

## 1.2 Execution Model

```
┌─────────────────────────────────────────────────────────────────┐
│                    SINGLE-THREADED POLL LOOP                    │
│                                                                 │
│   ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐        │
│   │ Phase 1 │ → │ Phase 2 │ → │ Phase 3 │ → │ Phase 4 │ → ...  │
│   │ RX Fill │   │ smoltcp │   │ TX Drain│   │ App Step│        │
│   └─────────┘   └─────────┘   └─────────┘   └─────────┘        │
│                                                                 │
│   Target: <1ms per iteration, Maximum: 5ms                     │
└─────────────────────────────────────────────────────────────────┘
```

**INVARIANT**: No function may block. All operations return immediately with status.

## 1.3 Layered Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     APPLICATION LAYER                           │
│          ISO Download State Machine, Progress UI                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     PROTOCOL LAYER                              │
│              HTTP State Machine, TCP, DHCP, DNS                 │
│                    (via smoltcp library)                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DEVICE LAYER                                │
│         NetworkDevice trait, DeviceAdapter for smoltcp          │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DRIVER LAYER                                │
│       VirtIO Driver (reference), Intel/Realtek (future)         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     ASM LAYER (Standalone)                      │
│   Generic: TSC, Barriers, MMIO │ Driver-Specific: VirtIO ops   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     HARDWARE                                    │
│              VirtIO-net, Intel e1000, Realtek RTL8168           │
└─────────────────────────────────────────────────────────────────┘
```

## 1.4 Key Design Decisions

| Decision | Rationale | Reference |
|----------|-----------|-----------|
| Standalone ASM (no inline) | Compiler cannot reorder; explicit barrier control | ABI Contract §1.2 |
| Poll-driven (no interrupts) | Simplicity; UEFI interrupt state undefined post-EBS | AUDIT §7.1.3 |
| State machines (no blocking) | Single thread cannot wait; must yield | REDESIGN §8 |
| Fire-and-forget TX | Completion collected separately; no send-wait | AUDIT §5.5.2 |
| Pre-allocated DMA | No allocator dependency; deterministic | REDESIGN §4.1 |
| smoltcp for TCP/IP | Battle-tested no_std stack; level-triggered poll | AUDIT §3.6 |

## 1.5 File Organization

```
network2/
├── asm/
│   ├── generic.s           # TSC, barriers, MMIO (9 functions)
│   └── virtio.s            # VirtIO-specific (11 functions)
├── src/
│   ├── lib.rs              # Crate root, re-exports
│   ├── asm/
│   │   ├── mod.rs          # ASM module
│   │   ├── bindings.rs     # extern "win64" declarations
│   │   └── types.rs        # VirtqueueState, RxResult structs
│   ├── dma/
│   │   ├── mod.rs          # DMA module
│   │   ├── buffer.rs       # DmaBuffer with ownership
│   │   └── pool.rs         # BufferPool management
│   ├── device/
│   │   ├── mod.rs          # NetworkDevice trait
│   │   ├── factory.rs      # Auto-detection, UnifiedNetDevice
│   │   └── virtio.rs       # VirtIO-net driver
│   ├── stack/
│   │   ├── mod.rs          # Stack module
│   │   ├── adapter.rs      # smoltcp Device impl
│   │   └── interface.rs    # NetInterface wrapper
│   ├── state/
│   │   ├── mod.rs          # State machine module
│   │   ├── dhcp.rs         # DhcpState
│   │   ├── tcp.rs          # TcpConnState
│   │   └── http.rs         # HttpDownloadState
│   ├── time/
│   │   ├── mod.rs          # Time module
│   │   └── timeout.rs      # TimeoutConfig
│   └── mainloop.rs         # 5-phase main loop
├── build.rs                # NASM assembly compilation
└── Cargo.toml
```

## 1.6 Forbidden Patterns

These patterns are **strictly prohibited** anywhere in the codebase:

```rust
// ❌ FORBIDDEN: Blocking loop
while !condition {
    do_work();
}

// ❌ FORBIDDEN: Busy-wait with delay
while time_elapsed < timeout {
    spin_loop();
}

// ❌ FORBIDDEN: Multiple smoltcp polls per iteration
loop {
    iface.poll(...);
    // ... other work ...
    iface.poll(...);  // WRONG - only once per iteration
}

// ❌ FORBIDDEN: Inline assembly
unsafe { core::arch::asm!("mfence"); }  // Use asm_bar_mfence() instead

// ❌ FORBIDDEN: Hardcoded TSC frequency
const TSC_FREQ: u64 = 2_500_000_000;  // Use calibrated value

// ❌ FORBIDDEN: TX completion wait
fn transmit(&mut self, pkt: &[u8]) {
    self.submit(pkt);
    while !self.tx_complete() { }  // NEVER WAIT
}
```

## 1.7 Required Patterns

These patterns **must be used** for correctness:

```rust
// ✅ REQUIRED: State machine with step()
pub fn step(&mut self, now_tsc: u64, timeout: u64) -> StepResult {
    if now_tsc.wrapping_sub(self.start_tsc) > timeout {
        return StepResult::Timeout;
    }
    // Check condition, transition if met, return immediately
    StepResult::Pending
}

// ✅ REQUIRED: Fire-and-forget TX
pub fn transmit(&mut self, pkt: &[u8]) -> Result<(), TxError> {
    if !self.can_transmit() {
        return Err(TxError::QueueFull);  // Backpressure
    }
    self.submit_tx(pkt)?;  // Returns immediately
    Ok(())  // Completion collected in Phase 5
}

// ✅ REQUIRED: Timeout as observation
let elapsed = now_tsc.wrapping_sub(start_tsc);
if elapsed > timeout_ticks {
    // Handle timeout
}

// ✅ REQUIRED: ASM for all hardware access
let value = unsafe { asm_mmio_read32(register_addr) };
unsafe { asm_bar_mfence(); }
unsafe { asm_mmio_write32(notify_addr, queue_index) };
```

---

# 2. ASM Layer Specification

## 2.1 Design Principles

The ASM layer provides:

1. **Guaranteed memory ordering** — Compiler cannot reorder across ASM calls
2. **Explicit barrier placement** — Developer controls when barriers execute
3. **Volatile hardware access** — MMIO reads/writes not optimized away
4. **Microsoft x64 ABI** — Compatible with UEFI calling convention

**Reference**: NETWORK_ASM_RUST_ABI_CONTRACT.md (FROZEN v1.0)

## 2.2 Function Inventory

### 2.2.1 Generic Functions (9) — Reusable by All Drivers

| Symbol | Parameters | Returns | Purpose |
|--------|------------|---------|---------|
| `asm_tsc_read` | None | `RAX: u64` | Read TSC (~40 cycles) |
| `asm_tsc_read_serialized` | None | `RAX: u64` | TSC with CPUID serialize (~200 cycles) |
| `asm_bar_sfence` | None | None | Store fence |
| `asm_bar_lfence` | None | None | Load fence |
| `asm_bar_mfence` | None | None | Full memory fence |
| `asm_mmio_read32` | `RCX: addr` | `RAX: u32` | 32-bit MMIO read |
| `asm_mmio_write32` | `RCX: addr, RDX: val` | None | 32-bit MMIO write |
| `asm_mmio_read16` | `RCX: addr` | `RAX: u16` | 16-bit MMIO read |
| `asm_mmio_write16` | `RCX: addr, RDX: val` | None | 16-bit MMIO write |

### 2.2.2 VirtIO Functions (11) — VirtIO Driver Only

| Symbol | Parameters | Returns | Purpose |
|--------|------------|---------|---------|
| `asm_vq_submit_tx` | `RCX: *VqState, RDX: idx, R8: len` | `RAX: 0=ok, 1=full` | Submit TX with barriers |
| `asm_vq_poll_tx_complete` | `RCX: *VqState` | `RAX: idx or 0xFFFFFFFF` | Poll TX used ring |
| `asm_vq_submit_rx` | `RCX: *VqState, RDX: idx, R8: cap` | `RAX: 0=ok, 1=full` | Submit RX with barriers |
| `asm_vq_poll_rx` | `RCX: *VqState, RDX: *RxResult` | `RAX: 0=empty, 1=pkt` | Poll RX used ring |
| `asm_vq_notify` | `RCX: *VqState` | None | Notify device (mfence + MMIO) |
| `asm_nic_reset` | `RCX: mmio_base` | `RAX: 0=ok, 1=timeout` | Reset device (≤100ms) |
| `asm_nic_set_status` | `RCX: mmio_base, RDX: status` | None | Write status register |
| `asm_nic_get_status` | `RCX: mmio_base` | `RAX: u8` | Read status register |
| `asm_nic_read_features` | `RCX: mmio_base` | `RAX: u64` | Read feature bits |
| `asm_nic_write_features` | `RCX: mmio_base, RDX: features` | None | Write feature bits |
| `asm_nic_read_mac` | `RCX: mmio_base, RDX: *[u8;6]` | `RAX: 0=ok, 1=unavail` | Read MAC address |

## 2.3 Calling Convention (Microsoft x64)

```
Parameters:  RCX, RDX, R8, R9 (first 4 integer/pointer args)
Return:      RAX (integer), XMM0 (float)
Volatile:    RAX, RCX, RDX, R8, R9, R10, R11
Non-volatile: RBX, RBP, RDI, RSI, R12-R15
Stack:       16-byte aligned, 32-byte shadow space
```

## 2.4 Memory Barrier Contracts

### TX Submit Sequence (asm_vq_submit_tx)

```asm
; Internal barrier sequence:
; 1. Write descriptor (addr, len, flags, next)
; 2. SFENCE - ensure descriptor visible
; 3. Write avail.ring[avail.idx & mask] = desc_idx
; 4. SFENCE - ensure ring entry visible  
; 5. Write avail.idx += 1
; 6. MFENCE - full barrier before notify decision
; 7. IF notify needed: MMIO write to notify register
```

### RX Poll Sequence (asm_vq_poll_rx)

```asm
; Internal barrier sequence:
; 1. Read used.idx (volatile)
; 2. Compare with last_seen_used_idx
; 3. If equal: return 0 (no packet)
; 4. LFENCE - ensure index read completes
; 5. Read used.ring[last_seen & mask] → (desc_idx, len)
; 6. LFENCE - ensure ring entry read before buffer access
; 7. Return 1, populate RxResult
```

## 2.5 Rust Bindings

```rust
// src/asm/bindings.rs

//! ASM function bindings for MorpheusX network stack.
//! 
//! All functions use Microsoft x64 calling convention (extern "win64").
//! SAFETY: See individual function documentation for preconditions.

use crate::asm::types::{VirtqueueState, RxResult};

// ═══════════════════════════════════════════════════════════════
// GENERIC FUNCTIONS (usable by all drivers)
// ═══════════════════════════════════════════════════════════════

extern "win64" {
    /// Read Time Stamp Counter (non-serializing, ~40 cycles).
    /// 
    /// # Safety
    /// Always safe. Requires invariant TSC (verify at boot via CPUID).
    pub fn asm_tsc_read() -> u64;

    /// Read TSC with full CPU serialization via CPUID (~200 cycles).
    /// 
    /// # Safety
    /// Always safe. Use only when precise measurement needed.
    pub fn asm_tsc_read_serialized() -> u64;

    /// Store fence - ensures all prior stores are globally visible.
    pub fn asm_bar_sfence();

    /// Load fence - ensures all prior loads complete before subsequent.
    pub fn asm_bar_lfence();

    /// Full memory fence - ensures all prior loads AND stores complete.
    pub fn asm_bar_mfence();

    /// Read 32-bit value from MMIO address.
    /// 
    /// # Safety
    /// - `addr` must be valid MMIO address (4-byte aligned)
    /// - Address must be mapped with appropriate attributes
    pub fn asm_mmio_read32(addr: u64) -> u32;

    /// Write 32-bit value to MMIO address.
    /// 
    /// # Safety
    /// - `addr` must be valid MMIO address (4-byte aligned)
    pub fn asm_mmio_write32(addr: u64, value: u32);

    /// Read 16-bit value from MMIO address.
    pub fn asm_mmio_read16(addr: u64) -> u16;

    /// Write 16-bit value to MMIO address.
    pub fn asm_mmio_write16(addr: u64, value: u16);
}

// ═══════════════════════════════════════════════════════════════
// VIRTIO FUNCTIONS (VirtIO driver only)
// ═══════════════════════════════════════════════════════════════

extern "win64" {
    /// Submit buffer to TX virtqueue with correct barrier sequence.
    /// 
    /// # Safety
    /// - `vq` must point to valid, initialized VirtqueueState
    /// - `buffer_idx` must be < queue_size
    /// - Buffer at index must be DRIVER-OWNED
    /// - Buffer must contain valid 12-byte VirtIO header + payload
    /// 
    /// # Returns
    /// - 0: Success (buffer now DEVICE-OWNED)
    /// - 1: Queue full (buffer remains DRIVER-OWNED)
    pub fn asm_vq_submit_tx(
        vq: *mut VirtqueueState,
        buffer_idx: u16,
        buffer_len: u16,
    ) -> u32;

    /// Poll TX used ring for completed transmissions.
    /// 
    /// # Safety
    /// - `vq` must point to valid VirtqueueState
    /// 
    /// # Returns
    /// - 0x0000..0xFFFE: Buffer index now DRIVER-OWNED
    /// - 0xFFFFFFFF: No completion available
    pub fn asm_vq_poll_tx_complete(vq: *mut VirtqueueState) -> u32;

    /// Submit empty buffer to RX virtqueue for receiving.
    /// 
    /// # Safety
    /// - `vq` must point to valid VirtqueueState
    /// - `buffer_idx` must be < queue_size
    /// - Buffer must be DRIVER-OWNED
    /// - `capacity` must be ≥ 1526 (12-byte header + 1514 max frame)
    /// 
    /// # Returns
    /// - 0: Success (buffer now DEVICE-OWNED)
    /// - 1: Queue full
    pub fn asm_vq_submit_rx(
        vq: *mut VirtqueueState,
        buffer_idx: u16,
        capacity: u16,
    ) -> u32;

    /// Poll RX used ring for received packets.
    /// 
    /// # Safety
    /// - `vq` must point to valid VirtqueueState
    /// - `result` must point to valid RxResult
    /// 
    /// # Returns
    /// - 0: No packet available
    /// - 1: Packet received (result populated, buffer now DRIVER-OWNED)
    pub fn asm_vq_poll_rx(
        vq: *mut VirtqueueState,
        result: *mut RxResult,
    ) -> u32;

    /// Notify device that buffers are available.
    /// 
    /// # Safety
    /// - `vq` must point to valid VirtqueueState with notify_addr set
    /// 
    /// # Note
    /// Includes mfence before MMIO write.
    pub fn asm_vq_notify(vq: *mut VirtqueueState);

    /// Reset VirtIO device.
    /// 
    /// # Safety
    /// - `mmio_base` must be valid VirtIO MMIO base address
    /// 
    /// # Returns
    /// - 0: Reset successful
    /// - 1: Timeout (device did not reset within 100ms) - FATAL
    /// 
    /// # Note
    /// This is the ONLY ASM function that may block (bounded at 100ms).
    pub fn asm_nic_reset(mmio_base: u64) -> u32;

    /// Write VirtIO device status register.
    pub fn asm_nic_set_status(mmio_base: u64, status: u8);

    /// Read VirtIO device status register.
    pub fn asm_nic_get_status(mmio_base: u64) -> u8;

    /// Read VirtIO device feature bits (64-bit).
    pub fn asm_nic_read_features(mmio_base: u64) -> u64;

    /// Write driver-accepted feature bits.
    pub fn asm_nic_write_features(mmio_base: u64, features: u64);

    /// Read MAC address from VirtIO config space.
    /// 
    /// # Safety
    /// - `mac_out` must point to valid [u8; 6]
    /// 
    /// # Returns
    /// - 0: Success
    /// - 1: MAC feature not negotiated
    pub fn asm_nic_read_mac(mmio_base: u64, mac_out: *mut [u8; 6]) -> u32;
}
```

## 2.6 Shared Types

```rust
// src/asm/types.rs

/// Virtqueue state passed to ASM functions.
/// Must match ASM layout exactly.
#[repr(C)]
pub struct VirtqueueState {
    /// Base address of descriptor table (bus address for device)
    pub desc_base: u64,
    /// Base address of available ring
    pub avail_base: u64,
    /// Base address of used ring
    pub used_base: u64,
    /// Queue size (number of descriptors)
    pub queue_size: u16,
    /// Queue index (0=RX, 1=TX for virtio-net)
    pub queue_index: u16,
    /// Padding for alignment
    pub _pad: u32,
    /// MMIO address for queue notification
    pub notify_addr: u64,
    /// Last seen used index (for polling)
    pub last_used_idx: u16,
    /// Next available index (for submission)
    pub next_avail_idx: u16,
    /// Padding
    pub _pad2: u32,
    /// CPU pointer to descriptor table (for driver access)
    pub desc_cpu_ptr: u64,
    /// CPU pointer to buffer region
    pub buffer_cpu_base: u64,
    /// Bus address of buffer region
    pub buffer_bus_base: u64,
    /// Size of each buffer
    pub buffer_size: u32,
    /// Number of buffers
    pub buffer_count: u32,
}

/// Result from asm_vq_poll_rx.
#[repr(C)]
pub struct RxResult {
    /// Index of buffer containing received packet
    pub buffer_idx: u16,
    /// Length of received data (including 12-byte header)
    pub length: u16,
    /// Reserved for future use
    pub _reserved: u32,
}

/// VirtIO network header (12 bytes for modern devices).
#[repr(C)]
pub struct VirtioNetHdr {
    pub flags: u8,           // 0: no flags
    pub gso_type: u8,        // 0: VIRTIO_NET_HDR_GSO_NONE
    pub hdr_len: u16,        // 0: no header length hint
    pub gso_size: u16,       // 0: no GSO
    pub csum_start: u16,     // 0: no checksum offload
    pub csum_offset: u16,    // 0: no checksum offload
    pub num_buffers: u16,    // Only with MRG_RXBUF (we don't use)
}

impl VirtioNetHdr {
    /// Create zeroed header (correct for all our transmits).
    pub const fn zeroed() -> Self {
        Self {
            flags: 0,
            gso_type: 0,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
            num_buffers: 0,
        }
    }

    /// Header size in bytes.
    pub const SIZE: usize = 12;
}
```

## 2.7 Build Integration

```rust
// build.rs

use std::process::Command;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=asm/generic.s");
    println!("cargo:rerun-if-changed=asm/virtio.s");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    
    // Assemble generic.s
    let status = Command::new("nasm")
        .args(&[
            "-f", "win64",
            "-o", &format!("{}/generic.o", out_dir),
            "asm/generic.s",
        ])
        .status()
        .expect("Failed to run nasm for generic.s");
    assert!(status.success(), "NASM failed for generic.s");

    // Assemble virtio.s
    let status = Command::new("nasm")
        .args(&[
            "-f", "win64",
            "-o", &format!("{}/virtio.o", out_dir),
            "asm/virtio.s",
        ])
        .status()
        .expect("Failed to run nasm for virtio.s");
    assert!(status.success(), "NASM failed for virtio.s");

    // Create static library
    let status = Command::new("ar")
        .args(&[
            "rcs",
            &format!("{}/libnetwork_asm.a", out_dir),
            &format!("{}/generic.o", out_dir),
            &format!("{}/virtio.o", out_dir),
        ])
        .status()
        .expect("Failed to run ar");
    assert!(status.success(), "ar failed");

    println!("cargo:rustc-link-search=native={}", out_dir);
    println!("cargo:rustc-link-lib=static=network_asm");
}
```

---

# 3. DMA & Buffer Management

## 3.1 Design Principles

DMA (Direct Memory Access) allows the NIC to read/write memory without CPU involvement. This requires:

1. **Physical address visibility** — Device sees bus addresses, not virtual
2. **Cache coherency** — CPU caches must not hide device writes
3. **Ownership tracking** — Prevent use-after-submit bugs
4. **Alignment** — Hardware requires specific alignments

**Reference**: AUDIT §7.2.1, §7.2.2, REDESIGN §4.1

## 3.2 Memory Allocation Strategy

### Pre-ExitBootServices (UEFI Active)

```rust
/// Allocate DMA-capable memory using UEFI PCI I/O Protocol.
/// 
/// This is the ONLY correct method per AUDIT §7.2.1 (correction).
/// Raw AllocatePages() does NOT handle IOMMU.
fn allocate_dma_region(
    pci_io: &PciRootBridgeIoProtocol,
    size: usize,
) -> Result<DmaRegion, AllocationError> {
    // 1. Allocate buffer (handles IOMMU internally)
    let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
    let cpu_addr = pci_io.allocate_buffer(
        AllocationType::MaxAddress(0xFFFF_FFFF),  // Below 4GB for 32-bit DMA
        MemoryType::EfiBootServicesData,
        pages,
    )?;
    
    // 2. Map to get device-visible address
    let (bus_addr, mapping) = pci_io.map(
        PciIoOperation::BusMasterCommonBuffer,  // Bidirectional
        cpu_addr,
        size,
    )?;
    
    // 3. Set cache attributes (UC or WC for coherency)
    // NOTE: May require page table manipulation post-EBS
    
    Ok(DmaRegion {
        cpu_ptr: cpu_addr as *mut u8,
        bus_addr,
        size,
        mapping_token: mapping,
    })
}
```

### Post-ExitBootServices Considerations

After ExitBootServices:
- Cannot allocate new DMA memory
- May need to remap as Uncached (UC) via page tables
- Must use pre-allocated regions from BootHandoff

## 3.3 Memory Layout (2MB Region)

```
Offset      Size        Content                     Notes
────────────────────────────────────────────────────────────────────
0x00000     0x0200      RX Descriptor Table         32 × 16 bytes
0x00200     0x0048      RX Available Ring           4 + 32×2 + 2 pad
0x00400     0x0108      RX Used Ring                4 + 32×8 + 2 pad
0x00800     0x0200      TX Descriptor Table         32 × 16 bytes
0x00A00     0x0048      TX Available Ring           
0x00C00     0x0108      TX Used Ring                
0x01000     0x10000     RX Buffers                  32 × 2KB = 64KB
0x11000     0x10000     TX Buffers                  32 × 2KB = 64KB
0x21000     ...         Reserved                    ~1.87MB remaining
────────────────────────────────────────────────────────────────────
Minimum:    0x21000 (132KB used of 2MB allocation)
```

### VirtIO Descriptor Format (16 bytes)

```rust
#[repr(C)]
pub struct VirtqDesc {
    /// Buffer physical/bus address
    pub addr: u64,
    /// Buffer length in bytes
    pub len: u32,
    /// Flags (NEXT=1, WRITE=2, INDIRECT=4)
    pub flags: u16,
    /// Next descriptor index (if NEXT flag set)
    pub next: u16,
}
```

### Available Ring Layout

```rust
#[repr(C)]
pub struct VirtqAvail {
    pub flags: u16,              // 0: no interrupt suppression
    pub idx: u16,                // Next available index (increments)
    pub ring: [u16; QUEUE_SIZE], // Descriptor indices
    pub used_event: u16,         // Event suppression (optional)
}
```

### Used Ring Layout

```rust
#[repr(C)]
pub struct VirtqUsed {
    pub flags: u16,              // 0: no notification suppression
    pub idx: u16,                // Next used index (device increments)
    pub ring: [VirtqUsedElem; QUEUE_SIZE],
}

#[repr(C)]
pub struct VirtqUsedElem {
    pub id: u32,   // Descriptor chain head index
    pub len: u32,  // Total bytes written (RX) or consumed (TX)
}
```

## 3.4 Buffer Ownership Model

```
                    BUFFER OWNERSHIP STATE MACHINE
                    
              ┌─────────┐
              │  FREE   │  Not allocated to any operation
              └────┬────┘
                   │ pool.alloc()
                   ▼
          ┌──────────────────┐
          │   DRIVER-OWNED   │  Rust code may read/write buffer
          │   (CPU Access)   │  
          └────────┬─────────┘
                   │ asm_vq_submit_tx() or asm_vq_submit_rx()
                   │ [Ownership transferred to device]
                   ▼
          ┌──────────────────┐
          │   DEVICE-OWNED   │  *** DRIVER MUST NOT ACCESS ***
          │   (DMA Active)   │  Any access is UNDEFINED BEHAVIOR
          └────────┬─────────┘
                   │ asm_vq_poll_tx_complete() or asm_vq_poll_rx()
                   │ [Ownership returned to driver]
                   ▼
          ┌──────────────────┐
          │   DRIVER-OWNED   │  Rust code may read/write buffer
          └────────┬─────────┘
                   │ pool.free() [optional, for reuse]
                   ▼
              ┌─────────┐
              │  FREE   │
              └─────────┘

INVARIANT DMA-OWN-1: Each buffer is in EXACTLY ONE state at any time.
INVARIANT DMA-OWN-2: Only ASM functions may transition ownership.
INVARIANT DMA-OWN-3: Accessing DEVICE-OWNED buffer is instant UB.
```

## 3.5 Buffer Pool Implementation

```rust
// src/dma/buffer.rs

/// Ownership state of a DMA buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferOwnership {
    /// Available for allocation
    Free,
    /// Owned by driver (CPU may access)
    DriverOwned,
    /// Owned by device (NO ACCESS ALLOWED)
    DeviceOwned,
}

/// A single DMA buffer with ownership tracking.
pub struct DmaBuffer {
    /// CPU-accessible pointer
    cpu_ptr: *mut u8,
    /// Device-visible bus address
    bus_addr: u64,
    /// Buffer capacity in bytes
    capacity: usize,
    /// Current ownership state
    ownership: BufferOwnership,
    /// Buffer index in pool
    index: u16,
}

impl DmaBuffer {
    /// Get CPU pointer. PANICS if not DriverOwned.
    pub fn as_slice(&self) -> &[u8] {
        assert_eq!(self.ownership, BufferOwnership::DriverOwned,
            "BUG: Accessing buffer not owned by driver");
        unsafe { core::slice::from_raw_parts(self.cpu_ptr, self.capacity) }
    }

    /// Get mutable CPU pointer. PANICS if not DriverOwned.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        assert_eq!(self.ownership, BufferOwnership::DriverOwned,
            "BUG: Accessing buffer not owned by driver");
        unsafe { core::slice::from_raw_parts_mut(self.cpu_ptr, self.capacity) }
    }

    /// Bus address for device descriptor.
    pub fn bus_addr(&self) -> u64 {
        self.bus_addr
    }

    /// Buffer index.
    pub fn index(&self) -> u16 {
        self.index
    }

    /// Transition to DeviceOwned. Called by ASM submit functions.
    /// 
    /// # Safety
    /// Only call immediately before ASM submit.
    pub(crate) unsafe fn mark_device_owned(&mut self) {
        debug_assert_eq!(self.ownership, BufferOwnership::DriverOwned);
        self.ownership = BufferOwnership::DeviceOwned;
    }

    /// Transition to DriverOwned. Called after ASM poll returns buffer.
    /// 
    /// # Safety
    /// Only call immediately after ASM poll confirms ownership transfer.
    pub(crate) unsafe fn mark_driver_owned(&mut self) {
        debug_assert_eq!(self.ownership, BufferOwnership::DeviceOwned);
        self.ownership = BufferOwnership::DriverOwned;
    }
}
```

```rust
// src/dma/pool.rs

/// Pre-allocated buffer pool for a virtqueue.
pub struct BufferPool {
    buffers: [DmaBuffer; 32],
    free_list: [u16; 32],
    free_count: usize,
}

impl BufferPool {
    /// Create pool from DMA region.
    /// 
    /// # Arguments
    /// - `cpu_base`: CPU pointer to buffer region start
    /// - `bus_base`: Bus address of buffer region start
    /// - `buffer_size`: Size of each buffer (must be ≥ 1526 for RX)
    /// - `buffer_count`: Number of buffers (typically 32)
    pub fn new(
        cpu_base: *mut u8,
        bus_base: u64,
        buffer_size: usize,
        buffer_count: usize,
    ) -> Self {
        // Initialize buffers and free list
        // ...
    }

    /// Allocate a buffer. Returns None if pool exhausted.
    pub fn alloc(&mut self) -> Option<&mut DmaBuffer> {
        if self.free_count == 0 {
            return None;
        }
        self.free_count -= 1;
        let idx = self.free_list[self.free_count] as usize;
        let buf = &mut self.buffers[idx];
        debug_assert_eq!(buf.ownership, BufferOwnership::Free);
        buf.ownership = BufferOwnership::DriverOwned;
        Some(buf)
    }

    /// Return buffer to pool.
    pub fn free(&mut self, buf: &mut DmaBuffer) {
        debug_assert_eq!(buf.ownership, BufferOwnership::DriverOwned);
        buf.ownership = BufferOwnership::Free;
        self.free_list[self.free_count] = buf.index;
        self.free_count += 1;
    }

    /// Get buffer by index. For completion handling.
    pub fn get_mut(&mut self, index: u16) -> &mut DmaBuffer {
        &mut self.buffers[index as usize]
    }
}
```

## 3.6 Cache Coherency

### The Problem

x86 CPUs cache memory by default (Write-Back mode). When device writes via DMA:
1. Device writes to RAM
2. CPU cache still contains stale data
3. CPU read returns stale cached value

### Solutions (in preference order)

| Method | How | When |
|--------|-----|------|
| **UC Memory** | Map DMA region as Uncached | Best: hardware enforces |
| **WC Memory** | Map as Write-Combining | Good: allows write coalescing |
| **CLFLUSH** | Explicit cache line flush | Fallback: per-access overhead |

### Implementation Notes

```rust
/// Ensure buffer is visible to device before submit.
/// 
/// Required if DMA region is Write-Back (WB) instead of UC/WC.
#[inline]
pub fn flush_buffer_for_device(cpu_ptr: *const u8, len: usize) {
    // If UC/WC mapped: no-op (hardware handles coherency)
    // If WB mapped: flush cache lines
    #[cfg(feature = "wb_dma")]
    {
        let start = cpu_ptr as usize & !63;  // Cache line align
        let end = (cpu_ptr as usize + len + 63) & !63;
        for addr in (start..end).step_by(64) {
            unsafe { core::arch::x86_64::_mm_clflush(addr as *const u8); }
        }
        unsafe { asm_bar_sfence(); }
    }
}

/// Invalidate cache before reading buffer filled by device.
/// 
/// Required if DMA region is WB instead of UC/WC.
#[inline]
pub fn invalidate_buffer_from_device(cpu_ptr: *const u8, len: usize) {
    #[cfg(feature = "wb_dma")]
    {
        let start = cpu_ptr as usize & !63;
        let end = (cpu_ptr as usize + len + 63) & !63;
        for addr in (start..end).step_by(64) {
            unsafe { core::arch::x86_64::_mm_clflush(addr as *const u8); }
        }
        unsafe { asm_bar_lfence(); }
    }
}
```

## 3.7 Invariants

| ID | Invariant | Verification |
|----|-----------|--------------|
| **DMA-1** | All DMA memory allocated via PCI I/O Protocol | Code review |
| **DMA-2** | DMA region is UC or WC mapped (preferred) | Runtime check impossible; design |
| **DMA-3** | bus_addr used in descriptors, cpu_ptr for driver access | Code review |
| **DMA-4** | Each buffer in exactly one ownership state | Debug assertions |
| **DMA-5** | DEVICE-OWNED buffers never accessed | Debug assertions + design |
| **DMA-6** | Buffer capacity ≥ 1526 for RX (12 + 1514) | Constructor check |
| **DMA-7** | Descriptor tables page-aligned (4096) | Constructor check |

---

# 4. VirtIO Driver Implementation

## 4.1 VirtIO Overview

VirtIO is a standard interface for virtual devices. VirtIO-net provides network connectivity in VMs.

**Reference**: VirtIO Specification v1.2, AUDIT §7.2

### Key Concepts

| Concept | Description |
|---------|-------------|
| **Virtqueue** | Ring buffer for communication (RX=queue 0, TX=queue 1) |
| **Descriptor** | Points to buffer, includes flags |
| **Available Ring** | Driver → Device: "these buffers are ready" |
| **Used Ring** | Device → Driver: "I processed these buffers" |
| **Features** | Capability negotiation (both sides advertise, agree on subset) |
| **Status** | Device lifecycle state machine |

## 4.2 Device Discovery

VirtIO devices are identified by PCI vendor/device IDs:

```rust
/// VirtIO PCI vendor ID
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

/// VirtIO-net PCI device IDs
pub const VIRTIO_NET_DEVICE_IDS: &[u16] = &[
    0x1000,  // Legacy virtio-net (transitional)
    0x1041,  // Modern virtio-net (virtio 1.0+)
];

/// Check if PCI device is VirtIO-net.
pub fn is_virtio_net(vendor: u16, device: u16) -> bool {
    vendor == VIRTIO_VENDOR_ID && VIRTIO_NET_DEVICE_IDS.contains(&device)
}
```

## 4.3 Device Status State Machine

```
┌─────────────────────────────────────────────────────────────────┐
│                  VirtIO Device Status Bits                      │
├─────────────────────────────────────────────────────────────────┤
│  Bit 0: ACKNOWLEDGE (0x01) - Driver found device                │
│  Bit 1: DRIVER      (0x02) - Driver knows how to drive device   │
│  Bit 2: DRIVER_OK   (0x04) - Driver ready, device may operate   │
│  Bit 3: FEATURES_OK (0x08) - Feature negotiation complete       │
│  Bit 6: DEVICE_NEEDS_RESET (0x40) - Device error, needs reset   │
│  Bit 7: FAILED      (0x80) - Driver gave up on device           │
└─────────────────────────────────────────────────────────────────┘

Initialization sequence (per VirtIO spec §3.1):

  ┌──────────┐
  │  Reset   │  Write 0 to status, wait for status=0
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │   0x01   │  Set ACKNOWLEDGE
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │   0x03   │  Set DRIVER (0x01 | 0x02)
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │ Features │  Read device features, select subset
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │   0x0B   │  Set FEATURES_OK (0x01 | 0x02 | 0x08)
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │  Verify  │  Re-read status, check FEATURES_OK still set
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │  Queues  │  Configure virtqueues (RX, TX)
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │   0x0F   │  Set DRIVER_OK (device operational)
  └──────────┘
```

## 4.4 Feature Negotiation

```rust
/// VirtIO feature bits
pub mod features {
    /// VirtIO 1.0+ (modern device)
    pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;
    
    /// Device has MAC address in config space
    pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;
    
    /// Device has link status in config space
    pub const VIRTIO_NET_F_STATUS: u64 = 1 << 16;
    
    // ═══════════════════════════════════════════════════════════
    // FORBIDDEN FEATURES - DO NOT NEGOTIATE
    // ═══════════════════════════════════════════════════════════
    
    /// Guest TSO4 - complicates buffer management
    pub const VIRTIO_NET_F_GUEST_TSO4: u64 = 1 << 7;
    
    /// Guest TSO6 - complicates buffer management  
    pub const VIRTIO_NET_F_GUEST_TSO6: u64 = 1 << 8;
    
    /// Guest UFO - complicates buffer management
    pub const VIRTIO_NET_F_GUEST_UFO: u64 = 1 << 10;
    
    /// Mergeable RX buffers - changes header size (to 12 bytes)
    /// Actually we use 12-byte header always (modern), so this is OK to not use
    pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;
    
    /// Control virtqueue - not needed for basic operation
    pub const VIRTIO_NET_F_CTRL_VQ: u64 = 1 << 17;
}

/// Required features (device must support, else reject)
pub const REQUIRED_FEATURES: u64 = features::VIRTIO_F_VERSION_1;

/// Desired features (use if available)
pub const DESIRED_FEATURES: u64 = 
    features::VIRTIO_NET_F_MAC |
    features::VIRTIO_NET_F_STATUS;

/// Forbidden features (never negotiate)
pub const FORBIDDEN_FEATURES: u64 =
    features::VIRTIO_NET_F_GUEST_TSO4 |
    features::VIRTIO_NET_F_GUEST_TSO6 |
    features::VIRTIO_NET_F_GUEST_UFO |
    features::VIRTIO_NET_F_CTRL_VQ;

/// Negotiate features with device.
pub fn negotiate_features(device_features: u64) -> Result<u64, FeatureError> {
    // Check required
    if device_features & REQUIRED_FEATURES != REQUIRED_FEATURES {
        return Err(FeatureError::MissingRequired(REQUIRED_FEATURES));
    }
    
    // Select: required + (desired ∩ device) - forbidden
    let our_features = REQUIRED_FEATURES 
        | (DESIRED_FEATURES & device_features)
        & !FORBIDDEN_FEATURES;
    
    Ok(our_features)
}
```

## 4.5 Initialization Sequence

```rust
/// Initialize VirtIO network device.
/// 
/// # Arguments
/// - `mmio_base`: MMIO base address from PCI BAR
/// - `dma`: Pre-allocated DMA region
/// 
/// # Returns
/// Initialized driver or error.
/// 
/// # Reference
/// REDESIGN §4.3, AUDIT §7.2.3-7.2.8
pub fn virtio_net_init(
    mmio_base: u64,
    dma: &mut DmaRegion,
) -> Result<VirtioNetDriver, InitError> {
    
    // ═══════════════════════════════════════════════════════════
    // STEP 1: RESET DEVICE
    // Reference: VirtIO spec §3.1 step 1, AUDIT §7.2.3
    // ═══════════════════════════════════════════════════════════
    unsafe { asm_nic_set_status(mmio_base, 0) };
    
    // Wait for reset (bounded, per AUDIT correction)
    let start = unsafe { asm_tsc_read() };
    let timeout = tsc_freq / 10;  // 100ms
    loop {
        let status = unsafe { asm_nic_get_status(mmio_base) };
        if status == 0 {
            break;
        }
        if unsafe { asm_tsc_read() }.wrapping_sub(start) > timeout {
            return Err(InitError::ResetTimeout);
        }
        // Small spin to avoid hammering MMIO
        for _ in 0..1000 { core::hint::spin_loop(); }
    }
    
    // Conservative delay for device stability
    for _ in 0..100_000 { core::hint::spin_loop(); }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 2: SET ACKNOWLEDGE
    // Reference: VirtIO spec §3.1 step 2
    // ═══════════════════════════════════════════════════════════
    unsafe { asm_nic_set_status(mmio_base, STATUS_ACKNOWLEDGE) };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 3: SET DRIVER
    // Reference: VirtIO spec §3.1 step 3
    // ═══════════════════════════════════════════════════════════
    unsafe { 
        asm_nic_set_status(mmio_base, STATUS_ACKNOWLEDGE | STATUS_DRIVER) 
    };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 4: FEATURE NEGOTIATION
    // Reference: VirtIO spec §3.1 step 4, AUDIT §7.2.4
    // ═══════════════════════════════════════════════════════════
    let device_features = unsafe { asm_nic_read_features(mmio_base) };
    let our_features = negotiate_features(device_features)?;
    unsafe { asm_nic_write_features(mmio_base, our_features) };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 5: SET FEATURES_OK
    // Reference: VirtIO spec §3.1 step 5
    // ═══════════════════════════════════════════════════════════
    unsafe {
        asm_nic_set_status(mmio_base, 
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK)
    };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 6: VERIFY FEATURES ACCEPTED
    // Reference: VirtIO spec §3.1 step 6, AUDIT §7.2.4
    // ═══════════════════════════════════════════════════════════
    let status = unsafe { asm_nic_get_status(mmio_base) };
    if status & STATUS_FEATURES_OK == 0 {
        unsafe { asm_nic_set_status(mmio_base, STATUS_FAILED) };
        return Err(InitError::FeatureNegotiationFailed);
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 7: CONFIGURE VIRTQUEUES
    // Reference: REDESIGN §4.3, AUDIT §7.2.5
    // ═══════════════════════════════════════════════════════════
    
    // RX Queue (index 0)
    let rx_queue = setup_virtqueue(mmio_base, 0, dma, QUEUE_SIZE)?;
    
    // TX Queue (index 1)  
    let tx_queue = setup_virtqueue(mmio_base, 1, dma, QUEUE_SIZE)?;
    
    // ═══════════════════════════════════════════════════════════
    // STEP 8: PRE-FILL RX QUEUE
    // Reference: REDESIGN §4.3 "RX queue pre-fill"
    // ═══════════════════════════════════════════════════════════
    for i in 0..QUEUE_SIZE {
        let result = unsafe {
            asm_vq_submit_rx(&mut rx_queue.state, i as u16, BUFFER_SIZE as u16)
        };
        if result != 0 {
            return Err(InitError::RxPrefillFailed(i));
        }
    }
    
    // Notify device RX buffers are available
    unsafe { asm_vq_notify(&mut rx_queue.state) };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 9: SET DRIVER_OK
    // Reference: VirtIO spec §3.1 step 8
    // ═══════════════════════════════════════════════════════════
    unsafe {
        asm_nic_set_status(mmio_base,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK)
    };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 10: READ MAC ADDRESS
    // Reference: AUDIT §7.2.8
    // ═══════════════════════════════════════════════════════════
    let mac = if our_features & features::VIRTIO_NET_F_MAC != 0 {
        let mut mac = [0u8; 6];
        unsafe { asm_nic_read_mac(mmio_base, &mut mac) };
        mac
    } else {
        generate_local_mac()
    };
    
    Ok(VirtioNetDriver {
        mmio_base,
        mac,
        features: our_features,
        rx_queue,
        tx_queue,
        rx_pool: BufferPool::new(dma.rx_buffers_cpu(), dma.rx_buffers_bus(), BUFFER_SIZE, QUEUE_SIZE),
        tx_pool: BufferPool::new(dma.tx_buffers_cpu(), dma.tx_buffers_bus(), BUFFER_SIZE, QUEUE_SIZE),
    })
}
```

## 4.6 Transmit Path (Fire-and-Forget)

```rust
impl VirtioNetDriver {
    /// Transmit a packet. Returns immediately (no completion wait).
    /// 
    /// # Arguments
    /// - `packet`: Ethernet frame (without VirtIO header)
    /// 
    /// # Returns
    /// - `Ok(())`: Packet queued for transmission
    /// - `Err(TxError::QueueFull)`: No space, try again after collecting completions
    /// 
    /// # Reference
    /// AUDIT §5.5.2 (fire-and-forget), REDESIGN §7.3
    pub fn transmit(&mut self, packet: &[u8]) -> Result<(), TxError> {
        // Collect any pending completions first (reclaim buffers)
        self.collect_tx_completions();
        
        // Allocate TX buffer
        let buf = self.tx_pool.alloc().ok_or(TxError::QueueFull)?;
        
        // Write VirtIO header (12 bytes, all zeros)
        let hdr = VirtioNetHdr::zeroed();
        buf.as_mut_slice()[..VirtioNetHdr::SIZE].copy_from_slice(
            unsafe { core::slice::from_raw_parts(&hdr as *const _ as *const u8, VirtioNetHdr::SIZE) }
        );
        
        // Copy packet after header
        let total_len = VirtioNetHdr::SIZE + packet.len();
        buf.as_mut_slice()[VirtioNetHdr::SIZE..total_len].copy_from_slice(packet);
        
        // Mark device-owned BEFORE submit
        unsafe { buf.mark_device_owned(); }
        
        // Submit via ASM (includes barriers)
        let result = unsafe {
            asm_vq_submit_tx(&mut self.tx_queue.state, buf.index(), total_len as u16)
        };
        
        if result != 0 {
            // Queue was full (shouldn't happen after collect, but handle it)
            unsafe { buf.mark_driver_owned(); }
            self.tx_pool.free(buf);
            return Err(TxError::QueueFull);
        }
        
        // *** DO NOT WAIT FOR COMPLETION ***
        // Completion collected in main loop Phase 5
        
        Ok(())
    }
    
    /// Collect TX completions. Call in main loop Phase 5.
    pub fn collect_tx_completions(&mut self) {
        loop {
            let idx = unsafe { asm_vq_poll_tx_complete(&mut self.tx_queue.state) };
            if idx == 0xFFFFFFFF {
                break;  // No more completions
            }
            
            // Return buffer to pool
            let buf = self.tx_pool.get_mut(idx as u16);
            unsafe { buf.mark_driver_owned(); }
            self.tx_pool.free(buf);
        }
    }
}
```

## 4.7 Receive Path (Poll-Based)

```rust
impl VirtioNetDriver {
    /// Poll for received packet. Returns immediately.
    /// 
    /// # Arguments
    /// - `out_buffer`: Buffer to copy received frame into
    /// 
    /// # Returns
    /// - `Ok(Some(len))`: Packet received, `len` bytes copied (without VirtIO header)
    /// - `Ok(None)`: No packet available (normal, not an error)
    /// - `Err(RxError)`: Receive error
    /// 
    /// # Reference
    /// REDESIGN §7.2
    pub fn receive(&mut self, out_buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        let mut result = RxResult { buffer_idx: 0, length: 0, _reserved: 0 };
        
        // Poll via ASM (includes barriers)
        let has_packet = unsafe {
            asm_vq_poll_rx(&mut self.rx_queue.state, &mut result)
        };
        
        if has_packet == 0 {
            return Ok(None);  // No packet available
        }
        
        // Get buffer (now driver-owned)
        let buf = self.rx_pool.get_mut(result.buffer_idx);
        unsafe { buf.mark_driver_owned(); }
        
        // Copy frame (skip 12-byte VirtIO header)
        let frame_len = result.length as usize - VirtioNetHdr::SIZE;
        if frame_len > out_buffer.len() {
            // Frame too large for caller's buffer
            // Still resubmit our buffer, but return error
            self.resubmit_rx_buffer(buf);
            return Err(RxError::BufferTooSmall { needed: frame_len });
        }
        
        out_buffer[..frame_len].copy_from_slice(
            &buf.as_slice()[VirtioNetHdr::SIZE..VirtioNetHdr::SIZE + frame_len]
        );
        
        // Resubmit buffer to RX queue
        self.resubmit_rx_buffer(buf);
        
        Ok(Some(frame_len))
    }
    
    /// Resubmit RX buffer after processing.
    fn resubmit_rx_buffer(&mut self, buf: &mut DmaBuffer) {
        unsafe { buf.mark_device_owned(); }
        
        let result = unsafe {
            asm_vq_submit_rx(&mut self.rx_queue.state, buf.index(), BUFFER_SIZE as u16)
        };
        
        if result != 0 {
            // Queue full - this shouldn't happen with proper sizing
            // Log error but continue (we'll lose this buffer temporarily)
            unsafe { buf.mark_driver_owned(); }
        }
    }
    
    /// Refill RX queue. Call in main loop Phase 1.
    pub fn refill_rx_queue(&mut self) {
        // This is handled by resubmit_rx_buffer after each receive
        // But we can also top up here if needed
        
        // Optionally notify device if we submitted buffers
        // (submit functions handle notification internally)
    }
}
```

## 4.8 Invariants

| ID | Invariant | Reference |
|----|-----------|-----------|
| **VIO-1** | Reset waits bounded at 100ms | AUDIT §7.2.3 |
| **VIO-2** | FEATURES_OK verified before queue setup | VirtIO spec §3.1 |
| **VIO-3** | RX queue pre-filled before DRIVER_OK | REDESIGN §4.3 |
| **VIO-4** | TX submit returns immediately | AUDIT §5.5.2 |
| **VIO-5** | 12-byte VirtIO header (modern) | AUDIT §7.2.7 |
| **VIO-6** | Queue size read from device, not hardcoded | AUDIT §7.2.5 |
| **VIO-7** | bus_addr in descriptors, cpu_ptr for access | §3.3 |
| **VIO-8** | Forbidden features never negotiated | §4.4 |

---

# 5. State Machines

## 5.1 Design Principles

State machines replace all blocking patterns. Each state machine:

1. **Has a `step()` method** that advances by one logical step
2. **Returns immediately** without waiting for external events
3. **Checks timeouts** as observations, not waits
4. **Transitions on conditions** being met at call time

**Reference**: REDESIGN §8, REFACTOR_BLOCKING_PATTERNS.md

## 5.2 State Machine Contract

```rust
/// Result of a state machine step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Still in progress, call step() again next iteration
    Pending,
    /// Operation completed successfully
    Done,
    /// Operation timed out
    Timeout,
    /// Operation failed with error
    Failed,
}

/// Trait for all state machines.
pub trait StateMachine {
    /// Output type when operation completes successfully.
    type Output;
    
    /// Error type for failures.
    type Error;
    
    /// Advance state machine by one step.
    /// 
    /// # Contract
    /// - MUST return immediately (no blocking)
    /// - MUST NOT loop waiting for conditions
    /// - SHOULD check timeout first
    /// - SHOULD transition state if condition met
    /// 
    /// # Arguments
    /// - `iface`: Network interface for socket operations
    /// - `sockets`: Socket set
    /// - `now_tsc`: Current TSC value
    /// - `timeouts`: Timeout configuration
    /// 
    /// # Returns
    /// - `Pending`: Not complete, call again
    /// - `Done`: Check `output()` for result
    /// - `Timeout`/`Failed`: Check `error()` for details
    fn step(
        &mut self,
        iface: &mut Interface,
        sockets: &mut SocketSet,
        now_tsc: u64,
        timeouts: &TimeoutConfig,
    ) -> StepResult;
    
    /// Get output after `Done`. Panics if not done.
    fn output(&self) -> &Self::Output;
    
    /// Get error after `Timeout`/`Failed`. Panics if not failed.
    fn error(&self) -> &Self::Error;
}
```

## 5.3 DHCP State Machine

```rust
// src/state/dhcp.rs

/// DHCP client state machine.
/// 
/// Waits for smoltcp's DHCP client to obtain an IP address.
/// Does NOT implement DHCP itself—relies on smoltcp.
#[derive(Debug)]
pub enum DhcpState {
    /// Initial state, DHCP not started
    Init,
    
    /// Waiting for DHCP to complete
    Discovering {
        /// When we started waiting
        start_tsc: u64,
    },
    
    /// IP address obtained
    Bound {
        /// Assigned IP address
        ip: Ipv4Addr,
        /// Subnet mask
        subnet: Ipv4Addr,
        /// Default gateway
        gateway: Option<Ipv4Addr>,
        /// DNS server
        dns: Option<Ipv4Addr>,
    },
    
    /// DHCP failed (timeout or error)
    Failed {
        error: DhcpError,
    },
}

#[derive(Debug, Clone)]
pub enum DhcpError {
    Timeout,
    NoInterface,
}

impl DhcpState {
    /// Create new DHCP state machine.
    pub fn new() -> Self {
        DhcpState::Init
    }
    
    /// Start DHCP process.
    pub fn start(&mut self, now_tsc: u64) {
        *self = DhcpState::Discovering { start_tsc: now_tsc };
    }
    
    /// Advance state machine by one step.
    /// 
    /// # Contract
    /// - Returns immediately
    /// - Checks smoltcp interface for IP assignment
    /// - Transitions to Bound when IP obtained
    /// - Transitions to Failed on timeout
    pub fn step(
        &mut self,
        iface: &mut Interface,
        now_tsc: u64,
        timeouts: &TimeoutConfig,
    ) -> StepResult {
        match self {
            DhcpState::Init => {
                // Not started yet
                StepResult::Pending
            }
            
            DhcpState::Discovering { start_tsc } => {
                // Check timeout FIRST
                let elapsed = now_tsc.wrapping_sub(*start_tsc);
                if elapsed > timeouts.dhcp_timeout() {
                    *self = DhcpState::Failed { error: DhcpError::Timeout };
                    return StepResult::Timeout;
                }
                
                // Check if smoltcp has assigned an IP
                if let Some(ip_cidr) = iface.ipv4_addr() {
                    let ip = ip_cidr;
                    
                    // Get gateway and DNS from smoltcp config
                    let gateway = iface.routes().default_gateway();
                    
                    *self = DhcpState::Bound {
                        ip,
                        subnet: Ipv4Addr::new(255, 255, 255, 0), // Simplified
                        gateway,
                        dns: None, // Get from DHCP options if available
                    };
                    return StepResult::Done;
                }
                
                // Still waiting
                StepResult::Pending
            }
            
            DhcpState::Bound { .. } => StepResult::Done,
            DhcpState::Failed { .. } => StepResult::Failed,
        }
    }
    
    /// Check if DHCP completed successfully.
    pub fn is_bound(&self) -> bool {
        matches!(self, DhcpState::Bound { .. })
    }
    
    /// Get bound IP address.
    pub fn ip(&self) -> Option<Ipv4Addr> {
        match self {
            DhcpState::Bound { ip, .. } => Some(*ip),
            _ => None,
        }
    }
}
```

## 5.4 TCP Connection State Machine

```rust
// src/state/tcp.rs

/// TCP connection state machine.
/// 
/// Manages non-blocking TCP connection establishment.
#[derive(Debug)]
pub enum TcpConnState {
    /// Not connected
    Closed,
    
    /// Connection initiated, waiting for establishment
    Connecting {
        /// Socket handle
        socket: SocketHandle,
        /// Remote address
        remote: (Ipv4Addr, u16),
        /// When connect started
        start_tsc: u64,
    },
    
    /// Connection established
    Established {
        socket: SocketHandle,
    },
    
    /// Connection closing
    Closing {
        socket: SocketHandle,
        start_tsc: u64,
    },
    
    /// Error state
    Error {
        error: TcpError,
    },
}

#[derive(Debug, Clone)]
pub enum TcpError {
    ConnectTimeout,
    ConnectionRefused,
    ConnectionReset,
    CloseTimeout,
    SocketError(smoltcp::socket::tcp::ConnectError),
}

impl TcpConnState {
    /// Create new TCP state machine (not connected).
    pub fn new() -> Self {
        TcpConnState::Closed
    }
    
    /// Initiate connection.
    /// 
    /// # Arguments
    /// - `sockets`: Socket set to allocate from
    /// - `remote`: Remote address (IP, port)
    /// - `local_port`: Local port (0 for ephemeral)
    /// - `now_tsc`: Current TSC for timeout tracking
    pub fn connect(
        &mut self,
        sockets: &mut SocketSet,
        iface: &mut Interface,
        remote: (Ipv4Addr, u16),
        local_port: u16,
        now_tsc: u64,
    ) -> Result<(), TcpError> {
        // Allocate socket
        let rx_buffer = TcpSocketBuffer::new(vec![0; 65535]);
        let tx_buffer = TcpSocketBuffer::new(vec![0; 65535]);
        let socket = TcpSocket::new(rx_buffer, tx_buffer);
        let handle = sockets.add(socket);
        
        // Initiate connection
        let socket = sockets.get_mut::<TcpSocket>(handle);
        let remote_endpoint = (IpAddress::Ipv4(remote.0), remote.1);
        let local_endpoint = local_port;
        
        socket.connect(iface.context(), remote_endpoint, local_endpoint)
            .map_err(TcpError::SocketError)?;
        
        *self = TcpConnState::Connecting {
            socket: handle,
            remote,
            start_tsc: now_tsc,
        };
        
        Ok(())
    }
    
    /// Advance state machine by one step.
    pub fn step(
        &mut self,
        sockets: &mut SocketSet,
        now_tsc: u64,
        timeouts: &TimeoutConfig,
    ) -> StepResult {
        match self {
            TcpConnState::Closed => StepResult::Pending,
            
            TcpConnState::Connecting { socket, start_tsc, .. } => {
                // Check timeout
                let elapsed = now_tsc.wrapping_sub(*start_tsc);
                if elapsed > timeouts.tcp_connect() {
                    let handle = *socket;
                    *self = TcpConnState::Error { error: TcpError::ConnectTimeout };
                    // Close the socket
                    sockets.get_mut::<TcpSocket>(handle).abort();
                    return StepResult::Timeout;
                }
                
                // Check socket state
                let tcp = sockets.get_mut::<TcpSocket>(*socket);
                
                if tcp.is_active() && tcp.may_send() && tcp.may_recv() {
                    // Connected!
                    let handle = *socket;
                    *self = TcpConnState::Established { socket: handle };
                    return StepResult::Done;
                }
                
                if tcp.state() == TcpState::Closed {
                    // Connection refused or reset
                    *self = TcpConnState::Error { error: TcpError::ConnectionRefused };
                    return StepResult::Failed;
                }
                
                StepResult::Pending
            }
            
            TcpConnState::Established { .. } => StepResult::Done,
            
            TcpConnState::Closing { socket, start_tsc } => {
                let elapsed = now_tsc.wrapping_sub(*start_tsc);
                if elapsed > timeouts.tcp_close() {
                    *self = TcpConnState::Error { error: TcpError::CloseTimeout };
                    return StepResult::Timeout;
                }
                
                let tcp = sockets.get_mut::<TcpSocket>(*socket);
                if tcp.state() == TcpState::Closed {
                    *self = TcpConnState::Closed;
                    return StepResult::Done;
                }
                
                StepResult::Pending
            }
            
            TcpConnState::Error { .. } => StepResult::Failed,
        }
    }
    
    /// Close connection gracefully.
    pub fn close(&mut self, sockets: &mut SocketSet, now_tsc: u64) {
        if let TcpConnState::Established { socket } = self {
            let handle = *socket;
            sockets.get_mut::<TcpSocket>(handle).close();
            *self = TcpConnState::Closing { socket: handle, start_tsc: now_tsc };
        }
    }
    
    /// Get socket handle if connected.
    pub fn socket(&self) -> Option<SocketHandle> {
        match self {
            TcpConnState::Established { socket } => Some(*socket),
            _ => None,
        }
    }
}
```

## 5.5 HTTP Download State Machine

```rust
// src/state/http.rs

/// HTTP download state machine.
/// 
/// Performs HTTP GET request with non-blocking state transitions.
#[derive(Debug)]
pub enum HttpDownloadState {
    /// Initial state
    Init {
        url: Url,
    },
    
    /// Resolving DNS (if hostname, not IP)
    Resolving {
        host: String,
        port: u16,
        path: String,
        start_tsc: u64,
    },
    
    /// Connecting to server
    Connecting {
        ip: Ipv4Addr,
        port: u16,
        path: String,
        tcp: TcpConnState,
    },
    
    /// Sending HTTP request
    SendingRequest {
        socket: SocketHandle,
        request: Vec<u8>,
        sent: usize,
        start_tsc: u64,
    },
    
    /// Receiving HTTP headers
    ReceivingHeaders {
        socket: SocketHandle,
        buffer: Vec<u8>,
        start_tsc: u64,
    },
    
    /// Receiving HTTP body
    ReceivingBody {
        socket: SocketHandle,
        headers: HttpHeaders,
        received: usize,
        content_length: Option<usize>,
        callback: Option<Box<dyn FnMut(&[u8])>>,
        start_tsc: u64,
    },
    
    /// Download complete
    Done {
        total_bytes: usize,
    },
    
    /// Download failed
    Failed {
        error: HttpError,
    },
}

#[derive(Debug, Clone)]
pub enum HttpError {
    DnsTimeout,
    DnsError(String),
    ConnectTimeout,
    ConnectError(TcpError),
    SendTimeout,
    ReceiveTimeout,
    InvalidResponse,
    HttpStatus(u16, String),
    ConnectionClosed,
}

impl HttpDownloadState {
    /// Create new HTTP download for URL.
    pub fn new(url: Url) -> Self {
        HttpDownloadState::Init { url }
    }
    
    /// Start the download.
    pub fn start(&mut self, now_tsc: u64) {
        if let HttpDownloadState::Init { url } = self {
            // Parse URL and transition to appropriate state
            let host = url.host().to_string();
            let port = url.port().unwrap_or(80);
            let path = url.path().to_string();
            
            // Check if host is already an IP
            if let Ok(ip) = host.parse::<Ipv4Addr>() {
                *self = HttpDownloadState::Connecting {
                    ip,
                    port,
                    path,
                    tcp: TcpConnState::new(),
                };
            } else {
                *self = HttpDownloadState::Resolving {
                    host,
                    port,
                    path,
                    start_tsc: now_tsc,
                };
            }
        }
    }
    
    /// Advance state machine by one step.
    pub fn step(
        &mut self,
        iface: &mut Interface,
        sockets: &mut SocketSet,
        now_tsc: u64,
        timeouts: &TimeoutConfig,
    ) -> StepResult {
        // Take ownership temporarily for state transitions
        let current = core::mem::replace(self, HttpDownloadState::Init { 
            url: Url::parse("http://temp").unwrap() 
        });
        
        let (new_state, result) = match current {
            HttpDownloadState::Init { url } => {
                (HttpDownloadState::Init { url }, StepResult::Pending)
            }
            
            HttpDownloadState::Resolving { host, port, path, start_tsc } => {
                // Check timeout
                let elapsed = now_tsc.wrapping_sub(start_tsc);
                if elapsed > timeouts.dns_timeout() {
                    (HttpDownloadState::Failed { error: HttpError::DnsTimeout }, StepResult::Timeout)
                } else {
                    // Check DNS result from smoltcp
                    // For simplicity, assume we use smoltcp's DNS or hardcoded
                    // In real impl, check DNS socket for response
                    
                    // Placeholder: transition when DNS resolves
                    (HttpDownloadState::Resolving { host, port, path, start_tsc }, StepResult::Pending)
                }
            }
            
            HttpDownloadState::Connecting { ip, port, path, mut tcp } => {
                // Initialize TCP connection if not started
                if matches!(tcp, TcpConnState::Closed) {
                    let _ = tcp.connect(sockets, iface, (ip, port), 0, now_tsc);
                }
                
                // Step TCP state machine
                match tcp.step(sockets, now_tsc, timeouts) {
                    StepResult::Done => {
                        // Connected! Build HTTP request
                        let socket = tcp.socket().unwrap();
                        let request = format!(
                            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                            path, ip
                        ).into_bytes();
                        
                        (HttpDownloadState::SendingRequest {
                            socket,
                            request,
                            sent: 0,
                            start_tsc: now_tsc,
                        }, StepResult::Pending)
                    }
                    StepResult::Timeout | StepResult::Failed => {
                        (HttpDownloadState::Failed { 
                            error: HttpError::ConnectTimeout 
                        }, StepResult::Failed)
                    }
                    StepResult::Pending => {
                        (HttpDownloadState::Connecting { ip, port, path, tcp }, StepResult::Pending)
                    }
                }
            }
            
            HttpDownloadState::SendingRequest { socket, request, sent, start_tsc } => {
                // Check timeout
                let elapsed = now_tsc.wrapping_sub(start_tsc);
                if elapsed > timeouts.http_send() {
                    (HttpDownloadState::Failed { error: HttpError::SendTimeout }, StepResult::Timeout)
                } else {
                    // Try to send more data
                    let tcp = sockets.get_mut::<TcpSocket>(socket);
                    
                    if tcp.can_send() {
                        let remaining = &request[sent..];
                        match tcp.send_slice(remaining) {
                            Ok(n) => {
                                let new_sent = sent + n;
                                if new_sent >= request.len() {
                                    // Request fully sent
                                    (HttpDownloadState::ReceivingHeaders {
                                        socket,
                                        buffer: Vec::new(),
                                        start_tsc: now_tsc,
                                    }, StepResult::Pending)
                                } else {
                                    (HttpDownloadState::SendingRequest {
                                        socket, request, sent: new_sent, start_tsc
                                    }, StepResult::Pending)
                                }
                            }
                            Err(_) => {
                                (HttpDownloadState::Failed { 
                                    error: HttpError::ConnectionClosed 
                                }, StepResult::Failed)
                            }
                        }
                    } else {
                        (HttpDownloadState::SendingRequest { socket, request, sent, start_tsc }, StepResult::Pending)
                    }
                }
            }
            
            HttpDownloadState::ReceivingHeaders { socket, mut buffer, start_tsc } => {
                let elapsed = now_tsc.wrapping_sub(start_tsc);
                if elapsed > timeouts.http_receive() {
                    (HttpDownloadState::Failed { error: HttpError::ReceiveTimeout }, StepResult::Timeout)
                } else {
                    let tcp = sockets.get_mut::<TcpSocket>(socket);
                    
                    if tcp.can_recv() {
                        let mut temp = [0u8; 1024];
                        match tcp.recv_slice(&mut temp) {
                            Ok(n) if n > 0 => {
                                buffer.extend_from_slice(&temp[..n]);
                                
                                // Check for end of headers
                                if let Some(pos) = find_header_end(&buffer) {
                                    // Parse headers
                                    let header_bytes = &buffer[..pos];
                                    match parse_http_headers(header_bytes) {
                                        Ok(headers) => {
                                            let body_start = &buffer[pos + 4..]; // Skip \r\n\r\n
                                            let content_length = headers.content_length;
                                            
                                            (HttpDownloadState::ReceivingBody {
                                                socket,
                                                headers,
                                                received: body_start.len(),
                                                content_length,
                                                callback: None,
                                                start_tsc: now_tsc,
                                            }, StepResult::Pending)
                                        }
                                        Err(_) => {
                                            (HttpDownloadState::Failed { 
                                                error: HttpError::InvalidResponse 
                                            }, StepResult::Failed)
                                        }
                                    }
                                } else {
                                    (HttpDownloadState::ReceivingHeaders { socket, buffer, start_tsc }, StepResult::Pending)
                                }
                            }
                            _ => (HttpDownloadState::ReceivingHeaders { socket, buffer, start_tsc }, StepResult::Pending)
                        }
                    } else {
                        (HttpDownloadState::ReceivingHeaders { socket, buffer, start_tsc }, StepResult::Pending)
                    }
                }
            }
            
            HttpDownloadState::ReceivingBody { socket, headers, received, content_length, callback, start_tsc } => {
                let elapsed = now_tsc.wrapping_sub(start_tsc);
                if elapsed > timeouts.http_receive() {
                    (HttpDownloadState::Failed { error: HttpError::ReceiveTimeout }, StepResult::Timeout)
                } else {
                    let tcp = sockets.get_mut::<TcpSocket>(socket);
                    
                    // Check if complete
                    let is_complete = match content_length {
                        Some(len) => received >= len,
                        None => !tcp.is_active(), // Connection closed
                    };
                    
                    if is_complete {
                        (HttpDownloadState::Done { total_bytes: received }, StepResult::Done)
                    } else if tcp.can_recv() {
                        let mut temp = [0u8; 8192];
                        match tcp.recv_slice(&mut temp) {
                            Ok(n) if n > 0 => {
                                // Call progress callback if set
                                // callback.as_mut().map(|cb| cb(&temp[..n]));
                                
                                (HttpDownloadState::ReceivingBody {
                                    socket,
                                    headers,
                                    received: received + n,
                                    content_length,
                                    callback,
                                    start_tsc,
                                }, StepResult::Pending)
                            }
                            _ => (HttpDownloadState::ReceivingBody {
                                socket, headers, received, content_length, callback, start_tsc
                            }, StepResult::Pending)
                        }
                    } else {
                        (HttpDownloadState::ReceivingBody {
                            socket, headers, received, content_length, callback, start_tsc
                        }, StepResult::Pending)
                    }
                }
            }
            
            HttpDownloadState::Done { total_bytes } => {
                (HttpDownloadState::Done { total_bytes }, StepResult::Done)
            }
            
            HttpDownloadState::Failed { error } => {
                (HttpDownloadState::Failed { error }, StepResult::Failed)
            }
        };
        
        *self = new_state;
        result
    }
}
```

## 5.6 State Machine Composition

```rust
// src/state/mod.rs

/// Top-level download orchestration state machine.
/// 
/// Composes lower-level state machines: DHCP → HTTP → Verify
pub enum IsoDownloadState {
    /// Initial state
    Init,
    
    /// Waiting for network (DHCP)
    WaitingForNetwork {
        dhcp: DhcpState,
    },
    
    /// Downloading ISO
    Downloading {
        http: HttpDownloadState,
        progress: DownloadProgress,
    },
    
    /// Verifying checksum (if provided)
    Verifying {
        data_ptr: *const u8,
        data_len: usize,
        expected_hash: [u8; 32],
    },
    
    /// Complete
    Done {
        iso_ptr: *const u8,
        iso_len: usize,
    },
    
    /// Failed
    Failed {
        error: DownloadError,
    },
}

impl IsoDownloadState {
    pub fn step(
        &mut self,
        iface: &mut Interface,
        sockets: &mut SocketSet,
        now_tsc: u64,
        timeouts: &TimeoutConfig,
    ) -> StepResult {
        match self {
            Self::WaitingForNetwork { dhcp } => {
                // Step the nested DHCP state machine
                match dhcp.step(iface, now_tsc, timeouts) {
                    StepResult::Done => {
                        // Network ready, start download
                        let http = HttpDownloadState::new(/* url */);
                        *self = Self::Downloading { 
                            http, 
                            progress: DownloadProgress::default() 
                        };
                        StepResult::Pending
                    }
                    StepResult::Timeout | StepResult::Failed => {
                        *self = Self::Failed { 
                            error: DownloadError::NetworkTimeout 
                        };
                        StepResult::Failed
                    }
                    StepResult::Pending => StepResult::Pending,
                }
            }
            
            Self::Downloading { http, progress } => {
                match http.step(iface, sockets, now_tsc, timeouts) {
                    StepResult::Done => {
                        *self = Self::Done { 
                            iso_ptr: core::ptr::null(), 
                            iso_len: 0 
                        };
                        StepResult::Done
                    }
                    StepResult::Timeout | StepResult::Failed => {
                        *self = Self::Failed { 
                            error: DownloadError::HttpError 
                        };
                        StepResult::Failed
                    }
                    StepResult::Pending => StepResult::Pending,
                }
            }
            
            // ... other states
            _ => StepResult::Pending,
        }
    }
}
```

## 5.7 Timeout Configuration

```rust
// src/time/timeout.rs

/// Timeout configuration based on calibrated TSC frequency.
/// 
/// All timeouts are calculated from TSC frequency obtained at boot.
/// NO HARDCODED VALUES.
pub struct TimeoutConfig {
    /// TSC ticks per second (calibrated at boot)
    tsc_freq: u64,
}

impl TimeoutConfig {
    /// Create from calibrated TSC frequency.
    /// 
    /// # Panics
    /// Panics if `tsc_freq` is zero.
    pub fn new(tsc_freq: u64) -> Self {
        assert!(tsc_freq > 0, "TSC frequency must be calibrated at boot");
        Self { tsc_freq }
    }
    
    /// Convert milliseconds to TSC ticks.
    #[inline]
    pub fn ms_to_ticks(&self, ms: u64) -> u64 {
        ms * self.tsc_freq / 1_000
    }
    
    /// Convert seconds to TSC ticks.
    #[inline]  
    pub fn secs_to_ticks(&self, secs: u64) -> u64 {
        secs * self.tsc_freq
    }
    
    // ═══════════════════════════════════════════════════════════
    // DEFINED TIMEOUTS
    // Reference: REDESIGN §7.4
    // ═══════════════════════════════════════════════════════════
    
    /// DHCP timeout (30 seconds)
    pub fn dhcp_timeout(&self) -> u64 {
        self.secs_to_ticks(30)
    }
    
    /// TCP connect timeout (30 seconds)
    pub fn tcp_connect(&self) -> u64 {
        self.secs_to_ticks(30)
    }
    
    /// TCP close timeout (10 seconds)
    pub fn tcp_close(&self) -> u64 {
        self.secs_to_ticks(10)
    }
    
    /// DNS query timeout (5 seconds)
    pub fn dns_timeout(&self) -> u64 {
        self.secs_to_ticks(5)
    }
    
    /// HTTP send timeout (30 seconds)
    pub fn http_send(&self) -> u64 {
        self.secs_to_ticks(30)
    }
    
    /// HTTP receive timeout (60 seconds for slow connections)
    pub fn http_receive(&self) -> u64 {
        self.secs_to_ticks(60)
    }
    
    /// Main loop iteration warning threshold (5ms)
    pub fn loop_iteration_warning(&self) -> u64 {
        self.ms_to_ticks(5)
    }
}
```

## 5.8 Invariants

| ID | Invariant | Verification |
|----|-----------|--------------|
| **SM-1** | `step()` returns immediately | Code review |
| **SM-2** | No loops waiting for conditions | Code review, grep for `while` |
| **SM-3** | Timeout checked before any blocking-like operation | Code review |
| **SM-4** | State transitions are explicit | Match arms exhaustive |
| **SM-5** | Nested state machines step once per parent step | Code review |
| **SM-6** | No hardcoded timeout values | grep for numeric literals |

---

# 6. Main Loop & Execution Model

## 6.1 Design Principles

The main loop is the **only execution context** for network operations:

1. **Single entry point** — All network activity flows through main loop
2. **5-phase structure** — Predictable execution order
3. **Bounded iteration** — Target <1ms, maximum 5ms
4. **No re-entrancy** — Functions don't call back into main loop

**Reference**: REDESIGN §3.2, AUDIT §5.4

## 6.2 The 5-Phase Main Loop

```rust
// src/mainloop.rs

/// Main loop execution phases.
/// 
/// Each phase has a specific purpose and bounded execution time.
/// Total iteration target: <1ms, maximum: 5ms.
pub fn main_loop(
    device: &mut impl NetworkDevice,
    iface: &mut Interface,
    sockets: &mut SocketSet,
    app: &mut impl StateMachine,
    handoff: &BootHandoff,
) -> ! {
    let timeouts = TimeoutConfig::new(handoff.tsc_freq);
    
    loop {
        let iteration_start = unsafe { asm_tsc_read() };
        
        // Get current timestamp for smoltcp
        let now_tsc = iteration_start;
        let timestamp = tsc_to_smoltcp_instant(now_tsc, handoff.tsc_freq);
        
        // ═══════════════════════════════════════════════════════
        // PHASE 1: REFILL RX QUEUE
        // Budget: ~20µs
        // Purpose: Ensure device has buffers to receive into
        // ═══════════════════════════════════════════════════════
        device.refill_rx_queue();
        
        // ═══════════════════════════════════════════════════════
        // PHASE 2: SMOLTCP POLL — EXACTLY ONCE
        // Budget: ~200µs
        // Purpose: Process all pending network events
        // 
        // CRITICAL: smoltcp is level-triggered. Calling poll()
        // multiple times per iteration is wasteful and may cause
        // subtle timing bugs. EXACTLY ONCE.
        // ═══════════════════════════════════════════════════════
        let mut adapter = DeviceAdapter::new(device);
        iface.poll(timestamp, &mut adapter, sockets);
        
        // ═══════════════════════════════════════════════════════
        // PHASE 3: DRAIN TX QUEUE
        // Budget: ~40µs (max TX_BUDGET packets)
        // Purpose: Send pending outbound frames
        // 
        // Bounded to prevent one large send from starving RX.
        // ═══════════════════════════════════════════════════════
        const TX_BUDGET: usize = 16;
        adapter.drain_tx(TX_BUDGET);
        
        // ═══════════════════════════════════════════════════════
        // PHASE 4: APPLICATION STATE MACHINE STEP
        // Budget: ~400µs
        // Purpose: Advance application logic (DHCP, HTTP, etc.)
        // 
        // The application state machine gets ONE step per iteration.
        // It must return immediately without blocking.
        // ═══════════════════════════════════════════════════════
        let app_result = app.step(iface, sockets, now_tsc, &timeouts);
        
        match app_result {
            StepResult::Done => {
                // Application completed successfully
                handle_completion(app);
            }
            StepResult::Failed | StepResult::Timeout => {
                // Application failed
                handle_failure(app);
            }
            StepResult::Pending => {
                // Continue next iteration
            }
        }
        
        // ═══════════════════════════════════════════════════════
        // PHASE 5: COLLECT TX COMPLETIONS
        // Budget: ~20µs
        // Purpose: Reclaim TX buffers for reuse
        // 
        // TX submit is fire-and-forget. Completions collected here.
        // ═══════════════════════════════════════════════════════
        device.collect_tx_completions();
        
        // ═══════════════════════════════════════════════════════
        // ITERATION TIMING CHECK (Debug)
        // ═══════════════════════════════════════════════════════
        #[cfg(debug_assertions)]
        {
            let iteration_end = unsafe { asm_tsc_read() };
            let elapsed = iteration_end.wrapping_sub(iteration_start);
            if elapsed > timeouts.loop_iteration_warning() {
                // Log warning: iteration exceeded 5ms
            }
        }
    }
}

/// Convert TSC ticks to smoltcp Instant.
fn tsc_to_smoltcp_instant(tsc: u64, tsc_freq: u64) -> smoltcp::time::Instant {
    // Convert to milliseconds
    let ms = tsc / (tsc_freq / 1_000);
    smoltcp::time::Instant::from_millis(ms as i64)
}
```

## 6.3 Phase Budget Breakdown

| Phase | Target | Maximum | Activity |
|-------|--------|---------|----------|
| **1. RX Refill** | 20µs | 100µs | Submit empty buffers to RX queue |
| **2. smoltcp Poll** | 200µs | 1ms | Process TCP/IP stack events |
| **3. TX Drain** | 40µs | 200µs | Send up to 16 pending frames |
| **4. App Step** | 400µs | 2ms | One state machine step |
| **5. TX Complete** | 20µs | 100µs | Reclaim completed TX buffers |
| **Total** | ~700µs | 5ms | Full iteration |

## 6.4 smoltcp Integration

### Why EXACTLY ONCE?

smoltcp's `poll()` is **level-triggered**:
- It processes ALL pending events in one call
- Calling multiple times wastes CPU
- Multiple calls may cause timestamp inconsistencies

```rust
// ❌ WRONG: Multiple polls per iteration
loop {
    iface.poll(...);  // First poll
    
    if need_to_send {
        socket.send(...);
        iface.poll(...);  // Second poll - WRONG
    }
}

// ✅ CORRECT: Single poll per iteration
loop {
    // Phase 2: exactly once
    iface.poll(timestamp, device, sockets);
    
    // Phase 4: app may queue data, but doesn't poll
    app.step(iface, sockets, ...);
    
    // Queued data sent next iteration's Phase 2
}
```

### DeviceAdapter Implementation

```rust
// src/stack/adapter.rs

/// Adapter bridging NetworkDevice to smoltcp Device trait.
pub struct DeviceAdapter<'a, D: NetworkDevice> {
    device: &'a mut D,
    tx_pending: VecDeque<Vec<u8>>,
}

impl<'a, D: NetworkDevice> DeviceAdapter<'a, D> {
    pub fn new(device: &'a mut D) -> Self {
        Self {
            device,
            tx_pending: VecDeque::new(),
        }
    }
    
    /// Drain TX queue (Phase 3).
    pub fn drain_tx(&mut self, budget: usize) {
        for _ in 0..budget {
            if let Some(frame) = self.tx_pending.pop_front() {
                if self.device.transmit(&frame).is_err() {
                    // Queue full, put back and stop
                    self.tx_pending.push_front(frame);
                    break;
                }
            } else {
                break;
            }
        }
    }
}

impl<'a, D: NetworkDevice> smoltcp::phy::Device for DeviceAdapter<'a, D> {
    type RxToken<'b> = RxToken<'b> where Self: 'b;
    type TxToken<'b> = TxToken<'b, D> where Self: 'b;
    
    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Check for received packet
        let mut buffer = [0u8; 1514];
        match self.device.receive(&mut buffer) {
            Ok(Some(len)) => {
                Some((
                    RxToken { buffer: buffer[..len].to_vec() },
                    TxToken { adapter: self },
                ))
            }
            _ => None,
        }
    }
    
    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.device.can_transmit() {
            Some(TxToken { adapter: self })
        } else {
            None
        }
    }
    
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps.max_burst_size = Some(1);
        caps
    }
}

/// RX token for smoltcp.
pub struct RxToken {
    buffer: Vec<u8>,
}

impl smoltcp::phy::RxToken for RxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.buffer.clone())
    }
}

/// TX token for smoltcp.
pub struct TxToken<'a, D: NetworkDevice> {
    adapter: &'a mut DeviceAdapter<'a, D>,
}

impl<'a, D: NetworkDevice> smoltcp::phy::TxToken for TxToken<'a, D> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let result = f(&mut buffer);
        
        // Queue for transmission (not sent immediately!)
        // Sent in Phase 3 drain_tx()
        self.adapter.tx_pending.push_back(buffer);
        
        result
    }
}
```

## 6.5 Invariants

| ID | Invariant | Reference |
|----|-----------|-----------|
| **LOOP-1** | Main loop never exits (except fatal) | Design |
| **LOOP-2** | smoltcp poll exactly once per iteration | §6.4 |
| **LOOP-3** | Iteration time <5ms | §6.3 |
| **LOOP-4** | TX budget limits Phase 3 | §6.3 |
| **LOOP-5** | App state machine gets one step | §6.2 |
| **LOOP-6** | No function re-enters main loop | Design |
| **LOOP-7** | Timestamp consistent within iteration | Use iteration_start |

---

# 7. Boot Integration

## 7.1 Two-Phase Boot Model

Network initialization spans the ExitBootServices boundary:

```
┌─────────────────────────────────────────────────────────────────┐
│            PHASE 1: UEFI BOOT SERVICES ACTIVE                   │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ 1. Verify CPU features (Invariant TSC)                    │  │
│  │ 2. Calibrate TSC using UEFI Stall()                       │  │
│  │ 3. Allocate DMA region via PCI I/O Protocol               │  │
│  │ 4. Allocate stack (64KB minimum)                          │  │
│  │ 5. Scan PCI for VirtIO NIC (record MMIO base only)        │  │
│  │ 6. Get final memory map                                   │  │
│  │ 7. Populate BootHandoff structure                         │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ═══════════════════ ExitBootServices() ═══════════════════════ │
│                    POINT OF NO RETURN                           │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│            PHASE 2: BARE METAL (NO UEFI)                        │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ 8. Switch to pre-allocated stack                          │  │
│  │ 9. Remap DMA region as UC (if needed, via page tables)    │  │
│  │ 10. Initialize VirtIO NIC (full init sequence)            │  │
│  │ 11. Create smoltcp Interface                              │  │
│  │ 12. Enter main poll loop (NEVER RETURNS)                  │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Reference**: REDESIGN §5, AUDIT §7.1

## 7.2 BootHandoff Structure

The BootHandoff structure transfers information across ExitBootServices:

```rust
/// Data passed from UEFI boot phase to bare-metal phase.
/// 
/// Populated before ExitBootServices, consumed after.
/// All pointers must remain valid post-EBS.
#[repr(C)]
pub struct BootHandoff {
    // ═══════════════════════════════════════════════════════════
    // HEADER
    // ═══════════════════════════════════════════════════════════
    
    /// Magic number for validation: "MORPHEUS" = 0x4D4F5250_48455553
    pub magic: u64,
    
    /// Structure version (currently 1)
    pub version: u32,
    
    /// Structure size in bytes
    pub size: u32,
    
    // ═══════════════════════════════════════════════════════════
    // NIC INFORMATION
    // ═══════════════════════════════════════════════════════════
    
    /// VirtIO MMIO base address (from PCI BAR)
    pub nic_mmio_base: u64,
    
    /// PCI location
    pub nic_pci_bus: u8,
    pub nic_pci_device: u8,
    pub nic_pci_function: u8,
    
    /// NIC type: 0=None, 1=VirtIO, 2=Intel, 3=Realtek
    pub nic_type: u8,
    
    /// MAC address (if known from PCI config, else zeros)
    pub mac_address: [u8; 6],
    
    /// Padding for alignment
    pub _pad1: [u8; 2],
    
    // ═══════════════════════════════════════════════════════════
    // DMA REGION
    // ═══════════════════════════════════════════════════════════
    
    /// CPU pointer for software access
    pub dma_cpu_ptr: u64,
    
    /// Bus address for device DMA
    pub dma_bus_addr: u64,
    
    /// Region size (minimum 2MB)
    pub dma_size: u64,
    
    // ═══════════════════════════════════════════════════════════
    // TIMING (REQUIRED - NO DEFAULTS)
    // ═══════════════════════════════════════════════════════════
    
    /// Calibrated TSC frequency (ticks per second)
    /// 
    /// MUST be calibrated at boot using UEFI Stall().
    /// NO HARDCODED VALUES.
    pub tsc_freq: u64,
    
    // ═══════════════════════════════════════════════════════════
    // STACK
    // ═══════════════════════════════════════════════════════════
    
    /// Top of stack (highest address, stack grows down)
    pub stack_top: u64,
    
    /// Stack size in bytes (minimum 64KB)
    pub stack_size: u64,
    
    // ═══════════════════════════════════════════════════════════
    // DEBUG / OPTIONAL
    // ═══════════════════════════════════════════════════════════
    
    /// Framebuffer base for debug output (0 if unavailable)
    pub framebuffer_base: u64,
    
    /// Framebuffer dimensions
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_stride: u32,
    
    /// Reserved for future use
    pub _reserved: [u8; 64],
}

impl BootHandoff {
    pub const MAGIC: u64 = 0x4D4F5250_48455553;  // "MORPHEUS"
    pub const VERSION: u32 = 1;
    
    /// Validate handoff structure.
    pub fn validate(&self) -> Result<(), HandoffError> {
        if self.magic != Self::MAGIC {
            return Err(HandoffError::InvalidMagic);
        }
        if self.version != Self::VERSION {
            return Err(HandoffError::UnsupportedVersion);
        }
        if self.tsc_freq == 0 {
            return Err(HandoffError::TscNotCalibrated);
        }
        if self.dma_size < 2 * 1024 * 1024 {
            return Err(HandoffError::DmaRegionTooSmall);
        }
        if self.stack_size < 64 * 1024 {
            return Err(HandoffError::StackTooSmall);
        }
        Ok(())
    }
}
```

## 7.3 TSC Calibration (Pre-EBS)

```rust
/// Calibrate TSC frequency using UEFI Stall().
/// 
/// MUST be called before ExitBootServices.
/// 
/// # Arguments
/// - `boot_services`: UEFI boot services table
/// 
/// # Returns
/// TSC frequency in ticks per second.
/// 
/// # Reference
/// REDESIGN §5.4, AUDIT §7.1.4
pub fn calibrate_tsc(boot_services: &BootServices) -> u64 {
    // Verify invariant TSC is available
    let cpuid_result = unsafe { core::arch::x86_64::__cpuid(0x80000007) };
    let invariant_tsc = (cpuid_result.edx >> 8) & 1 != 0;
    
    if !invariant_tsc {
        // WARNING: TSC may vary with CPU frequency
        // Proceed anyway, but timing may be inaccurate
    }
    
    // Read TSC before and after 1-second delay
    let start = unsafe { asm_tsc_read() };
    
    // UEFI Stall() takes microseconds
    boot_services.stall(1_000_000);  // 1 second = 1,000,000 µs
    
    let end = unsafe { asm_tsc_read() };
    
    // Calculate frequency
    let tsc_freq = end.wrapping_sub(start);
    
    // Sanity check (expect 1-5 GHz range)
    assert!(tsc_freq > 1_000_000_000, "TSC frequency too low");
    assert!(tsc_freq < 10_000_000_000, "TSC frequency too high");
    
    tsc_freq
}
```

## 7.4 DMA Allocation (Pre-EBS)

```rust
/// Allocate DMA region using PCI I/O Protocol.
/// 
/// MUST be called before ExitBootServices.
/// 
/// # Arguments
/// - `pci_io`: PCI Root Bridge I/O Protocol
/// - `size`: Region size (minimum 2MB)
/// 
/// # Returns
/// DMA region with CPU pointer and bus address.
/// 
/// # Reference
/// AUDIT §7.2.1 (correction: use PCI I/O Protocol, not raw AllocatePages)
pub fn allocate_dma_region(
    pci_io: &PciRootBridgeIoProtocol,
    size: usize,
) -> Result<DmaRegion, AllocationError> {
    let pages = (size + 0xFFF) / 0x1000;  // Round up to pages
    
    // Allocate buffer via PCI I/O Protocol
    // This handles IOMMU correctly
    let cpu_addr = pci_io.allocate_buffer(
        AllocationType::MaxAddress(0xFFFF_FFFF),  // Below 4GB
        MemoryType::EfiBootServicesData,
        pages,
    )?;
    
    // Map to get bus address
    let (bus_addr, mapping) = pci_io.map(
        PciIoOperation::BusMasterCommonBuffer,
        cpu_addr,
        size,
    )?;
    
    // Note: Cannot set memory type (UC/WC) via UEFI
    // May need page table manipulation post-EBS
    
    Ok(DmaRegion {
        cpu_ptr: cpu_addr as *mut u8,
        bus_addr,
        size,
        mapping_token: mapping,
    })
}
```

## 7.5 PCI Discovery (Pre-EBS)

```rust
/// Scan PCI for VirtIO network device.
/// 
/// MUST be called before ExitBootServices.
/// Does NOT initialize the device—only locates it.
/// 
/// # Reference
/// REDESIGN §5.1, AUDIT §5.2 (NIC init AFTER EBS)
pub fn find_virtio_nic(pci_io: &PciRootBridgeIoProtocol) -> Option<NicInfo> {
    // Scan all PCI buses
    for bus in 0..=255 {
        for device in 0..32 {
            for function in 0..8 {
                let addr = pci_config_addr(bus, device, function, 0);
                
                // Read vendor/device ID
                let vendor_device = pci_io.pci_read32(addr);
                let vendor = (vendor_device & 0xFFFF) as u16;
                let device_id = ((vendor_device >> 16) & 0xFFFF) as u16;
                
                if vendor == 0xFFFF {
                    continue;  // No device
                }
                
                // Check for VirtIO-net
                if is_virtio_net(vendor, device_id) {
                    // Read BAR0 for MMIO base
                    let bar0 = pci_io.pci_read32(
                        pci_config_addr(bus, device, function, 0x10)
                    );
                    
                    let mmio_base = if bar0 & 0x1 == 0 {
                        // Memory BAR
                        (bar0 & 0xFFFFFFF0) as u64
                    } else {
                        // I/O BAR - not supported
                        continue;
                    };
                    
                    return Some(NicInfo {
                        mmio_base,
                        pci_bus: bus,
                        pci_device: device,
                        pci_function: function,
                        nic_type: NicType::VirtIO,
                    });
                }
            }
        }
    }
    
    None
}
```

## 7.6 Post-EBS Initialization

```rust
/// Initialize network stack after ExitBootServices.
/// 
/// # Arguments
/// - `handoff`: Populated BootHandoff structure
/// 
/// # Returns
/// Never returns (enters main loop).
/// 
/// # Reference
/// REDESIGN §5.6, AUDIT §5.2
pub fn post_ebs_init(handoff: &BootHandoff) -> ! {
    // Validate handoff
    handoff.validate().expect("Invalid BootHandoff");
    
    // ═══════════════════════════════════════════════════════════
    // STEP 1: SWITCH TO PRE-ALLOCATED STACK
    // ═══════════════════════════════════════════════════════════
    // (Usually done in assembly before calling Rust)
    
    // ═══════════════════════════════════════════════════════════
    // STEP 2: REMAP DMA AS UNCACHED (if needed)
    // ═══════════════════════════════════════════════════════════
    // This requires page table manipulation
    // Deferred to hardware testing phase
    
    // ═══════════════════════════════════════════════════════════
    // STEP 3: INITIALIZE DMA POOLS
    // ═══════════════════════════════════════════════════════════
    let dma = DmaRegion {
        cpu_ptr: handoff.dma_cpu_ptr as *mut u8,
        bus_addr: handoff.dma_bus_addr,
        size: handoff.dma_size as usize,
    };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 4: INITIALIZE NIC DRIVER
    // ═══════════════════════════════════════════════════════════
    let mut driver = match handoff.nic_type {
        1 => {
            // VirtIO
            virtio_net_init(handoff.nic_mmio_base, &dma)
                .expect("VirtIO init failed")
        }
        _ => panic!("Unsupported NIC type"),
    };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 5: CREATE SMOLTCP INTERFACE
    // ═══════════════════════════════════════════════════════════
    let mac = driver.mac_address();
    let hardware_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
    
    let mut config = Config::new(hardware_addr);
    config.random_seed = generate_random_seed();
    
    let mut iface = Interface::new(config, &mut DeviceAdapter::new(&mut driver));
    
    // Configure IP (will be set by DHCP)
    iface.update_ip_addrs(|addrs| {
        // Initially no IP
    });
    
    // ═══════════════════════════════════════════════════════════
    // STEP 6: CREATE SOCKET SET
    // ═══════════════════════════════════════════════════════════
    let mut sockets = SocketSet::new(vec![]);
    
    // Add DHCP socket
    let dhcp_socket = Dhcpv4Socket::new();
    let dhcp_handle = sockets.add(dhcp_socket);
    
    // Pre-allocate TCP sockets for HTTP
    for _ in 0..4 {
        let rx = TcpSocketBuffer::new(vec![0; 65535]);
        let tx = TcpSocketBuffer::new(vec![0; 65535]);
        let tcp = TcpSocket::new(rx, tx);
        sockets.add(tcp);
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 7: CREATE APPLICATION STATE MACHINE
    // ═══════════════════════════════════════════════════════════
    let mut app = IsoDownloadState::new(/* url from config */);
    
    // ═══════════════════════════════════════════════════════════
    // STEP 8: ENTER MAIN LOOP (NEVER RETURNS)
    // ═══════════════════════════════════════════════════════════
    main_loop(&mut driver, &mut iface, &mut sockets, &mut app, handoff)
}
```

## 7.7 Memory Map Considerations

After ExitBootServices, the memory map is fixed:

| Type | Treatment |
|------|-----------|
| `EfiLoaderCode` | Bootloader code (us) |
| `EfiLoaderData` | Our allocations (DMA, stack) |
| `EfiBootServicesCode` | Now available (freed) |
| `EfiBootServicesData` | Now available (freed) |
| `EfiRuntimeServicesCode` | UEFI runtime (do not touch) |
| `EfiRuntimeServicesData` | UEFI runtime (do not touch) |
| `EfiConventionalMemory` | Free memory |
| `EfiACPIReclaimMemory` | ACPI tables (can reclaim after parsing) |
| `EfiACPIMemoryNVS` | ACPI NVS (do not touch) |
| `EfiMemoryMappedIO` | Device MMIO |
| `EfiMemoryMappedIOPortSpace` | Device I/O ports |

**Critical**: Our DMA region must be `EfiBootServicesData` or `EfiLoaderData` to survive ExitBootServices.

## 7.8 Invariants

| ID | Invariant | Reference |
|----|-----------|-----------|
| **BOOT-1** | TSC calibrated before EBS | §7.3 |
| **BOOT-2** | DMA allocated via PCI I/O Protocol | §7.4 |
| **BOOT-3** | NIC located but NOT initialized pre-EBS | §7.5 |
| **BOOT-4** | NIC initialized AFTER EBS | §7.6 |
| **BOOT-5** | BootHandoff validated before use | §7.2 |
| **BOOT-6** | Memory map recorded before EBS | REDESIGN §5.2 |
| **BOOT-7** | No UEFI calls after EBS | Design |

---

# 8. Driver Abstraction Layer

## 8.1 Design Principles

The driver abstraction enables:

1. **Pluggable drivers** — New hardware = implement traits
2. **Uniform interface** — Higher layers don't know driver type
3. **Factory pattern** — Auto-detect and create appropriate driver
4. **Shared ASM** — Generic functions reused across drivers

**Reference**: §1.3 Layered Architecture

## 8.2 NetworkDevice Trait

```rust
// src/device/mod.rs

/// Core network device interface.
/// 
/// All NIC drivers must implement this trait.
/// Higher layers (smoltcp adapter, state machines) use this interface.
pub trait NetworkDevice {
    /// Get MAC address.
    fn mac_address(&self) -> [u8; 6];
    
    /// Check if device can accept a TX frame.
    /// 
    /// Returns true if `transmit()` will succeed.
    /// Used for backpressure.
    fn can_transmit(&self) -> bool;
    
    /// Check if device has a received frame ready.
    /// 
    /// Returns true if `receive()` will return `Ok(Some(_))`.
    fn can_receive(&self) -> bool;
    
    /// Transmit an Ethernet frame.
    /// 
    /// # Arguments
    /// - `frame`: Complete Ethernet frame (no VirtIO header)
    /// 
    /// # Returns
    /// - `Ok(())`: Frame queued (fire-and-forget)
    /// - `Err(TxError::QueueFull)`: No space, try again later
    /// 
    /// # Contract
    /// - MUST return immediately (no completion wait)
    /// - Caller should check `can_transmit()` first
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError>;
    
    /// Receive an Ethernet frame.
    /// 
    /// # Arguments
    /// - `buffer`: Buffer to copy frame into
    /// 
    /// # Returns
    /// - `Ok(Some(len))`: Frame received, `len` bytes copied
    /// - `Ok(None)`: No frame available (normal)
    /// - `Err(RxError)`: Receive error
    /// 
    /// # Contract
    /// - MUST return immediately (no blocking)
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError>;
    
    /// Refill RX queue with available buffers.
    /// 
    /// Called in main loop Phase 1.
    fn refill_rx_queue(&mut self);
    
    /// Collect TX completions.
    /// 
    /// Called in main loop Phase 5.
    fn collect_tx_completions(&mut self);
}

/// TX errors.
#[derive(Debug, Clone, Copy)]
pub enum TxError {
    /// TX queue is full, try again after completions collected
    QueueFull,
    /// Device not ready
    DeviceNotReady,
}

/// RX errors.
#[derive(Debug, Clone, Copy)]
pub enum RxError {
    /// Provided buffer too small for frame
    BufferTooSmall { needed: usize },
    /// Device error
    DeviceError,
}
```

## 8.3 NicDriver Trait (Factory Integration)

```rust
// src/device/factory.rs

/// Extended trait for factory integration.
/// 
/// Drivers implement this to be auto-detected.
pub trait NicDriver: NetworkDevice + Sized {
    /// Error type for initialization.
    type Error: core::fmt::Debug;
    
    /// PCI vendor IDs this driver supports.
    fn supported_vendors() -> &'static [u16];
    
    /// PCI device IDs this driver supports.
    fn supported_devices() -> &'static [u16];
    
    /// Check if driver supports a PCI device.
    fn supports_device(vendor: u16, device: u16) -> bool {
        Self::supported_vendors().contains(&vendor) &&
        Self::supported_devices().contains(&device)
    }
    
    /// Create driver from MMIO base and DMA region.
    /// 
    /// # Safety
    /// - `mmio_base` must be valid device MMIO address
    /// - `dma` must be valid DMA region
    unsafe fn create(
        mmio_base: u64,
        dma: &mut DmaRegion,
    ) -> Result<Self, Self::Error>;
}
```

## 8.4 VirtIO Driver Implementation

```rust
// src/device/virtio.rs

/// VirtIO-net driver.
pub struct VirtioNetDriver {
    mmio_base: u64,
    mac: [u8; 6],
    features: u64,
    rx_queue: Virtqueue,
    tx_queue: Virtqueue,
    rx_pool: BufferPool,
    tx_pool: BufferPool,
}

impl NetworkDevice for VirtioNetDriver {
    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }
    
    fn can_transmit(&self) -> bool {
        self.tx_pool.available() > 0
    }
    
    fn can_receive(&self) -> bool {
        // Check used ring has entries
        unsafe {
            let mut result = RxResult::default();
            // Peek without consuming
            asm_vq_poll_rx(&self.rx_queue.state, &mut result) == 1
        }
    }
    
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        // Implementation from §4.6
        // ...
    }
    
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        // Implementation from §4.7
        // ...
    }
    
    fn refill_rx_queue(&mut self) {
        // Re-submit any available RX buffers
        while let Some(buf) = self.rx_pool.alloc() {
            unsafe { buf.mark_device_owned(); }
            let result = unsafe {
                asm_vq_submit_rx(&mut self.rx_queue.state, buf.index(), BUFFER_SIZE as u16)
            };
            if result != 0 {
                // Queue full, return buffer
                unsafe { buf.mark_driver_owned(); }
                self.rx_pool.free(buf);
                break;
            }
        }
    }
    
    fn collect_tx_completions(&mut self) {
        // Implementation from §4.6
        loop {
            let idx = unsafe { asm_vq_poll_tx_complete(&mut self.tx_queue.state) };
            if idx == 0xFFFFFFFF {
                break;
            }
            let buf = self.tx_pool.get_mut(idx as u16);
            unsafe { buf.mark_driver_owned(); }
            self.tx_pool.free(buf);
        }
    }
}

impl NicDriver for VirtioNetDriver {
    type Error = VirtioError;
    
    fn supported_vendors() -> &'static [u16] {
        &[0x1AF4]  // VirtIO vendor
    }
    
    fn supported_devices() -> &'static [u16] {
        &[0x1000, 0x1041]  // virtio-net legacy and modern
    }
    
    unsafe fn create(
        mmio_base: u64,
        dma: &mut DmaRegion,
    ) -> Result<Self, Self::Error> {
        // Full initialization from §4.5
        virtio_net_init(mmio_base, dma)
    }
}
```

## 8.5 Device Factory

```rust
// src/device/factory.rs

/// Supported driver types.
#[derive(Debug, Clone, Copy)]
pub enum DriverType {
    VirtIO,
    // Future: Intel, Realtek, Broadcom
}

/// Unified device wrapper.
/// 
/// Hides concrete driver type from higher layers.
pub enum UnifiedNetDevice {
    VirtIO(VirtioNetDriver),
    // Future: Intel(IntelDriver), etc.
}

impl NetworkDevice for UnifiedNetDevice {
    fn mac_address(&self) -> [u8; 6] {
        match self {
            Self::VirtIO(d) => d.mac_address(),
        }
    }
    
    fn can_transmit(&self) -> bool {
        match self {
            Self::VirtIO(d) => d.can_transmit(),
        }
    }
    
    fn can_receive(&self) -> bool {
        match self {
            Self::VirtIO(d) => d.can_receive(),
        }
    }
    
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        match self {
            Self::VirtIO(d) => d.transmit(frame),
        }
    }
    
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        match self {
            Self::VirtIO(d) => d.receive(buffer),
        }
    }
    
    fn refill_rx_queue(&mut self) {
        match self {
            Self::VirtIO(d) => d.refill_rx_queue(),
        }
    }
    
    fn collect_tx_completions(&mut self) {
        match self {
            Self::VirtIO(d) => d.collect_tx_completions(),
        }
    }
}

/// Device factory.
pub struct DeviceFactory;

impl DeviceFactory {
    /// Auto-detect and create appropriate driver.
    /// 
    /// # Arguments
    /// - `vendor`: PCI vendor ID
    /// - `device`: PCI device ID
    /// - `mmio_base`: Device MMIO base address
    /// - `dma`: DMA region
    /// 
    /// # Returns
    /// Unified device wrapper or error.
    pub unsafe fn create_auto(
        vendor: u16,
        device: u16,
        mmio_base: u64,
        dma: &mut DmaRegion,
    ) -> Result<UnifiedNetDevice, FactoryError> {
        // Check VirtIO
        if VirtioNetDriver::supports_device(vendor, device) {
            let driver = VirtioNetDriver::create(mmio_base, dma)
                .map_err(FactoryError::VirtioError)?;
            return Ok(UnifiedNetDevice::VirtIO(driver));
        }
        
        // Future: Check Intel, Realtek, etc.
        
        Err(FactoryError::UnsupportedDevice { vendor, device })
    }
}

#[derive(Debug)]
pub enum FactoryError {
    UnsupportedDevice { vendor: u16, device: u16 },
    VirtioError(VirtioError),
    // Future: IntelError, RealtekError, etc.
}
```

## 8.6 Adding a New Driver

To add support for a new NIC (e.g., Intel e1000):

### Step 1: Create ASM Functions (if needed)

```nasm
; asm/intel.s

global asm_e1k_reset
global asm_e1k_read_eeprom
; ... etc

asm_e1k_reset:
    ; Intel-specific reset sequence
    ret
```

### Step 2: Create Driver Module

```rust
// src/device/intel.rs

pub struct IntelE1000Driver {
    mmio_base: u64,
    mac: [u8; 6],
    // ... driver state
}

impl NetworkDevice for IntelE1000Driver {
    // Implement all methods
}

impl NicDriver for IntelE1000Driver {
    type Error = IntelError;
    
    fn supported_vendors() -> &'static [u16] { &[0x8086] }
    fn supported_devices() -> &'static [u16] { &[0x100E, 0x100F, /* ... */] }
    
    unsafe fn create(mmio_base: u64, dma: &mut DmaRegion) -> Result<Self, Self::Error> {
        // Initialize Intel NIC
    }
}
```

### Step 3: Update Factory

```rust
// In factory.rs

pub enum UnifiedNetDevice {
    VirtIO(VirtioNetDriver),
    Intel(IntelE1000Driver),  // Add variant
}

// Update all match arms in NetworkDevice impl

impl DeviceFactory {
    pub unsafe fn create_auto(...) -> Result<UnifiedNetDevice, FactoryError> {
        // Check VirtIO
        if VirtioNetDriver::supports_device(vendor, device) { ... }
        
        // Check Intel (NEW)
        if IntelE1000Driver::supports_device(vendor, device) {
            let driver = IntelE1000Driver::create(mmio_base, dma)?;
            return Ok(UnifiedNetDevice::Intel(driver));
        }
        
        Err(FactoryError::UnsupportedDevice { ... })
    }
}
```

### Step 4: Update Build

```rust
// build.rs - add new ASM file
let asm_files = ["asm/generic.s", "asm/virtio.s", "asm/intel.s"];
```

## 8.7 Invariants

| ID | Invariant | Verification |
|----|-----------|--------------|
| **DRV-1** | All drivers implement NetworkDevice | Compiler enforced |
| **DRV-2** | Factory covers all driver types | Match exhaustiveness |
| **DRV-3** | PCI IDs don't overlap between drivers | Code review |
| **DRV-4** | Generic ASM shared by all drivers | File organization |
| **DRV-5** | Driver-specific ASM isolated | File organization |

---

# 9. smoltcp Integration

## 9.1 Configuration

```rust
/// Create smoltcp configuration.
pub fn create_smoltcp_config(mac: [u8; 6]) -> Config {
    let hardware_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
    let mut config = Config::new(hardware_addr);
    
    // Random seed for TCP sequence numbers
    config.random_seed = generate_random_seed();
    
    config
}

/// Create smoltcp Interface.
pub fn create_interface<D: NetworkDevice>(
    config: Config,
    device: &mut DeviceAdapter<D>,
) -> Interface {
    Interface::new(config, device)
}
```

## 9.2 Socket Set Pre-allocation

```rust
/// Create socket set with pre-allocated sockets.
pub fn create_socket_set() -> SocketSet<'static> {
    let mut sockets = SocketSet::new(vec![]);
    
    // DHCP socket (required)
    let dhcp = Dhcpv4Socket::new();
    sockets.add(dhcp);
    
    // TCP sockets (for HTTP)
    for _ in 0..4 {
        let rx_buffer = TcpSocketBuffer::new(vec![0; 65535]);
        let tx_buffer = TcpSocketBuffer::new(vec![0; 65535]);
        let tcp = TcpSocket::new(rx_buffer, tx_buffer);
        sockets.add(tcp);
    }
    
    // Optional: DNS socket
    let dns_rx = UdpSocketBuffer::new(vec![UdpPacketMetadata::EMPTY; 4], vec![0; 1024]);
    let dns_tx = UdpSocketBuffer::new(vec![UdpPacketMetadata::EMPTY; 4], vec![0; 1024]);
    let dns = UdpSocket::new(dns_rx, dns_tx);
    sockets.add(dns);
    
    sockets
}
```

## 9.3 DHCP Integration

```rust
/// Process DHCP events from smoltcp.
pub fn process_dhcp(
    iface: &mut Interface,
    sockets: &mut SocketSet,
    dhcp_handle: SocketHandle,
) -> Option<DhcpConfig> {
    let socket = sockets.get_mut::<Dhcpv4Socket>(dhcp_handle);
    
    match socket.poll() {
        Some(Dhcpv4Event::Configured(config)) => {
            // Apply configuration to interface
            iface.update_ip_addrs(|addrs| {
                addrs.clear();
                addrs.push(IpCidr::Ipv4(config.address)).unwrap();
            });
            
            // Set default gateway
            if let Some(router) = config.router {
                iface.routes_mut().add_default_ipv4_route(router).unwrap();
            }
            
            Some(DhcpConfig {
                ip: config.address.address(),
                gateway: config.router,
                dns: config.dns_servers.get(0).copied(),
            })
        }
        Some(Dhcpv4Event::Deconfigured) => {
            // Lost lease
            iface.update_ip_addrs(|addrs| addrs.clear());
            None
        }
        None => None,
    }
}
```

---

# 10. Testing & Validation

## 10.1 Testing Strategy

| Level | Scope | Environment |
|-------|-------|-------------|
| **Unit** | Individual functions, state machines | Host (x86_64-unknown-linux-gnu) |
| **Integration** | Driver + stack | QEMU with virtio-net |
| **System** | Full boot → download | QEMU with user networking |
| **Hardware** | Real NIC drivers | Physical machine |

## 10.2 QEMU Test Configuration

```bash
#!/bin/bash
# Run MorpheusX in QEMU with networking

qemu-system-x86_64 \
    -enable-kvm \
    -m 2G \
    -bios /usr/share/OVMF/OVMF_CODE.fd \
    -drive file=esp.img,format=raw \
    -device virtio-net-pci,netdev=net0 \
    -netdev user,id=net0,hostfwd=tcp::8080-:80 \
    -serial stdio \
    -no-reboot
```

## 10.3 Acceptance Tests

### Test: DHCP Completion

```
1. Boot MorpheusX in QEMU
2. Network stack initializes
3. DHCP state machine starts
4. IP address obtained within 30 seconds
5. Gateway and DNS configured
```

### Test: HTTP Download

```
1. Boot with network ready
2. Start HTTP download (http://mirror.example.com/test.iso)
3. HTTP state machine progresses:
   - Resolving → Connecting → SendingRequest → ReceivingHeaders → ReceivingBody → Done
4. Download completes without timeout
5. Bytes received matches Content-Length
```

### Test: Main Loop Timing

```
1. Enable iteration timing instrumentation
2. Run for 10,000 iterations
3. Measure each iteration duration
4. Assert: 99% of iterations < 1ms
5. Assert: 100% of iterations < 5ms
```

## 10.4 Invariant Verification Checklist

| Category | Invariants | Verification Method |
|----------|------------|---------------------|
| ASM | A1-A4 | Code review, symbol check |
| DMA | DMA-1 to DMA-7 | Debug assertions, code review |
| VirtIO | VIO-1 to VIO-8 | Integration test, code review |
| State Machines | SM-1 to SM-6 | Code review, grep |
| Main Loop | LOOP-1 to LOOP-7 | Runtime instrumentation |
| Boot | BOOT-1 to BOOT-7 | Integration test |
| Driver | DRV-1 to DRV-5 | Compiler, code review |

---

# Appendix A: Quick Reference

## A.1 ASM Function Signatures

```rust
// Generic (all drivers)
extern "win64" {
    fn asm_tsc_read() -> u64;
    fn asm_tsc_read_serialized() -> u64;
    fn asm_bar_sfence();
    fn asm_bar_lfence();
    fn asm_bar_mfence();
    fn asm_mmio_read32(addr: u64) -> u32;
    fn asm_mmio_write32(addr: u64, value: u32);
    fn asm_mmio_read16(addr: u64) -> u16;
    fn asm_mmio_write16(addr: u64, value: u16);
}

// VirtIO-specific
extern "win64" {
    fn asm_vq_submit_tx(vq: *mut VirtqueueState, idx: u16, len: u16) -> u32;
    fn asm_vq_poll_tx_complete(vq: *mut VirtqueueState) -> u32;
    fn asm_vq_submit_rx(vq: *mut VirtqueueState, idx: u16, cap: u16) -> u32;
    fn asm_vq_poll_rx(vq: *mut VirtqueueState, result: *mut RxResult) -> u32;
    fn asm_vq_notify(vq: *mut VirtqueueState);
    fn asm_nic_reset(mmio_base: u64) -> u32;
    fn asm_nic_set_status(mmio_base: u64, status: u8);
    fn asm_nic_get_status(mmio_base: u64) -> u8;
    fn asm_nic_read_features(mmio_base: u64) -> u64;
    fn asm_nic_write_features(mmio_base: u64, features: u64);
    fn asm_nic_read_mac(mmio_base: u64, mac_out: *mut [u8; 6]) -> u32;
}
```

## A.2 Main Loop Phases

| Phase | Budget | Purpose |
|-------|--------|---------|
| 1 | 20µs | Refill RX queue |
| 2 | 200µs | smoltcp poll (ONCE) |
| 3 | 40µs | Drain TX queue |
| 4 | 400µs | App state step |
| 5 | 20µs | TX completions |

## A.3 State Machine States

```
DhcpState: Init → Discovering → Bound | Failed
TcpConnState: Closed → Connecting → Established | Error
HttpDownloadState: Init → Resolving → Connecting → SendingRequest → ReceivingHeaders → ReceivingBody → Done | Failed
```

## A.4 VirtIO Init Sequence

```
1. Reset (write 0, wait for 0)
2. Acknowledge (0x01)
3. Driver (0x03)
4. Feature negotiation
5. Features_OK (0x0B)
6. Verify Features_OK
7. Queue setup
8. Pre-fill RX
9. Driver_OK (0x0F)
10. Read MAC
```

---

*End of Implementation Guide*
