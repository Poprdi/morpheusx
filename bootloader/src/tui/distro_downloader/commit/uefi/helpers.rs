//! UEFI utility functions for boot services exit.

/// Exit boot services with memory map handling.
///
/// Returns Ok(()) on success, Err(()) on failure.
/// Also switches the allocator to post-EBS mode.
pub unsafe fn exit_boot_services_with_retry(
    bs: &crate::BootServices,
    image_handle: *mut (),
) -> Result<(), ()> {
    // Disable watchdog timer BEFORE GetMemoryMap
    let _ = (bs.set_watchdog_timer)(0, 0, 0, core::ptr::null());

    let mut mmap_size: usize = 4096;
    let mut mmap_buf = [0u8; 8192];
    let mut map_key: usize = 0;
    let mut desc_size: usize = 0;
    let mut desc_version: u32 = 0;

    // First call to get required size
    let _ = (bs.get_memory_map)(
        &mut mmap_size,
        mmap_buf.as_mut_ptr(),
        &mut map_key,
        &mut desc_size,
        &mut desc_version,
    );

    // Increase buffer size to be safe
    mmap_size += 2048;

    // Second call with proper size
    let status = (bs.get_memory_map)(
        &mut mmap_size,
        mmap_buf.as_mut_ptr(),
        &mut map_key,
        &mut desc_size,
        &mut desc_version,
    );

    if status != 0 {
        return Err(());
    }

    // Exit boot services IMMEDIATELY
    let status = (bs.exit_boot_services)(image_handle, map_key);

    if status != 0 {
        // Retry once with fresh map
        let _ = (bs.get_memory_map)(
            &mut mmap_size,
            mmap_buf.as_mut_ptr(),
            &mut map_key,
            &mut desc_size,
            &mut desc_version,
        );
        let status = (bs.exit_boot_services)(image_handle, map_key);
        if status != 0 {
            return Err(());
        }
    }

    // CRITICAL: Switch allocator to post-EBS mode (uses linked_list_allocator)
    // UEFI allocate_pool is no longer available after ExitBootServices
    crate::uefi_allocator::switch_to_post_ebs();

    Ok(())
}

/// Leak a string to make it 'static.
///
/// Safe to use when exiting boot services since we won't return.
pub fn leak_string(s: &str) -> &'static str {
    let boxed = alloc::boxed::Box::new(alloc::string::String::from(s));
    alloc::boxed::Box::leak(boxed).as_str()
}
