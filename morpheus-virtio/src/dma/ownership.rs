//! DMA buffer ownership state machine.
//!
//! ```text
//!     FREE ──alloc()──> DRIVER_OWNED ──submit()──> DEVICE_OWNED
//!       ▲                     │                         │
//!       └────free()───────────┴─────poll_complete()─────┘
//! ```
//! INVARIANT: accessing a DEVICE_OWNED buffer is instant UB.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BufferOwnership {
    #[default]
    Free,
    DriverOwned,
    DeviceOwned,
}

impl BufferOwnership {
    pub fn can_access(&self) -> bool {
        matches!(self, BufferOwnership::DriverOwned)
    }

    pub fn is_free(&self) -> bool {
        matches!(self, BufferOwnership::Free)
    }

    pub fn is_device_owned(&self) -> bool {
        matches!(self, BufferOwnership::DeviceOwned)
    }
}
