//! ELF64 parser and user-process loader. Arch-agnostic: all paging and
//! physical-allocation work routes through the HAL trait.
//!
//! Phase 3.7 K7/B3 migration: this file used to live in `hwinit` and used
//! x86-specific `PageTableManager` / `PageFlags` bit ops. After the HAL grew
//! `pml4_new_empty` + `pml4_clone_kernel_half` + `pml4_map_user_4k`, the
//! whole loader is portable. The fn-pointer indirection through
//! `sched_hooks::install_elf_loader` is gone — callers invoke `load_elf64`
//! directly.

use crate::hal;
use crate::sched_hooks::LoadedElfImage;
use alloc::boxed::Box;
use alloc::vec::Vec;
use morpheus_hal_api::{AllocKind, MemoryType, PageFlags, Pml4Handle};

pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const EM_X86_64: u16 = 62;
pub const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// 4 KiB page size — matches every HAL impl's `PhysAlloc::page_size`. Kept as a
/// const so segment-size math doesn't need an extra HAL round-trip per call.
const PAGE_SIZE: u64 = 4096;

pub const USER_STACK_PAGES: u64 = 32;
pub const USER_STACK_SIZE: u64 = USER_STACK_PAGES * PAGE_SIZE;
pub const USER_STACK_TOP: u64 = 0x0000_007F_FFFF_F000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    Not64Bit,
    NotLittleEndian,
    NotX86_64,
    NotExecutable,
    BadPhdr,
    NoLoadSegments,
    MapFailed,
    AllocFailed,
}

pub fn validate_elf64(data: &[u8]) -> Result<&Elf64Ehdr, ElfError> {
    if data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err(ElfError::TooSmall);
    }
    // SAFETY: bounds checked immediately above; layout is repr(C); the
    // returned shared reference lives only as long as `data`.
    let ehdr = unsafe { &*(data.as_ptr() as *const Elf64Ehdr) };

    if ehdr.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if ehdr.e_ident[4] != ELFCLASS64 {
        return Err(ElfError::Not64Bit);
    }
    if ehdr.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::NotLittleEndian);
    }
    if ehdr.e_machine != EM_X86_64 {
        return Err(ElfError::NotX86_64);
    }
    if ehdr.e_type != ET_EXEC && ehdr.e_type != ET_DYN {
        return Err(ElfError::NotExecutable);
    }
    Ok(ehdr)
}

pub fn program_headers<'a>(data: &'a [u8], ehdr: &Elf64Ehdr) -> Result<&'a [Elf64Phdr], ElfError> {
    let off = ehdr.e_phoff as usize;
    let num = ehdr.e_phnum as usize;
    let entry_size = ehdr.e_phentsize as usize;
    let total = off + num * entry_size;

    if total > data.len() || entry_size < core::mem::size_of::<Elf64Phdr>() {
        return Err(ElfError::BadPhdr);
    }
    // SAFETY: `total <= data.len()`; `entry_size >= size_of::<Elf64Phdr>()`;
    // layout is repr(C); slice lifetime is bounded by `data`.
    Ok(unsafe { core::slice::from_raw_parts(data.as_ptr().add(off) as *const Elf64Phdr, num) })
}

/// Pick a HAL `PageFlags` preset for an ELF segment.
///
/// The HAL exposes presets only; we don't have an arbitrary "USER + W + X"
/// option. ELF segments with both PF_W and PF_X are nonconformant (modern
/// linkers split them); we promote them to `USER_CODE` (executable, USER, no
/// W). The kernel writes the segment image into the physical pages before
/// mapping, so initial contents are still set up correctly even without W.
fn elf_flags_to_preset(p_flags: u32) -> PageFlags {
    let writable = p_flags & PF_W != 0;
    let executable = p_flags & PF_X != 0;
    match (writable, executable) {
        (false, false) => PageFlags::USER_RO,
        (true, false) => PageFlags::USER_RW,
        (false, true) => PageFlags::USER_CODE,
        // Misconfigured W+X segment — defer to USER_CODE (executable, USER).
        (true, true) => PageFlags::USER_CODE,
    }
}

/// Load an ELF64 image into a fresh PML4, cloning kernel mappings so
/// interrupts/syscalls keep working in the new address space.
///
/// Returns a heap-allocated [`LoadedElfImage`] suitable for handing straight
/// to the scheduler's spawn path.
///
/// # Safety
/// HAL must be installed (paging + phys allocator initialized). `data` must
/// remain valid for the duration of the call.
pub unsafe fn load_elf64(data: &[u8]) -> Result<Box<LoadedElfImage>, ElfError> {
    crate::serial::log_info("ELF", 300, "loading user image");

    let ehdr = validate_elf64(data)?;
    let phdrs = program_headers(data, ehdr)?;

    let paging = hal().paging();
    let phys = hal().phys();

    // Fresh PML4 + clone kernel half (entries 256-511) so the new address
    // space can take interrupts / run kernel-mode handlers.
    let pml4 = paging
        .pml4_new_empty()
        .map_err(|_| ElfError::AllocFailed)?;
    paging
        .pml4_clone_kernel_half(pml4)
        .map_err(|_| ElfError::AllocFailed)?;

    let mut segments: Vec<(u64, u64, u64)> = Vec::new();
    let mut has_load = false;

    for ph in phdrs {
        if ph.p_type != PT_LOAD || ph.p_memsz == 0 {
            continue;
        }
        has_load = true;

        let preset = elf_flags_to_preset(ph.p_flags);
        let vaddr_base = ph.p_vaddr & !0xFFF;
        let vaddr_end = (ph.p_vaddr + ph.p_memsz + 0xFFF) & !0xFFF;
        let num_pages = (vaddr_end - vaddr_base) / PAGE_SIZE;

        let phys_base = phys
            .allocate_pages(AllocKind::AnyPages, MemoryType::LoaderData, num_pages)
            .map_err(|_| ElfError::AllocFailed)?;

        // SAFETY: `phys_base` is identity-mapped per the HAL contract; the
        // region spans `num_pages * PAGE_SIZE` bytes; the allocator just
        // handed it to us, so no concurrent access.
        core::ptr::write_bytes(phys_base as *mut u8, 0, (num_pages * PAGE_SIZE) as usize);
        if ph.p_filesz > 0 {
            let file_off = ph.p_offset as usize;
            let file_end = file_off + ph.p_filesz as usize;
            if file_end > data.len() {
                return Err(ElfError::BadPhdr);
            }
            let vaddr_off = (ph.p_vaddr - vaddr_base) as usize;
            // SAFETY: source range bounds-checked above; destination is the
            // freshly-allocated identity-mapped region; `vaddr_off +
            // p_filesz <= num_pages * PAGE_SIZE` because
            // `vaddr_end >= p_vaddr + p_filesz`.
            core::ptr::copy_nonoverlapping(
                data[file_off..].as_ptr(),
                (phys_base as *mut u8).add(vaddr_off),
                ph.p_filesz as usize,
            );
        }

        for i in 0..num_pages {
            let virt = vaddr_base + i * PAGE_SIZE;
            let phys_pg = phys_base + i * PAGE_SIZE;
            paging
                .pml4_map_user_4k(pml4, virt, phys_pg, preset)
                .map_err(|_| ElfError::MapFailed)?;
        }

        segments.push((vaddr_base, phys_base, num_pages * PAGE_SIZE));
    }

    if !has_load {
        crate::serial::log_error("ELF", 404, "no PT_LOAD segments in image");
        return Err(ElfError::NoLoadSegments);
    }

    // User stack — fixed VA at top of the user half.
    let stack_phys = phys
        .allocate_pages(
            AllocKind::AnyPages,
            MemoryType::AllocatedStack,
            USER_STACK_PAGES,
        )
        .map_err(|_| ElfError::AllocFailed)?;
    // SAFETY: identity-mapped; size = `USER_STACK_SIZE`; freshly allocated.
    core::ptr::write_bytes(stack_phys as *mut u8, 0, USER_STACK_SIZE as usize);

    let stack_bottom = USER_STACK_TOP - USER_STACK_SIZE;
    for i in 0..USER_STACK_PAGES {
        let virt = stack_bottom + i * PAGE_SIZE;
        let phys_pg = stack_phys + i * PAGE_SIZE;
        paging
            .pml4_map_user_4k(pml4, virt, phys_pg, PageFlags::USER_RW)
            .map_err(|_| ElfError::MapFailed)?;
    }

    segments.push((stack_bottom, stack_phys, USER_STACK_SIZE));

    crate::serial::log_ok("ELF", 301, "user image mapped successfully");

    Ok(Box::new(LoadedElfImage {
        entry: ehdr.e_entry,
        pml4_phys: pml4_phys_from_handle(pml4),
        segments,
    }))
}

/// Extract the raw phys address from a `Pml4Handle`. The kernel scheduler
/// stores PML4 phys as a `u64` (loaded into CR3 / TTBR0 at context-switch
/// time), so we unwrap the newtype here in one place.
#[inline]
fn pml4_phys_from_handle(h: Pml4Handle) -> u64 {
    h.0
}
