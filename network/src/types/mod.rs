//! Shared data types module.
//!
//! Contains all #[repr(C)] structures that are shared between Rust and ASM,
//! as well as other common type definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6, ARCHITECTURE_V3.md

pub mod ethernet;
pub mod repr_c;
pub mod result;
pub mod virtio_hdr;

// Re-exports
pub use ethernet::{EthernetHeader, MacAddress, ETH_ALEN, ETH_FRAME_MAX, ETH_HLEN, ETH_MTU};
pub use repr_c::{DriverState, RxPollResult, RxResult, TxPollResult, VirtqDesc, VirtqueueState};
pub use result::AsmResult;
pub use virtio_hdr::{VirtioNetHdr, VIRTIO_NET_HDR_GSO_NONE};

// ═══════════════════════════════════════════════════════════════════════════
// HTTP Types
// ═══════════════════════════════════════════════════════════════════════════

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Head,
    Put,
    Delete,
    Patch,
    Options,
}

impl HttpMethod {
    /// Get the method name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Head => "HEAD",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Options => "OPTIONS",
        }
    }
}

/// Progress callback type for download operations.
/// Simple function pointer type (bytes_downloaded, total_bytes_or_none).
pub type ProgressCallback = fn(usize, Option<usize>);

/// Progress callback with message type for operations with status messages.
/// Parameters: (bytes_downloaded, total_bytes, message)
pub type ProgressCallbackWithMessage<'a> = Option<&'a mut dyn FnMut(usize, usize, &str)>;
