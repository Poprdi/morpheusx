//! Hardware primitives: ports, PCI, DMA, IRQs, cache, framebuffer.
//! The exokernel escape hatch — drivers from Ring 3.

use crate::raw::*;

// Port I/O.

pub fn port_inb(port: u16) -> u8 {
    unsafe { syscall2(SYS_PORT_IN, port as u64, 1) as u8 }
}

pub fn port_inw(port: u16) -> u16 {
    unsafe { syscall2(SYS_PORT_IN, port as u64, 2) as u16 }
}

pub fn port_inl(port: u16) -> u32 {
    unsafe { syscall2(SYS_PORT_IN, port as u64, 4) as u32 }
}

pub fn port_outb(port: u16, value: u8) {
    unsafe {
        syscall3(SYS_PORT_OUT, port as u64, 1, value as u64);
    }
}

pub fn port_outw(port: u16, value: u16) {
    unsafe {
        syscall3(SYS_PORT_OUT, port as u64, 2, value as u64);
    }
}

pub fn port_outl(port: u16, value: u32) {
    unsafe {
        syscall3(SYS_PORT_OUT, port as u64, 4, value as u64);
    }
}

// PCI config space.

#[inline]
pub fn pci_bdf(bus: u8, device: u8, function: u8) -> u64 {
    ((bus as u64) << 16) | ((device as u64) << 8) | (function as u64)
}

pub fn pci_cfg_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let bdf = pci_bdf(bus, device, function);
    unsafe { syscall3(SYS_PCI_CFG_READ, bdf, offset as u64, 1) as u8 }
}

pub fn pci_cfg_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let bdf = pci_bdf(bus, device, function);
    unsafe { syscall3(SYS_PCI_CFG_READ, bdf, offset as u64, 2) as u16 }
}

pub fn pci_cfg_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let bdf = pci_bdf(bus, device, function);
    unsafe { syscall3(SYS_PCI_CFG_READ, bdf, offset as u64, 4) as u32 }
}

pub fn pci_cfg_write8(bus: u8, device: u8, function: u8, offset: u8, value: u8) {
    let bdf = pci_bdf(bus, device, function);
    unsafe {
        syscall4(SYS_PCI_CFG_WRITE, bdf, offset as u64, 1, value as u64);
    }
}

pub fn pci_cfg_write16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    let bdf = pci_bdf(bus, device, function);
    unsafe {
        syscall4(SYS_PCI_CFG_WRITE, bdf, offset as u64, 2, value as u64);
    }
}

pub fn pci_cfg_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let bdf = pci_bdf(bus, device, function);
    unsafe {
        syscall4(SYS_PCI_CFG_WRITE, bdf, offset as u64, 4, value as u64);
    }
}

/// Allocate DMA-safe pages below 4 GiB; zeroed and physically contiguous.
pub fn dma_alloc(pages: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_DMA_ALLOC, pages) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

pub fn dma_free(phys: u64, pages: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_DMA_FREE, phys, pages) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Map physical pages. Flags: bit 0 = writable, bit 1 = uncacheable. Returns VA.
pub fn map_phys(phys: u64, pages: u64, flags: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall3(SYS_MAP_PHYS, phys, pages, flags) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

pub fn map_phys_rw(phys: u64, pages: u64) -> Result<u64, u64> {
    map_phys(phys, pages, 1)
}

/// Writable + uncacheable; for MMIO.
pub fn map_mmio(phys: u64, pages: u64) -> Result<u64, u64> {
    map_phys(phys, pages, 3)
}

pub fn virt_to_phys(virt: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_VIRT_TO_PHYS, virt) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Enable a PIC line (8259A: IRQ 0-15).
pub fn irq_attach(irq: u8) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_IRQ_ATTACH, irq as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// EOI to the PIC.
pub fn irq_ack(irq: u8) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_IRQ_ACK, irq as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// CLFLUSH the range; required before DMA reads to avoid stale lines.
pub fn cache_flush(addr: u64, len: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_CACHE_FLUSH, addr, len) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// Boundary structs are canonical in morpheus-foundation — single source.
pub use morpheus_foundation::types::{CpuidResult, FbInfo, MemmapEntry, TscResult};

pub fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    let mut result = CpuidResult {
        eax: 0,
        ebx: 0,
        ecx: 0,
        edx: 0,
    };
    unsafe {
        syscall3(
            SYS_CPUID,
            leaf as u64,
            subleaf as u64,
            &mut result as *mut CpuidResult as u64,
        );
    }
    result
}

/// TSC value plus calibrated frequency.
pub fn rdtsc() -> TscResult {
    let mut result = TscResult {
        tsc: 0,
        frequency: 0,
    };
    let tsc = unsafe { syscall1(SYS_RDTSC, &mut result as *mut TscResult as u64) };
    result.tsc = tsc;
    result
}

/// TSC only; skips the frequency lookup.
pub fn rdtsc_raw() -> u64 {
    unsafe { syscall1(SYS_RDTSC, 0) }
}

pub fn fb_info() -> Result<FbInfo, u64> {
    let mut info = FbInfo {
        base: 0,
        size: 0,
        width: 0,
        height: 0,
        stride: 0,
        format: 0,
    };
    let ret = unsafe { syscall1(SYS_FB_INFO, &mut info as *mut FbInfo as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(info)
    }
}

/// Map the back buffer. Kernel double-buffers; delta-presents on each tick.
pub fn fb_map() -> Result<u64, u64> {
    let ret = unsafe { syscall0(SYS_FB_MAP) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Acquire exclusive FB access; cooperative. Err(EBUSY) if held elsewhere.
pub fn fb_lock() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_FB_LOCK) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Only the holder may unlock; Err(EPERM) otherwise.
pub fn fb_unlock() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_FB_UNLOCK) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// 0 if unlocked, else the holder's PID.
pub fn fb_is_locked() -> u32 {
    unsafe { syscall0(SYS_FB_IS_LOCKED) as u32 }
}

/// Run the delta presenter now instead of waiting for the 100 Hz tick.
pub fn fb_present() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_FB_PRESENT) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// One-shot memcpy of the back buffer to VRAM. Beats `fb_present` when
/// most pixels change every frame (skips the per-pixel diff).
pub fn fb_blit() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_FB_BLIT) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Set the dirty flag so the next timer tick presents. Cheaper than a full scan when idle.
#[inline]
pub fn fb_mark_dirty() {
    unsafe {
        syscall0(SYS_FB_MARK_DIRTY);
    }
}

#[derive(Clone, Copy, Default)]
pub struct MouseState {
    pub dx: i16,
    pub dy: i16,
    /// Bit 0 = left, bit 1 = right, bit 2 = middle.
    pub buttons: u8,
    pub wheel: i8,
}

/// Read and reset the per-process motion accumulator.
pub fn mouse_read() -> MouseState {
    let raw = unsafe { syscall0(SYS_MOUSE_READ) };
    MouseState {
        dx: raw as i16,
        dy: (raw >> 16) as i16,
        buttons: (raw >> 32) as u8,
        wheel: (raw >> 48) as i8,
    }
}

pub fn boot_log_size() -> u64 {
    unsafe { syscall2(SYS_BOOT_LOG, 0, 0) }
}

pub fn boot_log(buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe { syscall2(SYS_BOOT_LOG, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

// MemmapEntry: canonical in morpheus_foundation::types (re-exported above).

pub fn memmap_count() -> u64 {
    unsafe { syscall2(SYS_MEMMAP, 0, 0) }
}

pub fn memmap(entries: &mut [MemmapEntry]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall2(
            SYS_MEMMAP,
            entries.as_mut_ptr() as u64,
            entries.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}
