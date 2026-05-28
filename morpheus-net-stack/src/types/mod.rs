//! Shared types, including `#[repr(C)]` structs shared with ASM.

pub mod ethernet;
pub mod repr_c;
pub mod result;
pub mod virtio_hdr;

// Re-exports
pub use ethernet::{EthernetHeader, MacAddress, ETH_ALEN, ETH_FRAME_MAX, ETH_HLEN, ETH_MTU};
pub use repr_c::{DriverState, RxPollResult, TxPollResult};
pub use result::AsmResult;
pub use virtio_hdr::{VirtioNetHdr, VIRTIO_NET_HDR_GSO_NONE};

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

/// `(bytes_downloaded, total_bytes_or_none)`.
pub type ProgressCallback = fn(usize, Option<usize>);

/// `(bytes_downloaded, total_bytes, message)`.
pub type ProgressCallbackWithMessage<'a> = Option<&'a mut dyn FnMut(usize, usize, &str)>;
