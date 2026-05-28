use alloc::vec::Vec;

/// Output sink for the pipeline. Color + 16.16 depth buffer.
pub trait RenderTarget {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn color_buffer_mut(&mut self) -> &mut [u32];
    fn depth_buffer_mut(&mut self) -> &mut [u32];
    fn buffers_mut(&mut self) -> (&mut [u32], &mut [u32]);
    fn pixel_format(&self) -> TargetPixelFormat;

    /// Pixel stride (may exceed width on hardware framebuffers).
    fn stride(&self) -> u32 {
        self.width()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPixelFormat {
    /// B[7:0] G[15:8] R[23:16] X[31:24] — most UEFI framebuffers.
    Bgrx,
    /// R[7:0] G[15:8] B[23:16] X[31:24].
    Rgbx,
    /// 0xRRGGBBAA, R in MSB.
    InternalRgba,
}

/// Heap-backed; consumer blits to the real framebuffer.
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
            depth: alloc::vec![0xFFFF_FFFF; size], // far plane (reversed-Z)
            format,
        }
    }

    pub fn clear(&mut self, clear_color: u32) {
        self.color.fill(clear_color);
        unsafe {
            core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len());
        }
    }

    pub fn clear_color(&mut self, clear_color: u32) {
        self.color.fill(clear_color);
    }

    pub fn clear_depth(&mut self) {
        unsafe {
            core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len());
        }
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

/// Writes color into an externally-owned buffer (e.g. MMIO framebuffer). Depth stays on heap.
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
    /// # Safety
    /// `ptr` must address ≥ `stride_px * height` valid, writable u32s for the
    /// target's entire lifetime. `stride_px` is in pixels, not bytes.
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
        unsafe {
            core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len());
        }
    }

    pub fn clear_color(&mut self, clear_color: u32) {
        let buf = unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) };
        buf.fill(clear_color);
    }

    pub fn clear_depth(&mut self) {
        unsafe {
            core::ptr::write_bytes(self.depth.as_mut_ptr(), 0xFF, self.depth.len());
        }
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

/// 0xRRGGBBAA → target format.
#[inline(always)]
pub fn convert_pixel(rgba: u32, format: TargetPixelFormat) -> u32 {
    match format {
        TargetPixelFormat::InternalRgba => rgba,
        TargetPixelFormat::Rgbx => {
            let r = (rgba >> 24) & 0xFF;
            let g = (rgba >> 16) & 0xFF;
            let b = (rgba >> 8) & 0xFF;
            r | (g << 8) | (b << 16)
        },
        TargetPixelFormat::Bgrx => {
            let r = (rgba >> 24) & 0xFF;
            let g = (rgba >> 16) & 0xFF;
            let b = (rgba >> 8) & 0xFF;
            b | (g << 8) | (r << 16)
        },
    }
}
