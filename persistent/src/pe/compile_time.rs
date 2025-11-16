//! Compile-time PE constants
//!
//! These values are extracted from the linker at build time
//! to avoid runtime heuristics.

/// Original ImageBase from linker script
///
/// This is the ImageBase value that the linker embedded in the PE file.
/// At runtime, UEFI loads the image at a different address and patches
/// this field. We need the original value to calculate the relocation delta.
///
/// This constant is set at build time by the build script or can be
/// overridden via environment variable MORPHEUS_IMAGE_BASE.
///
/// Common values:
/// - 0x0000000140000000 (typical UEFI x64)
/// - 0x0000000000400000 (classic Windows)
/// - 0x0000000100000000 (alternative)
/// Original ImageBase from linker script
///
/// This is the ImageBase value that the linker embedded in the PE file.
/// At runtime, UEFI loads the image at a different address and patches
/// this field. We need the original value to calculate the relocation delta.
///
/// This constant is set at build time by the build script or can be
/// overridden via environment variable MORPHEUS_IMAGE_BASE.
///
/// Common values:
/// - 0x0000000140000000 (typical UEFI x64)
/// - 0x0000000000400000 (classic Windows)
/// - 0x0000000100000000 (alternative)
///
/// Note: Compile-time parsing not available yet, use runtime helper instead
pub const LINKER_IMAGE_BASE_STR: Option<&str> = option_env!("MORPHEUS_IMAGE_BASE");

/// Get the original ImageBase, preferring compile-time constant
///
/// Falls back to heuristic algorithm if LINKER_IMAGE_BASE not set.
pub fn get_original_image_base_hint() -> Option<u64> {
    LINKER_IMAGE_BASE_STR.and_then(|s| {
        let s = s.trim_start_matches("0x").trim_start_matches("0X");
        u64::from_str_radix(s, 16).ok()
    })
}
