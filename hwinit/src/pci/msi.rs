//! PCI MSI and MSI-X capability discovery and programming.
//!
//! Walks the capability list (already provided by `capability::walk_capabilities_rust`)
//! looking for cap IDs `0x05` (MSI) and `0x11` (MSI-X). Programs message
//! address/data so a device's interrupts land on a chosen IDT vector via the
//! LAPIC. See `.claude/skills/pci-msi-programming/SKILL.md`.
//!
//! Conservative scope: single vector, fixed delivery, edge-triggered, physical
//! destination mode targeting the BSP. No multi-MSI, no logical destination.

use super::capability::walk_capabilities_rust;
use super::config::{
    offset, pci_cfg_read16, pci_cfg_read32, pci_cfg_write16, pci_cfg_write32, PciAddr,
};

/// Capability ID for MSI.
pub const CAP_ID_MSI: u8 = 0x05;
/// Capability ID for MSI-X.
pub const CAP_ID_MSIX: u8 = 0x11;

/// PCI command register bit 10: when set, the device must not assert legacy
/// INTx. We always set this when enabling MSI/MSI-X.
const CMD_INTX_DISABLE: u16 = 1 << 10;

/// Compose the LAPIC-targeting MSI message address for a given APIC ID.
///
/// Bits: `0xFEE0_0000 | (apic_id << 12) | (RH=0 << 3) | (DM=0 << 2)`.
/// Physical destination mode, no redirection hint.
#[inline]
pub fn lapic_msi_addr(apic_id: u32) -> u32 {
    0xFEE0_0000 | ((apic_id & 0xFF) << 12)
}

/// Compose the MSI data word for `vector`: fixed delivery mode, edge triggered.
#[inline]
pub fn msi_data(vector: u8) -> u32 {
    vector as u32
}

/// Disable legacy INTx on the device. Required when enabling MSI/MSI-X so the
/// device does not raise both signaling paths.
#[inline]
pub fn disable_intx(addr: PciAddr) {
    let cmd = pci_cfg_read16(addr, offset::COMMAND);
    pci_cfg_write16(addr, offset::COMMAND, cmd | CMD_INTX_DISABLE);
}

// ── MSI ──────────────────────────────────────────────────────────────────

/// MSI capability layout, located in PCI config space.
///
/// Single-vector use only.
#[derive(Debug, Clone, Copy)]
pub struct MsiCapability {
    pub addr: PciAddr,
    pub cap_off: u8,
    pub is_64bit: bool,
    pub per_vector_mask: bool,
}

impl MsiCapability {
    /// Message control register at `cap_off + 2`.
    fn msg_ctrl(self) -> u16 {
        pci_cfg_read16(self.addr, self.cap_off + 2)
    }

    fn set_msg_ctrl(self, v: u16) {
        pci_cfg_write16(self.addr, self.cap_off + 2, v)
    }

    fn data_off(self) -> u8 {
        if self.is_64bit {
            0x0C
        } else {
            0x08
        }
    }

    /// Program a single vector and enable MSI. Caller must call `disable_intx`
    /// separately (or use the higher-level `enable_msi_single`).
    pub fn program(self, msg_addr_low: u32, vector: u8) {
        // 1. Mask before reconfigure: clear MSI enable.
        let mc = self.msg_ctrl();
        self.set_msg_ctrl(mc & !1);

        // 2. Address low (must be DWORD-aligned).
        pci_cfg_write32(self.addr, self.cap_off + 4, msg_addr_low);
        if self.is_64bit {
            pci_cfg_write32(self.addr, self.cap_off + 8, 0);
        }

        // 3. Data (16-bit, low half of the LAPIC delivery word).
        pci_cfg_write16(
            self.addr,
            self.cap_off + self.data_off(),
            msi_data(vector) as u16,
        );

        // 4. Multi-message enable = 0 (single vector), then set enable.
        let mut mc = self.msg_ctrl();
        mc &= !(0b111 << 4); // MME = 0
        mc |= 1; // MSI enable
        self.set_msg_ctrl(mc);
    }

    pub fn disable(self) {
        let mc = self.msg_ctrl();
        self.set_msg_ctrl(mc & !1);
    }
}

/// Find an MSI capability on the device, if present.
pub fn find_msi(addr: PciAddr) -> Option<MsiCapability> {
    for (off, id) in walk_capabilities_rust(addr) {
        if id == CAP_ID_MSI {
            let mc = pci_cfg_read16(addr, off + 2);
            return Some(MsiCapability {
                addr,
                cap_off: off,
                is_64bit: (mc & (1 << 7)) != 0,
                per_vector_mask: (mc & (1 << 8)) != 0,
            });
        }
    }
    None
}

// ── MSI-X ────────────────────────────────────────────────────────────────

/// MSI-X table entry, 16 bytes, in BAR memory.
#[repr(C)]
struct MsixEntry {
    addr_lo: u32,
    addr_hi: u32,
    data: u32,
    vec_ctrl: u32,
}

const MSIX_VEC_CTRL_MASK: u32 = 1;

/// MSI-X capability layout.
#[derive(Debug, Clone, Copy)]
pub struct MsixCapability {
    pub addr: PciAddr,
    pub cap_off: u8,
    /// Number of entries (`MC.table_size + 1`).
    pub table_size: u16,
    /// Table BIR (0..=5) and DWORD-aligned offset within that BAR.
    pub table_bir: u8,
    pub table_offset: u32,
}

impl MsixCapability {
    fn msg_ctrl(self) -> u16 {
        pci_cfg_read16(self.addr, self.cap_off + 2)
    }

    fn set_msg_ctrl(self, v: u16) {
        pci_cfg_write16(self.addr, self.cap_off + 2, v)
    }

    /// Resolve the absolute virtual (= physical, identity-mapped MMIO)
    /// address of MSI-X table entry `idx`.
    ///
    /// # Safety
    /// BAR must be a valid memory BAR pointing at MMIO that is mapped UC
    /// (uncached). `idx` must be `< self.table_size`.
    unsafe fn entry_ptr(self, idx: u16) -> *mut MsixEntry {
        let bar = read_bar64(self.addr, self.table_bir);
        let base = bar.wrapping_add(self.table_offset as u64);
        (base + (idx as u64) * 16) as *mut MsixEntry
    }

    /// Mask or unmask the function (mask-all bit). Use this around table
    /// reconfiguration.
    pub fn set_function_mask(self, masked: bool) {
        let mut mc = self.msg_ctrl();
        if masked {
            mc |= 1 << 14;
        } else {
            mc &= !(1 << 14);
        }
        self.set_msg_ctrl(mc);
    }

    /// Set or clear the MSI-X enable bit.
    pub fn set_enable(self, enable: bool) {
        let mut mc = self.msg_ctrl();
        if enable {
            mc |= 1 << 15;
        } else {
            mc &= !(1 << 15);
        }
        self.set_msg_ctrl(mc);
    }

    /// Program one MSI-X table entry.
    ///
    /// # Safety
    /// See `entry_ptr`. Caller should hold `set_function_mask(true)` while
    /// programming, then `set_function_mask(false)` and `set_enable(true)`.
    pub unsafe fn program_entry(self, idx: u16, msg_addr_low: u32, vector: u8, masked: bool) {
        let e = self.entry_ptr(idx);
        core::ptr::write_volatile(&mut (*e).addr_lo, msg_addr_low);
        core::ptr::write_volatile(&mut (*e).addr_hi, 0);
        core::ptr::write_volatile(&mut (*e).data, msi_data(vector));
        core::ptr::write_volatile(
            &mut (*e).vec_ctrl,
            if masked { MSIX_VEC_CTRL_MASK } else { 0 },
        );
    }
}

/// Find an MSI-X capability on the device, if present.
pub fn find_msix(addr: PciAddr) -> Option<MsixCapability> {
    for (off, id) in walk_capabilities_rust(addr) {
        if id == CAP_ID_MSIX {
            let mc = pci_cfg_read16(addr, off + 2);
            let tbl = pci_cfg_read32(addr, off + 4);
            return Some(MsixCapability {
                addr,
                cap_off: off,
                table_size: (mc & 0x07FF) + 1,
                table_bir: (tbl & 0x7) as u8,
                table_offset: tbl & !0x7,
            });
        }
    }
    None
}

// ── High-level helpers ───────────────────────────────────────────────────

/// Errors that can come out of MSI/MSI-X enablement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsiError {
    NoCapability,
    BarNotMemory,
}

/// Enable MSI-X with a single entry directed at `vector`, prefer this over
/// MSI when both are present. Caller chooses the target APIC ID; for the
/// common single-CPU-target case use `unsafe { read_lapic_id() }` on the BSP.
///
/// # Safety
/// MSI-X table BAR must be a memory BAR mapped UC. PCI config writes must be
/// observed by the device before this returns; the underlying asm thunks
/// already serialize via port I/O.
pub unsafe fn enable_msix_single(
    addr: PciAddr,
    target_apic_id: u32,
    vector: u8,
) -> Result<MsixCapability, MsiError> {
    let cap = find_msix(addr).ok_or(MsiError::NoCapability)?;

    // 1. Mask everything before touching the table.
    cap.set_function_mask(true);
    cap.set_enable(false);

    // 2. Program entry 0.
    cap.program_entry(
        0,
        lapic_msi_addr(target_apic_id),
        vector,
        /*masked=*/ false,
    );

    // 3. Disable legacy INTx so the device does not double-signal.
    disable_intx(addr);

    // 4. Enable, unmask function.
    cap.set_enable(true);
    cap.set_function_mask(false);

    Ok(cap)
}

/// Fallback path: enable plain MSI with a single vector. Used when MSI-X is
/// not present.
///
/// # Safety
/// See `enable_msix_single`.
pub unsafe fn enable_msi_single(
    addr: PciAddr,
    target_apic_id: u32,
    vector: u8,
) -> Result<MsiCapability, MsiError> {
    let cap = find_msi(addr).ok_or(MsiError::NoCapability)?;
    cap.program(lapic_msi_addr(target_apic_id), vector);
    disable_intx(addr);
    Ok(cap)
}

// ── BAR resolution ───────────────────────────────────────────────────────

/// Read a 32- or 64-bit memory BAR. Returns 0 if the BAR is I/O space.
fn read_bar64(addr: PciAddr, bar_idx: u8) -> u64 {
    if bar_idx >= 6 {
        return 0;
    }
    let off = offset::BAR0 + bar_idx * 4;
    let lo = pci_cfg_read32(addr, off);
    // I/O BAR (bit 0 set) — not usable for MSI-X tables.
    if (lo & 0x1) != 0 {
        return 0;
    }
    let is_64 = ((lo >> 1) & 0x3) == 2;
    let base_lo = (lo & 0xFFFF_FFF0) as u64;
    if is_64 && bar_idx < 5 {
        let hi = pci_cfg_read32(addr, off + 4) as u64;
        base_lo | (hi << 32)
    } else {
        base_lo
    }
}
