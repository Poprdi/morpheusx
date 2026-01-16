//! Orchestrator — clean entry point for state machine download.
//!
//! Self-contained. No handoff. Receives already-initialized driver.
//! Flow: EBS → hwinit → network init → download

use smoltcp::iface::{Config as IfaceConfig, Interface, SocketSet, SocketStorage};
use smoltcp::socket::dhcpv4::Socket as Dhcpv4Socket;
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress};

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};
use crate::mainloop::states::InitState;

extern crate alloc;
use alloc::boxed::Box;

/// Result of a download operation.
#[derive(Debug)]
pub enum DownloadResult {
    Success { bytes_downloaded: u64 },
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
    serial::println("=================================");
    serial::println("  MorpheusX Network Download     ");
    serial::println("=================================");
    serial::print("URL: ");
    serial::println(url);

    let mac = driver.mac_address();
    let eth_addr = EthernetAddress(mac);

    serial::print("MAC: ");
    serial::print_mac(&mac);
    serial::println("");

    let mut adapter = SmoltcpAdapter::new(driver);

    let iface_config = IfaceConfig::new(HardwareAddress::Ethernet(eth_addr));
    let mut iface = Interface::new(iface_config, &mut adapter, Instant::ZERO);

    // Static socket storage
    let mut socket_storage: [SocketStorage; 4] = Default::default();
    let mut sockets = SocketSet::new(&mut socket_storage[..]);

    // DHCP socket (smoltcp 0.11 API)
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
    let mut ctx = Context::new(url, tsc_freq);
    ctx.dhcp_handle = Some(dhcp_handle);
    ctx.tcp_handle = Some(tcp_handle);

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
                return DownloadResult::Success {
                    bytes_downloaded: ctx.bytes_downloaded,
                };
            }
            StepResult::Failed(reason) => {
                serial::println("---------------------------------");
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
