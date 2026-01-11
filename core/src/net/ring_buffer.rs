//! Error Ring Buffer for Network Initialization
//!
//! Circular buffer that captures initialization errors and debug logs
//! for display in the bootstrap UI. Also drains and forwards entries
//! from the network crate's internal debug ring buffer.
//!
//! # Design
//!
//! - Fixed-size, no_std compatible (no heap allocation for buffer)
//! - Lock-free for single-producer single-consumer pattern
//! - Overwrites oldest entries when full
//! - Includes stage/source tracking for categorization

use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum message length in bytes
pub const ERROR_MSG_LEN: usize = 96;

/// Number of entries in the ring buffer (power of 2 for efficient modulo)
pub const ERROR_RING_SIZE: usize = 32;

/// Stage identifiers for error categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InitStage {
    /// DMA pool initialization
    DmaPool = 0,
    /// Hardware abstraction layer setup
    Hal = 1,
    /// PCI bus scanning
    PciScan = 2,
    /// VirtIO device initialization
    VirtioDevice = 3,
    /// DHCP negotiation
    Dhcp = 4,
    /// Network client creation
    NetworkClient = 5,
    /// General/unknown stage
    General = 6,
    /// Forwarded from network crate's ring buffer
    NetworkCrate = 7,
}

impl InitStage {
    /// Get human-readable stage name
    pub const fn name(&self) -> &'static str {
        match self {
            Self::DmaPool => "DMA",
            Self::Hal => "HAL",
            Self::PciScan => "PCI",
            Self::VirtioDevice => "VIRTIO",
            Self::Dhcp => "DHCP",
            Self::NetworkClient => "CLIENT",
            Self::General => "INIT",
            Self::NetworkCrate => "NET",
        }
    }

    /// Convert from network crate stage byte
    pub fn from_network_stage(stage: u8) -> Self {
        // Map network crate stages to our stages
        // Network crate uses: 0=CLIENT, 1=DEVICE, 2=DHCP, 3=DNS, 4=HTTP
        match stage {
            0 => Self::NetworkClient,
            1 => Self::VirtioDevice,
            2 => Self::Dhcp,
            3 => Self::General, // DNS
            4 => Self::General, // HTTP
            _ => Self::NetworkCrate,
        }
    }
}

/// Single log entry in the ring buffer
#[derive(Clone)]
pub struct ErrorLogEntry {
    /// Message content (null-terminated or full)
    pub msg: [u8; ERROR_MSG_LEN],
    /// Actual message length (excluding null)
    pub len: u8,
    /// Initialization stage when error occurred
    pub stage: InitStage,
    /// True if this is an error, false if just a debug/info log
    pub is_error: bool,
}

impl ErrorLogEntry {
    /// Create a new empty entry
    const fn empty() -> Self {
        Self {
            msg: [0u8; ERROR_MSG_LEN],
            len: 0,
            stage: InitStage::General,
            is_error: false,
        }
    }

    /// Get message as string slice
    pub fn message(&self) -> &str {
        let len = self.len as usize;
        let slice = &self.msg[..len.min(ERROR_MSG_LEN)];
        core::str::from_utf8(slice).unwrap_or("<invalid utf8>")
    }

    /// Format entry for display: "[STAGE] message"
    pub fn format(&self, buf: &mut [u8]) -> usize {
        let prefix = if self.is_error { "ERR " } else { "" };
        let stage = self.stage.name();
        let msg = self.message();

        let mut pos = 0;

        // Write "[STAGE] " prefix
        if pos < buf.len() {
            buf[pos] = b'[';
            pos += 1;
        }
        for &b in prefix.as_bytes() {
            if pos >= buf.len() {
                break;
            }
            buf[pos] = b;
            pos += 1;
        }
        for &b in stage.as_bytes() {
            if pos >= buf.len() {
                break;
            }
            buf[pos] = b;
            pos += 1;
        }
        if pos < buf.len() {
            buf[pos] = b']';
            pos += 1;
        }
        if pos < buf.len() {
            buf[pos] = b' ';
            pos += 1;
        }

        // Write message
        for &b in msg.as_bytes() {
            if pos >= buf.len() {
                break;
            }
            buf[pos] = b;
            pos += 1;
        }

        pos
    }
}

impl Default for ErrorLogEntry {
    fn default() -> Self {
        Self::empty()
    }
}

/// Static ring buffer storage
static mut ERROR_RING: [ErrorLogEntry; ERROR_RING_SIZE] = {
    const EMPTY: ErrorLogEntry = ErrorLogEntry::empty();
    [EMPTY; ERROR_RING_SIZE]
};

/// Write position (wraps around)
static WRITE_POS: AtomicUsize = AtomicUsize::new(0);

/// Read position (wraps around)
static READ_POS: AtomicUsize = AtomicUsize::new(0);

/// Total entries written (for overflow detection)
static TOTAL_WRITTEN: AtomicUsize = AtomicUsize::new(0);

/// Log an error message to the ring buffer
///
/// # Arguments
/// * `stage` - Which initialization stage this error occurred in
/// * `msg` - Error message (will be truncated if too long)
pub fn error_log(stage: InitStage, msg: &str) {
    log_internal(stage, msg, true);
}

/// Log a debug/info message to the ring buffer
///
/// # Arguments
/// * `stage` - Which initialization stage this message is from
/// * `msg` - Debug message (will be truncated if too long)
pub fn debug_log(stage: InitStage, msg: &str) {
    log_internal(stage, msg, false);
}

fn log_internal(stage: InitStage, msg: &str, is_error: bool) {
    let write_idx = WRITE_POS.fetch_add(1, Ordering::SeqCst) % ERROR_RING_SIZE;

    let mut entry = ErrorLogEntry::empty();
    entry.stage = stage;
    entry.is_error = is_error;

    let bytes = msg.as_bytes();
    let copy_len = bytes.len().min(ERROR_MSG_LEN);
    entry.msg[..copy_len].copy_from_slice(&bytes[..copy_len]);
    entry.len = copy_len as u8;

    // SAFETY: Single writer assumed (bootloader is single-threaded during init)
    unsafe {
        ERROR_RING[write_idx] = entry;
    }

    TOTAL_WRITTEN.fetch_add(1, Ordering::SeqCst);
}

/// Pop the oldest entry from the ring buffer
///
/// Returns `None` if buffer is empty
pub fn error_log_pop() -> Option<ErrorLogEntry> {
    let total = TOTAL_WRITTEN.load(Ordering::SeqCst);
    let read = READ_POS.load(Ordering::SeqCst);

    // Check if there are unread entries
    if read >= total {
        return None;
    }

    // If we've overflowed, skip to newest available
    let available = total.saturating_sub(read);
    if available > ERROR_RING_SIZE {
        // Skip ahead - some entries were overwritten
        let skip = available - ERROR_RING_SIZE;
        READ_POS.fetch_add(skip, Ordering::SeqCst);
    }

    let read_idx = READ_POS.fetch_add(1, Ordering::SeqCst) % ERROR_RING_SIZE;

    // SAFETY: Single reader assumed
    let entry = unsafe { ERROR_RING[read_idx].clone() };
    Some(entry)
}

/// Check how many entries are available to read
pub fn error_log_available() -> usize {
    let total = TOTAL_WRITTEN.load(Ordering::SeqCst);
    let read = READ_POS.load(Ordering::SeqCst);

    let available = total.saturating_sub(read);
    available.min(ERROR_RING_SIZE)
}

/// Get total number of entries ever written (for overflow detection)
pub fn error_log_count() -> usize {
    TOTAL_WRITTEN.load(Ordering::SeqCst)
}

/// Clear the ring buffer
pub fn error_log_clear() {
    let current = WRITE_POS.load(Ordering::SeqCst);
    READ_POS.store(current, Ordering::SeqCst);
}

/// Drain entries from the network crate's debug ring buffer
/// and forward them to our error ring buffer.
///
/// Call this periodically during init or after errors to capture
/// network stack debug output.
///
/// NOTE: This is currently a stub. The network crate dependency was removed
/// to break a cyclic dependency. Network logging should be handled directly
/// in the network crate or via a shared types crate in the future.
pub fn drain_network_logs() {
    // Stub implementation - network crate dependency removed to break cycle
    // See morpheus-network::stack for direct logging access if needed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_log_basic() {
        error_log_clear();
        error_log(InitStage::DmaPool, "Test error");

        assert_eq!(error_log_available(), 1);

        let entry = error_log_pop().unwrap();
        assert_eq!(entry.stage, InitStage::DmaPool);
        assert!(entry.is_error);
        assert_eq!(entry.message(), "Test error");
    }

    #[test]
    fn test_stage_names() {
        assert_eq!(InitStage::DmaPool.name(), "DMA");
        assert_eq!(InitStage::Dhcp.name(), "DHCP");
        assert_eq!(InitStage::VirtioDevice.name(), "VIRTIO");
    }

    #[test]
    fn test_format_entry() {
        let mut entry = ErrorLogEntry::empty();
        entry.stage = InitStage::PciScan;
        entry.is_error = true;
        let msg = b"No devices found";
        entry.msg[..msg.len()].copy_from_slice(msg);
        entry.len = msg.len() as u8;

        let mut buf = [0u8; 128];
        let len = entry.format(&mut buf);
        let formatted = core::str::from_utf8(&buf[..len]).unwrap();

        assert_eq!(formatted, "[ERR PCI] No devices found");
    }
}
