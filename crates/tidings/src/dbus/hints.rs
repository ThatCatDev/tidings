//! Parsing the `Notify` hints dictionary and the `app_icon` argument.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::sync::Arc;

use tidings_core::model::{Image, Urgency};
use zbus::{OwnedVariant, Variant};

/// Largest accepted `image-data` dimension; bigger images are ignored.
const MAX_IMAGE_SIDE: u32 = 4096;

/// The hints tidings understands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hints {
    /// `urgency` (byte 0..=2).
    pub urgency: Urgency,
    /// `image-data` / `image_data` / `image-path` / `image_path`.
    pub image: Option<Image>,
    /// `icon_data`: deprecated pixel icon, used when `app_icon` is empty.
    pub icon_data: Option<Image>,
    /// `desktop-entry`, without any `.desktop` suffix.
    pub desktop_entry: Option<String>,
    /// `transient`: not kept in history.
    pub transient: bool,
    /// `resident`: not removed when an action is invoked.
    pub resident: bool,
}

/// Parses the hints dictionary. Unknown or malformed hints are ignored.
#[must_use]
pub fn parse_hints<S: BuildHasher>(hints: &HashMap<String, OwnedVariant, S>) -> Hints {
    let get = |key: &str| hints.get(key).map(|v| unwrap_variant(v));
    let image = ["image-data", "image_data"]
        .into_iter()
        .find_map(|k| get(k).and_then(image_data))
        .or_else(|| {
            ["image-path", "image_path"]
                .into_iter()
                .find_map(|k| get(k).and_then(as_str).and_then(image_from_str))
        });
    Hints {
        urgency: get("urgency")
            .and_then(as_int)
            .map_or(Urgency::Normal, |u| match u {
                0 => Urgency::Low,
                2.. => Urgency::Critical,
                _ => Urgency::Normal,
            }),
        image,
        icon_data: get("icon_data").and_then(image_data),
        desktop_entry: get("desktop-entry")
            .and_then(as_str)
            .map(|s| s.trim().trim_end_matches(".desktop").to_owned())
            .filter(|s| !s.is_empty()),
        transient: get("transient").is_some_and(as_bool),
        resident: get("resident").is_some_and(as_bool),
    }
}

/// Interprets the `app_icon` argument: a `file://` URI or absolute path, or
/// an icon theme name. Empty means none.
#[must_use]
pub fn image_from_str(s: &str) -> Option<Image> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else if let Some(path) = s.strip_prefix("file://") {
        Some(Image::Path(PathBuf::from(percent_decode(path))))
    } else if s.starts_with('/') {
        Some(Image::Path(PathBuf::from(s)))
    } else {
        Some(Image::IconName(s.to_owned()))
    }
}

/// Decodes `%XX` escapes in a URI path; malformed escapes are kept verbatim.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Strips nested `v` wrappers some clients add.
fn unwrap_variant<'a>(v: &'a Variant<'a>) -> &'a Variant<'a> {
    match v {
        Variant::Variant(inner) => unwrap_variant(inner),
        other => other,
    }
}

fn as_str<'a>(v: &'a Variant<'_>) -> Option<&'a str> {
    match v {
        Variant::Str(s) => Some(s.as_str()),
        _ => None,
    }
}

fn as_int(v: &Variant<'_>) -> Option<i64> {
    match *v {
        Variant::U8(n) => Some(i64::from(n)),
        Variant::I16(n) => Some(i64::from(n)),
        Variant::U16(n) => Some(i64::from(n)),
        Variant::I32(n) => Some(i64::from(n)),
        Variant::U32(n) => Some(i64::from(n)),
        Variant::I64(n) => Some(n),
        Variant::U64(n) => i64::try_from(n).ok(),
        _ => None,
    }
}

fn as_bool(v: &Variant<'_>) -> bool {
    match v {
        Variant::Bool(b) => *b,
        other => as_int(other).is_some_and(|n| n != 0),
    }
}

/// Decodes an `(iiibiiay)` image-data structure.
fn image_data(variant: &Variant<'_>) -> Option<Image> {
    let Variant::Structure(structure) = variant else {
        return None;
    };
    let [width, height, stride, alpha, bits, channels, data] = structure.fields() else {
        return None;
    };
    let int = |field: &Variant<'_>| match field {
        Variant::I32(n) => Some(*n),
        _ => None,
    };
    let (Variant::Bool(has_alpha), Variant::Array(data)) = (alpha, data) else {
        return None;
    };
    let bytes: Vec<u8> = data
        .inner()
        .iter()
        .map(|byte| match byte {
            Variant::U8(byte) => Some(*byte),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let raw = RawImage {
        width: int(width)?,
        height: int(height)?,
        rowstride: int(stride)?,
        has_alpha: *has_alpha,
        bits_per_sample: int(bits)?,
        channels: int(channels)?,
    };
    raw.to_rgba(&bytes)
}

/// The header fields of an `image-data` hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawImage {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Bytes between row starts.
    pub rowstride: i32,
    /// Whether pixels carry an alpha sample.
    pub has_alpha: bool,
    /// Must be 8.
    pub bits_per_sample: i32,
    /// 4 with alpha, 3 without.
    pub channels: i32,
}

impl RawImage {
    /// Converts `data` to straight (non-premultiplied) RGBA8.
    ///
    /// The spec's pixel data is `GdkPixbuf` layout: rows `rowstride` bytes
    /// apart, RGB or RGBA with straight alpha, 8 bits per sample. Returns
    /// `None` for inconsistent headers or short data.
    #[must_use]
    pub fn to_rgba(&self, data: &[u8]) -> Option<Image> {
        let width = u32::try_from(self.width)
            .ok()
            .filter(|w| (1..=MAX_IMAGE_SIDE).contains(w))?;
        let height = u32::try_from(self.height)
            .ok()
            .filter(|h| (1..=MAX_IMAGE_SIDE).contains(h))?;
        let channels = match (self.has_alpha, self.channels) {
            (true, 4) => 4,
            (false, 3) => 3,
            _ => return None,
        };
        if self.bits_per_sample != 8 {
            return None;
        }
        let (w, h) = (width as usize, height as usize);
        let stride = usize::try_from(self.rowstride).ok()?;
        let row_bytes = w * channels;
        if stride < row_bytes || data.len() < stride * (h - 1) + row_bytes {
            return None;
        }
        let mut pixels = Vec::with_capacity(w * h * 4);
        for row in data.chunks(stride).take(h) {
            let row = &row[..row_bytes];
            if channels == 4 {
                pixels.extend_from_slice(row);
            } else {
                for px in row.as_chunks::<3>().0 {
                    pixels.extend_from_slice(&[px[0], px[1], px[2], 0xff]);
                }
            }
        }
        Some(Image::Rgba {
            width,
            height,
            pixels: Arc::from(pixels),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn owned(v: Variant<'_>) -> OwnedVariant {
        OwnedVariant::try_from(v).unwrap()
    }

    fn hints(pairs: Vec<(&str, Variant<'_>)>) -> HashMap<String, OwnedVariant> {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), owned(v)))
            .collect()
    }

    pub(crate) fn image_variant(
        w: i32,
        h: i32,
        stride: i32,
        alpha: bool,
        data: Vec<u8>,
    ) -> Variant<'static> {
        let channels = if alpha { 4 } else { 3 };
        Variant::from((w, h, stride, alpha, 8i32, channels, data))
    }

    #[test]
    fn defaults() {
        assert_eq!(parse_hints(&HashMap::new()), Hints::default());
    }

    #[test]
    fn urgency_flags_and_desktop_entry() {
        let h = parse_hints(&hints(vec![
            ("urgency", Variant::U8(2)),
            ("transient", Variant::Bool(true)),
            ("resident", Variant::U32(1)),
            ("desktop-entry", Variant::from("org.gnome.Nautilus.desktop")),
        ]));
        assert_eq!(h.urgency, Urgency::Critical);
        assert!(h.transient);
        assert!(h.resident);
        assert_eq!(h.desktop_entry.as_deref(), Some("org.gnome.Nautilus"));
        assert_eq!(
            parse_hints(&hints(vec![("urgency", Variant::U8(0))])).urgency,
            Urgency::Low
        );
        assert_eq!(
            parse_hints(&hints(vec![("urgency", Variant::I32(1))])).urgency,
            Urgency::Normal
        );
        let wrong = parse_hints(&hints(vec![
            ("urgency", Variant::from("high")),
            ("transient", Variant::from("yes")),
            ("desktop-entry", Variant::from("  ")),
        ]));
        assert_eq!(wrong, Hints::default());
    }

    #[test]
    fn nested_variants_are_unwrapped() {
        let h = parse_hints(&hints(vec![(
            "urgency",
            Variant::Variant(Box::new(Variant::U8(0))),
        )]));
        assert_eq!(h.urgency, Urgency::Low);
    }

    #[test]
    fn rgba_with_row_padding() {
        // 2x2 RGBA, stride 12 (4 bytes padding per row).
        let mut data = Vec::new();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0]);
        data.extend_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]); // last row unpadded
        let h = parse_hints(&hints(vec![(
            "image-data",
            image_variant(2, 2, 12, true, data),
        )]));
        assert_eq!(
            h.image,
            Some(Image::Rgba {
                width: 2,
                height: 2,
                pixels: Arc::from((1..=16).collect::<Vec<u8>>()),
            })
        );
    }

    #[test]
    fn rgb_gets_opaque_alpha() {
        let data = vec![10, 20, 30, 40, 50, 60, 0, 0];
        let h = parse_hints(&hints(vec![(
            "image_data",
            image_variant(2, 1, 8, false, data),
        )]));
        assert_eq!(
            h.image,
            Some(Image::Rgba {
                width: 2,
                height: 1,
                pixels: Arc::from(vec![10, 20, 30, 255, 40, 50, 60, 255]),
            })
        );
    }

    #[test]
    fn bad_image_headers_are_rejected() {
        let ok = RawImage {
            width: 1,
            height: 1,
            rowstride: 4,
            has_alpha: true,
            bits_per_sample: 8,
            channels: 4,
        };
        assert!(ok.to_rgba(&[0; 4]).is_some());
        assert_eq!(ok.to_rgba(&[0; 3]), None, "short data");
        assert_eq!(RawImage { width: 0, ..ok }.to_rgba(&[0; 4]), None);
        assert_eq!(RawImage { height: -1, ..ok }.to_rgba(&[0; 4]), None);
        assert_eq!(RawImage { rowstride: 3, ..ok }.to_rgba(&[0; 4]), None);
        assert_eq!(
            RawImage {
                bits_per_sample: 16,
                ..ok
            }
            .to_rgba(&[0; 4]),
            None
        );
        assert_eq!(RawImage { channels: 3, ..ok }.to_rgba(&[0; 4]), None);
        assert_eq!(
            RawImage { width: 5000, ..ok }.to_rgba(&vec![0; 20_000]),
            None
        );
        let h = parse_hints(&hints(vec![("image-data", Variant::from((1i32, 2i32)))]));
        assert_eq!(h.image, None, "wrong structure shape");
    }

    #[test]
    fn image_precedence_and_paths() {
        let h = parse_hints(&hints(vec![
            ("image-path", Variant::from("file:///tmp/a%20b.png")),
            ("icon_data", image_variant(1, 1, 3, false, vec![1, 2, 3])),
        ]));
        assert_eq!(h.image, Some(Image::Path("/tmp/a b.png".into())));
        assert!(matches!(h.icon_data, Some(Image::Rgba { .. })));
        let h = parse_hints(&hints(vec![
            ("image_path", Variant::from("dialog-warning")),
            ("image-data", image_variant(1, 1, 3, false, vec![1, 2, 3])),
        ]));
        assert!(
            matches!(h.image, Some(Image::Rgba { .. })),
            "image-data wins"
        );
    }

    #[test]
    fn app_icon_strings() {
        assert_eq!(image_from_str(""), None);
        assert_eq!(image_from_str("  "), None);
        assert_eq!(
            image_from_str("firefox"),
            Some(Image::IconName("firefox".into()))
        );
        assert_eq!(
            image_from_str("/usr/share/a.svg"),
            Some(Image::Path("/usr/share/a.svg".into()))
        );
        assert_eq!(
            image_from_str("file:///x/%zz%41"),
            Some(Image::Path("/x/%zzA".into()))
        );
    }
}
