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

/// Helper to convert ASCII string to UTF-16 (null-terminated)
pub fn ascii_to_utf16(ascii: &str, buf: &mut [u16]) -> usize {
    let mut i = 0;
    for (idx, byte) in ascii.bytes().enumerate() {
        if idx >= buf.len() - 1 {
            break;
        }
        buf[idx] = byte as u16;
        i = idx + 1;
    }
    buf[i] = 0; // Null terminator
    i + 1
}

/// Get Loaded Image Protocol to access current binary
pub unsafe fn get_loaded_image(
    bs: &BootServices,
    image_handle: *mut (),
) -> Result<*mut LoadedImageProtocol, ()> {
    let mut loaded_image: *mut () = core::ptr::null_mut();

    let status = (bs.handle_protocol)(image_handle, &LOADED_IMAGE_PROTOCOL_GUID, &mut loaded_image);

    if status != 0 {
        return Err(());
    }

    Ok(loaded_image as *mut LoadedImageProtocol)
}

/// Get actual PE file size from headers (not memory image size)
pub unsafe fn get_pe_file_size(image_base: *const u8) -> Result<usize, ()> {
    // DOS Header: verify MZ signature
    let dos_signature = u16::from_le_bytes([*image_base, *image_base.offset(1)]);
    if dos_signature != 0x5A4D {
        // "MZ"
        return Err(());
    }

    // e_lfanew at offset 0x3C: points to PE header
    let pe_offset = u32::from_le_bytes([
        *image_base.offset(0x3C),
        *image_base.offset(0x3D),
        *image_base.offset(0x3E),
        *image_base.offset(0x3F),
    ]) as isize;

    // PE Signature: verify "PE\0\0"
    let pe_sig = u32::from_le_bytes([
        *image_base.offset(pe_offset),
        *image_base.offset(pe_offset + 1),
        *image_base.offset(pe_offset + 2),
        *image_base.offset(pe_offset + 3),
    ]);
    if pe_sig != 0x00004550 {
        return Err(());
    }

    // COFF Header starts at pe_offset + 4
    let coff_header = pe_offset + 4;

    // NumberOfSections at offset 0x02 in COFF header
    let num_sections = u16::from_le_bytes([
        *image_base.offset(coff_header + 0x02),
        *image_base.offset(coff_header + 0x03),
    ]) as usize;

    // SizeOfOptionalHeader at offset 0x10 in COFF header
    let opt_header_size = u16::from_le_bytes([
        *image_base.offset(coff_header + 0x10),
        *image_base.offset(coff_header + 0x11),
    ]) as isize;

    // Section table starts after: PE sig (4) + COFF header (20) + optional header
    let section_table = pe_offset + 4 + 20 + opt_header_size;

    // Each section header is 40 bytes
    // We need to find the highest (PointerToRawData + SizeOfRawData)
    let mut max_file_offset = 0usize;

    for i in 0..num_sections {
        let section_header = section_table + (i as isize * 40);

        // SizeOfRawData at offset 0x10 in section header
        let size_of_raw_data = u32::from_le_bytes([
            *image_base.offset(section_header + 0x10),
            *image_base.offset(section_header + 0x11),
            *image_base.offset(section_header + 0x12),
            *image_base.offset(section_header + 0x13),
        ]) as usize;

        // PointerToRawData at offset 0x14 in section header
        let pointer_to_raw_data = u32::from_le_bytes([
            *image_base.offset(section_header + 0x14),
            *image_base.offset(section_header + 0x15),
            *image_base.offset(section_header + 0x16),
            *image_base.offset(section_header + 0x17),
        ]) as usize;

        // Skip sections with no raw data
        if size_of_raw_data > 0 && pointer_to_raw_data > 0 {
            let section_end = pointer_to_raw_data + size_of_raw_data;
            if section_end > max_file_offset {
                max_file_offset = section_end;
            }
        }
    }

    // File size is the end of the last section
    if max_file_offset == 0 {
        return Err(());
    }

    Ok(max_file_offset)
}

/// Restore original ImageBase in PE header
/// UEFI relocates executables and modifies the ImageBase field - we need to restore it
pub fn restore_pe_image_base(pe_data: &mut [u8]) -> Result<(), ()> {
    // DOS header check
    if pe_data.len() < 0x40 {
        return Err(());
    }

    let dos_sig = u16::from_le_bytes([pe_data[0], pe_data[1]]);
    if dos_sig != 0x5A4D {
        return Err(());
    }

    // Get PE offset
    let pe_offset =
        u32::from_le_bytes([pe_data[0x3C], pe_data[0x3D], pe_data[0x3E], pe_data[0x3F]]) as usize;

    if pe_offset + 0xB8 > pe_data.len() {
        return Err(());
    }

    // Verify PE signature
    let pe_sig = u32::from_le_bytes([
        pe_data[pe_offset],
        pe_data[pe_offset + 1],
        pe_data[pe_offset + 2],
        pe_data[pe_offset + 3],
    ]);
    if pe_sig != 0x00004550 {
        return Err(());
    }

    // ImageBase is at offset 0x18 in PE32+ optional header
    // PE header + 4 (signature) + 20 (COFF) + 0x18 (ImageBase offset in optional header)
    let image_base_offset = pe_offset + 4 + 20 + 0x18;

    // For x86_64 UEFI, the original ImageBase is typically 0x400000 or 0x140000000
    // We'll use the standard UEFI loader base: 0x400000 (4MB)
    let original_image_base = 0x0000000000400000u64;

    // Write original ImageBase back
    pe_data[image_base_offset..image_base_offset + 8]
        .copy_from_slice(&original_image_base.to_le_bytes());

    Ok(())
}

/// Open file for reading only
pub unsafe fn open_file_read(
    root: *mut FileProtocol,
    path: &[u16],
) -> Result<*mut FileProtocol, usize> {
    let mut file: *mut FileProtocol = core::ptr::null_mut();

    let status = ((*root).open)(root, &mut file, path.as_ptr(), EFI_FILE_MODE_READ, 0);

    if status != 0 {
        return Err(status);
    }

    Ok(file)
}
