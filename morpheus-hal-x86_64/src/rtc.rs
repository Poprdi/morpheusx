//! MC146818 CMOS real-time clock, read once at boot to anchor wall time; the
//! kernel then extrapolates CLOCK_REALTIME off the monotonic TSC. x86-only:
//! CMOS access is two fixed I/O ports, so non-x86 HALs never anchor.

use crate::io::{port_in, port_out};

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

// CMOS register file. Bit 7 of the index port is the NMI-disable line; we keep
// NMIs enabled (clear) since the boot read is brief and atomic per byte.
const REG_SECONDS: u8 = 0x00;
const REG_MINUTES: u8 = 0x02;
const REG_HOURS: u8 = 0x04;
const REG_DAY: u8 = 0x07;
const REG_MONTH: u8 = 0x08;
const REG_YEAR: u8 = 0x09;
const REG_CENTURY: u8 = 0x32; // ACPI FADT default; ignored if implausible.
const REG_STATUS_A: u8 = 0x0A;
const REG_STATUS_B: u8 = 0x0B;

const STATUS_A_UIP: u8 = 0x80; // update-in-progress
const STATUS_B_24H: u8 = 0x02; // else 12-hour with PM flag in hour bit 7
const STATUS_B_BIN: u8 = 0x04; // else packed BCD
const HOUR_PM_FLAG: u8 = 0x80;

#[inline]
fn read_reg(reg: u8) -> u8 {
    port_out(CMOS_ADDR, 1, reg as u32);
    port_in(CMOS_DATA, 1) as u8
}

#[inline]
fn bcd_to_bin(v: u8) -> u8 {
    (v & 0x0F) + ((v >> 4) * 10)
}

/// True while the RTC is mid-update; reading registers then would tear.
#[inline]
fn update_in_progress() -> bool {
    read_reg(REG_STATUS_A) & STATUS_A_UIP != 0
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RawTime {
    sec: u8,
    min: u8,
    hour: u8,
    day: u8,
    month: u8,
    year: u8,
    century: u8,
}

fn read_raw() -> RawTime {
    RawTime {
        sec: read_reg(REG_SECONDS),
        min: read_reg(REG_MINUTES),
        hour: read_reg(REG_HOURS),
        day: read_reg(REG_DAY),
        month: read_reg(REG_MONTH),
        year: read_reg(REG_YEAR),
        century: read_reg(REG_CENTURY),
    }
}

/// Read the wall clock as Unix epoch seconds (UTC), or `None` if the RTC looks
/// uninitialized/absent. The "read twice and compare" loop tolerates a register
/// rollover landing between byte reads without disabling interrupts.
pub fn read_unix_secs() -> Option<u64> {
    let mut guard = 0u32;
    while update_in_progress() {
        guard += 1;
        if guard > 1_000_000 {
            return None; // wedged UIP bit ⇒ no usable RTC
        }
    }

    let mut last = read_raw();
    loop {
        // A fresh UIP means our snapshot may straddle an update; retake.
        while update_in_progress() {}
        let cur = read_raw();
        if cur == last {
            break;
        }
        last = cur;
    }

    let status_b = read_reg(REG_STATUS_B);
    let binary = status_b & STATUS_B_BIN != 0;
    let h24 = status_b & STATUS_B_24H != 0;

    let conv = |v: u8| if binary { v } else { bcd_to_bin(v) };

    let sec = conv(last.sec) as u64;
    let min = conv(last.min) as u64;

    // 12-hour mode keeps the PM flag in bit 7 of the raw hour; strip it before
    // BCD-decoding the low 7 bits, then fold 12h→24h.
    let hour = {
        let pm = !h24 && (last.hour & HOUR_PM_FLAG != 0);
        let raw = last.hour & !HOUR_PM_FLAG;
        let mut h = conv(raw) as u64;
        if !h24 {
            if h == 12 {
                h = 0; // 12 AM ⇒ 00:xx
            }
            if pm {
                h += 12; // 1–11 PM ⇒ 13–23
            }
        }
        h
    };

    let day = conv(last.day) as u64;
    let month = conv(last.month) as u64;
    let year_lo = conv(last.year) as u64;

    // Prefer the century register when it decodes to a sane 19xx–21xx; otherwise
    // assume 2000+ (QEMU/older firmware often leaves 0x32 at 0).
    let century = conv(last.century) as u64;
    let year = if (19..=21).contains(&century) {
        century * 100 + year_lo
    } else {
        2000 + year_lo
    };

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }

    Some(civil_to_unix_secs(year, month, day, hour, min, sec))
}

/// Days-from-civil (Howard Hinnant's algorithm) folded into Unix epoch seconds.
fn civil_to_unix_secs(year: u64, month: u64, day: u64, hour: u64, min: u64, sec: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let m = month as i64;
    let d = day as i64;

    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146097 + doe - 719468; // days since 1970-01-01

    let secs = days * 86400 + (hour as i64) * 3600 + (min as i64) * 60 + sec as i64;
    if secs < 0 {
        0
    } else {
        secs as u64
    }
}
