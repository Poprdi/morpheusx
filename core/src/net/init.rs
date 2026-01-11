//! Network initialization orchestrator (DEPRECATED).
//!
//! # DEPRECATED
//!
//! This module is deprecated. Network initialization now happens **post-ExitBootServices**
//! using the bare-metal network stack directly in `morpheus_network::mainloop::bare_metal_main`.
//!
//! The old flow (this module):
//! 1. Initialize in UEFI environment
//! 2. Use factory pattern to detect/create devices
//!
//! The new flow:
//! 1. Bootstrap displays menu, user selects ISO
//! 2. ExitBootServices is called
//! 3. Bare-metal stack initializes VirtIO directly
//! 4. HTTP download proceeds
//!
//! This stub is kept for API compatibility with existing imports.

use super::config::InitConfig;
use super::error::{NetInitError, NetInitResult};
use super::ring_buffer::{error_log, InitStage};
use super::status::NetworkStatus;

/// Network initialization result (DEPRECATED).
///
/// This type is kept for API compatibility but `NetworkInit::initialize()`
/// always returns an error directing callers to use post-EBS flow.
#[deprecated(note = "Network init moved to post-EBS. Use bare_metal_main() instead.")]
pub struct NetworkInitResult {
    /// Network status with IP info.
    pub status: NetworkStatus,
}

/// Network initialization orchestrator (DEPRECATED).
///
/// Network initialization is now handled post-ExitBootServices by
/// `morpheus_network::mainloop::bare_metal_main()`.
#[deprecated(note = "Network init moved to post-EBS. Use bare_metal_main() instead.")]
pub struct NetworkInit;

#[allow(deprecated)]
impl NetworkInit {
    /// Perform complete network initialization (DEPRECATED).
    ///
    /// **Always returns error** - network init is now post-EBS.
    /// Use `morpheus_network::mainloop::bare_metal_main()` after ExitBootServices.
    #[deprecated(note = "Network init moved to post-EBS. Use bare_metal_main() instead.")]
    pub fn initialize(
        _config: &InitConfig,
        _get_time_ms: fn() -> u64,
    ) -> NetInitResult<NetworkInitResult> {
        error_log(InitStage::General, "DEPRECATED: Use post-EBS network init");
        Err(NetInitError::Deprecated)
    }

    /// Initialize with display polling (DEPRECATED).
    ///
    /// **Always returns error** - network init is now post-EBS.
    #[deprecated(note = "Network init moved to post-EBS. Use bare_metal_main() instead.")]
    pub fn initialize_with_poll<F>(
        _config: &InitConfig,
        _get_time_ms: fn() -> u64,
        _poll_display: F,
    ) -> NetInitResult<NetworkInitResult>
    where
        F: FnMut(),
    {
        error_log(InitStage::General, "DEPRECATED: Use post-EBS network init");
        Err(NetInitError::Deprecated)
    }

    /// Quick check if network initialization is possible (DEPRECATED).
    ///
    /// **Always returns false** - use post-EBS flow instead.
    #[deprecated(note = "Network init moved to post-EBS")]
    pub fn can_initialize() -> bool {
        false
    }

    /// Initialize only prerequisites (DEPRECATED).
    ///
    /// **Always returns error** - use post-EBS flow instead.
    #[deprecated(note = "Network init moved to post-EBS")]
    pub fn init_prerequisites(_config: &InitConfig) -> NetInitResult<()> {
        Err(NetInitError::Deprecated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = InitConfig::default();
        assert_eq!(config.dhcp_timeout_ms, 30_000);
        assert!(config.use_static_dma);
    }

    #[test]
    fn test_config_for_qemu() {
        let config = InitConfig::for_qemu();
        assert_eq!(config.dhcp_timeout_ms, 10_000);
    }

    #[test]
    fn test_error_descriptions() {
        let error = NetInitError::DhcpTimeout;
        assert!(!error.description().is_empty());
    }

    #[test]
    fn test_status_ip_str() {
        let mut status = NetworkStatus::new();
        status.ip_address = [192, 168, 1, 100];
        let ip_str = status.ip_str();
        let s = core::str::from_utf8(&ip_str).unwrap().trim();
        assert!(s.starts_with("192.168.1.100"));
    }
}
