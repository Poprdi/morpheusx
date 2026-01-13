# Post-ExitBootServices Reality Check

## Critical Analysis for Real Hardware (ThinkPad T450s)

**Date**: January 2026  
**Status**: CRITICAL ACTION ITEMS  

---

## Executive Summary

After ExitBootServices, we lose:
- UEFI runtime services
- ACPI event handling
- Power management handlers
- SMI handlers (partially)
- Timer interrupts
- All firmware-managed resources

**What could kill us on real hardware:**

| Risk | Impact | Current Status | Priority |
|------|--------|----------------|----------|
| SATA Link Power Management | Disk becomes unresponsive | ⚠️ NOT HANDLED | **P0** |
| Intel NIC PHY Power Down | NIC stops responding | ⚠️ NOT HANDLED | **P0** |
| PCI Power States (D3) | Devices go to sleep | ⚠️ NOT HANDLED | **P0** |
| Hardware Watchdogs | System reset | ⚠️ UNKNOWN | P1 |
| SMM Interrupts | Steals CPU cycles | ✅ Tolerated | P2 |
| Thermal Events | CPU throttle/shutdown | ⚠️ UNKNOWN | P1 |

---

## 1. AHCI/SATA Power Management (P0 - CRITICAL)

### The Problem

UEFI/BIOS may have enabled aggressive link power management on the SATA controller. Key bits in `PxCMD`:

```
Bit 26 (ALPE) - Aggressive Link Power Management Enable
Bit 27 (ASP)  - Aggressive Slumber/Partial
Bit 23 (APSTE) - Automatic Partial to Slumber Transitions
```

If these are set, the SATA link can transition to:
- **Partial state** (IPM=2): Fast wake, but still needs recovery
- **Slumber state** (IPM=6): Slow wake (~10ms), can miss I/O
- **DevSleep** (IPM=8): Even slower wake

### What We Currently Do

**NOTHING.** We read `PxSSTS` to detect link status but don't check or disable power management.

### The Fix (Required)

Add to `asm_ahci_port_start`:

```asm
; CRITICAL: Disable aggressive link power management
; Must do this BEFORE starting the port

; Read current PxCMD
lea     rcx, [r12 + AHCI_PxCMD]
call    asm_mmio_read32

; Clear ALPE (bit 26), ASP (bit 27), APSTE (bit 23)
and     eax, ~(AHCI_PXCMD_ALPE | AHCI_PXCMD_ASP | AHCI_PXCMD_APSTE)

; Set ICC to Active (bits 31:28 = 1)
and     eax, ~AHCI_PXCMD_ICC_MASK
or      eax, (AHCI_ICC_ACTIVE << 28)

; Write back
lea     rcx, [r12 + AHCI_PxCMD]
mov     edx, eax
call    asm_mmio_write32
```

Also need to write `PxSCTL` (SATA Control) to disable DIPM:

```asm
; Disable Device-Initiated Power Management (DIPM)
; PxSCTL.IPM bits [11:8] = 3 (disable Partial & Slumber)
lea     rcx, [r12 + AHCI_PxSCTL]
call    asm_mmio_read32
and     eax, ~0xF00           ; Clear IPM field
or      eax, 0x300            ; Set to 3 = disable Partial + Slumber
lea     rcx, [r12 + AHCI_PxSCTL]
mov     edx, eax
call    asm_mmio_write32
```

### Verification

After disabling, check `PxSSTS.IPM` (bits 11:8):
- Should be 1 (Active) not 2 (Partial) or 6 (Slumber)

---

## 2. Intel NIC Power Management (P0 - CRITICAL)

### The Problem

Intel NICs have multiple power management features that BIOS may enable:
- **PHY Power Down** (BMCR bit 11)
- **PCI Power State D3**
- **Link Power Management (LPM)**
- **Smart Power Down**

### What We Currently Do

We reset the device which should clear most things, but:
1. No explicit PHY power-up
2. No PCI PM state check
3. No verification PHY is active

### The Fix (Required)

Add to Intel init sequence:

```rust
// 1. Wake from D3 if needed (PCI Config Space offset 0x44/0x4C)
// Read PMCSR, check bits 0:1 for power state
let pmcsr = pci_read32(bus, dev, func, 0x44);
if (pmcsr & 0x3) != 0 {  // Not in D0
    // Write D0 (clear bits 0:1)
    pci_write32(bus, dev, func, 0x44, pmcsr & !0x3);
    // Wait 10ms for D3->D0 transition
    tsc_delay_us(10_000);
}

// 2. After reset, ensure PHY is powered
let bmcr = phy_read(BMCR);
if (bmcr & BMCR_PDOWN) != 0 {
    phy_write(BMCR, bmcr & ~BMCR_PDOWN);
    tsc_delay_us(1_000);  // Wait for PHY to wake
}
```

ASM implementation for PHY power check:

```asm
; Ensure PHY is not in power-down mode
global asm_intel_phy_wake
asm_intel_phy_wake:
    ; Read PHY BMCR (reg 0)
    mov     rcx, r12            ; mmio_base
    xor     edx, edx            ; PHY reg 0 = BMCR
    call    asm_intel_phy_read
    
    ; Check PDOWN bit (bit 11)
    test    ax, 0x0800
    jz      .phy_active
    
    ; Clear PDOWN
    and     ax, ~0x0800
    mov     rcx, r12
    xor     edx, edx            ; reg 0
    mov     r8w, ax
    call    asm_intel_phy_write
    
    ; Wait 1ms for PHY to wake
    mov     rcx, 1000           ; 1000us
    call    asm_tsc_delay_us

.phy_active:
    xor     eax, eax
    ret
```

---

## 3. PCI Device Power States (P0 - CRITICAL)

### The Problem

Both AHCI and Intel NIC can be in PCI power state D3 (off).

PCI PM Capability Structure (Cap ID 0x01):
- Offset +4: PMCSR (Power Management Control/Status)
  - Bits 1:0: PowerState (0=D0, 1=D1, 2=D2, 3=D3hot)

### The Fix (Required)

Before touching ANY device, ensure it's in D0:

```rust
/// Ensure PCI device is in D0 power state.
/// Must be called BEFORE any MMIO access.
pub unsafe fn ensure_pci_d0(bus: u8, device: u8, function: u8) -> bool {
    // Find PM capability
    let pm_cap = find_pci_capability(bus, device, function, PCI_CAP_PM);
    if pm_cap == 0 {
        return true;  // No PM cap, assume always on
    }
    
    // Read PMCSR (PM Capability + 4)
    let pmcsr = pci_read16(bus, device, function, pm_cap + 4);
    let power_state = pmcsr & 0x3;
    
    if power_state != 0 {
        // Device is in D1/D2/D3, transition to D0
        let new_pmcsr = pmcsr & !0x3;  // Clear power state bits
        pci_write16(bus, device, function, pm_cap + 4, new_pmcsr);
        
        // Wait for D3->D0 transition (up to 10ms per spec)
        tsc_delay_us(10_000);
        
        // Verify
        let verify = pci_read16(bus, device, function, pm_cap + 4);
        if (verify & 0x3) != 0 {
            return false;  // Failed to wake
        }
    }
    
    true
}
```

---

## 4. Hardware Watchdogs (P1 - Important)

### The Problem

Some platforms have:
- **TCO Watchdog** (Intel chipsets) - Causes system reset if not fed
- **Super I/O Watchdog** - Platform dependent
- **EC Watchdog** (Embedded Controller) - Laptop specific

### ThinkPad T450s Specifics

The T450s has an EC that manages:
- Power button
- Thermal
- Battery
- Potentially a watchdog

UEFI may have started a watchdog that we don't know about.

### Current Status

**UNKNOWN.** We haven't investigated this.

### Investigation Needed

1. Check if TCO watchdog is running:
   - Read TCO registers from LPC bridge (usually at IO 0x60-0x6F range)
   - TCO1_STS, TCO2_STS show if watchdog active

2. If running, either:
   - Disable it (write to TCO1_CNT)
   - Or feed it periodically (not ideal for download)

### Temporary Mitigation

Add to init sequence:

```rust
// Try to disable TCO watchdog (Intel PCH specific)
// LPC Controller PCI Dev 31, Func 0
const TCO_BASE: u16 = 0x400;  // From TCOBASE register

// Read TCO1_CNT
let tco1_cnt = inw(TCO_BASE + 0x08);
// Set bit 11 (TMR_HLT) to stop timer
outw(TCO_BASE + 0x08, tco1_cnt | 0x800);
```

---

## 5. Thermal Management (P1 - Important)

### The Problem

Without ACPI, we have no thermal notification:
- CPU thermal throttling happens automatically (good)
- But shutdown threshold needs EC handling (bad)

### ThinkPad Specifics

ThinkPad EC handles thermal shutdown. As long as EC is running
(which it should be, it's separate from main CPU), we should be OK.

### Mitigation

Keep operations relatively quick. A 1-2GB ISO download should
complete in minutes, not trigger thermal issues.

---

## 6. SMM/SMI (P2 - Tolerable)

### The Problem

System Management Interrupts (SMI) can fire at any time:
- Steal CPU cycles
- Handled by BIOS SMM code
- We can't disable them (locked by firmware)

### Impact

Timing may be slightly off. Not fatal.

### Mitigation

Use generous timeouts. Don't rely on precise timing.

---

## Implementation Priority

### Phase 1 (Must Have Before Real Hardware)

1. **AHCI Power Management Disable**
   - [ ] Add `asm_ahci_disable_link_pm()` function
   - [ ] Call during `asm_ahci_port_start()`
   - [ ] Verify IPM state after

2. **Intel NIC D0 Wake**
   - [ ] Add PCI PM state check
   - [ ] Add PHY power-up verification
   - [ ] Call before `asm_intel_reset()`

3. **Generic PCI D0 Ensure**
   - [ ] Implement `ensure_pci_d0()`
   - [ ] Call for both AHCI and NIC
   - [ ] Add to probe sequence

### Phase 2 (Should Have)

4. **TCO Watchdog Investigation**
   - [ ] Add TCO status read
   - [ ] Add TCO disable if needed
   - [ ] Test on real hardware

### Phase 3 (Nice to Have)

5. **Thermal Monitoring**
   - [ ] Add CPU temperature read (if possible without ACPI)
   - [ ] Log warnings if hot

---

## Testing Plan

### QEMU (Limited)

QEMU doesn't really emulate power management, so QEMU testing won't catch these issues.

### Real Hardware (Required)

1. Boot with laptop on battery (forces power saving)
2. Let laptop sit for 30+ seconds before triggering download
3. Monitor serial output for any "link down" or "device not responding"
4. Test with aggressive BIOS power settings enabled

---

## Conclusion

**We have gaps that WILL bite us on real hardware.**

The most critical are:
1. AHCI link power management (disk goes to sleep)
2. Intel NIC power states (NIC goes to sleep)

These need to be fixed before real ThinkPad testing.

---

## References

- AHCI 1.3.1 Specification, Section 3.3.7 (Port Power Management)
- PCI Bus Power Management Interface Specification 1.2
- Intel 82579 Datasheet, Section 15 (Power Management)
- Intel PCH Datasheet, Section 26 (TCO)
