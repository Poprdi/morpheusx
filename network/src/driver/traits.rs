//! Driver trait definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §8.2

// TODO: Implement traits
//
// /// Core network device interface.
// pub trait NetworkDriver {
//     fn mac_address(&self) -> [u8; 6];
//     fn can_transmit(&self) -> bool;
//     fn can_receive(&self) -> bool;
//     fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError>;
//     fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError>;
//     fn refill_rx_queue(&mut self);
//     fn collect_tx_completions(&mut self);
// }
//
// /// Driver initialization trait.
// pub trait DriverInit: Sized {
//     type Error;
//     type Config;
//     
//     fn supports_device(vendor: u16, device: u16) -> bool;
//     unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error>;
// }
//
// #[derive(Debug)]
// pub enum TxError { QueueFull, DeviceNotReady }
//
// #[derive(Debug)]
// pub enum RxError { BufferTooSmall { needed: usize }, DeviceError }
