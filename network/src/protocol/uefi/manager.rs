//! UEFI protocol manager for HTTP operations.
//!
//! Manages the lifecycle of UEFI HTTP protocol instances:
//! - Locating Service Binding Protocol
//! - Creating child HTTP protocol instances
//! - Configuring HTTP parameters
//! - Cleanup and resource management
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                   Boot Services                         │
//! ├─────────────────────────────────────────────────────────┤
//! │   ┌─────────────────────┐    ┌─────────────────────┐   │
//! │   │ Service Binding     │───▶│ HTTP Protocol       │   │
//! │   │ Protocol            │    │ Instance            │   │
//! │   └─────────────────────┘    └─────────────────────┘   │
//! │           │                           │                 │
//! │           │ CreateChild               │ Configure       │
//! │           │ DestroyChild              │ Request/Response│
//! │           ▼                           ▼                 │
//! │   ┌─────────────────────────────────────────────────┐  │
//! │   │              Protocol Manager                    │  │
//! │   │  - Handle lifecycle                              │  │
//! │   │  - Error handling                                │  │
//! │   │  - Resource cleanup                              │  │
//! │   └─────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! The ProtocolManager is designed to be used with UEFI Boot Services.
//! In a non-UEFI context (testing), it provides mock functionality.

use crate::error::{NetworkError, Result};
use super::bindings::{
    Guid, Handle, Status, HttpProtocol, ServiceBindingProtocol,
    HttpConfigData, HttpVersion, HttpAccessPoint, HttpIpv4AccessPoint,
    HttpConfigBuilder, status, HTTP_SERVICE_BINDING_GUID, HTTP_PROTOCOL_GUID,
    BootServices, OPEN_PROTOCOL_GET_PROTOCOL,
};
use core::ptr;

/// State of the protocol manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerState {
    /// Manager created but not initialized.
    Uninitialized,
    /// Service binding located.
    ServiceBound,
    /// Child HTTP instance created.
    ChildCreated,
    /// HTTP protocol configured and ready.
    Configured,
    /// Error state.
    Error,
}

/// UEFI Protocol Manager.
///
/// Manages HTTP protocol lifecycle in UEFI environment.
/// Provides safe wrappers around unsafe UEFI calls.
#[derive(Debug)]
pub struct ProtocolManager {
    /// Current state.
    state: ManagerState,
    /// Service binding protocol pointer (if available).
    service_binding: Option<*mut ServiceBindingProtocol>,
    /// Child handle created via service binding.
    child_handle: Option<Handle>,
    /// HTTP protocol instance.
    http_protocol: Option<*mut HttpProtocol>,
    /// Configuration used.
    config: HttpConfigBuilder,
    /// IPv4 access point (must be kept alive while configured).
    ipv4_access_point: Option<HttpIpv4AccessPoint>,
}

impl ProtocolManager {
    /// Create a new protocol manager.
    ///
    /// The manager is created in uninitialized state.
    /// Call `initialize()` with boot services to set up protocols.
    pub fn new() -> Self {
        Self {
            state: ManagerState::Uninitialized,
            service_binding: None,
            child_handle: None,
            http_protocol: None,
            config: HttpConfigBuilder::default(),
            ipv4_access_point: None,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: HttpConfigBuilder) -> Self {
        Self {
            state: ManagerState::Uninitialized,
            service_binding: None,
            child_handle: None,
            http_protocol: None,
            config,
            ipv4_access_point: None,
        }
    }

    /// Get current state.
    pub fn state(&self) -> ManagerState {
        self.state
    }

    /// Check if manager is ready for requests.
    pub fn is_ready(&self) -> bool {
        self.state == ManagerState::Configured
    }

    /// Get the HTTP protocol pointer (if available).
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid while the manager is in
    /// `Configured` state. Caller must not use after `shutdown()`.
    pub fn http_protocol(&self) -> Option<*mut HttpProtocol> {
        if self.state == ManagerState::Configured {
            self.http_protocol
        } else {
            None
        }
    }

    /// Initialize with mock data for testing.
    ///
    /// This allows testing the manager logic without actual UEFI.
    #[cfg(test)]
    pub fn initialize_mock(&mut self) -> Result<()> {
        self.state = ManagerState::Configured;
        Ok(())
    }

    /// Initialize the protocol manager directly from UEFI BootServices pointer.
    ///
    /// This is a simpler initialization path that takes a raw BootServices pointer
    /// instead of closures. This is the preferred method for bootloader integration.
    ///
    /// # Safety
    ///
    /// - `boot_services` must be a valid pointer to UEFI Boot Services table.
    /// - Must be called before `ExitBootServices()`.
    #[cfg(target_os = "uefi")]
    pub unsafe fn initialize_from_boot_services(
        &mut self,
        boot_services: *const BootServices,
    ) -> Result<()> {
        if boot_services.is_null() {
            return Err(NetworkError::InitializationFailed);
        }
        
        let bs = &*boot_services;
        
        // Step 1: Locate HTTP Service Binding Protocol
        let mut service_binding_ptr: *mut core::ffi::c_void = ptr::null_mut();
        let status = (bs.locate_protocol)(
            &HTTP_SERVICE_BINDING_GUID,
            ptr::null(),
            &mut service_binding_ptr,
        );
        
        if !status::is_success(status) || service_binding_ptr.is_null() {
            self.state = ManagerState::Error;
            return Err(NetworkError::ProtocolNotAvailable);
        }
        
        let service_binding = service_binding_ptr as *mut ServiceBindingProtocol;
        self.service_binding = Some(service_binding);
        self.state = ManagerState::ServiceBound;

        // Step 2: Create child HTTP protocol instance
        let mut child_handle: Handle = ptr::null_mut();
        let status = ((*service_binding).create_child)(
            service_binding,
            &mut child_handle,
        );
        
        if !status::is_success(status) || child_handle.is_null() {
            self.state = ManagerState::Error;
            return Err(NetworkError::InitializationFailed);
        }
        
        self.child_handle = Some(child_handle);
        self.state = ManagerState::ChildCreated;

        // Step 3: Open HTTP Protocol on child handle
        let mut http_ptr: *mut core::ffi::c_void = ptr::null_mut();
        let status = (bs.open_protocol)(
            child_handle,
            &HTTP_PROTOCOL_GUID,
            &mut http_ptr,
            ptr::null_mut(), // agent_handle (none)
            ptr::null_mut(), // controller_handle (none)
            OPEN_PROTOCOL_GET_PROTOCOL,
        );
        
        if !status::is_success(status) || http_ptr.is_null() {
            self.state = ManagerState::Error;
            return Err(NetworkError::ProtocolNotAvailable);
        }
        
        self.http_protocol = Some(http_ptr as *mut HttpProtocol);

        // Step 4: Configure HTTP
        self.configure_http()?;

        Ok(())
    }

    /// Initialize the protocol manager with UEFI boot services.
    ///
    /// This method:
    /// 1. Locates the HTTP Service Binding Protocol
    /// 2. Creates a child HTTP Protocol instance
    /// 3. Configures the HTTP instance
    ///
    /// # Arguments
    ///
    /// * `locate_protocol` - Function to locate protocol by GUID
    /// * `open_protocol` - Function to open protocol on handle
    ///
    /// # Safety
    ///
    /// Caller must ensure the function pointers are valid UEFI boot services.
    pub unsafe fn initialize<F, G>(
        &mut self,
        locate_protocol: F,
        open_protocol: G,
    ) -> Result<()>
    where
        F: Fn(&Guid) -> Option<*mut ServiceBindingProtocol>,
        G: Fn(Handle, &Guid) -> Option<*mut HttpProtocol>,
    {
        // Step 1: Locate Service Binding
        let service_binding = locate_protocol(&HTTP_SERVICE_BINDING_GUID)
            .ok_or(NetworkError::ProtocolNotAvailable)?;
        
        self.service_binding = Some(service_binding);
        self.state = ManagerState::ServiceBound;

        // Step 2: Create child
        let mut child_handle: Handle = ptr::null_mut();
        let status = ((*service_binding).create_child)(
            service_binding,
            &mut child_handle,
        );
        
        if !status::is_success(status) || child_handle.is_null() {
            self.state = ManagerState::Error;
            return Err(NetworkError::InitializationFailed);
        }
        
        self.child_handle = Some(child_handle);
        self.state = ManagerState::ChildCreated;

        // Step 3: Open HTTP protocol on child
        let http_protocol = open_protocol(child_handle, &HTTP_PROTOCOL_GUID)
            .ok_or_else(|| {
                self.state = ManagerState::Error;
                NetworkError::ProtocolNotAvailable
            })?;
        
        self.http_protocol = Some(http_protocol);

        // Step 4: Configure HTTP
        self.configure_http()?;

        Ok(())
    }

    /// Configure the HTTP protocol instance.
    ///
    /// # Safety
    ///
    /// Must be called after HTTP protocol is obtained.
    unsafe fn configure_http(&mut self) -> Result<()> {
        let http = self.http_protocol.ok_or(NetworkError::InitializationFailed)?;

        // Set up IPv4 access point
        let ipv4_ap = HttpIpv4AccessPoint::default();
        self.ipv4_access_point = Some(ipv4_ap.clone());
        
        // Get pointer to stored access point
        let ipv4_ptr = self.ipv4_access_point.as_mut().unwrap() as *mut HttpIpv4AccessPoint;

        let config_data = HttpConfigData {
            http_version: self.config.get_version(),
            timeout_millisec: self.config.get_timeout_ms(),
            local_addr_is_ipv6: self.config.is_ipv6(),
            access_point: HttpAccessPoint { ipv4_node: ipv4_ptr },
        };

        let status = ((*http).configure)(http, &config_data);
        
        if !status::is_success(status) {
            self.state = ManagerState::Error;
            return Err(NetworkError::InitializationFailed);
        }

        self.state = ManagerState::Configured;
        Ok(())
    }

    /// Reconfigure HTTP with new settings.
    ///
    /// # Safety
    ///
    /// Must be in Configured state.
    pub unsafe fn reconfigure(&mut self, config: HttpConfigBuilder) -> Result<()> {
        if self.state != ManagerState::Configured {
            return Err(NetworkError::InitializationFailed);
        }

        // First unconfigure by passing null
        let http = self.http_protocol.ok_or(NetworkError::InitializationFailed)?;
        let _ = ((*http).configure)(http, ptr::null());

        // Update config and reconfigure
        self.config = config;
        self.configure_http()
    }

    /// Shutdown and cleanup resources.
    ///
    /// Destroys the child HTTP instance and releases resources.
    ///
    /// # Safety
    ///
    /// After calling this, the manager cannot be used until reinitialized.
    pub unsafe fn shutdown(&mut self) -> Result<()> {
        // Unconfigure HTTP if configured
        if let Some(http) = self.http_protocol {
            let _ = ((*http).configure)(http, ptr::null());
        }

        // Destroy child handle
        if let (Some(service_binding), Some(child_handle)) = 
            (self.service_binding, self.child_handle) 
        {
            let status = ((*service_binding).destroy_child)(service_binding, child_handle);
            if !status::is_success(status) {
                // Log but don't fail - cleanup should be best effort
            }
        }

        // Reset state
        self.http_protocol = None;
        self.child_handle = None;
        self.service_binding = None;
        self.ipv4_access_point = None;
        self.state = ManagerState::Uninitialized;

        Ok(())
    }

    /// Poll for completion of async operations.
    ///
    /// # Safety
    ///
    /// Must be in Configured state with active operation.
    pub unsafe fn poll(&self) -> Result<bool> {
        let http = self.http_protocol.ok_or(NetworkError::InitializationFailed)?;
        
        let status = ((*http).poll)(http);
        
        match status {
            s if status::is_success(s) => Ok(true),
            s if s == status::NOT_FOUND => Ok(false), // Nothing to poll
            _ => Err(NetworkError::Unknown),
        }
    }

    /// Get the service binding GUID.
    pub fn service_binding_guid() -> &'static Guid {
        &HTTP_SERVICE_BINDING_GUID
    }

    /// Get the HTTP protocol GUID.
    pub fn http_protocol_guid() -> &'static Guid {
        &HTTP_PROTOCOL_GUID
    }
}

impl Default for ProtocolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ProtocolManager {
    fn drop(&mut self) {
        // Best-effort cleanup
        // In UEFI context, proper shutdown should be called explicitly
        if self.state != ManagerState::Uninitialized {
            // SAFETY: We're cleaning up, ignoring errors
            unsafe {
                let _ = self.shutdown();
            }
        }
    }
}

// ==================== Mock Protocol Manager for Testing ====================

/// Mock protocol manager for testing without UEFI.
#[cfg(test)]
pub struct MockProtocolManager {
    /// Simulated state.
    state: ManagerState,
    /// Simulated request count.
    request_count: usize,
}

#[cfg(test)]
impl MockProtocolManager {
    pub fn new() -> Self {
        Self {
            state: ManagerState::Configured,
            request_count: 0,
        }
    }

    pub fn state(&self) -> ManagerState {
        self.state
    }

    pub fn is_ready(&self) -> bool {
        self.state == ManagerState::Configured
    }

    pub fn simulate_request(&mut self) -> Result<()> {
        if self.state != ManagerState::Configured {
            return Err(NetworkError::InitializationFailed);
        }
        self.request_count += 1;
        Ok(())
    }

    pub fn request_count(&self) -> usize {
        self.request_count
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_new() {
        let manager = ProtocolManager::new();
        assert_eq!(manager.state(), ManagerState::Uninitialized);
        assert!(!manager.is_ready());
        assert!(manager.http_protocol().is_none());
    }

    #[test]
    fn test_manager_with_config() {
        let config = HttpConfigBuilder::new()
            .timeout_ms(5000)
            .version(HttpVersion::HTTP_1_0);
        
        let manager = ProtocolManager::with_config(config);
        assert_eq!(manager.state(), ManagerState::Uninitialized);
    }

    #[test]
    fn test_manager_mock_initialize() {
        let mut manager = ProtocolManager::new();
        manager.initialize_mock().unwrap();
        
        assert_eq!(manager.state(), ManagerState::Configured);
        assert!(manager.is_ready());
    }

    #[test]
    fn test_manager_guids() {
        let sb_guid = ProtocolManager::service_binding_guid();
        assert_eq!(sb_guid.data1, 0xbdc8e6af);
        
        let http_guid = ProtocolManager::http_protocol_guid();
        assert_eq!(http_guid.data1, 0x7a59b29b);
    }

    #[test]
    fn test_manager_state_enum() {
        assert_ne!(ManagerState::Uninitialized, ManagerState::Configured);
        assert_ne!(ManagerState::ServiceBound, ManagerState::ChildCreated);
    }

    #[test]
    fn test_mock_protocol_manager() {
        let mut mock = MockProtocolManager::new();
        
        assert!(mock.is_ready());
        assert_eq!(mock.request_count(), 0);
        
        mock.simulate_request().unwrap();
        assert_eq!(mock.request_count(), 1);
    }

    #[test]
    fn test_manager_default() {
        let manager = ProtocolManager::default();
        assert_eq!(manager.state(), ManagerState::Uninitialized);
    }
}
