//! UEFI HTTP Protocol bindings
//!
//! Based on UEFI Specification 2.10 Section 28.7 (EFI HTTP Protocol)

/// EFI Status type
pub type Status = usize;

/// EFI GUID structure
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    pub const fn from_values(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self { data1, data2, data3, data4 }
    }
}

/// EFI Event type
pub type Event = *mut core::ffi::c_void;

/// EFI HTTP Protocol GUID
pub const HTTP_PROTOCOL_GUID: Guid = Guid::from_values(
    0x7a59b29b,
    0x910b,
    0x4171,
    [0x82, 0x42, 0xa8, 0x5a, 0x0d, 0xf2, 0x5b, 0x5b],
);

/// EFI HTTP Service Binding Protocol GUID
pub const HTTP_SERVICE_BINDING_GUID: Guid = Guid::from_values(
    0xbdc8e6af,
    0xd9bc,
    0x4379,
    [0xa7, 0x2a, 0xe0, 0xc4, 0xe7, 0x5d, 0xae, 0x1c],
);

/// HTTP version
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HttpVersion {
    pub major: u16,
    pub minor: u16,
}

impl HttpVersion {
    pub const HTTP_1_0: Self = Self { major: 1, minor: 0 };
    pub const HTTP_1_1: Self = Self { major: 1, minor: 1 };
}

/// HTTP method
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethodType {
    Get = 0,
    Post = 1,
    Patch = 2,
    Options = 3,
    Connect = 4,
    Head = 5,
    Put = 6,
    Delete = 7,
    Trace = 8,
    Max = 9,
}

/// HTTP status code
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HttpStatusCode(pub u32);

impl HttpStatusCode {
    pub const HTTP_STATUS_100_CONTINUE: Self = Self(100);
    pub const HTTP_STATUS_200_OK: Self = Self(200);
    pub const HTTP_STATUS_201_CREATED: Self = Self(201);
    pub const HTTP_STATUS_204_NO_CONTENT: Self = Self(204);
    pub const HTTP_STATUS_301_MOVED_PERMANENTLY: Self = Self(301);
    pub const HTTP_STATUS_302_FOUND: Self = Self(302);
    pub const HTTP_STATUS_304_NOT_MODIFIED: Self = Self(304);
    pub const HTTP_STATUS_400_BAD_REQUEST: Self = Self(400);
    pub const HTTP_STATUS_401_UNAUTHORIZED: Self = Self(401);
    pub const HTTP_STATUS_403_FORBIDDEN: Self = Self(403);
    pub const HTTP_STATUS_404_NOT_FOUND: Self = Self(404);
    pub const HTTP_STATUS_500_INTERNAL_SERVER_ERROR: Self = Self(500);
    pub const HTTP_STATUS_503_SERVICE_UNAVAILABLE: Self = Self(503);
}

/// HTTP configuration data
#[repr(C)]
pub struct HttpConfigData {
    pub http_version: HttpVersion,
    pub time_out_millisec: u32,
    pub local_addr_is_ipv6: bool,
    pub access_point: HttpAccessPoint,
}

/// HTTP access point (IPv4 or IPv6)
#[repr(C)]
pub union HttpAccessPoint {
    pub ipv4_node: *mut HttpIpv4AccessPoint,
    pub ipv6_node: *mut HttpIpv6AccessPoint,
}

#[repr(C)]
pub struct HttpIpv4AccessPoint {
    pub use_default_address: bool,
    pub local_address: [u8; 4],
    pub local_subnet: [u8; 4],
    pub local_port: u16,
}

#[repr(C)]
pub struct HttpIpv6AccessPoint {
    pub local_address: [u8; 16],
    pub local_port: u16,
}

/// HTTP request data
#[repr(C)]
pub struct HttpRequestData {
    pub method: HttpMethodType,
    pub url: *const u16, // CHAR16*
}

/// HTTP response data
#[repr(C)]
pub struct HttpResponseData {
    pub status_code: HttpStatusCode,
}

/// HTTP header
#[repr(C)]
pub struct HttpHeader {
    pub field_name: *const u8,  // CHAR8*
    pub field_value: *const u8, // CHAR8*
}

/// HTTP message
#[repr(C)]
pub struct HttpMessage {
    pub data: HttpMessageData,
    pub header_count: usize,
    pub headers: *mut HttpHeader,
    pub body_length: usize,
    pub body: *mut u8,
}

#[repr(C)]
pub union HttpMessageData {
    pub request: *mut HttpRequestData,
    pub response: *mut HttpResponseData,
}

/// HTTP token for async operations
#[repr(C)]
pub struct HttpToken {
    pub event: Event,
    pub status: Status,
    pub message: *mut HttpMessage,
}

/// EFI HTTP Protocol
#[repr(C)]
pub struct HttpProtocol {
    pub get_mode_data: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        config_data: *mut HttpConfigData,
    ) -> Status,

    pub configure: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        config_data: *const HttpConfigData,
    ) -> Status,

    pub request: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        token: *mut HttpToken,
    ) -> Status,

    pub cancel: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        token: *mut HttpToken,
    ) -> Status,

    pub response: unsafe extern "efiapi" fn(
        this: *mut HttpProtocol,
        token: *mut HttpToken,
    ) -> Status,

    pub poll: unsafe extern "efiapi" fn(this: *mut HttpProtocol) -> Status,
}
