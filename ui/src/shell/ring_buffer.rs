use alloc::vec::Vec;

pub struct RingBuffer<T> {
    buf: Vec<T>,
    capacity: usize,
    head: usize,
    len: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
            capacity,
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, item: T) {
        if self.buf.len() < self.capacity {
            self.buf.push(item);
            self.len = self.buf.len();
        } else {
            self.buf[self.head] = item;
            self.head = (self.head + 1) % self.capacity;
            self.len = self.capacity;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        if self.buf.len() < self.capacity {
            self.buf.get(index)
        } else {
            let real = (self.head + index) % self.capacity;
            self.buf.get(real)
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.head = 0;
        self.len = 0;
    }

    pub fn iter(&self) -> RingIter<'_, T> {
        RingIter { ring: self, pos: 0 }
    }
}

pub struct RingIter<'a, T> {
    ring: &'a RingBuffer<T>,
    pos: usize,
}

impl<'a, T> Iterator for RingIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.ring.len() {
            return None;
        }
        let item = self.ring.get(self.pos);
        self.pos += 1;
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.ring.len() - self.pos;
        (remaining, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;

    #[test]
    fn push_within_capacity() {
        let mut rb: RingBuffer<u32> = RingBuffer::new(4);
        rb.push(1);
        rb.push(2);
        rb.push(3);
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.get(0), Some(&1));
        assert_eq!(rb.get(2), Some(&3));
    }

    #[test]
    fn push_wraps_around() {
        let mut rb: RingBuffer<u32> = RingBuffer::new(3);
        rb.push(1);
        rb.push(2);
        rb.push(3);
        rb.push(4);
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.get(0), Some(&2));
        assert_eq!(rb.get(1), Some(&3));
        assert_eq!(rb.get(2), Some(&4));
    }

    #[test]
    fn iter_works() {
        let mut rb: RingBuffer<u32> = RingBuffer::new(3);
        rb.push(10);
        rb.push(20);
        rb.push(30);
        rb.push(40);
        let vals: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(vals, &[20, 30, 40]);
    }
}
