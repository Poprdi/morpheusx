# VirtIO PCI Modern Integration — Session State

**Last Updated**: 2026-01-10  
**Current Session**: 0 (Context Bootstrapping)  
**Status**: BOOTSTRAP COMPLETE

---

## Session Progress Tracker

| Session | Description | Status | Completion |
|---------|-------------|--------|------------|
| 0 | Context/Environment Bootstrapping | ✅ COMPLETE | 100% |
| 1 | PCI Capability Infrastructure | ⏳ PENDING | 0% |
| 2 | VirtIO Transport Abstraction Layer | ⏳ PENDING | 0% |
| 3 | Modern PCI VirtIO-Net Driver Refactor | ⏳ PENDING | 0% |
| 4 | Modern PCI VirtIO-Blk Driver Refactor | ⏳ PENDING | 0% |
| 5 | Download Loop and Streaming ISO Write | ⏳ PENDING | 0% |
| 6 | End-to-End Test & Smoke Verification | ⏳ PENDING | 0% |

---

## Session 0: Context Bootstrapping

### Codebase Inventory

**Core Directories:**
```
network/
├── asm/
│   ├── core/          # TSC, barriers, MMIO, PIO, cache, delay
│   ├── drivers/
│   │   └── virtio/    # init.s, queue.s, tx.s, rx.s, notify.s, blk.s
│   ├── pci/           # legacy.s (CF8/CFC), ecam.s, bar.s
│   └── phy/           # MDIO, MII, link
├── src/
│   ├── boot/          # handoff.rs, init.rs
│   ├── driver/
│   │   ├── virtio/    # config.rs, driver.rs, init.rs, rx.rs, tx.rs
│   │   └── virtio_blk.rs
│   ├── mainloop/      # bare_metal.rs, runner.rs, phases.rs
│   ├── pci/           # mod.rs (placeholder)
│   ├── stack/         # smoltcp adapter
│   ├── state/         # DHCP, TCP, HTTP, download state machines
│   ├── time/          # timeout.rs
│   └── types/         # VirtqueueState, MacAddress, etc.
```

### Critical Files for Integration

| File | Purpose | Lines | Transport Type |
|------|---------|-------|----------------|
| `asm/drivers/virtio/init.s` | VirtIO init (MMIO offsets) | 364 | **MMIO ONLY** |
| `asm/drivers/virtio/queue.s` | Queue setup | - | MMIO |
| `asm/drivers/virtio/blk.s` | Block device ops | 503 | **MMIO ONLY** |
| `asm/pci/legacy.s` | PCI config space (CF8/CFC) | 388 | PCI access |
| `src/boot/handoff.rs` | Boot data transfer | 510 | N/A |
| `src/driver/virtio/driver.rs` | VirtIO-net driver | ~150 | **MMIO ONLY** |
| `src/driver/virtio_blk.rs` | VirtIO-blk driver | 604 | **MMIO ONLY** |
| `src/mainloop/bare_metal.rs` | Main orchestration | 1070 | N/A |

### BootHandoff Structure (Current State)

```rust
#[repr(C, align(64))]
pub struct BootHandoff {
    // Header (16 bytes)
    magic: u64,
    version: u32,
    size: u32,
    
    // NIC Info (24 bytes)
    nic_mmio_base: u64,      // ⚠️ Currently assumes MMIO transport
    nic_pci_bus: u8,
    nic_pci_device: u8,
    nic_pci_function: u8,
    nic_type: u8,
    mac_address: [u8; 6],
    _nic_pad: [u8; 2],
    
    // Block Device (24 bytes)
    blk_mmio_base: u64,      // ⚠️ Currently assumes MMIO transport
    blk_pci_bus: u8,
    blk_pci_device: u8,
    blk_pci_function: u8,
    blk_type: u8,
    blk_sector_size: u32,
    blk_total_sectors: u64,
    
    // DMA, Timing, Stack, Framebuffer, Memory Map...
    // Total: 256 bytes
}
```

### ASM VirtIO MMIO Offsets (Current — Incompatible with PCI)

```asm
; From init.s - These are VirtIO MMIO transport offsets
VIRTIO_MMIO_MAGIC           equ 0x000
VIRTIO_MMIO_VERSION         equ 0x004
VIRTIO_MMIO_DEVICE_ID       equ 0x008
VIRTIO_MMIO_DEVICE_FEATURES equ 0x010
VIRTIO_MMIO_DRIVER_FEATURES equ 0x020
VIRTIO_MMIO_QUEUE_SEL       equ 0x030
VIRTIO_MMIO_QUEUE_NUM_MAX   equ 0x034
VIRTIO_MMIO_QUEUE_NUM       equ 0x038
VIRTIO_MMIO_QUEUE_READY     equ 0x044
VIRTIO_MMIO_QUEUE_NOTIFY    equ 0x050
VIRTIO_MMIO_STATUS          equ 0x070
VIRTIO_MMIO_CONFIG          equ 0x100
```

### VirtIO PCI Modern Capability Structure (Target)

```
PCI Capability Type 0x09 (Vendor-Specific):
├── VIRTIO_PCI_CAP_COMMON_CFG (1)  → Common config access
├── VIRTIO_PCI_CAP_NOTIFY_CFG (2)  → Notification area
├── VIRTIO_PCI_CAP_ISR_CFG (3)     → ISR status
├── VIRTIO_PCI_CAP_DEVICE_CFG (4)  → Device-specific config
└── VIRTIO_PCI_CAP_PCI_CFG (5)     → PCI config access method

Each capability contains:
  - cfg_type (1 byte)
  - bar (1 byte)
  - offset (4 bytes)
  - length (4 bytes)
  - [notify_off_multiplier for notify cap]
```

### What's Missing for PCI Modern Support

1. **PCI Capability Chain Walking (ASM)**
   - Need `asm/pci/capability.s` with functions to:
     - Find capability list pointer (offset 0x34)
     - Walk capability chain
     - Parse VirtIO-specific capabilities

2. **VirtIO Transport Abstraction (Rust)**
   - Need `src/driver/virtio_transport.rs`:
     - `VirtioTransport` enum (MMIO, PciModern, PciLegacy stub)
     - Transport-agnostic register access

3. **BootHandoff Extensions**
   - Need per-device transport type
   - Need PCI capability offsets (common_cfg, notify, device_cfg)
   - Need BAR mappings

4. **Driver Refactor**
   - All register accesses must go through transport layer
   - Notification calculation: `base + queue_notify_off * multiplier`

### QEMU Configuration (Current)

```bash
# test-network.sh uses:
-device virtio-net-pci,netdev=net0,disable-legacy=on
-device virtio-blk-pci,drive=blk0,disable-legacy=on

# Both devices are PCI Modern only (VirtIO 1.0+)
# Current ASM uses MMIO offsets → WILL FAIL
```

### Chunk Size & Session Rules

**Chunk Boundary**: ~350 tokens/code lines OR one logical submodule

**Logical Submodules for Session 1**:
1. PCI capability chain walker (ASM) - ~200 lines
2. VirtIO capability parser (ASM) - ~150 lines
3. Rust PCI capability bindings - ~100 lines
4. Serial log dump function - ~50 lines

**Rules**:
1. After each chunk: update this file
2. On context limit: persist partial + state
3. Serial log every PCI probe/capability
4. Never merge/optimize until both transports work

---

## Carry-Forward Context

### Design Assumptions
- VirtIO PCI Modern uses BAR-relative offsets from capabilities
- Same capability parsing works for both net (0x1041) and blk (0x1042)
- MMIO code remains for potential ARM/embedded use
- Notification: `notify_base + queue_notify_off * notify_off_multiplier`

### Open Questions
- [ ] Should capability parsing happen in bootloader or post-EBS?
- [ ] Cache BAR mappings in BootHandoff or probe at driver init?
- [ ] Single DMA region or separate net/blk regions?

### Next Session (1) Goals
1. Implement `asm_pci_find_cap_list` - get capability pointer from offset 0x34
2. Implement `asm_pci_walk_caps` - iterate capability chain
3. Implement `asm_virtio_parse_caps` - extract VirtIO-specific caps
4. Rust bindings for above
5. Serial log: print all found capabilities for net/blk devices

---

## Changed Files Log

### Session 0
| File | Action | Reason |
|------|--------|--------|
| `docs/INTEGRATION_SESSION_STATE.md` | Created | Session tracking |
| N/A | N/A | Context gathering only |

---

## Code Snippets (For Resume)

### Target VirtioCapability Structure (Rust)
```rust
#[repr(C)]
pub struct VirtioPciCap {
    pub cap_vndr: u8,      // 0x09 for vendor-specific
    pub cap_next: u8,      // Offset to next capability
    pub cap_len: u8,       // Length of this capability
    pub cfg_type: u8,      // COMMON=1, NOTIFY=2, ISR=3, DEVICE=4
    pub bar: u8,           // BAR index (0-5)
    pub padding: [u8; 3],
    pub offset: u32,       // Offset within BAR
    pub length: u32,       // Length of region
}
```

### Target Transport Enum (Rust)
```rust
pub enum VirtioTransport {
    Mmio {
        base: u64,
    },
    PciModern {
        common_cfg_bar: u8,
        common_cfg_offset: u32,
        notify_bar: u8,
        notify_offset: u32,
        notify_multiplier: u32,
        device_cfg_bar: u8,
        device_cfg_offset: u32,
        bar_bases: [u64; 6],
    },
    PciLegacy, // Stub - not implemented
}
```

---

**Next Action**: Begin Session 1 — PCI Capability Infrastructure
