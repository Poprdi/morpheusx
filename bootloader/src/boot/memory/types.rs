// Memory management for kernel loading

use core::{marker::PhantomData, ptr};

use crate::boot::{KernelImage, LinuxBootParams};
use super::allocation::{align_up, pages_from_bytes, allocate_pages_max, allocate_pages_any};

const PAGE_SIZE: usize = 4096;
const EFI_ALLOCATE_ANY_PAGES: usize = 0;
const EFI_ALLOCATE_MAX_ADDRESS: usize = 1;
const EFI_ALLOCATE_ADDRESS: usize = 2;
const EFI_LOADER_DATA: usize = 2;
const EFI_SUCCESS: usize = 0;
const EFI_ERROR_BIT: usize = 1usize << (usize::BITS - 1);
const EFI_BUFFER_TOO_SMALL: usize = EFI_ERROR_BIT | 5;
const EFI_INVALID_PARAMETER: usize = EFI_ERROR_BIT | 2;
const LOW_MEMORY_MAX: u64 = 0x0000_FFFF_F000;
pub(crate) const INITRD_MIN_ADDR: u64 = 0x0010_0000;

#[derive(Debug)]
pub enum MemoryError {
    AllocationFailed,
    InvalidAddress,
    MapError(usize),
    ExitBootServicesFailed(usize),
}

#[repr(C)]
pub struct UefiMemoryDescriptor {
    pub typ: u32,
    _pad: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

pub struct MemoryMap {
    buffer: *mut u8,
    capacity: usize,
    pub size: usize,
    pub map_key: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
}

impl MemoryMap {
    pub const fn new() -> Self {
        Self {
            buffer: ptr::null_mut(),
            capacity: 0,
            size: 0,
            map_key: 0,
            descriptor_size: 0,
            descriptor_version: 0,
        }
    }

    pub unsafe fn ensure_snapshot(
        &mut self,
        boot_services: &crate::BootServices,
    ) -> Result<(), MemoryError> {
        if self.buffer.is_null() {
            self.bootstrap(boot_services)?;
        }

        loop {
            let mut reported_size = self.capacity;
            let status = (boot_services.get_memory_map)(
                &mut reported_size,
                self.buffer,
                &mut self.map_key,
                &mut self.descriptor_size,
                &mut self.descriptor_version,
            );

            if status == EFI_SUCCESS {
                self.size = reported_size;
                return Ok(());
            }

            if status == EFI_BUFFER_TOO_SMALL {
                let needed = align_up(
                    reported_size as u64 + (self.descriptor_size * 2) as u64,
                    PAGE_SIZE as u64,
                ) as usize;
                self.reserve(boot_services, needed)?;
                continue;
            }

            return Err(MemoryError::MapError(status));
        }
    }

    pub fn buffer_ptr(&self) -> *mut u8 {
        self.buffer
    }

    pub fn descriptors(&self) -> MemoryDescriptorIter {
        MemoryDescriptorIter {
            descriptor_size: self.descriptor_size,
            current: self.buffer as *const u8,
            remaining: self.size,
            _marker: PhantomData,
        }
    }

    unsafe fn bootstrap(&mut self, boot_services: &crate::BootServices) -> Result<(), MemoryError> {
        let mut needed = 0usize;
        let mut map_key = 0usize;
        let mut descriptor_size = 0usize;
        let mut descriptor_version = 0u32;

        let status = (boot_services.get_memory_map)(
            &mut needed,
            ptr::null_mut(),
            &mut map_key,
            &mut descriptor_size,
            &mut descriptor_version,
        );

        if status != EFI_BUFFER_TOO_SMALL {
            return Err(MemoryError::MapError(status));
        }

        self.descriptor_size = descriptor_size;
        self.descriptor_version = descriptor_version;

        let capacity = align_up(
            needed as u64 + (descriptor_size * 8) as u64,
            PAGE_SIZE as u64,
        ) as usize;
        self.reserve(boot_services, capacity)
    }

    unsafe fn reserve(
        &mut self,
        boot_services: &crate::BootServices,
        capacity: usize,
    ) -> Result<(), MemoryError> {
        let pages = pages_from_bytes(capacity);
        let buffer = match allocate_pages_max(boot_services, LOW_MEMORY_MAX, pages) {
            Ok(ptr) => ptr,
            Err(_) => allocate_pages_any(boot_services, pages)?,
        };
        self.buffer = buffer;
        self.capacity = pages * PAGE_SIZE;
        self.size = 0;
        Ok(())
    }
}

pub struct MemoryDescriptorIter<'a> {
    descriptor_size: usize,
    current: *const u8,
    remaining: usize,
    _marker: PhantomData<&'a UefiMemoryDescriptor>,
}

impl<'a> Iterator for MemoryDescriptorIter<'a> {
    type Item = &'a UefiMemoryDescriptor;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining < self.descriptor_size || self.descriptor_size == 0 {
            return None;
        }

        let descriptor = unsafe { &*(self.current as *const UefiMemoryDescriptor) };
        self.current = unsafe { self.current.add(self.descriptor_size) };
        self.remaining -= self.descriptor_size;
        Some(descriptor)
    }
}

// Allocate memory for kernel at preferred address
