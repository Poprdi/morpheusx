use alloc::vec::Vec;

/// Quake-style surface cache: texture * lightmap pre-combined per tile,
/// keyed by (texture, lightmap, rect). LRU ring eviction.
#[allow(dead_code)]
pub struct SurfaceCache {
    entries: Vec<CacheEntry>,
    capacity: usize,
    next_slot: usize,
    generation: u32,
}

#[allow(dead_code)]
struct CacheEntry {
    key: u64,
    gen: u32,
    width: u32,
    height: u32,
    data_offset: usize,
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
