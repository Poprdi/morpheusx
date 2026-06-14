//! The storage subsystem (spec §3–§7): the kernel's filesystem abstraction.
//! Replaces the old `FsGlobal` / `helix::vfs`. Three generational registries
//! (devices / volumes / mounts) behind one `STORAGE_LOCK`, an enum-dispatched
//! `MountedFs`, longest-prefix `resolve`, two-phase staged `mount`, refcounted
//! `umount`, and a RAM-budget `staging` chokepoint. No `dyn` in the I/O path; no
//! fixed-size registry arrays. The syscall handlers and boot drive this; wiring
//! lives in later phases.

pub mod backends;
pub mod fs_api;
pub mod registry;
pub mod slab;
pub mod staging;

use crate::sync::RawSpinLock;
use backends::{Fat32Fs as Fat32Adapter, HelixFs as HelixAdapter, MountedFs};
use fs_api::VfsError;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;
use morpheus_block_types::{DeviceKind, RawBlockDevice};
use morpheus_foundation::errno::{
    EBADF, EBUSY, EEXIST, EINVAL, EIO, EISDIR, EMFILE, ENODEV, ENOENT, ENOMEM, ENOSPC, ENOTDIR,
    ENOTEMPTY, EPERM, EROFS, EXDEV,
};
use morpheus_foundation::storage::{
    FS_AUTO, FS_FAT32, FS_HELIX, FS_NONE, FS_UNKNOWN, MNT_RDONLY, MNT_STAGED, VOLUME_NONE,
};
use registry::{
    DeviceEntry, DeviceRegistry, MountEntry, MountTable, RamBacking, Volume, VolumeRegistry,
};
use staging::{StageAccount, StagedRam};

/// Replaces `FsGlobal`: the three registries + staging accounting, all under one
/// lock (spec §3). A single static, mirroring the old `fs_global` pattern.
pub struct StorageGlobal {
    pub devices: DeviceRegistry,
    pub volumes: VolumeRegistry,
    pub mounts: MountTable,
    pub stage: StageAccount,
}

impl StorageGlobal {
    pub const fn new() -> Self {
        Self {
            devices: DeviceRegistry::new(),
            volumes: VolumeRegistry::new(),
            mounts: MountTable::new(),
            stage: StageAccount::new(),
        }
    }
}

impl StorageGlobal {
    /// Resolve `path` to its mount, returning that mount's backend and device as
    /// disjoint borrows for one-shot op dispatch (spec §7 path-op flow). The two
    /// borrows come from distinct registry fields, so they don't alias.
    pub fn resolve_mut<'s, 'p>(
        &'s mut self,
        path: &'p str,
    ) -> Option<(u64, &'s mut MountEntry, &'s mut RawBlockDevice, &'p str)> {
        let mount_id = self.mounts.resolve(path)?;
        // The backend's namespace is rooted at its mountpoint, so it must see a
        // mount-relative path, not the absolute one (read mp_len, drop the borrow,
        // then take the disjoint mut borrows below).
        let mp_len = self.mounts.get(mount_id)?.mount_point_len as usize;
        let rel = mount_relative(path, mp_len);
        let m = self.mounts.get_mut(mount_id)?;
        let device_id = m.device_id;
        let dev = self.devices.get_mut(device_id)?;
        Some((mount_id, m, &mut dev.device, rel))
    }

    /// Like `resolve_mut`, but for a path op keyed off an already-open fd's
    /// cached `mount_id` (read/write/close/seek). Returns `None` (→ EBADF) when
    /// the mount is gone — e.g. a `MNT_FORCE` umount bumped the slab generation.
    pub fn mount_dev_mut(
        &mut self,
        mount_id: u64,
    ) -> Option<(&mut MountEntry, &mut RawBlockDevice)> {
        let m = self.mounts.get_mut(mount_id)?;
        let device_id = m.device_id;
        let dev = self.devices.get_mut(device_id)?;
        Some((m, &mut dev.device))
    }
}

impl Default for StorageGlobal {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip a mount's prefix from an absolute path, yielding the backend-relative
/// path. Root ("/") passes through unchanged; "/mnt" + "/mnt/a" → "/a"; an exact
/// mountpoint hit → "/". `mp_len` is the mount point's byte length, which
/// `resolve` already matched as a component-boundary prefix of `path`, so the
/// slice is always on a char boundary (the `None` arm can't fire in practice).
pub fn mount_relative(path: &str, mp_len: usize) -> &str {
    if mp_len <= 1 {
        return path;
    }
    match path.get(mp_len..) {
        None | Some("") => "/",
        Some(rest) => rest,
    }
}

static mut STORAGE: StorageGlobal = StorageGlobal::new();

/// SMP serialization for the three registries (generalizes the old `VFS_LOCK`;
/// aliasing the static across cores corrupts the subsystem). The staging copy
/// happens outside this lock (spec §7); no sleeping while held.
pub static STORAGE_LOCK: RawSpinLock = RawSpinLock::new();

pub struct StorageGuard {
    pub g: &'static mut StorageGlobal,
}

impl Drop for StorageGuard {
    fn drop(&mut self) {
        STORAGE_LOCK.unlock();
    }
}

/// Acquire the subsystem lock and a `&mut StorageGlobal`. Drop the guard to
/// release. Mirrors `vfs_lock`.
///
/// # Safety
/// Caller must not hold `STORAGE_LOCK` already (it is not reentrant).
pub unsafe fn lock() -> StorageGuard {
    STORAGE_LOCK.lock();
    // SAFETY: the lock serializes all access to the static; the guard's lifetime
    // bounds the borrow and releases on drop.
    let g = &mut *core::ptr::addr_of_mut!(STORAGE);
    StorageGuard { g }
}

/// The single `VfsError → errno` table (spec §4; generalizes the old
/// `helix_err_to_errno`). Every backend funnels here.
pub fn vfs_err_to_errno(e: VfsError) -> u64 {
    match e {
        VfsError::NotFound => ENOENT,
        VfsError::Exists => EEXIST,
        VfsError::NotDir => ENOTDIR,
        VfsError::IsDir => EISDIR,
        VfsError::NotEmpty => ENOTEMPTY,
        VfsError::BadFd => EBADF,
        VfsError::TooManyOpen => EMFILE,
        VfsError::ReadOnly => EROFS,
        VfsError::NoSpace => ENOSPC,
        VfsError::Io => EIO,
        VfsError::Inval => EINVAL,
        VfsError::Unsupported => EINVAL,
        VfsError::Perm => EPERM,
        VfsError::NameTooLong => EINVAL,
        VfsError::CrossDevice => EXDEV,
        VfsError::Busy => EBUSY,
        VfsError::NoDev => ENODEV,
    }
}

/// Sniff the filesystem at partition LBA `lba_start` (spec §3 layer 2): Helix
/// superblock magic, else FAT32 boot-sector markers, else `FS_UNKNOWN`. Reads one
/// sector; any I/O error → `FS_NONE`.
pub fn detect_fs(dev: &mut RawBlockDevice, lba_start: u64) -> u32 {
    let bs = dev.block_size().to_u32() as usize;
    if bs == 0 {
        return FS_NONE;
    }
    let mut sec = alloc::vec![0u8; bs];
    if dev.read_blocks(Lba(lba_start), &mut sec).is_err() {
        return FS_NONE;
    }
    // Helix superblock starts at partition block 0 with the magic.
    if sec.len() >= 8 && sec[..8] == morpheus_helix::types::HELIX_MAGIC {
        return FS_HELIX;
    }
    // FAT32: 0x55AA boot sig + a plausible bytes-per-sector at offset 11.
    if sec.len() >= 512 && sec[510] == 0x55 && sec[511] == 0xAA {
        let bps = u16::from_le_bytes([sec[11], sec[12]]) as u32;
        if matches!(bps, 512 | 1024 | 2048 | 4096) {
            return FS_FAT32;
        }
    }
    FS_UNKNOWN
}

/// Outcome of constructing a backend over a device.
fn build_backend(
    fs_type: u32,
    dev: &mut RawBlockDevice,
    lba_start: u64,
    block_size: u32,
    read_only: bool,
) -> Result<(MountedFs, u32), VfsError> {
    let resolved = if fs_type == FS_AUTO {
        detect_fs(dev, lba_start)
    } else {
        fs_type
    };
    match resolved {
        FS_HELIX => {
            let engine = morpheus_helix::HelixFs::mount(dev, lba_start, block_size)
                .map_err(backends::helix_err_pub)?;
            Ok((
                MountedFs::Helix(HelixAdapter::new(engine, read_only)),
                FS_HELIX,
            ))
        },
        FS_FAT32 => {
            let fat = Fat32Adapter::mount(dev, lba_start)?;
            Ok((MountedFs::Fat32(fat), FS_FAT32))
        },
        _ => Err(VfsError::Inval),
    }
}

/// Format a fresh empty Helix FS over a RAM region then mount it (used for
/// `VOLUME_NONE` / tmpfs and the staged-from-nothing root).
fn build_fresh_helix(
    dev: &mut RawBlockDevice,
    lba_start: u64,
    lba_count: u64,
    block_size: u32,
    read_only: bool,
) -> Result<MountedFs, VfsError> {
    let engine = morpheus_helix::HelixFs::format_and_mount(
        dev, lba_start, lba_count, block_size, "ram", [0u8; 16],
    )
    .map_err(backends::helix_err_pub)?;
    Ok(MountedFs::Helix(HelixAdapter::new(engine, read_only)))
}

/// Mount request (spec §5 axes): source × residency × fs_type. `aux` = required
/// size when `source == VOLUME_NONE`; optional stage-size cap otherwise.
pub struct MountReq {
    pub source_volume_id: u64,
    pub mount_point: [u8; 256],
    pub mount_point_len: u16,
    pub fs_type: u32,
    pub flags: u32,
    pub aux: u64,
    /// Owning pid (0 = kernel/persistent); drives reclamation and skips policy
    /// caps when `privileged`.
    pub pid: u32,
    pub privileged: bool,
}

/// Mount per spec §7. Two-phase for staged mounts: the multi-MB source copy runs
/// **outside** `STORAGE_LOCK` so it can't stall other FS ops. Returns `mount_id`
/// or an errno. Caller must NOT hold `STORAGE_LOCK`.
pub fn mount(req: &MountReq) -> Result<u64, u64> {
    let mp_len = (req.mount_point_len as usize).min(256);
    let mp = core::str::from_utf8(&req.mount_point[..mp_len]).map_err(|_| EINVAL)?;
    if mp.is_empty() || !mp.starts_with('/') {
        return Err(EINVAL);
    }
    let staged = req.flags & MNT_STAGED != 0 || req.source_volume_id == VOLUME_NONE;
    let read_only = req.flags & MNT_RDONLY != 0;

    if staged {
        mount_staged(req, mp, read_only)
    } else {
        mount_direct(req, mp, read_only)
    }
}

/// Direct (live) mount: the backend drives the real device in place; the disk is
/// persistent. No RAM staging, no ephemeral volume.
fn mount_direct(req: &MountReq, mp: &str, read_only: bool) -> Result<u64, u64> {
    // SAFETY: we don't hold the lock yet; single critical section.
    let guard = unsafe { lock() };
    let g = &mut *guard.g;

    if g.mounts.resolve_exact(mp).is_some() {
        return Err(EEXIST);
    }
    let vol = g.volumes.get(req.source_volume_id).ok_or(ENODEV)?;
    if vol.mounted {
        return Err(EBUSY);
    }
    let (device_id, lba_start, block_size, vol_ro) =
        (vol.device_id, vol.lba_start, vol.block_size, vol.read_only);

    let dev = g.devices.get_mut(device_id).ok_or(ENODEV)?;
    let ro = read_only || vol_ro;
    let (fs, fs_type) = build_backend(req.fs_type, &mut dev.device, lba_start, block_size, ro)
        .map_err(vfs_err_to_errno)?;

    let entry = MountEntry {
        volume_id: req.source_volume_id,
        device_id,
        fs,
        fs_type,
        flags: req.flags,
        mount_point: req.mount_point,
        mount_point_len: req.mount_point_len,
        open_fds: 0,
        ephemeral: false,
        owner_pid: req.pid,
    };
    let mount_id = g.mounts.insert(entry).ok_or(ENOMEM)?;
    if let Some(v) = g.volumes.get_mut(req.source_volume_id) {
        v.mounted = true;
    }
    Ok(mount_id)
}

/// Staged mount (spec §7 two-phase). Phase A (locked): admission + reserve +
/// allocate. Phase B (unlocked): copy the source LBA range into RAM. Phase C
/// (relocked): register the `DEV_RAM` device + ephemeral volume, build the
/// backend, insert the mount. Any failure unwinds (free RAM, restore budget).
fn mount_staged(req: &MountReq, mp: &str, read_only: bool) -> Result<u64, u64> {
    // Resolve source geometry first (no copy yet).
    let (src_device_id, src_lba_start, src_lba_count, block_size, src_ro, from_nothing) = {
        // SAFETY: brief critical section; guard drops at block end.
        let guard = unsafe { lock() };
        let g = &mut *guard.g;
        if g.mounts.resolve_exact(mp).is_some() {
            return Err(EEXIST);
        }
        if req.source_volume_id == VOLUME_NONE {
            (0u64, 0u64, 0u64, 4096u32, false, true)
        } else {
            let vol = g.volumes.get(req.source_volume_id).ok_or(ENODEV)?;
            (
                vol.device_id,
                vol.lba_start,
                vol.lba_count,
                vol.block_size,
                vol.read_only,
                false,
            )
        }
    };

    // Size to stage: aux for from-nothing; else min(aux-or-full, source bytes).
    let src_bytes = src_lba_count.checked_mul(block_size as u64).ok_or(EINVAL)?;
    let size = if from_nothing {
        if req.aux == 0 {
            return Err(EINVAL);
        }
        req.aux
    } else if req.aux != 0 {
        req.aux.min(src_bytes)
    } else {
        src_bytes
    };

    // Phase A: admission (locked).
    let ram = {
        // SAFETY: brief critical section.
        let guard = unsafe { lock() };
        let g = &mut *guard.g;
        staging::admit(&mut g.stage, req.pid, size, req.privileged)?
    };

    // Phase B: copy source → RAM (unlocked). Build a temporary MemBlockDevice
    // over the staged region.
    // SAFETY: the region was just allocated, identity-mapped, and is uniquely
    // owned here until registered.
    let (mem_box, mut ram_dev) = unsafe { staging::mem_device(&ram, block_size) };

    if !from_nothing {
        if let Err(e) =
            copy_source_into_ram(src_device_id, src_lba_start, &ram, block_size, &mut ram_dev)
        {
            unwind_ram(&ram, mem_box);
            return Err(e);
        }
    }

    // Phase C: register device + ephemeral volume, build backend, insert mount.
    // SAFETY: critical section for the registration.
    let guard = unsafe { lock() };
    let g = &mut *guard.g;

    // The mountpoint check from the geometry phase raced the unlocked copy;
    // re-check and unwind the staged RAM if someone else took it meanwhile.
    if g.mounts.resolve_exact(mp).is_some() {
        drop(mem_box);
        staging::release(&mut g.stage, &ram);
        return Err(EEXIST);
    }

    let ram_pages = ram.pages;
    let ram_phys = ram.phys_addr;
    let ram_lba_count = ram.bytes / block_size as u64;

    let dev_entry = DeviceEntry {
        device: ram_dev,
        kind: DeviceKind::Ram,
        block_size,
        lba_count: ram_lba_count,
        ram: Some(RamBacking {
            mem: mem_box,
            phys_addr: ram_phys,
            pages: ram_pages,
        }),
    };
    let device_id = match g.devices.insert(dev_entry) {
        Some(id) => id,
        None => {
            // insert failed: reclaim the box back out is impossible (moved); free
            // pages + budget directly.
            staging::release(&mut g.stage, &ram);
            return Err(ENOMEM);
        },
    };

    // Build the backend over the in-registry RAM device.
    let dev_ref = match g.devices.get_mut(device_id) {
        Some(d) => d,
        None => {
            unwind_registered_device(g, device_id, &ram);
            return Err(ENODEV);
        },
    };
    let ro = read_only || src_ro;
    let build = if from_nothing {
        build_fresh_helix(&mut dev_ref.device, 0, ram_lba_count, block_size, ro)
    } else {
        build_backend(req.fs_type, &mut dev_ref.device, 0, block_size, ro).map(|(fs, _)| fs)
    };
    let fs = match build {
        Ok(fs) => fs,
        Err(e) => {
            unwind_registered_device(g, device_id, &ram);
            return Err(vfs_err_to_errno(e));
        },
    };
    let fs_type = match &fs {
        MountedFs::Helix(_) => FS_HELIX,
        MountedFs::Fat32(_) => FS_FAT32,
    };

    // Synthesize the ephemeral volume (visible in SYS_VOLUMES; owned by the pid).
    let mut label = [0u8; 64];
    let lbl = b"staged";
    label[..lbl.len()].copy_from_slice(lbl);
    let vol = Volume {
        device_id,
        lba_start: 0,
        lba_count: ram_lba_count,
        block_size,
        partition_guid: [0u8; 16],
        detected_fs: fs_type,
        label,
        read_only: ro,
        removable: false,
        ephemeral: true,
        owner_pid: req.pid,
        mounted: true,
    };
    let volume_id = match g.volumes.insert(vol) {
        Some(id) => id,
        None => {
            // backend built but no volume slot: drop fs, free device+RAM.
            drop(fs);
            unwind_registered_device(g, device_id, &ram);
            return Err(ENOMEM);
        },
    };

    let entry = MountEntry {
        volume_id,
        device_id,
        fs,
        fs_type,
        flags: req.flags,
        mount_point: req.mount_point,
        mount_point_len: req.mount_point_len,
        open_fds: 0,
        ephemeral: true,
        owner_pid: req.pid,
    };
    match g.mounts.insert(entry) {
        Some(mount_id) => Ok(mount_id),
        None => {
            let _ = g.volumes.remove(volume_id);
            unwind_registered_device(g, device_id, &ram);
            Err(ENOMEM)
        },
    }
}

/// Copy `[src_lba_start, ..)` from the source device into the staged RAM device,
/// sector by sector, bounded by the staged byte count. Runs unlocked.
fn copy_source_into_ram(
    src_device_id: u64,
    src_lba_start: u64,
    ram: &StagedRam,
    block_size: u32,
    ram_dev: &mut RawBlockDevice,
) -> Result<(), u64> {
    let bs = block_size as usize;
    if bs == 0 {
        return Err(EINVAL);
    }
    let total_sectors = ram.bytes / block_size as u64;
    let mut buf = alloc::vec![0u8; bs];
    let mut i = 0u64;
    while i < total_sectors {
        let abs = src_lba_start.checked_add(i).ok_or(EINVAL)?;
        // Read one source sector under the lock (serialized against other FS
        // ops), then release before the RAM write — never hold the lock across
        // the whole multi-MB copy.
        {
            // SAFETY: brief critical section per sector.
            let guard = unsafe { lock() };
            let g = &mut *guard.g;
            let src = g.devices.get_mut(src_device_id).ok_or(ENODEV)?;
            if src.device.read_blocks(Lba(abs), &mut buf).is_err() {
                return Err(EIO);
            }
        }
        if ram_dev.write_blocks(Lba(i), &buf).is_err() {
            return Err(EIO);
        }
        i += 1;
    }
    Ok(())
}

/// Unwind a staged region that was never registered: drop the box, free pages +
/// budget. Needs the lock for budget accounting.
fn unwind_ram(ram: &StagedRam, mem_box: alloc::boxed::Box<morpheus_block_types::MemBlockDevice>) {
    drop(mem_box);
    // SAFETY: brief critical section.
    let guard = unsafe { lock() };
    staging::release(&mut guard.g.stage, ram);
}

/// Unwind a device already inserted into the registry (later phase failed):
/// remove it (drops its `RamBacking` box) and free pages + budget.
fn unwind_registered_device(g: &mut StorageGlobal, device_id: u64, ram: &StagedRam) {
    let _ = g.devices.remove(device_id);
    staging::release(&mut g.stage, ram);
}

/// Umount per spec §7. Exact mountpoint only (sub-paths rejected); `/` → `EBUSY`.
/// Busy unless `MNT_FORCE` (revoke). Ephemeral mounts free their RAM + restore
/// the owner's staging budget. Caller must NOT hold `STORAGE_LOCK`.
pub fn umount(mount_point: &str, flags: u32) -> Result<(), u64> {
    if mount_point == "/" {
        return Err(EBUSY);
    }
    // SAFETY: single critical section.
    let guard = unsafe { lock() };
    let g = &mut *guard.g;

    let mount_id = g.mounts.resolve_exact(mount_point).ok_or(ENOENT)?;
    let force = flags & morpheus_foundation::storage::MNT_FORCE != 0;

    {
        let m = g.mounts.get(mount_id).ok_or(ENOENT)?;
        if m.open_fds > 0 && !force {
            return Err(EBUSY);
        }
    }
    // MNT_FORCE needs no explicit per-fd marking: removing the MountEntry bumps
    // the slab generation, so every still-open fd's cached `mount_id` is now
    // stale and its next op's `mounts.get(mount_id)` returns None (→ EBADF).
    teardown_mount(g, mount_id);
    Ok(())
}

/// Boot-only: tear down the current `/` mount (bypassing the `/`→EBUSY guard) so
/// the root-selection policy can reject a candidate that lacks `/bin/init` and try
/// the next. Frees staged RAM + restores budget for an ephemeral root. Only sound
/// during boot, before any process has opened an fd against `/`. No-op if `/` is
/// unmounted. Caller must NOT hold `STORAGE_LOCK`.
pub fn unmount_root_privileged() {
    // SAFETY: single critical section.
    let guard = unsafe { lock() };
    let g = &mut *guard.g;
    if let Some(mount_id) = g.mounts.resolve_exact("/") {
        teardown_mount(g, mount_id);
    }
}

/// Sync + drop a mount, freeing its volume/device/RAM if ephemeral and restoring
/// the staging budget. Assumes the lock is held.
fn teardown_mount(g: &mut StorageGlobal, mount_id: u64) {
    let mut entry = match g.mounts.remove(mount_id) {
        Some(e) => e,
        None => return,
    };
    // Best-effort sync against the still-registered device.
    if let Some(dev) = g.devices.get_mut(entry.device_id) {
        let _ = entry.fs.sync(&mut dev.device);
    }
    let volume_id = entry.volume_id;
    let device_id = entry.device_id;
    let ephemeral = entry.ephemeral;
    let owner_pid = entry.owner_pid;
    drop(entry); // drops MountedFs backend

    if ephemeral {
        // Free RAM + restore budget, then drop synth volume + device.
        if let Some(dev) = g.devices.remove(device_id) {
            if let Some(ram) = dev.ram {
                let staged = StagedRam {
                    phys_addr: ram.phys_addr,
                    pages: ram.pages,
                    bytes: ram.pages.saturating_mul(staging_page_size()),
                    owner_pid,
                };
                staging::release(&mut g.stage, &staged);
            }
        }
        let _ = g.volumes.remove(volume_id);
    } else if let Some(v) = g.volumes.get_mut(volume_id) {
        v.mounted = false;
    }
}

fn staging_page_size() -> u64 {
    let ps = crate::global::hal().phys().page_size();
    if ps == 0 {
        4096
    } else {
        ps
    }
}

/// Register a live (non-RAM) block device in the device registry, keeping its
/// driver alive in the caller's address space (spec §7 boot population: drivers
/// are KEPT, not dropped, so Direct mounts work at runtime). The caller owns the
/// backing driver/ctx that `device` bridges; it must outlive the registration.
/// Returns the generational `device_id`. Caller must NOT hold `STORAGE_LOCK`.
pub fn register_boot_device(
    device: RawBlockDevice,
    kind: DeviceKind,
    block_size: u32,
    lba_count: u64,
) -> Option<u64> {
    // SAFETY: single critical section; not holding the lock on entry.
    let guard = unsafe { lock() };
    guard.g.devices.insert(DeviceEntry {
        device,
        kind,
        block_size,
        lba_count,
        ram: None,
    })
}

/// Register a discovered volume (spec §3 layer 2) against an already-registered
/// device. Returns the generational `volume_id`. Caller must NOT hold
/// `STORAGE_LOCK`.
#[allow(clippy::too_many_arguments)]
pub fn register_volume(
    device_id: u64,
    lba_start: u64,
    lba_count: u64,
    block_size: u32,
    partition_guid: [u8; 16],
    detected_fs: u32,
    label: [u8; 64],
    read_only: bool,
    removable: bool,
) -> Option<u64> {
    // SAFETY: single critical section.
    let guard = unsafe { lock() };
    guard.g.volumes.insert(Volume {
        device_id,
        lba_start,
        lba_count,
        block_size,
        partition_guid,
        detected_fs,
        label,
        read_only,
        removable,
        ephemeral: false,
        owner_pid: 0,
        mounted: false,
    })
}

/// True iff `path` resolves and stats on the currently-mounted tree (boot uses
/// this for the `/bin/init` root-selection policy). Any miss → false.
pub fn path_exists(path: &str) -> bool {
    // SAFETY: single critical section.
    let guard = unsafe { lock() };
    let g = &mut *guard.g;
    match g.resolve_mut(path) {
        Some((_, m, dev, rel)) => m.fs.stat(dev, rel).is_ok(),
        None => false,
    }
}

/// Privileged mkdir on the mounted tree (boot directory bootstrap). Treats an
/// already-existing path as success. Returns an errno on real failure.
pub fn mkdir_root(path: &str, ts: u64) -> Result<(), u64> {
    // SAFETY: single critical section.
    let guard = unsafe { lock() };
    let g = &mut *guard.g;
    let (_, m, dev, rel) = g.resolve_mut(path).ok_or(ENOENT)?;
    match m.fs.mkdir(dev, rel, ts) {
        Ok(()) => Ok(()),
        Err(VfsError::Exists) => Ok(()),
        Err(e) => Err(vfs_err_to_errno(e)),
    }
}

/// Process reap (spec §7 reclamation; called from `wait.rs`). For dying `pid`:
/// close all its fds (decrement per-mount `open_fds`), then auto-umount every
/// ephemeral mount it owns (freeing RAM + restoring budget). Direct/global mounts
/// survive. Takes the fd table so the caller drops it afterward.
pub fn reap_process(pid: u32, fd_table: &mut fs_api::FdTable) {
    // SAFETY: single critical section.
    let guard = unsafe { lock() };
    let g = &mut *guard.g;

    // (1) close fds: decrement each referenced mount's refcount.
    for (_, fd) in fd_table.iter() {
        if let Some(m) = g.mounts.get_mut(fd.mount_id) {
            m.open_fds = m.open_fds.saturating_sub(1);
        }
    }

    // (2) auto-umount ephemeral mounts owned by this pid.
    let mut victims: alloc::vec::Vec<u64> = alloc::vec::Vec::new();
    for (id, m) in g.mounts.iter() {
        if m.ephemeral && m.owner_pid == pid {
            victims.push(id);
        }
    }
    for id in victims {
        teardown_mount(g, id);
    }
}
