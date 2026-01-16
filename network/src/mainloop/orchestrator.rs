//! Network Download Orchestrator
//!
//! # Entry Point Contract
//!
//! **SOLE ENTRY**: `download_with_config()` or convenience wrapper `download()`
//!
//! **PRECONDITIONS** (caller must ensure):
//! 1. ExitBootServices has been called (no UEFI runtime)
//! 2. hwinit has normalized platform (bus mastering, DMA policy, cache coherency)
//! 3. Driver has been instantiated and brutally reset to pristine state
//!
//! **WHAT THIS MODULE RECEIVES**:
//! - `&mut D` - Already-reset driver implementing `NetworkDriver`
//! - `DownloadConfig` - URL + optional disk write parameters
//! - `Option<UnifiedBlockDevice>` - Block device if writing
//! - `tsc_freq` - TSC frequency in Hz
//!
//! **WHAT THIS MODULE DOES NOT DO**:
//! - PCI enumeration (hwinit's job)
//! - Bus mastering setup (hwinit's job)
//! - DMA policy (hwinit's job)
//! - Driver instantiation (caller's job)
//! - Device reset (driver's job, done before entry)
//!
//! # State Machine Flow
//! ```text
//! Init → GptPrep → LinkWait → DHCP → DNS → Connect → HTTP → Manifest → Done
//! ```

use smoltcp::iface::{Config as IfaceConfig, Interface, SocketSet, SocketStorage};
use smoltcp::socket::dhcpv4::Socket as Dhcpv4Socket;
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress};

use crate::device::UnifiedBlockDevice;
use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::{Context, DownloadConfig};
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};
use crate::mainloop::states::InitState;

extern crate alloc;
use alloc::boxed::Box;

/// Result of a download operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadResult {
    /// Download completed successfully.
    Success { bytes_downloaded: u64, bytes_written: u64 },
    /// Download failed.
    Failed { reason: &'static str },
}

/// Execute HTTP download using state machine.
///
/// # Arguments
/// * `driver` - Already-initialized network driver
/// * `url` - Download URL
/// * `tsc_freq` - TSC frequency (Hz)
pub fn download<D: NetworkDriver>(
    driver: &mut D,
    url: &'static str,
    tsc_freq: u64,
) -> DownloadResult {
    let config = DownloadConfig::download_only(url);
    download_with_config(driver, config, None, tsc_freq)
}

/// Execute HTTP download with disk writing.
///
/// # Arguments
/// * `driver` - Already-initialized network driver
/// * `config` - Full download configuration
/// * `blk_device` - Optional block device for disk writes
/// * `tsc_freq` - TSC frequency (Hz)
pub fn download_with_config<D: NetworkDriver>(
    driver: &mut D,
    config: DownloadConfig<'static>,
    blk_device: Option<UnifiedBlockDevice>,
    tsc_freq: u64,
) -> DownloadResult {
    serial::println("=================================");
    serial::println("  MorpheusX Network Download     ");
    serial::println("=================================");
    serial::print("URL: ");
    serial::println(config.url);

    let mac = driver.mac_address();
    let eth_addr = EthernetAddress(mac);

    serial::print("MAC: ");
    serial::print_mac(&mac);
    serial::println("");

    if config.write_to_disk && blk_device.is_some() {
        serial::print("Disk write: enabled (sector ");
        serial::print_u32(config.target_start_sector as u32);
        serial::println(")");
    } else {
        serial::println("Disk write: disabled");
    }

    let mut adapter = SmoltcpAdapter::new(driver);

    let iface_config = IfaceConfig::new(HardwareAddress::Ethernet(eth_addr));
    let mut iface = Interface::new(iface_config, &mut adapter, Instant::ZERO);

    // Static socket storage
    let mut socket_storage: [SocketStorage; 4] = Default::default();
    let mut sockets = SocketSet::new(&mut socket_storage[..]);

    // DHCP socket
    let dhcp_socket = Dhcpv4Socket::new();
    let dhcp_handle = sockets.add(dhcp_socket);

    // TCP socket
    static mut TCP_RX_BUF: [u8; 65536] = [0u8; 65536];
    static mut TCP_TX_BUF: [u8; 65536] = [0u8; 65536];
    let tcp_socket = unsafe {
        TcpSocket::new(
            TcpSocketBuffer::new(&mut TCP_RX_BUF[..]),
            TcpSocketBuffer::new(&mut TCP_TX_BUF[..]),
        )
    };
    let tcp_handle = sockets.add(tcp_socket);

    // Context
    let mut ctx = Context::new(config, tsc_freq);
    ctx.dhcp_handle = Some(dhcp_handle);
    ctx.tcp_handle = Some(tcp_handle);
    ctx.blk_device = blk_device;

    let mut current_state: Box<dyn State<D>> = Box::new(InitState::new());

    serial::println("---------------------------------");
    serial::print("State: ");
    serial::println(current_state.name());

    loop {
        let tsc = read_tsc();
        let millis = if tsc_freq > 0 {
            (tsc / (tsc_freq / 1000)) as i64
        } else {
            0
        };
        let now = Instant::from_millis(millis);

        let _ = iface.poll(now, &mut adapter, &mut sockets);

        let (next_state, result) = current_state.step(
            &mut ctx,
            &mut iface,
            &mut sockets,
            &mut adapter,
            now,
            tsc,
        );
        current_state = next_state;

        match result {
            StepResult::Continue => {}
            StepResult::Transition => {
                serial::print("State: ");
                serial::println(current_state.name());
            }
            StepResult::Done => {
                serial::println("---------------------------------");
                serial::print("Downloaded: ");
                serial::print_u32((ctx.bytes_downloaded / 1024 / 1024) as u32);
                serial::println(" MB");
                if ctx.bytes_written > 0 {
                    serial::print("Written: ");
                    serial::print_u32((ctx.bytes_written / 1024 / 1024) as u32);
                    serial::println(" MB");
                }
                return DownloadResult::Success {
                    bytes_downloaded: ctx.bytes_downloaded,
                    bytes_written: ctx.bytes_written,
                };
            }
            StepResult::Failed(reason) => {
                serial::println("---------------------------------");
                serial::print("FAILED: ");
                serial::println(reason);
                return DownloadResult::Failed { reason };
            }
        }
    }
}

#[inline]
fn read_tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem));
        ((hi as u64) << 32) | (lo as u64)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}
