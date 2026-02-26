use crate::math::fixed::Fx16;

/// A horizontal span (scanline segment) produced by the edge walker.
///
/// The edge function approach: we walk left and right edges top-to-bottom,
/// producing one Span per scanline. Each span stores the start/end x and
/// interpolated attributes at both endpoints. The inner loop then steps
/// between them using fixed-point increments.
///
/// This is the Quake 1/2 approach: edge → span → pixel. Modern GPUs
/// use tile-based rasterization, but for software rendering, span-based
/// is still king because it's perfectly cache-friendly (sequential writes
/// to the framebuffer row).
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub y: u32,
    pub x_left: Fx16,
    pub x_right: Fx16,

    // Interpolated attributes at left edge (all pre-divided by w for perspective correction)
    pub inv_w_left: Fx16,
    pub inv_w_right: Fx16,

    pub r_left: Fx16,
    pub r_right: Fx16,
    pub g_left: Fx16,
    pub g_right: Fx16,
    pub b_left: Fx16,
    pub b_right: Fx16,

    pub u_left: Fx16,
    pub u_right: Fx16,
    pub v_left: Fx16,
    pub v_right: Fx16,

    pub z_left: Fx16,
    pub z_right: Fx16,

    pub fog_left: Fx16,
    pub fog_right: Fx16,
}

impl Span {
    pub const EMPTY: Self = Self {
        y: 0,
        x_left: Fx16::ZERO, x_right: Fx16::ZERO,
        inv_w_left: Fx16::ZERO, inv_w_right: Fx16::ZERO,
        r_left: Fx16::ZERO, r_right: Fx16::ZERO,
        g_left: Fx16::ZERO, g_right: Fx16::ZERO,
        b_left: Fx16::ZERO, b_right: Fx16::ZERO,
        u_left: Fx16::ZERO, u_right: Fx16::ZERO,
        v_left: Fx16::ZERO, v_right: Fx16::ZERO,
        z_left: Fx16::ZERO, z_right: Fx16::ZERO,
        fog_left: Fx16::ZERO, fog_right: Fx16::ZERO,
    };

    /// Width of this span in pixels.
    #[inline]
    pub fn width(&self) -> i32 {
        self.x_right.ceil() - self.x_left.ceil()
    }
}

/// Compute per-pixel step increments for a span.
///
/// This pre-computes 1/(x_right - x_left) once, then multiplies each
/// attribute delta by it. The inner pixel loop only needs additions.
#[derive(Debug, Clone, Copy)]
pub struct SpanGradients {
    pub inv_w_step: Fx16,
    pub r_step: Fx16,
    pub g_step: Fx16,
    pub b_step: Fx16,
    pub u_step: Fx16,
    pub v_step: Fx16,
    pub z_step: Fx16,
    pub fog_step: Fx16,
}

impl SpanGradients {
    pub fn from_span(span: &Span) -> Self {
        let dx = span.x_right - span.x_left;
        if dx.0 <= 0 {
            return Self {
                inv_w_step: Fx16::ZERO, r_step: Fx16::ZERO, g_step: Fx16::ZERO,
                b_step: Fx16::ZERO, u_step: Fx16::ZERO, v_step: Fx16::ZERO,
                z_step: Fx16::ZERO, fog_step: Fx16::ZERO,
            };
        }
        let inv_dx = Fx16::ONE.div(dx);

        Self {
            inv_w_step: (span.inv_w_right - span.inv_w_left).mul(inv_dx),
            r_step: (span.r_right - span.r_left).mul(inv_dx),
            g_step: (span.g_right - span.g_left).mul(inv_dx),
            b_step: (span.b_right - span.b_left).mul(inv_dx),
            u_step: (span.u_right - span.u_left).mul(inv_dx),
            v_step: (span.v_right - span.v_left).mul(inv_dx),
            z_step: (span.z_right - span.z_left).mul(inv_dx),
            fog_step: (span.fog_right - span.fog_left).mul(inv_dx),
        }
    }
}
