//! Driver abstractions. All drivers must full-reset on init (see RESET_CONTRACT.md).
//! Preconditions: bus mastering on, MMIO BAR mapped, DMA legal.

pub mod ahci;
pub mod block_io_adapter;
pub mod block_traits;
pub mod intel;
pub mod sdhci;
pub mod traits;
pub mod unified;
pub mod unified_block_io;
pub mod usb_msd;
pub mod virtio;
pub mod virtio_blk;

pub use traits::{DriverInit, NetworkDriver, RxError, TxError};
pub use virtio::{VirtioConfig, VirtioInitError, VirtioNetDriver};

pub use intel::{E1000eConfig, E1000eDriver, E1000eError, IntelNicInfo};

pub use unified::{UnifiedDriverError, UnifiedNetworkDriver};

pub use block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
pub use virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};

pub use ahci::{AhciConfig, AhciDriver, AhciInitError};
pub use sdhci::{SdhciConfig, SdhciDriver, SdhciInitError};
pub use usb_msd::{UsbMsdConfig, UsbMsdDriver, UsbMsdInitError};

pub use block_io_adapter::{BlockIoError, VirtioBlkBlockIo};
pub use unified_block_io::{GenericBlockIo, UnifiedBlockIo, UnifiedBlockIoError};
