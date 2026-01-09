//! ASM result and error code types.
//!
//! # Reference
//! ARCHITECTURE_V3.md

/// Result from ASM function calls.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsmResult {
    /// Success.
    Ok = 0,
    /// Queue is full.
    QueueFull = 1,
    /// Operation timed out.
    Timeout = 2,
    /// Invalid parameter.
    InvalidParam = 3,
    /// Device error.
    DeviceError = 4,
    /// Feature not supported.
    NotSupported = 5,
    /// Device not ready.
    NotReady = 6,
}

impl AsmResult {
    /// Check if result is success.
    pub fn is_ok(&self) -> bool {
        matches!(self, AsmResult::Ok)
    }
    
    /// Check if result is error.
    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }
    
    /// Convert from raw u32.
    pub fn from_u32(val: u32) -> Self {
        match val {
            0 => AsmResult::Ok,
            1 => AsmResult::QueueFull,
            2 => AsmResult::Timeout,
            3 => AsmResult::InvalidParam,
            4 => AsmResult::DeviceError,
            5 => AsmResult::NotSupported,
            6 => AsmResult::NotReady,
            _ => AsmResult::DeviceError,
        }
    }
}

impl From<u32> for AsmResult {
    fn from(val: u32) -> Self {
        Self::from_u32(val)
    }
}

impl Default for AsmResult {
    fn default() -> Self {
        AsmResult::Ok
    }
}
