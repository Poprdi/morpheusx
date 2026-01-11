//! Manifest I/O operations
//!
//! Handles reading and writing ISO manifests to/from the ESP filesystem.
//! Manifests are stored at `/morpheus/isos/<name>.manifest`.
//!
//! # Storage Layout
//!
//! ```text
//! ESP:/
//! └── morpheus/
//!     └── isos/
//!         ├── ubuntu-24.04.iso.manifest
//!         ├── fedora-40.iso.manifest
//!         └── ...
//! ```

use alloc::vec::Vec;
use alloc::string::String;
use morpheus_core::iso::{IsoManifest, IsoStorageManager, MAX_MANIFEST_SIZE};
use crate::BootServices;
use crate::uefi::file_system::{
    FileProtocol, create_directory, create_file, write_file, flush_file, close_file,
    open_file_read, EFI_FILE_MODE_READ, ascii_to_utf16, get_loaded_image,
};

/// Manifest directory path on ESP (without leading backslash for open)
const MANIFEST_DIR: &str = "\\morpheus\\isos";

/// Maximum number of manifests to scan
const MAX_MANIFESTS: usize = 16;

/// Result type for manifest I/O operations
pub type ManifestIoResult<T> = Result<T, ManifestIoError>;

/// Manifest I/O error types
#[derive(Debug, Clone, Copy)]
pub enum ManifestIoError {
    /// Failed to get ESP root
    EspAccessFailed,
    /// Failed to create directory
    DirectoryCreateFailed,
    /// Failed to create/open file
    FileCreateFailed,
    /// Failed to write file
    WriteFailed,
    /// Failed to read file
    ReadFailed,
    /// Failed to serialize manifest
    SerializeFailed,
    /// Failed to deserialize manifest
    DeserializeFailed,
    /// File not found
    NotFound,
}

/// Persist a manifest to the ESP filesystem
///
/// Creates the manifest directory if it doesn't exist, then writes
/// the serialized manifest to `/morpheus/isos/<name>.manifest`.
///
/// # Arguments
/// * `bs` - UEFI Boot Services
/// * `image_handle` - Current image handle
/// * `manifest` - The manifest to persist
///
/// # Returns
/// * `Ok(())` on success
/// * `Err(ManifestIoError)` on failure
pub unsafe fn persist_manifest(
    bs: &BootServices,
    image_handle: *mut (),
    manifest: &IsoManifest,
) -> ManifestIoResult<()> {
    // Get ESP root
    let root = get_esp_root(bs, image_handle)?;

    // Ensure morpheus directory exists
    let mut morpheus_path = [0u16; 32];
    ascii_to_utf16("\\morpheus", &mut morpheus_path);
    let _ = create_directory(root, &morpheus_path); // Ignore error if exists

    // Ensure isos subdirectory exists
    let mut isos_path = [0u16; 32];
    ascii_to_utf16("\\morpheus\\isos", &mut isos_path);
    let _ = create_directory(root, &isos_path); // Ignore error if exists

    // Build manifest filename: <name>.manifest
    let name = manifest.name_str();
    let mut filename = String::new();
    filename.push_str("\\morpheus\\isos\\");
    filename.push_str(name);
    filename.push_str(".manifest");

    // Convert to UTF-16
    let mut path_utf16 = [0u16; 128];
    ascii_to_utf16(&filename, &mut path_utf16);

    // Create/open the manifest file
    let file = create_file(root, &path_utf16).map_err(|_| ManifestIoError::FileCreateFailed)?;

    // Serialize manifest
    let mut buffer = [0u8; MAX_MANIFEST_SIZE];
    let size = manifest.serialize(&mut buffer).map_err(|_| ManifestIoError::SerializeFailed)?;

    // Write to file
    write_file(file, &buffer[..size]).map_err(|_| ManifestIoError::WriteFailed)?;

    // Flush and close
    let _ = flush_file(file);
    let _ = close_file(file);
    let _ = close_file(root);

    morpheus_core::logger::log("Manifest persisted to ESP");

    Ok(())
}

/// Load all manifests from ESP and populate a storage manager
///
/// Scans `/morpheus/isos/` for .manifest files and loads them.
///
/// # Arguments
/// * `bs` - UEFI Boot Services
/// * `image_handle` - Current image handle
/// * `storage` - Storage manager to populate
///
/// # Returns
/// * `Ok(count)` - Number of manifests loaded
/// * `Err(ManifestIoError)` on failure
pub unsafe fn load_manifests_from_esp(
    bs: &BootServices,
    image_handle: *mut (),
    storage: &mut IsoStorageManager,
) -> ManifestIoResult<usize> {
    // Get ESP root
    let root = get_esp_root(bs, image_handle)?;

    // Open manifest directory
    let mut dir_path = [0u16; 32];
    ascii_to_utf16("\\morpheus\\isos", &mut dir_path);

    let mut dir: *mut FileProtocol = core::ptr::null_mut();
    let status = ((*root).open)(
        root,
        &mut dir,
        dir_path.as_ptr(),
        EFI_FILE_MODE_READ,
        0,
    );

    if status != 0 || dir.is_null() {
        let _ = close_file(root);
        // Directory doesn't exist yet - that's OK, no manifests
        return Ok(0);
    }

    // Scan directory for .manifest files
    let mut count = 0;
    let mut buffer = [0u8; 512]; // For directory entry

    loop {
        let mut size = buffer.len();
        let status = ((*dir).read)(dir, &mut size, buffer.as_mut_ptr());

        if status != 0 || size == 0 {
            break; // End of directory
        }

        // Parse EFI_FILE_INFO structure
        // Offset 0x50 (80) is where filename starts in EFI_FILE_INFO
        // But we need to check the attribute at offset 0x48 (72) for directory flag
        if size < 82 {
            continue;
        }

        let attributes = u64::from_le_bytes([
            buffer[0x38], buffer[0x39], buffer[0x3A], buffer[0x3B],
            buffer[0x3C], buffer[0x3D], buffer[0x3E], buffer[0x3F],
        ]);

        // Skip directories (attribute bit 4 = EFI_FILE_DIRECTORY)
        if attributes & 0x10 != 0 {
            continue;
        }

        // Get filename from UTF-16 at offset 0x50
        let filename = extract_filename_from_file_info(&buffer);
        
        // Check if it ends with .MFS or .manifest (support both)
        if !filename.ends_with(".MFS") && !filename.ends_with(".mfs") && !filename.ends_with(".manifest") {
            continue;
        }

        // Load this manifest
        if let Ok(manifest) = load_single_manifest(root, &filename) {
            if storage.add_entry(manifest).is_ok() {
                count += 1;
                if count >= MAX_MANIFESTS {
                    break;
                }
            }
        }
    }

    let _ = close_file(dir);
    let _ = close_file(root);

    morpheus_core::logger::log(alloc::format!("Loaded {} manifests from ESP", count).leak());

    Ok(count)
}

/// Load a single manifest file by name
unsafe fn load_single_manifest(
    root: *mut FileProtocol,
    filename: &str,
) -> ManifestIoResult<IsoManifest> {
    // Build full path
    let mut full_path = String::new();
    full_path.push_str("\\morpheus\\isos\\");
    full_path.push_str(filename);

    // Convert to UTF-16
    let mut path_utf16 = [0u16; 128];
    ascii_to_utf16(&full_path, &mut path_utf16);

    // Open file
    let mut file: *mut FileProtocol = core::ptr::null_mut();
    let status = ((*root).open)(
        root,
        &mut file,
        path_utf16.as_ptr(),
        EFI_FILE_MODE_READ,
        0,
    );

    if status != 0 || file.is_null() {
        return Err(ManifestIoError::NotFound);
    }

    // Read manifest data
    let mut buffer = [0u8; MAX_MANIFEST_SIZE];
    let mut size = buffer.len();
    let status = ((*file).read)(file, &mut size, buffer.as_mut_ptr());

    let _ = close_file(file);

    if status != 0 || size == 0 {
        return Err(ManifestIoError::ReadFailed);
    }

    // Deserialize
    IsoManifest::deserialize(&buffer[..size]).map_err(|_| ManifestIoError::DeserializeFailed)
}

/// Get ESP root directory handle
unsafe fn get_esp_root(
    bs: &BootServices,
    image_handle: *mut (),
) -> ManifestIoResult<*mut FileProtocol> {
    // Get loaded image to find device
    let loaded_image = get_loaded_image(bs, image_handle)
        .map_err(|_| ManifestIoError::EspAccessFailed)?;

    let device_handle = (*loaded_image).device_handle;

    // Get filesystem protocol
    let mut fs_protocol: *mut () = core::ptr::null_mut();
    let status = (bs.handle_protocol)(
        device_handle,
        &crate::uefi::file_system::SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
        &mut fs_protocol,
    );

    if status != 0 || fs_protocol.is_null() {
        return Err(ManifestIoError::EspAccessFailed);
    }

    let fs = fs_protocol as *mut crate::uefi::file_system::SimpleFileSystemProtocol;

    // Open root volume
    let mut root: *mut FileProtocol = core::ptr::null_mut();
    let status = ((*fs).open_volume)(fs, &mut root);

    if status != 0 || root.is_null() {
        return Err(ManifestIoError::EspAccessFailed);
    }

    Ok(root)
}

/// Extract filename from EFI_FILE_INFO buffer (UTF-16 at offset 0x50)
fn extract_filename_from_file_info(buffer: &[u8]) -> String {
    let mut filename = String::new();
    let mut offset = 0x50; // FileName starts at offset 80

    while offset + 1 < buffer.len() {
        let c = u16::from_le_bytes([buffer[offset], buffer[offset + 1]]);
        if c == 0 {
            break;
        }
        if let Some(ch) = char::from_u32(c as u32) {
            filename.push(ch);
        }
        offset += 2;
    }

    filename
}

/// Delete a manifest file from ESP
///
/// # Arguments
/// * `bs` - UEFI Boot Services
/// * `image_handle` - Current image handle  
/// * `name` - ISO name (manifest filename will be <name>.manifest)
pub unsafe fn delete_manifest(
    bs: &BootServices,
    image_handle: *mut (),
    name: &str,
) -> ManifestIoResult<()> {
    let root = get_esp_root(bs, image_handle)?;

    // Build manifest filename
    let mut filename = String::new();
    filename.push_str("\\morpheus\\isos\\");
    filename.push_str(name);
    filename.push_str(".manifest");

    // Convert to UTF-16
    let mut path_utf16 = [0u16; 128];
    ascii_to_utf16(&filename, &mut path_utf16);

    // Open file
    let mut file: *mut FileProtocol = core::ptr::null_mut();
    let status = ((*root).open)(
        root,
        &mut file,
        path_utf16.as_ptr(),
        EFI_FILE_MODE_READ | crate::uefi::file_system::EFI_FILE_MODE_WRITE,
        0,
    );

    if status != 0 || file.is_null() {
        let _ = close_file(root);
        return Err(ManifestIoError::NotFound);
    }

    // Delete the file
    let status = ((*file).delete)(file);
    // Note: delete() closes the handle on success or failure

    let _ = close_file(root);

    if status != 0 {
        return Err(ManifestIoError::WriteFailed);
    }

    Ok(())
}
