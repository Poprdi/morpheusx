//! UEFI HTTP protocol type definitions.
//!
//! This module provides standalone UEFI type definitions for the network crate.
//! These are compatible with the bootloader's UEFI bindings but independent,
//! allowing the network crate to be used in different contexts.
//!
//! Based on UEFI Specification 2.10 Section 28.7 (EFI HTTP Protocol)
//!
//! # Design Note
//!
//! We duplicate some types from `bootloader/src/uefi/` intentionally to:
//! - Keep the network crate self-contained
//! - Allow use in test environments without UEFI
//! - Enable future platform-specific implementations
//!
//! # Safety
//!
//! All FFI types in this module are `#[repr(C)]` for UEFI ABI compatibility.
//! Function pointers use `extern "efiapi"` calling convention.

use core::ffi::c_void;

// ==================== Basic Types ====================

/// UEFI Status code.
pub type Status = usize;

/// UEFI Handle (opaque pointer).
pub type Handle = *mut c_void;

/// UEFI Event (opaque pointer for async operations).
pub type Event = *mut c_void;

/// Status code constants.
pub mod status {
    use super::Status;

    /// Operation completed successfully.
    pub const SUCCESS: Status = 0;
    
    /// The operation is not supported.
    pub const UNSUPPORTED: Status = 0x8000_0000_0000_0003;
    
    /// The protocol was not found.
    pub const NOT_FOUND: Status = 0x8000_0000_0000_000E;
    
    /// A timeout occurred.
    pub const TIMEOUT: Status = 0x8000_0000_0000_0012;
    
    /// The operation was aborted.
    pub const ABORTED: Status = 0x8000_0000_0000_0015;
    
    /// Invalid parameter was passed.
    pub const INVALID_PARAMETER: Status = 0x8000_0000_0000_0002;
    
    /// Out of resources.
    pub const OUT_OF_RESOURCES: Status = 0x8000_0000_0000_0009;
    
    /// Check if status indicates success.
    #[inline]
    pub const fn is_success(status: Status) -> bool {
        status == SUCCESS
    }
    
    /// Check if status indicates an error.
    #[inline]
    pub const fn is_error(status: Status) -> bool {
        (status & 0x8000_0000_0000_0000) != 0
    }
}

// ==================== GUID ====================

/// UEFI Globally Unique Identifier.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    /// Create a GUID from component values.
    pub const fn from_values(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self { data1, data2, data3, data4 }
    }
}

/// EFI HTTP Protocol GUID.
pub const HTTP_PROTOCOL_GUID: Guid = Guid::from_values(
    0x7a59b29b,
    0x910b,
    0x4171,
    [0x82, 0x42, 0xa8, 0x5a, 0x0d, 0xf2, 0x5b, 0x5b],
);

/// EFI HTTP Service Binding Protocol GUID.
pub const HTTP_SERVICE_BINDING_GUID: Guid = Guid::from_values(
    0xbdc8e6af,
    0xd9bc,
    0x4379,
    [0xa7, 0x2a, 0xe0, 0xc4, 0xe7, 0x5d, 0xae, 0x1c],
);

// ==================== Boot Services ====================

/// UEFI Boot Services table (partial definition for HTTP protocol access).
///
/// This matches the bootloader's BootServices struct layout.
/// We only define the fields needed for HTTP protocol initialization.
#[repr(C)]
pub struct BootServices {
    _header: [u8; 24],
    // Task Priority Services
    _raise_tpl: usize,
    _restore_tpl: usize,
    // Memory Services  
    pub allocate_pages: extern "efiapi" fn(
        allocate_type: usize,
        memory_type: usize,
        pages: usize,
        memory: *mut u64,
    ) -> Status,
    pub free_pages: extern "efiapi" fn(memory: u64, pages: usize) -> Status,
    pub get_memory_map: extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut u8,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> Status,
    pub allocate_pool: extern "efiapi" fn(pool_type: usize, size: usize, buffer: *mut *mut u8) -> Status,
    pub free_pool: extern "efiapi" fn(buffer: *mut u8) -> Status,
    // Event & Timer Services
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,
    // Protocol Handler Services
    _install_protocol_interface: usize,
    _reinstall_protocol_interface: usize,
    _uninstall_protocol_interface: usize,
    pub handle_protocol: extern "efiapi" fn(
        handle: Handle,
        protocol: *const Guid,
        interface: *mut *mut c_void,
    ) -> Status,
    _reserved: usize,
    _register_protocol_notify: usize,
    pub locate_handle: extern "efiapi" fn(
        search_type: usize,
        protocol: *const Guid,
        search_key: *const c_void,
        buffer_size: *mut usize,
        buffer: *mut Handle,
    ) -> Status,
    _locate_device_path: usize,
    _install_configuration_table: usize,
    // Image Services (skipped)
    _load_image: usize,
    _start_image: usize,
    _exit: usize,
    _unload_image: usize,
    _exit_boot_services: usize,
    // Miscellaneous Services (skipped)
    _get_next_monotonic_count: usize,
    _stall: usize,
    _set_watchdog_timer: usize,
    // Driver Support Services (skipped)
    _connect_controller: usize,
    _disconnect_controller: usize,
    // Open and Close Protocol Services
    pub open_protocol: extern "efiapi" fn(
        handle: Handle,
        protocol: *const Guid,
        interface: *mut *mut c_void,
        agent_handle: Handle,
        controller_handle: Handle,
        attributes: u32,
    ) -> Status,
    _close_protocol: usize,
    _open_protocol_information: usize,
    // Library Services
    _protocols_per_handle: usize,
    pub locate_handle_buffer: extern "efiapi" fn(
        search_type: usize,
        protocol: *const Guid,
        search_key: *const c_void,
        no_handles: *mut usize,
        buffer: *mut *mut Handle,
    ) -> Status,
    pub locate_protocol: extern "efiapi" fn(
        protocol: *const Guid,
        registration: *const c_void,
        interface: *mut *mut c_void,
    ) -> Status,
}

/// Search type for LocateHandle: ByProtocol.
pub const LOCATE_HANDLE_BY_PROTOCOL: usize = 2;

/// Open protocol attribute: BY_HANDLE_PROTOCOL.
pub const OPEN_PROTOCOL_BY_HANDLE_PROTOCOL: u32 = 0x00000001;
/// Open protocol attribute: GET_PROTOCOL.
pub const OPEN_PROTOCOL_GET_PROTOCOL: u32 = 0x00000002;

// ==================== HTTP Types ====================

/// HTTP version (major.minor).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpVersion {
    pub major: u16,
    pub minor: u16,
}

impl HttpVersion {
    /// HTTP/1.0
    pub const HTTP_1_0: Self = Self { major: 1, minor: 0 };
    /// HTTP/1.1
    pub const HTTP_1_1: Self = Self { major: 1, minor: 1 };
}

impl Default for HttpVersion {
    fn default() -> Self {
        Self::HTTP_1_1
    }
}

/// HTTP request method.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get = 0,
    Post = 1,
    Patch = 2,
    Options = 3,
    Connect = 4,
    Head = 5,
    Put = 6,
    Delete = 7,
    Trace = 8,
}

impl HttpMethod {
    /// Convert from our HttpMethod type to UEFI representation.
    pub fn from_types_method(method: crate::types::HttpMethod) -> Self {
        match method {
            crate::types::HttpMethod::Get => Self::Get,
            crate::types::HttpMethod::Head => Self::Head,
            crate::types::HttpMethod::Post => Self::Post,
            crate::types::HttpMethod::Put => Self::Put,
            crate::types::HttpMethod::Delete => Self::Delete,
        }
    }
}

/// HTTP status code wrapper.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpStatusCode(pub u32);

impl HttpStatusCode {
    // 1xx Informational
    pub const CONTINUE: Self = Self(100);
    pub const SWITCHING_PROTOCOLS: Self = Self(101);
    
    // 2xx Success
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const ACCEPTED: Self = Self(202);
    pub const NO_CONTENT: Self = Self(204);
    pub const PARTIAL_CONTENT: Self = Self(206);
    
    // 3xx Redirection
    pub const MOVED_PERMANENTLY: Self = Self(301);
    pub const FOUND: Self = Self(302);
    pub const SEE_OTHER: Self = Self(303);
    pub const NOT_MODIFIED: Self = Self(304);
    pub const TEMPORARY_REDIRECT: Self = Self(307);
    pub const PERMANENT_REDIRECT: Self = Self(308);
    
    // 4xx Client Error
    pub const BAD_REQUEST: Self = Self(400);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const FORBIDDEN: Self = Self(403);
    pub const NOT_FOUND: Self = Self(404);
    pub const METHOD_NOT_ALLOWED: Self = Self(405);
    pub const REQUEST_TIMEOUT: Self = Self(408);
    pub const RANGE_NOT_SATISFIABLE: Self = Self(416);
    
    // 5xx Server Error
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);
    pub const NOT_IMPLEMENTED: Self = Self(501);
    pub const BAD_GATEWAY: Self = Self(502);
    pub const SERVICE_UNAVAILABLE: Self = Self(503);
    pub const GATEWAY_TIMEOUT: Self = Self(504);
    
    /// Get the numeric status code.
    #[inline]
    pub const fn code(&self) -> u32 {
        self.0
    }
    
    /// Check if this is a success status (2xx).
    #[inline]
    pub const fn is_success(&self) -> bool {
        self.0 >= 200 && self.0 < 300
    }
    
    /// Check if this is a redirect status (3xx).
    #[inline]
    pub const fn is_redirect(&self) -> bool {
        self.0 >= 300 && self.0 < 400
    }
    
    /// Check if this is a client error (4xx).
    #[inline]
    pub const fn is_client_error(&self) -> bool {
        self.0 >= 400 && self.0 < 500
    }
    
    /// Check if this is a server error (5xx).
    #[inline]
    pub const fn is_server_error(&self) -> bool {
        self.0 >= 500 && self.0 < 600
    }
}

impl From<u32> for HttpStatusCode {
    fn from(code: u32) -> Self {
        Self(code)
    }
}

impl From<u16> for HttpStatusCode {
    fn from(code: u16) -> Self {
        Self(code as u32)
    }
}

// ==================== HTTP Configuration ====================

/// IPv4 access point configuration.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct HttpIpv4AccessPoint {
    /// Use default (DHCP) address.
    pub use_default_address: bool,
    /// Local IPv4 address.
    pub local_address: [u8; 4],
    /// Subnet mask.
    pub local_subnet: [u8; 4],
    /// Local port (0 for any).
    pub local_port: u16,
}

impl Default for HttpIpv4AccessPoint {
    fn default() -> Self {
        Self {
            use_default_address: true,
            local_address: [0, 0, 0, 0],
            local_subnet: [0, 0, 0, 0],
            local_port: 0,
        }
    }
}

/// IPv6 access point configuration.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct HttpIpv6AccessPoint {
    /// Local IPv6 address.
    pub local_address: [u8; 16],
    /// Local port (0 for any).
    pub local_port: u16,
}

impl Default for HttpIpv6AccessPoint {
    fn default() -> Self {
        Self {
            local_address: [0; 16],
            local_port: 0,
        }
    }
}

/// HTTP access point (union of IPv4 and IPv6).
#[repr(C)]
pub union HttpAccessPoint {
    pub ipv4_node: *mut HttpIpv4AccessPoint,
    pub ipv6_node: *mut HttpIpv6AccessPoint,
}

/// HTTP configuration data.
#[repr(C)]
pub struct HttpConfigData {
    /// HTTP version to use.
    pub http_version: HttpVersion,
    /// Timeout in milliseconds (0 = no timeout).
    pub timeout_millisec: u32,
    /// True if using IPv6.
    pub local_addr_is_ipv6: bool,
    /// Network access point.
    pub access_point: HttpAccessPoint,
}

// ==================== HTTP Messages ====================

/// HTTP request data.
#[repr(C)]
pub struct HttpRequestData {
    /// HTTP method.
    pub method: HttpMethod,
    /// URL as null-terminated UTF-16 string.
    pub url: *const u16,
}

/// HTTP response data.
#[repr(C)]
pub struct HttpResponseData {
    /// HTTP status code.
    pub status_code: HttpStatusCode,
}

/// HTTP header (name-value pair).
#[repr(C)]
pub struct HttpHeader {
    /// Header field name (null-terminated ASCII).
    pub field_name: *const u8,
    /// Header field value (null-terminated ASCII).
    pub field_value: *const u8,
}

/// Union for request or response data in an HTTP message.
#[repr(C)]
pub union HttpMessageData {
    pub request: *mut HttpRequestData,
    pub response: *mut HttpResponseData,
}

/// HTTP message (request or response).
#[repr(C)]
pub struct HttpMessage {
    /// Request or response data.
    pub data: HttpMessageData,
    /// Number of headers.
    pub header_count: usize,
    /// Array of headers.
    pub headers: *mut HttpHeader,
    /// Body length in bytes.
    pub body_length: usize,
    /// Body data.
    pub body: *mut u8,
}

/// HTTP token for async operations.
#[repr(C)]
pub struct HttpToken {
    /// Event to signal on completion.
    pub event: Event,
    /// Status after completion.
    pub status: Status,
    /// HTTP message.
    pub message: *mut HttpMessage,
}

// ==================== Protocol Definitions ====================

/// Service Binding Protocol for creating child instances.
#[repr(C)]
pub struct ServiceBindingProtocol {
    /// Create a child handle with the protocol.
    pub create_child: unsafe extern "efiapi" fn(
        this: *mut ServiceBindingProtocol,
        child_handle: *mut Handle,
    ) -> Status,

    /// Destroy a child handle.
    pub destroy_child: unsafe extern "efiapi" fn(
        this: *mut ServiceBindingProtocol,
        child_handle: Handle,
    ) -> Status,
}

/// EFI HTTP Protocol interface.
#[repr(C)]
pub struct HttpProtocol {
    /// Get current configuration.
    pub get_mode_data: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        config_data: *mut HttpConfigData,
    ) -> Status,

    /// Configure the HTTP instance.
    pub configure: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        config_data: *const HttpConfigData,
    ) -> Status,

    /// Queue an HTTP request.
    pub request: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        token: *mut HttpToken,
    ) -> Status,

    /// Cancel a pending request.
    pub cancel: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        token: *mut HttpToken,
    ) -> Status,

    /// Queue to receive HTTP response.
    pub response: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        token: *mut HttpToken,
    ) -> Status,

    /// Poll for completion.
    pub poll: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
    ) -> Status,
}

// ==================== Helper Types ====================

/// Builder for HTTP configuration.
#[derive(Debug)]
pub struct HttpConfigBuilder {
    version: HttpVersion,
    timeout_ms: u32,
    use_ipv6: bool,
}

impl HttpConfigBuilder {
    /// Create a new config builder with defaults.
    pub fn new() -> Self {
        Self {
            version: HttpVersion::HTTP_1_1,
            timeout_ms: 30_000, // 30 seconds
            use_ipv6: false,
        }
    }

    /// Set HTTP version.
    pub fn version(mut self, version: HttpVersion) -> Self {
        self.version = version;
        self
    }

    /// Set timeout in milliseconds.
    pub fn timeout_ms(mut self, ms: u32) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Use IPv6 instead of IPv4.
    pub fn use_ipv6(mut self, ipv6: bool) -> Self {
        self.use_ipv6 = ipv6;
        self
    }

    /// Get the HTTP version.
    pub fn get_version(&self) -> HttpVersion {
        self.version
    }

    /// Get the timeout.
    pub fn get_timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    /// Check if using IPv6.
    pub fn is_ipv6(&self) -> bool {
        self.use_ipv6
    }
}

impl Default for HttpConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guid_creation() {
        let guid = Guid::from_values(
            0x12345678,
            0x1234,
            0x5678,
            [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0],
        );
        assert_eq!(guid.data1, 0x12345678);
        assert_eq!(guid.data2, 0x1234);
    }

    #[test]
    fn test_http_protocol_guid() {
        assert_eq!(HTTP_PROTOCOL_GUID.data1, 0x7a59b29b);
    }

    #[test]
    fn test_http_version() {
        assert_eq!(HttpVersion::HTTP_1_1.major, 1);
        assert_eq!(HttpVersion::HTTP_1_1.minor, 1);
        assert_eq!(HttpVersion::default(), HttpVersion::HTTP_1_1);
    }

    #[test]
    fn test_status_code_checks() {
        assert!(HttpStatusCode::OK.is_success());
        assert!(!HttpStatusCode::OK.is_redirect());
        
        assert!(HttpStatusCode::MOVED_PERMANENTLY.is_redirect());
        assert!(!HttpStatusCode::MOVED_PERMANENTLY.is_success());
        
        assert!(HttpStatusCode::NOT_FOUND.is_client_error());
        assert!(HttpStatusCode::INTERNAL_SERVER_ERROR.is_server_error());
    }

    #[test]
    fn test_status_code_from() {
        let code: HttpStatusCode = 200u32.into();
        assert_eq!(code.code(), 200);
        
        let code: HttpStatusCode = 404u16.into();
        assert_eq!(code.code(), 404);
    }

    #[test]
    fn test_status_functions() {
        assert!(status::is_success(status::SUCCESS));
        assert!(!status::is_error(status::SUCCESS));
        assert!(status::is_error(status::NOT_FOUND));
    }

    #[test]
    fn test_http_method_conversion() {
        use crate::types::HttpMethod as TypesMethod;
        
        assert_eq!(HttpMethod::from_types_method(TypesMethod::Get), HttpMethod::Get);
        assert_eq!(HttpMethod::from_types_method(TypesMethod::Post), HttpMethod::Post);
        assert_eq!(HttpMethod::from_types_method(TypesMethod::Head), HttpMethod::Head);
    }

    #[test]
    fn test_ipv4_access_point_default() {
        let ap = HttpIpv4AccessPoint::default();
        assert!(ap.use_default_address);
        assert_eq!(ap.local_port, 0);
    }

    #[test]
    fn test_config_builder() {
        let config = HttpConfigBuilder::new()
            .version(HttpVersion::HTTP_1_0)
            .timeout_ms(5000)
            .use_ipv6(true);
        
        assert_eq!(config.get_version(), HttpVersion::HTTP_1_0);
        assert_eq!(config.get_timeout_ms(), 5000);
        assert!(config.is_ipv6());
    }

    #[test]
    fn test_config_builder_defaults() {
        let config = HttpConfigBuilder::default();
        assert_eq!(config.get_version(), HttpVersion::HTTP_1_1);
        assert_eq!(config.get_timeout_ms(), 30_000);
        assert!(!config.is_ipv6());
    }
}
