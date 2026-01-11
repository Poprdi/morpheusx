//! Buffer ownership state machine.
//!
//! # State Machine
//! ```text
//!     FREE ──alloc()──> DRIVER_OWNED ──submit()──> DEVICE_OWNED
//!       ▲                     │                         │
//!       └────free()───────────┴─────poll_complete()─────┘
//! ```
//!
//! INVARIANT: Accessing DEVICE_OWNED buffer is instant UB.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.4

/// Ownership state of a DMA buffer.
///
/// Tracks who owns each buffer to prevent use-after-submit bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferOwnership {
    /// Buffer is not allocated, available for use.
    Free,
    /// Buffer is owned by the driver (CPU may access).
    DriverOwned,
    /// Buffer is owned by the device (NO ACCESS ALLOWED).
    DeviceOwned,
}

impl BufferOwnership {
    /// Check if buffer can be accessed by CPU.
    pub fn can_access(&self) -> bool {
        matches!(self, BufferOwnership::DriverOwned)
    }

    /// Check if buffer is free.
    pub fn is_free(&self) -> bool {
        matches!(self, BufferOwnership::Free)
    }

    /// Check if buffer is device-owned.
    pub fn is_device_owned(&self) -> bool {
        matches!(self, BufferOwnership::DeviceOwned)
    }
}

impl Default for BufferOwnership {
    fn default() -> Self {
        BufferOwnership::Free
    }
}
