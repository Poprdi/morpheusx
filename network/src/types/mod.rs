//! Shared data types module.
//!
//! Contains all #[repr(C)] structures that are shared between Rust and ASM,
//! as well as other common type definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6, ARCHITECTURE_V3.md

pub mod repr_c;
pub mod virtio_hdr;
pub mod ethernet;
pub mod result;

// Re-exports
pub use repr_c::{VirtqueueState, RxResult, VirtqDesc, DriverState, RxPollResult, TxPollResult};
pub use virtio_hdr::{VirtioNetHdr, VIRTIO_NET_HDR_GSO_NONE};
pub use ethernet::{MacAddress, EthernetHeader, ETH_ALEN, ETH_HLEN, ETH_MTU, ETH_FRAME_MAX};
pub use result::AsmResult;

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
