---
name: hardware-abstraction
description: |
  Implement hardware drivers: AHCI, VirtIO, PCI, interrupts.
  Follow bare-metal driver patterns with proper error handling.
author: MorpheusX Architecture Team
version: 2026.1
---

## Inputs Required
- Hardware type (AHCI, VirtIO, PCI, interrupt controller)
- Bus addresses and interrupt vectors
- DMA requirements

## Process

### Step 1: PCI Discovery
```rust
// GOOD: Proper PCI config space access
fn read_config_word(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    let address = 0xCF8 | ((bus as u32) << 16)
                 | ((dev as u32) << 11)
                 | ((func as u32) << 8)
                 | ((offset as u32) & 0xFC);
    // ... port I/O access
}
```

### Step 2: Memory-Mapped I/O
- Use `core::ptr::read_volatile` / `write_volatile`
- Proper memory barriers before/after
- No speculative writes

### Step 3: DMA
- Pre-allocated DMA region (2 MB)
- Consistent mapping (non-cached)
- Sync before/after transfers
- No IOMMU (bare metal assumption)

### Step 4: Interrupts
```rust
// GOOD: Interrupt handler registration
devm_request_irq(&pdev->dev, irq_num, handler, 0, name, priv);

// GOOD: Interrupt handler
#[irq_handler(irq_num)]
fn my_handler(_irq: u8, _frame: &InterruptFrame) -> IrqResult {
    // Acknowledge interrupt
    // Handle (minimal work)
    IrqResult::Handled
}
```

## Critical Rules
- All MMIO accesses via volatile pointers
- Memory barriers on all device registers
- Disable interrupts during critical sections
- No sleeping in interrupt handlers
- Validate all bus-master addresses

## References
- PCI Express specification
- AHCI 1.3 specification
- VirtIO 1.1 specification