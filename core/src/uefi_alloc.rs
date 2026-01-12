//! UEFI Allocation helpers for pre-EBS memory management
//!
//! Provides utilities to allocate and free memory using UEFI BootServices
//! allocate_pages. This is the correct way to allocate memory pre-EBS when
//! the global heap allocator is not initialized.

/// UEFI BootServices allocate_pages signature
pub type AllocatePages = extern "efiapi" fn(
    allocate_type: usize,
    memory_type: usize,
    pages: usize,
    address: *mut u64,
) -> usize;

/// UEFI BootServices free_pages signature
pub type FreePages = extern "efiapi" fn(memory: u64, pages: usize) -> usize;

/// EFI_SUCCESS
pub const EFI_SUCCESS: usize = 0;

/// EFI_LOADER_DATA memory type (suitable for temporary allocations)
pub const EFI_LOADER_DATA: usize = 2;

/// EFI_ALLOCATE_ANY_PAGES - allocate from anywhere in memory
pub const EFI_ALLOCATE_ANY_PAGES: usize = 0;

/// Allocate memory pages from UEFI
///
/// # Safety
/// - allocate_pages must be a valid function pointer from BootServices
/// - pages must be > 0
/// - returned buffer must be freed with free_uefi_pages before ExitBootServices
pub unsafe fn allocate_pages(allocate_pages: AllocatePages, pages: usize) -> Result<u64, usize> {
    let mut addr = 0u64;
    let status = allocate_pages(EFI_ALLOCATE_ANY_PAGES, EFI_LOADER_DATA, pages, &mut addr);
    if status == EFI_SUCCESS {
        Ok(addr)
    } else {
        Err(status)
    }
}

/// Free memory pages to UEFI
///
/// # Safety
/// - addr must be the result of a successful allocate_pages call
/// - pages must match the allocation size
/// - must be called before ExitBootServices
pub unsafe fn free_pages(free_pages: FreePages, addr: u64, pages: usize) -> Result<(), usize> {
    let status = free_pages(addr, pages);
    if status == EFI_SUCCESS {
        Ok(())
    } else {
        Err(status)
    }
}

/// Calculate number of pages needed for a size in bytes
#[inline]
pub fn bytes_to_pages(bytes: usize) -> usize {
    (bytes + 4095) / 4096
}

/// A scoped UEFI-allocated buffer that automatically frees on drop
#[allow(dead_code)]
pub struct UefiBuffer {
    addr: u64,
    pages: usize,
    free_pages: FreePages,
}

impl UefiBuffer {
    /// Create a new UEFI-allocated buffer
    ///
    /// # Safety
    /// - allocate_pages and free_pages must be valid function pointers
    /// - pages must be > 0
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

    /// Get the buffer as a mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.addr as *mut u8, self.pages * 4096) }
    }

    /// Get the buffer as a slice
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.addr as *const u8, self.pages * 4096) }
    }
}

impl Drop for UefiBuffer {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.free_pages)(self.addr, self.pages);
        }
    }
}
