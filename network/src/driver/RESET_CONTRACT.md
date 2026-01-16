# Driver Reset Contract

## Overview

Every driver init **MUST** perform a brutal, explicit reset to pristine state.
No assumptions about UEFI, BIOS, or previous owner state.

## Preconditions (hwinit guarantees)

Before driver init runs, hwinit has:
- ✓ MMIO BAR mapped and accessible
- ✓ DMA addresses legal (no IOMMU blocking)
- ✓ Cache coherency configured

Note: Driver manages its own bus mastering (disables during reset, re-enables after).

## Driver Init Sequence (Intel e1000e Reference)

### Phase 1: Mask/Clear Interrupts
```
- Write IMC = 0xFFFFFFFF (mask all)
- Read STATUS (flush posted write)
- Read ICR (clear pending)
```

### Phase 2: Disable RX/TX, Wait for Quiescence
```
- Clear RCTL.EN and TCTL.EN
- Read STATUS (flush)
- Poll RXDCTL/TXDCTL until queue enable bits clear
- Timeout: 10ms
```

### Phase 3: Disable Bus Mastering
```
- Set CTRL.GIO_MASTER_DISABLE
- Read STATUS (flush)
- Poll STATUS.GIO_MASTER_EN until clear
- Timeout: 10ms
```

### Phase 4: Device Reset (MANDATORY)
```
- Set CTRL.RST
- Poll until RST bit self-clears
- Timeout: 100ms → HARD FAIL if exceeded
- Post-reset stabilization delay: 10ms
```

### Phase 5: Wait for EEPROM Auto-Read
```
- Poll EECD.AUTO_RD until set
- Timeout: 500ms (generous)
```

### Phase 6: Post-Reset Cleanup
```
- Mask/clear interrupts AGAIN (reset re-enables)
- Zero all descriptor ring pointers (RDBAL/RDBAH/RDLEN/RDH/RDT, TDBAL/...)
- Zero RAR[0] (will reprogram later)
- Clear RCTL.LBM (loopback mode)
- Clear MTA (multicast table)
- Read STATUS after each block (flush)
```

### Phase 7: Chip-Specific Workarounds
```
- I218/PCH: Disable ULP, ensure PHY accessible, wake PHY
- Gate on device ID where possible
```

### Phase 8: Read/Validate MAC
```
- Read MAC from EEPROM
- Validate: not all 0s, not all FFs
- Write to RAL/RAH with AV bit
- Read STATUS (flush)
```

### Phase 9: Program Descriptor Rings
```
- Interrupts still masked - safe to program
- Write RDBAL/RDBAH/RDLEN for RX ring
- Write TDBAL/TDBAH/TDLEN for TX ring
- Initialize all descriptors
- Read STATUS (flush)
```

### Phase 10: Enable RX/TX, Link Up
```
- Clear CTRL.GIO_MASTER_DISABLE (re-enable bus mastering)
- Enable RX (RCTL.EN)
- Write RDT to arm receive
- Enable TX (TCTL.EN)
- Set CTRL.SLU, restart PHY autoneg
- Read STATUS after each (flush)
- Delay 100ms for PHY negotiation start
```

## Critical Implementation Details

### MMIO Write Flushing
PCI posted writes are NOT flushed by CPU memory fences.
After every write block, do a dummy read (e.g., STATUS register).

### Time-Bounded Polls
Every poll MUST have a timeout. Never spin forever.
Timeouts should be generous (hardware can be slow).

### Interrupts
Remain MASKED throughout init (IMS = 0).
For polled I/O, never unmask.
For interrupt-driven operation, unmask ONLY after all rings are programmed.

### Bus Mastering
Driver disables during reset (Phase 3) to prevent stale DMA.
Driver re-enables after rings are programmed (Phase 10).

## Failure Modes

Reset timeout → Device non-functional → Return error, do not proceed

**Never** fall back to "maybe it's okay" state.
If reset fails, the device is unusable.

## Device-Specific Notes

### Intel E1000e/I218
- Reset may not self-clear on some PCH variants (timeout is the fallback)
- EEPROM auto-read takes variable time
- ULP mode must be disabled before PHY access
- LANPHYPC toggle may be needed to wake PHY

### VirtIO
- Device reset via status register (write 0)
- Must wait for status to read back 0
- Reset clears all feature negotiation

### Future Drivers
Follow this contract. No exceptions.
The 10 minutes you save skipping reset will cost 10 hours later.
