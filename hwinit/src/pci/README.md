# PCI Module

## Purpose

PCI enumeration, configuration space access, and capability handling.

## Files (to be populated in Phase 3)

| File | Description |
|------|-------------|
| `mod.rs` | Module root, re-exports |
| `config.rs` | PciAddr, config read/write wrappers |
| `capability.rs` | Capability chain walking, VirtIO caps |

## Key Types

```rust
/// PCI device address
pub struct PciAddr {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

/// Discovered PCI device
pub struct PciDevice {
    pub addr: PciAddr,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u32,
    pub bars: [BarInfo; 6],
    pub capabilities: CapabilityList,
}
```

## Operations

- Enumerate all devices on all buses
- Read/write config space (8/16/32 bit)
- Decode BAR addresses and sizes
- Walk capability chains
- Enable bus mastering / memory space

---


