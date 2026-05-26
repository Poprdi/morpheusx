//! Object-safe HAL trait surface; kernel depends on this crate, not on the concrete impl.
//!
//! All traits are object-safe so the kernel can dispatch through `&'static dyn Hal`.
//! Adding a method is a contract change requiring every HAL impl to update in lockstep.

#![no_std]


/// UEFI memory taxonomy + custom allocator tags (0x8000_xxxx range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MemoryType {
    Reserved = 0,
    LoaderCode = 1,
    LoaderData = 2,
    BootServicesCode = 3,
    BootServicesData = 4,
    RuntimeServicesCode = 5,
    RuntimeServicesData = 6,
    Conventional = 7,
    Unusable = 8,
    AcpiReclaim = 9,
    AcpiNvs = 10,
    Mmio = 11,
    MmioPortSpace = 12,
    PalCode = 13,
    Persistent = 14,
    Allocated = 0x8000_0000,
    AllocatedDma = 0x8000_0001,
    AllocatedStack = 0x8000_0002,
    AllocatedPageTable = 0x8000_0003,
    AllocatedHeap = 0x8000_0004,
}

/// UEFI EFI_MEMORY_* attribute bits, raw so HAL impls can pass through unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct MemoryAttribute(pub u64);

/// Mirrors `EFI_MEMORY_DESCRIPTOR`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryDescriptor {
    pub mem_type: MemoryType,
    pub phys_start: u64,
    pub virt_start: u64,
    pub num_pages: u64,
    pub attribute: MemoryAttribute,
}

/// Compact E820 entry for legacy bootloaders / dumpers.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct E820Entry {
    pub base: u64,
    pub size: u64,
    pub kind: u32,
}

/// Mirrors `AllocateType` in the buddy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocKind {
    AnyPages,
    /// `addr < max_address`.
    MaxAddress(u64),
    /// Exact physical address.
    Address(u64),
}

/// CPU-visible base + device-visible bus address + size.
#[derive(Debug, Clone, Copy)]
pub struct DmaRegion {
    pub cpu_ptr: *mut u8,
    pub bus_addr: u64,
    pub size: usize,
}

// SAFETY: pointer identifies an identity-mapped region; handle is cross-core safe.
unsafe impl Send for DmaRegion {}
unsafe impl Sync for DmaRegion {}

/// PCI/PCIe device coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct BusAddr {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl BusAddr {
    pub const fn new(bus: u8, device: u8, function: u8) -> Self {
        Self {
            bus,
            device,
            function,
        }
    }
}

/// Opaque page-table flag presets; arch HALs map these to native descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PageFlags(pub u64);

impl PageFlags {
    pub const KERNEL_RO: Self = Self(0);
    pub const KERNEL_RW: Self = Self(1);
    pub const KERNEL_CODE: Self = Self(2);
    pub const USER_RO: Self = Self(3);
    pub const USER_RW: Self = Self(4);
    /// Legacy alias for `USER_RX`.
    pub const USER_CODE: Self = Self(5);
    pub const USER_RX: Self = Self(5);
    pub const USER_RWX: Self = Self(7);
    /// User-accessible strongly-uncached MMIO (for `mmap_io` exposing BARs to ring 3).
    pub const USER_MMIO_UC: Self = Self(8);
    /// Kernel-side strongly-uncached MMIO.
    pub const MMIO_UC: Self = Self(6);
}

/// Opaque PML4 handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Pml4Handle(pub u64);

/// Typed ISR pointer; newtype prevents swapping with raw `u64`.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct IsrFn(pub unsafe extern "C" fn());


/// Opaque per-task register file. Sized for any arch (x86_64 uses 160 B, aarch64 ~256 B).
/// Kernel stores inline but mutates only via `Cpu` trait methods.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct CpuContext {
    _opaque: [u64; 64],
}

impl CpuContext {
    pub const fn zeroed() -> Self {
        Self { _opaque: [0; 64] }
    }
}

/// Opaque FPU/SIMD state. 512 B fits FXSAVE; HALs needing full XSAVE (AVX-512)
/// can use a side allocation keyed off the context.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct FpuState {
    _opaque: [u8; 512],
}

impl FpuState {
    pub const fn zeroed() -> Self {
        Self { _opaque: [0; 512] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    OutOfMemory,
    InvalidAddress,
    InvalidArgument,
    NotInitialized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageError {
    OutOfMemory,
    InvalidAddress,
    AlreadyMapped,
    NotMapped,
    HugePageBlocked,
    NotInitialized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsiError {
    CapabilityNotFound,
    InvalidConfig,
    UnsupportedMode,
}

/// SMP / AP bring-up failure modes. `NotSupported` triggers single-core fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmpError {
    /// RSDP zero / MADT missing / checksum invalid.
    AcpiUnavailable,
    /// Trampoline binary missing or relocation failed.
    TrampolineUnavailable,
    /// LAPIC ID list longer than `max_cpus`.
    TooManyCpus,
    /// Arch has no LAPIC-equivalent control surface.
    NotSupported,
}

/// Bundle of kernel-side callbacks installed atomically into the HAL at init,
/// so hardware paths can dispatch into kernel policy without knowing internals.
#[derive(Clone, Copy)]
pub struct KernelHooks {
    /// 100 Hz LAPIC timer ISR target.
    pub scheduler_tick: Option<unsafe extern "C" fn()>,
    /// Current PID (gs:[0x0C]). 0 before scheduler is live.
    pub current_pid: Option<unsafe fn() -> u32>,
    /// Crash-dump process-name lookup.
    pub process_lookup: Option<unsafe fn(pid: u32, name: &mut [u8; 32]) -> bool>,
    /// User-mode fault terminates the process.
    pub process_exit: Option<unsafe fn(code: i32) -> !>,
    /// Kernel CR3 for `KernelCr3Guard`.
    pub kernel_cr3: Option<unsafe fn() -> u64>,
    pub keyboard_sink: Option<fn(byte: u8, pressed: bool)>,
    pub mouse_sink: Option<fn(dx: i16, dy: i16, buttons: u8, wheel: i8)>,
}

impl KernelHooks {
    pub const fn empty() -> Self {
        Self {
            scheduler_tick: None,
            current_pid: None,
            process_lookup: None,
            process_exit: None,
            kernel_cr3: None,
            keyboard_sink: None,
            mouse_sink: None,
        }
    }
}


/// Top-level HAL trait. Per-arch crate provides one `HalImpl`; bootloader leaks it
/// as `&'static dyn Hal`. Narrow sub-trait accessors so drivers see only the cap they need.
pub trait Hal: Send + Sync {
    fn mmio(&self) -> &dyn Mmio;
    fn cpu(&self) -> &dyn Cpu;
    fn serial(&self) -> &dyn Serial;
    fn phys(&self) -> &dyn PhysAlloc;
    fn paging(&self) -> &dyn PageTable;
    fn intr(&self) -> &dyn InterruptController;
    fn timer(&self) -> &dyn Timer;
    fn dma(&self) -> &dyn DmaAllocator;
    fn bus(&self) -> &dyn BusEnumerator;
    fn usb(&self) -> &dyn UsbHost;
    fn reset(&self) -> &dyn Reset;
    fn smp(&self) -> &dyn Smp;
    fn compositor(&self) -> &dyn Compositor;
}

/// MMIO primitives. Width-specific because some platforms gate widths on alignment.
///
/// # Safety
/// `addr` must point at a valid MMIO region, satisfy width alignment, and respect
/// the device's serialization requirements (caller issues fences as needed).
pub trait Mmio: Send + Sync {
    /// # Safety: see trait docs.
    unsafe fn read8(&self, addr: u64) -> u8;
    /// # Safety: see trait docs.
    unsafe fn read16(&self, addr: u64) -> u16;
    /// # Safety: see trait docs.
    unsafe fn read32(&self, addr: u64) -> u32;
    /// # Safety: see trait docs.
    unsafe fn read64(&self, addr: u64) -> u64;
    /// # Safety: see trait docs.
    unsafe fn write8(&self, addr: u64, val: u8);
    /// # Safety: see trait docs.
    unsafe fn write16(&self, addr: u64, val: u16);
    /// # Safety: see trait docs.
    unsafe fn write32(&self, addr: u64, val: u32);
    /// # Safety: see trait docs.
    unsafe fn write64(&self, addr: u64, val: u64);
    fn mfence(&self);
    fn sfence(&self);
    fn lfence(&self);
}

/// CPU control primitives for critical-section entry/exit and the idle loop.
pub trait Cpu: Send + Sync {
    fn disable_interrupts(&self);
    fn enable_interrupts(&self);
    /// Halt until next IRQ. Safe with IF either way.
    fn halt(&self);
    fn interrupts_enabled(&self) -> bool;

    /// Atomic `sti; hlt; cli` (or arch equivalent). Splitting races: an IRQ between
    /// `sti` and `hlt` would be consumed before HLT, and HLT then never wakes.
    /// x86_64 relies on the STI interrupt-shadow delaying enable past the HLT.
    fn halt_wait_irq(&self);

    /// Halt with IF already disabled. Used by AP quiesce loops awaiting INIT/SIPI/NMI.
    fn halt_no_irq(&self);

    /// Unrecoverable exception that the IDT crash hook renders as BSoD.
    /// x86_64: `ud2`. ARM: `udf #0`.
    fn crash_now(&self) -> !;

    /// x86 CPUID; `[eax, ebx, ecx, edx]`. Non-x86 returns `[0; 4]` (kernel surfaces ENOSYS per LD25).
    fn cpuid(&self, leaf: u32, subleaf: u32) -> [u32; 4];

    /// One-shot install of syscall entry MSRs / vectors after IDT is up.
    /// x86_64: STAR/LSTAR/FMASK for `syscall`/`sysret`. ARM: VBAR_EL1.
    ///
    /// # Safety
    /// Exactly once per CPU, after IDT init. Subsequent calls are UB.
    unsafe fn install_syscall_msrs(&self);

    // Per-task context construction; kernel never touches the opaque bytes directly.

    /// Build a kernel-mode context that begins at `entry()` on `stack_top`, CPL=0, IF=1.
    fn ctx_init_kernel(&self, ctx: &mut CpuContext, entry: u64, stack_top: u64);

    /// Build a user-mode context. CPL=3, args go in calling-convention argument regs
    /// (x86_64: rdi/rsi/rdx/rcx/r8/r9; aarch64: x0..x5).
    fn ctx_init_user(
        &self,
        ctx: &mut CpuContext,
        entry_va: u64,
        user_stack_top: u64,
        args: &[u64; 6],
    );

    /// Override IP (signal dispatch). x86: RIP, aarch64: ELR_EL1.
    fn ctx_set_ip(&self, ctx: &mut CpuContext, ip: u64);

    /// Override SP (signal frame setup). x86: RSP, aarch64: SP_EL0.
    fn ctx_set_sp(&self, ctx: &mut CpuContext, sp: u64);

    /// Set Nth syscall arg reg. `n` outside [0,5] is a silent no-op.
    /// x86_64: 0=rdi,1=rsi,2=rdx,3=rcx,4=r8,5=r9. aarch64: x0..x5.
    fn ctx_set_arg(&self, ctx: &mut CpuContext, n: u8, val: u64);

    /// x86: RAX. aarch64: X0.
    fn ctx_set_return(&self, ctx: &mut CpuContext, val: u64);

    /// x86: RAX. aarch64: X0.
    fn ctx_get_return(&self, ctx: &CpuContext) -> u64;

    /// x86: rsp, ARM: sp_el0.
    fn ctx_get_sp(&self, ctx: &CpuContext) -> u64;

    /// True if context returns to ring 3 / EL0. x86: CS&3==3, aarch64: PSTATE.M.
    fn ctx_is_user_mode(&self, ctx: &CpuContext) -> bool;

    /// x86_64: ORs CPL=3 into SS (mirrors legacy `cur.context.ss |= 3`).
    /// aarch64: updates saved PSTATE.M.
    fn ctx_set_user_mode(&self, ctx: &mut CpuContext, user: bool);

    /// Seed an FPU blob with arch fresh-thread defaults.
    /// x86_64: FCW=0x037F, MXCSR=0x1F80, XMMs zeroed. aarch64: FPCR/FPSR/Qs zero.
    fn fpu_init(&self, fpu: &mut FpuState);

    /// Arm/disarm the "BSoD then hard reset" IDT path used by `SYS_SYSTEM_CONTROL(SHUTDOWN_PANIC)`.
    fn set_reset_on_crash(&self, enable: bool);
}

/// Lock-free, panic-safe UART output.
pub trait Serial: Send + Sync {
    fn putc(&self, b: u8);
    fn puts(&self, s: &str);
    fn put_hex32(&self, v: u32);
    fn put_hex64(&self, v: u64);
    fn newline(&self);
}

/// Physical memory allocator over a buddy + the UEFI memory map.
pub trait PhysAlloc: Send + Sync {
    /// Allocate `pages` contiguous 4 KiB pages; returns physical base.
    fn allocate_pages(
        &self,
        kind: AllocKind,
        mt: MemoryType,
        pages: u64,
    ) -> Result<u64, MemError>;
    fn free_pages(&self, addr: u64, pages: u64) -> Result<(), MemError>;
    fn is_initialized(&self) -> bool;
    fn page_size(&self) -> u64;
    fn total_memory(&self) -> u64;
    fn free_memory(&self) -> u64;
    fn allocated_memory(&self) -> u64;
    fn memory_type_at(&self, phys: u64) -> MemoryType;
    fn is_valid_cr3(&self, cr3: u64) -> bool;
    fn find_largest_free_below_4gb(&self) -> Option<(u64, u64)>;
    fn for_each_descriptor(&self, f: &mut dyn FnMut(&MemoryDescriptor));
    fn export_e820(&self, out: &mut [E820Entry]) -> usize;

    /// Reclaim UEFI BootServices{Code,Data} into the free pool. Returns bytes added.
    /// HAL impl must exclude live page-table pages and platform quirk holes (low 1 MiB on x86).
    ///
    /// # Safety
    /// Once only, post-`ExitBootServices`, after the scheduler is live and every late-boot
    /// transient BS reference is gone. Single-threaded; impl toggles IF as needed.
    unsafe fn reclaim_boot_services(&self) -> Result<u64, ()>;
}

/// Kernel + per-process page tables.
pub trait PageTable: Send + Sync {
    fn kmap_4k(&self, virt: u64, phys: u64, flags: PageFlags) -> Result<(), PageError>;
    fn kmap_2m(&self, virt: u64, phys: u64, flags: PageFlags) -> Result<(), PageError>;
    fn kunmap_4k(&self, virt: u64) -> Result<(), PageError>;
    fn kvirt_to_phys(&self, virt: u64) -> Option<u64>;
    fn kensure_4k(&self, virt: u64) -> Result<(), PageError>;
    fn kmap_mmio(&self, phys: u64, size: u64) -> Result<(), PageError>;
    fn kmark_uncacheable(&self, virt: u64) -> Result<(), PageError>;
    fn kernel_pml4_phys(&self) -> u64;

    fn pml4_new_empty(&self) -> Result<Pml4Handle, PageError>;
    fn pml4_translate(&self, pml4: Pml4Handle, virt: u64) -> Option<u64>;
    fn pml4_map_4k(
        &self,
        pml4: Pml4Handle,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), PageError>;
    fn pml4_clone_kernel_half(&self, dst: Pml4Handle) -> Result<(), PageError>;

    /// Map a 4 KiB user page in a non-current PML4. Used by the ELF loader.
    ///
    /// Propagates the USER bit through every intermediate level (AMD64 Vol 2 §5.4.1)
    /// regardless of preset. May allocate intermediates, split 2 MiB / 1 GiB huge
    /// entries, and COW kernel-shared no-USER intermediates so the kernel PML4
    /// is never corrupted. `pml4_map_4k` is the non-user fast path.
    fn pml4_map_user_4k(
        &self,
        pml4: Pml4Handle,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), PageError>;

    /// Idempotent. `HugePageBlocked` if the walk lands on a 2 MiB / 1 GiB leaf.
    fn pml4_unmap_4k(&self, pml4: u64, virt: u64) -> Result<(), PageError>;

    /// `mprotect` backend: rewrite PTE flags in place, preserving the frame.
    /// Returns `NotMapped` if any level is absent, `HugePageBlocked` on huge leaf.
    /// Issues `invlpg` (or arch equivalent) on success.
    fn pml4_remap_flags(&self, pml4: u64, virt: u64, flags: PageFlags) -> Result<(), PageError>;

    fn flush_tlb_page(&self, virt: u64);
    fn flush_tlb_all(&self);
    fn for_each_pt_page(&self, f: &mut dyn FnMut(u64));

    /// x86: CR3 (PML4 phys + PCID). ARM: TTBR0_EL1.
    fn current_cr3(&self) -> u64;

    /// Restore the previous CR3 (used by `KernelCr3Guard::Drop`, LD26).
    ///
    /// # Safety
    /// `cr3` must point to a valid root with the kernel half mapped. x86 CR3-write
    /// flushes TLB; ARM TTBR write requires explicit TLBI + DSB + ISB.
    unsafe fn write_cr3(&self, cr3: u64);
}

/// IDT + PIC + LAPIC + MSI.
pub trait InterruptController: Send + Sync {
    fn set_handler(&self, vector: u8, handler: IsrFn, ist: u8, dpl: u8);
    fn enable_irq(&self, irq: u8);
    fn disable_irq(&self, irq: u8);
    fn send_pic_eoi(&self, irq: u8);
    fn send_lapic_eoi(&self);
    fn read_lapic_id(&self) -> u32;
    fn lapic_base(&self) -> u64;
    fn enable_msi_single(&self, dev: BusAddr, apic_id: u32, vec: u8) -> Result<(), MsiError>;
    fn enable_msix_single(&self, dev: BusAddr, apic_id: u32, vec: u8) -> Result<(), MsiError>;
    fn disable_intx(&self, dev: BusAddr);

    /// Mask the legacy 8259 so LAPIC is sole IRQ source. Idempotent. No-op on aarch64.
    ///
    /// # Safety
    /// Run with IF off or the PIC already quiesced; an IRQ in the mask-write window
    /// can be lost.
    unsafe fn disable_legacy_pic(&self);
}

/// TSC + LAPIC timer + delay.
pub trait Timer: Send + Sync {
    fn read_tsc(&self) -> u64;
    fn tsc_frequency(&self) -> u64;
    fn delay_us(&self, us: u64);
    fn now_ns(&self) -> u64;
    /// Assumes the vector's handler is already installed via `InterruptController::set_handler`.
    fn setup_periodic(&self, hz: u32);
}

/// Identity-mapped DMA arena.
pub trait DmaAllocator: Send + Sync {
    fn alloc_dma(&self, bytes: usize) -> Result<DmaRegion, MemError>;
    fn free_dma(&self, region: DmaRegion);
    /// No-op on x86_64 (WB coherent); real work on ARM.
    fn sync_for_device(&self, region: &DmaRegion, off: usize, len: usize);
    /// No-op on x86_64 (WB coherent); real work on ARM.
    fn sync_for_cpu(&self, region: &DmaRegion, off: usize, len: usize);
}

/// PCI config space + bus walk. PCIe extended config uses the 12-bit `off`.
pub trait BusEnumerator: Send + Sync {
    fn cfg_read8(&self, dev: BusAddr, off: u16) -> u8;
    fn cfg_read16(&self, dev: BusAddr, off: u16) -> u16;
    fn cfg_read32(&self, dev: BusAddr, off: u16) -> u32;
    fn cfg_write8(&self, dev: BusAddr, off: u16, val: u8);
    fn cfg_write16(&self, dev: BusAddr, off: u16, val: u16);
    fn cfg_write32(&self, dev: BusAddr, off: u16, val: u32);
    fn for_each_device(&self, f: &mut dyn FnMut(BusAddr));
    fn read_bar(&self, dev: BusAddr, bar_idx: u8) -> u64;
    fn enable_bus_master(&self, dev: BusAddr);
}

/// USB HID input runtime. Bring-up runs inside `HalImpl::init`.
pub trait UsbHost: Send + Sync {
    fn keyboard_present(&self) -> bool;
    fn poll_keyboard(&self) -> bool;
}

/// Machine reset / shutdown.
pub trait Reset: Send + Sync {
    fn reset_machine(&self) -> !;
    /// BSoD "press any key to reboot" wait.
    fn wait_for_keypress_or_timeout_ms(&self, ms: u64);
}

/// CPU topology + AP bring-up. Quiesce policy lives in the kernel.
pub trait Smp: Send + Sync {
    fn cpu_count(&self) -> u32;
    fn max_cpus(&self) -> u32;
    fn current_core_index(&self) -> u32;
    fn current_lapic_id(&self) -> u32;
    fn current_pid(&self) -> u32;
    fn set_current_pid(&self, pid: u32);
    fn for_each_ap_lapic_id(&self, f: &mut dyn FnMut(u32));
    fn start_aps(&self) -> u32;
    fn release_parked_aps(&self);
    fn request_shutdown_quiesce(&self);
    fn shutdown_quiesce_ack(&self, core_idx: u32);
    fn wait_for_shutdown_quiesce(&self, ms: u64) -> bool;

    /// TSS RSP0 (or arch equivalent) for the given core; controls the ring-3→0 stack switch.
    fn set_kernel_stack_for_core(&self, core_idx: u32, kernel_sp: u64);

    /// "Next CR3 on context switch" slot on the current CPU. x86: `gs:[0x10]`.
    fn pcpu_set_next_cr3(&self, cr3: u64);

    /// FPU state pointer slot on the current CPU. x86: `gs:[0x18]` (read by context_switch.s).
    fn pcpu_set_fpu_ptr(&self, ptr: u64);

    /// Syscall RSP slot on the current CPU. x86: `gs:[0x20]` (loaded by SYSCALL entry).
    fn pcpu_set_kernel_syscall_rsp(&self, rsp: u64);

    /// Boot-time kernel RSP for the current CPU. Idle/wait loops park SP here between decisions.
    fn pcpu_boot_kernel_rsp(&self) -> u64;

    /// True if this core orchestrates shutdown.
    fn is_reboot_owner(&self, core_idx: u32) -> bool;

    fn set_reboot_owner(&self, core_idx: u32);

    fn clear_reboot_owner(&self);

    /// True after any core requests shutdown-quiesce; secondaries spin until set, then ack.
    fn shutdown_quiesce_requested(&self) -> bool;

    /// Online AP count (excludes BSP).
    fn ap_online_count(&self) -> u32;

    /// Parse MADT (or equivalent) and return AP LAPIC IDs excluding BSP.
    ///
    /// `rsdp_phys` overrides cached RSDP when non-zero. Returned slice is backed
    /// by a per-HAL static buffer; each call overwrites the previous result.
    /// Empty slice on x86 means caller should fall back to CPUID scan via `start_aps()`.
    ///
    /// # Safety
    /// BSP, single-threaded; ACPI tables identity-mapped and unreclaimed.
    unsafe fn discover_ap_lapic_ids(&self, rsdp_phys: u64) -> Result<&'static [u32], SmpError>;

    /// Decoupled from `start_aps()` (CPUID brute-force) so the kernel picks discovery path.
    /// HAL trampoline pivots into a Rust entry that dispatches via installed `KernelHooks`;
    /// no kernel-supplied entry point needed. Returns APs that reached the park loop.
    ///
    /// # Safety
    /// BSP, after `late_init` (scheduler live, LD16). Toggles IF internally.
    unsafe fn start_aps_from_list(&self, lapic_ids: &[u32]) -> Result<u32, SmpError>;

    /// IDT handle for the LAPIC timer ISR. HAL owns the trampoline (xsave/xrstor + dispatch
    /// to `KernelHooks::scheduler_tick`); kernel installs on vector 0x20 in late-init.
    fn timer_isr(&self) -> IsrFn;
}

/// Framebuffer composition primitives that benefit from arch SIMD.
/// Kernel composes the back buffer in neutral code; HAL handles present.
pub trait Compositor: Sync {
    /// Differential present: diff `back` vs `shadow` per row, copy changes to `vram`,
    /// update `shadow`. All buffers 32-bpp XRGB with identical `stride` bytes.
    ///
    /// # Safety
    /// `back`/`shadow`/`vram` distinct mapped buffers each `height * stride` bytes;
    /// `vram` uncached. Caller serializes — impl does not lock.
    unsafe fn fb_present_delta(
        &self,
        back: u64,
        shadow: u64,
        vram: u64,
        width: u64,
        height: u64,
        stride: u64,
    );
}
