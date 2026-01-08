//! Network initialization errors.
//!
//! Error types for the orchestration layer. Maps errors from underlying
//! crates into a unified error type for the bootloader.

/// Network initialization error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetInitError {
    /// DMA pool initialization failed.
    DmaPoolInit,
    /// No suitable DMA memory found.
    NoDmaMemory,
    /// HAL initialization failed.
    HalInit,
    /// PCI bus scan failed.
    PciScanFailed,
    /// No network device found on PCI bus.
    NoNetworkDevice,
    /// VirtIO device initialization failed.
    VirtioInit,
    /// Network interface creation failed.
    InterfaceCreation,
    /// DHCP discovery failed.
    DhcpFailed,
    /// DHCP timeout - no IP address assigned.
    DhcpTimeout,
    /// Network stack initialization failed.
    StackInit,
    /// Invalid configuration provided.
    InvalidConfig,
    /// Operation timed out.
    Timeout,
}

impl NetInitError {
    /// Get a human-readable description of the error.
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
        }
    }
}

/// Result type for network initialization.
pub type NetInitResult<T> = Result<T, NetInitError>;
