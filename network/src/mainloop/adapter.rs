//! smoltcp Device adapter for NetworkDriver trait.
//!
//! Bridges our NetworkDriver abstraction to smoltcp's Device trait.
//! Uses fixed-size stack buffers — no heap allocation in packet path.

use smoltcp::phy::{Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;

use crate::driver::traits::NetworkDriver;
use super::serial;

/// Adapter bridging NetworkDriver to smoltcp Device trait.
pub struct SmoltcpAdapter<'a, D: NetworkDriver> {
    driver: &'a mut D,
    rx_buffer: [u8; 2048],
    rx_len: usize,
    tx_count: u32,
    rx_count: u32,
}

impl<'a, D: NetworkDriver> SmoltcpAdapter<'a, D> {
    /// Create a new adapter wrapping a network driver.
    pub fn new(driver: &'a mut D) -> Self {
        Self {
            driver,
            rx_buffer: [0u8; 2048],
            rx_len: 0,
            tx_count: 0,
            rx_count: 0,
        }
    }

    /// Poll hardware for received packets.
    pub fn poll_receive(&mut self) {
        if self.rx_len == 0 {
            if let Ok(Some(len)) = self.driver.receive(&mut self.rx_buffer) {
                self.rx_len = len;
                self.rx_count += 1;
            }
        }
    }

    /// Refill RX descriptor queue.
    pub fn refill_rx(&mut self) {
        self.driver.refill_rx_queue();
    }

    /// Collect TX completions.
    pub fn collect_tx(&mut self) {
        self.driver.collect_tx_completions();
    }

    /// Get MAC address.
    pub fn mac_address(&self) -> [u8; 6] {
        self.driver.mac_address()
    }

    /// Get TX packet count.
    pub fn tx_count(&self) -> u32 {
        self.tx_count
    }

    /// Get RX packet count.
    pub fn rx_count(&self) -> u32 {
        self.rx_count
    }

    /// Check if PHY link is up.
    pub fn driver_link_up(&self) -> bool {
        self.driver.link_up()
    }

    /// Increment TX counter (called from TxToken).
    fn inc_tx(&mut self) {
        self.tx_count += 1;
    }
}

/// RX token — fixed-size buffer, no allocation.
pub struct RxToken {
    buffer: [u8; 2048],
    len: usize,
}

impl smoltcp::phy::RxToken for RxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.buffer[..self.len])
    }
}

/// TX token — writes directly via driver.
pub struct TxToken<'a, D: NetworkDriver> {
    driver: &'a mut D,
}

impl<'a, D: NetworkDriver> smoltcp::phy::TxToken for TxToken<'a, D> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        const MAX_FRAME: usize = 2048;
        let mut buffer = [0u8; MAX_FRAME];
        let actual_len = len.min(MAX_FRAME);
        
        let result = f(&mut buffer[..actual_len]);
        
        // Fire-and-forget transmit
        let _ = self.driver.transmit(&buffer[..actual_len]);
        
        result
    }
}

impl<'a, D: NetworkDriver> Device for SmoltcpAdapter<'a, D> {
    type RxToken<'b> = RxToken where Self: 'b;
    type TxToken<'b> = TxToken<'b, D> where Self: 'b;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.poll_receive();

        if self.rx_len > 0 {
            let mut rx_buf = [0u8; 2048];
            let copy_len = self.rx_len.min(2048);
            rx_buf[..copy_len].copy_from_slice(&self.rx_buffer[..copy_len]);
            let rx_len = copy_len;
            self.rx_len = 0;

            Some((
                RxToken { buffer: rx_buf, len: rx_len },
                TxToken { driver: self.driver },
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.driver.can_transmit() {
            Some(TxToken { driver: self.driver })
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps.max_burst_size = Some(32);
        caps
    }
}
