//! HID mouse report handling. Boot protocol gives a fixed layout; for devices
//! that won't enter boot protocol we parse the report descriptor instead, so
//! any mouse is decoded through the same path. PS/2 and USB are mutually
//! exclusive (see input.rs).

use crate::hid_iface::{HidField, MouseLayout};
use crate::usb::hid::sink;

const PAGE_GENERIC_DESKTOP: u32 = 0x01;
const PAGE_BUTTON: u32 = 0x09;
const USAGE_X: u32 = 0x30;
const USAGE_Y: u32 = 0x31;
const USAGE_WHEEL: u32 = 0x38;

/// Parse a HID report descriptor and locate the button/X/Y/wheel fields of the
/// first pointer report. Handles report IDs and arbitrary field widths (8/12/16
/// bit), so report-protocol mice — not just boot-protocol ones — decode
/// correctly. Returns `None` if no X+Y pair is found (caller falls back to the
/// boot layout). HID 1.11 §6.2.2.
pub fn parse_mouse_layout(desc: &[u8]) -> Option<MouseLayout> {
    let mut usage_page: u32 = 0;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;
    let mut report_id: u8 = 0;
    let mut logical_min: i32 = 0;

    let mut usages: [u32; 32] = [0; 32];
    let mut usage_len: usize = 0;
    let mut usage_min: u32 = 0;
    let mut usage_max: u32 = 0;

    // Bits consumed so far in the current report (excludes any report-ID byte).
    let mut bit_offset: u16 = 0;

    let mut layout = MouseLayout {
        report_id: 0,
        buttons: HidField::default(),
        x: HidField::default(),
        y: HidField::default(),
        wheel: HidField::default(),
    };
    let mut have_x = false;
    let mut have_y = false;

    let mut i = 0usize;
    while i < desc.len() {
        let prefix = desc[i];
        i += 1;
        if prefix == 0xFE {
            // Long item: bDataSize, bLongItemTag, then data. Not used by mice.
            if i >= desc.len() {
                break;
            }
            let dlen = desc[i] as usize;
            i = i.saturating_add(2 + dlen);
            continue;
        }

        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let btype = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0F;
        if i + size > desc.len() {
            break;
        }
        let mut data: u32 = 0;
        let mut b = 0usize;
        while b < size {
            data |= (desc[i + b] as u32) << (8 * b);
            b += 1;
        }
        let sdata: i32 = match size {
            1 => (data as u8) as i8 as i32,
            2 => (data as u16) as i16 as i32,
            4 => data as i32,
            _ => 0,
        };
        i += size;

        match btype {
            1 => match tag {
                // Global items.
                0x0 => usage_page = data,
                0x1 => logical_min = sdata,
                0x7 => report_size = data,
                0x8 => {
                    report_id = data as u8;
                    bit_offset = 0;
                },
                0x9 => report_count = data,
                _ => {},
            },
            2 => match tag {
                // Local items.
                0x0 => {
                    if usage_len < usages.len() {
                        usages[usage_len] = data & 0xFFFF;
                        usage_len += 1;
                    }
                },
                0x1 => usage_min = data & 0xFFFF,
                0x2 => usage_max = data & 0xFFFF,
                _ => {},
            },
            0 => {
                // Main items. Only Input (tag 0x8) defines report fields.
                if tag == 0x8 {
                    let is_const = data & 0x01 != 0;
                    let is_var = data & 0x02 != 0;
                    let item_bits = (report_size * report_count) as u16;
                    if !is_const && is_var {
                        if usage_page == PAGE_BUTTON {
                            if layout.buttons.bit_size == 0 {
                                layout.buttons = HidField {
                                    bit_offset,
                                    bit_size: (report_size * report_count).min(255) as u8,
                                    signed: false,
                                };
                            }
                        } else if usage_page == PAGE_GENERIC_DESKTOP {
                            let mut f = 0u32;
                            while f < report_count {
                                let usage = if usage_len > 0 {
                                    usages[(f as usize).min(usage_len - 1)]
                                } else if usage_max >= usage_min && usage_max != 0 {
                                    usage_min + f
                                } else {
                                    0
                                };
                                let fld = HidField {
                                    bit_offset: bit_offset + (f * report_size) as u16,
                                    bit_size: report_size.min(255) as u8,
                                    signed: logical_min < 0,
                                };
                                match usage {
                                    USAGE_X => {
                                        layout.x = fld;
                                        have_x = true;
                                        layout.report_id = report_id;
                                    },
                                    USAGE_Y => {
                                        layout.y = fld;
                                        have_y = true;
                                    },
                                    USAGE_WHEEL => layout.wheel = fld,
                                    _ => {},
                                }
                                f += 1;
                            }
                        }
                    }
                    bit_offset = bit_offset.saturating_add(item_bits);
                }
                // Every main item clears local (usage) state.
                usage_len = 0;
                usage_min = 0;
                usage_max = 0;
            },
            _ => {},
        }
    }

    if have_x && have_y {
        Some(layout)
    } else {
        None
    }
}

/// Extract one field from a report, sign-extending if `f.signed`.
fn extract(report: &[u8], f: HidField, base_bits: usize) -> i32 {
    if f.bit_size == 0 {
        return 0;
    }
    let mut val: u32 = 0;
    let mut b = 0u8;
    while b < f.bit_size {
        let bit = base_bits + f.bit_offset as usize + b as usize;
        let byte = bit >> 3;
        if byte >= report.len() {
            break;
        }
        let bitval = (report[byte] >> (bit & 7)) & 1;
        val |= (bitval as u32) << b;
        b += 1;
    }
    if f.signed && f.bit_size < 32 {
        let sign = 1u32 << (f.bit_size - 1);
        if val & sign != 0 {
            return (val | !((sign << 1).wrapping_sub(1))) as i32;
        }
    }
    val as i32
}

/// Decode a raw mouse report into (buttons, dx, dy, wheel) using `layout`.
/// Returns `None` when a report-ID prefix doesn't match this layout's report.
pub fn decode_mouse(layout: &MouseLayout, raw: &[u8]) -> Option<(u8, i16, i16, i8)> {
    let base = if layout.report_id != 0 {
        if raw.first().copied() != Some(layout.report_id) {
            return None;
        }
        8
    } else {
        0
    };
    let buttons = (extract(raw, layout.buttons, base) as u32 & 0xFF) as u8;
    let dx = extract(raw, layout.x, base).clamp(-32768, 32767) as i16;
    let dy = extract(raw, layout.y, base).clamp(-32768, 32767) as i16;
    let wheel = extract(raw, layout.wheel, base).clamp(-128, 127) as i8;
    Some((buttons, dx, dy, wheel))
}

/// Decode `raw` and forward to the kernel mouse sink.
pub fn dispatch_mouse(layout: &MouseLayout, raw: &[u8]) {
    if let Some((buttons, dx, dy, wheel)) = decode_mouse(layout, raw) {
        sink::push_mouse(dx, dy, buttons & 0x07, wheel);
    }
}
