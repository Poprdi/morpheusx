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

pub use morpheus_foundation::net::{
    NIC_CTRL_CAPS, NIC_CTRL_IRQ_COALESCE, NIC_CTRL_MAC_SET, NIC_CTRL_MTU, NIC_CTRL_MULTICAST,
    NIC_CTRL_PROMISC, NIC_CTRL_RX_CSUM, NIC_CTRL_RX_RING_SIZE, NIC_CTRL_STATS,
    NIC_CTRL_STATS_RESET, NIC_CTRL_TSO, NIC_CTRL_TX_CSUM, NIC_CTRL_TX_RING_SIZE, NIC_CTRL_VLAN,
};

/// Hardware NIC statistics (returned by NIC_CTRL_STATS).
pub use morpheus_foundation::types::{FbInfo, NicHwStats, NicInfo};

/// NIC capability bits (returned by NIC_CTRL_CAPS).
pub use morpheus_foundation::net::{
    NIC_CAP_IRQ_COALESCE, NIC_CAP_MAC_SET, NIC_CAP_MULTICAST, NIC_CAP_PROMISC, NIC_CAP_RX_CSUM,
    NIC_CAP_TSO, NIC_CAP_TX_CSUM, NIC_CAP_VLAN,
};

pub(crate) static mut NIC_OPS: NicOps = NicOps {
    tx: None,
    rx: None,
    link_up: None,
    mac: None,
    refill: None,
    ctrl: None,
};

/// Called by the bootloader after driver init.
pub unsafe fn register_nic(ops: NicOps) {
    NIC_OPS = ops;
}

// Write-once, read-many across cores; atomic ready-flag gates the static mut.
use core::sync::atomic::{AtomicBool, Ordering as FbOrd};
static mut FB_REGISTERED_STORAGE: Option<FbInfo> = None;
static FB_REGISTERED_READY: AtomicBool = AtomicBool::new(false);

/// Returns None before `register_framebuffer` is called.
#[inline]
pub unsafe fn fb_registered() -> Option<FbInfo> {
    if FB_REGISTERED_READY.load(FbOrd::Acquire) {
        FB_REGISTERED_STORAGE
    } else {
        None
    }
}

/// Called by bootloader after GOP framebuffer is set up.
pub unsafe fn register_framebuffer(info: FbInfo) {
    FB_REGISTERED_STORAGE = Some(info);
    FB_REGISTERED_READY.store(true, FbOrd::Release);
}

/// Zero = unallocated.
pub(crate) static FB_BACK_PHYS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub(crate) static FB_SHADOW_PHYS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Page count for each of the two buffers.
pub(crate) static FB_BACK_PAGES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Dirty flag: set by `SYS_FB_PRESENT`, `SYS_FB_BLIT`, and `fb_mark_dirty()`.
/// Cleared by `fb_present_tick()` after a successful delta present.
pub(crate) static FB_DIRTY: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[inline]
pub fn fb_mark_dirty() {
    FB_DIRTY.store(true, core::sync::atomic::Ordering::Relaxed);
}
