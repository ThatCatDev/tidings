//! Parsing `colors.json` into a [`Theme`], with built-in defaults.
//!
//! Every key is optional: missing keys take the defaults for the file's
//! `mode` (dark when absent). A malformed file is an error so the caller can
//! log it and fall back to [`Theme::default`].
//!
//! ```json
//! { "mode": "dark", "surface": "#18181d", "primary": "#a8c7fa", "surfaceAlpha": 0.62 }
//! ```

use serde::Deserialize;

use crate::theme::{Mode, Rgb, Theme};

/// Why a colours file was rejected.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    /// Not valid JSON, or a value of the wrong type.
    #[error("invalid colors.json: {0}")]
    Json(#[from] serde_json::Error),
    /// A colour that is not `#rrggbb`.
    #[error("invalid colour {value:?} for {key}: expected #rrggbb")]
    Colour {
        /// The JSON key.
        key: &'static str,
        /// The rejected value.
        value: String,
    },
    /// A mode other than dark/light.
    #[error("invalid mode {0:?}: expected \"dark\" or \"light\"")]
    Mode(String),
    /// A NaN or infinite alpha.
    #[error("surfaceAlpha {0} is not a finite number")]
    Alpha(f64),
}

impl Rgb {
    /// Parses `#rrggbb` (the leading `#` is optional).
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self(byte(0)?, byte(2)?, byte(4)?))
    }

    /// Formats as `#rrggbb`.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Relative luminance (WCAG): 0 for black, 1 for white.
    #[must_use]
    pub fn luminance(self) -> f32 {
        let linear = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.0) + 0.7152 * linear(self.1) + 0.0722 * linear(self.2)
    }
}

/// Glass alpha used when the file doesn't say.
pub const DEFAULT_SURFACE_ALPHA: f32 = 0.62;

impl Theme {
    /// Built-in dark theme (the Noctalia script in contrib/ uses the same
    /// dark surfaces).
    #[must_use]
    pub fn dark() -> Self {
        Self {
            mode: Mode::Dark,
            surface: Rgb(24, 24, 29),
            surface_variant: Rgb(34, 35, 42),
            on_surface: Rgb(228, 225, 231),
            on_surface_variant: Rgb(163, 171, 189),
            primary: Rgb(168, 199, 250),
            on_primary: Rgb(11, 14, 19),
            error: Rgb(211, 69, 59),
            on_error: Rgb(255, 255, 255),
            outline: Rgb(160, 178, 210),
            surface_alpha: DEFAULT_SURFACE_ALPHA,
        }
    }

    /// Built-in light theme.
    #[must_use]
    pub fn light() -> Self {
        Self {
            mode: Mode::Light,
            surface: Rgb(249, 249, 255),
            surface_variant: Rgb(238, 241, 249),
            on_surface: Rgb(23, 26, 33),
            on_surface_variant: Rgb(85, 96, 122),
            primary: Rgb(65, 95, 145),
            on_primary: Rgb(255, 255, 255),
            error: Rgb(192, 38, 31),
            on_error: Rgb(255, 255, 255),
            outline: Rgb(72, 95, 133),
            surface_alpha: DEFAULT_SURFACE_ALPHA,
        }
    }

    /// Parses a `colors.json` document.
    pub fn from_json(json: &str) -> Result<Self, ThemeError> {
        let raw: RawTheme = serde_json::from_str(json)?;
        let mut theme = match raw.mode.as_deref() {
            None | Some("dark") => Self::dark(),
            Some("light") => Self::light(),
            Some(other) => return Err(ThemeError::Mode(other.to_owned())),
        };
        let fields: [(&'static str, Option<String>, &mut Rgb); 9] = [
            ("surface", raw.surface, &mut theme.surface),
            (
                "surfaceVariant",
                raw.surface_variant,
                &mut theme.surface_variant,
            ),
            ("onSurface", raw.on_surface, &mut theme.on_surface),
            (
                "onSurfaceVariant",
                raw.on_surface_variant,
                &mut theme.on_surface_variant,
            ),
            ("primary", raw.primary, &mut theme.primary),
            ("onPrimary", raw.on_primary, &mut theme.on_primary),
            ("error", raw.error, &mut theme.error),
            ("onError", raw.on_error, &mut theme.on_error),
            ("outline", raw.outline, &mut theme.outline),
        ];
        for (key, value, slot) in fields {
            if let Some(value) = value {
                *slot = Rgb::from_hex(&value).ok_or(ThemeError::Colour { key, value })?;
            }
        }
        if let Some(alpha) = raw.surface_alpha {
            if !alpha.is_finite() {
                return Err(ThemeError::Alpha(alpha));
            }
            // Clamped to 0..=1 first, so narrowing to f32 is lossless enough.
            #[allow(clippy::cast_possible_truncation)]
            let alpha = alpha.clamp(0.0, 1.0) as f32;
            theme.surface_alpha = alpha;
        }
        Ok(theme)
    }
}

impl Default for Theme {
    /// The built-in dark theme.
    fn default() -> Self {
        Self::dark()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTheme {
    mode: Option<String>,
    surface: Option<String>,
    surface_variant: Option<String>,
    on_surface: Option<String>,
    on_surface_variant: Option<String>,
    primary: Option<String>,
    on_primary: Option<String>,
    error: Option<String>,
    on_error: Option<String>,
    outline: Option<String>,
    surface_alpha: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_runs_from_black_to_white() {
        assert!(Rgb(0, 0, 0).luminance().abs() < 1e-6);
        assert!((Rgb(255, 255, 255).luminance() - 1.0).abs() < 1e-4);
        // Green looks far brighter than blue.
        assert!(Rgb(0, 255, 0).luminance() > 5.0 * Rgb(0, 0, 255).luminance());
        assert!(Theme::dark().surface.luminance() < 0.1);
        assert!(Theme::light().surface.luminance() > 0.8);
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(Rgb::from_hex("#ffb4a7"), Some(Rgb(0xff, 0xb4, 0xa7)));
        assert_eq!(Rgb::from_hex("0A0b0C"), Some(Rgb(10, 11, 12)));
        assert_eq!(Rgb(1, 2, 255).to_hex(), "#0102ff");
        for bad in ["", "#fff", "#gggggg", "#1234567", "rgb(1,2,3)", "#ééé"] {
            assert_eq!(Rgb::from_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn full_file() {
        let json = r##"{
            "mode": "light", "surface": "#010203", "surfaceVariant": "#040506",
            "onSurface": "#070809", "onSurfaceVariant": "#0a0b0c", "primary": "#0d0e0f",
            "onPrimary": "#101112", "error": "#131415", "onError": "#161718",
            "outline": "#191a1b", "surfaceAlpha": 0.5, "extra": "ignored"
        }"##;
        let t = Theme::from_json(json).unwrap();
        assert_eq!(t.mode, Mode::Light);
        assert_eq!(t.surface, Rgb(1, 2, 3));
        assert_eq!(t.surface_variant, Rgb(4, 5, 6));
        assert_eq!(t.on_surface, Rgb(7, 8, 9));
        assert_eq!(t.on_surface_variant, Rgb(10, 11, 12));
        assert_eq!(t.primary, Rgb(13, 14, 15));
        assert_eq!(t.on_primary, Rgb(16, 17, 18));
        assert_eq!(t.error, Rgb(19, 20, 21));
        assert_eq!(t.on_error, Rgb(22, 23, 24));
        assert_eq!(t.outline, Rgb(25, 26, 27));
        assert!((t.surface_alpha - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn missing_keys_use_mode_defaults() {
        assert_eq!(Theme::from_json("{}").unwrap(), Theme::dark());
        let t = Theme::from_json(r##"{"mode":"light","primary":"#ff0000"}"##).unwrap();
        assert_eq!(
            t,
            Theme {
                primary: Rgb(255, 0, 0),
                ..Theme::light()
            }
        );
        assert_eq!(Theme::default(), Theme::dark());
        assert!((Theme::default().surface_alpha - 0.62).abs() < f32::EPSILON);
    }

    #[test]
    fn alpha_is_clamped() {
        let t = Theme::from_json(r#"{"surfaceAlpha": 3}"#).unwrap();
        assert!((t.surface_alpha - 1.0).abs() < f32::EPSILON);
        let t = Theme::from_json(r#"{"surfaceAlpha": -1}"#).unwrap();
        assert!(t.surface_alpha.abs() < f32::EPSILON);
    }

    #[test]
    fn invalid_files_are_errors() {
        assert!(matches!(
            Theme::from_json("not json"),
            Err(ThemeError::Json(_))
        ));
        assert!(matches!(Theme::from_json("[]"), Err(ThemeError::Json(_))));
        assert!(matches!(
            Theme::from_json(r#"{"primary":"red"}"#),
            Err(ThemeError::Colour { key: "primary", .. })
        ));
        assert!(matches!(
            Theme::from_json(r#"{"mode":"sepia"}"#),
            Err(ThemeError::Mode(_))
        ));
        assert!(matches!(
            Theme::from_json(r#"{"surface": 5}"#),
            Err(ThemeError::Json(_))
        ));
    }
}
