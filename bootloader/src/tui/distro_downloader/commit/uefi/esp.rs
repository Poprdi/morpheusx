//! ESP (EFI System Partition) LBA discovery.

use core::ffi::c_void;
use core::ptr;

/// UEFI protocol GUIDs
const EFI_LOADED_IMAGE_PROTOCOL_GUID: [u8; 16] = [
    0xa1, 0x31, 0x1b, 0x5b, 0x62, 0x95, 0xd2, 0x11, 0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

const EFI_DEVICE_PATH_PROTOCOL_GUID: [u8; 16] = [
    0x91, 0x6e, 0x57, 0x09, 0x3f, 0x6d, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

#[repr(C)]
struct LoadedImageProtocol {
    revision: u32,
    parent_handle: *mut c_void,
    system_table: *mut c_void,
    device_handle: *mut c_void,
}

#[repr(C)]
struct DevicePathHeader {
    type_: u8,
    sub_type: u8,
    length: [u8; 2],
}

/// Find ESP start LBA by querying LoadedImage -> DeviceHandle -> DevicePath.
///
/// Returns the LBA of the ESP partition start, or None if not found.
pub unsafe fn find_esp_lba(bs: &crate::BootServices, image_handle: *mut ()) -> Option<u64> {
    // 1. Get LoadedImageProtocol
    let mut loaded_image_ptr: *mut c_void = ptr::null_mut();
    let status = (bs.handle_protocol)(
        image_handle,
        &EFI_LOADED_IMAGE_PROTOCOL_GUID,
        &mut loaded_image_ptr as *mut *mut c_void as *mut *mut (),
    );
    if status != 0 {
        return None;
    }
    let loaded_image = &*(loaded_image_ptr as *const LoadedImageProtocol);

    // 2. Get DevicePathProtocol for the device handle
    let mut device_path_ptr: *mut c_void = ptr::null_mut();
    let status = (bs.handle_protocol)(
        loaded_image.device_handle as *mut _,
        &EFI_DEVICE_PATH_PROTOCOL_GUID,
        &mut device_path_ptr as *mut *mut c_void as *mut *mut (),
    );
    if status != 0 {
        return None;
    }

    // 3. Iterate device path nodes
    let mut current_ptr = device_path_ptr as *const DevicePathHeader;
    loop {
        let header = &*current_ptr;
        let type_ = header.type_;
        let sub_type = header.sub_type;
        let len = u16::from_le_bytes(header.length);

        if type_ == 0x7F && sub_type == 0xFF {
            // End of path
            break;
        }

        if type_ == 0x04 && sub_type == 0x01 {
            // Media / HardDrive
            // Structure: Header (4) + PartitionNumber (4) + PartitionStart (8) + ...
            let ptr = current_ptr as *const u8;
            let start_ptr = ptr.add(8) as *const u64;
            return Some(start_ptr.read_unaligned());
        }

        current_ptr = (current_ptr as *const u8).add(len as usize) as *const DevicePathHeader;
    }

    None
}
