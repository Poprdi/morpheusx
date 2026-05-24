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

/// Ownership state of a DMA buffer.
///
/// Tracks who owns each buffer to prevent use-after-submit bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BufferOwnership {
    /// Buffer is not allocated, available for use.
    #[default]
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

    pub fn is_free(&self) -> bool {
        matches!(self, BufferOwnership::Free)
    }

    pub fn is_device_owned(&self) -> bool {
        matches!(self, BufferOwnership::DeviceOwned)
    }
}
