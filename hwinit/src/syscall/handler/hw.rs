
/// `width`: 1/2/4 bytes.
pub unsafe fn sys_port_in(port: u64, width: u64) -> u64 {
    if port > 0xFFFF {
        return EINVAL;
    }
    let port = port as u16;
    match width {
        1 => crate::cpu::pio::inb(port) as u64,
        2 => crate::cpu::pio::inw(port) as u64,
        4 => crate::cpu::pio::inl(port) as u64,
        _ => EINVAL,
    }
}

pub unsafe fn sys_port_out(port: u64, width: u64, value: u64) -> u64 {
    if port > 0xFFFF {
        return EINVAL;
    }
    let port = port as u16;
    match width {
        1 => {
            crate::cpu::pio::outb(port, value as u8);
            0
        }
        2 => {
            crate::cpu::pio::outw(port, value as u16);
            0
        }
        4 => {
            crate::cpu::pio::outl(port, value as u32);
            0
        }
        _ => EINVAL,
    }
}

/// bdf = (bus << 16) | (dev << 8) | func.
pub unsafe fn sys_pci_cfg_read(bdf: u64, offset: u64, width: u64) -> u64 {
    let bus = ((bdf >> 16) & 0xFF) as u8;
    let dev = ((bdf >> 8) & 0x1F) as u8;
    let func = (bdf & 0x07) as u8;
    if offset > 255 {
        return EINVAL;
    }
    let addr = crate::pci::PciAddr {
        bus,
        device: dev,
        function: func,
    };
    let off = offset as u8;
    match width {
        1 => crate::pci::pci_cfg_read8(addr, off) as u64,
        2 => crate::pci::pci_cfg_read16(addr, off) as u64,
        4 => crate::pci::pci_cfg_read32(addr, off) as u64,
        _ => EINVAL,
    }
}

pub unsafe fn sys_pci_cfg_write(bdf: u64, offset: u64, width: u64, value: u64) -> u64 {
    let bus = ((bdf >> 16) & 0xFF) as u8;
    let dev = ((bdf >> 8) & 0x1F) as u8;
    let func = (bdf & 0x07) as u8;
    if offset > 255 {
        return EINVAL;
    }
    let addr = crate::pci::PciAddr {
        bus,
        device: dev,
        function: func,
    };
    let off = offset as u8;
    match width {
        1 => {
            crate::pci::pci_cfg_write8(addr, off, value as u8);
            0
        }
        2 => {
            crate::pci::pci_cfg_write16(addr, off, value as u16);
            0
        }
        4 => {
            crate::pci::pci_cfg_write32(addr, off, value as u32);
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
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }
    let mut registry = crate::memory::global_registry_mut();
    match registry.alloc_dma_pages(pages) {
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
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }
    let mut registry = crate::memory::global_registry_mut();
    match registry.free_pages(phys, pages) {
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
        proc.mmap_brk = 0x0000_0040_0000_0000;
    }
    let vaddr = proc.mmap_brk;

    let writable = flags & 1 != 0;
    let uncacheable = flags & 2 != 0;

    let mut pte_flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);
    if writable {
        pte_flags = pte_flags.with(crate::paging::entry::PageFlags::WRITABLE);
    }
    if uncacheable {
        pte_flags = pte_flags.with(crate::paging::entry::PageFlags::CACHE_DISABLE);
    }

    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: proc.cr3,
    };

    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if crate::elf::map_user_page(&mut ptm, page_virt, page_phys, pte_flags).is_err() {
            return ENOMEM;
        }
    }

    if proc.vma_table.insert(vaddr, phys, pages, false).is_err() {
        // Roll back the mapping; table is full.
        let mut ptm2 = crate::paging::table::PageTableManager {
            pml4_phys: proc.cr3,
        };
        for i in 0..pages {
            let _ = ptm2.unmap_4k(vaddr + i * 4096);
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
    match crate::paging::kvirt_to_phys(virt) {
        Some(phys) => phys,
        None => EINVAL,
    }
}

/// Unmasks the IRQ on the PIC. Caller polls or shares the line.
pub unsafe fn sys_irq_attach(irq_num: u64) -> u64 {
    if irq_num > 15 {
        return EINVAL;
    }
    crate::cpu::pic::enable_irq(irq_num as u8);
    0
}

pub unsafe fn sys_irq_ack(irq_num: u64) -> u64 {
    if irq_num > 15 {
        return EINVAL;
    }
    crate::cpu::pic::send_eoi(irq_num as u8);
    0
}

/// CPU→device DMA coherence. Caps at 64 MiB to bound stall time.
pub unsafe fn sys_cache_flush(addr: u64, len: u64) -> u64 {
    if addr == 0 || len == 0 {
        return EINVAL;
    }
    if len > 64 * 1024 * 1024 {
        return EINVAL;
    }
    crate::cpu::cache::flush_range(addr as *const u8, len as usize);
    0
}
