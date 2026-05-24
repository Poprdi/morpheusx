//! Supplementary VD (type 2). Joliet uses UCS-2 strings and escape sequences.

/// Placeholder; fields mirror the PVD when implemented.
pub struct SupplementaryVolumeDescriptor {}

/// Detect Joliet via the escape sequence field at offset 88
/// (`%/@`, `%/C`, `%/E` for Level 1/2/3). Currently a stub.
pub fn is_joliet(_data: &[u8]) -> bool {
    false
}
