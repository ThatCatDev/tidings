//! Sprites: cached layer pixels and a fast premultiplied blend to put them
//! on screen.
//!
//! tiny-skia's general compositing pipeline costs ~15 ns per pixel here,
//! which made every animation frame that repainted the panel take ~30 ms.
//! Layers ([`crate::scene::Layer`]) are painted once and kept in a
//! [`SpriteCache`]; a frame then only blends sprites with [`blend`], a plain
//! integer source-over loop.

use tiny_skia::Pixmap;

use crate::geom::{PxRect, f32_to_u32};
use crate::scene::Scene;
use crate::style::Palette;

/// Sprites kept between paints (cards, the panel glass, ...).
const CAPACITY: usize = 64;

#[derive(Debug)]
struct Entry {
    scene: Scene,
    scale: u32,
    palette: Palette,
    pixmap: Pixmap,
    used: u64,
}

/// Layer pixels keyed by what was painted (scene, scale and palette),
/// evicting the least recently used beyond [`CAPACITY`].
#[derive(Debug, Default)]
pub struct SpriteCache {
    entries: Vec<Entry>,
    clock: u64,
}

impl SpriteCache {
    /// The cached pixels for `scene`, painting them with `paint` on a miss.
    pub fn get_or_paint(
        &mut self,
        scene: &Scene,
        palette: &Palette,
        scale: f32,
        paint: impl FnOnce() -> Option<Pixmap>,
    ) -> Option<&Pixmap> {
        self.clock += 1;
        let key = scale.to_bits();
        let found = self
            .entries
            .iter()
            .position(|e| e.scale == key && e.scene == *scene && e.palette == *palette);
        let index = if let Some(i) = found {
            i
        } else {
            let pixmap = paint()?;
            if self.entries.len() >= CAPACITY
                && let Some(oldest) = self
                    .entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, e)| e.used)
                    .map(|(i, _)| i)
            {
                self.entries.swap_remove(oldest);
            }
            self.entries.push(Entry {
                scene: scene.clone(),
                scale: key,
                palette: palette.clone(),
                pixmap,
                used: 0,
            });
            self.entries.len() - 1
        };
        let entry = self.entries.get_mut(index)?;
        entry.used = self.clock;
        Some(&entry.pixmap)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Composites premultiplied `src` over `dst` with its top-left at physical
/// `(x, y)`, faded by `opacity`, touching only pixels inside `clip`.
#[allow(
    clippy::many_single_char_names,
    reason = "conventional pixel-loop names"
)]
pub fn blend(dst: &mut Pixmap, src: &Pixmap, x: i32, y: i32, opacity: f32, clip: Option<PxRect>) {
    let o = f32_to_u32((opacity.clamp(0.0, 1.0) * 256.0).round());
    if o == 0 {
        return;
    }
    let (dw, dh) = (i64::from(dst.width()), i64::from(dst.height()));
    let (sw, sh) = (i64::from(src.width()), i64::from(src.height()));
    let (x, y) = (i64::from(x), i64::from(y));
    let (mut x0, mut y0) = (x.max(0), y.max(0));
    let (mut x1, mut y1) = ((x + sw).min(dw), (y + sh).min(dh));
    if let Some(c) = clip {
        x0 = x0.max(i64::from(c.x));
        y0 = y0.max(i64::from(c.y));
        x1 = x1.min(i64::from(c.x) + i64::from(c.w));
        y1 = y1.min(i64::from(c.y) + i64::from(c.h));
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    // Every offset below is non-negative: x0 >= max(x, 0), y0 >= max(y, 0).
    let ix = |v: i64| usize::try_from(v).unwrap_or(0);
    let (cols, dw, sw) = (ix(x1 - x0), ix(dw), ix(sw));
    let (dx0, sx0) = (ix(x0), ix(x0 - x));
    let src = src.data();
    let dst = dst.data_mut();
    for row in y0..y1 {
        let d = (ix(row) * dw + dx0) * 4;
        let s = (ix(row - y) * sw + sx0) * 4;
        if let (Some(d), Some(s)) = (dst.get_mut(d..d + cols * 4), src.get(s..s + cols * 4)) {
            for (d, s) in d
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(s.as_chunks::<4>().0)
            {
                *d = over(u32::from_ne_bytes(*d), u32::from_ne_bytes(*s), o).to_ne_bytes();
            }
        }
    }
}

/// Even bytes (two channels) of a packed pixel, each in its own 16 bits.
const LO: u32 = 0x00ff_00ff;

/// One packed premultiplied RGBA pixel, `d = s·o + d·(1 − s.a·o)` with `o`
/// in 0..=256, two channels per multiply (byte order does not matter
/// except for finding alpha, the last byte in memory).
#[inline]
fn over(d: u32, s: u32, o: u32) -> u32 {
    let s = if o >= 256 { s } else { scale(s, o) };
    let sa = s.to_ne_bytes()[3];
    match sa {
        0 => d,
        _ if d == 0 => s,
        255 => s,
        _ => s + scale(d, 256 - u32::from(sa)),
    }
}

/// Every channel of `p` times `k / 256` (`k` in 0..=256).
#[inline]
fn scale(p: u32, k: u32) -> u32 {
    let lo = (((p & LO) * k) >> 8) & LO;
    let hi = (((p >> 8) & LO) * k) & !LO;
    lo | hi
}

#[cfg(test)]
mod tests {
    use tiny_skia::{Color, PremultipliedColorU8};

    use super::*;
    use crate::geom::Size;
    use crate::style::default_theme;

    fn filled(w: u32, h: u32, c: Color) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap_or_else(|| panic!("pixmap"));
        p.fill(c);
        p
    }

    fn px(p: &Pixmap, x: u32, y: u32) -> PremultipliedColorU8 {
        p.pixel(x, y).unwrap_or_else(|| panic!("pixel"))
    }

    #[test]
    fn blend_matches_source_over() {
        let mut dst = filled(4, 4, Color::from_rgba8(0, 0, 255, 255));
        let src = filled(2, 2, Color::from_rgba8(255, 0, 0, 128));
        blend(&mut dst, &src, 1, 1, 1.0, None);
        let p = px(&dst, 1, 1);
        assert_eq!(p.alpha(), 255);
        assert!((127..=129).contains(&p.red()) && (126..=128).contains(&p.blue()));
        // Outside the sprite is untouched.
        assert_eq!(px(&dst, 0, 0).blue(), 255);
        assert_eq!(px(&dst, 3, 3).red(), 0);
    }

    #[test]
    fn blend_fades_clips_and_clamps_to_edges() {
        let mut dst = Pixmap::new(4, 4).unwrap_or_else(|| panic!("pixmap"));
        let src = filled(4, 4, Color::WHITE);
        blend(&mut dst, &src, 2, -2, 0.5, None);
        assert!((127..=129).contains(&px(&dst, 3, 0).alpha()));
        assert_eq!(px(&dst, 1, 0).alpha(), 0);
        assert_eq!(px(&dst, 3, 2).alpha(), 0);

        let mut dst = Pixmap::new(4, 4).unwrap_or_else(|| panic!("pixmap"));
        let clip = PxRect {
            x: 0,
            y: 0,
            w: 2,
            h: 4,
        };
        blend(&mut dst, &src, 0, 0, 1.0, Some(clip));
        assert_eq!(px(&dst, 1, 1).alpha(), 255);
        assert_eq!(px(&dst, 2, 1).alpha(), 0);
        // Entirely off-screen is a no-op, not a panic.
        blend(&mut dst, &src, 100, 100, 1.0, None);
    }

    #[test]
    fn blend_without_opacity_or_overlap_changes_nothing() {
        let blue = Color::from_rgba8(0, 0, 255, 255);
        let src = filled(2, 2, Color::WHITE);
        let untouched = filled(4, 4, blue);
        let mut dst = untouched.clone();
        blend(&mut dst, &src, 1, 1, 0.0, None);
        blend(&mut dst, &src, 1, 1, 0.001, None);
        // Above, left of and below the destination.
        blend(&mut dst, &src, 1, -2, 1.0, None);
        blend(&mut dst, &src, -2, 1, 1.0, None);
        blend(&mut dst, &src, 1, 4, 1.0, None);
        // A clip that misses the sprite.
        let clip = PxRect {
            x: 3,
            y: 3,
            w: 1,
            h: 1,
        };
        blend(&mut dst, &src, 0, 0, 1.0, Some(clip));
        assert_eq!(dst, untouched);
    }

    #[test]
    fn over_handles_clear_and_empty_pixels() {
        let red = u32::from_ne_bytes([200, 0, 0, 200]);
        let blue = u32::from_ne_bytes([0, 0, 255, 255]);
        assert_eq!(
            over(blue, 0, 256),
            blue,
            "clear source keeps the destination"
        );
        assert_eq!(over(0, red, 256), red, "empty destination takes the source");
        assert_eq!(over(blue, blue, 256), blue, "opaque source replaces");
        let half = over(0, red, 128).to_ne_bytes();
        assert_eq!(half, [100, 0, 0, 100], "faded source");
    }

    #[test]
    fn cache_reuses_until_the_scene_changes() {
        let mut cache = SpriteCache::default();
        let palette = Palette::new(default_theme());
        let a = Scene::new(Size::new(4.0, 4.0));
        let b = Scene::new(Size::new(5.0, 4.0));
        let mut paints = 0;
        let mut get = |cache: &mut SpriteCache, s: &Scene| {
            cache
                .get_or_paint(s, &palette, 1.0, || {
                    paints += 1;
                    Pixmap::new(1, 1)
                })
                .is_some()
        };
        assert!(get(&mut cache, &a));
        assert!(get(&mut cache, &a));
        assert!(get(&mut cache, &b));
        assert_eq!(paints, 2);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let mut cache = SpriteCache::default();
        let palette = Palette::new(default_theme());
        let scenes: Vec<Scene> = (0..=CAPACITY)
            .map(|i| {
                Scene::new(Size::new(
                    crate::geom::u32_to_f32(u32::try_from(i).unwrap_or(0)),
                    1.0,
                ))
            })
            .collect();
        for s in &scenes {
            cache.get_or_paint(s, &palette, 1.0, || Pixmap::new(1, 1));
        }
        assert_eq!(cache.len(), CAPACITY);
    }
}
