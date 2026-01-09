//! Boot integration module.
//!
//! Handles the transition from UEFI boot services to bare metal.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §7

pub mod handoff;
pub mod init;
