//! Network initialization orchestrator.
//!
//! This module performs COMPLETE network initialization - the bootstrap/bootloader
//! just calls `NetworkInit::initialize()` and waits for success or failure.
//!
//! # Initialization Sequence
//!
//! 1. Initialize DMA pool (via dma-pool crate)
//! 2. Initialize HAL (via morpheus_network)
//! 3. Scan PCI for network devices
//! 4. Create device via DeviceFactory (auto-detects VirtIO, Intel, Realtek, etc.)
//! 5. Create HTTP client with DHCP
//! 6. Wait for DHCP to assign IP address
//! 7. Return NetworkStatus with IP info
//!
//! # Error Handling
//!
//! All errors are logged to the ring buffer for later display by the bootstrap UI.
//! Use `error_log_pop()` to retrieve error entries after a failure.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_core::net::{NetworkInit, InitConfig, error_log_pop};
//!
//! fn get_time_ms() -> u64 { /* ... */ }
//!
//! let config = InitConfig::for_qemu();
//! match NetworkInit::initialize(&config, get_time_ms) {
//!     Ok(status) => {
//!         // Network ready, status.ip_address has our IP
//!         println!("Got IP: {}.{}.{}.{}", 
//!             status.ip_address[0], status.ip_address[1],
//!             status.ip_address[2], status.ip_address[3]);
//!     }
//!     Err(e) => {
//!         // Dump error ring buffer to UI
//!         while let Some(entry) = error_log_pop() {
//!             println!("{}", entry.message());
//!         }
//!     }
//! }
//! ```

use super::config::InitConfig;
use super::error::{NetInitError, NetInitResult};
use super::status::NetworkStatus;
use super::ring_buffer::{error_log, debug_log, drain_network_logs, InitStage};

use morpheus_network::{
    DeviceFactory, DeviceConfig, UnifiedNetDevice,
    NativeHttpClient, NetConfig, StaticHal,
};

/// Network initialization result containing the client and status.
/// 
/// Returned on successful initialization. The client is ready to use
/// for HTTP requests. The status contains IP configuration info.
pub struct NetworkInitResult {
    /// The HTTP client, ready for requests.
    pub client: NativeHttpClient<UnifiedNetDevice>,
    /// Network status with IP info.
    pub status: NetworkStatus,
}

/// Network initialization orchestrator.
///
/// Handles complete network initialization from DMA pool setup through
/// DHCP completion. The bootstrap just calls `initialize()` and waits.
pub struct NetworkInit;

impl NetworkInit {
    /// Perform complete network initialization.
    ///
    /// This is the main entry point. It handles the entire sequence:
    /// DMA → HAL → PCI scan → Device creation → DHCP → IP assignment.
    ///
    /// # Arguments
    ///
    /// * `config` - Initialization configuration
    /// * `get_time_ms` - Function to get current time in milliseconds
    ///
    /// # Returns
    ///
    /// * `Ok(NetworkInitResult)` - Network ready, contains client and status
    /// * `Err(NetInitError)` - Failed, check ring buffer for details
    pub fn initialize(
        config: &InitConfig,
        get_time_ms: fn() -> u64,
    ) -> NetInitResult<NetworkInitResult> {
        let start_time = get_time_ms();
        
        debug_log(InitStage::General, "Starting network initialization");

        // Step 1: Initialize DMA pool
        debug_log(InitStage::DmaPool, "Initializing DMA memory pool");
        Self::init_dma_pool(config)?;
        debug_log(InitStage::DmaPool, "DMA pool initialized");

        // Step 2: Initialize HAL
        debug_log(InitStage::Hal, "Initializing hardware abstraction layer");
        Self::init_hal()?;
        debug_log(InitStage::Hal, "HAL initialized");

        // Step 3: Scan PCI and create device
        debug_log(InitStage::PciScan, "Scanning PCI bus for network devices");
        let device = Self::create_device(config)?;
        debug_log(InitStage::VirtioDevice, "Network device created");

        // Step 4: Get MAC address before we move the device
        let mac_address = device.mac_address();
        debug_log(InitStage::VirtioDevice, "MAC address retrieved");

        // Step 5: Create HTTP client with DHCP
        debug_log(InitStage::NetworkClient, "Creating HTTP client");
        let mut client = NativeHttpClient::new(device, NetConfig::Dhcp, get_time_ms);
        debug_log(InitStage::NetworkClient, "HTTP client created, starting DHCP");

        // Step 6: Wait for DHCP
        debug_log(InitStage::Dhcp, "Waiting for DHCP lease");
        client.wait_for_network(config.dhcp_timeout_ms)
            .map_err(|e| {
                error_log(InitStage::Dhcp, "DHCP failed or timed out");
                drain_network_logs(); // Capture any network crate debug info
                NetInitError::DhcpTimeout
            })?;

        // Extract IP information
        let ip = client.ip_address()
            .map(|ip| ip.octets())
            .unwrap_or([0, 0, 0, 0]);

        let init_time_ms = get_time_ms() - start_time;

        let status = NetworkStatus {
            ip_address: ip,
            subnet_mask: [255, 255, 255, 0], // Default, could extract from DHCP
            gateway: [0, 0, 0, 0],           // Could extract from DHCP
            dns_server: None,                 // Could extract from DHCP
            mac_address,
            init_time_ms,
            is_dhcp: true,
        };

        debug_log(InitStage::General, "Network initialization complete");

        Ok(NetworkInitResult { client, status })
    }

    /// Initialize DMA memory pool.
    ///
    /// Tries cave discovery first if image bounds provided,
    /// falls back to static pool.
    fn init_dma_pool(config: &InitConfig) -> NetInitResult<()> {
        // Try cave discovery if image bounds provided
        if let (Some(base), Some(end)) = (config.image_base, config.image_end) {
            debug_log(InitStage::DmaPool, "Trying DMA cave discovery");
            // SAFETY: Caller guarantees image bounds are valid
            let result = unsafe { dma_pool::DmaPool::init_from_caves(base, end) };
            if result.is_ok() {
                debug_log(InitStage::DmaPool, "Cave discovery successful");
                return Ok(());
            }
            debug_log(InitStage::DmaPool, "Cave discovery failed, trying static");
            // Fall through to static if cave discovery fails
        }

        // Fall back to static pool
        if config.use_static_dma {
            debug_log(InitStage::DmaPool, "Using static DMA pool");
            dma_pool::DmaPool::init_static();
            return Ok(());
        }

        error_log(InitStage::DmaPool, "No DMA memory source available");
        Err(NetInitError::NoDmaMemory)
    }

    /// Initialize hardware abstraction layer.
    fn init_hal() -> NetInitResult<()> {
        StaticHal::init();
        Ok(())
    }

    /// Create network device using the factory.
    fn create_device(config: &InitConfig) -> NetInitResult<UnifiedNetDevice> {
        // Build DeviceConfig from our InitConfig
        let device_config = DeviceConfig {
            ecam_base: config.ecam_base,
            preferred_driver: morpheus_network::device::factory::PreferredDriver::Any,
            scan_bus: 0,
        };

        // Scan for devices first to log what we find
        match DeviceFactory::scan(&device_config) {
            Ok(devices) => {
                if devices.is_empty() {
                    error_log(InitStage::PciScan, "No network devices found on PCI bus");
                    return Err(NetInitError::NoNetworkDevice);
                }

                // Log what we found
                for (i, dev) in devices.iter().enumerate() {
                    let mut msg = [0u8; 64];
                    let name = dev.driver_type.name();
                    let len = name.len().min(50);
                    msg[..len].copy_from_slice(&name.as_bytes()[..len]);
                    if let Ok(s) = core::str::from_utf8(&msg[..len]) {
                        debug_log(InitStage::PciScan, s);
                    }
                }
            }
            Err(_) => {
                error_log(InitStage::PciScan, "PCI scan failed");
                return Err(NetInitError::PciScanFailed);
            }
        }

        // Create device
        DeviceFactory::create_auto(&device_config)
            .map_err(|e| {
                error_log(InitStage::VirtioDevice, "Device creation failed");
                drain_network_logs();
                NetInitError::VirtioInit
            })
    }

    /// Quick check if network initialization is possible.
    ///
    /// Does minimal checks without full initialization.
    /// Useful for early bootstrap to decide whether to attempt network.
    pub fn can_initialize() -> bool {
        // In a real implementation, this would check for PCI bus access, etc.
        true
    }

    /// Initialize only prerequisites (DMA + HAL) without device creation.
    ///
    /// Useful if the caller wants to do custom device handling.
    pub fn init_prerequisites(config: &InitConfig) -> NetInitResult<()> {
        Self::init_dma_pool(config)?;
        Self::init_hal()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = InitConfig::default();
        assert_eq!(config.dhcp_timeout_ms, 30_000);
        assert!(config.use_static_dma);
    }

    #[test]
    fn test_config_for_qemu() {
        let config = InitConfig::for_qemu();
        assert_eq!(config.dhcp_timeout_ms, 10_000);
    }

    #[test]
    fn test_error_descriptions() {
        let error = NetInitError::DhcpTimeout;
        assert!(!error.description().is_empty());
    }

    #[test]
    fn test_status_ip_str() {
        let mut status = NetworkStatus::new();
        status.ip_address = [192, 168, 1, 100];
        let ip_str = status.ip_str();
        let s = core::str::from_utf8(&ip_str).unwrap().trim();
        assert!(s.starts_with("192.168.1.100"));
    }
}

