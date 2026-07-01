//! Directory-entry decoding: 8.3 short names and LFN reassembly.

use crate::types::*;
use alloc::string::String;
use alloc::vec::Vec;

/// A raw 32-byte short-name (8.3) directory entry, already classified.
pub struct ShortEntry {
    pub name: String,
    pub attr: u8,
    pub size: u32,
    pub start_cluster: u32,
}

impl ShortEntry {
    pub fn is_dir(&self) -> bool {
        self.attr & ATTR_DIRECTORY != 0
    }

    /// 8.3 → "NAME.EXT" lowercased. The case bits at 0x0C (NT reserved) carry
    /// the original case for base/ext; honor them so `Foo.Txt` round-trips.
    fn decode_short(raw: &[u8]) -> String {
        let lower_base = raw[12] & 0x08 != 0;
        let lower_ext = raw[12] & 0x10 != 0;

        let mut out = String::new();
        for &c in &raw[0..8] {
            if c == b' ' {
                break;
            }
            let c = if lower_base {
                c.to_ascii_lowercase()
            } else {
                c
            };
            out.push(c as char);
        }
        let mut ext = String::new();
        for &c in &raw[8..11] {
            if c == b' ' {
                break;
            }
            let c = if lower_ext { c.to_ascii_lowercase() } else { c };
            ext.push(c as char);
        }
        if !ext.is_empty() {
            out.push('.');
            out.push_str(&ext);
        }
        out
    }
}

/// Decode the 13 UTF-16 units carried by one LFN slot into `dst`.
fn lfn_units(raw: &[u8], dst: &mut Vec<u16>) {
    // Slot byte ranges holding name chars: 1..11, 14..26, 28..32.
    const RANGES: [(usize, usize); 3] = [(1, 11), (14, 26), (28, 32)];
    for (start, end) in RANGES {
        let mut i = start;
        while i + 1 < end {
            dst.push(u16::from_le_bytes([raw[i], raw[i + 1]]));
            i += 2;
        }
    }
}

/// Walk a directory blob (concatenated cluster data) and yield resolved
/// entries, skipping `.`/`..`, volume labels, and free/deleted slots.
pub fn parse_entries(blob: &[u8]) -> Vec<DirEntry> {
    let mut out = Vec::new();
    let mut lfn_rev: Vec<u16> = Vec::new(); // collected most-significant-slot-first

    let mut off = 0;
    while off + DIR_ENTRY_SIZE <= blob.len() {
        let raw = &blob[off..off + DIR_ENTRY_SIZE];
        off += DIR_ENTRY_SIZE;

        let first = raw[0];
        if first == ENTRY_END {
            break;
        }
        if first == ENTRY_FREE {
            lfn_rev.clear();
            continue;
        }

        let attr = raw[11];
        if attr & ATTR_LONG_NAME == ATTR_LONG_NAME {
            // LFN slots precede their short entry, ordered last-slot-first on disk.
            let mut units = Vec::new();
            lfn_units(raw, &mut units);
            // Prepend so final order is logical (low ordinals first).
            let mut merged = units;
            merged.extend_from_slice(&lfn_rev);
            lfn_rev = merged;
            continue;
        }

        if attr & ATTR_VOLUME_ID != 0 {
            lfn_rev.clear();
            continue;
        }

        let short = ShortEntry {
            name: ShortEntry::decode_short(raw),
            attr,
            size: u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]),
            start_cluster: ((u16::from_le_bytes([raw[20], raw[21]]) as u32) << 16)
                | (u16::from_le_bytes([raw[26], raw[27]]) as u32),
        };

        let name = if lfn_rev.is_empty() {
            short.name.clone()
        } else {
            decode_lfn(&lfn_rev).unwrap_or_else(|| short.name.clone())
        };
        lfn_rev.clear();

        if name == "." || name == ".." {
            continue;
        }

        out.push(DirEntry {
            name,
            file_type: if short.is_dir() {
                FileType::Directory
            } else {
                FileType::Regular
            },
            size: short.size as u64,
            start_cluster: short.start_cluster,
        });
    }
    out
}

/// UTF-16 LFN → String, trimming the 0x0000 terminator and 0xFFFF padding.
fn decode_lfn(units: &[u16]) -> Option<String> {
    let end = units
        .iter()
        .position(|&u| u == 0x0000)
        .unwrap_or(units.len());
    let trimmed: Vec<u16> = units[..end]
        .iter()
        .copied()
        .filter(|&u| u != 0xFFFF)
        .collect();
    if trimmed.is_empty() {
        return None;
    }
    String::from_utf16(&trimmed).ok()
}
