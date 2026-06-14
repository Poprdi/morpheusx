//! Generational arena. Each slot carries a generation counter; a handle is
//! `foundation::storage::pack(index, generation)`. Removing a slot bumps its
//! generation, so a handle to a freed (and possibly reused) slot fails the
//! generation check — turning a use-after-free on a fuzzed id into a clean
//! `None` (→ `ENODEV`) instead of an alias. Spec §3 capacity decision.

use alloc::vec::Vec;
use morpheus_foundation::storage::{pack, unpack};

struct Slot<T> {
    /// Even = empty, odd = occupied. The low bit doubles as the live flag, so a
    /// freed slot's next allocation lands on a different generation than any
    /// handle minted before the free.
    generation: u32,
    value: Option<T>,
}

/// Stable-id generational arena with arbitrary growth. Not `Copy`.
pub struct Slab<T> {
    slots: Vec<Slot<T>>,
    /// Indices of empty slots, reused before growing.
    free: Vec<u32>,
    len: usize,
}

impl<T> Slab<T> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Insert `value`, returning its stable handle, or `None` if the index space
    /// is exhausted (>= u32::MAX slots — a DoS bound, not an array size).
    pub fn insert(&mut self, value: T) -> Option<u64> {
        if let Some(index) = self.free.pop() {
            let slot = self.slots.get_mut(index as usize)?;
            slot.generation |= 1; // mark occupied
            slot.value = Some(value);
            self.len += 1;
            return Some(pack(index, slot.generation));
        }
        let index_usize = self.slots.len();
        if index_usize >= u32::MAX as usize {
            return None;
        }
        let index = index_usize as u32;
        self.slots.push(Slot {
            generation: 1,
            value: Some(value),
        });
        self.len += 1;
        Some(pack(index, 1))
    }

    /// Live slot only if both the index is in range and the generation matches.
    fn resolve(&self, handle: u64) -> Option<u32> {
        let (index, generation) = unpack(handle);
        let slot = self.slots.get(index as usize)?;
        if slot.generation == generation && slot.value.is_some() {
            Some(index)
        } else {
            None
        }
    }

    pub fn get(&self, handle: u64) -> Option<&T> {
        let index = self.resolve(handle)?;
        self.slots.get(index as usize)?.value.as_ref()
    }

    pub fn get_mut(&mut self, handle: u64) -> Option<&mut T> {
        let index = self.resolve(handle)?;
        self.slots.get_mut(index as usize)?.value.as_mut()
    }

    /// Remove and return the value; stale handles return `None`. Bumps the
    /// generation so the freed slot can never be aliased by the old handle.
    pub fn remove(&mut self, handle: u64) -> Option<T> {
        let index = self.resolve(handle)?;
        let slot = self.slots.get_mut(index as usize)?;
        let value = slot.value.take();
        // Wrapping is fine: a collision needs 2^31 reuses of one slot.
        slot.generation = slot.generation.wrapping_add(1);
        if value.is_some() {
            self.len -= 1;
            self.free.push(index);
        }
        value
    }

    /// Iterate live `(handle, &T)` pairs. Order is slot order, not insertion.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            slot.value
                .as_ref()
                .map(|v| (pack(i as u32, slot.generation), v))
        })
    }

    /// Mutable variant of [`iter`](Self::iter).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u64, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, slot)| {
            let gen = slot.generation;
            slot.value.as_mut().map(|v| (pack(i as u32, gen), v))
        })
    }
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}
