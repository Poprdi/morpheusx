//! ISO 9660 datetime types: 7-byte (§9.1.5) and 17-byte ASCII (§8.4.26).

/// Directory-record timestamp, 7 bytes packed.
#[derive(Debug, Clone, Copy)]
pub struct DateTime7 {
    /// Year offset from 1900.
    pub year: u8,
    /// Month, 1-12.
    pub month: u8,
    /// Day, 1-31.
    pub day: u8,
    /// Hour, 0-23.
    pub hour: u8,
    /// Minute, 0-59.
    pub minute: u8,
    /// Second, 0-59.
    pub second: u8,
    /// GMT offset in 15-minute units, range -48..=52.
    pub gmt_offset: i8,
}

impl DateTime7 {
    /// Decode the 7-byte field.
    pub fn from_bytes(bytes: &[u8; 7]) -> Self {
        Self {
            year: bytes[0],
            month: bytes[1],
            day: bytes[2],
            hour: bytes[3],
            minute: bytes[4],
            second: bytes[5],
            gmt_offset: bytes[6] as i8,
        }
    }

    /// `1900 + year`.
    pub fn full_year(&self) -> u16 {
        1900 + self.year as u16
    }
}

/// 17-byte ASCII timestamp used in volume descriptors.
#[derive(Debug, Clone)]
pub struct DateTime17 {
    /// Year, 4-digit.
    pub year: u16,
    /// Month, 1-12.
    pub month: u8,
    /// Day, 1-31.
    pub day: u8,
    /// Hour, 0-23.
    pub hour: u8,
    /// Minute, 0-59.
    pub minute: u8,
    /// Second, 0-59.
    pub second: u8,
    /// Hundredths of a second.
    pub hundredths: u8,
    /// GMT offset in 15-minute units.
    pub gmt_offset: i8,
}

impl DateTime17 {
    /// Stub; ASCII decoding not yet implemented.
    pub fn from_bytes(_bytes: &[u8; 17]) -> Option<Self> {
        None
    }
}
