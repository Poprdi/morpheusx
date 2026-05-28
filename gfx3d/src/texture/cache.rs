use alloc::vec::Vec;

/// Quake-style surface cache: texture * lightmap pre-combined per tile,
/// keyed by (texture, lightmap, rect). LRU ring eviction.
pub struct SurfaceCache {
    entries: Vec<CacheEntry>,
    #[allow(dead_code)] // retained for cache sizing/diagnostics
    capacity: usize,
    next_slot: usize,
    generation: u32,
}

struct CacheEntry {
    key: u64,
    gen: u32,
    #[allow(dead_code)] // cached surface metadata, not yet read back
    width: u32,
    #[allow(dead_code)]
    height: u32,
    #[allow(dead_code)]
    data_offset: usize,
    #[allow(dead_code)]
    data_len: usize,
}

impl SurfaceCache {
    pub fn new(pixel_budget: usize) -> Self {
        Self {
            entries: Vec::with_capacity(512),
            capacity: pixel_budget,
            next_slot: 0,
            generation: 0,
        }
    }

    pub fn lookup(&self, key: u64) -> Option<(&[u32], u32, u32)> {
        // Linear scan; <512 entries.
        for entry in &self.entries {
            if entry.key == key && entry.gen == self.generation {
                // TODO: return arena-backed pixels.
                return None;
            }
        }
        None
    }

    pub fn insert(&mut self, key: u64, _width: u32, _height: u32, _pixels: &[u32]) {
        self.entries.push(CacheEntry {
            key,
            gen: self.generation,
            width: _width,
            height: _height,
            data_offset: 0,
            data_len: (_width * _height) as usize,
        });
    }

    /// Invalidate all entries without touching memory.
    pub fn next_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_slot = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}
