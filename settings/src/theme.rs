// semantic color tokens for the oneiric chamber system.
// not your typical rgba palette — these map to meaning, not pixels.

#[derive(Clone, Copy)]
pub struct OneiricTheme {
    // substrate — the deepest background layer
    pub substrate: u32,
    // contour — borders, dividers, structural edges
    pub contour: u32,
    // glyph — primary text
    pub glyph: u32,
    // glyph_dim — secondary/muted text
    pub glyph_dim: u32,
    // signal — accent, active selection, highlighted elements
    pub signal: u32,
    // signal_dim — muted accent for inactive highlights
    pub signal_dim: u32,
    // warning — caution states
    pub warning: u32,
    // destructive — dangerous action highlights
    pub destructive: u32,
    // immutable — read-only info display
    pub immutable: u32,
    // telemetry — live data readouts
    pub telemetry: u32,
    // archive — historical/log entries
    pub archive: u32,
    // surface — elevated panel backgrounds
    pub surface: u32,
    // input_bg — text input field backgrounds
    pub input_bg: u32,
    // focus_ring — keyboard focus indicator
    pub focus_ring: u32,
    // armed — the moment before the trigger pull
    pub armed: u32,
    // success — positive outcome confirmation
    pub success: u32,
    // rail_bg — navigation rail background
    pub rail_bg: u32,
    // rail_active — active rail item
    pub rail_active: u32,
    // bar_bg — command bar background
    pub bar_bg: u32,
    // strip_bg — top command strip background
    pub strip_bg: u32,
}

impl OneiricTheme {
    // dark mode — the canonical morpheus aesthetic.
    // deep blacks, desaturated greens, everything feels like 3am serial console.
    pub const fn dark() -> Self {
        Self {
            substrate:   pack(10, 10, 14),
            contour:     pack(40, 45, 40),
            glyph:       pack(180, 200, 180),
            glyph_dim:   pack(90, 100, 90),
            signal:      pack(0, 170, 0),
            signal_dim:  pack(0, 85, 0),
            warning:     pack(200, 170, 0),
            destructive: pack(200, 40, 40),
            immutable:   pack(100, 120, 140),
            telemetry:   pack(0, 200, 180),
            archive:     pack(120, 140, 120),
            surface:     pack(18, 20, 22),
            input_bg:    pack(14, 16, 18),
            focus_ring:  pack(85, 255, 85),
            armed:       pack(255, 100, 0),
            success:     pack(0, 200, 80),
            rail_bg:     pack(14, 14, 18),
            rail_active: pack(0, 85, 0),
            bar_bg:      pack(12, 12, 16),
            strip_bg:    pack(16, 16, 20),
        }
    }

    // light mode — still oneiric, just inverted substrate.
    // high contrast, same signal semantics.
    pub const fn light() -> Self {
        Self {
            substrate:   pack(220, 225, 220),
            contour:     pack(160, 165, 160),
            glyph:       pack(20, 25, 20),
            glyph_dim:   pack(100, 110, 100),
            signal:      pack(0, 130, 0),
            signal_dim:  pack(0, 70, 0),
            warning:     pack(180, 140, 0),
            destructive: pack(180, 30, 30),
            immutable:   pack(60, 80, 100),
            telemetry:   pack(0, 150, 130),
            archive:     pack(80, 100, 80),
            surface:     pack(210, 215, 210),
            input_bg:    pack(235, 240, 235),
            focus_ring:  pack(0, 170, 0),
            armed:       pack(220, 80, 0),
            success:     pack(0, 160, 60),
            rail_bg:     pack(200, 205, 200),
            rail_active: pack(0, 130, 0),
            bar_bg:      pack(190, 195, 190),
            strip_bg:    pack(200, 205, 200),
        }
    }
}

// pack rgb into bgrx u32. uefi said b comes first and uefi answers to no one.
#[inline(always)]
pub const fn pack(r: u8, g: u8, b: u8) -> u32 {
    (b as u32) | ((g as u32) << 8) | ((r as u32) << 16) | (0xFF << 24)
}

// pack for rgbx format (the other one)
#[inline(always)]
pub const fn pack_rgbx(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | (0xFF << 24)
}

// runtime format-aware packing
#[inline(always)]
pub fn pack_pixel(r: u8, g: u8, b: u8, is_bgrx: bool) -> u32 {
    if is_bgrx {
        pack(r, g, b)
    } else {
        pack_rgbx(r, g, b)
    }
}
