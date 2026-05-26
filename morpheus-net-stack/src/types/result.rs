//! ABI return codes for `extern "win64"` ASM calls.

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AsmResult {
    #[default]
    Ok = 0,
    QueueFull = 1,
    Timeout = 2,
    InvalidParam = 3,
    DeviceError = 4,
    NotSupported = 5,
    NotReady = 6,
}

impl AsmResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, AsmResult::Ok)
    }

    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }

    /// Unknown values fold to `DeviceError`.
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
