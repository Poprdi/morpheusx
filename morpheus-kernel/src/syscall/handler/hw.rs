// Hardware-facing syscalls: port I/O, PCI cfg, DMA, virt-phys, IRQ, cache flush.

use super::common::*;
use super::mem::USER_MMAP_BASE;
use crate::hal;
use crate::schedular::SCHEDULER;
use morpheus_hal_api::{AllocKind, BusAddr, MemoryType, PageFlags, Pml4Handle};

// Port-IO seam (LD25): port I/O is x86-only and deliberately kept out of the
// HAL trait so a non-x86 arch can't ship a broken stub. Instead the x86 HAL
// installs these fn-pointer hooks; arches that don't install them leave port
// I/O at ENOSYS. Exokernel: ports are exposed raw to userland — a caller with
// the syscall can touch any port, by design.
use core::sync::atomic::{AtomicPtr, Ordering};

type PortInFn = fn(u16, u8) -> u32;
type PortOutFn = fn(u16, u8, u32);

static PORT_IN_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static PORT_OUT_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Install the platform port-IO primitives. Call once at boot (x86 only).
pub fn set_port_io_hooks(port_in: PortInFn, port_out: PortOutFn) {
    PORT_IN_HOOK.store(port_in as *mut (), Ordering::Release);
    PORT_OUT_HOOK.store(port_out as *mut (), Ordering::Release);
}

/// `width`: 1/2/4 bytes. Returns the read value, or ENOSYS if no platform hook.
pub unsafe fn sys_port_in(port: u64, width: u64) -> u64 {
    if port > 0xFFFF || !matches!(width, 1 | 2 | 4) {
        return EINVAL;
    }
    let p = PORT_IN_HOOK.load(Ordering::Acquire);
    if p.is_null() {
        return ENOSYS;
    }
    // SAFETY: pointer was installed from a `PortInFn` by `set_port_io_hooks`.
    let f: PortInFn = core::mem::transmute(p);
    f(port as u16, width as u8) as u64
}

pub unsafe fn sys_port_out(port: u64, width: u64, value: u64) -> u64 {
    if port > 0xFFFF || !matches!(width, 1 | 2 | 4) {
        return EINVAL;
    }
    let p = PORT_OUT_HOOK.load(Ordering::Acquire);
    if p.is_null() {
        return ENOSYS;
    }
    // SAFETY: pointer was installed from a `PortOutFn` by `set_port_io_hooks`.
    let f: PortOutFn = core::mem::transmute(p);
    f(port as u16, width as u8, value as u32);
    0
}

/// getrandom(buf, len, flags) -> bytes written. Fills `[buf, buf+len)` from the
/// platform RNG (x86 RDRAND). `flags` bit0 = GRND_NONBLOCK (advisory; RDRAND
/// doesn't truly block). ENOSYS if the platform has no RNG; EFAULT on a bad buf.
pub unsafe fn sys_getrandom(buf: u64, len: u64, _flags: u64) -> u64 {
    if len == 0 {
        return 0;
    }
    if !validate_user_buf(buf, len) {
        return EFAULT;
    }
    let total = len as usize;
    let dst = buf as *mut u8;
    let mut written = 0usize;
    while written < total {
        let word = match hal().cpu().hw_random() {
            Some(w) => w,
            // No RNG → ENOSYS; transient starvation after a partial fill → return
            // what we have (a short read, like Linux getrandom under signal).
            None if written == 0 => return ENOSYS,
            None => break,
        };
        let bytes = word.to_ne_bytes();
        let take = core::cmp::min(8, total - written);
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.add(written), take);
        written += take;
    }
    written as u64
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
        },
        2 => {
            hal().bus().cfg_write16(addr, off, value as u16);
            0
        },
        4 => {
            hal().bus().cfg_write32(addr, off, value as u32);
            0
        },
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
        },
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
    // 16384 pages = 64 MiB per call. The old 4 MiB (1024-page) cap rejected
    // whole-framebuffer maps on real panels (1080p = 2025 pages, 4K = 8100) —
    // QEMU's default FB stays under 4 MiB, so it only bit on hardware.
    if phys == 0 || pages == 0 || pages > 16384 {
        return EINVAL;
    }
    if phys & 0xFFF != 0 {
        return EINVAL;
    }

    // Kernel (PID 0) is already identity-mapped — no work to do.
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    // Same per-address-space serialization as mmap (this also backs the fb
    // mappers, which delegate their address-space mutation here).
    let lock = SCHEDULER.current_address_space_lock();
    lock.lock();
    let ret = map_phys_locked(phys, pages, flags);
    lock.unlock();
    ret
}

unsafe fn map_phys_locked(phys: u64, pages: u64, flags: u64) -> u64 {
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

/// No-op on x86_64 (cache-coherent WB memory); only validates args. Caps at
/// 64 MiB. ARM CMO routes through the HAL's region-keyed sync_for_device/cpu,
/// which this addr/len-keyed call can't express.
pub unsafe fn sys_cache_flush(addr: u64, len: u64) -> u64 {
    if addr == 0 || len == 0 {
        return EINVAL;
    }
    if len > 64 * 1024 * 1024 {
        return EINVAL;
    }
    0
}
