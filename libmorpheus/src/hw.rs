//! Hardware primitives — exokernel direct hardware access.
//!
//! These syscalls give userland programs direct access to x86 I/O ports,
//! PCI configuration space, DMA memory, physical address mapping, IRQ
//! management, and CPU cache control.
//!
//! With these primitives, userland can build its own device drivers for
//! NICs, storage controllers, GPUs, and any other PCI/PCIe device.

use crate::raw::*;

// ═══════════════════════════════════════════════════════════════════════════
// PORT I/O
// ═══════════════════════════════════════════════════════════════════════════

/// Read a byte from an x86 I/O port.
pub fn port_inb(port: u16) -> u8 {
    unsafe { syscall2(SYS_PORT_IN, port as u64, 1) as u8 }
}

/// Read a word (16-bit) from an x86 I/O port.
pub fn port_inw(port: u16) -> u16 {
    unsafe { syscall2(SYS_PORT_IN, port as u64, 2) as u16 }
}

/// Read a dword (32-bit) from an x86 I/O port.
pub fn port_inl(port: u16) -> u32 {
    unsafe { syscall2(SYS_PORT_IN, port as u64, 4) as u32 }
}

/// Write a byte to an x86 I/O port.
pub fn port_outb(port: u16, value: u8) {
    unsafe {
        syscall3(SYS_PORT_OUT, port as u64, 1, value as u64);
    }
}

/// Write a word (16-bit) to an x86 I/O port.
pub fn port_outw(port: u16, value: u16) {
    unsafe {
        syscall3(SYS_PORT_OUT, port as u64, 2, value as u64);
    }
}

/// Write a dword (32-bit) to an x86 I/O port.
pub fn port_outl(port: u16, value: u32) {
    unsafe {
        syscall3(SYS_PORT_OUT, port as u64, 4, value as u64);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PCI CONFIGURATION SPACE
// ═══════════════════════════════════════════════════════════════════════════

/// Encode bus/device/function into the BDF format used by PCI syscalls.
#[inline]
pub fn pci_bdf(bus: u8, device: u8, function: u8) -> u64 {
    ((bus as u64) << 16) | ((device as u64) << 8) | (function as u64)
}

/// Read a byte from PCI configuration space.
pub fn pci_cfg_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let bdf = pci_bdf(bus, device, function);
    unsafe { syscall3(SYS_PCI_CFG_READ, bdf, offset as u64, 1) as u8 }
}

/// Read a word (16-bit) from PCI configuration space.
pub fn pci_cfg_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let bdf = pci_bdf(bus, device, function);
    unsafe { syscall3(SYS_PCI_CFG_READ, bdf, offset as u64, 2) as u16 }
}

/// Read a dword (32-bit) from PCI configuration space.
pub fn pci_cfg_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let bdf = pci_bdf(bus, device, function);
    unsafe { syscall3(SYS_PCI_CFG_READ, bdf, offset as u64, 4) as u32 }
}

/// Write a byte to PCI configuration space.
pub fn pci_cfg_write8(bus: u8, device: u8, function: u8, offset: u8, value: u8) {
    let bdf = pci_bdf(bus, device, function);
    unsafe {
        syscall4(SYS_PCI_CFG_WRITE, bdf, offset as u64, 1, value as u64);
    }
}

/// Write a word (16-bit) to PCI configuration space.
pub fn pci_cfg_write16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    let bdf = pci_bdf(bus, device, function);
    unsafe {
        syscall4(SYS_PCI_CFG_WRITE, bdf, offset as u64, 2, value as u64);
    }
}

/// Write a dword (32-bit) to PCI configuration space.
pub fn pci_cfg_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let bdf = pci_bdf(bus, device, function);
    unsafe {
        syscall4(SYS_PCI_CFG_WRITE, bdf, offset as u64, 4, value as u64);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DMA MEMORY
// ═══════════════════════════════════════════════════════════════════════════

/// Allocate DMA-safe physical memory below 4GB.
///
/// Returns the physical address of the allocation.
/// Memory is zeroed and physically contiguous.
pub fn dma_alloc(pages: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_DMA_ALLOC, pages) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Free DMA memory previously allocated with `dma_alloc`.
pub fn dma_free(phys: u64, pages: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_DMA_FREE, phys, pages) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PHYSICAL MEMORY MAPPING
// ═══════════════════════════════════════════════════════════════════════════

/// Map physical address into the process virtual address space.
///
/// Flags: bit 0 = writable, bit 1 = uncacheable.
/// Returns the virtual address.
pub fn map_phys(phys: u64, pages: u64, flags: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall3(SYS_MAP_PHYS, phys, pages, flags) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Map physical address as writable.
pub fn map_phys_rw(phys: u64, pages: u64) -> Result<u64, u64> {
    map_phys(phys, pages, 1)
}

/// Map physical address as writable + uncacheable (for MMIO).
pub fn map_mmio(phys: u64, pages: u64) -> Result<u64, u64> {
    map_phys(phys, pages, 3) // writable | uncacheable
}

/// Translate a virtual address to its physical address.
pub fn virt_to_phys(virt: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_VIRT_TO_PHYS, virt) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// IRQ MANAGEMENT
// ═══════════════════════════════════════════════════════════════════════════

/// Enable an IRQ line on the PIC.
///
/// IRQ numbers 0-15 are valid (8259A PIC).
pub fn irq_attach(irq: u8) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_IRQ_ATTACH, irq as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Send End-Of-Interrupt for the specified IRQ.
pub fn irq_ack(irq: u8) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_IRQ_ACK, irq as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CPU CACHE
// ═══════════════════════════════════════════════════════════════════════════

/// Flush CPU cache lines covering the address range `[addr, addr+len)`.
///
/// Essential for DMA coherence when the CPU writes data that a device
/// will read via DMA.
pub fn cache_flush(addr: u64, len: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_CACHE_FLUSH, addr, len) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CPUID
// ═══════════════════════════════════════════════════════════════════════════

/// CPUID result registers.
#[repr(C)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// Execute the CPUID instruction.
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

// ═══════════════════════════════════════════════════════════════════════════
// RDTSC
// ═══════════════════════════════════════════════════════════════════════════

/// TSC result (value + calibrated frequency).
#[repr(C)]
pub struct TscResult {
    pub tsc: u64,
    pub frequency: u64,
}

/// Read the Time Stamp Counter with calibrated frequency.
pub fn rdtsc() -> TscResult {
    let mut result = TscResult {
        tsc: 0,
        frequency: 0,
    };
    let tsc = unsafe { syscall1(SYS_RDTSC, &mut result as *mut TscResult as u64) };
    result.tsc = tsc;
    result
}

/// Read the raw TSC value (faster, no frequency info).
pub fn rdtsc_raw() -> u64 {
    unsafe { syscall1(SYS_RDTSC, 0) }
}

// ═══════════════════════════════════════════════════════════════════════════
// FRAMEBUFFER
// ═══════════════════════════════════════════════════════════════════════════

/// Framebuffer information.
#[repr(C)]
pub struct FbInfo {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 0 = RGBX, 1 = BGRX
    pub format: u32,
}

/// Get framebuffer information.
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

/// Map the framebuffer into the process's virtual address space.
///
/// Returns the virtual address of the mapped framebuffer.
/// The mapping is writable and uncacheable.
pub fn fb_map() -> Result<u64, u64> {
    let ret = unsafe { syscall0(SYS_FB_MAP) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BOOT LOG
// ═══════════════════════════════════════════════════════════════════════════

/// Get the total size of the kernel boot log.
pub fn boot_log_size() -> u64 {
    unsafe { syscall2(SYS_BOOT_LOG, 0, 0) }
}

/// Read the kernel boot log into `buf`.
///
/// Returns the number of bytes written.
pub fn boot_log(buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe { syscall2(SYS_BOOT_LOG, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY MAP
// ═══════════════════════════════════════════════════════════════════════════

/// Physical memory map entry.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemmapEntry {
    pub phys_start: u64,
    pub num_pages: u64,
    /// Memory type (matches UEFI EFI_MEMORY_TYPE).
    pub mem_type: u32,
    pub _pad: u32,
}

/// Get the total number of memory map entries.
pub fn memmap_count() -> u64 {
    unsafe { syscall2(SYS_MEMMAP, 0, 0) }
}

/// Read the physical memory map into `entries`.
///
/// Returns the number of entries written.
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
