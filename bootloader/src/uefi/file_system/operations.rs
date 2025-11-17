use super::{FileProtocol, LoadedImageProtocol, LOADED_IMAGE_PROTOCOL_GUID, EFI_FILE_MODE_READ};
use crate::BootServices;

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
