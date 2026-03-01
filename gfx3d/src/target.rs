use alloc::vec::Vec;

/// Render target — the abstraction that decouples the 3D engine from any
/// specific framebuffer or windowing system.
///
/// The consumer provides a `RenderTarget` implementation. The pipeline writes
/// finished pixels here. That's it — zero knowledge of OS, framebuffer format,
/// memory-mapped I/O, or anything platform-specific.
///
/// Two key buffers:
/// - `color`: packed RGBA pixels (internal format, engine-defined)
/// - `depth`: Z-buffer values, 16.16 fixed-point or f32
///
/// We use our own `TargetPixelFormat` to describe how the consumer's framebuffer
/// stores pixels, so the pipeline's final color write does the conversion.
pub trait RenderTarget {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn color_buffer_mut(&mut self) -> &mut [u32];
    fn depth_buffer_mut(&mut self) -> &mut [u32]; // 16.16 fixed-point depth
    fn buffers_mut(&mut self) -> (&mut [u32], &mut [u32]);
    fn pixel_format(&self) -> TargetPixelFormat;

    /// Stride in pixels (may differ from width for hardware framebuffers).
    fn stride(&self) -> u32 {
        self.width()
    }
}

/// The consumer's framebuffer pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPixelFormat {
    /// Blue-Green-Red-X (most UEFI framebuffers)
    Bgrx,
    /// Red-Green-Blue-X
    Rgbx,
    /// Our internal format (R=MSB): 0xRRGGBBAA
    InternalRgba,
}

/// Software render target backed by Vec<u32>.
///
/// This is the zero-overhead path: color and depth are contiguous allocations,
/// no syscalls, no MMIO — just raw memory. The consumer blits this to the
/// real framebuffer as a final step (or writes it to a PNG for testing).
pub struct SoftwareTarget {
    pub width: u32,
    pub height: u32,
    pub color: Vec<u32>,
    pub depth: Vec<u32>,
    pub format: TargetPixelFormat,
}

impl SoftwareTarget {
    pub fn new(width: u32, height: u32, format: TargetPixelFormat) -> Self {
        let size = (width * height) as usize;
        Self {
            width,
            height,
            color: alloc::vec![0u32; size],
            depth: alloc::vec![0xFFFF_FFFF; size], // max depth = furthest away (reversed-Z)
            format,
        }
    }

    pub fn clear(&mut self, clear_color: u32) {
        self.color.fill(clear_color);
        unsafe { core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len()); }
    }

    pub fn clear_color(&mut self, clear_color: u32) {
        self.color.fill(clear_color);
    }

    pub fn clear_depth(&mut self) {
        unsafe { core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len()); }
    }
}

impl RenderTarget for SoftwareTarget {
    #[inline(always)]
    fn width(&self) -> u32 {
        self.width
    }
    #[inline(always)]
    fn height(&self) -> u32 {
        self.height
    }
    #[inline(always)]
    fn color_buffer_mut(&mut self) -> &mut [u32] {
        &mut self.color
    }
    #[inline(always)]
    fn depth_buffer_mut(&mut self) -> &mut [u32] {
        &mut self.depth
    }
    #[inline(always)]
    fn buffers_mut(&mut self) -> (&mut [u32], &mut [u32]) {
        (&mut self.color, &mut self.depth)
    }
    #[inline(always)]
    fn pixel_format(&self) -> TargetPixelFormat {
        self.format
    }
    #[inline(always)]
    fn stride(&self) -> u32 {
        self.width
    }
}

/// Render target that writes directly into an externally-owned pixel buffer
/// (e.g. a memory-mapped framebuffer back buffer).
///
/// Eliminates the intermediate copy that `SoftwareTarget` requires.
/// The depth buffer is still heap-allocated since there's no hardware
/// equivalent to share.
pub struct DirectTarget {
    ptr: *mut u32,
    width: u32,
    height: u32,
    stride_px: u32,
    len: usize,
    depth: Vec<u32>,
    format: TargetPixelFormat,
}

impl DirectTarget {
    /// Wrap an externally-owned pixel buffer as a render target.
    ///
    /// `ptr` must point to at least `stride_px * height` u32 pixels.
    /// `stride_px` is measured in pixels (not bytes).
    pub unsafe fn new(
        ptr: *mut u32,
        width: u32,
        height: u32,
        stride_px: u32,
        format: TargetPixelFormat,
    ) -> Self {
        let len = (stride_px as usize) * (height as usize);
        Self {
            ptr,
            width,
            height,
            stride_px,
            len,
            depth: alloc::vec![0xFFFF_FFFFu32; len],
            format,
        }
    }

    pub fn clear(&mut self, clear_color: u32) {
        let buf = unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) };
        buf.fill(clear_color);
        unsafe { core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len()); }
    }

    pub fn clear_color(&mut self, clear_color: u32) {
        let buf = unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) };
        buf.fill(clear_color);
    }

    pub fn clear_depth(&mut self) {
        unsafe { core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len()); }
    }
}

impl RenderTarget for DirectTarget {
    #[inline(always)]
    fn width(&self) -> u32 {
        self.width
    }
    #[inline(always)]
    fn height(&self) -> u32 {
        self.height
    }
    #[inline(always)]
    fn color_buffer_mut(&mut self) -> &mut [u32] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
    #[inline(always)]
    fn depth_buffer_mut(&mut self) -> &mut [u32] {
        &mut self.depth
    }
    #[inline(always)]
    fn buffers_mut(&mut self) -> (&mut [u32], &mut [u32]) {
        let color = unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) };
        (color, &mut self.depth)
    }
    #[inline(always)]
    fn pixel_format(&self) -> TargetPixelFormat {
        self.format
    }
    #[inline(always)]
    fn stride(&self) -> u32 {
        self.stride_px
    }
}

/// Convert internal RGBA (0xRRGGBBAA) to target pixel format.
#[inline(always)]
pub fn convert_pixel(rgba: u32, format: TargetPixelFormat) -> u32 {
    match format {
        TargetPixelFormat::InternalRgba => rgba,
        TargetPixelFormat::Rgbx => {
            // RGBA → RGBX: just clear alpha byte and shift
            // Internal: R[31:24] G[23:16] B[15:8] A[7:0]
            // RGBX:     R[7:0] G[15:8] B[23:16] X[31:24]
            let r = (rgba >> 24) & 0xFF;
            let g = (rgba >> 16) & 0xFF;
            let b = (rgba >> 8) & 0xFF;
            r | (g << 8) | (b << 16)
        }
        TargetPixelFormat::Bgrx => {
            // Internal: R[31:24] G[23:16] B[15:8] A[7:0]
            // BGRX:     B[7:0] G[15:8] R[23:16] X[31:24]
            let r = (rgba >> 24) & 0xFF;
            let g = (rgba >> 16) & 0xFF;
            let b = (rgba >> 8) & 0xFF;
            b | (g << 8) | (r << 16)
        }
    }
}
