use alloc::vec::Vec;
use core::convert::Infallible;
use core::ffi::c_void;
use core::fmt;
use core::mem::size_of;
use core::ptr;

const EFI_SUCCESS: usize = 0;
const EFI_ERROR_BIT: usize = 1usize << (size_of::<usize>() * 8 - 1);
const EFI_INVALID_PARAMETER: usize = EFI_ERROR_BIT | 2;
const EFI_BUFFER_TOO_SMALL: usize = EFI_ERROR_BIT | 5;
const EFI_NATIVE_INTERFACE: usize = 0;
const EFI_POOL_TYPE_LOADER_DATA: usize = 2;
const EFI_LOADER_DATA: u32 = 4;
const EFI_HARDWARE_DEVICE_PATH_TYPE: u8 = 0x01;
const EFI_MEDIA_DEVICE_PATH_TYPE: u8 = 0x04;
const EFI_MEMORY_MAPPED_DEVICE_PATH_SUBTYPE: u8 = 0x03;
const EFI_VENDOR_MEDIA_DEVICE_PATH_SUBTYPE: u8 = 0x03;
const EFI_END_DEVICE_PATH_TYPE: u8 = 0x7f;
const EFI_END_ENTIRE_DEVICE_PATH_SUBTYPE: u8 = 0xff;

const EFI_LOADED_IMAGE_PROTOCOL_GUID: [u8; 16] = [
    0xa1, 0x31, 0x1b, 0x5b, 0x62, 0x95, 0xd2, 0x11, 0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

const EFI_LOAD_FILE2_PROTOCOL_GUID: [u8; 16] = [
    0xc1, 0xc0, 0x06, 0x40, 0xb3, 0xfc, 0x3e, 0x40, 0x99, 0x6d, 0x4a, 0x6c, 0x87, 0x24, 0xe0, 0x6d,
];

const EFI_DEVICE_PATH_PROTOCOL_GUID: [u8; 16] = [
    0x91, 0x6e, 0x57, 0x09, 0x3f, 0x6d, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

const LINUX_EFI_INITRD_MEDIA_GUID: [u8; 16] = [
    0x27, 0xe4, 0x68, 0x55, 0xfc, 0x68, 0x3d, 0x4f, 0xac, 0x74, 0xca, 0x55, 0x52, 0x31, 0xcc, 0x68,
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

#[repr(C, packed)]
struct VendorDevicePath {
    header: EfiDevicePath,
    vendor_guid: [u8; 16],
}

#[repr(C, packed)]
struct InitrdDevicePath {
    vendor: VendorDevicePath,
    end: EndDevicePath,
}

impl InitrdDevicePath {
    const fn new() -> Self {
        Self {
            vendor: VendorDevicePath {
                header: EfiDevicePath::new(
                    EFI_MEDIA_DEVICE_PATH_TYPE,
                    EFI_VENDOR_MEDIA_DEVICE_PATH_SUBTYPE,
                    size_of::<VendorDevicePath>() as u16,
                ),
                vendor_guid: LINUX_EFI_INITRD_MEDIA_GUID,
            },
            end: EndDevicePath {
                header: EfiDevicePath::new(
                    EFI_END_DEVICE_PATH_TYPE,
                    EFI_END_ENTIRE_DEVICE_PATH_SUBTYPE,
                    size_of::<EndDevicePath>() as u16,
                ),
            },
        }
    }
}

#[repr(C)]
struct LoadFile2Protocol {
    load_file: unsafe extern "efiapi" fn(
        this: *mut LoadFile2Protocol,
        file_path: *mut c_void,
        boot_policy: bool,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> usize,
}

#[repr(C)]
struct InitrdLoadFile2 {
    protocol: LoadFile2Protocol,
    data_ptr: *const u8,
    data_len: usize,
}

struct InitrdProtocolHandles {
    handle: *mut (),
    loader: *mut InitrdLoadFile2,
    device_path: *mut InitrdDevicePath,
}

unsafe extern "efiapi" fn initrd_load_file(
    this: *mut LoadFile2Protocol,
    _file_path: *mut c_void,
    _boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut c_void,
) -> usize {
    if this.is_null() || buffer_size.is_null() {
        return EFI_INVALID_PARAMETER;
    }

    let loader = this as *mut InitrdLoadFile2;
    let required = (*loader).data_len;
    let available = *buffer_size;

    if buffer.is_null() || available < required {
        *buffer_size = required;
        return EFI_BUFFER_TOO_SMALL;
    }

    if required != 0 {
        ptr::copy_nonoverlapping((*loader).data_ptr, buffer as *mut u8, required);
    }

    *buffer_size = required;
    EFI_SUCCESS
}

unsafe fn install_initrd_protocol(
    boot_services: &crate::BootServices,
    initrd: &[u8],
) -> Result<InitrdProtocolHandles, EfiStubError> {
    let mut loader_buf: *mut u8 = ptr::null_mut();
    let status = (boot_services.allocate_pool)(
        EFI_POOL_TYPE_LOADER_DATA,
        size_of::<InitrdLoadFile2>(),
        &mut loader_buf,
    );
    if status != EFI_SUCCESS || loader_buf.is_null() {
        return Err(EfiStubError::AllocateInitrd("LoadFile2", status));
    }

    let loader = loader_buf as *mut InitrdLoadFile2;
    (*loader).protocol = LoadFile2Protocol {
        load_file: initrd_load_file,
    };
    (*loader).data_ptr = initrd.as_ptr();
    (*loader).data_len = initrd.len();

    let mut path_buf: *mut u8 = ptr::null_mut();
    let status = (boot_services.allocate_pool)(
        EFI_POOL_TYPE_LOADER_DATA,
        size_of::<InitrdDevicePath>(),
        &mut path_buf,
    );
    if status != EFI_SUCCESS || path_buf.is_null() {
        (boot_services.free_pool)(loader_buf);
        return Err(EfiStubError::AllocateInitrd("DevicePath", status));
    }

    let device_path = path_buf as *mut InitrdDevicePath;
    *device_path = InitrdDevicePath::new();

    let mut handle: *mut () = ptr::null_mut();
    let status = (boot_services.install_protocol_interface)(
        &mut handle,
        &EFI_DEVICE_PATH_PROTOCOL_GUID,
        EFI_NATIVE_INTERFACE,
        device_path as *mut c_void,
    );
    if status != EFI_SUCCESS {
        (boot_services.free_pool)(path_buf);
        (boot_services.free_pool)(loader_buf);
        return Err(EfiStubError::InstallProtocol("DevicePath", status));
    }

    let status = (boot_services.install_protocol_interface)(
        &mut handle,
        &EFI_LOAD_FILE2_PROTOCOL_GUID,
        EFI_NATIVE_INTERFACE,
        loader as *mut c_void,
    );
    if status != EFI_SUCCESS {
        (boot_services.uninstall_protocol_interface)(
            handle,
            &EFI_DEVICE_PATH_PROTOCOL_GUID,
            device_path as *mut c_void,
        );
        (boot_services.free_pool)(path_buf);
        (boot_services.free_pool)(loader_buf);
        return Err(EfiStubError::InstallProtocol("LoadFile2", status));
    }

    Ok(InitrdProtocolHandles {
        handle,
        loader,
        device_path,
    })
}

unsafe fn uninstall_initrd_protocol(
    boot_services: &crate::BootServices,
    ctx: InitrdProtocolHandles,
) {
    let _ = (boot_services.uninstall_protocol_interface)(
        ctx.handle,
        &EFI_LOAD_FILE2_PROTOCOL_GUID,
        ctx.loader as *mut c_void,
    );
    let _ = (boot_services.uninstall_protocol_interface)(
        ctx.handle,
        &EFI_DEVICE_PATH_PROTOCOL_GUID,
        ctx.device_path as *mut c_void,
    );
    (boot_services.free_pool)(ctx.device_path as *mut u8);
    (boot_services.free_pool)(ctx.loader as *mut u8);
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

pub enum EfiStubError {
    Unsupported,
    LoadImage(usize),
    HandleProtocol(usize),
    AllocateCommandLine(usize),
    AllocateInitrd(&'static str, usize),
    InstallProtocol(&'static str, usize),
    StartImage(usize),
}

impl fmt::Debug for EfiStubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("Unsupported"),
            Self::LoadImage(status) => f.debug_tuple("LoadImage").field(status).finish(),
            Self::HandleProtocol(status) => f.debug_tuple("HandleProtocol").field(status).finish(),
            Self::AllocateCommandLine(status) => {
                f.debug_tuple("AllocateCommandLine").field(status).finish()
            }
            Self::AllocateInitrd(kind, status) => f
                .debug_tuple("AllocateInitrd")
                .field(kind)
                .field(status)
                .finish(),
            Self::InstallProtocol(kind, status) => f
                .debug_tuple("InstallProtocol")
                .field(kind)
                .field(status)
                .finish(),
            Self::StartImage(status) => f.debug_tuple("StartImage").field(status).finish(),
        }
    }
}

pub unsafe fn boot_via_efi_stub(
    boot_services: &crate::BootServices,
    image_handle: *mut (),
    kernel_data: &[u8],
    initrd_data: Option<&[u8]>,
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
        let status = (boot_services.allocate_pool)(
            EFI_POOL_TYPE_LOADER_DATA,
            options_len_bytes,
            &mut options_ptr,
        );
        if status != EFI_SUCCESS || options_ptr.is_null() {
            (boot_services.unload_image)(loaded_image_handle);
            return Err(EfiStubError::AllocateCommandLine(status));
        }
        ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, options_ptr, options_len_bytes);
        (*loaded_image_proto).load_options = options_ptr as *mut c_void;
        (*loaded_image_proto).load_options_size = options_len_bytes as u32;
        options_allocation = Some(options_ptr);
    } else {
        (*loaded_image_proto).load_options = ptr::null_mut();
        (*loaded_image_proto).load_options_size = 0;
    }

    let mut initrd_context: Option<InitrdProtocolHandles> = None;
    if let Some(initrd) = initrd_data.filter(|data| !data.is_empty()) {
        match install_initrd_protocol(boot_services, initrd) {
            Ok(handles) => initrd_context = Some(handles),
            Err(err) => {
                if let Some(ptr) = options_allocation {
                    (boot_services.free_pool)(ptr);
                }
                (boot_services.unload_image)(loaded_image_handle);
                return Err(err);
            }
        }
    }

    let status = (boot_services.start_image)(loaded_image_handle, ptr::null_mut(), ptr::null_mut());

    if status == EFI_SUCCESS {
        unsafe { core::hint::unreachable_unchecked() };
    }

    if let Some(ctx) = initrd_context {
        uninstall_initrd_protocol(boot_services, ctx);
    }

    if let Some(ptr) = options_allocation {
        (boot_services.free_pool)(ptr);
    }
    (boot_services.unload_image)(loaded_image_handle);
    Err(EfiStubError::StartImage(status))
}
