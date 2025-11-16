// UEFI-specific disk operations

use super::block_io::{BlockIoProtocol, EFI_BLOCK_IO_PROTOCOL_GUID};
use crate::BootServices;
use morpheus_core::disk::manager::{DiskInfo, DiskManager};

/// Enumerate all physical disks in the system
pub fn enumerate_disks(bs: &BootServices, manager: &mut DiskManager) -> Result<(), usize> {
    manager.clear();

    // Get buffer size needed for all Block I/O handles
    let mut buffer_size: usize = 0;
    let _ = (bs.locate_handle)(
        2, // ByProtocol
        &EFI_BLOCK_IO_PROTOCOL_GUID,
        core::ptr::null(),
        &mut buffer_size,
        core::ptr::null_mut(),
    );

    if buffer_size == 0 {
        return Err(1); // No devices found
    }

    // Allocate buffer for handles
    let mut handle_buffer: *mut u8 = core::ptr::null_mut();
    let alloc_status = (bs.allocate_pool)(2, buffer_size, &mut handle_buffer);

    if alloc_status != 0 {
        return Err(alloc_status);
    }

    // Get all Block I/O handles
    let status = (bs.locate_handle)(
        2,
        &EFI_BLOCK_IO_PROTOCOL_GUID,
        core::ptr::null(),
        &mut buffer_size,
        handle_buffer as *mut *mut (),
    );

    if status != 0 {
        (bs.free_pool)(handle_buffer);
        return Err(status);
    }

    // Iterate through handles and find physical disks
    let handles = handle_buffer as *const *mut ();
    let handle_count = buffer_size / core::mem::size_of::<*mut ()>();

    for i in 0..handle_count {
        let handle = unsafe { *handles.add(i) };

        let mut block_io_ptr: *mut () = core::ptr::null_mut();
        let proto_status =
            (bs.handle_protocol)(handle, &EFI_BLOCK_IO_PROTOCOL_GUID, &mut block_io_ptr);

        if proto_status == 0 {
            let block_io = unsafe { &*(block_io_ptr as *const BlockIoProtocol) };
            let media = unsafe { &*block_io.media };

            // Only add physical disks (not partitions)
            if !media.logical_partition && media.media_present {
                let disk_info = DiskInfo::new(
                    media.media_id,
                    media.block_size,
                    media.last_block,
                    media.removable_media,
                    media.read_only,
                );

                let _ = manager.add_disk(disk_info);
            }
        }
    }

    (bs.free_pool)(handle_buffer);
    Ok(())
}

/// Get Block I/O protocol for a physical disk by index
pub fn get_disk_protocol(
    bs: &BootServices,
    disk_index: usize,
) -> Result<*mut BlockIoProtocol, usize> {
    // Get buffer size
    let mut buffer_size: usize = 0;
    let _ = (bs.locate_handle)(
        2,
        &EFI_BLOCK_IO_PROTOCOL_GUID,
        core::ptr::null(),
        &mut buffer_size,
        core::ptr::null_mut(),
    );

    if buffer_size == 0 {
        return Err(1);
    }

    let mut handle_buffer: *mut u8 = core::ptr::null_mut();
    let alloc_status = (bs.allocate_pool)(2, buffer_size, &mut handle_buffer);

    if alloc_status != 0 {
        return Err(alloc_status);
    }

    let status = (bs.locate_handle)(
        2,
        &EFI_BLOCK_IO_PROTOCOL_GUID,
        core::ptr::null(),
        &mut buffer_size,
        handle_buffer as *mut *mut (),
    );

    if status != 0 {
        (bs.free_pool)(handle_buffer);
        return Err(status);
    }

    // Find the Nth physical disk
    let handles = handle_buffer as *const *mut ();
    let handle_count = buffer_size / core::mem::size_of::<*mut ()>();
    let mut physical_disk_count = 0;
    let mut result: Option<*mut BlockIoProtocol> = None;

    for i in 0..handle_count {
        let handle = unsafe { *handles.add(i) };

        let mut block_io_ptr: *mut () = core::ptr::null_mut();
        let proto_status =
            (bs.handle_protocol)(handle, &EFI_BLOCK_IO_PROTOCOL_GUID, &mut block_io_ptr);

        if proto_status == 0 {
            let block_io = unsafe { &*(block_io_ptr as *const BlockIoProtocol) };
            let media = unsafe { &*block_io.media };

            if !media.logical_partition && media.media_present {
                if physical_disk_count == disk_index {
                    result = Some(block_io_ptr as *mut BlockIoProtocol);
                    break;
                }
                physical_disk_count += 1;
            }
        }
    }

    (bs.free_pool)(handle_buffer);

    match result {
        Some(ptr) => Ok(ptr),
        None => Err(2), // Not found
    }
}

/// Get disk handle (needed for file system protocol)
pub fn get_disk_handle(bs: &BootServices, disk_index: usize) -> Result<*mut (), usize> {
    // Get buffer size needed
    let mut buffer_size: usize = 0;
    let _ = (bs.locate_handle)(
        2, // ByProtocol
        &EFI_BLOCK_IO_PROTOCOL_GUID,
        core::ptr::null(),
        &mut buffer_size,
        core::ptr::null_mut(),
    );

    if buffer_size == 0 {
        return Err(1);
    }

    // Allocate buffer
    let mut handle_buffer: *mut u8 = core::ptr::null_mut();
    let alloc_status = (bs.allocate_pool)(2, buffer_size, &mut handle_buffer);

    if alloc_status != 0 {
        return Err(alloc_status);
    }

    // Get all handles
    let status = (bs.locate_handle)(
        2,
        &EFI_BLOCK_IO_PROTOCOL_GUID,
        core::ptr::null(),
        &mut buffer_size,
        handle_buffer as *mut *mut (),
    );

    if status != 0 {
        (bs.free_pool)(handle_buffer);
        return Err(status);
    }

    // Find physical disk by index
    let handles = handle_buffer as *const *mut ();
    let handle_count = buffer_size / core::mem::size_of::<*mut ()>();
    let mut physical_disk_count = 0;
    let mut result = None;

    for i in 0..handle_count {
        let handle = unsafe { *handles.add(i) };

        let mut block_io_ptr: *mut () = core::ptr::null_mut();
        let proto_status =
            (bs.handle_protocol)(handle, &EFI_BLOCK_IO_PROTOCOL_GUID, &mut block_io_ptr);

        if proto_status == 0 {
            let block_io = unsafe { &*(block_io_ptr as *const BlockIoProtocol) };
            let media = unsafe { &*block_io.media };

            if !media.logical_partition && media.media_present {
                if physical_disk_count == disk_index {
                    result = Some(handle);
                    break;
                }
                physical_disk_count += 1;
            }
        }
    }

    (bs.free_pool)(handle_buffer);

    match result {
        Some(handle) => Ok(handle),
        None => Err(2),
    }
}
