//! UEFI Service Binding Protocol
//!
//! Used to create and destroy child protocol instances

/// EFI Handle type
pub type Handle = *mut core::ffi::c_void;

/// EFI Status type
pub type Status = usize;

/// Service Binding Protocol function pointers
#[repr(C)]
pub struct ServiceBindingProtocol {
    pub create_child: unsafe extern "efiapi" fn(
        this: *mut ServiceBindingProtocol,
        child_handle: *mut Handle,
    ) -> Status,

    pub destroy_child: unsafe extern "efiapi" fn(
        this: *mut ServiceBindingProtocol,
        child_handle: Handle,
    ) -> Status,
}
