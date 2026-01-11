//! Date/time parsing
//!
//! ISO9660 has two datetime formats: 7-byte and 17-byte.

/// 7-byte directory record datetime
#[derive(Debug, Clone, Copy)]
pub struct DateTime7 {
    /// Years since 1900
    pub year: u8,

    /// Month (1-12)
    pub month: u8,

    /// Day (1-31)
    pub day: u8,

    /// Hour (0-23)
    pub hour: u8,

    /// Minute (0-59)
    pub minute: u8,

    /// Second (0-59)
    pub second: u8,

    /// GMT offset in 15-minute intervals (-48 to +52)
    pub gmt_offset: i8,
}

impl DateTime7 {
    /// Parse from 7-byte array
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

    /// Get full year (1900 + year)
    pub fn full_year(&self) -> u16 {
        1900 + self.year as u16
    }
}

/// 17-byte ASCII datetime (volume descriptors)
#[derive(Debug, Clone)]
pub struct DateTime17 {
    /// Year (4 ASCII digits)
    pub year: u16,

    /// Month (2 ASCII digits, 1-12)
    pub month: u8,

    /// Day (2 ASCII digits, 1-31)
    pub day: u8,

    /// Hour (2 ASCII digits, 0-23)
    pub hour: u8,

    /// Minute (2 ASCII digits, 0-59)
    pub minute: u8,

    /// Second (2 ASCII digits, 0-59)
    pub second: u8,

    /// Hundredths (2 ASCII digits)
    pub hundredths: u8,

    /// GMT offset in 15-minute intervals
    pub gmt_offset: i8,
}

impl DateTime17 {
    /// Parse from 17-byte ASCII string
    pub fn from_bytes(_bytes: &[u8; 17]) -> Option<Self> {
        // TODO: Parse ASCII digits
        None
    }
}
