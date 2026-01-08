//! MorpheusX Network Stack
//!
//! Firmware-agnostic bare-metal HTTP client for bootloader environments.
//! Uses code caves in our PE binary for DMA memory - no firmware calls needed.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    HTTP Client                              │
//! │            NativeHttpClient (bare metal TCP/IP)             │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              NetInterface (smoltcp TCP/IP stack)            │
//! │  DHCP | TCP sockets | IP routing | ARP                      │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              NetworkDevice trait                            │
//! │  Abstraction over hardware: transmit, receive, mac_address │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!         ┌────────────────────┼────────────────────┐
//!         ▼                    ▼                    ▼
//!    VirtIO-net           Intel i210           Realtek RTL
//!    (QEMU/KVM)           (future)             (future)
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              StaticHal (dma-pool crate)                     │
//! │  DMA memory from code caves in PE section padding           │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use dma_pool::{DmaPool, MemoryDiscovery, PaddingPattern};
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::device::hal::StaticHal;
//! use morpheus_network::client::NativeHttpClient;
//! use morpheus_network::stack::NetConfig;
//!
//! // Initialize DMA pool from caves in our PE image
//! unsafe { DmaPool::init_from_caves(image_base, image_end) };
//!
//! // Initialize HAL
//! StaticHal::init();
//!
//! // Create network device (VirtIO for QEMU)
//! let device = VirtioNetDevice::<StaticHal, _>::new(transport)?;
//!
//! // Create HTTP client with DHCP
//! let mut client = NativeHttpClient::new(device, NetConfig::Dhcp, get_time_ms);
//!
//! // Wait for network
//! client.wait_for_network(30_000)?;
//!
//! // Download ISO
//! client.get_streaming("http://mirror.example.com/tails.iso", |chunk| {
//!     write_to_disk(chunk)?;
//!     Ok(())
//! })?;
//! ```

#![no_std]
#![allow(dead_code)]
#![allow(unused_imports)]

extern crate alloc;

pub mod error;
pub mod types;
pub mod http;
pub mod url;
pub mod transfer;
pub mod client;
pub mod device;
pub mod stack;

pub use error::{NetworkError, Result};
pub use types::{HttpMethod, ProgressCallback};
pub use client::HttpClient;
pub use client::NativeHttpClient;
pub use device::NetworkDevice;
pub use stack::{DeviceAdapter, NetInterface, NetConfig, NetState, ecam_bases};
pub use stack::{set_debug_stage, debug_stage};  // Debug stage tracking
pub use stack::{debug_log, debug_log_pop, debug_log_available, debug_log_clear, DebugLogEntry};  // Ring buffer logging
pub use device::hal::StaticHal;

// Re-export device factory types
pub use device::factory::{DeviceFactory, DeviceConfig, UnifiedNetDevice, DetectedDevice, DriverType, PciAccessMethod};

// Re-export standalone assembly functions
#[cfg(target_arch = "x86_64")]
pub use device::pci::{read_tsc, pci_io_test, tsc_delay_us};


