//! Icon and image loading: freedesktop icon lookup, SVG via resvg, raster
//! via `image`, all rasterised to the exact physical size and cached.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use tidings_core::model::Image;
use tiny_skia::Pixmap;

/// Bound on cached rasters before the cache is flushed.
const CACHE_LIMIT: usize = 256;

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
    entries: HashMap<(Key, u32), Option<Arc<Pixmap>>>,
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

    /// Returns `image` cover-fitted into a `size` x `size` pixmap, or `None`
    /// if it cannot be found or decoded (failures are cached too).
    pub fn get(&mut self, image: &Image, size: u32) -> Option<Arc<Pixmap>> {
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

    fn load(&self, image: &Image, size: u32) -> Option<Pixmap> {
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
                cover(&DynamicImage::ImageRgba8(img), size)
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

fn load_file(path: &Path, size: u32) -> Option<Pixmap> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
    if is_svg {
        render_svg(&std::fs::read(path).ok()?, size)
    } else {
        let img = image::open(path).ok()?;
        cover(&img, size)
    }
}

/// Renders SVG data fitted (contain) into a `size` square.
#[must_use]
pub fn render_svg(data: &[u8], size: u32) -> Option<Pixmap> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let mut pm = Pixmap::new(size, size)?;
    let ts = tree.size();
    let s = crate::geom::u32_to_f32(size);
    let k = (s / ts.width()).min(s / ts.height());
    let tx = (s - ts.width() * k) / 2.0;
    let ty = (s - ts.height() * k) / 2.0;
    let transform = tiny_skia::Transform::from_row(k, 0.0, 0.0, k, tx, ty);
    resvg::render(&tree, transform, &mut pm.as_mut());
    Some(pm)
}

/// Scales `img` to cover a `size` square (centre crop) and premultiplies.
#[must_use]
pub fn cover(img: &DynamicImage, size: u32) -> Option<Pixmap> {
    let filled = img
        .resize_to_fill(size, size, FilterType::CatmullRom)
        .to_rgba8();
    let mut data = filled.into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            // Exact: (c * a + 127) / 255 <= 255.
            *c = u8::try_from((u16::from(*c) * a + 127) / 255).unwrap_or(u8::MAX);
        }
    }
    tiny_skia::IntSize::from_wh(size, size).and_then(|s| Pixmap::from_vec(data, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_images_are_cover_fitted_and_premultiplied() {
        // 4x2 half-transparent red.
        let px: Vec<u8> = [255u8, 0, 0, 128].repeat(8);
        let img = Image::Rgba {
            width: 4,
            height: 2,
            pixels: px.into(),
        };
        let mut cache = ImageCache::default();
        let pm = cache.get(&img, 16);
        let pm = pm.as_deref();
        assert_eq!(pm.map(|p| (p.width(), p.height())), Some((16, 16)));
        let p = pm.and_then(|p| p.pixel(8, 8));
        assert!(p.is_some_and(|p| p.alpha() == 128 && p.red() <= 129 && p.red() >= 127));
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
    fn svg_renders() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#00ff00"/></svg>"##;
        let pm = render_svg(svg, 24);
        assert!(pm.is_some_and(|p| {
            p.pixel(12, 12)
                .is_some_and(|c| c.green() == 255 && c.alpha() == 255)
        }));
    }
}
