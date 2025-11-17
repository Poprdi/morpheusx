// UEFI Simple File System Protocol implementation

use crate::BootServices;

// Protocol GUIDs
pub const SIMPLE_FILE_SYSTEM_PROTOCOL_GUID: [u8; 16] = [
    0x22, 0x5b, 0x4e, 0x96, 0x59, 0x64, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

pub const LOADED_IMAGE_PROTOCOL_GUID: [u8; 16] = [
    0xa1, 0x31, 0x1b, 0x5b, 0x62, 0x95, 0xd2, 0x11, 0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

pub const FILE_INFO_GUID: [u8; 16] = [
    0x92, 0xec, 0x79, 0x09, 0x96, 0x5f, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

// File attributes
pub const EFI_FILE_MODE_READ: u64 = 0x0000000000000001;
pub const EFI_FILE_MODE_WRITE: u64 = 0x0000000000000002;
pub const EFI_FILE_MODE_CREATE: u64 = 0x8000000000000000;

pub const EFI_FILE_DIRECTORY: u64 = 0x0000000000000010;

#[repr(C)]
pub struct LoadedImageProtocol {
    revision: u32,
    parent_handle: *mut (),
    system_table: *mut (),
    pub device_handle: *mut (),
    file_path: *mut (),
    _reserved: *mut (),
    load_options_size: u32,
    load_options: *mut (),
    pub image_base: *mut (),
    pub image_size: u64,
    image_code_type: u32,
    image_data_type: u32,
    unload: usize,
}

#[repr(C)]
pub struct SimpleFileSystemProtocol {
    revision: u64,
    pub open_volume: extern "efiapi" fn(
        this: *mut SimpleFileSystemProtocol,
        root: *mut *mut FileProtocol,
    ) -> usize,
}

#[repr(C)]
pub struct FileProtocol {
    revision: u64,
    pub open: extern "efiapi" fn(
        this: *mut FileProtocol,
        new_handle: *mut *mut FileProtocol,
        file_name: *const u16,
        open_mode: u64,
        attributes: u64,
    ) -> usize,
    pub close: extern "efiapi" fn(this: *mut FileProtocol) -> usize,
    pub delete: extern "efiapi" fn(this: *mut FileProtocol) -> usize,
    pub read: extern "efiapi" fn(
        this: *mut FileProtocol,
        buffer_size: *mut usize,
        buffer: *mut u8,
    ) -> usize,
    pub write: extern "efiapi" fn(
        this: *mut FileProtocol,
        buffer_size: *mut usize,
        buffer: *const u8,
    ) -> usize,
    pub get_position: usize,
    pub set_position: extern "efiapi" fn(this: *mut FileProtocol, position: u64) -> usize,
    pub get_info: usize,
    pub set_info: usize,
    pub flush: extern "efiapi" fn(this: *mut FileProtocol) -> usize,
}

/// Get Simple File System Protocol for a disk handle
pub unsafe fn get_file_system_protocol(
    bs: &BootServices,
    disk_handle: *mut (),
) -> Result<*mut SimpleFileSystemProtocol, ()> {
    let mut fs_protocol: *mut () = core::ptr::null_mut();

    let status = (bs.handle_protocol)(
        disk_handle,
        &SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
        &mut fs_protocol,
    );

    if status != 0 {
        return Err(());
    }

    Ok(fs_protocol as *mut SimpleFileSystemProtocol)
}

/// Open root directory of a volume
pub unsafe fn open_root_volume(
    fs_protocol: *mut SimpleFileSystemProtocol,
) -> Result<*mut FileProtocol, ()> {
    let mut root: *mut FileProtocol = core::ptr::null_mut();

    let status = ((*fs_protocol).open_volume)(fs_protocol, &mut root);

    if status != 0 {
        return Err(());
    }

    Ok(root)
}

/// Create directory (recursive if needed)
pub unsafe fn create_directory(
    root: *mut FileProtocol,
    path: &[u16], // UTF-16 path
) -> Result<*mut FileProtocol, ()> {
    let mut dir: *mut FileProtocol = core::ptr::null_mut();

    let status = ((*root).open)(
        root,
        &mut dir,
        path.as_ptr(),
        EFI_FILE_MODE_READ | EFI_FILE_MODE_WRITE | EFI_FILE_MODE_CREATE,
        EFI_FILE_DIRECTORY,
    );

    if status != 0 {
        return Err(());
    }

    Ok(dir)
}

/// Create or open a file for writing
pub unsafe fn create_file(
    root: *mut FileProtocol,
    path: &[u16], // UTF-16 path
) -> Result<*mut FileProtocol, ()> {
    let mut file: *mut FileProtocol = core::ptr::null_mut();

    let status = ((*root).open)(
        root,
        &mut file,
        path.as_ptr(),
        EFI_FILE_MODE_READ | EFI_FILE_MODE_WRITE | EFI_FILE_MODE_CREATE,
        0, // Regular file, no special attributes
    );

    if status != 0 {
        return Err(());
    }

    Ok(file)
}

/// Write data to a file
pub unsafe fn write_file(file: *mut FileProtocol, data: &[u8]) -> Result<(), ()> {
    let mut size = data.len();

    let status = ((*file).write)(file, &mut size, data.as_ptr());

    if status != 0 || size != data.len() {
        return Err(());
    }

    Ok(())
}

/// Flush file buffers to disk
pub unsafe fn flush_file(file: *mut FileProtocol) -> Result<(), ()> {
    let status = ((*file).flush)(file);

    if status != 0 {
        return Err(());
    }

    Ok(())
}

/// Close a file handle
pub unsafe fn close_file(file: *mut FileProtocol) -> Result<(), ()> {
    let status = ((*file).close)(file);

    if status != 0 {
        return Err(());
    }

    Ok(())
}
