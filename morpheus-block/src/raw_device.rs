//! Device-handle types live in the leaf `morpheus-block-types` crate so the
//! kernel can use them without pulling this crate's USB-MSD/xhci driver stack
//! (which depends back on the kernel and would cycle). Re-exported here so
//! existing `morpheus_block::raw_device::*` / `morpheus_block::*` paths resolve.
pub use morpheus_block_types::{
    DeviceKind, MemBlockDevice, MemIoError, RawBlockDevice, RawIoError,
};
