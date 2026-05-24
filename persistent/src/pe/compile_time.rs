//! Linker-supplied ImageBase, needed to compute the relocation delta after
//! UEFI patches the PE header to the actual load address.

/// Linker ImageBase, overridable via `MORPHEUS_IMAGE_BASE`. Typical UEFI x64
/// uses 0x140000000.
pub const LINKER_IMAGE_BASE_STR: Option<&str> = option_env!("MORPHEUS_IMAGE_BASE");

pub fn get_original_image_base_hint() -> Option<u64> {
    LINKER_IMAGE_BASE_STR.and_then(|s| {
        let s = s.trim_start_matches("0x").trim_start_matches("0X");
        u64::from_str_radix(s, 16).ok()
    })
}
