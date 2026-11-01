//! Icon and image loading: freedesktop icon lookup, SVG via resvg, raster
//! via `image`, all rasterised to the exact physical size and cached.
//!
//! Every avatar is round, but images are not all alike: a photo should fill
//! the circle, while an app icon has its own outline that a circular crop
//! would cut into. Each image is classified once, when loaded, as a
//! [`Shape::Photo`] or a [`Shape::Icon`], and the painter presents it
//! accordingly.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use tidings_core::model::Image;
use tiny_skia::Pixmap;

use crate::geom::{f32_to_u32, u32_to_f32};
use crate::style::metrics as m;

/// Bound on cached rasters before the cache is flushed.
const CACHE_LIMIT: usize = 256;

/// Share of see-through pixels above which an image counts as having its own
/// outline ([`Shape::Icon`]). Photos are fully opaque; an app icon with
/// rounded corners or a transparent background clears this easily.
const ICON_TRANSPARENCY: f32 = 0.02;

/// Alpha below which a pixel counts as see-through for [`ICON_TRANSPARENCY`].
const CLEAR_ALPHA: u8 = 200;

/// How an image sits in its round avatar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// An opaque picture (a photo, a contact avatar): centre-cropped to fill
    /// the circle.
    Photo,
    /// Art with its own outline (an app icon with transparent edges): shown
    /// whole, inset on a round plate, so the circle never cuts into it.
    Icon,
}

/// A rasterised avatar image and how it is presented.
#[derive(Debug)]
pub struct Raster {
    pub pixmap: Pixmap,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Name(String),
    Path(PathBuf),
    /// Raw pixels are keyed by content hash (ids of `Arc`s may be reused).
    Rgba(u32, u32, u64),
}

impl Key {
    fn of(image: &Image) -> Self {
        match image {
            Image::IconName(n) => Self::Name(n.clone()),
            Image::Path(p) => Self::Path(p.clone()),
            Image::Rgba {
                width,
                height,
                pixels,
            } => {
                let mut h = DefaultHasher::new();
                pixels.hash(&mut h);
                Self::Rgba(*width, *height, h.finish())
            }
        }
    }
}

/// Rasterised-image cache keyed by source and pixel size.
#[derive(Debug, Default)]
pub struct ImageCache {
    entries: HashMap<(Key, u32), Option<Arc<Raster>>>,
    icon_theme: Option<String>,
}

impl ImageCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            icon_theme: freedesktop_icons::default_theme_gtk(),
        }
    }

    /// Returns `image` rasterised for a `size` x `size` avatar, or `None` if
    /// it cannot be found or decoded (failures are cached too).
    pub fn get(&mut self, image: &Image, size: u32) -> Option<Arc<Raster>> {
        let key = (Key::of(image), size);
        if let Some(hit) = self.entries.get(&key) {
            return hit.clone();
        }
        if self.entries.len() > CACHE_LIMIT {
            self.entries.clear();
        }
        let loaded = self.load(image, size).map(Arc::new);
        if loaded.is_none() {
            log::debug!("tidings-ui: could not load image {image:?}");
        }
        self.entries.insert(key, loaded.clone());
        loaded
    }

    fn load(&self, image: &Image, size: u32) -> Option<Raster> {
        match image {
            Image::IconName(name) => {
                let path = self.resolve_icon(name, size)?;
                load_file(&path, size)
            }
            Image::Path(p) => load_file(p, size),
            Image::Rgba {
                width,
                height,
                pixels,
            } => {
                let img = RgbaImage::from_raw(*width, *height, pixels.to_vec())?;
                fit(&DynamicImage::ImageRgba8(img), size)
            }
        }
    }

    /// Icon names may also be paths or `file://` URIs (the spec allows both).
    fn resolve_icon(&self, name: &str, size: u32) -> Option<PathBuf> {
        if let Some(path) = name.strip_prefix("file://") {
            return Some(PathBuf::from(path));
        }
        if name.starts_with('/') {
            return Some(PathBuf::from(name));
        }
        let px = u16::try_from(size).unwrap_or(u16::MAX);
        let themed = self.icon_theme.as_deref().and_then(|t| {
            freedesktop_icons::lookup(name)
                .with_size(px)
                .with_theme(t)
                .with_cache()
                .find()
        });
        themed.or_else(|| {
            freedesktop_icons::lookup(name)
                .with_size(px)
                .with_cache()
                .find()
        })
    }
}

fn load_file(path: &Path, size: u32) -> Option<Raster> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
    if is_svg {
        // Vector art is an icon by nature: it draws its own outline.
        let pixmap = render_svg(&std::fs::read(path).ok()?, size, inset_side(size))?;
        Some(Raster {
            pixmap,
            shape: Shape::Icon,
        })
    } else {
        fit(&image::open(path).ok()?, size)
    }
}

/// Side of the box a [`Shape::Icon`] is contained in, for a `size` avatar.
fn inset_side(size: u32) -> u32 {
    f32_to_u32((u32_to_f32(size) * m::ICON_CONTENT).round()).clamp(1, size)
}

/// Classifies `img` and rasterises it for a `size` avatar.
#[must_use]
pub fn fit(img: &DynamicImage, size: u32) -> Option<Raster> {
    if has_own_outline(img) {
        Some(Raster {
            pixmap: contain(img, size, inset_side(size))?,
            shape: Shape::Icon,
        })
    } else {
        Some(Raster {
            pixmap: cover(img, size)?,
            shape: Shape::Photo,
        })
    }
}

/// Whether enough of `img` is see-through that it has an outline of its own.
fn has_own_outline(img: &DynamicImage) -> bool {
    if !img.color().has_alpha() {
        return false;
    }
    let rgba = img.to_rgba8();
    let total = rgba.pixels().len().max(1);
    let clear = rgba.pixels().filter(|p| p[3] < CLEAR_ALPHA).count();
    // Pixel counts of icon-sized images fit in f32 exactly enough for a ratio.
    #[allow(clippy::cast_precision_loss)]
    let share = clear as f32 / total as f32;
    share > ICON_TRANSPARENCY
}

/// Renders SVG data fitted (contain) into an `inner` square centred in a
/// `size` square.
#[must_use]
pub fn render_svg(data: &[u8], size: u32, inner: u32) -> Option<Pixmap> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let mut pm = Pixmap::new(size, size)?;
    let ts = tree.size();
    let s = u32_to_f32(size);
    let i = u32_to_f32(inner.min(size));
    let k = (i / ts.width()).min(i / ts.height());
    let tx = (s - ts.width() * k) / 2.0;
    let ty = (s - ts.height() * k) / 2.0;
    let transform = tiny_skia::Transform::from_row(k, 0.0, 0.0, k, tx, ty);
    resvg::render(&tree, transform, &mut pm.as_mut());
    Some(pm)
}

/// Scales `img` to cover a `size` square (centre crop) and premultiplies.
#[must_use]
pub fn cover(img: &DynamicImage, size: u32) -> Option<Pixmap> {
    premultiplied(
        img.resize_to_fill(size, size, FilterType::CatmullRom)
            .to_rgba8(),
    )
}

/// Scales `img` to fit inside an `inner` square (aspect ratio kept), centred
/// on a transparent `size` square, and premultiplies.
#[must_use]
pub fn contain(img: &DynamicImage, size: u32, inner: u32) -> Option<Pixmap> {
    let inner = inner.clamp(1, size);
    let scaled = img.resize(inner, inner, FilterType::CatmullRom).to_rgba8();
    let mut canvas = RgbaImage::new(size, size);
    let x = (size - scaled.width()) / 2;
    let y = (size - scaled.height()) / 2;
    image::imageops::overlay(&mut canvas, &scaled, i64::from(x), i64::from(y));
    premultiplied(canvas)
}

/// Converts straight RGBA to tiny-skia's premultiplied pixmap.
fn premultiplied(img: RgbaImage) -> Option<Pixmap> {
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            // Exact: (c * a + 127) / 255 <= 255.
            *c = u8::try_from((u16::from(*c) * a + 127) / 255).unwrap_or(u8::MAX);
        }
    }
    tiny_skia::IntSize::from_wh(w, h).and_then(|s| Pixmap::from_vec(data, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(width: u32, height: u32, px: [u8; 4]) -> Image {
        let n = usize::try_from(width * height).unwrap_or(0);
        Image::Rgba {
            width,
            height,
            pixels: px.repeat(n).into(),
        }
    }

    #[test]
    fn opaque_images_are_photos_cover_fitted() {
        let mut cache = ImageCache::default();
        let r = cache.get(&rgba(4, 2, [255, 0, 0, 255]), 16);
        let r = r.as_deref();
        assert_eq!(r.map(|r| r.shape), Some(Shape::Photo));
        let pm = r.map(|r| &r.pixmap);
        assert_eq!(pm.map(|p| (p.width(), p.height())), Some((16, 16)));
        // Covered: even the corners are filled.
        assert!(
            pm.and_then(|p| p.pixel(0, 0))
                .is_some_and(|p| p.alpha() == 255)
        );
    }

    #[test]
    fn see_through_images_are_icons_inset_whole() {
        // A 10x10 icon whose outer ring of pixels is transparent.
        let mut px = Vec::new();
        for y in 0..10u32 {
            for x in 0..10u32 {
                let edge = x == 0 || y == 0 || x == 9 || y == 9;
                px.extend_from_slice(&if edge { [0, 0, 0, 0] } else { [0, 0, 255, 255] });
            }
        }
        let img = Image::Rgba {
            width: 10,
            height: 10,
            pixels: px.into(),
        };
        let mut cache = ImageCache::default();
        let r = cache.get(&img, 40);
        let r = r.as_deref();
        assert_eq!(r.map(|r| r.shape), Some(Shape::Icon));
        let pm = r.map(|r| &r.pixmap);
        // Inset: the avatar's corners stay clear, the centre holds the icon.
        assert!(
            pm.and_then(|p| p.pixel(2, 2))
                .is_some_and(|p| p.alpha() == 0)
        );
        assert!(
            pm.and_then(|p| p.pixel(20, 20))
                .is_some_and(|p| p.blue() > 200)
        );
    }

    #[test]
    fn inset_keeps_a_square_icon_inside_the_circle() {
        // A square of side ICON_CONTENT * d fits in a circle of diameter d
        // when ICON_CONTENT <= 1/sqrt(2).
        const { assert!(m::ICON_CONTENT <= std::f32::consts::FRAC_1_SQRT_2) };
        assert_eq!(inset_side(40), 28);
    }

    #[test]
    fn premultiplies_straight_alpha() {
        let pm = premultiplied(RgbaImage::from_raw(1, 1, vec![255, 0, 0, 128]).unwrap_or_default());
        let p = pm.and_then(|p| p.pixel(0, 0));
        assert!(p.is_some_and(|p| p.alpha() == 128 && (127..=129).contains(&p.red())));
    }

    #[test]
    fn bad_images_are_none_and_cached() {
        let mut cache = ImageCache::default();
        let img = Image::Path("/nonexistent/x.png".into());
        assert!(cache.get(&img, 32).is_none());
        assert_eq!(cache.entries.len(), 1);
        let bad = Image::Rgba {
            width: 10,
            height: 10,
            pixels: vec![0u8; 3].into(),
        };
        assert!(cache.get(&bad, 32).is_none());
    }

    #[test]
    fn svg_renders_inset() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#00ff00"/></svg>"##;
        let pm = render_svg(svg, 24, 12);
        assert!(pm.as_ref().is_some_and(|p| {
            p.pixel(12, 12)
                .is_some_and(|c| c.green() == 255 && c.alpha() == 255)
        }));
        assert!(pm.is_some_and(|p| p.pixel(1, 1).is_some_and(|c| c.alpha() == 0)));
    }
}
