//! Dirty-region bookkeeping for the coalescing flusher.
//!
//! The backend keeps the full decoded image; what travels to the UI is only the part that
//! changed since the last flush. Many small updates (a typing cursor, a progress bar) are
//! merged so that one flush sends a handful of rectangles instead of one message per PDU.
//!
//! This is a copy of `portolano-session-rdp`'s `dirty.rs`, kept as a copy so that this crate
//! does not depend on the RDP stack.

/// Merge candidates whose union is at most this much bigger than the sum of the two areas
/// are merged even when they do not touch (numerator / denominator).
const MERGE_SLACK: (u64, u64) = (3, 2);
/// Above this many rectangles everything collapses into the bounding box.
const MAX_RECTS: usize = 24;
/// When the dirty area reaches this share of the image, one full-image rectangle is cheaper.
const FULL_THRESHOLD: (u64, u64) = (1, 2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    fn right(&self) -> u32 {
        self.x + self.w
    }

    fn bottom(&self) -> u32 {
        self.y + self.h
    }

    pub fn area(&self) -> u64 {
        u64::from(self.w) * u64::from(self.h)
    }

    fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            w: self.right().max(other.right()) - x,
            h: self.bottom().max(other.bottom()) - y,
        }
    }

    /// Overlapping or touching.
    fn touches(&self, other: &Rect) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    fn should_merge(&self, other: &Rect) -> bool {
        if self.touches(other) {
            return true;
        }
        let union = self.union(other).area();
        union * MERGE_SLACK.1 <= (self.area() + other.area()) * MERGE_SLACK.0
    }
}

#[derive(Debug, Default)]
pub struct DirtyRegion {
    rects: Vec<Rect>,
    full: bool,
}

impl DirtyRegion {
    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
        self.full = false;
    }

    /// Marks the whole image as dirty.
    pub fn mark_full(&mut self) {
        self.rects.clear();
        self.full = true;
    }

    pub fn add(&mut self, rect: Rect) {
        if self.full || rect.w == 0 || rect.h == 0 {
            return;
        }
        let mut current = rect;
        while let Some(index) = self.rects.iter().position(|r| r.should_merge(&current)) {
            current = self.rects.swap_remove(index).union(&current);
        }
        self.rects.push(current);
        if self.rects.len() > MAX_RECTS {
            let bounds = self
                .rects
                .iter()
                .skip(1)
                .fold(self.rects[0], |a, r| a.union(r));
            self.rects.clear();
            self.rects.push(bounds);
        }
    }

    /// Drains the region as rectangles clipped to a `width` x `height` image. A full mark, or
    /// a dirty area above the threshold, yields the single full-image rectangle.
    pub fn take(&mut self, width: u16, height: u16) -> Vec<Rect> {
        let image = Rect::new(0, 0, u32::from(width), u32::from(height));
        let full = std::mem::take(&mut self.full);
        let rects = std::mem::take(&mut self.rects);
        if image.area() == 0 {
            return Vec::new();
        }
        if full {
            return vec![image];
        }
        let clipped: Vec<Rect> = rects.into_iter().filter_map(|r| clip(r, &image)).collect();
        let dirty: u64 = clipped.iter().map(Rect::area).sum();
        if dirty * FULL_THRESHOLD.1 >= image.area() * FULL_THRESHOLD.0 {
            return vec![image];
        }
        clipped
    }
}

fn clip(rect: Rect, image: &Rect) -> Option<Rect> {
    let x = rect.x.min(image.w);
    let y = rect.y.min(image.h);
    let right = rect.right().min(image.w);
    let bottom = rect.bottom().min(image.h);
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_and_touching_rects_merge() {
        let mut d = DirtyRegion::default();
        d.add(Rect::new(0, 0, 10, 10));
        d.add(Rect::new(5, 5, 10, 10));
        d.add(Rect::new(15, 0, 5, 5));
        assert_eq!(d.take(1000, 1000), vec![Rect::new(0, 0, 20, 15)]);
        assert!(d.is_empty());
    }

    #[test]
    fn distant_small_rects_stay_separate() {
        let mut d = DirtyRegion::default();
        d.add(Rect::new(0, 0, 10, 10));
        d.add(Rect::new(900, 900, 10, 10));
        let rects = d.take(1000, 1000);
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn too_many_rects_collapse_to_the_bounding_box() {
        let mut d = DirtyRegion::default();
        for i in 0..40 {
            d.add(Rect::new(i * 100, (i % 2) * 400, 2, 2));
        }
        let rects = d.take(5000, 1000);
        assert!(rects.len() <= MAX_RECTS);
        assert!(rects.iter().all(|r| r.right() <= 5000));
    }

    #[test]
    fn large_dirty_area_becomes_one_full_rect() {
        let mut d = DirtyRegion::default();
        d.add(Rect::new(0, 0, 100, 60));
        assert_eq!(d.take(100, 100), vec![Rect::new(0, 0, 100, 100)]);
    }

    #[test]
    fn full_mark_wins_and_clips_to_the_image() {
        let mut d = DirtyRegion::default();
        d.mark_full();
        d.add(Rect::new(1, 1, 2, 2));
        assert_eq!(d.take(64, 48), vec![Rect::new(0, 0, 64, 48)]);

        d.add(Rect::new(60, 40, 100, 100));
        assert_eq!(d.take(640, 480), vec![Rect::new(60, 40, 100, 100)]);
        d.add(Rect::new(700, 0, 10, 10));
        assert!(d.take(640, 480).is_empty());
    }
}
