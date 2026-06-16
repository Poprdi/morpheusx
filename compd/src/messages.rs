//! Inter-island messages. All cross-island state travels in these enums.

#[allow(dead_code)] // referenced only by dead message types; retained as protocol definition.
pub const MAX_WINDOWS: usize = 16;

#[allow(dead_code)] // protocol message type, not yet wired into the live pipeline.
#[derive(Clone, Copy)]
pub enum VsyncMsg {
    Tick { now_ns: u64 },
}

/// `surface` is valid only for the compose() call (single-core, no preemption
/// between surface_mgr and renderer).
#[allow(dead_code)] // protocol message type, not yet wired into the live pipeline.
#[derive(Clone, Copy)]
pub struct CompositeEntry {
    pub pid: u32,
    pub surface: *const u32,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub src_stride: u32, // pixels (fb stride is bytes).
    pub z_layer: u8,     // 0=bg 1=bottom 2=top 3=overlay
    pub dirty: bool,
}

// SAFETY: single-core, no preemption between islands.
unsafe impl Send for CompositeEntry {}
unsafe impl Sync for CompositeEntry {}

#[allow(dead_code)] // protocol message type, not yet wired into the live pipeline.
pub enum SurfaceMsg {
    CompositeList {
        entries: [Option<CompositeEntry>; MAX_WINDOWS],
        count: u8,
    },
}

#[derive(Clone, Copy)]
pub enum InputMsg {
    /// Move focus to the next z1 window — `reverse` walks the ring backward (Shift+Alt+Tab) vs the
    /// forward Alt+Tab / Ctrl+] default.
    FocusCycleRequest { reverse: bool },
    #[allow(dead_code)] // protocol variant, not yet emitted by the live pipeline.
    WindowClosed { idx: u8, pid: u32 },
}

#[allow(dead_code)] // protocol message type, not yet wired into the live pipeline.
#[derive(Clone, Copy)]
pub enum FocusMsg {
    FocusChanged { old: Option<u8>, new: Option<u8> },
}

/// Absolute desktop position plus edge transitions, sampled from input polling.
#[derive(Clone, Copy)]
pub struct MouseSpatialMsg {
    pub mx: i32,
    pub my: i32,
    pub buttons: u8,
    pub left_pressed: bool,
    pub left_released: bool,
    pub right_pressed: bool,
    pub in_panel: bool,
}

/// Z-layer routing decision derived from a spatial sample.
#[derive(Clone, Copy)]
pub enum MouseZRouteMsg {
    Desktop { buttons: u8 },
    Child { idx: u8, buttons: u8 },
    None,
}
