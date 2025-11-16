// Memory management for kernel loading

use core::{marker::PhantomData, ptr};

use super::{KernelImage, LinuxBootParams};

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

    unsafe fn bootstrap(
        &mut self,
        boot_services: &crate::BootServices,
    ) -> Result<(), MemoryError> {
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
pub unsafe fn allocate_kernel_memory(
    boot_services: &crate::BootServices,
    kernel: &KernelImage,
) -> Result<*mut u8, MemoryError> {
    let init_size = align_up(kernel.init_size() as u64, PAGE_SIZE as u64) as usize;
    let pages = pages_from_bytes(init_size);
    let alignment = kernel.kernel_alignment().max(PAGE_SIZE as u32) as u64;

    let preferred = align_up(kernel.pref_address(), alignment);
    if kernel.is_relocatable() {
        if let Ok(ptr) = allocate_at(boot_services, preferred, pages) {
            return Ok(ptr);
        }
    } else if preferred == kernel.pref_address() {
        if let Ok(ptr) = allocate_at(boot_services, preferred, pages) {
            return Ok(ptr);
        }
        return Err(MemoryError::AllocationFailed);
    }

    let limit = if kernel.can_load_above_4g() {
        u64::MAX
    } else {
        LOW_MEMORY_MAX
    };

    match allocate_pages_max(boot_services, limit, pages) {
        Ok(ptr) => Ok(ptr),
        Err(err) => {
            if kernel.is_relocatable() {
                allocate_pages_any(boot_services, pages).or(Err(err))
            } else {
                Err(err)
            }
        }
    }
}

// Allocate memory for boot params (zero page)
pub unsafe fn allocate_boot_params(
    boot_services: &crate::BootServices,
) -> Result<*mut LinuxBootParams, MemoryError> {
    let size = core::mem::size_of::<LinuxBootParams>();
    let ptr = allocate_pages_max(boot_services, LOW_MEMORY_MAX, pages_from_bytes(size))?;
    ptr::write_bytes(ptr, 0, size);
    Ok(ptr as *mut LinuxBootParams)
}

// Allocate memory for command line string
pub unsafe fn allocate_cmdline(
    boot_services: &crate::BootServices,
    cmdline: &str,
) -> Result<*mut u8, MemoryError> {
    let size = cmdline.len() + 1;
    let ptr = allocate_pages_max(boot_services, LOW_MEMORY_MAX, pages_from_bytes(size))?;
    ptr::copy_nonoverlapping(cmdline.as_ptr(), ptr, cmdline.len());
    *ptr.add(cmdline.len()) = 0;
    Ok(ptr)
}

pub unsafe fn allocate_low_buffer(
    boot_services: &crate::BootServices,
    max_address: u64,
    size: usize,
) -> Result<*mut u8, MemoryError> {
    allocate_buffer_in_range(boot_services, INITRD_MIN_ADDR, max_address, size)
}

unsafe fn allocate_buffer_in_range(
    boot_services: &crate::BootServices,
    min_address: u64,
    max_address: u64,
    size: usize,
) -> Result<*mut u8, MemoryError> {
    if size == 0 || max_address < min_address {
        return Err(MemoryError::InvalidAddress);
    }

    let pages = pages_from_bytes(size);
    let total_bytes = (pages as u64)
        .checked_mul(PAGE_SIZE as u64)
        .ok_or(MemoryError::AllocationFailed)?;

    let span = max_address
        .saturating_sub(min_address)
        .saturating_add(1);
    if span < total_bytes {
        return Err(MemoryError::AllocationFailed);
    }

    let max_start = max_address
        .checked_add(1)
        .and_then(|limit| limit.checked_sub(total_bytes))
        .ok_or(MemoryError::AllocationFailed)?;

    let mut candidate = align_down(max_start, PAGE_SIZE as u64);
    let step = PAGE_SIZE as u64;

    while candidate >= min_address {
        match allocate_at(boot_services, candidate, pages) {
            Ok(ptr) => return Ok(ptr),
            Err(_) => {
                if candidate < min_address + step {
                    break;
                }
                candidate = candidate.saturating_sub(step);
            }
        }
    }

    Err(MemoryError::AllocationFailed)
}

// Load kernel image into allocated memory
pub unsafe fn load_kernel_image(
    kernel: &KernelImage,
    dest: *mut u8,
) -> Result<(), MemoryError> {
    let kernel_data = core::slice::from_raw_parts(
        kernel.kernel_base(),
        kernel.kernel_size(),
    );

    ptr::copy_nonoverlapping(kernel_data.as_ptr(), dest, kernel_data.len());

    let init_size = kernel.init_size() as usize;
    if init_size > kernel_data.len() {
        ptr::write_bytes(dest.add(kernel_data.len()), 0, init_size - kernel_data.len());
    }

    Ok(())
}

pub unsafe fn exit_boot_services(
    boot_services: &crate::BootServices,
    image_handle: *mut (),
    map: &mut MemoryMap,
) -> Result<(), MemoryError> {
    loop {
        map.ensure_snapshot(boot_services)?;

        let status = (boot_services.exit_boot_services)(
            image_handle,
            map.map_key,
        );

        if status == EFI_SUCCESS {
            return Ok(());
        }

        if status == EFI_BUFFER_TOO_SMALL || status == EFI_INVALID_PARAMETER {
            continue;
        }

        return Err(MemoryError::ExitBootServicesFailed(status));
    }
}

unsafe fn allocate_at(
    boot_services: &crate::BootServices,
    address: u64,
    pages: usize,
) -> Result<*mut u8, MemoryError> {
    let mut addr = address;
    let status = (boot_services.allocate_pages)(
        EFI_ALLOCATE_ADDRESS,
        EFI_LOADER_DATA,
        pages,
        &mut addr,
    );

    if status == EFI_SUCCESS {
        Ok(addr as *mut u8)
    } else {
        Err(MemoryError::AllocationFailed)
    }
}

unsafe fn allocate_pages_max(
    boot_services: &crate::BootServices,
    max_address: u64,
    pages: usize,
) -> Result<*mut u8, MemoryError> {
    let mut addr = max_address;
    let status = (boot_services.allocate_pages)(
        EFI_ALLOCATE_MAX_ADDRESS,
        EFI_LOADER_DATA,
        pages,
        &mut addr,
    );

    if status == EFI_SUCCESS {
        Ok(addr as *mut u8)
    } else {
        Err(MemoryError::AllocationFailed)
    }
}

unsafe fn allocate_pages_any(
    boot_services: &crate::BootServices,
    pages: usize,
) -> Result<*mut u8, MemoryError> {
    let mut addr = 0u64;
    let status = (boot_services.allocate_pages)(
        EFI_ALLOCATE_ANY_PAGES,
        EFI_LOADER_DATA,
        pages,
        &mut addr,
    );

    if status == EFI_SUCCESS {
        Ok(addr as *mut u8)
    } else {
        Err(MemoryError::AllocationFailed)
    }
}

fn pages_from_bytes(bytes: usize) -> usize {
    ((bytes + PAGE_SIZE - 1) / PAGE_SIZE).max(1)
}

fn align_up(value: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        return value;
    }
    (value + alignment - 1) & !(alignment - 1)
}

fn align_down(value: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        return value;
    }
    value & !(alignment - 1)
}
