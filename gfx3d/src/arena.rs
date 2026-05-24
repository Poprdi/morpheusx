use alloc::vec::Vec;

/// Frame-scoped bump allocator. Reset between frames; allocations freed in O(1).
pub struct Arena {
    data: Vec<u8>,
    offset: usize,
}

impl Arena {
    pub fn new(capacity: usize) -> Self {
        Self {
            data: alloc::vec![0u8; capacity],
            offset: 0,
        }
    }

    /// Returns None on overflow; callers skip the offending triangle/mesh.
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

        // SAFETY: region is exclusive, aligned, and inside the backing Vec.
        let slice = unsafe { core::slice::from_raw_parts_mut(ptr, count) };

        for item in slice.iter_mut() {
            *item = T::default();
        }

        Some(slice)
    }

    #[inline]
    pub fn reset(&mut self) {
        self.offset = 0;
    }

    pub fn used(&self) -> usize {
        self.offset
    }

    pub fn capacity(&self) -> usize {
        self.data.len()
    }

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
        let _: &mut [u8] = arena.alloc_slice(1).unwrap();
        let aligned: &mut [u32] = arena.alloc_slice(1).unwrap();
        let ptr = aligned.as_ptr() as usize;
        assert_eq!(ptr % 4, 0);
    }
}
