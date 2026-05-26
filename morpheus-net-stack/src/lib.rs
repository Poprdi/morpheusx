//! Kernel-side TCP/IP, HTTP, and download state machines. Stack runs in ring 0.

#![no_std]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(static_mut_refs)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::fn_to_numeric_cast)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::new_without_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::large_enum_variant)]
// Poll-based state machines return early from loops.
#![allow(clippy::never_loop)]

extern crate alloc;

/// `impl From<Src> for Dst` via a single variant. `(_)` form drops the payload.
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

pub mod display;
pub mod time;
pub mod types;

/// Re-export of `morpheus_foundation::error` (canonical `NetworkError`).
pub mod error {
    pub use morpheus_foundation::error::*;
}

pub mod client;
pub mod http;
pub mod stack;
pub mod url;

pub mod mainloop;
pub mod state;
pub mod transfer;

pub mod entry;

pub use morpheus_foundation::error::{NetworkError, Result};
pub use types::{HttpMethod, ProgressCallback};

pub use client::{HttpClient, NativeHttpClient};
pub use http::{Headers, Request, Response};
pub use url::Url;

pub use stack::{DeviceAdapter, NetConfig, NetInterface, NetState};

pub use entry::{run_download, RunConfig, RunResult};
