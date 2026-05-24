//! AHCI driver init types.

/// DMA buffer pointers required by `AhciDriver::new`. Alignment per AHCI §4.2:
/// cmd_list 1 KB, fis 256 B, cmd_tables 128 B each (8 KB total for 32 slots),
/// identify 2 B (use 512).
#[derive(Debug, Clone)]
pub struct AhciConfig {
    pub tsc_freq: u64,

    pub cmd_list_cpu: *mut u8,
    pub cmd_list_phys: u64,

    pub fis_cpu: *mut u8,
    pub fis_phys: u64,

    pub cmd_tables_cpu: *mut u8,
    pub cmd_tables_phys: u64,

    pub identify_cpu: *mut u8,
    pub identify_phys: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AhciInitError {
    InvalidConfig,
    ResetFailed,
    NoDeviceFound,
    PortStopTimeout,
    PortStartFailed,
    IdentifyFailed,
    /// HBA is 32-bit but DMA region crosses the 4 GB line.
    No64BitSupport,
    DeviceNotResponding,
    DmaSetupFailed,
}

impl core::fmt::Display for AhciInitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidConfig => write!(f, "Invalid AHCI configuration"),
            Self::ResetFailed => write!(f, "AHCI HBA reset failed"),
            Self::NoDeviceFound => write!(f, "No SATA device found"),
            Self::PortStopTimeout => write!(f, "Port stop timed out"),
            Self::PortStartFailed => write!(f, "Port start failed"),
            Self::IdentifyFailed => write!(f, "IDENTIFY DEVICE failed"),
            Self::No64BitSupport => write!(f, "64-bit addressing not supported"),
            Self::DeviceNotResponding => write!(f, "Device not responding"),
            Self::DmaSetupFailed => write!(f, "DMA setup failed"),
        }
    }
}
