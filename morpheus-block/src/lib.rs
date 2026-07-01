//! Block-device drivers (AHCI, SDHCI, virtio_blk, USB-MSD) and unified I/O.

#![no_std]
extern crate alloc;

/// `impl From<Src> for Dst { fn from(e) -> Self { Dst::Variant(e) } }`. `(_)` form drops the payload.
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

pub mod ahci;
pub mod block_io_adapter;
pub mod block_traits;
pub mod boot_probe;
pub mod device;
pub mod gpt;
pub mod raw_device;
pub mod sdhci;
pub mod transfer;
pub mod unified_block_io;
pub mod usb_class;
pub mod usb_msd;
pub mod virtio_blk;

pub use block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};
pub use device::{UnifiedBlockDevice, UnifiedBlockError};
pub use gpt::{enumerate_partitions, PartitionEntry, PART_NAME_LEN};
pub use raw_device::{DeviceKind, MemBlockDevice, MemIoError, RawBlockDevice, RawIoError};
