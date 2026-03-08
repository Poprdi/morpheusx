/// message types for inter-island communication within compd.
/// all state crosses island boundaries inside these enums. nothing else.

pub const MAX_WINDOWS: usize = 16;

/// vsync island → renderer island.
#[derive(Clone, Copy)]
pub enum VsyncMsg {
    Tick { now_ns: u64 },
}

/// a single window's compositing parameters. surface is a raw pointer valid
/// only during the compose() call on single-core, no preemption between
/// surface_mgr and renderer.
#[derive(Clone, Copy)]
pub struct CompositeEntry {
    pub pid:        u32,
    pub surface:    *const u32,
    pub x:          i32,
    pub y:          i32,
    pub w:          u32,
    pub h:          u32,
    pub src_w:      u32,
    pub src_h:      u32,
    pub src_stride: u32, // stride in PIXELS not bytes. the other stride is bytes. welcome to abi hell.
    pub z_layer:    u8,  // 0=bg 1=bottom 2=top 3=overlay
    pub dirty:      bool,
}

// SAFETY: single-core scheduler, no preemption between islands.
unsafe impl Send for CompositeEntry {}
unsafe impl Sync for CompositeEntry {}

/// surface_mgr → renderer.
pub enum SurfaceMsg {
    CompositeList {
        entries: [Option<CompositeEntry>; MAX_WINDOWS],
        count:   u8,
    },
}

/// input island → focus island / surface_mgr.
#[derive(Clone, Copy)]
pub enum InputMsg {
    FocusCycleRequest,
    WindowClosed { idx: u8, pid: u32 },
}

/// focus island → renderer.
#[derive(Clone, Copy)]
pub enum FocusMsg {
    FocusChanged { old: Option<u8>, new: Option<u8> },
}
