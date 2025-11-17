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

    let span = max_address.saturating_sub(min_address).saturating_add(1);
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
pub unsafe fn load_kernel_image(kernel: &KernelImage, dest: *mut u8) -> Result<(), MemoryError> {
    let kernel_data = core::slice::from_raw_parts(kernel.kernel_base(), kernel.kernel_size());

    ptr::copy_nonoverlapping(kernel_data.as_ptr(), dest, kernel_data.len());

    let init_size = kernel.init_size() as usize;
    if init_size > kernel_data.len() {
        ptr::write_bytes(
            dest.add(kernel_data.len()),
            0,
            init_size - kernel_data.len(),
        );
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

        let status = (boot_services.exit_boot_services)(image_handle, map.map_key);

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
    let status =
        (boot_services.allocate_pages)(EFI_ALLOCATE_ADDRESS, EFI_LOADER_DATA, pages, &mut addr);

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
    let status =
        (boot_services.allocate_pages)(EFI_ALLOCATE_MAX_ADDRESS, EFI_LOADER_DATA, pages, &mut addr);

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
    let status =
        (boot_services.allocate_pages)(EFI_ALLOCATE_ANY_PAGES, EFI_LOADER_DATA, pages, &mut addr);

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
