//! AHCI port management types.

/// Port detection status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PortDetection {
    /// No device detected
    None = 0,
    /// Device present but no PHY communication
    Present = 1,
    /// Device present and PHY communication established
    Ready = 3,
    /// PHY in offline mode
    Offline = 4,
}

impl From<u32> for PortDetection {
    fn from(det: u32) -> Self {
        match det & 0x0F {
            0 => Self::None,
            1 => Self::Present,
            3 => Self::Ready,
            4 => Self::Offline,
            _ => Self::None,
        }
    }
}

/// Device type detected from signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    /// ATA hard drive or SSD
    Ata,
    /// ATAPI device (CD/DVD)
    Atapi,
    /// Enclosure Management Bridge
    Semb,
    /// Port Multiplier
    PortMultiplier,
    /// Unknown device type
    Unknown(u32),
}

impl From<u32> for DeviceType {
    fn from(sig: u32) -> Self {
        match sig {
            0x00000101 => Self::Ata,
            0xEB140101 => Self::Atapi,
            0xC33C0101 => Self::Semb,
            0x96690101 => Self::PortMultiplier,
            _ => Self::Unknown(sig),
        }
    }
}

/// Link speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSpeed {
    /// No speed negotiated
    None,
    /// SATA Gen1 (1.5 Gbps)
    Gen1,
    /// SATA Gen2 (3.0 Gbps)
    Gen2,
    /// SATA Gen3 (6.0 Gbps)
    Gen3,
}

impl From<u32> for LinkSpeed {
    fn from(spd: u32) -> Self {
        match (spd >> 4) & 0x0F {
            0 => Self::None,
            1 => Self::Gen1,
            2 => Self::Gen2,
            3 => Self::Gen3,
            _ => Self::None,
        }
    }
}

/// Port status information.
#[derive(Debug, Clone, Copy)]
pub struct PortStatus {
    /// Detection status
    pub detection: PortDetection,
    /// Link speed
    pub speed: LinkSpeed,
    /// Device type (from signature)
    pub device_type: DeviceType,
    /// Is device busy
    pub busy: bool,
    /// Data request pending
    pub drq: bool,
    /// Error occurred
    pub error: bool,
}
