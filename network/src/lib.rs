//! MorpheusX Network Stack
//!
//! Bare-metal HTTP client for UEFI bootloader environment.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    HTTP Clients                             │
//! │  NativeHttpClient (bare metal) | UefiHttpClient (firmware) │
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
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use morpheus_network::client::NativeHttpClient;
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::stack::NetConfig;
//!
//! // Create network device (VirtIO for QEMU)
//! let device = VirtioNetDevice::new(transport)?;
//!
//! // Create HTTP client with DHCP
//! let mut client = NativeHttpClient::new(device, NetConfig::dhcp(), get_time_ms);
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
pub mod protocol;
pub mod http;
pub mod url;
pub mod transfer;
pub mod client;
pub mod utils;
pub mod device;
pub mod stack;

pub use error::{NetworkError, Result};
pub use types::{HttpMethod, ProgressCallback};
pub use client::HttpClient;
pub use client::NativeHttpClient;
pub use device::NetworkDevice;
pub use stack::{DeviceAdapter, NetInterface, NetConfig, NetState};

#[cfg(feature = "uefi")]
pub use stack::{UefiNetworkStack, init_virtio_network, init_qemu_network, ecam_bases};

#[cfg(target_os = "uefi")]
pub use client::uefi::UefiHttpClient;

