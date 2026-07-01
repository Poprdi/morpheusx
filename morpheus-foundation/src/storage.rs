//! Storage-subsystem ABI constants and generational-handle packing.
//!
//! Part of the frozen kernel↔userland seam (see `flags`/`syscall_abi`): the
//! `DEV_*`/`FS_*`/`MNT_*`/`VOL_*` values cross `SYS_VOLUMES`/`SYS_MOUNTS`/
//! `SYS_MOUNT`, so they live here once and consumers re-export. See the storage
//! subsystem spec §5.

/// `VolumeInfo::device_kind`. Live drivers and RAM regions look identical above
/// the device layer; this only records provenance.
pub const DEV_RAM: u32 = 0;
pub const DEV_VIRTIO: u32 = 1;
pub const DEV_AHCI: u32 = 2;
pub const DEV_SDHCI: u32 = 3;
pub const DEV_USBMSD: u32 = 4;

/// `fs_type`. `FS_AUTO`/`FS_HELIX`/`FS_FAT32` are mount selectors (`SYS_MOUNT`);
/// `FS_NONE`/`FS_UNKNOWN` only appear as `VolumeInfo::fs_type` detection results.
pub const FS_AUTO: u32 = 0;
pub const FS_HELIX: u32 = 1;
pub const FS_FAT32: u32 = 2;
pub const FS_NONE: u32 = 3;
pub const FS_UNKNOWN: u32 = 4;

/// `SYS_MOUNT`/`SYS_UMOUNT` flags. `MNT_STAGED` = copy source into RAM (residency
/// axis); `MNT_FORCE` is umount-only (revoke open fds).
pub const MNT_RDONLY: u32 = 1 << 0;
pub const MNT_STAGED: u32 = 1 << 1;
pub const MNT_FORCE: u32 = 1 << 2;

/// `VolumeInfo::flags`. `VOL_EPHEMERAL` marks a synthesized RAM volume backing a
/// staged mount (owned by its creating process, reclaimed on reap).
pub const VOL_RDONLY: u32 = 1 << 0;
pub const VOL_MOUNTED: u32 = 1 << 1;
pub const VOL_REMOVABLE: u32 = 1 << 2;
pub const VOL_EPHEMERAL: u32 = 1 << 3;

/// `SYS_MOUNT` source sentinel: mount from nothing (fresh empty RAM volume).
pub const VOLUME_NONE: u64 = 0;

/// Bytes of backend-private per-fd state in `FdState` (Helix index key; FAT32
/// start+current cluster; future ext4 inode#+cursor). Bump to recompile if a
/// backend needs more persistent per-fd state.
pub const FD_COOKIE_LEN: usize = 32;

/// Pack a slab `(index, generation)` into a stable u64 handle: `generation` high,
/// `index` low. A stale handle fails the generation check → `ENODEV`, so reusing a
/// freed slot can't alias.
#[inline]
pub const fn pack(index: u32, generation: u32) -> u64 {
    ((generation as u64) << 32) | index as u64
}

/// Inverse of [`pack`]: `(index, generation)`.
#[inline]
pub const fn unpack(handle: u64) -> (u32, u32) {
    (handle as u32, (handle >> 32) as u32)
}
