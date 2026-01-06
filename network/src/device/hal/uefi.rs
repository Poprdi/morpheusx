//! UEFI HAL implementation for VirtIO drivers.
//!
//! This HAL uses UEFI Boot Services for memory allocation. It must be used
//! **before** calling `ExitBootServices()`.
//!
//! # Memory Model
//!
//! In UEFI, memory is identity-mapped (physical == virtual address).
//! We use `AllocatePages` with `EfiBootServicesData` memory type for DMA.
//!
//! # Thread Safety
//!
//! UEFI is single-threaded, so no synchronization is needed.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::hal::UefiHal;
//! use virtio_drivers::device::net::VirtIONetRaw;
//!
//! // Initialize UEFI HAL with boot services pointer
//! unsafe { UefiHal::init(boot_services_ptr) };
//!
//! // Now VirtIO drivers can be used
//! let net = VirtIONetRaw::<UefiHal, _>::new(transport)?;
//! ```

use core::ptr::NonNull;
use core::sync::atomic::{AtomicPtr, Ordering};
use virtio_drivers::{BufferDirection, Hal, PhysAddr, PAGE_SIZE};

use super::common;

/// UEFI memory type for Boot Services Data.
const EFI_BOOT_SERVICES_DATA: u32 = 4;

/// UEFI memory allocation type: allocate any available pages.
const ALLOCATE_ANY_PAGES: u32 = 0;

/// UEFI status codes.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EfiStatus {
    Success = 0,
    // Add other status codes as needed
}

/// UEFI Boot Services table (partial definition for memory allocation).
///
/// We only define the fields we need for memory allocation.
#[repr(C)]
pub struct EfiBootServices {
    pub hdr: EfiTableHeader,

    // Task Priority Services (2 entries)
    _raise_tpl: usize,
    _restore_tpl: usize,

    // Memory Services (5 entries)
    pub allocate_pages: unsafe extern "efiapi" fn(
        alloc_type: u32,
        memory_type: u32,
        pages: usize,
        memory: *mut PhysAddr,
    ) -> usize,
    pub free_pages: unsafe extern "efiapi" fn(memory: PhysAddr, pages: usize) -> usize,
    _get_memory_map: usize,
    _allocate_pool: usize,
    _free_pool: usize,

    // ... other services omitted
}

/// EFI Table Header (common to all UEFI tables).
#[repr(C)]
pub struct EfiTableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

/// Global pointer to UEFI Boot Services.
///
/// Must be initialized via `UefiHal::init()` before use.
static BOOT_SERVICES: AtomicPtr<EfiBootServices> = AtomicPtr::new(core::ptr::null_mut());

/// UEFI HAL implementation.
///
/// Uses UEFI Boot Services for DMA memory allocation.
/// Must be initialized with `init()` before first use.
pub struct UefiHal;

impl UefiHal {
    /// Initialize the UEFI HAL with a pointer to Boot Services.
    ///
    /// # Safety
    ///
    /// - `boot_services` must be a valid pointer to UEFI Boot Services.
    /// - Must be called before any VirtIO operations.
    /// - Must not be called after `ExitBootServices()`.
    pub unsafe fn init(boot_services: *mut EfiBootServices) {
        BOOT_SERVICES.store(boot_services, Ordering::SeqCst);
    }

    /// Check if the HAL has been initialized.
    pub fn is_initialized() -> bool {
        !BOOT_SERVICES.load(Ordering::SeqCst).is_null()
    }

    /// Get the Boot Services pointer.
    ///
    /// # Panics
    ///
    /// Panics if `init()` has not been called.
    fn boot_services() -> &'static EfiBootServices {
        let ptr = BOOT_SERVICES.load(Ordering::SeqCst);
        if ptr.is_null() {
            panic!("UefiHal: Boot Services not initialized. Call UefiHal::init() first.");
        }
        // SAFETY: Pointer was validated in init()
        unsafe { &*ptr }
    }

    /// Pre-allocate a DMA pool for use after ExitBootServices.
    ///
    /// Call this before ExitBootServices to reserve memory that will be
    /// available for the BareHal after boot services are no longer accessible.
    ///
    /// Returns (physical_address, virtual_address, size_in_bytes).
    ///
    /// # Safety
    ///
    /// Must be called before ExitBootServices.
    #[cfg(feature = "bare")]
    pub unsafe fn preallocate_dma_pool(
        pages: usize,
    ) -> Result<(PhysAddr, NonNull<u8>, usize), usize> {
        let bs = Self::boot_services();
        let mut paddr: PhysAddr = 0;

        let status = (bs.allocate_pages)(ALLOCATE_ANY_PAGES, EFI_BOOT_SERVICES_DATA, pages, &mut paddr);

        if status != EfiStatus::Success as usize {
            return Err(status);
        }

        let vaddr = NonNull::new(paddr as *mut u8).ok_or(status)?;
        let size = common::pages_to_bytes(pages);

        // Zero the memory
        unsafe {
            core::ptr::write_bytes(vaddr.as_ptr(), 0, size);
        }

        Ok((paddr, vaddr, size))
    }
}

// SAFETY: We implement the Hal trait requirements correctly:
// - dma_alloc returns valid, aligned, zeroed memory
// - dma_dealloc properly frees the memory
// - Identity mapping is correct for UEFI
unsafe impl Hal for UefiHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        if pages == 0 {
            panic!("UefiHal: Cannot allocate 0 pages");
        }

        let bs = Self::boot_services();
        let mut paddr: PhysAddr = 0;

        // SAFETY: Boot services pointer is valid (checked in boot_services())
        let status = unsafe {
            (bs.allocate_pages)(ALLOCATE_ANY_PAGES, EFI_BOOT_SERVICES_DATA, pages, &mut paddr)
        };

        if status != EfiStatus::Success as usize {
            panic!("UefiHal: AllocatePages failed with status {:#x}", status);
        }

        // In UEFI, physical == virtual (identity mapping)
        let vaddr = NonNull::new(paddr as *mut u8).expect("UefiHal: AllocatePages returned null");

        // Zero the memory (required by Hal trait)
        let size = common::pages_to_bytes(pages);
        // SAFETY: Memory was just allocated and is valid
        unsafe {
            core::ptr::write_bytes(vaddr.as_ptr(), 0, size);
        }

        (paddr, vaddr)
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        let bs = Self::boot_services();

        // SAFETY: Caller guarantees paddr was from dma_alloc
        let status = unsafe { (bs.free_pages)(paddr, pages) };

        if status != EfiStatus::Success as usize {
            return status as i32;
        }

        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        // UEFI uses identity mapping
        NonNull::new(paddr as *mut u8).expect("UefiHal: null MMIO address")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        // Identity mapping: physical == virtual
        // No IOMMU in simple UEFI case
        buffer.as_ptr() as *const u8 as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // No-op for identity mapping without IOMMU
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Most tests require actual UEFI environment.
    // These are compile-time checks and basic logic tests.

    #[test]
    fn test_hal_not_initialized() {
        // Reset the global state for this test
        BOOT_SERVICES.store(core::ptr::null_mut(), Ordering::SeqCst);
        assert!(!UefiHal::is_initialized());
    }

    #[test]
    fn test_page_size_is_4k() {
        assert_eq!(PAGE_SIZE, 4096);
    }

    #[test]
    fn test_efi_status_success() {
        assert_eq!(EfiStatus::Success as usize, 0);
    }

    #[test]
    fn test_memory_type_constants() {
        assert_eq!(EFI_BOOT_SERVICES_DATA, 4);
        assert_eq!(ALLOCATE_ANY_PAGES, 0);
    }
}
