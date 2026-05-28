//! Concrete `HalImpl` satisfying `morpheus_hal_api::Hal`. Trait bodies delegate
//! to the local subsystems. `UsbHost` still reaches into `hwinit::usb` —
//! pulling USB in is blocked by a dep cycle with morpheus-xhci.

use morpheus_hal_api::{
    AllocKind, BusAddr, BusEnumerator, Compositor, Cpu, CpuContext, DmaAllocator, DmaRegion,
    E820Entry, FpuState, Hal, InterruptController, IsrFn, MemError, MemoryDescriptor, MemoryType,
    Mmio, MsiError, PageError, PageFlags, PageTable, PhysAlloc, Pml4Handle, Reset, Serial, Smp,
    SmpError, Timer, UsbHost,
};

pub struct HalImpl {}

impl HalImpl {
    pub fn new() -> Self {
        Self {}
    }
}

impl Hal for HalImpl {
    fn mmio(&self) -> &dyn Mmio {
        self
    }
    fn cpu(&self) -> &dyn Cpu {
        self
    }
    fn serial(&self) -> &dyn Serial {
        self
    }
    fn phys(&self) -> &dyn PhysAlloc {
        self
    }
    fn paging(&self) -> &dyn PageTable {
        self
    }
    fn intr(&self) -> &dyn InterruptController {
        self
    }
    fn timer(&self) -> &dyn Timer {
        self
    }
    fn dma(&self) -> &dyn DmaAllocator {
        self
    }
    fn bus(&self) -> &dyn BusEnumerator {
        self
    }
    fn usb(&self) -> &dyn UsbHost {
        self
    }
    fn reset(&self) -> &dyn Reset {
        self
    }
    fn smp(&self) -> &dyn Smp {
        self
    }
    fn compositor(&self) -> &dyn Compositor {
        self
    }
}

impl Mmio for HalImpl {
    unsafe fn read8(&self, addr: u64) -> u8 {
        crate::asm::mmio::read8(addr)
    }
    unsafe fn read16(&self, addr: u64) -> u16 {
        crate::asm::mmio::read16(addr)
    }
    unsafe fn read32(&self, addr: u64) -> u32 {
        crate::asm::mmio::read32(addr)
    }
    unsafe fn read64(&self, addr: u64) -> u64 {
        // No native 64-bit MMIO thunk.
        let lo = crate::asm::mmio::read32(addr) as u64;
        let hi = crate::asm::mmio::read32(addr + 4) as u64;
        (hi << 32) | lo
    }
    unsafe fn write8(&self, addr: u64, val: u8) {
        crate::asm::mmio::write8(addr, val);
    }
    unsafe fn write16(&self, addr: u64, val: u16) {
        crate::asm::mmio::write16(addr, val);
    }
    unsafe fn write32(&self, addr: u64, val: u32) {
        crate::asm::mmio::write32(addr, val);
    }
    unsafe fn write64(&self, addr: u64, val: u64) {
        // No native 64-bit MMIO thunk.
        crate::asm::mmio::write32(addr, val as u32);
        crate::asm::mmio::write32(addr + 4, (val >> 32) as u32);
    }
    fn mfence(&self) {
        crate::asm::barriers::mfence();
    }
    fn sfence(&self) {
        crate::asm::barriers::sfence();
    }
    fn lfence(&self) {
        crate::asm::barriers::lfence();
    }
}

impl Cpu for HalImpl {
    fn disable_interrupts(&self) {
        crate::intr::disable_interrupts();
    }
    fn enable_interrupts(&self) {
        crate::intr::enable_interrupts();
    }
    fn halt(&self) {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
    fn interrupts_enabled(&self) -> bool {
        crate::intr::interrupts_enabled()
    }

    fn halt_wait_irq(&self) {
        // SAFETY: STI interrupt-shadow delays enable past the HLT, so an IRQ
        // posted between STI and HLT still wakes us. CLI restores the kernel
        // idle loop's IF=0 invariant.
        unsafe {
            core::arch::asm!("sti", "hlt", "cli", options(nomem, nostack));
        }
    }

    fn halt_no_irq(&self) {
        // SAFETY: caller asserts IF=0. Wakeup arrives via INIT/SIPI/NMI (ignore IF).
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }

    fn crash_now(&self) -> ! {
        // SAFETY: UD2 → #UD unconditionally; IDT crash hook renders BSoD.
        unsafe {
            core::arch::asm!("ud2", options(noreturn));
        }
    }

    fn cpuid(&self, leaf: u32, subleaf: u32) -> [u32; 4] {
        let mut eax = leaf;
        let ebx: u32;
        let mut ecx = subleaf;
        let edx: u32;
        // SAFETY: pure ID instruction. push/pop rbx because LLVM reserves it in
        // some PIC modes; this preserves the caller's value.
        unsafe {
            core::arch::asm!(
                "push rbx",
                "cpuid",
                "mov {ebx_out:e}, ebx",
                "pop rbx",
                inout("eax") eax,
                inout("ecx") ecx,
                out("edx") edx,
                ebx_out = out(reg) ebx,
                options(nostack),
            );
        }
        [eax, ebx, ecx, edx]
    }

    unsafe fn install_syscall_msrs(&self) {
        // STAR/LSTAR/FMASK live in asm/cpu/syscall.s; same `syscall_init` AP boot uses.
        extern "C" {
            fn syscall_init();
        }
        syscall_init();
    }

    fn ctx_init_kernel(&self, ctx: &mut CpuContext, entry: u64, stack_top: u64) {
        crate::cpu::context::ctx_init_kernel(ctx, entry, stack_top);
    }

    fn ctx_init_user(
        &self,
        ctx: &mut CpuContext,
        entry_va: u64,
        user_stack_top: u64,
        args: &[u64; 6],
    ) {
        crate::cpu::context::ctx_init_user(ctx, entry_va, user_stack_top, args);
    }

    fn ctx_set_ip(&self, ctx: &mut CpuContext, ip: u64) {
        crate::cpu::context::ctx_set_ip(ctx, ip);
    }

    fn ctx_set_sp(&self, ctx: &mut CpuContext, sp: u64) {
        crate::cpu::context::ctx_set_sp(ctx, sp);
    }

    fn ctx_set_arg(&self, ctx: &mut CpuContext, n: u8, val: u64) {
        crate::cpu::context::ctx_set_arg(ctx, n, val);
    }

    fn ctx_set_return(&self, ctx: &mut CpuContext, val: u64) {
        crate::cpu::context::ctx_set_return(ctx, val);
    }

    fn ctx_get_return(&self, ctx: &CpuContext) -> u64 {
        crate::cpu::context::ctx_get_return(ctx)
    }

    fn ctx_get_sp(&self, ctx: &CpuContext) -> u64 {
        crate::cpu::context::ctx_get_sp(ctx)
    }

    fn ctx_is_user_mode(&self, ctx: &CpuContext) -> bool {
        crate::cpu::context::ctx_is_user_mode(ctx)
    }

    fn ctx_set_user_mode(&self, ctx: &mut CpuContext, user: bool) {
        crate::cpu::context::ctx_set_user_mode(ctx, user);
    }

    fn fpu_init(&self, fpu: &mut FpuState) {
        crate::cpu::context::fpu_init(fpu);
    }

    fn set_reset_on_crash(&self, enable: bool) {
        crate::cpu::idt::set_reset_on_crash(enable);
    }
}

impl Serial for HalImpl {
    fn putc(&self, b: u8) {
        crate::serial::putc(b);
    }
    fn puts(&self, s: &str) {
        crate::serial::puts(s);
    }
    fn put_hex32(&self, v: u32) {
        crate::serial::put_hex32(v);
    }
    fn put_hex64(&self, v: u64) {
        crate::serial::put_hex64(v);
    }
    fn newline(&self) {
        crate::serial::newline();
    }
}

fn map_alloc_kind(k: AllocKind) -> crate::memory::AllocateType {
    match k {
        AllocKind::AnyPages => crate::memory::AllocateType::AnyPages,
        AllocKind::MaxAddress(a) => crate::memory::AllocateType::MaxAddress(a),
        AllocKind::Address(a) => crate::memory::AllocateType::Address(a),
    }
}

fn map_mem_type(mt: MemoryType) -> crate::memory::MemoryType {
    // SAFETY: both enums are `#[repr(u32)]` with identical discriminants by design.
    unsafe { core::mem::transmute(mt) }
}

fn unmap_mem_type(mt: crate::memory::MemoryType) -> MemoryType {
    // SAFETY: same discriminants as `map_mem_type`.
    unsafe { core::mem::transmute(mt) }
}

impl PhysAlloc for HalImpl {
    fn allocate_pages(&self, kind: AllocKind, mt: MemoryType, pages: u64) -> Result<u64, MemError> {
        unsafe {
            crate::memory::global_registry_mut()
                .allocate_pages(map_alloc_kind(kind), map_mem_type(mt), pages)
                .map_err(|_| MemError::OutOfMemory)
        }
    }
    fn free_pages(&self, addr: u64, pages: u64) -> Result<(), MemError> {
        unsafe {
            crate::memory::global_registry_mut()
                .free_pages(addr, pages)
                .map_err(|_| MemError::InvalidAddress)
        }
    }
    fn is_initialized(&self) -> bool {
        crate::memory::is_registry_initialized()
    }
    fn page_size(&self) -> u64 {
        crate::memory::PAGE_SIZE
    }
    fn total_memory(&self) -> u64 {
        unsafe { crate::memory::global_registry().total_memory() }
    }
    fn free_memory(&self) -> u64 {
        unsafe { crate::memory::global_registry().free_memory() }
    }
    fn allocated_memory(&self) -> u64 {
        unsafe { crate::memory::global_registry().allocated_memory() }
    }
    fn memory_type_at(&self, phys: u64) -> MemoryType {
        unsafe { unmap_mem_type(crate::memory::global_registry().memory_type_at(phys)) }
    }
    fn is_valid_cr3(&self, cr3: u64) -> bool {
        crate::memory::is_valid_cr3(cr3)
    }
    fn find_largest_free_below_4gb(&self) -> Option<(u64, u64)> {
        unsafe { crate::memory::global_registry().find_largest_free_below_4gb() }
    }
    fn for_each_descriptor(&self, _f: &mut dyn FnMut(&MemoryDescriptor)) {
        // TODO: bridge buddy iterator into hal-api shape; no caller today.
    }
    fn export_e820(&self, _out: &mut [E820Entry]) -> usize {
        // TODO: legacy bootloader E820 handoff.
        0
    }
    unsafe fn reclaim_boot_services(&self) -> Result<u64, ()> {
        // TODO: FIX! BootServices reclaim is no-op'd as a boot-unblock.
        //
        // On real hardware the post-EBS reclaim adds a region whose contents
        // appear as corrupt free-list nodes (kernel-half / poison `next`
        // pointers) and hangs the buddy validate walk. This is a regression vs
        // pre-refactor (commit 9276267): the buddy/reclaim/paging code is
        // byte-identical to the working version, so the offending data/layout
        // is introduced elsewhere in the refactor (something live left in a
        // BootServices-typed region). Root cause still OPEN — needs the corrupt
        // pointer values from a real-HW run to pin the source.
        //
        // Skipping reclaim only forfeits the firmware's BootServices RAM (~tens
        // of MB, <1% on a multi-GB box; conventional memory is already in the
        // buddy and plentiful), so boot proceeds to userland. Restore the
        // original body (see `git show 9276267^:hwinit/src/platform.rs` reclaim
        // sequence, mirrored here) once the corrupt-region source is found.
        Ok(0)
    }
}

fn page_err(_: &'static str) -> PageError {
    PageError::OutOfMemory
}

fn map_page_flags(flags: PageFlags) -> crate::paging::PageFlags {
    use crate::paging::PageFlags as Pf;
    match flags {
        PageFlags::KERNEL_RO => Pf::KERNEL_RO,
        PageFlags::KERNEL_RW => Pf::KERNEL_RW,
        PageFlags::KERNEL_CODE => Pf::KERNEL_CODE,
        PageFlags::USER_RO => Pf::USER_RO,
        PageFlags::USER_RW => Pf::USER_RW,
        // USER_RX and USER_CODE share the same discriminant.
        PageFlags::USER_CODE => Pf::USER_CODE,
        PageFlags::USER_RWX => Pf::PRESENT.with(Pf::WRITABLE).with(Pf::USER),
        PageFlags::USER_MMIO_UC => Pf::PRESENT
            .with(Pf::WRITABLE)
            .with(Pf::USER)
            .with(Pf::CACHE_DISABLE)
            .with(Pf::NO_EXECUTE),
        PageFlags::MMIO_UC => Pf(Pf::KERNEL_RW.0 | Pf::CACHE_DISABLE.0),
        _ => Pf::KERNEL_RO,
    }
}

impl PageTable for HalImpl {
    fn kmap_4k(&self, virt: u64, phys: u64, flags: PageFlags) -> Result<(), PageError> {
        unsafe { crate::paging::kmap_4k(virt, phys, map_page_flags(flags)).map_err(page_err) }
    }
    fn kmap_2m(&self, virt: u64, phys: u64, flags: PageFlags) -> Result<(), PageError> {
        unsafe { crate::paging::kmap_2m(virt, phys, map_page_flags(flags)).map_err(page_err) }
    }
    fn kunmap_4k(&self, virt: u64) -> Result<(), PageError> {
        unsafe { crate::paging::kunmap_4k(virt).map_err(page_err) }
    }
    fn kvirt_to_phys(&self, virt: u64) -> Option<u64> {
        unsafe { crate::paging::kvirt_to_phys(virt) }
    }
    fn kensure_4k(&self, virt: u64) -> Result<(), PageError> {
        unsafe { crate::paging::kensure_4k(virt).map_err(page_err) }
    }
    fn kmap_mmio(&self, phys: u64, size: u64) -> Result<(), PageError> {
        unsafe { crate::paging::kmap_mmio(phys, size).map_err(page_err) }
    }
    fn kmark_uncacheable(&self, virt: u64) -> Result<(), PageError> {
        unsafe { crate::paging::kmark_uncacheable(virt).map_err(page_err) }
    }
    fn kernel_pml4_phys(&self) -> u64 {
        unsafe { crate::paging::kernel_pml4_phys() }
    }

    fn pml4_new_empty(&self) -> Result<Pml4Handle, PageError> {
        unsafe {
            let phys = crate::memory::global_registry_mut()
                .allocate_pages(
                    crate::memory::AllocateType::AnyPages,
                    crate::memory::MemoryType::AllocatedPageTable,
                    1,
                )
                .map_err(|_| PageError::OutOfMemory)?;
            core::ptr::write_bytes(phys as *mut u8, 0, 4096);
            Ok(Pml4Handle(phys))
        }
    }
    fn pml4_translate(&self, pml4: Pml4Handle, virt: u64) -> Option<u64> {
        unsafe {
            let mgr = crate::paging::PageTableManager { pml4_phys: pml4.0 };
            mgr.translate(virt)
        }
    }
    fn pml4_map_4k(
        &self,
        pml4: Pml4Handle,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), PageError> {
        unsafe {
            let mut mgr = crate::paging::PageTableManager { pml4_phys: pml4.0 };
            mgr.map_4k(virt, phys, map_page_flags(flags))
                .map_err(page_err)
        }
    }
    fn pml4_clone_kernel_half(&self, dst: Pml4Handle) -> Result<(), PageError> {
        // Shallow-copy all 512 entries; kernel-half entries have no USER bit so
        // ring 3 still cannot walk them.
        // SAFETY: identity-mapped long mode; kernel PML4 live; `dst` owned.
        unsafe {
            let kernel_pml4 = crate::paging::kernel_pml4_phys();
            let mut mgr = crate::paging::PageTableManager { pml4_phys: dst.0 };
            mgr.clone_kernel_half_from(kernel_pml4);
        }
        Ok(())
    }

    fn pml4_map_user_4k(
        &self,
        pml4: Pml4Handle,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), PageError> {
        // SAFETY: PML4 from HAL is identity-mapped + mutable; `map_user_4k` is
        // single-threaded-only per its docs.
        unsafe {
            let mut mgr = crate::paging::PageTableManager { pml4_phys: pml4.0 };
            mgr.map_user_4k(virt, phys, map_page_flags(flags))
                .map_err(page_err)
        }
    }

    fn pml4_unmap_4k(&self, pml4: u64, virt: u64) -> Result<(), PageError> {
        // SAFETY: caller owns the PML4; `unmap_4k` is idempotent on absent pages.
        unsafe {
            let mut mgr = crate::paging::PageTableManager { pml4_phys: pml4 };
            mgr.unmap_4k(virt).map_err(|e| {
                if e.contains("huge") {
                    PageError::HugePageBlocked
                } else {
                    PageError::InvalidAddress
                }
            })
        }
    }

    fn pml4_remap_flags(&self, pml4: u64, virt: u64, flags: PageFlags) -> Result<(), PageError> {
        // SAFETY: caller owns the PML4; `remap_4k_flags` mutates one leaf + invlpg.
        unsafe {
            let mut mgr = crate::paging::PageTableManager { pml4_phys: pml4 };
            mgr.remap_4k_flags(virt, map_page_flags(flags))
                .map_err(|e| {
                    if e.contains("huge") {
                        PageError::HugePageBlocked
                    } else {
                        PageError::NotMapped
                    }
                })
        }
    }

    fn flush_tlb_page(&self, virt: u64) {
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack));
        }
    }
    fn flush_tlb_all(&self) {
        unsafe {
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
        }
    }
    fn for_each_pt_page(&self, f: &mut dyn FnMut(u64)) {
        // SAFETY: paging live; walk is read-only.
        unsafe {
            let (pages, count) = crate::paging::collect_page_table_pages();
            for &p in &pages[..count] {
                f(p);
            }
        }
    }

    fn current_cr3(&self) -> u64 {
        let cr3: u64;
        // SAFETY: privileged read, no memory side effects.
        unsafe {
            core::arch::asm!(
                "mov {}, cr3",
                out(reg) cr3,
                options(nomem, nostack, preserves_flags),
            );
        }
        cr3
    }

    unsafe fn write_cr3(&self, cr3: u64) {
        // SAFETY: caller asserts valid root w/ kernel half mapped. CR3 write
        // implicitly flushes non-global TLB entries.
        core::arch::asm!(
            "mov cr3, {}",
            in(reg) cr3,
            options(nostack, preserves_flags),
        );
    }
}

impl InterruptController for HalImpl {
    fn set_handler(&self, vector: u8, handler: IsrFn, ist: u8, dpl: u8) {
        unsafe {
            crate::cpu::idt::set_interrupt_handler(vector, handler.0 as u64, ist, dpl);
        }
    }
    fn enable_irq(&self, irq: u8) {
        unsafe { crate::cpu::pic::enable_irq(irq) }
    }
    fn disable_irq(&self, irq: u8) {
        unsafe { crate::cpu::pic::disable_irq(irq) }
    }
    fn send_pic_eoi(&self, irq: u8) {
        unsafe { crate::cpu::pic::send_eoi(irq) }
    }
    fn send_lapic_eoi(&self) {
        unsafe { crate::cpu::apic::send_eoi() }
    }
    fn read_lapic_id(&self) -> u32 {
        unsafe { crate::cpu::apic::read_lapic_id() }
    }
    fn lapic_base(&self) -> u64 {
        crate::cpu::apic::lapic_base()
    }
    fn enable_msi_single(&self, dev: BusAddr, apic_id: u32, vec: u8) -> Result<(), MsiError> {
        let pci_addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        let cap = crate::pci::msi::find_msi(pci_addr).ok_or(MsiError::CapabilityNotFound)?;
        let addr = crate::pci::msi::lapic_msi_addr(apic_id);
        cap.program(addr, vec);
        Ok(())
    }
    fn enable_msix_single(&self, dev: BusAddr, apic_id: u32, vec: u8) -> Result<(), MsiError> {
        let pci_addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        let cap = crate::pci::msi::find_msix(pci_addr).ok_or(MsiError::CapabilityNotFound)?;
        let addr = crate::pci::msi::lapic_msi_addr(apic_id);
        unsafe { cap.program_entry(0, addr, vec, false) };
        cap.set_function_mask(false);
        cap.set_enable(true);
        Ok(())
    }
    fn disable_intx(&self, dev: BusAddr) {
        let pci_addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        crate::pci::msi::disable_intx(pci_addr);
    }
    unsafe fn disable_legacy_pic(&self) {
        // SAFETY: 0xFF→PIC1/PIC2 IMR. Caller runs with IF=0 or PIC already quiesced.
        crate::cpu::apic::disable_pic8259();
    }
}

impl Timer for HalImpl {
    fn read_tsc(&self) -> u64 {
        crate::cpu::tsc::read_tsc()
    }
    fn tsc_frequency(&self) -> u64 {
        crate::cpu::tsc::tsc_frequency()
    }
    fn delay_us(&self, us: u64) {
        unsafe { crate::cpu::apic::delay_us(us) }
    }
    fn now_ns(&self) -> u64 {
        let tsc = crate::cpu::tsc::read_tsc();
        let hz = crate::cpu::tsc::tsc_frequency();
        if hz == 0 {
            return 0;
        }
        // u128 to avoid overflow.
        let ns = (tsc as u128).saturating_mul(1_000_000_000) / (hz as u128);
        ns as u64
    }
    fn setup_periodic(&self, hz: u32) {
        unsafe { crate::cpu::apic::setup_timer(hz) }
    }
}

impl DmaAllocator for HalImpl {
    fn alloc_dma(&self, bytes: usize) -> Result<DmaRegion, MemError> {
        let pages = bytes.div_ceil(crate::memory::PAGE_SIZE as usize) as u64;
        unsafe {
            let phys = crate::memory::global_registry_mut()
                .allocate_pages(
                    crate::memory::AllocateType::MaxAddress(0x1_0000_0000),
                    crate::memory::MemoryType::AllocatedDma,
                    pages,
                )
                .map_err(|_| MemError::OutOfMemory)?;
            Ok(DmaRegion {
                cpu_ptr: phys as *mut u8,
                bus_addr: phys,
                size: bytes,
            })
        }
    }
    fn free_dma(&self, region: DmaRegion) {
        let pages = region.size.div_ceil(crate::memory::PAGE_SIZE as usize) as u64;
        unsafe {
            let _ = crate::memory::global_registry_mut().free_pages(region.bus_addr, pages);
        }
    }
    fn sync_for_device(&self, _region: &DmaRegion, _off: usize, _len: usize) {
        // x86_64 is WB-coherent for DMA.
    }
    fn sync_for_cpu(&self, _region: &DmaRegion, _off: usize, _len: usize) {
        // x86_64 is WB-coherent for DMA.
    }
}

impl BusEnumerator for HalImpl {
    fn cfg_read8(&self, dev: BusAddr, off: u16) -> u8 {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        crate::pci::config::pci_cfg_read8(addr, off as u8)
    }
    fn cfg_read16(&self, dev: BusAddr, off: u16) -> u16 {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        crate::pci::config::pci_cfg_read16(addr, off as u8)
    }
    fn cfg_read32(&self, dev: BusAddr, off: u16) -> u32 {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        crate::pci::config::pci_cfg_read32(addr, off as u8)
    }
    fn cfg_write8(&self, dev: BusAddr, off: u16, val: u8) {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        crate::pci::config::pci_cfg_write8(addr, off as u8, val)
    }
    fn cfg_write16(&self, dev: BusAddr, off: u16, val: u16) {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        crate::pci::config::pci_cfg_write16(addr, off as u8, val)
    }
    fn cfg_write32(&self, dev: BusAddr, off: u16, val: u32) {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        crate::pci::config::pci_cfg_write32(addr, off as u8, val)
    }
    fn for_each_device(&self, f: &mut dyn FnMut(BusAddr)) {
        for bus in 0..=255u8 {
            for device in 0..32u8 {
                let addr = crate::pci::config::PciAddr::new(bus, device, 0);
                let vendor = crate::pci::config::pci_cfg_read16(addr, 0x00);
                if vendor == 0xFFFF {
                    continue;
                }
                f(BusAddr::new(bus, device, 0));
                let header = crate::pci::config::pci_cfg_read8(addr, 0x0E);
                if header & 0x80 != 0 {
                    // Multi-function device.
                    for function in 1..8u8 {
                        let fa = crate::pci::config::PciAddr::new(bus, device, function);
                        let v = crate::pci::config::pci_cfg_read16(fa, 0x00);
                        if v != 0xFFFF {
                            f(BusAddr::new(bus, device, function));
                        }
                    }
                }
            }
        }
    }
    fn read_bar(&self, dev: BusAddr, bar_idx: u8) -> u64 {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        let off = 0x10u8 + bar_idx * 4;
        (crate::pci::config::pci_cfg_read32(addr, off) as u64) & !0xFu64
    }
    fn enable_bus_master(&self, dev: BusAddr) {
        let addr = crate::pci::config::PciAddr::new(dev.bus, dev.device, dev.function);
        let cmd = crate::pci::config::pci_cfg_read16(addr, 0x04);
        crate::pci::config::pci_cfg_write16(addr, 0x04, cmd | 0x06);
    }
}

impl UsbHost for HalImpl {
    // Stubs; real impl lives in `hwinit::usb::runtime` until the morpheus-xhci
    // dep cycle is broken. Kernel path bypasses the trait and calls hwinit directly.
    fn keyboard_present(&self) -> bool {
        false
    }
    fn poll_keyboard(&self) -> bool {
        false
    }
}

impl Reset for HalImpl {
    fn reset_machine(&self) -> ! {
        unsafe { crate::cpu::reset::reset_machine_now() }
    }
    fn wait_for_keypress_or_timeout_ms(&self, ms: u64) {
        unsafe { crate::cpu::reset::wait_for_keypress_or_timeout_ms(ms) }
    }
}

impl Smp for HalImpl {
    fn cpu_count(&self) -> u32 {
        crate::cpu::per_cpu::cpu_count()
    }
    fn max_cpus(&self) -> u32 {
        crate::cpu::per_cpu::MAX_CPUS as u32
    }
    fn current_core_index(&self) -> u32 {
        unsafe { crate::cpu::per_cpu::current_core_index() }
    }
    fn current_lapic_id(&self) -> u32 {
        unsafe { crate::cpu::apic::read_lapic_id() }
    }
    fn current_pid(&self) -> u32 {
        unsafe { crate::cpu::per_cpu::current_pid() }
    }
    fn set_current_pid(&self, pid: u32) {
        unsafe { crate::cpu::per_cpu::set_current_pid(pid) }
    }
    fn for_each_ap_lapic_id(&self, _f: &mut dyn FnMut(u32)) {
        // TODO: expose `ApLapicIds` snapshot as a callback walk.
    }
    fn start_aps(&self) -> u32 {
        // CPUID brute-force path (used when MADT discovery yields no APs).
        // Detect topology via CPUID and publish it so `ap_boot::start_aps`
        // (which reads `cpu_count()`) sees the real core count instead of the
        // default 1 — otherwise it short-circuits as "single-core".
        unsafe {
            let n = crate::cpu::apic::detect_cpu_count().max(1);
            crate::cpu::per_cpu::set_cpu_count(n);
            // Match start_aps_from_list's IF discipline: bring APs up with
            // interrupts masked, then restore.
            let was_enabled = crate::intr::interrupts_enabled();
            crate::intr::disable_interrupts();
            crate::cpu::ap_boot::start_aps();
            if was_enabled {
                crate::intr::enable_interrupts();
            }
        }
        crate::cpu::per_cpu::cpu_count()
    }
    fn release_parked_aps(&self) {
        crate::cpu::ap_boot::release_parked_aps()
    }
    fn request_shutdown_quiesce(&self) {
        crate::cpu::per_cpu::request_shutdown_quiesce();
    }
    fn shutdown_quiesce_ack(&self, core_idx: u32) {
        crate::cpu::per_cpu::shutdown_quiesce_ack(core_idx);
    }
    fn wait_for_shutdown_quiesce(&self, ms: u64) -> bool {
        unsafe { crate::cpu::per_cpu::wait_for_shutdown_quiesce(ms) }
    }

    fn set_kernel_stack_for_core(&self, core_idx: u32, kernel_sp: u64) {
        // SAFETY: bounds-checks `core_idx`. Caller asserts `kernel_sp` is a valid mapped stack top.
        unsafe { crate::cpu::gdt::set_kernel_stack_for_core(core_idx, kernel_sp) }
    }

    fn pcpu_set_next_cr3(&self, cr3: u64) {
        // SAFETY: writes `gs:[0x10]` on current CPU. GS base set by init_bsp/init_ap.
        unsafe {
            core::arch::asm!(
                "mov gs:[0x10], {0}",
                in(reg) cr3,
                options(nostack, preserves_flags),
            );
        }
    }

    fn pcpu_set_fpu_ptr(&self, ptr: u64) {
        // SAFETY: writes `gs:[0x18]` on current CPU.
        unsafe {
            core::arch::asm!(
                "mov gs:[0x18], {0}",
                in(reg) ptr,
                options(nostack, preserves_flags),
            );
        }
    }

    fn pcpu_set_kernel_syscall_rsp(&self, rsp: u64) {
        // SAFETY: writes `gs:[0x20]` on current CPU. Caller asserts `rsp` is a mapped stack top.
        unsafe {
            core::arch::asm!(
                "mov gs:[0x20], {0}",
                in(reg) rsp,
                options(nostack, preserves_flags),
            );
        }
    }

    fn pcpu_boot_kernel_rsp(&self) -> u64 {
        // `boot_kernel_rsp` sits past the gs:[0x00..0x48] ABI region; read via
        // the PerCpu pointer rather than inline asm.
        // SAFETY: GS base set by init_bsp/init_ap; ref is to a 'static array entry.
        unsafe { crate::cpu::per_cpu::current().boot_kernel_rsp }
    }

    fn is_reboot_owner(&self, core_idx: u32) -> bool {
        crate::cpu::per_cpu::is_reboot_owner(core_idx)
    }

    fn set_reboot_owner(&self, core_idx: u32) {
        crate::cpu::per_cpu::set_reboot_owner(core_idx);
    }

    fn clear_reboot_owner(&self) {
        crate::cpu::per_cpu::clear_reboot_owner();
    }

    fn shutdown_quiesce_requested(&self) -> bool {
        crate::cpu::per_cpu::shutdown_quiesce_requested()
    }

    fn ap_online_count(&self) -> u32 {
        // AP_ONLINE_COUNT includes BSP (init_bsp stores 1; each AP increments).
        let total =
            crate::cpu::per_cpu::AP_ONLINE_COUNT.load(core::sync::atomic::Ordering::Acquire);
        total.saturating_sub(1)
    }

    fn percpu_ready(&self) -> bool {
        // Raw count > 0 ⇔ init_bsp ran (it stores 1), so the GS per-CPU block is
        // live — true even single-core. (ap_online_count subtracts the BSP.)
        crate::cpu::per_cpu::AP_ONLINE_COUNT.load(core::sync::atomic::Ordering::Acquire) > 0
    }

    unsafe fn discover_ap_lapic_ids(&self, rsdp_phys: u64) -> Result<&'static [u32], SmpError> {
        // SAFETY: BSP, single-threaded, ACPI tables still mapped per trait docs.
        // Empty slice ≠ error — signals "fall back to CPUID".
        Ok(crate::cpu::acpi::discover_ap_lapic_ids_static(rsdp_phys))
    }

    unsafe fn start_aps_from_list(&self, lapic_ids: &[u32]) -> Result<u32, SmpError> {
        // SAFETY: BSP, scheduler live (LD16).
        if lapic_ids.is_empty() {
            return Ok(0);
        }
        if lapic_ids.len() >= crate::cpu::per_cpu::MAX_CPUS {
            return Err(SmpError::TooManyCpus);
        }

        crate::cpu::per_cpu::set_cpu_count(lapic_ids.len() as u32 + 1);

        let was_enabled = crate::intr::interrupts_enabled();
        crate::intr::disable_interrupts();
        crate::cpu::ap_boot::start_aps_from_list(lapic_ids);
        if was_enabled {
            crate::intr::enable_interrupts();
        }

        let online =
            crate::cpu::per_cpu::AP_ONLINE_COUNT.load(core::sync::atomic::Ordering::Acquire);
        Ok(online.saturating_sub(1))
    }

    fn timer_isr(&self) -> IsrFn {
        // `irq_timer_isr` lives in asm/cpu/context_switch.s.
        extern "C" {
            fn irq_timer_isr();
        }
        IsrFn(irq_timer_isr as unsafe extern "C" fn())
    }
}

impl Compositor for HalImpl {
    unsafe fn fb_present_delta(
        &self,
        back: u64,
        shadow: u64,
        vram: u64,
        width: u64,
        height: u64,
        stride: u64,
    ) {
        // `asm_fb_present_delta` lives in asm/fb/present.s.
        extern "win64" {
            fn asm_fb_present_delta(
                back: u64,
                shadow: u64,
                vram: u64,
                width: u64,
                height: u64,
                stride: u64,
            );
        }
        // SAFETY: caller asserts distinct mapped buffers of `height * stride`.
        asm_fb_present_delta(back, shadow, vram, width, height, stride);
    }
}
