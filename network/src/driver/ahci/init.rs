//! AHCI driver initialization and configuration.

/// AHCI driver configuration.
///
/// Contains DMA buffer pointers and addresses for AHCI operation.
#[derive(Debug, Clone)]
pub struct AhciConfig {
    /// TSC frequency for timeouts
    pub tsc_freq: u64,
    
    /// Command List: CPU pointer (1K aligned, 1KB)
    pub cmd_list_cpu: *mut u8,
    /// Command List: Physical/bus address
    pub cmd_list_phys: u64,
    
    /// FIS Receive buffer: CPU pointer (256-byte aligned, 256 bytes)
    pub fis_cpu: *mut u8,
    /// FIS Receive buffer: Physical/bus address
    pub fis_phys: u64,
    
    /// Command Tables: CPU pointer (128-byte aligned, 256 bytes per slot)
    /// Total size: 32 slots × 256 bytes = 8KB
    pub cmd_tables_cpu: *mut u8,
    /// Command Tables: Physical/bus address
    pub cmd_tables_phys: u64,
    
    /// IDENTIFY buffer: CPU pointer (512 bytes for IDENTIFY data)
    pub identify_cpu: *mut u8,
    /// IDENTIFY buffer: Physical/bus address
    pub identify_phys: u64,
}

/// AHCI driver initialization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AhciInitError {
    /// Invalid configuration parameters
    InvalidConfig,
    /// HBA reset failed/timed out
    ResetFailed,
    /// No SATA device found on any port
    NoDeviceFound,
    /// Port stop timed out
    PortStopTimeout,
    /// Port start failed
    PortStartFailed,
    /// IDENTIFY DEVICE command failed
    IdentifyFailed,
    /// HBA doesn't support 64-bit addressing but addresses are >4GB
    No64BitSupport,
    /// Device not responding
    DeviceNotResponding,
    /// DMA setup failed
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
