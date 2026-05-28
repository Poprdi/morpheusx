//! NIC driver traits.

/// Local alias mirroring `morpheus_network::types::MacAddress`, kept here to
/// avoid depending on `morpheus-network`.
pub type MacAddress = [u8; 6];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    QueueFull,
    DeviceNotReady,
    FrameTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxError {
    BufferTooSmall { needed: usize },
    DeviceError,
}

pub trait NetworkDriver {
    fn mac_address(&self) -> MacAddress;

    fn can_transmit(&self) -> bool;

    fn can_receive(&self) -> bool;

    /// Queue an Ethernet frame (no VirtIO header). Fire-and-forget; never blocks.
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError>;

    /// Non-blocking RX. `Ok(None)` means no frame ready.
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError>;

    /// Mainloop Phase 1: replenish RX descriptors.
    fn refill_rx_queue(&mut self);

    /// Mainloop Phase 5: reap completed TX descriptors.
    fn collect_tx_completions(&mut self);

    fn link_up(&self) -> bool {
        true
    }
}

pub trait DriverInit: Sized {
    type Error: core::fmt::Debug;
    type Config;

    fn supported_vendors() -> &'static [u16];
    fn supported_devices() -> &'static [u16];

    fn supports_device(vendor: u16, device: u16) -> bool {
        Self::supported_vendors().contains(&vendor) && Self::supported_devices().contains(&device)
    }

    /// # Safety
    /// `mmio_base` must point at the device's MMIO BAR.
    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error>;
}
