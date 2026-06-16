//! `#[repr(C)]` types forming the stable ABI between the helix surface and libmorpheus.

/// `stat(path, &mut buf)` writes this into `buf`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FileStat {
    /// Full-path hash (helix key).
    pub key: u64,
    pub size: u64,
    pub is_dir: bool,
    /// TSC nanoseconds since boot.
    pub created_ns: u64,
    /// TSC nanoseconds since boot.
    pub modified_ns: u64,
    pub version_count: u32,
    /// Helix log sequence number.
    pub lsn: u64,
    /// Creation LSN.
    pub first_lsn: u64,
    /// Entry flags.
    pub flags: u32,
}

/// One entry from `versions(path, &mut buf, max)` — SYS_VERSIONS. One record per
/// log entry that touched the path, oldest-to-newest. `op` mirrors the HelixFS
/// `LogOp` discriminant (Write=1, Append=2, Delete=3, Rename=5, SetMeta=6,
/// Truncate=0x0D, …); `lsn` is the version's log sequence number and can be
/// passed to a time-travel open (`O_AT_LSN`).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FileVersion {
    /// Log sequence number of this version.
    pub lsn: u64,
    /// TSC nanoseconds since boot when the record was written.
    pub timestamp_ns: u64,
    /// HelixFS `LogOp` discriminant.
    pub op: u32,
    pub _pad: u32,
}

impl FileVersion {
    pub const fn zeroed() -> Self {
        Self {
            lsn: 0,
            timestamp_ns: 0,
            op: 0,
            _pad: 0,
        }
    }
}

/// One entry from `readdir(fd, &mut buf, max_entries)`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DirEntry {
    /// Filename — last path component only. Length in `name_len`.
    pub name: [u8; 256],
    pub name_len: u16,
    pub is_dir: bool,
    /// 0 for directories.
    pub size: u64,
    pub modified_ns: u64,
    pub version_count: u32,
}

impl DirEntry {
    pub const fn zeroed() -> Self {
        Self {
            name: [0u8; 256],
            name_len: 0,
            is_dir: false,
            size: 0,
            modified_ns: 0,
            version_count: 0,
        }
    }

    /// The filename as a `&str`, bounded by `name_len` (clamped to the buffer); lossy-empty on
    /// bad UTF-8.
    pub fn name_str(&self) -> &str {
        let len = (self.name_len as usize).min(self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

// `[u8; 256]` is past the array-`Default` bound, so derive can't reach it — the zeroed value
// is the honest empty entry.
impl Default for DirEntry {
    fn default() -> Self {
        Self::zeroed()
    }
}

// ── Syscall boundary structs ────────────────────────────────────────────────
// Every `#[repr(C)]` struct whose bytes cross a syscall lives here exactly once.
// The kernel handler and libmorpheus both `use` these, so the two sides cannot
// drift: there is only one definition. (Previously each was declared twice with
// a "must match byte-for-byte" comment — comments don't compile-check.)

/// Per-CPU bound for `SysInfo::per_core_idle_tsc`.
pub const SYSINFO_MAX_CPUS: usize = 16;

/// `sysinfo(&mut buf)` — SYS_SYSINFO.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SysInfo {
    pub total_mem: u64,
    pub free_mem: u64,
    pub num_procs: u32,
    pub cpu_count: u32,
    pub uptime_ticks: u64,
    pub tsc_freq: u64,
    pub heap_total: u64,
    pub heap_used: u64,
    pub heap_free: u64,
    pub sched_ticks: u64,
    pub idle_tsc: u64,
    pub per_core_idle_tsc: [u64; SYSINFO_MAX_CPUS],
}

impl SysInfo {
    pub const fn zeroed() -> Self {
        Self {
            total_mem: 0,
            free_mem: 0,
            num_procs: 0,
            cpu_count: 0,
            uptime_ticks: 0,
            tsc_freq: 0,
            heap_total: 0,
            heap_used: 0,
            heap_free: 0,
            sched_ticks: 0,
            idle_tsc: 0,
            per_core_idle_tsc: [0; SYSINFO_MAX_CPUS],
        }
    }

    pub fn uptime_ms(&self) -> u64 {
        if self.tsc_freq == 0 {
            return 0;
        }
        (self.uptime_ticks as u128 * 1000 / self.tsc_freq as u128) as u64
    }

    /// Uptime in whole seconds, from the raw TSC and its frequency (0 if uncalibrated).
    pub fn uptime_s(&self) -> u64 {
        if self.tsc_freq == 0 {
            return 0;
        }
        (self.uptime_ticks as u128 / self.tsc_freq as u128) as u64
    }
}

/// One row from `ps(&mut buf, max)` — SYS_PS. `state`: 0=Ready 1=Running
/// 2=Blocked 3=Zombie 4=Terminated. `name` is NUL-terminated.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct PsEntry {
    pub pid: u32,
    pub ppid: u32,
    pub state: u32,
    pub priority: u32,
    pub cpu_ticks: u64,
    pub cpu_tsc: u64,
    pub pages_alloc: u64,
    pub name: [u8; 32],
}

impl PsEntry {
    pub const fn zeroed() -> Self {
        Self {
            pid: 0,
            ppid: 0,
            state: 0,
            priority: 0,
            cpu_ticks: 0,
            cpu_tsc: 0,
            pages_alloc: 0,
            name: [0u8; 32],
        }
    }

    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

/// `cpuid(leaf, subleaf, &mut result)` — SYS_CPUID.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// `rdtsc(&mut result)` — SYS_RDTSC.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TscResult {
    pub tsc: u64,
    pub frequency: u64,
}

/// One row from `memmap(&mut buf, max)` — SYS_MEMMAP. `mem_type` is the UEFI
/// memory type.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct MemmapEntry {
    pub phys_start: u64,
    pub num_pages: u64,
    pub mem_type: u32,
    pub _pad: u32,
}

/// `persist_info(&mut buf)` — SYS_PERSIST_INFO. `backend_flags` bit0 = HelixFS.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct PersistInfo {
    pub backend_flags: u32,
    pub _pad0: u32,
    pub num_keys: u64,
    pub used_bytes: u64,
}

/// `pe_info(path, &mut buf)` — SYS_PE_INFO. `format`: 0=unknown 1=ELF64 2=PE32+;
/// `arch`: 0=unknown 1=x86_64 2=aarch64 3=arm.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct BinaryInfo {
    pub format: u32,
    pub arch: u32,
    pub entry_point: u64,
    pub image_base: u64,
    pub image_size: u64,
    pub num_sections: u32,
    pub _pad0: u32,
}

/// One row from `win_surface_list(&mut buf, max)` — SYS_WIN_SURFACE_LIST.
/// `format`: 0=RGBX 1=BGRX.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SurfaceEntry {
    pub pid: u32,
    pub _pad: u32,
    pub phys_addr: u64,
    pub pages: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub dirty: u32,
    pub _pad2: u32,
}

/// `nic_info(&mut buf)` — SYS_NIC_INFO. `mac` is 6 bytes padded to 8.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NicInfo {
    pub mac: [u8; 8],
    pub link_up: u32,
    pub present: u32,
}

/// `fb_info(&mut buf)` — SYS_FB_INFO. `format`: 0=RGBX 1=BGRX.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FbInfo {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}

/// NIC hardware stats (SYS_NET_CFG / NIC_CTRL_STATS).
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct NicHwStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    pub rx_dropped: u64,
    pub rx_crc_errors: u64,
    pub collisions: u64,
}

/// Network config snapshot (SYS_NET_CFG / NET_CFG_GET). `hostname` NUL-terminated.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetConfigInfo {
    pub state: u32,
    pub flags: u32,
    pub ipv4_addr: u32,
    pub prefix_len: u8,
    pub _pad0: [u8; 3],
    pub gateway: u32,
    pub dns_primary: u32,
    pub dns_secondary: u32,
    pub mac: [u8; 6],
    pub _pad1: [u8; 2],
    pub mtu: u32,
    pub hostname: [u8; 64],
}

impl NetConfigInfo {
    pub const fn zeroed() -> Self {
        Self {
            state: 0,
            flags: 0,
            ipv4_addr: 0,
            prefix_len: 0,
            _pad0: [0; 3],
            gateway: 0,
            dns_primary: 0,
            dns_secondary: 0,
            mac: [0; 6],
            _pad1: [0; 2],
            mtu: 0,
            hostname: [0; 64],
        }
    }
}

// All-zero is the honest "unconfigured / no NIC" state the kernel itself writes when no net
// stack is registered, so a zeroed buffer is a valid `NetConfigInfo`. `[u8; 64]` is past the
// array-`Default` bound, so derive can't reach it.
impl Default for NetConfigInfo {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Network stack stats (SYS_NET_POLL / NET_POLL_STATS).
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct NetStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    pub tcp_active: u32,
    pub _pad: u32,
}

/// `SYS_NET(NET_UDP_SEND_TO, handle, &desc, 0)` — pointed to by a3. 24 bytes.
/// `buf` is a userland address the kernel validates and reads.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct UdpSendDesc {
    pub ip: u32,
    pub port: u16,
    pub _pad: u16,
    pub buf: *const u8,
    pub len: u64,
}

/// `SYS_NET(NET_UDP_RECV_FROM, handle, &desc, 0)` — pointed to by a3. 24 bytes.
/// Kernel reads `buf`/`buf_len`, writes back `src_ip`/`src_port` on success.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct UdpRecvDesc {
    pub buf: *mut u8,
    pub buf_len: u64,
    pub src_ip: u32,
    pub src_port: u16,
    pub _pad: u16,
}

/// One row from `volumes(&mut buf, max)` — SYS_VOLUMES. lsblk-style projection of
/// a `VolumeRegistry` entry. `device_kind` is `DEV_*`, `fs_type` is detected
/// (`FS_NONE|FS_HELIX|FS_FAT32|FS_UNKNOWN`), `flags` is `VOL_*`. Both ids are
/// generational handles (see `storage::pack`).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VolumeInfo {
    pub volume_id: u64,
    pub device_id: u64,
    pub device_kind: u32,
    pub fs_type: u32,
    pub lba_start: u64,
    pub lba_count: u64,
    pub block_size: u32,
    pub flags: u32,
    pub partition_guid: [u8; 16],
    pub label: [u8; 64],
}

impl VolumeInfo {
    pub const fn zeroed() -> Self {
        Self {
            volume_id: 0,
            device_id: 0,
            device_kind: 0,
            fs_type: 0,
            lba_start: 0,
            lba_count: 0,
            block_size: 0,
            flags: 0,
            partition_guid: [0u8; 16],
            label: [0u8; 64],
        }
    }

    /// The NUL-terminated `label` as a `&str` (lossy-empty on bad UTF-8 / no label).
    pub fn label_str(&self) -> &str {
        let end = self.label.iter().position(|&b| b == 0).unwrap_or(self.label.len());
        core::str::from_utf8(&self.label[..end]).unwrap_or("")
    }

    /// Raw capacity in bytes from geometry (`lba_count * block_size`), overflow-saturating.
    pub fn size_bytes(&self) -> u64 {
        self.lba_count.saturating_mul(self.block_size as u64)
    }
}

// `[u8; 64]` is past the array-`Default` bound, so derive can't reach it.
impl Default for VolumeInfo {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// One row from `mounts(&mut buf, max)` — SYS_MOUNTS. `mount_point` is the
/// absolute path, length in `mount_point_len`; `flags` is `MNT_*`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct MountInfo {
    pub mount_id: u64,
    pub volume_id: u64,
    pub fs_type: u32,
    pub flags: u32,
    pub mount_point: [u8; 256],
    pub mount_point_len: u16,
    pub _pad: [u8; 6],
}

impl MountInfo {
    pub const fn zeroed() -> Self {
        Self {
            mount_id: 0,
            volume_id: 0,
            fs_type: 0,
            flags: 0,
            mount_point: [0u8; 256],
            mount_point_len: 0,
            _pad: [0u8; 6],
        }
    }

    /// The mount point as a `&str`, bounded by `mount_point_len` (clamped to the buffer) and
    /// stopping at any embedded NUL (lossy-empty on bad UTF-8).
    pub fn mount_point_str(&self) -> &str {
        let n = (self.mount_point_len as usize).min(self.mount_point.len());
        let bytes = &self.mount_point[..n];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(n);
        core::str::from_utf8(&bytes[..end]).unwrap_or("")
    }
}

// `[u8; 256]` is past the array-`Default` bound, so derive can't reach it.
impl Default for MountInfo {
    fn default() -> Self {
        Self::zeroed()
    }
}
