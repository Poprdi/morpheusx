//! UEFI protocol management implementation.
//!
//! Provides:
//! - `bindings` - UEFI HTTP protocol type definitions
//! - `ProtocolManager` - Protocol lifecycle management

pub mod bindings;
pub mod manager;

pub use manager::{ProtocolManager, ManagerState};
pub use bindings::{
    Guid, Handle, Status, Event,
    HttpProtocol, ServiceBindingProtocol,
    HttpVersion, HttpMethod, HttpStatusCode,
    HttpConfigBuilder, HttpIpv4AccessPoint, HttpIpv6AccessPoint,
    HTTP_PROTOCOL_GUID, HTTP_SERVICE_BINDING_GUID,
    status,
};
