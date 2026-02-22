use crate::device::MemBlockDevice;
use crate::error::HelixError;
use crate::format;
use crate::log::recovery::recover_superblock;
use crate::types::*;
use crate::vfs::{HelixInstance, MountTable};

pub struct FsGlobal {
    pub mount_table: MountTable,
    pub device: MemBlockDevice,
}

static mut FS_GLOBAL: Option<FsGlobal> = None;
static mut FS_INITIALIZED: bool = false;

/// Initialize the root filesystem on a memory-backed block device.
///
/// Formats the region as HelixFS, recovers the superblock, and mounts at "/".
///
/// # Safety
/// `base` must point to `size` bytes of zeroed, identity-mapped physical RAM.
/// Must be called exactly once, after heap is ready.
pub unsafe fn init_root_fs(base: *mut u8, size: usize) -> Result<(), HelixError> {
    if FS_INITIALIZED {
        return Ok(());
    }

    let sector_size = BLOCK_SIZE;
    let mut device = MemBlockDevice::new(base, size, sector_size);

    let total_sectors = size as u64 / sector_size as u64;
    let uuid = [0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]; // "MXROOT"

    format::format_helix(&mut device, 0, total_sectors, sector_size, "root", uuid)?;

    let sb = recover_superblock(&mut device, 0, sector_size)?;
    let instance = HelixInstance::new(sb, 0, sector_size);

    let mut mount_table = MountTable::new();
    mount_table.mount("/", instance, false)?;

    FS_GLOBAL = Some(FsGlobal { mount_table, device });
    FS_INITIALIZED = true;
    Ok(())
}

pub fn is_fs_initialized() -> bool {
    unsafe { FS_INITIALIZED }
}

/// Get immutable reference to global FS state.
///
/// # Safety
/// Must be called after `init_root_fs()`. Single-threaded access only.
pub unsafe fn fs_global() -> Option<&'static FsGlobal> {
    FS_GLOBAL.as_ref()
}

/// Get mutable reference to global FS state.
///
/// # Safety
/// Must be called after `init_root_fs()`. Caller must ensure no aliasing.
pub unsafe fn fs_global_mut() -> Option<&'static mut FsGlobal> {
    FS_GLOBAL.as_mut()
}
