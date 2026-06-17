//! The three registries (spec §3). Each wraps a generational `Slab`, so ids are
//! stable across removals and stale ids fail cleanly. `DeviceEntry` keeps live
//! drivers alive in the same address space so Direct mounts work at runtime.

use super::backends::MountedFs;
use super::slab::Slab;
use morpheus_block_types::{DeviceKind, MemBlockDevice, RawBlockDevice};

/// One registered block device (live driver or RAM region). The `RawBlockDevice`
/// is the universal handle; `mem` is `Some` only for synthesized RAM volumes so
/// reclamation can free the backing pages on the last umount.
pub struct DeviceEntry {
    pub device: RawBlockDevice,
    pub kind: DeviceKind,
    pub block_size: u32,
    pub lba_count: u64,
    /// RAM backing for an ephemeral device: (phys_addr, page_count). The
    /// `MemBlockDevice` whose pointer `device` wraps lives here so it outlives the
    /// `RawBlockDevice` (which only holds a raw ctx pointer).
    pub ram: Option<RamBacking>,
}

/// Backing store + accounting for a synthesized RAM device.
pub struct RamBacking {
    /// Heap-owned `MemBlockDevice`; `RawBlockDevice::ctx` points into this box, so
    /// it must not move or drop while the device is registered.
    pub mem: alloc::boxed::Box<MemBlockDevice>,
    pub phys_addr: u64,
    pub pages: u64,
}

pub struct DeviceRegistry {
    slab: Slab<DeviceEntry>,
}

impl DeviceRegistry {
    pub const fn new() -> Self {
        Self { slab: Slab::new() }
    }
    pub fn insert(&mut self, e: DeviceEntry) -> Option<u64> {
        self.slab.insert(e)
    }
    pub fn get(&self, id: u64) -> Option<&DeviceEntry> {
        self.slab.get(id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut DeviceEntry> {
        self.slab.get_mut(id)
    }
    pub fn remove(&mut self, id: u64) -> Option<DeviceEntry> {
        self.slab.remove(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (u64, &DeviceEntry)> {
        self.slab.iter()
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A byte range on a device that may hold a filesystem (spec §3 layer 2).
pub struct Volume {
    pub device_id: u64,
    pub lba_start: u64,
    pub lba_count: u64,
    pub block_size: u32,
    pub partition_guid: [u8; 16],
    /// Detected FS (`FS_NONE|FS_HELIX|FS_FAT32|FS_UNKNOWN`).
    pub detected_fs: u32,
    pub label: [u8; 64],
    pub read_only: bool,
    pub removable: bool,
    /// Synthesized RAM volume backing a staged mount (spec §5). Owned by
    /// `owner_pid`; reclaimed on umount or reap.
    pub ephemeral: bool,
    /// Creating pid for an ephemeral volume; 0 = kernel/persistent.
    pub owner_pid: u32,
    pub mounted: bool,
}

pub struct VolumeRegistry {
    slab: Slab<Volume>,
}

impl VolumeRegistry {
    pub const fn new() -> Self {
        Self { slab: Slab::new() }
    }
    pub fn insert(&mut self, v: Volume) -> Option<u64> {
        self.slab.insert(v)
    }
    pub fn get(&self, id: u64) -> Option<&Volume> {
        self.slab.get(id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Volume> {
        self.slab.get_mut(id)
    }
    pub fn remove(&mut self, id: u64) -> Option<Volume> {
        self.slab.remove(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (u64, &Volume)> {
        self.slab.iter()
    }
}

impl Default for VolumeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Binds a `MountedFs` at a path (spec §3 layer 4). Caches `device_id` so a path
/// op dispatches with one index lookup. `open_fds` is the busy refcount (umount
/// rejects a busy mount unless `MNT_FORCE`).
pub struct MountEntry {
    pub volume_id: u64,
    pub device_id: u64,
    pub fs: MountedFs,
    pub fs_type: u32,
    pub flags: u32,
    pub mount_point: [u8; 256],
    pub mount_point_len: u16,
    pub open_fds: u32,
    /// True if the backing volume is ephemeral (staged); drives reap/umount
    /// reclamation.
    pub ephemeral: bool,
    pub owner_pid: u32,
}

impl MountEntry {
    pub fn path(&self) -> &str {
        let len = (self.mount_point_len as usize).min(self.mount_point.len());
        core::str::from_utf8(&self.mount_point[..len]).unwrap_or("")
    }
}

pub struct MountTable {
    slab: Slab<MountEntry>,
}

impl MountTable {
    pub const fn new() -> Self {
        Self { slab: Slab::new() }
    }
    pub fn insert(&mut self, m: MountEntry) -> Option<u64> {
        self.slab.insert(m)
    }
    pub fn get(&self, id: u64) -> Option<&MountEntry> {
        self.slab.get(id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut MountEntry> {
        self.slab.get_mut(id)
    }
    pub fn remove(&mut self, id: u64) -> Option<MountEntry> {
        self.slab.remove(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (u64, &MountEntry)> {
        self.slab.iter()
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u64, &mut MountEntry)> {
        self.slab.iter_mut()
    }

    /// Longest-prefix mount resolution (spec §7). Returns the `mount_id` of the
    /// most-specific mount whose point is a path-component prefix of `path`.
    pub fn resolve(&self, path: &str) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut best_len = 0usize;
        for (id, m) in self.slab.iter() {
            let mp = m.path();
            if path_has_prefix(path, mp) && mp.len() >= best_len {
                best = Some(id);
                best_len = mp.len();
            }
        }
        best
    }

    /// Exact-mountpoint match (umount; sub-paths must be rejected).
    pub fn resolve_exact(&self, path: &str) -> Option<u64> {
        self.slab
            .iter()
            .find(|(_, m)| m.path() == path)
            .map(|(id, _)| id)
    }
}

impl Default for MountTable {
    fn default() -> Self {
        Self::new()
    }
}

/// True iff `mp` is `path` or a parent directory of `path` on a component
/// boundary. `/` matches everything; `/a` matches `/a` and `/a/b` but not `/ab`.
fn path_has_prefix(path: &str, mp: &str) -> bool {
    if mp == "/" {
        return path.starts_with('/');
    }
    if !path.starts_with(mp) {
        return false;
    }
    match path.as_bytes().get(mp.len()) {
        None => true,
        Some(&b) => b == b'/',
    }
}
