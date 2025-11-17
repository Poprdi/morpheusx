//! UEFI protocol management
//!
//! TODO: Implement protocol handling
//! - Locate protocols (HTTP, ServiceBinding, DHCP)
//! - Handle creation and destruction
//! - Protocol instance management
//! - GUID definitions

#[cfg(target_os = "uefi")]
pub mod uefi;

#[cfg(target_os = "uefi")]
pub use uefi::ProtocolManager;
