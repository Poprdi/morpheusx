// Hardware-facing syscalls: port I/O, PCI cfg, DMA, virt-phys, IRQ, cache flush.

use super::common::*;
use super::mem::USER_MMAP_BASE;
use crate::hal;
use crate::schedular::SCHEDULER;
use morpheus_hal_api::{AllocKind, BusAddr, MemoryType, PageFlags, Pml4Handle};

/// `width`: 1/2/4 bytes.
///
/// LD25: x86-only. The HAL trait surface intentionally has no port-IO methods
/// so non-x86 arches can't get tempted to ship a stub. Returns ENOSYS.
pub unsafe fn sys_port_in(_port: u64, _width: u64) -> u64 {
    ENOSYS
}

pub unsafe fn sys_port_out(_port: u64, _width: u64, _value: u64) -> u64 {
    ENOSYS
}

/// bdf = (bus << 16) | (dev << 8) | func.
pub unsafe fn sys_pci_cfg_read(bdf: u64, offset: u64, width: u64) -> u64 {
    let bus = ((bdf >> 16) & 0xFF) as u8;
    let dev = ((bdf >> 8) & 0x1F) as u8;
    let func = (bdf & 0x07) as u8;
    if offset > 4095 {
        return EINVAL;
    }
    let addr = BusAddr::new(bus, dev, func);
    let off = offset as u16;
    match width {
        1 => hal().bus().cfg_read8(addr, off) as u64,
        2 => hal().bus().cfg_read16(addr, off) as u64,
        4 => hal().bus().cfg_read32(addr, off) as u64,
        _ => EINVAL,
    }
}

pub unsafe fn sys_pci_cfg_write(bdf: u64, offset: u64, width: u64, value: u64) -> u64 {
    let bus = ((bdf >> 16) & 0xFF) as u8;
    let dev = ((bdf >> 8) & 0x1F) as u8;
    let func = (bdf & 0x07) as u8;
    if offset > 4095 {
        return EINVAL;
    }
    let addr = BusAddr::new(bus, dev, func);
    let off = offset as u16;
    match width {
        1 => {
            hal().bus().cfg_write8(addr, off, value as u8);
            0
        }
        2 => {
            hal().bus().cfg_write16(addr, off, value as u16);
            0
        }
        4 => {
            hal().bus().cfg_write32(addr, off, value as u32);
            0
        }
        _ => EINVAL,
    }
}

/// Contiguous pages below 4 GiB.
pub unsafe fn sys_dma_alloc(pages: u64) -> u64 {
    if pages == 0 || pages > 512 {
        return EINVAL;
    }
    if !hal().phys().is_initialized() {
        return ENOMEM;
    }
    // Use AllocKind::MaxAddress(4GiB) — DMA needs sub-4GiB physical addresses.
    let max_addr: u64 = 1 << 32;
    match hal().phys().allocate_pages(
        AllocKind::MaxAddress(max_addr),
        MemoryType::AllocatedDma,
        pages,
    ) {
        Ok(addr) => {
            core::ptr::write_bytes(addr as *mut u8, 0, (pages * 4096) as usize);
            addr
        }
        Err(_) => ENOMEM,
    }
}

pub unsafe fn sys_dma_free(phys: u64, pages: u64) -> u64 {
    if phys == 0 || pages == 0 || pages > 512 {
        return EINVAL;
    }
    if !hal().phys().is_initialized() {
        return ENOMEM;
    }
    match hal().phys().free_pages(phys, pages) {
        Ok(()) => 0,
        Err(_) => EINVAL,
    }
}

/// flags: bit0 W, bit1 UC. Physical pages are caller-owned; MUNMAP only
/// drops PTEs, never frees the backing memory.
pub unsafe fn sys_map_phys(phys: u64, pages: u64, flags: u64) -> u64 {
    if phys == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if phys & 0xFFF != 0 {
        return EINVAL;
    }

    // Kernel (PID 0) is already identity-mapped — no work to do.
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    let proc = SCHEDULER.current_memory_leader_mut();
    if proc.mmap_brk == 0 {
        proc.mmap_brk = USER_MMAP_BASE;
    }
    let vaddr = proc.mmap_brk;

    // flags: bit0 = W, bit1 = UC. UC takes precedence — MMIO mapping is the
    // intended use; we never request write-without-UC for device BARs.
    let preset = if flags & 2 != 0 {
        PageFlags::USER_MMIO_UC
    } else if flags & 1 != 0 {
        PageFlags::USER_RW
    } else {
        PageFlags::USER_RO
    };

    let pml4 = Pml4Handle(proc.cr3);
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if hal()
            .paging()
            .pml4_map_user_4k(pml4, page_virt, page_phys, preset)
            .is_err()
        {
            return ENOMEM;
        }
    }

    if proc.vma_table.insert(vaddr, phys, pages, false).is_err() {
        for i in 0..pages {
            let _ = hal().paging().pml4_unmap_4k(proc.cr3, vaddr + i * 4096);
        }
        return ENOMEM;
    }

    proc.mmap_brk = vaddr + pages * 4096;
    vaddr
}

/// Rejects kernel addresses (info-leak prevention).
pub unsafe fn sys_virt_to_phys(virt: u64) -> u64 {
    if virt >= USER_ADDR_LIMIT {
        return EFAULT;
    }
    match hal().paging().kvirt_to_phys(virt) {
        Some(phys) => phys,
        None => EINVAL,
    }
}

/// Unmasks the IRQ on the PIC. Caller polls or shares the line.
pub unsafe fn sys_irq_attach(irq_num: u64) -> u64 {
    if irq_num > 15 {
        return EINVAL;
    }
    hal().intr().enable_irq(irq_num as u8);
    0
}

pub unsafe fn sys_irq_ack(irq_num: u64) -> u64 {
    if irq_num > 15 {
        return EINVAL;
    }
    hal().intr().send_pic_eoi(irq_num as u8);
    0
}

/// CPU→device DMA coherence. Caps at 64 MiB to bound stall time.
///
/// On x86_64 the architecture is cache-coherent for WB normal memory, so
/// this is a no-op — userspace DMA setup should use `SYS_DMA_ALLOC`, which
/// returns pages out of the identity-mapped DMA arena. The HAL exposes
/// per-DMA-region `sync_for_device`/`sync_for_cpu` for fine-grained CMO on
/// ARM. Userspace `cache_flush(addr, len)` semantics don't map onto the
/// HAL's region-keyed sync, so we accept the call but only validate the
/// arguments.
pub unsafe fn sys_cache_flush(addr: u64, len: u64) -> u64 {
    if addr == 0 || len == 0 {
        return EINVAL;
    }
    if len > 64 * 1024 * 1024 {
        return EINVAL;
    }
    0
}
