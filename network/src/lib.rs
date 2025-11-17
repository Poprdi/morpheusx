//! MorpheusX Network Stack
//!
//! Modular HTTP/HTTPS client for UEFI bootloader environment.

#![no_std]
#![allow(dead_code)]
#![allow(unused_imports)]

extern crate alloc;

pub mod error;
pub mod types;
pub mod protocol;
pub mod http;
pub mod url;
pub mod transfer;
pub mod client;
pub mod utils;

pub use error::{NetworkError, Result};
pub use types::{HttpMethod, ProgressCallback};
pub use client::HttpClient;

#[cfg(target_os = "uefi")]
pub use client::uefi::UefiHttpClient;
