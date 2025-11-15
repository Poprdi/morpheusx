use alloc::vec::Vec;
use core::convert::Infallible;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;

const EFI_SUCCESS: usize = 0;
const EFI_LOADER_DATA: u32 = 4;
const EFI_HARDWARE_DEVICE_PATH_TYPE: u8 = 0x01;
const EFI_MEMORY_MAPPED_DEVICE_PATH_SUBTYPE: u8 = 0x03;
const EFI_END_DEVICE_PATH_TYPE: u8 = 0x7f;
const EFI_END_ENTIRE_DEVICE_PATH_SUBTYPE: u8 = 0xff;

const EFI_LOADED_IMAGE_PROTOCOL_GUID: [u8; 16] = [
    0xa1, 0x31, 0x1b, 0x5b,
    0x62, 0x95,
    0xd2, 0x11,
    0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

#[repr(C)]
struct EfiDevicePath {
    r#type: u8,
    sub_type: u8,
    length: [u8; 2],
}

impl EfiDevicePath {
    const fn new(r#type: u8, sub_type: u8, len: u16) -> Self {
        Self {
            r#type,
            sub_type,
            length: len.to_le_bytes(),
        }
    }
}

#[repr(C)]
struct MemoryMappedDevicePath {
    header: EfiDevicePath,
    memory_type: u32,
    start_address: u64,
    end_address: u64,
}

#[repr(C)]
struct EndDevicePath {
    header: EfiDevicePath,
}

#[repr(C)]
struct KernelDevicePath {
    mem: MemoryMappedDevicePath,
    end: EndDevicePath,
}

impl KernelDevicePath {
    fn new(start: u64, end: u64) -> Self {
        let mem = MemoryMappedDevicePath {
            header: EfiDevicePath::new(
                EFI_HARDWARE_DEVICE_PATH_TYPE,
                EFI_MEMORY_MAPPED_DEVICE_PATH_SUBTYPE,
                size_of::<MemoryMappedDevicePath>() as u16,
            ),
            memory_type: EFI_LOADER_DATA,
            start_address: start,
            end_address: end,
        };

        let end = EndDevicePath {
            header: EfiDevicePath::new(
                EFI_END_DEVICE_PATH_TYPE,
                EFI_END_ENTIRE_DEVICE_PATH_SUBTYPE,
                size_of::<EndDevicePath>() as u16,
            ),
        };

        Self { mem, end }
    }
}

#[repr(C)]
struct LoadedImageProtocol {
    _revision: u32,
    _parent_handle: *mut (),
    _system_table: *mut (),
    _device_handle: *mut (),
    _file_path: *mut (),
    _reserved: *mut (),
    load_options_size: u32,
    load_options: *mut c_void,
    _image_base: *mut c_void,
    _image_size: u64,
    _image_code_type: u32,
    _image_data_type: u32,
    _unload: extern "efiapi" fn(image_handle: *mut ()) -> usize,
}

#[derive(Debug)]
pub enum EfiStubError {
    Unsupported,
    LoadImage(usize),
    HandleProtocol(usize),
    AllocateCommandLine(usize),
    StartImage(usize),
}

pub unsafe fn boot_via_efi_stub(
    boot_services: &crate::BootServices,
    image_handle: *mut (),
    kernel_data: &[u8],
    cmdline: &str,
) -> Result<Infallible, EfiStubError> {
    if kernel_data.len() < 2 || &kernel_data[0..2] != b"MZ" {
        return Err(EfiStubError::Unsupported);
    }

    let start = kernel_data.as_ptr() as u64;
    let end = start + kernel_data.len() as u64;
    let mut device_path = KernelDevicePath::new(start, end);

    let mut loaded_image_handle: *mut () = ptr::null_mut();
    let status = (boot_services.load_image)(
        false,
        image_handle,
        &mut device_path.mem.header as *mut _ as *const (),
        kernel_data.as_ptr() as *const c_void,
        kernel_data.len(),
        &mut loaded_image_handle,
    );
    if status != EFI_SUCCESS {
        return Err(EfiStubError::LoadImage(status));
    }

    let mut loaded_image_proto: *mut LoadedImageProtocol = ptr::null_mut();
    let status = (boot_services.handle_protocol)(
        loaded_image_handle,
        &EFI_LOADED_IMAGE_PROTOCOL_GUID,
        &mut loaded_image_proto as *mut _ as *mut *mut (),
    );
    if status != EFI_SUCCESS || loaded_image_proto.is_null() {
        (boot_services.unload_image)(loaded_image_handle);
        return Err(EfiStubError::HandleProtocol(status));
    }

    let mut options_allocation: Option<*mut u8> = None;
    if !cmdline.is_empty() {
        let mut utf16: Vec<u16> = cmdline.encode_utf16().collect();
        utf16.push(0);
        let options_len_bytes = utf16.len() * size_of::<u16>();
        let mut options_ptr: *mut u8 = ptr::null_mut();
        let status = (boot_services.allocate_pool)(2, options_len_bytes, &mut options_ptr);
        if status != EFI_SUCCESS || options_ptr.is_null() {
            (boot_services.unload_image)(loaded_image_handle);
            return Err(EfiStubError::AllocateCommandLine(status));
        }
        ptr::copy_nonoverlapping(
            utf16.as_ptr() as *const u8,
            options_ptr,
            options_len_bytes,
        );
        (*loaded_image_proto).load_options = options_ptr as *mut c_void;
        (*loaded_image_proto).load_options_size = options_len_bytes as u32;
        options_allocation = Some(options_ptr);
    } else {
        (*loaded_image_proto).load_options = ptr::null_mut();
        (*loaded_image_proto).load_options_size = 0;
    }

    let status = (boot_services.start_image)(
        loaded_image_handle,
        ptr::null_mut(),
        ptr::null_mut(),
    );

    if status == EFI_SUCCESS {
        unsafe { core::hint::unreachable_unchecked() };
    }

    if let Some(ptr) = options_allocation {
        (boot_services.free_pool)(ptr);
    }
    (boot_services.unload_image)(loaded_image_handle);
    Err(EfiStubError::StartImage(status))
}
