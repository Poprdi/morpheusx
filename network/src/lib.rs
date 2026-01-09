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

// ═══════════════════════════════════════════════════════════════
// EXISTING MODULES (virtio-drivers based implementation)
// ═══════════════════════════════════════════════════════════════
pub mod error;
pub mod http;
pub mod url;
pub mod transfer;
pub mod client;
pub mod device;
pub mod stack;

// ═══════════════════════════════════════════════════════════════
// NEW BARE-METAL ASM-BASED MODULES
// These provide post-ExitBootServices network support
// ═══════════════════════════════════════════════════════════════
pub mod asm;          // ASM bindings (TSC, MMIO, PIO, barriers)
pub mod types;        // Shared types (#[repr(C)] structs)
pub mod dma;          // DMA buffer management with ownership tracking
pub mod driver;       // Driver abstraction and implementations
pub mod mainloop;     // 5-phase poll loop

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

// ===================== Serial Debug Output =====================
// Write directly to COM1 (0x3f8) for QEMU -serial stdio debugging
// This works even when the main display is blocked

/// Write a single byte to COM1 serial port (non-blocking with bounded wait)
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn serial_byte(b: u8) {
    unsafe {
        // Bounded wait for TX buffer empty - max ~100 iterations
        // If serial port doesn't exist, we just skip the write
        let mut retries = 0u32;
        loop {
            let status: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") 0x3fdu16,  // COM1 + 5 = line status register
                out("al") status,
                options(nostack, preserves_flags)
            );
            if status & 0x20 != 0 {
                // TX buffer empty, safe to write
                core::arch::asm!(
                    "out dx, al",
                    in("dx") 0x3f8u16,  // COM1 data register
                    in("al") b,
                    options(nostack, preserves_flags)
                );
                return;
            }
            retries += 1;
            if retries > 100 {
                // Serial port not responding - abandon write
                return;
            }
            core::hint::spin_loop();
        }
    }
}

/// Write a string to COM1 serial port
#[cfg(target_arch = "x86_64")]
pub fn serial_str(s: &str) {
    for b in s.bytes() {
        serial_byte(b);
    }
}

/// Write a u32 as decimal to serial
#[cfg(target_arch = "x86_64")]
pub fn serial_u32(n: u32) {
    if n == 0 {
        serial_byte(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut val = n;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        serial_byte(buf[i]);
    }
}

/// Log debug stage to serial - format: "[NET:XX] message\n"
#[cfg(target_arch = "x86_64")]
pub fn serial_stage(stage: u32, msg: &str) {
    serial_str("[NET:");
    serial_u32(stage);
    serial_str("] ");
    serial_str(msg);
    serial_byte(b'\n');
}


