//! Supplementary Volume Descriptor (Joliet support)
//!
//! The Supplementary VD enables Joliet extensions for long Unicode filenames.

/// Supplementary Volume Descriptor (type 2)
///
/// Same structure as Primary VD but uses UCS-2 encoding for strings
pub struct SupplementaryVolumeDescriptor {
    // TODO: Same fields as PrimaryVolumeDescriptor
    // but escape sequences in unused3 field indicate Joliet
}

/// Check if supplementary descriptor is Joliet
pub fn is_joliet(_data: &[u8]) -> bool {
    // TODO: Check escape sequences at offset 88:
    // %/@, %/C, or %/E indicate Joliet Level 1/2/3
    false
}
