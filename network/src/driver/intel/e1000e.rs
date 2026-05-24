//! e1000e `NetworkDriver` impl. See 82579 datasheet §10.

use crate::asm::drivers::intel::{asm_intel_link_status, LinkStatusResult};
use crate::driver::traits::{DriverInit, NetworkDriver, RxError, TxError};
use crate::mainloop::serial::serial_println;
use crate::types::MacAddress;

use super::init::{init_e1000e, E1000eConfig, E1000eInitError};
use super::phy::PhyManager;
use super::rx::RxRing;
use super::tx::TxRing;
use super::{E1000E_DEVICE_IDS, INTEL_VENDOR_ID};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E1000eError {
    InitFailed(E1000eInitError),
    NotReady,
    LinkDown,
}

crate::impl_from!(E1000eInitError => E1000eError : InitFailed);

pub struct E1000eDriver {
    mmio_base: u64,
    mac: MacAddress,
    phy: PhyManager,
    rx_ring: RxRing,
    tx_ring: TxRing,
    initialized: bool,
}

impl E1000eDriver {
    /// # Safety
    /// `mmio_base` must be the device's mapped BAR0; DMA must be set up.
    pub unsafe fn new(mmio_base: u64, config: E1000eConfig) -> Result<Self, E1000eError> {
        serial_println("    [e1000e] E1000eDriver::new() entered");
        serial_println("    [e1000e] About to call init_e1000e()...");

        let result = init_e1000e(mmio_base, &config)?;

        serial_println("    [e1000e] init_e1000e() returned");

        let phy = PhyManager::new(mmio_base, config.tsc_freq);

        Ok(Self {
            mmio_base,
            mac: result.mac,
            phy,
            rx_ring: result.rx_ring,
            tx_ring: result.tx_ring,
            initialized: true,
        })
    }

    pub fn mmio_base(&self) -> u64 {
        self.mmio_base
    }

    pub fn phy(&mut self) -> &mut PhyManager {
        &mut self.phy
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn wait_for_link(&mut self, timeout_us: u64) -> bool {
        self.phy.wait_for_link(timeout_us).is_ok()
    }
}

impl NetworkDriver for E1000eDriver {
    fn mac_address(&self) -> MacAddress {
        self.mac
    }

    fn can_transmit(&self) -> bool {
        self.initialized && self.tx_ring.can_transmit()
    }

    fn can_receive(&self) -> bool {
        self.initialized && self.rx_ring.can_receive()
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        if !self.initialized {
            return Err(TxError::DeviceNotReady);
        }

        self.tx_ring.transmit(frame).map_err(|e| match e {
            super::tx::TxError::QueueFull => TxError::QueueFull,
            super::tx::TxError::FrameTooLarge { .. } => TxError::FrameTooLarge,
        })
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        if !self.initialized {
            return Err(RxError::DeviceError);
        }

        self.rx_ring.receive(buffer).map_err(|e| match e {
            super::rx::RxError::BufferTooSmall { needed, .. } => RxError::BufferTooSmall { needed },
            super::rx::RxError::PacketError(_) => RxError::DeviceError,
        })
    }

    fn refill_rx_queue(&mut self) {
        // Descriptors are resubmitted inline in `receive()`.
    }

    fn collect_tx_completions(&mut self) {
        if self.initialized {
            self.tx_ring.collect_completions();
        }
    }

    fn link_up(&self) -> bool {
        // Always read live STATUS — cached state lies after suspend/resume.
        let mut result = LinkStatusResult::default();
        unsafe {
            asm_intel_link_status(self.mmio_base, &mut result);
        }
        result.link_up != 0
    }
}

impl DriverInit for E1000eDriver {
    type Error = E1000eError;
    type Config = E1000eConfig;

    fn supported_vendors() -> &'static [u16] {
        &[INTEL_VENDOR_ID]
    }

    fn supported_devices() -> &'static [u16] {
        E1000E_DEVICE_IDS
    }

    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error> {
        Self::new(mmio_base, config)
    }
}

// SAFETY: single-threaded driver; raw pointers never escape.
unsafe impl Send for E1000eDriver {}
