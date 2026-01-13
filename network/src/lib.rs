//! MorpheusX Network Stack
//!
//! ASM-first bare-metal HTTP client for bootloader environments.
//! Designed for post-ExitBootServices execution with no firmware dependencies.
//!
//! # Architecture (ASM-First)
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
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              VirtioNetDriver (ASM-backed)                   │
//! │  Uses hand-written assembly for all hardware access         │
//! │  - MMIO with explicit barriers                              │
//! │  - Virtqueue descriptor ring management                     │
//! │  - Fire-and-forget TX, poll-based RX                        │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              ASM Layer (network/asm/)                       │
//! │  Core: TSC, barriers, MMIO, PIO, cache                      │
//! │  VirtIO: init, queue, TX, RX, notify                        │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage (Post-ExitBootServices)
//!
//! ```ignore
//! use morpheus_network::{VirtioNetDriver, VirtioConfig, NetworkDriver};
//! use morpheus_network::boot::BootHandoff;
//! use morpheus_network::mainloop::bare_metal_main;
//!
//! // After ExitBootServices, with BootHandoff from pre-EBS setup:
//! let result = unsafe { bare_metal_main(handoff, config) };
//! ```

#![no_std]
#![allow(dead_code)]
#![allow(unused_imports)]
// Allow never_loop - our poll-based state machines intentionally return
// from loops early (single-threaded cooperative polling pattern)
#![allow(clippy::never_loop)]

extern crate alloc;

// ═══════════════════════════════════════════════════════════════
// POST-EBS ALLOCATOR (must be first - provides global allocator)
// ═══════════════════════════════════════════════════════════════
pub mod alloc_heap;

// ═══════════════════════════════════════════════════════════════
// CORE MODULES
// ═══════════════════════════════════════════════════════════════
pub mod client;
pub mod device;
pub mod error;
pub mod http;
pub mod stack;
pub mod transfer;
pub mod url;

// ═══════════════════════════════════════════════════════════════
// ASM-FIRST BARE-METAL MODULES
// These provide post-ExitBootServices network support using
// hand-written assembly for all hardware access.
// ═══════════════════════════════════════════════════════════════
pub mod asm; // ASM bindings (TSC, MMIO, PIO, barriers)
pub mod boot; // Boot handoff and initialization
pub mod dma; // DMA buffer management with ownership tracking
pub mod driver; // Driver abstraction and implementations
pub mod mainloop; // 5-phase poll loop
pub mod pci;
pub mod state; // State machines (DHCP, TCP, HTTP, etc.)
pub mod time; // Timing utilities
pub mod types; // Shared types (#[repr(C)] structs) // PCI bus access

// ═══════════════════════════════════════════════════════════════
// RE-EXPORTS
// ═══════════════════════════════════════════════════════════════

// Core types - Network
pub use device::NetworkDevice;
pub use device::UnifiedNetDevice;
pub use error::{NetworkError, Result};
pub use types::{HttpMethod, ProgressCallback};

// Core types - Block (Unified Block Device)
pub use device::UnifiedBlockDevice;
pub use driver::block_traits::{BlockCompletion, BlockDeviceInfo, BlockDriver, BlockError};

// Block drivers
pub use driver::ahci::{AhciConfig, AhciDriver, AhciInitError};
pub use driver::virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};

// BlockIo adapters (for filesystem compatibility)
pub use driver::unified_block_io::{GenericBlockIo, UnifiedBlockIo, UnifiedBlockIoError};
pub use driver::block_io_adapter::{BlockIoError, VirtioBlkBlockIo};

// Block probe
pub use boot::block_probe::{
    BlockDmaConfig, BlockDeviceType, BlockProbeError, BlockProbeResult,
    probe_and_create_block_driver, probe_unified_block_device, detect_block_device_type,
};

// Client
pub use client::HttpClient;
pub use client::NativeHttpClient;

// Stack (smoltcp integration)
pub use stack::{debug_log, debug_log_available, debug_log_clear, debug_log_pop, DebugLogEntry};
pub use stack::{debug_stage, set_debug_stage}; // Debug stage tracking
pub use stack::{ecam_bases, DeviceAdapter, NetConfig, NetInterface, NetState}; // Ring buffer logging

// ASM-backed VirtIO driver (primary driver for post-EBS execution)
pub use driver::{NetworkDriver as AsmNetworkDriver, VirtioConfig, VirtioNetDriver};

// Standalone assembly functions
#[cfg(target_arch = "x86_64")]
pub use device::pci::{pci_io_test, read_tsc, tsc_delay_us};

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

/// Log debug stage to serial - format: `[NET:XX] message`
#[cfg(target_arch = "x86_64")]
pub fn serial_stage(stage: u32, msg: &str) {
    serial_str("[NET:");
    serial_u32(stage);
    serial_str("] ");
    serial_str(msg);
    serial_byte(b'\n');
}
