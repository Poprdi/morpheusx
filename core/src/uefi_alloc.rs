//! Pre-EBS page allocation via UEFI BootServices.

pub type AllocatePages = extern "efiapi" fn(
    allocate_type: usize,
    memory_type: usize,
    pages: usize,
    address: *mut u64,
) -> usize;

pub type FreePages = extern "efiapi" fn(memory: u64, pages: usize) -> usize;

pub const EFI_SUCCESS: usize = 0;
pub const EFI_LOADER_DATA: usize = 2;
pub const EFI_ALLOCATE_ANY_PAGES: usize = 0;

/// SAFETY: `allocate_pages` must be the live BootServices fn; pages > 0;
/// caller must free before ExitBootServices.
pub unsafe fn allocate_pages(allocate_pages: AllocatePages, pages: usize) -> Result<u64, usize> {
    let mut addr = 0u64;
    let status = allocate_pages(EFI_ALLOCATE_ANY_PAGES, EFI_LOADER_DATA, pages, &mut addr);
    if status == EFI_SUCCESS {
        Ok(addr)
    } else {
        Err(status)
    }
}

/// SAFETY: `addr`/`pages` must match a prior `allocate_pages`;
/// call before ExitBootServices.
pub unsafe fn free_pages(free_pages: FreePages, addr: u64, pages: usize) -> Result<(), usize> {
    let status = free_pages(addr, pages);
    if status == EFI_SUCCESS {
        Ok(())
    } else {
        Err(status)
    }
}

#[inline]
pub fn bytes_to_pages(bytes: usize) -> usize {
    (bytes + 4095) / 4096
}

/// Scoped UEFI page allocation; frees on drop.
#[allow(dead_code)]
pub struct UefiBuffer {
    addr: u64,
    pages: usize,
    free_pages: FreePages,
}

impl UefiBuffer {
    /// SAFETY: function pointers must be valid BootServices entries; pages > 0.
    pub unsafe fn new(
        allocate_pages: AllocatePages,
        free_pages: FreePages,
        pages: usize,
    ) -> Result<Self, usize> {
        let mut real_addr = 0u64;
        let status = allocate_pages(
            EFI_ALLOCATE_ANY_PAGES,
            EFI_LOADER_DATA,
            pages,
            &mut real_addr,
        );
        if status == EFI_SUCCESS {
            Ok(UefiBuffer {
                addr: real_addr,
                pages,
                free_pages,
            })
        } else {
            Err(status)
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.addr as *mut u8, self.pages * 4096) }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.addr as *const u8, self.pages * 4096) }
    }
}

impl Drop for UefiBuffer {
    fn drop(&mut self) {
        let _ = (self.free_pages)(self.addr, self.pages);
    }
}
