use alloc::vec::Vec;

/// Frame-scoped bump allocator for zero-overhead temporary storage.
///
/// Every frame, the 3D pipeline needs temporary memory for:
/// - Transformed vertices (one per visible mesh vertex)
/// - Clipped polygon output (up to 9 vertices per triangle)
/// - Span arrays (one per scanline per triangle)
/// - Sorted face lists
///
/// Using Vec::push() for these would cause hundreds of tiny heap allocations
/// per frame, hammering the global allocator and fragmenting memory.
///
/// The arena allocates one big chunk at startup and bumps a pointer forward
/// for each allocation. At frame end, reset the pointer to zero — all
/// "allocations" are freed in O(1) with no bookkeeping.
///
/// This is the same pattern used by:
/// - Quake 3's Hunk allocator (per-frame temp memory)
/// - Unreal Engine's FMemStack
/// - DOOM's Z_Malloc with PU_LEVEL tag
pub struct Arena {
    data: Vec<u8>,
    offset: usize,
}

impl Arena {
    /// Pre-allocate `capacity` bytes.
    pub fn new(capacity: usize) -> Self {
        Self {
            data: alloc::vec![0u8; capacity],
            offset: 0,
        }
    }

    /// Allocate `count` elements of type T from the arena.
    ///
    /// Returns None if the arena is full (caller should handle gracefully
    /// by skipping that triangle/mesh rather than panicking).
    pub fn alloc_slice<T: Copy + Default>(&mut self, count: usize) -> Option<&mut [T]> {
        let align = core::mem::align_of::<T>();
        let aligned_offset = (self.offset + align - 1) & !(align - 1);
        let byte_size = count * core::mem::size_of::<T>();
        let new_offset = aligned_offset + byte_size;

        if new_offset > self.data.len() {
            return None;
        }

        let ptr = self.data[aligned_offset..].as_mut_ptr() as *mut T;
        self.offset = new_offset;

        // Safety: we have exclusive access to this region, it's properly aligned,
        // and the memory is within our Vec's allocation.
        let slice = unsafe { core::slice::from_raw_parts_mut(ptr, count) };

        // Zero-initialize (Default)
        for item in slice.iter_mut() {
            *item = T::default();
        }

        Some(slice)
    }

    /// Reset the arena for the next frame. O(1), no deallocation.
    #[inline]
    pub fn reset(&mut self) {
        self.offset = 0;
    }

    /// How many bytes are currently in use.
    pub fn used(&self) -> usize {
        self.offset
    }

    /// Total capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    /// Remaining bytes available.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_alloc_and_reset() {
        let mut arena = Arena::new(1024);
        let slice: &mut [u32] = arena.alloc_slice(10).unwrap();
        assert_eq!(slice.len(), 10);
        slice[0] = 42;
        assert_eq!(slice[0], 42);
        assert!(arena.used() >= 40);

        arena.reset();
        assert_eq!(arena.used(), 0);
    }

    #[test]
    fn overflow_returns_none() {
        let mut arena = Arena::new(32);
        let result: Option<&mut [u64]> = arena.alloc_slice(100);
        assert!(result.is_none());
    }

    #[test]
    fn alignment() {
        let mut arena = Arena::new(256);
        let _: &mut [u8] = arena.alloc_slice(1).unwrap(); // offset = 1
        let aligned: &mut [u32] = arena.alloc_slice(1).unwrap();
        let ptr = aligned.as_ptr() as usize;
        assert_eq!(ptr % 4, 0); // u32 should be 4-byte aligned
    }
}
