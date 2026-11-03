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
use crate::style::{Rgba, TextStyle, Type, Weight};
use crate::text::{NO_WRAP, TextFit, TextMeasure, ellipsize, normalise};

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
    typ: Type,
    fits: HashMap<FitKey, TextFit>,
    shaped: HashMap<ShapeKey, Buffer>,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextEngine")
            .field("family", &self.family)
            .field("type", &self.typ)
            .field("cached_fits", &self.fits.len())
            .field("cached_buffers", &self.shaped.len())
            .finish_non_exhaustive()
    }
}

/// Inter (or a close relative) first, then whatever fontconfig calls "sans".
const PREFERRED: &[&str] = &["Inter", "Inter Variable", "Inter Display"];

impl TextEngine {
    /// Loads system fonts and picks the UI family at the default sizes.
    /// Slow (font scan); call once.
    #[must_use]
    pub fn new() -> Self {
        Self::with_type(Type::default(), None)
    }

    /// Like [`TextEngine::new`], with the theme's sizes and spacing and,
    /// if given and installed, its font `family` (a name or a fontconfig
    /// alias such as `monospace`).
    #[must_use]
    pub fn with_type(typ: Type, family: Option<&str>) -> Self {
        let fonts = FontSystem::new();
        let family = family
            .and_then(|f| resolve_family(&fonts, f))
            .unwrap_or_else(|| pick_family(&fonts, || fc_match("sans-serif")));
        log::debug!("tidings-ui: using font family {family:?}");
        Self {
            fonts,
            swash: SwashCache::new(),
            family,
            typ,
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
        shape(
            &mut self.shaped,
            &mut self.fonts,
            &self.family,
            self.typ,
            text,
            style,
            width,
        )
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
        // The shaped buffer borrows only the cache, leaving the fonts and
        // glyph cache free for rasterising.
        let buf = shape(
            &mut self.shaped,
            &mut self.fonts,
            &self.family,
            self.typ,
            text,
            style,
            wrap_width,
        );
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
    fn line_height(&self, style: TextStyle) -> f32 {
        self.typ.line_height(style)
    }

    fn caret_stops(&mut self, text: &str, style: TextStyle) -> Vec<f32> {
        // Shaped at the same width the painter draws it with, so the
        // caret lands exactly between the drawn glyphs.
        let buf = self.buffer(text, style, NO_WRAP);
        let glyphs: Vec<(usize, usize, f32, f32)> = buf
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().map(|g| (g.start, g.end, g.x, g.w)))
            .collect();
        caret_stops_from_glyphs(text, &glyphs)
    }

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
            height: crate::geom::u32_to_f32(lines) * self.typ.line_height(style),
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

/// `text` shaped in `style` and wrapped at `width`, from the `shaped` cache
/// or shaped now and cached (the cache is flushed when full).
fn shape<'a>(
    shaped: &'a mut HashMap<ShapeKey, Buffer>,
    fonts: &mut FontSystem,
    family: &str,
    typ: Type,
    text: &str,
    style: TextStyle,
    width: f32,
) -> &'a Buffer {
    let key = ShapeKey {
        text: text.to_owned(),
        style,
        width_bits: width.to_bits(),
    };
    if shaped.len() > CACHE_LIMIT {
        shaped.clear();
    }
    shaped.entry(key).or_insert_with(|| {
        let metrics = Metrics::new(typ.size(style), typ.line_height(style));
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

/// Caret stops (see [`TextMeasure::caret_stops`]) from shaped glyphs given
/// as `(start byte, end byte, x, width)`. A char inside a multi-char cluster
/// (a ligature, say) gets a stop part-way across it, in proportion to its
/// byte offset; a char no glyph covers shares the previous stop.
fn caret_stops_from_glyphs(text: &str, glyphs: &[(usize, usize, f32, f32)]) -> Vec<f32> {
    let width = glyphs.iter().map(|&(_, _, x, w)| x + w).fold(0.0, f32::max);
    let mut stops = Vec::with_capacity(text.len() + 1);
    let mut last = 0.0;
    for (b, _) in text.char_indices() {
        let at = glyphs
            .iter()
            .find(|&&(s, e, _, _)| s <= b && b < e)
            .map(|&(s, e, x, w)| {
                let frac = crate::geom::u32_to_f32(u32::try_from(b - s).unwrap_or(0))
                    / crate::geom::u32_to_f32(u32::try_from(e - s).unwrap_or(1).max(1));
                x + w * frac
            });
        last = at.unwrap_or(last);
        stops.push(last);
    }
    stops.push(width);
    stops
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

/// `name` if installed, else what fontconfig substitutes for it (which
/// resolves aliases like `monospace`); `None` (with a warning) if neither.
fn resolve_family(fonts: &FontSystem, name: &str) -> Option<String> {
    if has_family(fonts, name) {
        return Some(name.to_owned());
    }
    let found = fc_match(name).filter(|f| has_family(fonts, f));
    match &found {
        Some(f) => log::info!("tidings-ui: font {name:?} resolves to {f:?}"),
        None => log::warn!("tidings-ui: font {name:?} is not installed; using the UI font"),
    }
    found
}

/// fontconfig's first family for `pattern`.
fn fc_match(pattern: &str) -> Option<String> {
    Command::new("fc-match")
        .args(["-f", "%{family[0]}", pattern])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Common sans families to try when fontconfig gives no usable answer.
const FALLBACK: &[&str] = &[
    "Noto Sans",
    "Adwaita Sans",
    "Cantarell",
    "DejaVu Sans",
    "Liberation Sans",
];

/// Inter if installed, else the system `sans` family (asked for lazily),
/// else a common sans, else any face.
fn pick_family(fonts: &FontSystem, sans: impl FnOnce() -> Option<String>) -> String {
    first_installed(fonts, PREFERRED)
        .or_else(|| sans().filter(|s| has_family(fonts, s)))
        .or_else(|| first_installed(fonts, FALLBACK))
        .or_else(|| any_family(fonts))
        .unwrap_or_else(|| "sans-serif".to_owned())
}

/// The first of `names` that is installed.
fn first_installed(fonts: &FontSystem, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|n| has_family(fonts, n))
        .map(|s| (*s).to_owned())
}

/// The family of the first installed face.
fn any_family(fonts: &FontSystem) -> Option<String> {
    fonts
        .db()
        .faces()
        .find_map(|f| f.families.first().map(|(n, _)| n.clone()))
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
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;
    use tiny_skia::Pixmap;

    /// Font scanning is slow; share one engine across these tests.
    fn engine() -> MutexGuard<'static, TextEngine> {
        static E: OnceLock<Mutex<TextEngine>> = OnceLock::new();
        E.get_or_init(|| Mutex::new(TextEngine::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    const WHITE: Rgba = Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    /// A `w` x `h` glyph image of `content` with its top-left at the origin.
    fn glyph_image(
        content: SwashContent,
        w: u32,
        h: u32,
        data: Vec<u8>,
    ) -> cosmic_text::SwashImage {
        let mut img = cosmic_text::SwashImage::new();
        img.content = content;
        img.placement.width = w;
        img.placement.height = h;
        img.data = data;
        img
    }

    fn full_clip(w: u32, h: u32) -> PxRect {
        PxRect { x: 0, y: 0, w, h }
    }

    #[test]
    fn debug_shows_family_and_cache_sizes() {
        let t = TextEngine::default();
        let shown = format!("{t:?}");
        assert!(shown.contains(&format!("{:?}", t.family())), "{shown}");
        assert!(shown.contains("cached_fits: 0"), "{shown}");
        assert!(
            has_family(&t.fonts, t.family()),
            "the UI family is installed"
        );
    }

    #[test]
    fn installed_family_names_resolve_to_themselves() {
        let t = engine();
        let name = t.family().to_owned();
        assert_eq!(resolve_family(&t.fonts, &name), Some(name.clone()));
        // Case does not matter.
        let upper = name.to_uppercase();
        assert_eq!(resolve_family(&t.fonts, &upper), Some(upper));
    }

    #[test]
    fn unknown_families_resolve_to_an_installed_face_or_none() {
        let t = engine();
        // fontconfig substitutes something for any name; if what it picks is
        // not installed (or fc-match is missing) the theme font is ignored.
        let got = resolve_family(&t.fonts, "No Such Font Tidings");
        assert!(
            got.as_deref().is_none_or(|f| has_family(&t.fonts, f)),
            "{got:?}"
        );
        let mono = resolve_family(&t.fonts, "monospace");
        assert!(
            mono.as_deref().is_none_or(|f| has_family(&t.fonts, f)),
            "{mono:?}"
        );
    }

    #[test]
    fn picked_family_is_always_installed() {
        let t = engine();
        let fonts = &t.fonts;
        let preferred = first_installed(fonts, PREFERRED);
        // Without fontconfig, or when it names a missing font, a known sans
        // (or any face) is used.
        for sans in [None, Some("Not Installed Sans".to_owned())] {
            let picked = pick_family(fonts, || sans);
            assert!(has_family(fonts, &picked), "{picked}");
            if let Some(p) = &preferred {
                assert_eq!(&picked, p, "Inter wins when installed");
            }
        }
        // An installed system sans is taken unless Inter is installed.
        let installed = any_family(fonts).unwrap_or_default();
        let picked = pick_family(fonts, || Some(installed.clone()));
        assert_eq!(picked, preferred.unwrap_or(installed));
    }

    #[test]
    fn first_installed_skips_missing_names() {
        let t = engine();
        let real = t.family().to_owned();
        assert_eq!(
            first_installed(&t.fonts, &["Missing One", &real]),
            Some(real)
        );
        assert_eq!(first_installed(&t.fonts, &["Missing One"]), None);
    }

    #[test]
    fn caches_are_flushed_when_full() {
        let mut t = engine();
        t.fits.clear();
        t.shaped.clear();
        for i in 0..=CACHE_LIMIT + 1 {
            t.fit(&i.to_string(), TextStyle::Meta, 100.0, 1);
        }
        assert!(t.fits.len() <= CACHE_LIMIT, "{}", t.fits.len());
        assert!(t.shaped.len() <= CACHE_LIMIT, "{}", t.shaped.len());
        // Still correct after a flush, and cached again.
        let fit = t.fit("12", TextStyle::Meta, 100.0, 1);
        assert_eq!((fit.text.as_str(), fit.lines), ("12", 1));
        assert_eq!(t.fit("12", TextStyle::Meta, 100.0, 1), fit);
    }

    #[test]
    fn aligned_text_lands_left_centre_or_right() {
        let mut t = engine();
        // Leftmost and rightmost lit columns of "Hi" in a 200 px box.
        let mut extent = |align| {
            let mut pm = Pixmap::new(200, 30).unwrap();
            t.draw(
                &mut pm.as_mut(),
                "Hi",
                TextStyle::Summary,
                200.0,
                Rect::new(0.0, 0.0, 200.0, 30.0),
                align,
                WHITE,
                1.0,
                None,
            );
            let lit: Vec<u32> = (0..200)
                .filter(|&x| (0..30).any(|y| pm.pixel(x, y).is_some_and(|p| p.alpha() > 0)))
                .collect();
            (lit[0], lit[lit.len() - 1])
        };
        let (l0, l1) = extent(Align::Left);
        let (c0, c1) = extent(Align::Center);
        let (r0, r1) = extent(Align::Right);
        assert!(l0 < 4, "left starts at the edge: {l0}");
        assert!(r1 > 195, "right ends at the edge: {r1}");
        let (left_gap, right_gap) = (i64::from(c0), 199 - i64::from(c1));
        assert!((left_gap - right_gap).abs() <= 3, "centred: {c0}..{c1}");
        // Same glyphs, only moved (subpixel placement may shift a column).
        assert!((l1 - l0).abs_diff(r1 - r0) <= 1, "{l0}..{l1} vs {r0}..{r1}");
    }

    #[test]
    fn text_outside_the_clip_or_pixmap_is_not_drawn() {
        let mut t = engine();
        let mut pm = Pixmap::new(100, 30).unwrap();
        let draw = |t: &mut TextEngine, pm: &mut Pixmap, clip| {
            t.draw(
                &mut pm.as_mut(),
                "Hello",
                TextStyle::Summary,
                100.0,
                Rect::new(0.0, 0.0, 100.0, 30.0),
                Align::Left,
                WHITE,
                1.0,
                clip,
            );
        };
        // A clip wholly outside the pixmap draws nothing.
        draw(
            &mut t,
            &mut pm,
            Some(PxRect {
                x: 200,
                y: 0,
                w: 10,
                h: 10,
            }),
        );
        assert!(pm.pixels().iter().all(|p| p.alpha() == 0));
        // A clip over the right half keeps the left half clear.
        draw(
            &mut t,
            &mut pm,
            Some(PxRect {
                x: 20,
                y: 0,
                w: 80,
                h: 30,
            }),
        );
        let lit_x: Vec<u32> = (0..100)
            .filter(|&x| (0..30).any(|y| pm.pixel(x, y).is_some_and(|p| p.alpha() > 0)))
            .collect();
        assert!(!lit_x.is_empty(), "the visible half is drawn");
        assert!(lit_x.iter().all(|&x| x >= 20), "{lit_x:?}");
    }

    #[test]
    fn mask_glyphs_blend_over_and_respect_the_clip() {
        // 2x1 mask: opaque, then half coverage; one pixel already blue.
        let img = glyph_image(SwashContent::Mask, 2, 1, vec![255, 128]);
        let mut data = vec![0u8; 3 * 4];
        data[4..8].copy_from_slice(&[0, 0, 255, 255]);
        let red = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        blit_glyph(&mut data, 3, &full_clip(3, 1), 0, 0, &img, red);
        assert_eq!(&data[0..4], &[255, 0, 0, 255], "opaque red");
        // Half red over opaque blue: premultiplied mix.
        assert_eq!(&data[4..8], &[128, 0, 127, 255]);
        assert_eq!(&data[8..12], &[0, 0, 0, 0], "outside the glyph");

        // Clipped to the second column only.
        let mut data = vec![0u8; 2 * 4];
        let clip = PxRect {
            x: 1,
            y: 0,
            w: 1,
            h: 1,
        };
        let solid = glyph_image(SwashContent::Mask, 2, 1, vec![255, 255]);
        blit_glyph(&mut data, 2, &clip, 0, 0, &solid, red);
        assert_eq!(&data[0..4], &[0, 0, 0, 0]);
        assert_eq!(&data[4..8], &[255, 0, 0, 255]);
    }

    #[test]
    fn colour_glyphs_keep_their_own_colours() {
        // An emoji-like 1x1 colour glyph: green, half transparent.
        let img = glyph_image(SwashContent::Color, 1, 1, vec![0, 255, 0, 128]);
        let mut data = vec![0u8; 4];
        // The text colour's alpha fades it; its hue is ignored.
        let faded = Rgba { a: 0.5, ..WHITE };
        blit_glyph(&mut data, 1, &full_clip(1, 1), 0, 0, &img, faded);
        assert_eq!(data, [0, 64, 0, 64]);
    }

    #[test]
    fn subpixel_glyphs_use_their_strongest_channel_as_coverage() {
        let img = glyph_image(
            SwashContent::SubpixelMask,
            2,
            1,
            vec![
                10, 255, 40, 0, // full coverage in green
                0, 0, 0, 0, // empty: left untouched
            ],
        );
        let mut data = vec![9u8; 2 * 4];
        let blue = Rgba {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        };
        blit_glyph(&mut data, 2, &full_clip(2, 1), 0, 0, &img, blue);
        assert_eq!(&data[0..4], &[0, 0, 255, 255]);
        assert_eq!(&data[4..8], &[9, 9, 9, 9]);
    }

    #[test]
    fn caret_stops_split_clusters_and_keep_spaces() {
        // "fi" as one ligature glyph, then a space and "é" (2 bytes).
        let glyphs = [(0, 2, 0.0, 10.0), (2, 3, 10.0, 4.0), (3, 5, 14.0, 6.0)];
        assert_eq!(
            caret_stops_from_glyphs("fi é", &glyphs),
            [0.0, 5.0, 10.0, 14.0, 20.0]
        );
        assert_eq!(caret_stops_from_glyphs("", &[]), [0.0]);
    }

    #[test]
    fn caret_stops_with_real_fonts_count_trailing_spaces() {
        let mut t = TextEngine::new();
        let stops = t.caret_stops("ab  ", TextStyle::Body);
        assert_eq!(stops.len(), 5);
        assert!(stops.windows(2).all(|w| w[0] < w[1]), "{stops:?}");
    }

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
