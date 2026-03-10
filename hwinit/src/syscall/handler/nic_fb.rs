
// NIC REGISTRATION — function-pointer bridge for network drivers
//
// hwinit does not depend on morpheus-network, so the NIC driver is
// registered by the bootloader via function pointers.

/// NIC operations function-pointer table.  The bootloader fills this in
/// after initialising the network driver, before entering the event loop.
#[repr(C)]
pub struct NicOps {
    /// Transmit a raw Ethernet frame.  Returns 0 on success, -1 on error.
    pub tx: Option<unsafe fn(frame: *const u8, len: usize) -> i64>,
    /// Receive a raw Ethernet frame into `buf`.  Returns bytes received, 0 if none.
    pub rx: Option<unsafe fn(buf: *mut u8, buf_len: usize) -> i64>,
    /// Get link status.  Returns 1 if link is up, 0 if down.
    pub link_up: Option<unsafe fn() -> i64>,
    /// Get 6-byte MAC address.  Writes to `out`.  Returns 0 on success.
    pub mac: Option<unsafe fn(out: *mut u8) -> i64>,
    /// Refill RX descriptor ring.
    pub refill: Option<unsafe fn() -> i64>,
    /// Hardware control — set promisc, MAC, VLAN, offloads, etc.
    /// `cmd` selects the operation, `arg` is command-specific.
    /// Returns 0 on success, negative on error.
    pub ctrl: Option<unsafe fn(cmd: u32, arg: u64) -> i64>,
}

// nic_ctrl command constants
/// Enable/disable promiscuous mode.  arg: 1=on, 0=off.
pub const NIC_CTRL_PROMISC: u32 = 1;
/// Set MAC address (arg = pointer to 6 bytes).
pub const NIC_CTRL_MAC_SET: u32 = 2;
/// Get hardware statistics (arg = pointer to NicHwStats).
pub const NIC_CTRL_STATS: u32 = 3;
/// Reset hardware statistics counters.
pub const NIC_CTRL_STATS_RESET: u32 = 4;
/// Set MTU.  arg = new MTU value.
pub const NIC_CTRL_MTU: u32 = 5;
/// Enable/disable multicast (arg: 1=accept all, 0=filter).
pub const NIC_CTRL_MULTICAST: u32 = 6;
/// Set VLAN tag (arg: 0=disable, 1..4095=VLAN ID).
pub const NIC_CTRL_VLAN: u32 = 7;
/// Enable/disable TX checksum offload (arg: 1=on, 0=off).
pub const NIC_CTRL_TX_CSUM: u32 = 8;
/// Enable/disable RX checksum offload (arg: 1=on, 0=off).
pub const NIC_CTRL_RX_CSUM: u32 = 9;
/// Enable/disable TCP segmentation offload (arg: 1=on, 0=off).
pub const NIC_CTRL_TSO: u32 = 10;
/// Set RX ring buffer size (arg: number of descriptors).
pub const NIC_CTRL_RX_RING_SIZE: u32 = 11;
/// Set TX ring buffer size (arg: number of descriptors).
pub const NIC_CTRL_TX_RING_SIZE: u32 = 12;
/// Set interrupt coalescing (arg: microseconds between interrupts).
pub const NIC_CTRL_IRQ_COALESCE: u32 = 13;
/// Get NIC capabilities bitmask (arg = pointer to u64 out).
pub const NIC_CTRL_CAPS: u32 = 14;

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

static mut NIC_OPS: NicOps = NicOps {
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

/// Read the registered framebuffer info.  Returns None before register_framebuffer.
#[inline]
pub unsafe fn fb_registered() -> Option<FbInfo> {
    if FB_REGISTERED_READY.load(FbOrd::Acquire) {
        FB_REGISTERED_STORAGE
    } else {
        None
    }
}

/// Register framebuffer info.  Called by bootloader before entering desktop.
pub unsafe fn register_framebuffer(info: FbInfo) {
    FB_REGISTERED_STORAGE = Some(info);
    FB_REGISTERED_READY.store(true, FbOrd::Release);
}

// DOUBLE BUFFER — kernel-owned back buffer + shadow for delta presentation

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Diff back vs shadow, write changed pixel spans to VRAM, update shadow.
    /// All three buffers use the same stride (pixels per row).
    fn asm_fb_present_delta(
        back: u64,
        shadow: u64,
        vram: u64,
        width: u64,
        height: u64,
        stride: u64,
    );
}

/// Physical address of the kernel-allocated back buffer (zero = unallocated).
static FB_BACK_PHYS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Physical address of the kernel-allocated shadow buffer.
static FB_SHADOW_PHYS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Number of physical pages allocated for each of the two buffers.
static FB_BACK_PAGES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Dirty flag: set by `SYS_FB_PRESENT`, `SYS_FB_BLIT`, and `fb_mark_dirty()`.
/// Cleared by `fb_present_tick()` after a successful delta present.
/// When false, `fb_present_tick()` skips the full-screen scan entirely.
static FB_DIRTY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Mark the framebuffer back buffer as dirty (needs present).
/// Called by syscalls that write to the back buffer.
#[inline]
pub fn fb_mark_dirty() {
    FB_DIRTY.store(true, core::sync::atomic::Ordering::Relaxed);
}

