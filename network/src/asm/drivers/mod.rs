//! Driver-specific ASM bindings.
//!
//! Each driver has its own module with bindings to that driver's ASM functions.

pub mod virtio;

// Future drivers - feature-gated
// #[cfg(feature = "intel")]
// pub mod intel;
// #[cfg(feature = "realtek")]
// pub mod realtek;
// #[cfg(feature = "broadcom")]
// pub mod broadcom;
