//! Network stack setup helpers.
//!
//! High-level convenience functions for initializing the native network stack.
//! This module is completely firmware-agnostic - no UEFI dependencies.
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::stack::setup::{NetworkStack, init_virtio_network};
//! use morpheus_network::device::hal::StaticHal;
//!
//! // Initialize HAL (once at boot)
//! StaticHal::init();
//!
//! // Initialize network stack
//! let mut stack = init_virtio_network(ecam_base, get_time_ms)?;
//!
//! // Now download files
//! let response = stack.client().get("http://example.com/file.iso")?;
//! ```

extern crate alloc;

use alloc::format;

use crate::device::hal::StaticHal;
use crate::device::pci::{EcamAccess, PciScanner};
use crate::device::virtio::VirtioNetDevice;
use crate::client::native::NativeHttpClient;
use crate::stack::interface::NetConfig;
use crate::error::{NetworkError, Result};

use virtio_drivers::transport::pci::PciTransport;
use virtio_drivers::transport::pci::bus::{ConfigurationAccess, DeviceFunction, PciRoot};

/// Network stack ready to use for HTTP requests.
///
/// Contains all the components needed for native bare-metal networking.
pub struct NetworkStack {
    /// The HTTP client (owns interface which owns device)
    client: NativeHttpClient<VirtioNetDevice<StaticHal, PciTransport>>,
}

impl NetworkStack {
    /// Get a mutable reference to the HTTP client.
    pub fn client(&mut self) -> &mut NativeHttpClient<VirtioNetDevice<StaticHal, PciTransport>> {
        &mut self.client
    }
}

/// ECAM Configuration Access implementation.
///
/// Provides access to PCIe configuration space via memory-mapped ECAM.
pub struct EcamConfigAccess {
    base: usize,
}

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

/// Initialize the full VirtIO network stack.
///
/// This is firmware-agnostic - works in UEFI, bare metal, or any other environment.
///
/// This performs the full setup:
/// 1. Scan PCI for VirtIO network device
/// 2. Create PciTransport and VirtioNetDevice
/// 3. Create NativeHttpClient with DHCP
///
/// # Prerequisites
///
/// You must initialize the HAL before calling this function:
/// ```ignore
/// StaticHal::init();  // or init_discover(), or init_external()
/// ```
///
/// # Safety
///
/// - The ECAM base address must point to valid PCIe configuration space.
/// - The HAL must be initialized before calling this function.
///
/// # Arguments
///
/// * `ecam_base` - PCIe ECAM base address (use `ecam_bases::QEMU_Q35` for QEMU)
/// * `get_time_ms` - Function that returns current time in milliseconds
///
/// # Returns
///
/// A fully initialized `NetworkStack` ready for HTTP requests.
pub unsafe fn init_virtio_network(
    ecam_base: usize,
    get_time_ms: fn() -> u64,
) -> Result<NetworkStack> {
    // Step 1: Scan PCI for VirtIO network device using our scanner
    let ecam = EcamAccess::new(ecam_base as *mut u8);
    let scanner = PciScanner::new(ecam);
    
    let virtio_devices = scanner.find_virtio_net();
    if virtio_devices.is_empty() {
        return Err(NetworkError::DeviceError(
            format!("No VirtIO network device found on PCI bus")
        ));
    }

    let device_info = virtio_devices[0];
    
    // Step 2: Create PCI transport using virtio-drivers types
    let device_fn = DeviceFunction {
        bus: device_info.location.bus,
        device: device_info.location.device,
        function: device_info.location.function,
    };

    // Create ConfigurationAccess for virtio-drivers
    let cam = EcamConfigAccess::new(ecam_base);
    let mut pci_root = PciRoot::new(cam);
    
    let transport = PciTransport::new::<StaticHal, EcamConfigAccess>(&mut pci_root, device_fn)
        .map_err(|e| NetworkError::DeviceError(format!("PCI transport failed: {:?}", e)))?;

    // Step 3: Create VirtIO network device
    let virtio_device = VirtioNetDevice::<StaticHal, PciTransport>::new(transport)?;

    // Step 4: Create HTTP client with DHCP
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
pub unsafe fn init_qemu_network(get_time_ms: fn() -> u64) -> Result<NetworkStack> {
    init_virtio_network(crate::device::pci::ecam_bases::QEMU_Q35, get_time_ms)
}

