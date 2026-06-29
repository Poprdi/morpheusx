//! `#[repr(C)]` types forming the stable ABI between the helix surface and libmorpheus.

/// `stat(path, &mut buf)` / `fstat(fd, &mut buf)` writes this into `buf`.
///
/// `mode` carries the `S_IFMT` type bits + low `0o7777` perm bits so std's
/// `FileType`/`Permissions` are first-class. Readers trust `min(struct_size,
/// size_of::<Self>())` and treat the `reserved` tail as future ino/dev/rdev/blocks.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FileStat {
    pub version: u16,
    pub struct_size: u16,
    pub mode: u32,
    /// Full-path hash (helix key).
    pub key: u64,
    pub size: u64,
    /// TSC nanoseconds since boot.
    pub created_ns: u64,
    /// TSC nanoseconds since boot.
    pub modified_ns: u64,
    /// TSC nanoseconds since boot (atime).
    pub accessed_ns: u64,
    /// Helix log sequence number.
    pub lsn: u64,
    /// Creation LSN.
    pub first_lsn: u64,
    pub nlink: u64,
    pub uid: u32,
    pub gid: u32,
    pub version_count: u32,
    pub _pad0: u32,
    pub reserved: [u64; 4],
}

impl FileStat {
    /// `S_IFMT` type bits decode to a directory.
    pub fn is_dir(&self) -> bool {
        self.mode & crate::flags::mode::S_IFMT == crate::flags::mode::S_IFDIR
    }

    /// `S_IFMT` type bits decode to a regular file.
    pub fn is_file(&self) -> bool {
        self.mode & crate::flags::mode::S_IFMT == crate::flags::mode::S_IFREG
    }
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

/// One entry from `readdir(fd, &mut buf, max_entries)`. `d_type` is the Linux
/// `DT_*` file type so readdir reports type without a follow-up stat.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DirEntry {
    pub version: u16,
    pub struct_size: u16,
    pub name_len: u16,
    pub d_type: u8,
    pub _pad0: u8,
    pub version_count: u32,
    pub _pad1: u32,
    /// 0 for directories.
    pub size: u64,
    pub modified_ns: u64,
    /// Filename — last path component only. Length in `name_len`.
    pub name: [u8; 256],
    pub reserved: [u8; 16],
}

impl DirEntry {
    pub const fn zeroed() -> Self {
        Self {
            version: 0,
            struct_size: 0,
            name_len: 0,
            d_type: 0,
            _pad0: 0,
            version_count: 0,
            _pad1: 0,
            size: 0,
            modified_ns: 0,
            name: [0u8; 256],
            reserved: [0u8; 16],
        }
    }

    /// The filename as a `&str`, bounded by `name_len` (clamped to the buffer); lossy-empty on
    /// bad UTF-8.
    pub fn name_str(&self) -> &str {
        let len = (self.name_len as usize).min(self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    pub fn is_dir(&self) -> bool {
        self.d_type == crate::flags::dirent_type::DT_DIR
    }
}

// `[u8; 256]` is past the array-`Default` bound, so derive can't reach it — the zeroed value
// is the honest empty entry.
impl Default for DirEntry {
    fn default() -> Self {
        Self::zeroed()
    }
}

// Every `#[repr(C)]` struct whose bytes cross a syscall lives here exactly once.
// Kernel handler and libmorpheus both `use` these — there is only one definition,
// so the two sides cannot drift. (Previously each was declared twice; comments
// don't compile-check.)

/// Per-CPU bound for `SysInfo::per_core_idle_tsc`.
pub const SYSINFO_MAX_CPUS: usize = 16;

/// `sysinfo(&mut buf)` — SYS_SYSINFO.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SysInfo {
    pub version: u16,
    pub struct_size: u16,
    pub num_procs: u32,
    pub cpu_count: u32,
    pub _pad0: u32,
    pub total_mem: u64,
    pub free_mem: u64,
    pub uptime_ticks: u64,
    pub tsc_freq: u64,
    pub heap_total: u64,
    pub heap_used: u64,
    pub heap_free: u64,
    pub sched_ticks: u64,
    pub idle_tsc: u64,
    pub per_core_idle_tsc: [u64; SYSINFO_MAX_CPUS],
    pub reserved: [u64; 4],
}

impl SysInfo {
    pub const fn zeroed() -> Self {
        Self {
            version: 0,
            struct_size: 0,
            num_procs: 0,
            cpu_count: 0,
            _pad0: 0,
            total_mem: 0,
            free_mem: 0,
            uptime_ticks: 0,
            tsc_freq: 0,
            heap_total: 0,
            heap_used: 0,
            heap_free: 0,
            sched_ticks: 0,
            idle_tsc: 0,
            per_core_idle_tsc: [0; SYSINFO_MAX_CPUS],
            reserved: [0; 4],
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
    pub version: u16,
    pub struct_size: u16,
    pub pid: u32,
    pub ppid: u32,
    pub state: u32,
    pub priority: u32,
    pub _pad0: u32,
    pub cpu_ticks: u64,
    pub cpu_tsc: u64,
    pub pages_alloc: u64,
    pub name: [u8; 32],
    pub reserved: [u8; 16],
}

impl PsEntry {
    pub const fn zeroed() -> Self {
        Self {
            version: 0,
            struct_size: 0,
            pid: 0,
            ppid: 0,
            state: 0,
            priority: 0,
            _pad0: 0,
            cpu_ticks: 0,
            cpu_tsc: 0,
            pages_alloc: 0,
            name: [0u8; 32],
            reserved: [0u8; 16],
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

/// `nic_info(&mut buf)` — SYS_NIC_INFO. `mac` is 6 bytes padded to 8. All fields
/// are <=u16, so `align(8)` is forced for a stable 8-aligned stride.
#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct NicInfo {
    pub version: u16,
    pub struct_size: u16,
    pub link_up: u16,
    pub present: u16,
    pub mac: [u8; 8],
    pub reserved: [u8; 8],
}

/// `fb_info(&mut buf)` — SYS_FB_INFO. `format`: 0=RGBX 1=BGRX.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FbInfo {
    pub version: u16,
    pub struct_size: u16,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub _pad0: u32,
    pub base: u64,
    pub size: u64,
    pub reserved: [u64; 2],
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
    pub version: u16,
    pub struct_size: u16,
    pub device_kind: u32,
    pub fs_type: u32,
    pub flags: u32,
    pub block_size: u32,
    pub _pad0: u32,
    pub volume_id: u64,
    pub device_id: u64,
    pub lba_start: u64,
    pub lba_count: u64,
    pub partition_guid: [u8; 16],
    pub label: [u8; 64],
    pub reserved: [u8; 16],
}

impl VolumeInfo {
    pub const fn zeroed() -> Self {
        Self {
            version: 0,
            struct_size: 0,
            device_kind: 0,
            fs_type: 0,
            flags: 0,
            block_size: 0,
            _pad0: 0,
            volume_id: 0,
            device_id: 0,
            lba_start: 0,
            lba_count: 0,
            partition_guid: [0u8; 16],
            label: [0u8; 64],
            reserved: [0u8; 16],
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
    pub version: u16,
    pub struct_size: u16,
    pub fs_type: u32,
    pub flags: u32,
    pub mount_point_len: u16,
    pub _pad0: [u8; 2],
    pub mount_id: u64,
    pub volume_id: u64,
    pub mount_point: [u8; 256],
    pub reserved: [u8; 16],
}

impl MountInfo {
    pub const fn zeroed() -> Self {
        Self {
            version: 0,
            struct_size: 0,
            fs_type: 0,
            flags: 0,
            mount_point_len: 0,
            _pad0: [0u8; 2],
            mount_id: 0,
            volume_id: 0,
            mount_point: [0u8; 256],
            reserved: [0u8; 16],
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

// Fixed POSIX/option payloads carry no version head — the layout is the one
// correct Linux x86-64 form (documented ABI exemption).

/// `struct timespec` (seconds + nanoseconds). Also the `FUTEX_WAIT` timeout payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// `struct timeval` (seconds + MICROSECONDS) — `SO_RCVTIMEO`/`SO_SNDTIMEO` optval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct KTimeval {
    pub tv_sec: i64,
    pub tv_usec: i64,
}

/// `struct linger` — `SO_LINGER` optval.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Linger {
    pub l_onoff: i32,
    pub l_linger: i32,
}

/// `struct ip_mreq` — `IP_ADD/DROP_MEMBERSHIP` optval. Addresses network byte order.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct IpMreq {
    pub imr_multiaddr: u32,
    pub imr_interface: u32,
}

/// `struct ipv6_mreq` — `IPV6_ADD/DROP_MEMBERSHIP` optval.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Ipv6Mreq {
    pub ipv6mr_multiaddr: [u8; 16],
    pub ipv6mr_interface: u32,
}

/// `waitid(.., *mut WaitStatus, ..)` result. The pid is on the syscall value
/// channel; `wstatus` carries the encoded status word. Decode with the `W*`
/// helpers (the one correct Linux bit form).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct WaitStatus {
    pub version: u16,
    pub struct_size: u16,
    pub _pad0: u32,
    pub pid: i32,
    pub wstatus: i32,
    /// Reserved for utime/stime (rusage-lite).
    pub reserved: [u64; 2],
}

impl WaitStatus {
    #[inline]
    pub const fn exited(s: i32) -> bool {
        (s & 0x7f) == 0
    }
    #[inline]
    pub const fn exit_status(s: i32) -> i32 {
        (s >> 8) & 0xff
    }
    #[inline]
    pub const fn signaled(s: i32) -> bool {
        ((s & 0x7f) + 1) >> 1 > 0
    }
    #[inline]
    pub const fn term_sig(s: i32) -> i32 {
        s & 0x7f
    }
    #[inline]
    pub const fn core_dumped(s: i32) -> bool {
        (s & 0x80) != 0
    }
    #[inline]
    pub const fn stopped(s: i32) -> bool {
        (s & 0xff) == 0x7f
    }
    #[inline]
    pub const fn stop_sig(s: i32) -> i32 {
        (s >> 8) & 0xff
    }
    #[inline]
    pub const fn continued(s: i32) -> bool {
        s == 0xffff
    }
}

/// `posix_spawn`-style argument block (`SYS_SPAWN`). `argv_ptr`/`envp_ptr` point
/// to arrays of `{ptr:u64, len:u64}`; `cwd_ptr==0` inherits. `fa_stride` declares
/// the per-`SpawnFileAction` stride the kernel indexes by, so the action record
/// can grow without breaking shipped fixed-stride arrays.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SpawnArgs {
    pub version: u16,
    pub struct_size: u16,
    pub flags: u32,
    pub path_ptr: u64,
    pub path_len: u64,
    pub argv_ptr: u64,
    pub argc: u64,
    pub envp_ptr: u64,
    pub envc: u64,
    pub cwd_ptr: u64,
    pub cwd_len: u64,
    pub file_actions_ptr: u64,
    pub file_actions_count: u64,
    pub fa_stride: u32,
    pub _pad0: u32,
    pub reserved: [u64; 4],
}

/// One `SpawnArgs.file_actions[]` record. `op` is `SPAWN_FA_*`, replayed in order.
/// `{version, struct_size}` head + `reserved` tail let the record grow within the
/// declared `SpawnArgs.fa_stride`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SpawnFileAction {
    pub version: u16,
    pub struct_size: u16,
    pub op: u32,
    pub fd: i32,
    pub newfd: i32,
    pub oflags: u32,
    pub mode: u32,
    pub path_ptr: u64,
    pub path_len: u64,
    pub reserved: [u64; 2],
}

/// Tagged socket-address envelope: `sa_family` then opaque bytes overlaying
/// `SockAddrIn`/`SockAddrIn6` (and future families) by family. `align(8)` is
/// forced for a stable stride. AF_INET uses the first 16 bytes, AF_INET6 28.
#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct SockAddrStorage {
    pub sa_family: u16,
    pub _opaque: [u8; 126],
}

impl SockAddrStorage {
    pub const fn zeroed() -> Self {
        Self {
            sa_family: 0,
            _opaque: [0u8; 126],
        }
    }
}

impl Default for SockAddrStorage {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// `struct sockaddr_in`. Port/addr cross the ABI in network byte order.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SockAddrIn {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: u32,
    pub sin_zero: [u8; 8],
}

/// `struct sockaddr_in6`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SockAddrIn6 {
    pub sin6_family: u16,
    pub sin6_port: u16,
    pub sin6_flowinfo: u32,
    pub sin6_addr: [u8; 16],
    pub sin6_scope_id: u32,
}

/// `struct pollfd` (exact Linux layout) — `SYS_POLL`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

/// `epoll_event` — deliberately naturally-aligned 16B (not Linux's packed 12B) to
/// avoid `repr(packed)` UB. `data` is the opaque echoed token; `_pad` reserved.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct EpollEvent {
    pub events: u32,
    pub _pad: u32,
    pub data: u64,
}

/// `struct sigaction` mirror (`SYS_SIGACTION`). Reserved layout; signal delivery
/// is deferred. `sa_handler`: `SIG_DFL=0`/`SIG_IGN=1`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct KSigAction {
    pub sa_handler: u64,
    pub sa_flags: u64,
    pub sa_restorer: u64,
    pub sa_mask: u64,
    pub reserved: [u64; 4],
}

/// `siginfo_t`-compatible 128B envelope with an opaque union tail. Reserved.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct KSigInfo {
    pub si_signo: i32,
    pub si_errno: i32,
    pub si_code: i32,
    pub _pad0: u32,
    pub fields: [u64; 14],
}

// Layout lock: size/align/key-offset of every boundary struct is pinned so any
// field reorder, resize, or padding drift is a COMPILE error — the struct mirror
// of the syscall-number lock in `syscall_abi.rs`.
const _: () = {
    use core::mem::{align_of, offset_of, size_of};

    assert!(size_of::<FileStat>() == 120 && align_of::<FileStat>() == 8);
    assert!(offset_of!(FileStat, mode) == 4);
    assert!(offset_of!(FileStat, key) == 8);
    assert!(offset_of!(FileStat, accessed_ns) == 40);

    assert!(size_of::<DirEntry>() == 304 && align_of::<DirEntry>() == 8);
    assert!(offset_of!(DirEntry, size) == 16);
    assert!(offset_of!(DirEntry, name) == 32);

    assert!(size_of::<SysInfo>() == 248 && align_of::<SysInfo>() == 8);
    assert!(offset_of!(SysInfo, total_mem) == 16);
    assert!(offset_of!(SysInfo, per_core_idle_tsc) == 88);

    assert!(size_of::<PsEntry>() == 96 && align_of::<PsEntry>() == 8);
    assert!(offset_of!(PsEntry, cpu_ticks) == 24);
    assert!(offset_of!(PsEntry, name) == 48);

    assert!(size_of::<VolumeInfo>() == 152 && align_of::<VolumeInfo>() == 8);
    assert!(offset_of!(VolumeInfo, volume_id) == 24);
    assert!(offset_of!(VolumeInfo, label) == 72);

    assert!(size_of::<MountInfo>() == 304 && align_of::<MountInfo>() == 8);
    assert!(offset_of!(MountInfo, mount_id) == 16);
    assert!(offset_of!(MountInfo, mount_point) == 32);

    assert!(size_of::<NicInfo>() == 24 && align_of::<NicInfo>() == 8);
    assert!(offset_of!(NicInfo, mac) == 8);

    assert!(size_of::<FbInfo>() == 56 && align_of::<FbInfo>() == 8);
    assert!(offset_of!(FbInfo, base) == 24);

    assert!(size_of::<Timespec>() == 16 && align_of::<Timespec>() == 8);
    assert!(offset_of!(Timespec, tv_nsec) == 8);

    assert!(size_of::<KTimeval>() == 16 && align_of::<KTimeval>() == 8);
    assert!(offset_of!(KTimeval, tv_usec) == 8);

    assert!(size_of::<Linger>() == 8 && align_of::<Linger>() == 4);
    assert!(offset_of!(Linger, l_linger) == 4);

    assert!(size_of::<IpMreq>() == 8 && align_of::<IpMreq>() == 4);
    assert!(offset_of!(IpMreq, imr_interface) == 4);

    assert!(size_of::<Ipv6Mreq>() == 20 && align_of::<Ipv6Mreq>() == 4);
    assert!(offset_of!(Ipv6Mreq, ipv6mr_interface) == 16);

    assert!(size_of::<WaitStatus>() == 32 && align_of::<WaitStatus>() == 8);
    assert!(offset_of!(WaitStatus, wstatus) == 12);

    assert!(size_of::<SpawnArgs>() == 128 && align_of::<SpawnArgs>() == 8);
    assert!(offset_of!(SpawnArgs, file_actions_ptr) == 72);
    assert!(offset_of!(SpawnArgs, fa_stride) == 88);

    assert!(size_of::<SpawnFileAction>() == 56 && align_of::<SpawnFileAction>() == 8);
    assert!(offset_of!(SpawnFileAction, path_ptr) == 24);

    assert!(size_of::<SockAddrStorage>() == 128 && align_of::<SockAddrStorage>() == 8);
    assert!(offset_of!(SockAddrStorage, sa_family) == 0);

    assert!(size_of::<SockAddrIn>() == 16);
    assert!(offset_of!(SockAddrIn, sin_port) == 2);
    assert!(offset_of!(SockAddrIn, sin_addr) == 4);

    assert!(size_of::<SockAddrIn6>() == 28);
    assert!(offset_of!(SockAddrIn6, sin6_port) == 2);
    assert!(offset_of!(SockAddrIn6, sin6_addr) == 8);
    assert!(offset_of!(SockAddrIn6, sin6_scope_id) == 24);

    assert!(size_of::<PollFd>() == 8 && align_of::<PollFd>() == 4);
    assert!(offset_of!(PollFd, events) == 4);
    assert!(offset_of!(PollFd, revents) == 6);

    assert!(size_of::<EpollEvent>() == 16 && align_of::<EpollEvent>() == 8);
    assert!(offset_of!(EpollEvent, data) == 8);

    assert!(size_of::<KSigAction>() == 64 && align_of::<KSigAction>() == 8);

    assert!(size_of::<KSigInfo>() == 128 && align_of::<KSigInfo>() == 8);
    assert!(offset_of!(KSigInfo, fields) == 16);
};
