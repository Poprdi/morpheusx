#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[inline]
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    #[inline]
    pub const fn zero() -> Self {
        Self { x: 0, y: 0, w: 0, h: 0 }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    #[inline]
    pub const fn right(self) -> u32 {
        self.x.saturating_add(self.w)
    }

    #[inline]
    pub const fn bottom(self) -> u32 {
        self.y.saturating_add(self.h)
    }

    #[inline]
    pub const fn contains(self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    #[inline]
    #[must_use]
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());

        if x0 >= x1 || y0 >= y1 {
            None
        } else {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
    }

    #[inline]
    #[must_use]
    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self.right().max(other.right());
        let y1 = self.bottom().max(other.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    #[inline]
    pub const fn area(self) -> u32 {
        self.w.saturating_mul(self.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_overlap() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        let i = a.intersect(b).unwrap();
        assert_eq!(i, Rect::new(5, 5, 5, 5));
    }

    #[test]
    fn intersect_no_overlap() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(10, 10, 5, 5);
        assert!(a.intersect(b).is_none());
    }

    #[test]
    fn union_rects() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(3, 3, 5, 5);
        let u = a.union(b);
        assert_eq!(u, Rect::new(0, 0, 8, 8));
    }

    #[test]
    fn contains_point() {
        let r = Rect::new(10, 10, 20, 20);
        assert!(r.contains(10, 10));
        assert!(r.contains(29, 29));
        assert!(!r.contains(30, 30));
        assert!(!r.contains(9, 10));
    }

    #[test]
    fn empty_rect() {
        assert!(Rect::new(0, 0, 0, 5).is_empty());
        assert!(Rect::new(0, 0, 5, 0).is_empty());
        assert!(!Rect::new(0, 0, 1, 1).is_empty());
    }
}
