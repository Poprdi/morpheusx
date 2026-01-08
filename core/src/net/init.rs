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

/// Helper to format TX/RX stats into a buffer.
fn write_to_buf(buf: &mut [u8], tx: u32, rx: u32) -> Result<usize, ()> {
    use core::fmt::Write;
    struct BufWriter<'a> { buf: &'a mut [u8], pos: usize }
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len().saturating_sub(self.pos);
            let to_write = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_write].copy_from_slice(&bytes[..to_write]);
            self.pos += to_write;
            if to_write < bytes.len() { Err(core::fmt::Error) } else { Ok(()) }
        }
    }
    let mut writer = BufWriter { buf, pos: 0 };
    write!(&mut writer, "TX: {} RX: {}", tx, rx).map_err(|_| ())?;
    Ok(writer.pos)
}

use morpheus_network::{
    DeviceFactory, DeviceConfig, UnifiedNetDevice,
    NativeHttpClient, NetConfig, StaticHal,
    NetworkDevice,  // Trait for mac_address()
    tsc_delay_us,   // TSC-based delay
};
use morpheus_network::stack::{tx_packet_count, rx_packet_count};

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
        // No-op poll function for basic init
        Self::initialize_with_poll(config, get_time_ms, || {})
    }

    /// Initialize with display polling.
    ///
    /// Same as `initialize`, but calls `poll_display` periodically so the
    /// caller can update the UI with new log entries.
    ///
    /// # Arguments
    ///
    /// * `config` - Initialization configuration
    /// * `get_time_ms` - Function to get current time in milliseconds
    /// * `poll_display` - Called periodically to let caller update display
    pub fn initialize_with_poll<F>(
        config: &InitConfig,
        get_time_ms: fn() -> u64,
        mut poll_display: F,
    ) -> NetInitResult<NetworkInitResult>
    where
        F: FnMut(),
    {
        let start_time = get_time_ms();
        
        debug_log(InitStage::General, "Starting network initialization");
        poll_display();

        // Step 1: Initialize DMA pool
        debug_log(InitStage::DmaPool, "Initializing DMA memory pool");
        poll_display();
        Self::init_dma_pool(config)?;
        debug_log(InitStage::DmaPool, "DMA pool initialized");
        poll_display();

        // Step 2: Initialize HAL
        debug_log(InitStage::Hal, "Initializing HAL");
        poll_display();
        Self::init_hal()?;
        debug_log(InitStage::Hal, "HAL initialized");
        poll_display();

        // Step 3: Scan PCI and create device
        debug_log(InitStage::PciScan, "Scanning PCI bus");
        poll_display();
        let device = Self::create_device(config)?;
        debug_log(InitStage::VirtioDevice, "Device created, self-test passed");
        poll_display();

        // Step 4: Get MAC address before we move the device
        let mac_address = device.mac_address();
        debug_log(InitStage::VirtioDevice, "TX/RX queues ready");
        poll_display();

        // Step 5: Create HTTP client with DHCP
        debug_log(InitStage::NetworkClient, "Creating HTTP client");
        poll_display();
        let mut client = NativeHttpClient::new(device, NetConfig::Dhcp, get_time_ms);
        debug_log(InitStage::NetworkClient, "Starting DHCP");
        poll_display();

        // Step 6: Wait for DHCP with polling
        debug_log(InitStage::Dhcp, "Waiting for DHCP lease...");
        poll_display();
        
        // Poll-based DHCP wait so we can update display
        let dhcp_start = get_time_ms();
        let mut last_progress_log = dhcp_start;
        loop {
            client.poll();
            poll_display();
            
            // Small delay to avoid spinning CPU at 100%
            tsc_delay_us(1000); // 1ms between polls
            
            if client.ip_address().is_some() {
                break; // Got IP!
            }
            
            let elapsed = get_time_ms() - dhcp_start;
            if elapsed > config.dhcp_timeout_ms {
                error_log(InitStage::Dhcp, "DHCP timeout");
                drain_network_logs();
                return Err(NetInitError::DhcpTimeout);
            }
            
            // Log progress every 2 seconds with packet stats
            if get_time_ms() - last_progress_log > 2000 {
                // Format stats: "TX: 5 RX: 3"
                let tx = tx_packet_count();
                let rx = rx_packet_count();
                let mut msg_buf = [0u8; 64];
                let msg = if let Ok(len) = write_to_buf(&mut msg_buf, tx, rx) {
                    core::str::from_utf8(&msg_buf[..len]).unwrap_or("TX/RX stats error")
                } else {
                    "Stats unavailable"
                };
                debug_log(InitStage::Dhcp, msg);
                last_progress_log = get_time_ms();
                poll_display();
            }
        }

        // Extract IP information
        let ip = client.ip_address()
            .map(|ip| ip.octets())
            .unwrap_or([0, 0, 0, 0]);

        debug_log(InitStage::Dhcp, "DHCP complete");
        poll_display();

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
        poll_display();

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
        // Build DeviceConfig - let factory use auto-probe
        let device_config = DeviceConfig {
            ecam_base: config.ecam_base, // None = auto-probe
            preferred_driver: morpheus_network::device::factory::PreferredDriver::Any,
            scan_bus: 0,
        };

        debug_log(InitStage::PciScan, "Probing PCI access methods...");
        let (devices, method) = DeviceFactory::scan(&device_config)
            .map_err(|_| {
                error_log(InitStage::PciScan, "PCI scan failed");
                NetInitError::PciScanFailed
            })?;

        match method {
            morpheus_network::PciAccessMethod::LegacyIo => {
                debug_log(InitStage::PciScan, "Using Legacy I/O port access");
            }
            morpheus_network::PciAccessMethod::Ecam(base) => {
                if base == 0xB000_0000 {
                    debug_log(InitStage::PciScan, "Using ECAM access (Q35)");
                } else if base == 0xE000_0000 {
                    debug_log(InitStage::PciScan, "Using ECAM access (i440FX)");
                } else {
                    debug_log(InitStage::PciScan, "Using ECAM access");
                }
            }
        }

        if devices.is_empty() {
            error_log(InitStage::PciScan, "No network devices found on PCI bus");
            return Err(NetInitError::NoNetworkDevice);
        }

        for dev in devices.iter() {
            let name = dev.driver_type.name();
            debug_log(InitStage::PciScan, name);
        }

        // Choose device based on preference
        let device = match device_config.preferred_driver {
            morpheus_network::device::factory::PreferredDriver::Any => {
                devices.iter()
                    .find(|d| d.driver_type.is_implemented())
                    .or_else(|| devices.first())
            }
            morpheus_network::device::factory::PreferredDriver::VirtIO => {
                devices.iter().find(|d| d.driver_type == morpheus_network::device::factory::DriverType::VirtIO)
            }
            morpheus_network::device::factory::PreferredDriver::Intel => {
                devices.iter().find(|d| matches!(d.driver_type, morpheus_network::device::factory::DriverType::IntelIgb | morpheus_network::device::factory::DriverType::IntelE1000))
            }
            morpheus_network::device::factory::PreferredDriver::Realtek => {
                devices.iter().find(|d| matches!(d.driver_type, morpheus_network::device::factory::DriverType::RealtekRtl8168 | morpheus_network::device::factory::DriverType::RealtekRtl8139))
            }
            morpheus_network::device::factory::PreferredDriver::Broadcom => {
                devices.iter().find(|d| d.driver_type == morpheus_network::device::factory::DriverType::BroadcomBcm57xx)
            }
        };

        let device = device.ok_or_else(|| {
            error_log(InitStage::PciScan, "No matching network device found");
            NetInitError::NoNetworkDevice
        })?;

        if !device.driver_type.is_implemented() {
            error_log(InitStage::PciScan, "Found device but driver not implemented");
            return Err(NetInitError::NoNetworkDevice);
        }

        DeviceFactory::create_from_detected_with_method(device, method)
            .map_err(|_e| {
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

