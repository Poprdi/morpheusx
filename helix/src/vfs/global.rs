use crate::device::{MemBlockDevice, RawBlockDevice};
use crate::error::HelixError;
use crate::format;
use crate::log::recovery::recover_superblock;
use crate::types::*;
use crate::vfs::{HelixInstance, MountTable};
use gpt_disk_io::BlockIo;

pub struct FsGlobal {
    pub mount_table: MountTable,
    pub device: RawBlockDevice,
}

/// Backing storage for the RAM-disk MemBlockDevice when no real disk is available.
/// Must live in a static so the RawBlockDevice function pointers remain valid.
static mut MEM_DEVICE: Option<MemBlockDevice> = None;

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

    // Store MemBlockDevice in static so RawBlockDevice pointers stay valid.
    MEM_DEVICE = Some(MemBlockDevice::new(base, size, sector_size));
    let mem_dev = MEM_DEVICE.as_mut().unwrap();

    let total_sectors = size as u64 / sector_size as u64;
    let uuid = [0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]; // "MXROOT"

    format::format_helix(mem_dev, 0, total_sectors, sector_size, "root", uuid)?;

    let sb = recover_superblock(mem_dev, 0, sector_size)?;
    let instance = HelixInstance::new(sb, 0, sector_size);

    let mut mount_table = MountTable::new();
    mount_table.mount("/", instance, false)?;

    let raw_device = MemBlockDevice::into_raw(mem_dev);

    FS_GLOBAL = Some(FsGlobal { mount_table, device: raw_device });
    FS_INITIALIZED = true;
    Ok(())
}

/// Initialize the root filesystem on a pre-existing RawBlockDevice.
///
/// If `format` is true, formats the device before mounting.
/// If false, tries to recover an existing HelixFS.
///
/// # Safety
/// The RawBlockDevice's backing storage must remain valid for the kernel lifetime.
pub unsafe fn init_root_fs_raw(
    mut device: RawBlockDevice,
    do_format: bool,
) -> Result<(), HelixError> {
    if FS_INITIALIZED {
        return Ok(());
    }

    let sector_size = {
        let bs = device.block_size();
        // BlockSize wraps NonZeroU32; we extract via Display → parse.
        // Simpler: match known sizes.
        if bs == gpt_disk_types::BlockSize::BS_512 { 512u32 }
        else if bs == gpt_disk_types::BlockSize::BS_4096 { 4096u32 }
        else { return Err(HelixError::InvalidBlockSize); }
    };
    let total_sectors = device.num_blocks().map_err(|_| HelixError::IoReadFailed)?;

    if do_format {
        let uuid = [0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        format::format_helix(&mut device, 0, total_sectors, sector_size, "root", uuid)?;
    }

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

/// Replace the root filesystem device with a persistent block device.
///
/// Keeps all VFS state (open fds, etc.) but re-reads the superblock from
/// the new device and re-mounts "/".
///
/// If `do_format` is true, formats the device before mounting.
/// If false, tries to recover an existing HelixFS.
///
/// # Safety
/// The RawBlockDevice's backing storage must remain valid for the kernel lifetime.
/// Must be called while FS is not being accessed from another context.
pub unsafe fn replace_root_device(
    mut device: RawBlockDevice,
    do_format: bool,
) -> Result<(), HelixError> {
    let sector_size = {
        let bs = device.block_size();
        if bs == gpt_disk_types::BlockSize::BS_512 { 512u32 }
        else if bs == gpt_disk_types::BlockSize::BS_4096 { 4096u32 }
        else { return Err(HelixError::InvalidBlockSize); }
    };
    let total_sectors = device.num_blocks().map_err(|_| HelixError::IoReadFailed)?;

    if do_format {
        let uuid = [0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        format::format_helix(&mut device, 0, total_sectors, sector_size, "root", uuid)?;
    }

    let sb = recover_superblock(&mut device, 0, sector_size)?;
    let instance = HelixInstance::new(sb, 0, sector_size);

    let mut mount_table = MountTable::new();
    mount_table.mount("/", instance, false)?;

    FS_GLOBAL = Some(FsGlobal { mount_table, device });
    FS_INITIALIZED = true;
    Ok(())
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
