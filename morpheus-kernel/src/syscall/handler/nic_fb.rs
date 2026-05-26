// NIC function-pointer bridge + framebuffer registration. hwinit doesn't
// depend on morpheus-network; bootloader registers ops after init.

#[repr(C)]
pub struct NicOps {
    pub tx: Option<unsafe fn(frame: *const u8, len: usize) -> i64>,
    pub rx: Option<unsafe fn(buf: *mut u8, buf_len: usize) -> i64>,
    pub link_up: Option<unsafe fn() -> i64>,
    /// Writes 6 bytes to `out`.
    pub mac: Option<unsafe fn(out: *mut u8) -> i64>,
    pub refill: Option<unsafe fn() -> i64>,
    /// cmd → NIC_CTRL_*; arg is per-cmd.
    pub ctrl: Option<unsafe fn(cmd: u32, arg: u64) -> i64>,
}

pub const NIC_CTRL_PROMISC: u32 = 1; // arg: 1=on/0=off
pub const NIC_CTRL_MAC_SET: u32 = 2; // arg: *const [u8; 6]
pub const NIC_CTRL_STATS: u32 = 3; // arg: *mut NicHwStats
pub const NIC_CTRL_STATS_RESET: u32 = 4;
pub const NIC_CTRL_MTU: u32 = 5;
pub const NIC_CTRL_MULTICAST: u32 = 6; // arg: 1=accept-all/0=filter
pub const NIC_CTRL_VLAN: u32 = 7; // arg: 0=off, 1..4095=VID
pub const NIC_CTRL_TX_CSUM: u32 = 8;
pub const NIC_CTRL_RX_CSUM: u32 = 9;
pub const NIC_CTRL_TSO: u32 = 10;
pub const NIC_CTRL_RX_RING_SIZE: u32 = 11;
pub const NIC_CTRL_TX_RING_SIZE: u32 = 12;
pub const NIC_CTRL_IRQ_COALESCE: u32 = 13; // arg: µs
pub const NIC_CTRL_CAPS: u32 = 14; // arg: *mut u64

/// Hardware NIC statistics (returned by NIC_CTRL_STATS).
#[repr(C)]
#[derive(Clone, Copy)]
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

/// NIC capability bits (returned by NIC_CTRL_CAPS).
pub const NIC_CAP_PROMISC: u64 = 1 << 0;
pub const NIC_CAP_MAC_SET: u64 = 1 << 1;
pub const NIC_CAP_MULTICAST: u64 = 1 << 2;
pub const NIC_CAP_VLAN: u64 = 1 << 3;
pub const NIC_CAP_TX_CSUM: u64 = 1 << 4;
pub const NIC_CAP_RX_CSUM: u64 = 1 << 5;
pub const NIC_CAP_TSO: u64 = 1 << 6;
pub const NIC_CAP_IRQ_COALESCE: u64 = 1 << 7;

pub(crate) static mut NIC_OPS: NicOps = NicOps {
    tx: None,
    rx: None,
    link_up: None,
    mac: None,
    refill: None,
    ctrl: None,
};

/// Register NIC function pointers.  Called by the bootloader after driver init.
pub unsafe fn register_nic(ops: NicOps) {
    NIC_OPS = ops;
}

/// NIC info returned by SYS_NIC_INFO.
#[repr(C)]
pub struct NicInfo {
    /// 6-byte MAC address, padded to 8.
    pub mac: [u8; 8],
    /// 1 if link up, 0 if down.
    pub link_up: u32,
    /// 1 if NIC is registered, 0 if not.
    pub present: u32,
}

// FRAMEBUFFER REGISTRATION — pass FB info from bootloader to hwinit

/// Framebuffer information registered by the bootloader.
/// Matches display/src/types.rs FramebufferInfo layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FbInfo {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 0 = RGBX, 1 = BGRX
    pub format: u32,
}

// write-once, read-many from any core. atomic flag + raw pointer avoids static mut UB.
use core::sync::atomic::{AtomicBool, Ordering as FbOrd};
static mut FB_REGISTERED_STORAGE: Option<FbInfo> = None;
static FB_REGISTERED_READY: AtomicBool = AtomicBool::new(false);

/// Read the registered framebuffer info. Returns None before register_framebuffer.
#[inline]
pub unsafe fn fb_registered() -> Option<FbInfo> {
    if FB_REGISTERED_READY.load(FbOrd::Acquire) {
        FB_REGISTERED_STORAGE
    } else {
        None
    }
}

/// Register framebuffer info. Called by bootloader before entering desktop.
pub unsafe fn register_framebuffer(info: FbInfo) {
    FB_REGISTERED_STORAGE = Some(info);
    FB_REGISTERED_READY.store(true, FbOrd::Release);
}

// DOUBLE BUFFER — kernel-owned back buffer + shadow for delta presentation

/// Physical address of the kernel-allocated back buffer (zero = unallocated).
pub(crate) static FB_BACK_PHYS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Physical address of the kernel-allocated shadow buffer.
pub(crate) static FB_SHADOW_PHYS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Number of physical pages allocated for each of the two buffers.
pub(crate) static FB_BACK_PAGES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Dirty flag: set by `SYS_FB_PRESENT`, `SYS_FB_BLIT`, and `fb_mark_dirty()`.
/// Cleared by `fb_present_tick()` after a successful delta present.
pub(crate) static FB_DIRTY: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Mark the framebuffer back buffer as dirty (needs present).
#[inline]
pub fn fb_mark_dirty() {
    FB_DIRTY.store(true, core::sync::atomic::Ordering::Relaxed);
}
