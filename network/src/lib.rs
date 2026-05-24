//! Bare-metal HTTP client for post-ExitBootServices execution.
//!
//! State machine: Init -> GptPrep -> LinkWait -> DHCP -> DNS -> Connect -> HTTP -> Manifest -> Done.
//! Preconditions: EBS done, hwinit has normalized DMA/bus mastering, driver instantiated.
//! All drivers must perform a full reset on init (see `driver/RESET_CONTRACT.md`).

#![no_std]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(static_mut_refs)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::fn_to_numeric_cast)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::new_without_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::large_enum_variant)]
// Poll-based state machines return early from loops.
#![allow(clippy::never_loop)]

extern crate alloc;

/// Trivial `impl From<Src> for Dst { fn from(e) -> Self { Dst::Variant(e) } }`.
/// Use the `(_)` form for variants that drop the payload.
#[macro_export]
macro_rules! impl_from {
    ($src:ty => $dst:ty : $variant:ident) => {
        impl From<$src> for $dst {
            fn from(e: $src) -> Self {
                <$dst>::$variant(e)
            }
        }
    };
    ($src:ty => $dst:ty : $variant:ident(_)) => {
        impl From<$src> for $dst {
            fn from(_: $src) -> Self {
                <$dst>::$variant
            }
        }
    };
}

pub mod alloc_heap;
pub mod display;

pub mod client;
pub mod device;
pub mod error;
pub mod http;
pub mod stack;
pub mod transfer;
pub mod url;

pub mod asm;
pub mod boot;
pub mod dma;
pub mod driver;
pub mod entry;
pub mod mainloop;
pub mod pci;
pub mod state;
pub mod time;
pub mod types;

pub use device::NetworkDevice;
pub use device::UnifiedNetDevice;
pub use error::{NetworkError, Result};
pub use types::{HttpMethod, ProgressCallback};

pub use entry::{run_download, RunConfig, RunResult};

pub use device::UnifiedBlockDevice;
pub use driver::block_traits::{BlockCompletion, BlockDeviceInfo, BlockDriver, BlockError};

pub use driver::ahci::{AhciConfig, AhciDriver, AhciInitError};
pub use driver::sdhci::{SdhciConfig, SdhciDriver, SdhciInitError};
pub use driver::usb_msd::{UsbMsdConfig, UsbMsdDriver, UsbMsdInitError};
pub use driver::virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};

pub use driver::block_io_adapter::{BlockIoError, VirtioBlkBlockIo};
pub use driver::unified_block_io::{GenericBlockIo, UnifiedBlockIo, UnifiedBlockIoError};

pub use gpt_disk_io::BlockIo as GptBlockIo;
pub use gpt_disk_types::{BlockSize as GptBlockSize, Lba as GptLba};

pub use boot::block_probe::{
    create_unified_from_detected, create_unified_from_detected_ahci_port, detect_block_device_type,
    probe_and_create_block_driver, probe_unified_block_device, scan_all_block_devices,
    BlockDeviceType, BlockDmaConfig, BlockProbeError, BlockProbeResult, DetectedBlockDevice,
};

pub use client::HttpClient;
pub use client::NativeHttpClient;

pub use stack::{debug_log, debug_log_available, debug_log_clear, debug_log_pop, DebugLogEntry};
pub use stack::{debug_stage, set_debug_stage};
pub use stack::{ecam_bases, DeviceAdapter, NetConfig, NetInterface, NetState};

pub use driver::{NetworkDriver as AsmNetworkDriver, VirtioConfig, VirtioNetDriver};

#[cfg(target_arch = "x86_64")]
pub use device::pci::{pci_io_test, read_tsc, tsc_delay_us};

// COM1 (0x3f8) serial for QEMU -serial stdio debugging.

/// Write one byte to COM1, abandoning after ~100 spins if TX never empties.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn serial_byte(b: u8) {
    unsafe {
        let mut retries = 0u32;
        loop {
            let status: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") 0x3fdu16,
                out("al") status,
                options(nostack, preserves_flags)
            );
            if status & 0x20 != 0 {
                core::arch::asm!(
                    "out dx, al",
                    in("dx") 0x3f8u16,
                    in("al") b,
                    options(nostack, preserves_flags)
                );
                return;
            }
            retries += 1;
            if retries > 100 {
                return;
            }
            core::hint::spin_loop();
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub fn serial_str(s: &str) {
    for b in s.bytes() {
        serial_byte(b);
    }
    display::display_write(s);
}

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

/// Emit `[NET:<stage>] <msg>` on COM1.
#[cfg(target_arch = "x86_64")]
pub fn serial_stage(stage: u32, msg: &str) {
    serial_str("[NET:");
    serial_u32(stage);
    serial_str("] ");
    serial_str(msg);
    serial_byte(b'\n');
}
