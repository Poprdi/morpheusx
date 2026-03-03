use alloc::vec::Vec;

/// Potentially Visible Set — pre-computed inter-leaf visibility.
///
/// This is the SINGLE BIGGEST optimization in any indoor 3D engine.
/// Without PVS, you render every triangle in the map every frame.
/// With PVS, you render only the ~5-15% that are POSSIBLY visible.
///
/// How it works:
/// 1. At map compile time, for each BSP leaf (cluster), sample visibility
///    by casting rays from that leaf to every other leaf.
/// 2. Store results as a compressed bitfield: bit N = "can cluster N be
///    seen from this cluster?"
/// 3. At runtime: find camera's cluster, decompress its PVS row, iterate
///    only the set bits to get the list of visible clusters.
///
/// Quake's PVS used run-length encoding (RLE) of the bitfield — a 1024-cluster
/// map needs 128 bytes/row uncompressed, but most rows are 90%+ zeros, so
/// RLE compresses to ~10-30 bytes/row.
///
/// Our implementation stores the full bitfield (no RLE) because memory is
/// cheap and decompression latency isn't worth saving a few KB on modern
/// hardware. For a 256-cluster map, that's 256 × 32 bytes = 8KB total.

pub struct PvsTable {
    num_clusters: u32,
    bytes_per_row: u32,
    /// Packed visibility bits: row[cluster] = bytes_per_row bytes.
    data: Vec<u8>,
}

impl PvsTable {
    /// Create an empty PVS (all clusters visible from all clusters).
    pub fn new_all_visible(num_clusters: u32) -> Self {
        let bytes_per_row = num_clusters.div_ceil(8);
        let total = (num_clusters * bytes_per_row) as usize;
        Self {
            num_clusters,
            bytes_per_row,
            data: alloc::vec![0xFF; total], // all bits set = all visible
        }
    }

    /// Create an empty PVS (nothing visible — must be filled in).
    pub fn new_empty(num_clusters: u32) -> Self {
        let bytes_per_row = num_clusters.div_ceil(8);
        let total = (num_clusters * bytes_per_row) as usize;
        Self {
            num_clusters,
            bytes_per_row,
            data: alloc::vec![0; total],
        }
    }

    /// Mark cluster `to` as visible from cluster `from`.
    pub fn set_visible(&mut self, from: u32, to: u32) {
        if from >= self.num_clusters || to >= self.num_clusters {
            return;
        }
        let byte_idx = (from * self.bytes_per_row + to / 8) as usize;
        let bit = 1u8 << (to & 7);
        if let Some(b) = self.data.get_mut(byte_idx) {
            *b |= bit;
        }
    }

    /// Check if cluster `to` is visible from cluster `from`.
    #[inline]
    pub fn is_visible(&self, from: u32, to: u32) -> bool {
        if from >= self.num_clusters || to >= self.num_clusters {
            return false;
        }
        let byte_idx = (from * self.bytes_per_row + to / 8) as usize;
        let bit = 1u8 << (to & 7);
        match self.data.get(byte_idx) {
            Some(&b) => b & bit != 0,
            None => false,
        }
    }

    /// Iterate all clusters visible from `from_cluster`.
    ///
    /// Returns an iterator of cluster indices. Uses bit-scan (trailing_zeros)
    /// to skip zero bytes efficiently — a 256-cluster PVS with 20 visible
    /// clusters processes ~6 bytes instead of 32.
    pub fn visible_from(&self, from_cluster: u32) -> PvsIterator<'_> {
        let start = (from_cluster * self.bytes_per_row) as usize;
        let end = start + self.bytes_per_row as usize;
        PvsIterator {
            data: &self.data,
            start,
            end: end.min(self.data.len()),
            byte_idx: start,
            current_byte: 0,
            bit_offset: 0,
            base_cluster: 0,
            initialized: false,
        }
    }

    pub fn num_clusters(&self) -> u32 {
        self.num_clusters
    }
}

pub struct PvsIterator<'a> {
    data: &'a [u8],
    start: usize,
    end: usize,
    byte_idx: usize,
    current_byte: u8,
    bit_offset: u32,
    base_cluster: u32,
    initialized: bool,
}

impl<'a> Iterator for PvsIterator<'a> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if !self.initialized {
            self.initialized = true;
            self.byte_idx = self.start;
            if self.byte_idx < self.end {
                self.current_byte = self.data[self.byte_idx];
            }
            self.bit_offset = 0;
            self.base_cluster = 0;
        }

        loop {
            // Skip zero bytes entirely (the fast path)
            while self.current_byte == 0 {
                self.byte_idx += 1;
                self.base_cluster += 8 - self.bit_offset;
                self.bit_offset = 0;
                if self.byte_idx >= self.end {
                    return None;
                }
                self.current_byte = self.data[self.byte_idx];
            }

            // Find lowest set bit
            let tz = self.current_byte.trailing_zeros();
            if tz >= 8 - self.bit_offset {
                // Shouldn't happen, but guard
                self.byte_idx += 1;
                self.base_cluster += 8 - self.bit_offset;
                self.bit_offset = 0;
                if self.byte_idx >= self.end {
                    return None;
                }
                self.current_byte = self.data[self.byte_idx];
                continue;
            }

            let cluster = self.base_cluster + tz;
            // Clear this bit
            self.current_byte &= !(1 << tz);
            return Some(cluster);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_visible() {
        let pvs = PvsTable::new_all_visible(16);
        for i in 0..16 {
            for j in 0..16 {
                assert!(pvs.is_visible(i, j));
            }
        }
    }

    #[test]
    fn selective_visibility() {
        let mut pvs = PvsTable::new_empty(32);
        pvs.set_visible(0, 5);
        pvs.set_visible(0, 10);
        pvs.set_visible(0, 31);

        assert!(pvs.is_visible(0, 5));
        assert!(pvs.is_visible(0, 10));
        assert!(pvs.is_visible(0, 31));
        assert!(!pvs.is_visible(0, 6));
        assert!(!pvs.is_visible(0, 0));
        assert!(!pvs.is_visible(1, 5)); // different source
    }

    #[test]
    fn iterator_correctness() {
        let mut pvs = PvsTable::new_empty(64);
        pvs.set_visible(0, 3);
        pvs.set_visible(0, 17);
        pvs.set_visible(0, 63);

        let visible: Vec<u32> = pvs.visible_from(0).collect();
        assert_eq!(visible, alloc::vec![3, 17, 63]);
    }
}
