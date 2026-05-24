//! DEPRECATED stub. Real net init runs post-ExitBootServices via
//! `morpheus_network::mainloop::orchestrator::download_with_config()`.

use super::config::InitConfig;
use super::error::{NetInitError, NetInitResult};
use super::ring_buffer::{error_log, InitStage};
use super::status::NetworkStatus;

#[deprecated(note = "Network init moved to post-EBS. Use download_with_config() instead.")]
pub struct NetworkInitResult {
    pub status: NetworkStatus,
}

#[deprecated(note = "Network init moved to post-EBS. Use download_with_config() instead.")]
pub struct NetworkInit;

#[allow(deprecated)]
impl NetworkInit {
    #[deprecated(note = "Network init moved to post-EBS. Use download_with_config() instead.")]
    pub fn initialize(
        _config: &InitConfig,
        _get_time_ms: fn() -> u64,
    ) -> NetInitResult<NetworkInitResult> {
        error_log(InitStage::General, "DEPRECATED: Use post-EBS network init");
        Err(NetInitError::Deprecated)
    }

    #[deprecated(note = "Network init moved to post-EBS. Use download_with_config() instead.")]
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

    #[deprecated(note = "Network init moved to post-EBS")]
    pub fn can_initialize() -> bool {
        false
    }

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
