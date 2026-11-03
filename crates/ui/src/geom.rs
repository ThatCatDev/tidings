//! Logical-pixel geometry shared by layout, hit-testing and painting.
//!
//! All layout happens in logical (surface-local) pixels as `f32`. The painter
//! multiplies by the output scale; the helpers here centralise the float/int
//! conversions so the casts (and their justification) live in one place.

/// A point in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    #[must_use]
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    #[must_use]
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Half-open containment: the right/bottom edges are outside.
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    #[must_use]
    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Grows the rect by `d` on every side (shrinks for negative `d`).
    #[must_use]
    pub fn outset(&self, d: f32) -> Self {
        Self::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }

    #[must_use]
    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        (r > x && b > y).then(|| Self::new(x, y, r - x, b - y))
    }

    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    /// Converts to a physical-pixel rect at `scale`, rounding outwards so the
    /// result covers every pixel the logical rect touches.
    #[must_use]
    pub fn to_physical(&self, scale: f32) -> PxRect {
        let x0 = (self.x * scale).floor();
        let y0 = (self.y * scale).floor();
        let x1 = (self.right() * scale).ceil();
        let y1 = (self.bottom() * scale).ceil();
        PxRect {
            x: f32_to_i32(x0),
            y: f32_to_i32(y0),
            w: f32_to_u32(x1 - x0),
            h: f32_to_u32(y1 - y0),
        }
    }
}

/// A rectangle in physical (buffer) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl PxRect {
    /// Clamps the rect to a `width` x `height` buffer; `None` if nothing remains.
    #[must_use]
    pub fn clamp_to(&self, width: u32, height: u32) -> Option<Self> {
        let x0 = i64::from(self.x).max(0);
        let y0 = i64::from(self.y).max(0);
        let x1 = (i64::from(self.x) + i64::from(self.w)).min(i64::from(width));
        let y1 = (i64::from(self.y) + i64::from(self.h)).min(i64::from(height));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Self {
            x: i32::try_from(x0).ok()?,
            y: i32::try_from(y0).ok()?,
            w: u32::try_from(x1 - x0).ok()?,
            h: u32::try_from(y1 - y0).ok()?,
        })
    }
}

/// Logical length → physical pixel count at `scale` (rounded, at least 1).
#[must_use]
pub fn physical_len(logical: f32, scale: f32) -> u32 {
    f32_to_u32((logical * scale).round()).max(1)
}

/// Rounds `v` to the nearest physical pixel boundary at `scale`, so 1 px
/// outlines and card edges land on whole device pixels.
#[must_use]
pub fn snap(v: f32, scale: f32) -> f32 {
    if scale <= 0.0 {
        return v;
    }
    (v * scale).round() / scale
}

/// Saturating float → `u32` (negative and NaN become 0).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "saturating conversion is the documented intent; `as` saturates"
)]
pub fn f32_to_u32(v: f32) -> u32 {
    v.max(0.0) as u32
}

/// Saturating float → `i32` (NaN becomes 0).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "saturating conversion is the documented intent; `as` saturates"
)]
pub fn f32_to_i32(v: f32) -> i32 {
    v as i32
}

/// Integer → float for pixel math. Pixel counts are far below 2^24, so the
/// conversion is exact in practice.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "pixel dimensions are < 2^24 and convert exactly"
)]
pub fn u32_to_f32(v: u32) -> f32 {
    v as f32
}

/// See [`u32_to_f32`].
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "pixel offsets are < 2^24 and convert exactly"
)]
pub fn i32_to_f32(v: i32) -> f32 {
    v as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(10.0, 10.0, 20.0, 10.0);
        assert!(r.contains(Point::new(10.0, 10.0)));
        assert!(r.contains(Point::new(29.9, 19.9)));
        assert!(!r.contains(Point::new(30.0, 15.0)));
        assert!(!r.contains(Point::new(15.0, 20.0)));
    }

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert_eq!(a.intersect(&Rect::new(20.0, 0.0, 1.0, 1.0)), None);
        assert_eq!(Rect::default().union(&a), a);
        assert_eq!(a.union(&Rect::default()), a);
    }

    #[test]
    fn physical_rounds_outwards() {
        let r = Rect::new(0.5, 0.5, 10.0, 10.0).to_physical(1.6);
        assert_eq!(
            r,
            PxRect {
                x: 0,
                y: 0,
                w: 17,
                h: 17
            }
        );
    }

    #[test]
    fn px_rect_clamps() {
        let r = PxRect {
            x: -5,
            y: 2,
            w: 10,
            h: 100,
        };
        assert_eq!(
            r.clamp_to(20, 50),
            Some(PxRect {
                x: 0,
                y: 2,
                w: 5,
                h: 48
            })
        );
        assert_eq!(
            PxRect {
                x: 30,
                y: 0,
                w: 5,
                h: 5
            }
            .clamp_to(20, 20),
            None
        );
    }

    #[test]
    fn snapping_lands_on_device_pixels() {
        let v = snap(10.3, 1.6);
        assert!(((v * 1.6) - (v * 1.6).round()).abs() < 1e-4);
        assert_eq!(physical_len(388.0, 1.6), 621);
        // No scale yet (surface not configured): left as is.
        assert!((snap(10.3, 0.0) - 10.3).abs() < f32::EPSILON);
    }
}
