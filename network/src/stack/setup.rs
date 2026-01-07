//! Network stack setup helpers.
//!
//! High-level convenience functions for initializing the native network stack
//! in UEFI or bare metal environments.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::stack::setup::init_virtio_network;
//!
//! // In bootloader (before ExitBootServices)
//! let client = init_virtio_network(boot_services_ptr, ecam_base, get_time_ms)?;
//!
//! // Now download files
//! let response = client.get("http://example.com/file.iso")?;
//! ```

extern crate alloc;

use alloc::format;

#[cfg(feature = "uefi")]
use crate::device::hal::uefi::EfiBootServices;
#[cfg(feature = "uefi")]
use crate::device::hal::UefiHal;

use crate::device::pci::{EcamAccess, PciScanner, ConfigAccess};
use crate::device::virtio::VirtioNetDevice;
use crate::device::NetworkDevice;
use crate::client::native::NativeHttpClient;
use crate::stack::interface::NetConfig;
use crate::error::{NetworkError, Result};

use virtio_drivers::transport::pci::PciTransport;
use virtio_drivers::transport::pci::bus::{ConfigurationAccess, DeviceFunction, PciRoot};

/// Network stack ready to use for HTTP requests.
///
/// Contains all the components needed for native bare-metal networking.
#[cfg(feature = "uefi")]
pub struct NetworkStack {
    /// The HTTP client (owns interface which owns device)
    client: NativeHttpClient<VirtioNetDevice<UefiHal, PciTransport>>,
}

#[cfg(feature = "uefi")]
impl NetworkStack {
    /// Get a mutable reference to the HTTP client.
    pub fn client(&mut self) -> &mut NativeHttpClient<VirtioNetDevice<UefiHal, PciTransport>> {
        &mut self.client
    }
}

/// ECAM Configuration Access implementation.
///
/// Provides access to PCIe configuration space via memory-mapped ECAM.
#[cfg(feature = "uefi")]
pub struct EcamConfigAccess {
    base: usize,
}

#[cfg(feature = "uefi")]
impl EcamConfigAccess {
    /// Create a new ECAM configuration access.
    ///
    /// # Safety
    ///
    /// The base address must point to valid ECAM space.
    pub unsafe fn new(base: usize) -> Self {
        Self { base }
    }

    fn offset(&self, device_function: DeviceFunction, register_offset: u8) -> usize {
        self.base
            + ((device_function.bus as usize) << 20)
            + ((device_function.device as usize) << 15)
            + ((device_function.function as usize) << 12)
            + (register_offset as usize)
    }
}

#[cfg(feature = "uefi")]
impl ConfigurationAccess for EcamConfigAccess {
    fn read_word(&self, device_function: DeviceFunction, register_offset: u8) -> u32 {
        let ptr = self.offset(device_function, register_offset) as *const u32;
        // SAFETY: Caller guarantees base is valid ECAM space
        unsafe { core::ptr::read_volatile(ptr) }
    }

    fn write_word(&mut self, device_function: DeviceFunction, register_offset: u8, data: u32) {
        let ptr = self.offset(device_function, register_offset) as *mut u32;
        // SAFETY: Caller guarantees base is valid ECAM space
        unsafe { core::ptr::write_volatile(ptr, data) }
    }

    unsafe fn unsafe_clone(&self) -> Self {
        Self { base: self.base }
    }
}

/// Initialize the full VirtIO network stack in UEFI environment.
///
/// This performs the full setup:
/// 1. Initialize UefiHal with boot services
/// 2. Scan PCI for VirtIO network device
/// 3. Create PciTransport and VirtioNetDevice
/// 4. Create NativeHttpClient with DHCP
///
/// # Safety
///
/// - `boot_services` must be a valid pointer to UEFI Boot Services.
/// - Must be called before `ExitBootServices()`.
/// - The ECAM base address must match your platform.
///
/// # Arguments
///
/// * `boot_services` - Pointer to UEFI Boot Services
/// * `ecam_base` - PCIe ECAM base address (use `ecam_bases::QEMU_Q35` for QEMU)
/// * `get_time_ms` - Function that returns current time in milliseconds
///
/// # Returns
///
/// A fully initialized `NetworkStack` ready for HTTP requests.
#[cfg(feature = "uefi")]
pub unsafe fn init_virtio_network(
    boot_services: *mut EfiBootServices,
    ecam_base: usize,
    get_time_ms: fn() -> u64,
) -> Result<NetworkStack> {
    // Step 1: Initialize UEFI HAL
    UefiHal::init(boot_services);

    // Step 2: Scan PCI for VirtIO network device using our scanner
    let ecam = EcamAccess::new(ecam_base as *mut u8);
    let scanner = PciScanner::new(ecam);
    
    let virtio_devices = scanner.find_virtio_net();
    if virtio_devices.is_empty() {
        return Err(NetworkError::DeviceError(
            format!("No VirtIO network device found on PCI bus")
        ));
    }

    let device_info = virtio_devices[0];
    
    // Step 3: Create PCI transport using virtio-drivers types
    let device_fn = DeviceFunction {
        bus: device_info.location.bus,
        device: device_info.location.device,
        function: device_info.location.function,
    };

    // Create ConfigurationAccess for virtio-drivers
    let cam = EcamConfigAccess::new(ecam_base);
    let mut pci_root = PciRoot::new(cam);
    
    let transport = PciTransport::new::<UefiHal, EcamConfigAccess>(&mut pci_root, device_fn)
        .map_err(|e| NetworkError::DeviceError(format!("PCI transport failed: {:?}", e)))?;

    // Step 4: Create VirtIO network device
    let virtio_device = VirtioNetDevice::<UefiHal, PciTransport>::new(transport)?;

    // Step 5: Create HTTP client with DHCP
    let client = NativeHttpClient::new(virtio_device, NetConfig::Dhcp, get_time_ms);

    Ok(NetworkStack { client })
}

/// Quick initialization for QEMU Q35 machine type.
///
/// Uses the standard QEMU Q35 ECAM base address (0xB000_0000).
///
/// # Safety
///
/// Same requirements as `init_virtio_network`.
#[cfg(feature = "uefi")]
pub unsafe fn init_qemu_network(
    boot_services: *mut EfiBootServices,
    get_time_ms: fn() -> u64,
) -> Result<NetworkStack> {
    init_virtio_network(boot_services, crate::device::pci::ecam_bases::QEMU_Q35, get_time_ms)
}
