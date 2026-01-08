//! Network Device Factory
//!
//! Unified device creation API that abstracts over different NIC drivers.
//! Designed for seamless integration of future hardware drivers (Intel, Realtek, Broadcom).
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    DeviceFactory                            │
//! │  Scan → Detect → Create appropriate driver                  │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!         ┌────────────────────┼────────────────────┐
//!         ▼                    ▼                    ▼
//!    VirtIO-net           Intel i210           Realtek RTL
//!    (QEMU/KVM)           (future)             (future)
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                  UnifiedNetDevice                           │
//! │  Enum wrapper implementing NetworkDevice trait              │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::device::factory::{DeviceFactory, DeviceConfig};
//! use morpheus_network::device::hal::StaticHal;
//!
//! // Initialize HAL first
//! StaticHal::init();
//!
//! // Auto-detect and create device
//! let config = DeviceConfig::default();
//! let device = DeviceFactory::create_auto::<StaticHal>(config)?;
//!
//! // Use with smoltcp
//! let mac = device.mac_address();
//! ```

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use virtio_drivers::transport::pci::bus::{
    ConfigurationAccess as VirtioConfigAccess,
    DeviceFunction as VirtioDeviceFunction,
    PciRoot,
};
use virtio_drivers::transport::pci::PciTransport;
use virtio_drivers::Hal;

use crate::device::pci::{ConfigAccess, DeviceFunction, EcamAccess, PciDeviceInfo, PciScanner, ecam_bases};
use crate::device::virtio::VirtioNetDevice;
use crate::device::hal::StaticHal;
use crate::device::NetworkDevice;
use crate::error::{NetworkError, Result};

#[cfg(target_arch = "x86_64")]
use crate::device::pci::LegacyIoAccess;

// =============================================================================
// Bridge: Our PCI access → virtio-drivers ConfigurationAccess
// =============================================================================

/// Wrapper to implement virtio-drivers' ConfigurationAccess for our types.
/// This bridges our PCI scanner to virtio-drivers' PciTransport.
pub struct VirtioConfigBridge<A: ConfigAccess> {
    access: A,
}

impl<A: ConfigAccess> VirtioConfigBridge<A> {
    pub fn new(access: A) -> Self {
        Self { access }
    }
}

impl<A: ConfigAccess> VirtioConfigAccess for VirtioConfigBridge<A> {
    fn read_word(&self, device_function: VirtioDeviceFunction, register: u8) -> u32 {
        // Convert virtio-drivers DeviceFunction to our DeviceFunction
        let our_df = DeviceFunction::new(
            device_function.bus,
            device_function.device,
            device_function.function,
        );
        // SAFETY: We trust the caller to provide valid device/register
        unsafe { self.access.read32(our_df, register) }
    }

    fn write_word(&mut self, device_function: VirtioDeviceFunction, register: u8, data: u32) {
        let our_df = DeviceFunction::new(
            device_function.bus,
            device_function.device,
            device_function.function,
        );
        // SAFETY: We trust the caller to provide valid device/register
        unsafe { self.access.write32(our_df, register, data) }
    }

    unsafe fn unsafe_clone(&self) -> Self {
        // SAFETY: Our access types are stateless or use global state
        // This is safe for LegacyIoAccess (stateless) and EcamAccess (pointer copy)
        Self {
            access: core::ptr::read(&self.access),
        }
    }
}

// SAFETY: Our bridge is safe to send between threads (single-threaded in bootloader anyway)
unsafe impl<A: ConfigAccess + Send> Send for VirtioConfigBridge<A> {}
unsafe impl<A: ConfigAccess + Sync> Sync for VirtioConfigBridge<A> {}

// =============================================================================
// Device Configuration
// =============================================================================

/// Configuration for device creation.
#[derive(Debug, Clone)]
pub struct DeviceConfig {
    /// ECAM base address for PCIe config access.
    /// If None, uses legacy I/O ports (x86) or auto-detection.
    pub ecam_base: Option<usize>,
    
    /// Prefer specific device type if multiple found.
    pub preferred_driver: PreferredDriver,
    
    /// Bus number to scan (default: 0).
    pub scan_bus: u8,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            ecam_base: Some(ecam_bases::QEMU_Q35),
            preferred_driver: PreferredDriver::Any,
            scan_bus: 0,
        }
    }
}

impl DeviceConfig {
    /// Create config for QEMU/KVM (Q35 machine type).
    pub fn qemu() -> Self {
        Self {
            ecam_base: Some(ecam_bases::QEMU_Q35),
            ..Default::default()
        }
    }

    /// Create config for legacy I/O port access (fallback).
    #[cfg(target_arch = "x86_64")]
    pub fn legacy_io() -> Self {
        Self {
            ecam_base: None,
            ..Default::default()
        }
    }
}

/// Preferred driver type when multiple devices are found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreferredDriver {
    /// Use first available device.
    #[default]
    Any,
    /// Prefer VirtIO devices (virtual machines).
    VirtIO,
    /// Prefer Intel NICs.
    Intel,
    /// Prefer Realtek NICs.
    Realtek,
    /// Prefer Broadcom NICs.
    Broadcom,
}

// =============================================================================
// Detected Device Info
// =============================================================================

/// Information about a detected network device.
#[derive(Debug, Clone)]
pub struct DetectedDevice {
    /// PCI device info.
    pub pci_info: PciDeviceInfo,
    /// Detected driver type.
    pub driver_type: DriverType,
}

/// Type of driver needed for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverType {
    /// VirtIO network device (QEMU, KVM).
    VirtIO,
    /// Intel i210/i211/i219 family.
    IntelIgb,
    /// Intel e1000/e1000e family.
    IntelE1000,
    /// Realtek RTL8111/8168 family.
    RealtekRtl8168,
    /// Realtek RTL8139 (legacy).
    RealtekRtl8139,
    /// Broadcom BCM57xx family.
    BroadcomBcm57xx,
    /// Unknown network device.
    Unknown,
}

impl DriverType {
    /// Detect driver type from PCI device info.
    pub fn from_pci_info(info: &PciDeviceInfo) -> Self {
        // VirtIO
        if info.is_virtio_net() {
            return Self::VirtIO;
        }

        // Intel (vendor 0x8086)
        if info.vendor_id == 0x8086 && info.is_network() {
            return match info.device_id {
                // i210/i211 family
                0x1533 | 0x1536 | 0x1537 | 0x1538 | 0x1539 | 0x157B | 0x157C => Self::IntelIgb,
                // i219 family (integrated in chipsets)
                0x15B7..=0x15BE | 0x15D6..=0x15DF | 0x15E0..=0x15E3 => Self::IntelIgb,
                // e1000 family
                0x100E | 0x100F | 0x1010..=0x1019 | 0x101D | 0x101E | 0x1026..=0x102F => Self::IntelE1000,
                // e1000e family
                0x10D3 | 0x10DE | 0x10DF | 0x10E5 | 0x10EA | 0x10EB | 0x10EF | 0x10F0 => Self::IntelE1000,
                _ => Self::Unknown,
            };
        }

        // Realtek (vendor 0x10EC)
        if info.vendor_id == 0x10EC && info.is_network() {
            return match info.device_id {
                // RTL8111/8168 family
                0x8168 | 0x8161 | 0x8136 => Self::RealtekRtl8168,
                // RTL8169
                0x8169 => Self::RealtekRtl8168,
                // RTL8139 (legacy)
                0x8139 => Self::RealtekRtl8139,
                _ => Self::Unknown,
            };
        }

        // Broadcom (vendor 0x14E4)
        if info.vendor_id == 0x14E4 && info.is_network() {
            return match info.device_id {
                // BCM57xx family
                0x1600..=0x16FF | 0x1700..=0x17FF => Self::BroadcomBcm57xx,
                _ => Self::Unknown,
            };
        }

        Self::Unknown
    }

    /// Check if this driver type is currently implemented.
    pub fn is_implemented(&self) -> bool {
        matches!(self, Self::VirtIO)
    }

    /// Get human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::VirtIO => "VirtIO-net",
            Self::IntelIgb => "Intel i210/i211/i219",
            Self::IntelE1000 => "Intel e1000/e1000e",
            Self::RealtekRtl8168 => "Realtek RTL8168",
            Self::RealtekRtl8139 => "Realtek RTL8139",
            Self::BroadcomBcm57xx => "Broadcom BCM57xx",
            Self::Unknown => "Unknown",
        }
    }
}

// =============================================================================
// Unified Network Device
// =============================================================================

/// Unified network device enum that wraps different driver implementations.
/// 
/// This allows the rest of the stack to work with any NIC driver through
/// a single type, while still allowing driver-specific optimizations.
pub enum UnifiedNetDevice {
    /// VirtIO network device (QEMU, KVM, VirtualBox).
    VirtIO(VirtioNetDevice<StaticHal, PciTransport>),
    
    // Future drivers:
    // Intel(IntelNetDevice),
    // Realtek(RealtekNetDevice),
    // Broadcom(BroadcomNetDevice),
}

impl NetworkDevice for UnifiedNetDevice {
    fn mac_address(&self) -> [u8; 6] {
        match self {
            Self::VirtIO(dev) => dev.mac_address(),
        }
    }

    fn can_transmit(&self) -> bool {
        match self {
            Self::VirtIO(dev) => dev.can_transmit(),
        }
    }

    fn can_receive(&self) -> bool {
        match self {
            Self::VirtIO(dev) => dev.can_receive(),
        }
    }

    fn transmit(&mut self, packet: &[u8]) -> Result<()> {
        match self {
            Self::VirtIO(dev) => dev.transmit(packet),
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>> {
        match self {
            Self::VirtIO(dev) => dev.receive(buffer),
        }
    }
}

impl fmt::Debug for UnifiedNetDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VirtIO(_) => write!(f, "UnifiedNetDevice::VirtIO"),
        }
    }
}

// =============================================================================
// Device Factory
// =============================================================================

/// Factory for creating network devices.
/// 
/// Handles PCI scanning, device detection, and driver instantiation.
pub struct DeviceFactory;

impl DeviceFactory {
    /// Scan for network devices on the PCI bus.
    pub fn scan(config: &DeviceConfig) -> Result<Vec<DetectedDevice>> {
        let mut devices = Vec::new();

        #[cfg(target_arch = "x86_64")]
        {
            if let Some(ecam_base) = config.ecam_base {
                // Use ECAM access
                let ecam = unsafe { EcamAccess::new(ecam_base as *mut u8) };
                let scanner = PciScanner::new(ecam);
                let pci_devices = scanner.find_network();
                
                for pci_info in pci_devices {
                    let driver_type = DriverType::from_pci_info(&pci_info);
                    devices.push(DetectedDevice { pci_info, driver_type });
                }
            } else {
                // Use legacy I/O access
                let legacy = LegacyIoAccess::new();
                let scanner = PciScanner::new(legacy);
                let pci_devices = scanner.find_network();
                
                for pci_info in pci_devices {
                    let driver_type = DriverType::from_pci_info(&pci_info);
                    devices.push(DetectedDevice { pci_info, driver_type });
                }
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            if let Some(ecam_base) = config.ecam_base {
                let ecam = unsafe { EcamAccess::new(ecam_base as *mut u8) };
                let scanner = PciScanner::new(ecam);
                let pci_devices = scanner.find_network();
                
                for pci_info in pci_devices {
                    let driver_type = DriverType::from_pci_info(&pci_info);
                    devices.push(DetectedDevice { pci_info, driver_type });
                }
            }
        }

        Ok(devices)
    }

    /// Create a network device from detected device info.
    /// 
    /// Returns the unified device wrapper that implements NetworkDevice.
    pub fn create_from_detected(
        detected: &DetectedDevice,
        config: &DeviceConfig,
    ) -> Result<UnifiedNetDevice> {
        match detected.driver_type {
            DriverType::VirtIO => {
                Self::create_virtio(&detected.pci_info, config)
            }
            DriverType::IntelIgb | DriverType::IntelE1000 => {
                Err(NetworkError::Other("Intel NIC driver not yet implemented"))
            }
            DriverType::RealtekRtl8168 | DriverType::RealtekRtl8139 => {
                Err(NetworkError::Other("Realtek NIC driver not yet implemented"))
            }
            DriverType::BroadcomBcm57xx => {
                Err(NetworkError::Other("Broadcom NIC driver not yet implemented"))
            }
            DriverType::Unknown => {
                Err(NetworkError::Other("Unknown network device type"))
            }
        }
    }

    /// Auto-detect and create the best available network device.
    pub fn create_auto(config: &DeviceConfig) -> Result<UnifiedNetDevice> {
        let devices = Self::scan(config)?;
        
        if devices.is_empty() {
            return Err(NetworkError::Other("No network devices found"));
        }

        // Filter by preference
        let device = match config.preferred_driver {
            PreferredDriver::Any => {
                // Prefer implemented drivers
                devices.iter()
                    .find(|d| d.driver_type.is_implemented())
                    .or_else(|| devices.first())
            }
            PreferredDriver::VirtIO => {
                devices.iter().find(|d| d.driver_type == DriverType::VirtIO)
            }
            PreferredDriver::Intel => {
                devices.iter().find(|d| matches!(d.driver_type, DriverType::IntelIgb | DriverType::IntelE1000))
            }
            PreferredDriver::Realtek => {
                devices.iter().find(|d| matches!(d.driver_type, DriverType::RealtekRtl8168 | DriverType::RealtekRtl8139))
            }
            PreferredDriver::Broadcom => {
                devices.iter().find(|d| d.driver_type == DriverType::BroadcomBcm57xx)
            }
        };

        let device = device.ok_or(NetworkError::Other("No matching network device found"))?;
        
        if !device.driver_type.is_implemented() {
            return Err(NetworkError::Other("Found device but driver not implemented"));
        }

        Self::create_from_detected(device, config)
    }

    /// Create a VirtIO network device.
    #[cfg(target_arch = "x86_64")]
    fn create_virtio(pci_info: &PciDeviceInfo, config: &DeviceConfig) -> Result<UnifiedNetDevice> {
        // Convert our DeviceFunction to virtio-drivers' DeviceFunction
        let virt_df = VirtioDeviceFunction {
            bus: pci_info.location.bus,
            device: pci_info.location.device,
            function: pci_info.location.function,
        };

        if let Some(ecam_base) = config.ecam_base {
            // ECAM path
            let ecam = unsafe { EcamAccess::new(ecam_base as *mut u8) };
            let bridge = VirtioConfigBridge::new(ecam);
            let mut pci_root = PciRoot::new(bridge);

            // Create PCI transport
            let transport = PciTransport::new::<StaticHal, _>(&mut pci_root, virt_df)
                .map_err(|e| NetworkError::Other("Failed to create PCI transport"))?;

            // Create VirtIO device
            let device = VirtioNetDevice::new(transport)
                .map_err(|_| NetworkError::Other("Failed to create VirtIO device"))?;

            Ok(UnifiedNetDevice::VirtIO(device))
        } else {
            // Legacy I/O path
            let legacy = LegacyIoAccess::new();
            let bridge = VirtioConfigBridge::new(legacy);
            let mut pci_root = PciRoot::new(bridge);

            let transport = PciTransport::new::<StaticHal, _>(&mut pci_root, virt_df)
                .map_err(|e| NetworkError::Other("Failed to create PCI transport"))?;

            let device = VirtioNetDevice::new(transport)
                .map_err(|_| NetworkError::Other("Failed to create VirtIO device"))?;

            Ok(UnifiedNetDevice::VirtIO(device))
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn create_virtio(pci_info: &PciDeviceInfo, config: &DeviceConfig) -> Result<UnifiedNetDevice> {
        let virt_df = VirtioDeviceFunction {
            bus: pci_info.location.bus,
            device: pci_info.location.device,
            function: pci_info.location.function,
        };

        let ecam_base = config.ecam_base
            .ok_or(NetworkError::Other("ECAM base required on non-x86"))?;
        
        let ecam = unsafe { EcamAccess::new(ecam_base as *mut u8) };
        let bridge = VirtioConfigBridge::new(ecam);
        let mut pci_root = PciRoot::new(bridge);

        let transport = PciTransport::new::<StaticHal, _>(&mut pci_root, virt_df)
            .map_err(|e| NetworkError::Other("Failed to create PCI transport"))?;

        let device = VirtioNetDevice::new(transport)
            .map_err(|_| NetworkError::Other("Failed to create VirtIO device"))?;

        Ok(UnifiedNetDevice::VirtIO(device))
    }
}

// =============================================================================
// Future Driver Trait (for hardware expansion)
// =============================================================================

/// Trait for NIC drivers to implement.
/// 
/// This provides a standard interface for driver initialization and
/// allows the factory to work with any compliant driver.
pub trait NicDriver: NetworkDevice + Sized {
    /// Error type for driver initialization.
    type Error: fmt::Debug;
    
    /// PCI vendor ID(s) this driver supports.
    fn supported_vendors() -> &'static [u16];
    
    /// PCI device ID(s) this driver supports.
    fn supported_devices() -> &'static [u16];
    
    /// Check if this driver supports the given PCI device.
    fn supports_device(info: &PciDeviceInfo) -> bool {
        Self::supported_vendors().contains(&info.vendor_id)
            && Self::supported_devices().contains(&info.device_id)
    }
    
    /// Create driver instance from PCI device.
    /// 
    /// # Safety
    /// 
    /// Caller must ensure:
    /// - HAL is initialized
    /// - Device is a valid network device
    /// - No other driver is using this device
    unsafe fn create_from_pci<A: ConfigAccess>(
        info: &PciDeviceInfo,
        access: A,
    ) -> core::result::Result<Self, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_type_detection() {
        // VirtIO
        let virtio = PciDeviceInfo {
            location: DeviceFunction::new(0, 3, 0),
            vendor_id: 0x1AF4,
            device_id: 0x1041,
            class: 0x02,
            subclass: 0x00,
            prog_if: 0,
            revision: 0,
            header_type: 0,
            multifunction: false,
        };
        assert_eq!(DriverType::from_pci_info(&virtio), DriverType::VirtIO);

        // Intel i210
        let intel = PciDeviceInfo {
            location: DeviceFunction::new(0, 4, 0),
            vendor_id: 0x8086,
            device_id: 0x1533,
            class: 0x02,
            subclass: 0x00,
            prog_if: 0,
            revision: 0,
            header_type: 0,
            multifunction: false,
        };
        assert_eq!(DriverType::from_pci_info(&intel), DriverType::IntelIgb);

        // Realtek 8168
        let realtek = PciDeviceInfo {
            location: DeviceFunction::new(0, 5, 0),
            vendor_id: 0x10EC,
            device_id: 0x8168,
            class: 0x02,
            subclass: 0x00,
            prog_if: 0,
            revision: 0,
            header_type: 0,
            multifunction: false,
        };
        assert_eq!(DriverType::from_pci_info(&realtek), DriverType::RealtekRtl8168);
    }

    #[test]
    fn test_driver_type_names() {
        assert_eq!(DriverType::VirtIO.name(), "VirtIO-net");
        assert_eq!(DriverType::IntelIgb.name(), "Intel i210/i211/i219");
        assert!(DriverType::VirtIO.is_implemented());
        assert!(!DriverType::IntelIgb.is_implemented());
    }
}
