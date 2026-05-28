//! HAL global. Bootloader installs once via `install_hal`; rest of kernel reads through `hal()`.

use morpheus_hal_api::Hal;
use spin::Once;

static HAL_INSTANCE: Once<&'static dyn Hal> = Once::new();

/// Once-only. Subsequent calls are silently ignored by `Once::call_once`.
pub fn install_hal(hal: &'static dyn Hal) {
    HAL_INSTANCE.call_once(|| hal);
}

/// Panics if a kernel module ran before bootloader handoff (logic bug).
pub fn hal() -> &'static dyn Hal {
    HAL_INSTANCE
        .get()
        .copied()
        .expect("HAL not initialized — kernel ran before install_hal")
}
