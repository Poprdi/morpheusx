use crate::device::{MemBlockDevice, RawBlockDevice};
use crate::error::HelixError;
use crate::format;
use crate::log::recovery::{recover_superblock, replay_log};
use crate::types::*;
use crate::vfs::{HelixInstance, MountTable};
use gpt_disk_io::BlockIo;

/// After replay the bitmap is zero. Mark every extent-backed live file's
/// blocks used, or new allocations will overlap existing data.
fn rebuild_bitmap_from_index(instance: &mut HelixInstance) {
    for entry in instance.index.all_entries() {
        if entry.flags & entry_flags::IS_DELETED != 0 {
            continue;
        }
        if entry.flags & entry_flags::IS_DIR != 0 {
            continue;
        }
        if entry.flags & entry_flags::IS_INLINE != 0 {
            continue;
        }
        if entry.extent_root == BLOCK_NULL {
            continue;
        }
        let blocks_needed = entry.size.div_ceil(BLOCK_SIZE as u64);
        if blocks_needed > 0 {
            instance
                .bitmap
                .mark_range_used(entry.extent_root, blocks_needed);
        }
    }
}

pub struct FsGlobal {
    pub mount_table: MountTable,
    pub device: RawBlockDevice,
}

// MemBlockDevice must outlive the fn pointers in RawBlockDevice.
static mut MEM_DEVICE: Option<MemBlockDevice> = None;

static mut FS_GLOBAL: Option<FsGlobal> = None;
static mut FS_INITIALIZED: bool = false;

/// Format the region, recover the superblock, mount at "/".
///
/// # Safety
/// `base..base+size` must be zeroed, identity-mapped RAM. Call once,
/// post-heap-init.
pub unsafe fn init_root_fs(base: *mut u8, size: usize) -> Result<(), HelixError> {
    if FS_INITIALIZED {
        return Ok(());
    }

    let sector_size = BLOCK_SIZE;

    MEM_DEVICE = Some(MemBlockDevice::new(base, size, sector_size));
    #[allow(static_mut_refs)]
    let mem_dev = MEM_DEVICE.as_mut().unwrap();

    let total_sectors = size as u64 / sector_size as u64;
    let uuid = [
        0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]; // "MXROOT\0\0\0\0\0\0\0\0\0\x01"

    format::format_helix(mem_dev, 0, total_sectors, sector_size, "root", uuid)?;

    let sb = recover_superblock(mem_dev, 0, sector_size)?;
    let instance = HelixInstance::new(sb, 0, sector_size);

    let mut mount_table = MountTable::new();
    mount_table.mount("/", instance, false)?;

    let raw_device = MemBlockDevice::into_raw(mem_dev);

    FS_GLOBAL = Some(FsGlobal {
        mount_table,
        device: raw_device,
    });
    FS_INITIALIZED = true;
    Ok(())
}

/// Mount (or first-format) an existing RawBlockDevice at "/".
///
/// # Safety
/// Backing storage must live for the kernel's lifetime.
pub unsafe fn init_root_fs_raw(
    mut device: RawBlockDevice,
    do_format: bool,
) -> Result<(), HelixError> {
    if FS_INITIALIZED {
        return Ok(());
    }

    let sector_size = {
        let bs = device.block_size();
        if bs == gpt_disk_types::BlockSize::BS_512 {
            512u32
        } else if bs == gpt_disk_types::BlockSize::BS_4096 {
            4096u32
        } else {
            return Err(HelixError::InvalidBlockSize);
        }
    };
    let total_sectors = device.num_blocks().map_err(|_| HelixError::IoReadFailed)?;

    if do_format {
        let uuid = [
            0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ];
        format::format_helix(&mut device, 0, total_sectors, sector_size, "root", uuid)?;
    }

    let sb = recover_superblock(&mut device, 0, sector_size)?;

    // v2 added paths in log payloads; older volumes must be reformatted.
    if sb.version != HELIX_VERSION {
        return Err(HelixError::IncompatibleVersion);
    }

    let mut instance = HelixInstance::new(sb, 0, sector_size);

    // Reload head so flush() doesn't clobber existing records.
    instance.log.reload_head_segment(&mut device)?;

    replay_log(&mut device, &instance.log, &mut instance.index)?;
    rebuild_bitmap_from_index(&mut instance);

    let mut mount_table = MountTable::new();
    mount_table.mount("/", instance, false)?;

    FS_GLOBAL = Some(FsGlobal {
        mount_table,
        device,
    });
    FS_INITIALIZED = true;
    Ok(())
}

pub fn is_fs_initialized() -> bool {
    unsafe { FS_INITIALIZED }
}

/// Swap root storage. Preserves VFS state; re-reads SB and remounts "/".
///
/// # Safety
/// Backing storage must live for the kernel's lifetime. FS must be quiescent.
pub unsafe fn replace_root_device(
    mut device: RawBlockDevice,
    do_format: bool,
) -> Result<(), HelixError> {
    let sector_size = {
        let bs = device.block_size();
        if bs == gpt_disk_types::BlockSize::BS_512 {
            512u32
        } else if bs == gpt_disk_types::BlockSize::BS_4096 {
            4096u32
        } else {
            return Err(HelixError::InvalidBlockSize);
        }
    };
    let total_sectors = device.num_blocks().map_err(|_| HelixError::IoReadFailed)?;

    if do_format {
        let uuid = [
            0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ];
        format::format_helix(&mut device, 0, total_sectors, sector_size, "root", uuid)?;
    }

    let sb = recover_superblock(&mut device, 0, sector_size)?;

    if sb.version != HELIX_VERSION {
        return Err(HelixError::IncompatibleVersion);
    }

    let mut instance = HelixInstance::new(sb, 0, sector_size);
    instance.log.reload_head_segment(&mut device)?;
    replay_log(&mut device, &instance.log, &mut instance.index)?;
    rebuild_bitmap_from_index(&mut instance);

    let mut mount_table = MountTable::new();
    mount_table.mount("/", instance, false)?;

    FS_GLOBAL = Some(FsGlobal {
        mount_table,
        device,
    });
    FS_INITIALIZED = true;
    Ok(())
}

/// # Safety
/// Post-init only; single-threaded access.
#[allow(static_mut_refs)]
pub unsafe fn fs_global() -> Option<&'static FsGlobal> {
    FS_GLOBAL.as_ref()
}

/// # Safety
/// Post-init only; caller guarantees no aliasing.
#[allow(static_mut_refs)]
pub unsafe fn fs_global_mut() -> Option<&'static mut FsGlobal> {
    FS_GLOBAL.as_mut()
}
