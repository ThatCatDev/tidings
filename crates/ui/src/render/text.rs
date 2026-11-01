//! cosmic-text backed [`TextMeasure`] and glyph rasteriser.
//!
//! Text is shaped once at logical size and cached; glyphs are rasterised at
//! `font_size * scale` via swash, so line breaks are identical between layout
//! and painting at any scale.

use std::collections::HashMap;
use std::process::Command;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Wrap,
};
use tiny_skia::PixmapMut;

use crate::geom::{PxRect, Rect, f32_to_u32};
use crate::scene::Align;
use crate::style::{Rgba, TextStyle, Weight};
use crate::text::{TextFit, TextMeasure, ellipsize, normalise};

/// Upper bound on cached shaped buffers before the cache is flushed.
const CACHE_LIMIT: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FitKey {
    text: String,
    style: TextStyle,
    width_bits: u32,
    max_lines: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    text: String,
    style: TextStyle,
    width_bits: u32,
}

/// Font system, glyph cache and shaping caches.
pub struct TextEngine {
    fonts: FontSystem,
    swash: SwashCache,
    family: String,
    fits: HashMap<FitKey, TextFit>,
    shaped: HashMap<ShapeKey, Buffer>,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextEngine")
            .field("family", &self.family)
            .field("cached_fits", &self.fits.len())
            .field("cached_buffers", &self.shaped.len())
            .finish_non_exhaustive()
    }
}

/// Inter (or a close relative) first, then whatever fontconfig calls "sans".
const PREFERRED: &[&str] = &["Inter", "Inter Variable", "Inter Display"];

impl TextEngine {
    /// Loads system fonts and picks the UI family. Slow (font scan); call once.
    #[must_use]
    pub fn new() -> Self {
        let fonts = FontSystem::new();
        let family = pick_family(&fonts);
        log::debug!("tidings-ui: using font family {family:?}");
        Self {
            fonts,
            swash: SwashCache::new(),
            family,
            fits: HashMap::new(),
            shaped: HashMap::new(),
        }
    }

    /// The resolved UI font family.
    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }

    fn buffer(&mut self, text: &str, style: TextStyle, width: f32) -> &Buffer {
        let key = ShapeKey {
            text: text.to_owned(),
            style,
            width_bits: width.to_bits(),
        };
        if self.shaped.len() > CACHE_LIMIT {
            self.shaped.clear();
        }
        let fonts = &mut self.fonts;
        let family = &self.family;
        self.shaped.entry(key).or_insert_with(|| {
            let metrics = Metrics::new(style.size(), style.line_height());
            let mut buf = Buffer::new(fonts, metrics);
            buf.set_wrap(fonts, Wrap::WordOrGlyph);
            buf.set_size(fonts, Some(width.max(1.0)), None);
            let attrs = Attrs::new()
                .family(Family::Name(family))
                .weight(cosmic_text::Weight(weight(style.weight())));
            buf.set_text(fonts, text, &attrs, Shaping::Advanced, None);
            buf.shape_until_scroll(fonts, false);
            buf
        })
    }

    fn line_count(&mut self, text: &str, style: TextStyle, width: f32) -> usize {
        self.buffer(text, style, width).layout_runs().count()
    }

    /// Draws fitted text into `pixmap` (physical pixels) at logical `rect`.
    #[allow(
        clippy::too_many_arguments,
        reason = "a flat draw call reads better than a struct here"
    )]
    pub fn draw(
        &mut self,
        pixmap: &mut PixmapMut<'_>,
        text: &str,
        style: TextStyle,
        wrap_width: f32,
        rect: Rect,
        align: Align,
        color: Rgba,
        scale: f32,
        clip: Option<PxRect>,
    ) {
        // Shape (cached) first, then borrow the parts we need disjointly.
        self.buffer(text, style, wrap_width);
        let key = ShapeKey {
            text: text.to_owned(),
            style,
            width_bits: wrap_width.to_bits(),
        };
        let Some(buf) = self.shaped.get(&key) else {
            return;
        };
        let (pw, ph) = (pixmap.width(), pixmap.height());
        let clip = clip
            .unwrap_or(PxRect {
                x: 0,
                y: 0,
                w: pw,
                h: ph,
            })
            .clamp_to(pw, ph);
        let Some(clip) = clip else { return };
        let data = pixmap.data_mut();
        for run in buf.layout_runs() {
            let dx = match align {
                Align::Left => 0.0,
                Align::Center => ((rect.w - run.line_w) / 2.0).max(0.0),
                Align::Right => (rect.w - run.line_w).max(0.0),
            };
            let ox = (rect.x + dx) * scale;
            let oy = (rect.y + run.line_y) * scale;
            for glyph in run.glyphs {
                let phys = glyph.physical((ox, oy), scale);
                let Some(img) = self.swash.get_image(&mut self.fonts, phys.cache_key) else {
                    continue;
                };
                let gx = phys.x + img.placement.left;
                let gy = phys.y - img.placement.top;
                blit_glyph(data, pw, &clip, gx, gy, img, color);
            }
        }
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextMeasure for TextEngine {
    fn fit(&mut self, text: &str, style: TextStyle, max_width: f32, max_lines: u32) -> TextFit {
        let key = FitKey {
            text: text.to_owned(),
            style,
            width_bits: max_width.to_bits(),
            max_lines,
        };
        if let Some(f) = self.fits.get(&key) {
            return f.clone();
        }
        let width = max_width.max(1.0);
        let max = usize::try_from(max_lines.max(1)).unwrap_or(1);
        let normal = normalise(text, style);
        let shown = if self.line_count(&normal, style, width) <= max {
            normal
        } else {
            ellipsize(&normal, |cand| self.line_count(cand, style, width) <= max)
        };
        let buf = self.buffer(&shown, style, width);
        let (mut w, mut lines) = (0.0f32, 0u32);
        for run in buf.layout_runs() {
            w = w.max(run.line_w);
            lines += 1;
        }
        let fit = TextFit {
            width: w.ceil().min(width),
            height: crate::geom::u32_to_f32(lines) * style.line_height(),
            lines,
            text: shown,
        };
        if self.fits.len() > CACHE_LIMIT {
            self.fits.clear();
        }
        self.fits.insert(key, fit.clone());
        fit
    }
}

fn weight(w: Weight) -> u16 {
    match w {
        Weight::Regular => 400,
        Weight::Medium => 500,
        Weight::Semibold => 600,
        Weight::Bold => 700,
    }
}

fn has_family(fonts: &FontSystem, name: &str) -> bool {
    fonts
        .db()
        .faces()
        .any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
}

/// Inter if installed, else fontconfig's `sans-serif`, else any face.
fn pick_family(fonts: &FontSystem) -> String {
    if let Some(name) = PREFERRED.iter().find(|n| has_family(fonts, n)) {
        return (*name).to_owned();
    }
    let fc = Command::new("fc-match")
        .args(["-f", "%{family[0]}", "sans-serif"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty() && has_family(fonts, s));
    if let Some(name) = fc {
        return name;
    }
    [
        "Noto Sans",
        "Adwaita Sans",
        "Cantarell",
        "DejaVu Sans",
        "Liberation Sans",
    ]
    .iter()
    .find(|n| has_family(fonts, n))
    .map(|s| (*s).to_owned())
    .or_else(|| {
        fonts
            .db()
            .faces()
            .find_map(|f| f.families.first().map(|(n, _)| n.clone()))
    })
    .unwrap_or_else(|| "sans-serif".to_owned())
}

/// Source-over blend of one glyph image into premultiplied RGBA8 `data`.
fn blit_glyph(
    data: &mut [u8],
    stride_px: u32,
    clip: &PxRect,
    gx: i32,
    gy: i32,
    img: &cosmic_text::SwashImage,
    color: Rgba,
) {
    let (w, h) = (img.placement.width, img.placement.height);
    let x0 = gx.max(clip.x);
    let y0 = gy.max(clip.y);
    let x1 = (gx + i32::try_from(w).unwrap_or(0)).min(clip.x + i32::try_from(clip.w).unwrap_or(0));
    let y1 = (gy + i32::try_from(h).unwrap_or(0)).min(clip.y + i32::try_from(clip.h).unwrap_or(0));
    let (cr, cg, cb) = (color.r * 255.0, color.g * 255.0, color.b * 255.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let (sx, sy) = (
                usize::try_from(x - gx).unwrap_or(0),
                usize::try_from(y - gy).unwrap_or(0),
            );
            let si = sy * usize::try_from(w).unwrap_or(0) + sx;
            // Premultiplied source colour for this pixel.
            let src = match img.content {
                SwashContent::Mask => {
                    let a = f32::from(img.data[si]) / 255.0 * color.a;
                    [cr * a, cg * a, cb * a, a * 255.0]
                }
                SwashContent::Color => {
                    let p = &img.data[si * 4..si * 4 + 4];
                    let a = f32::from(p[3]) / 255.0 * color.a;
                    [
                        f32::from(p[0]) * a,
                        f32::from(p[1]) * a,
                        f32::from(p[2]) * a,
                        a * 255.0,
                    ]
                }
                SwashContent::SubpixelMask => {
                    let p = &img.data[si * 4..si * 4 + 4];
                    let a = f32::from(p[0].max(p[1]).max(p[2])) / 255.0 * color.a;
                    [cr * a, cg * a, cb * a, a * 255.0]
                }
            };
            if src[3] <= 0.0 {
                continue;
            }
            let di = (usize::try_from(y).unwrap_or(0) * usize::try_from(stride_px).unwrap_or(0)
                + usize::try_from(x).unwrap_or(0))
                * 4;
            let inv = 1.0 - src[3] / 255.0;
            for c in 0..4 {
                let d = f32::from(data[di + c]);
                data[di + c] =
                    u8::try_from(f32_to_u32((src[c] + d * inv).round()).min(255)).unwrap_or(255);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Pixmap;

    #[test]
    fn fits_wrap_and_ellipsize_with_real_fonts() {
        let mut t = TextEngine::new();
        let long = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        let fit = t.fit(&long, TextStyle::Body, 200.0, 2);
        assert_eq!(fit.lines, 2);
        assert!(fit.text.ends_with('…'));
        assert!(fit.width <= 200.0);
        let short = t.fit("Hi", TextStyle::Summary, 200.0, 2);
        assert_eq!(short.text, "Hi");
        assert_eq!(short.lines, 1);
        assert!(short.width > 5.0);
        // Single-line style folds newlines.
        let meta = t.fit("a\nb", TextStyle::Meta, 200.0, 1);
        assert_eq!(meta.text, "a b");
    }

    #[test]
    fn draws_glyph_pixels_in_colour() {
        let mut t = TextEngine::new();
        let mut pm = Pixmap::new(200, 40).unwrap();
        let red = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        t.draw(
            &mut pm.as_mut(),
            "Hello",
            TextStyle::Title,
            180.0,
            Rect::new(4.0, 4.0, 180.0, 21.0),
            Align::Left,
            red,
            1.6,
            None,
        );
        let lit: Vec<_> = pm.pixels().iter().filter(|p| p.alpha() > 128).collect();
        assert_ne!(lit.len(), 0);
        assert!(lit.iter().all(|p| p.red() >= p.green() && p.green() == 0));
    }
}
