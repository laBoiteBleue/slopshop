//! Integer pixel geometry.
//!
//! Coordinates are `u32` (images up to 4 billion pixels per side); anything that can exceed
//! `u32` (edges, pixel counts) is computed in `u64` so that huge images never overflow.

/// A size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Number of pixels. `u64` because 100k × 100k images exist.
    pub const fn pixel_count(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// The rectangle covering the whole size, anchored at the origin.
    pub const fn bounds(self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }
}

/// An axis-aligned rectangle of pixels: `[x, x + width) × [y, y + height)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    pub const fn is_empty(self) -> bool {
        self.size().is_empty()
    }

    /// Exclusive right edge.
    pub const fn right(self) -> u64 {
        self.x as u64 + self.width as u64
    }

    /// Exclusive bottom edge.
    pub const fn bottom(self) -> u64 {
        self.y as u64 + self.height as u64
    }

    /// The overlapping area, or `None` if the rectangles do not overlap.
    pub fn intersection(self, other: Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= u64::from(x) || bottom <= u64::from(y) {
            return None;
        }
        // Both extents are bounded by an input width/height, so they fit in u32.
        Some(Rect::new(
            x,
            y,
            (right - u64::from(x)) as u32,
            (bottom - u64::from(y)) as u32,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_count_does_not_overflow() {
        let huge = Size::new(u32::MAX, u32::MAX);
        assert_eq!(
            huge.pixel_count(),
            u64::from(u32::MAX) * u64::from(u32::MAX)
        );
    }

    #[test]
    fn edges_do_not_overflow() {
        let r = Rect::new(u32::MAX, u32::MAX, u32::MAX, 1);
        assert_eq!(r.right(), 2 * u64::from(u32::MAX));
        assert_eq!(r.bottom(), u64::from(u32::MAX) + 1);
    }

    #[test]
    fn intersection_overlapping() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 8, 10, 10);
        assert_eq!(a.intersection(b), Some(Rect::new(5, 8, 5, 2)));
        assert_eq!(b.intersection(a), a.intersection(b));
    }

    #[test]
    fn intersection_touching_or_disjoint_is_none() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersection(Rect::new(10, 0, 5, 5)), None);
        assert_eq!(a.intersection(Rect::new(20, 20, 5, 5)), None);
        assert_eq!(a.intersection(Rect::new(2, 2, 0, 5)), None);
    }

    #[test]
    fn intersection_contained() {
        let outer = Rect::new(0, 0, 100, 100);
        let inner = Rect::new(10, 20, 30, 40);
        assert_eq!(outer.intersection(inner), Some(inner));
    }
}
