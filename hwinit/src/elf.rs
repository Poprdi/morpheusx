//! ELF64 parser and user-process loader for x86-64.

extern crate alloc;
use crate::memory::{global_registry_mut, AllocateType, MemoryType, PAGE_SIZE};
use crate::paging::entry::PageFlags;
use crate::paging::table::PageTableManager;
use alloc::vec::Vec;

// ── ELF64 constants ──────────────────────────────────────────────────

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

pub const USER_STACK_PAGES: u64 = 8;
pub const USER_STACK_SIZE: u64 = USER_STACK_PAGES * PAGE_SIZE;
pub const USER_STACK_TOP: u64 = 0x0000_007F_FFFF_F000;

// ── ELF64 structures ─────────────────────────────────────────────────

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

// ── Error type ───────────────────────────────────────────────────────

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

// ── Parsed segment ───────────────────────────────────────────────────

pub struct LoadedSegment {
    pub vaddr: u64,
    pub phys: u64,
    pub memsz: u64,
    pub flags: PageFlags,
}

pub struct ElfImage {
    pub entry: u64,
    pub segments: Vec<LoadedSegment>,
}

// ── Parsing ──────────────────────────────────────────────────────────

pub fn validate_elf64(data: &[u8]) -> Result<&Elf64Ehdr, ElfError> {
    if data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err(ElfError::TooSmall);
    }
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
    Ok(unsafe { core::slice::from_raw_parts(data.as_ptr().add(off) as *const Elf64Phdr, num) })
}

fn elf_flags_to_page_flags(p_flags: u32) -> PageFlags {
    let mut f = PageFlags::PRESENT.with(PageFlags::USER);
    if p_flags & PF_W != 0 {
        f = f.with(PageFlags::WRITABLE);
    }
    if p_flags & PF_X == 0 {
        f = f.with(PageFlags::NO_EXECUTE);
    }
    f
}

// ── Loading into a new address space ──────────────────────────────────

/// Load an ELF64 binary into a fresh page table.
///
/// Returns the loaded image metadata and the new PageTableManager.
/// The kernel PML4 entries are cloned so interrupts/syscalls work.
///
/// # Safety
/// MemoryRegistry and paging must be initialized.
pub unsafe fn load_elf64(data: &[u8]) -> Result<(ElfImage, PageTableManager), ElfError> {
    let ehdr = validate_elf64(data)?;
    let phdrs = program_headers(data, ehdr)?;

    let mut pt = PageTableManager::new_empty().map_err(|_| ElfError::AllocFailed)?;
    clone_kernel_mappings(&mut pt)?;

    let mut segments = Vec::new();
    let mut has_load = false;

    for ph in phdrs {
        if ph.p_type != PT_LOAD || ph.p_memsz == 0 {
            continue;
        }
        has_load = true;

        let page_flags = elf_flags_to_page_flags(ph.p_flags);
        let vaddr_base = ph.p_vaddr & !0xFFF;
        let vaddr_end = (ph.p_vaddr + ph.p_memsz + 0xFFF) & !0xFFF;
        let num_pages = (vaddr_end - vaddr_base) / PAGE_SIZE;

        let phys_base = global_registry_mut()
            .allocate_pages(AllocateType::AnyPages, MemoryType::LoaderData, num_pages)
            .map_err(|_| ElfError::AllocFailed)?;

        // Zero the region, then copy file data.
        core::ptr::write_bytes(phys_base as *mut u8, 0, (num_pages * PAGE_SIZE) as usize);
        if ph.p_filesz > 0 {
            let file_off = ph.p_offset as usize;
            let file_end = file_off + ph.p_filesz as usize;
            if file_end > data.len() {
                return Err(ElfError::BadPhdr);
            }
            let vaddr_off = (ph.p_vaddr - vaddr_base) as usize;
            core::ptr::copy_nonoverlapping(
                data[file_off..].as_ptr(),
                (phys_base as *mut u8).add(vaddr_off),
                ph.p_filesz as usize,
            );
        }

        // Map each page with USER flag through all paging levels.
        for i in 0..num_pages {
            let virt = vaddr_base + i * PAGE_SIZE;
            let phys = phys_base + i * PAGE_SIZE;
            map_user_page(&mut pt, virt, phys, page_flags)?;
        }

        segments.push(LoadedSegment {
            vaddr: vaddr_base,
            phys: phys_base,
            memsz: num_pages * PAGE_SIZE,
            flags: page_flags,
        });
    }

    if !has_load {
        return Err(ElfError::NoLoadSegments);
    }

    // Allocate and map user stack.
    let stack_phys = global_registry_mut()
        .allocate_pages(
            AllocateType::AnyPages,
            MemoryType::AllocatedStack,
            USER_STACK_PAGES,
        )
        .map_err(|_| ElfError::AllocFailed)?;
    core::ptr::write_bytes(stack_phys as *mut u8, 0, USER_STACK_SIZE as usize);

    let stack_bottom = USER_STACK_TOP - USER_STACK_SIZE;
    for i in 0..USER_STACK_PAGES {
        let virt = stack_bottom + i * PAGE_SIZE;
        let phys = stack_phys + i * PAGE_SIZE;
        map_user_page(&mut pt, virt, phys, PageFlags::USER_RW)?;
    }

    segments.push(LoadedSegment {
        vaddr: stack_bottom,
        phys: stack_phys,
        memsz: USER_STACK_SIZE,
        flags: PageFlags::USER_RW,
    });

    let image = ElfImage {
        entry: ehdr.e_entry,
        segments,
    };
    Ok((image, pt))
}

/// Copy all 512 PML4 entries from the kernel page table into `dst`.
/// This gives the user process visibility of kernel memory (without USER bit),
/// ensuring interrupts and syscalls work in the user address space.
unsafe fn clone_kernel_mappings(dst: &mut PageTableManager) -> Result<(), ElfError> {
    let kernel_pt = crate::paging::kernel_page_table();
    let src_pml4 = kernel_pt.pml4_phys as *const crate::paging::entry::PageTable;
    let dst_pml4 = dst.pml4_phys as *mut crate::paging::entry::PageTable;

    core::ptr::copy_nonoverlapping(
        (*src_pml4).entries.as_ptr(),
        (*dst_pml4).entries.as_mut_ptr(),
        512,
    );
    Ok(())
}

/// Map a single 4 KiB user page, ensuring all intermediate paging levels
/// have the USER bit set (required by x86-64 for Ring 3 access).
unsafe fn map_user_page(
    pt: &mut PageTableManager,
    virt: u64,
    phys: u64,
    flags: PageFlags,
) -> Result<(), ElfError> {
    use crate::paging::entry::{PageTable, PageTableEntry};
    use crate::paging::table::VirtAddr;

    let va = VirtAddr::from_u64(virt);
    let inter = PageFlags::PRESENT
        .with(PageFlags::WRITABLE)
        .with(PageFlags::USER);

    let pml4 = pt.pml4_phys as *mut PageTable;

    let pdpt_phys = ensure_user_table((*pml4).entry_mut(va.pml4_idx), inter)?;
    let pdpt = pdpt_phys as *mut PageTable;

    let pd_phys = ensure_user_table((*pdpt).entry_mut(va.pdpt_idx), inter)?;
    let pd = pd_phys as *mut PageTable;

    let pt_phys = ensure_user_table((*pd).entry_mut(va.pd_idx), inter)?;
    let page_table = pt_phys as *mut PageTable;

    *(*page_table).entry_mut(va.pt_idx) = PageTableEntry::new(phys, flags.with(PageFlags::PRESENT));

    PageTableManager::flush_tlb_page(virt);
    Ok(())
}

/// Like `ensure_table` but upgrades existing entries with the USER bit
/// so intermediate levels are accessible from Ring 3.
unsafe fn ensure_user_table(
    e: &mut crate::paging::entry::PageTableEntry,
    flags: PageFlags,
) -> Result<u64, ElfError> {
    use crate::memory::{global_registry_mut, AllocateType, MemoryType};
    use crate::paging::entry::PageTableEntry;

    if e.is_present() {
        if e.is_huge() {
            return Err(ElfError::MapFailed);
        }
        // Upgrade: ensure USER + WRITABLE are set on the intermediate entry.
        let raw = e.raw();
        let needed = flags.0;
        if raw & needed != needed {
            e.set_raw(raw | needed);
        }
        return Ok(e.phys_addr());
    }

    // Allocate a fresh zeroed page table.
    let phys = global_registry_mut()
        .allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedPageTable, 1)
        .map_err(|_| ElfError::AllocFailed)?;
    let table = phys as *mut crate::paging::entry::PageTable;
    (*table).zero();

    *e = PageTableEntry::new(phys, flags);
    Ok(phys)
}
