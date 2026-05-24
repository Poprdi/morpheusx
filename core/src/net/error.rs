//! Network init errors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetInitError {
    DmaPoolInit,
    NoDmaMemory,
    HalInit,
    PciScanFailed,
    NoNetworkDevice,
    VirtioInit,
    InterfaceCreation,
    DhcpFailed,
    DhcpTimeout,
    StackInit,
    InvalidConfig,
    Timeout,
    /// Use post-EBS download_with_config() instead.
    Deprecated,
}

impl NetInitError {
    pub fn description(&self) -> &'static str {
        match self {
            Self::DmaPoolInit => "Failed to initialize DMA memory pool",
            Self::NoDmaMemory => "No suitable DMA memory region found",
            Self::HalInit => "Hardware abstraction layer init failed",
            Self::PciScanFailed => "PCI bus scan failed",
            Self::NoNetworkDevice => "No network device found on PCI bus",
            Self::VirtioInit => "VirtIO network device init failed",
            Self::InterfaceCreation => "Network interface creation failed",
            Self::DhcpFailed => "DHCP discovery failed",
            Self::DhcpTimeout => "DHCP timeout - no IP assigned",
            Self::StackInit => "Network stack initialization failed",
            Self::InvalidConfig => "Invalid network configuration",
            Self::Timeout => "Operation timed out",
            Self::Deprecated => "API deprecated - use post-EBS download_with_config()",
        }
    }
}

pub type NetInitResult<T> = Result<T, NetInitError>;
