//! AHCI port status types (PxSSTS decode).

/// PxSSTS.DET.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PortDetection {
    None = 0,
    Present = 1,
    Ready = 3,
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

/// Decoded PxSIG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Ata,
    Atapi,
    Semb,
    PortMultiplier,
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

/// PxSSTS.SPD: SATA Gen1/2/3 = 1.5/3/6 Gbps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSpeed {
    None,
    Gen1,
    Gen2,
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

#[derive(Debug, Clone, Copy)]
pub struct PortStatus {
    pub detection: PortDetection,
    pub speed: LinkSpeed,
    pub device_type: DeviceType,
    pub busy: bool,
    pub drq: bool,
    pub error: bool,
}
