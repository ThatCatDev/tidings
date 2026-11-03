//! Where the colours come from.
//!
//! - **A colours file** (`colors.json`): any palette tool can write it, and
//!   it always wins when it exists.
//! - **Noctalia**: when the Noctalia shell is in use, tidings reads the
//!   palette Noctalia generates (its Hyprland colour file) and follows the
//!   wallpaper with no hook or script.
//! - Neither: the built-in dark colours.
//!
//! Each source names the one file to watch for changes and turns it into a
//! [`Theme`].

use std::path::{Path, PathBuf};

use tidings_core::theme::{Rgb, Theme};

/// The `theme.colors` value that picks Noctalia's palette.
pub const NOCTALIA: &str = "noctalia";

/// A source of colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorSource {
    /// A `colors.json` file.
    File(PathBuf),
    /// Noctalia's generated palette file.
    Noctalia(PathBuf),
}

impl ColorSource {
    /// Picks the source: an explicit file or `"noctalia"` when given
    /// (`choice`); otherwise `colors_json` if it exists, else Noctalia's
    /// palette if *that* exists, else `colors_json` (watched, so creating it
    /// later takes effect).
    #[must_use]
    pub fn pick(choice: Option<&Path>, colors_json: PathBuf, noctalia: Option<PathBuf>) -> Self {
        match choice {
            Some(p) if p == Path::new(NOCTALIA) => match noctalia {
                Some(palette) => Self::Noctalia(palette),
                None => Self::File(colors_json),
            },
            Some(p) => Self::File(p.to_path_buf()),
            None => match noctalia {
                Some(palette) if !colors_json.exists() && palette.exists() => {
                    Self::Noctalia(palette)
                }
                _ => Self::File(colors_json),
            },
        }
    }

    /// The file whose changes mean new colours.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::File(p) | Self::Noctalia(p) => p,
        }
    }

    /// The colours now. A missing or unreadable source gives the defaults.
    #[must_use]
    pub fn load(&self) -> Theme {
        let text = match std::fs::read_to_string(self.path()) {
            Ok(text) => text,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("{}: {e}; using default colours", self.path().display());
                }
                return Theme::default();
            }
        };
        match self {
            Self::File(path) => Theme::from_json(&text).unwrap_or_else(|e| {
                tracing::warn!("{}: {e}; using default colours", path.display());
                Theme::default()
            }),
            Self::Noctalia(path) => from_noctalia(&text).unwrap_or_else(|| {
                tracing::warn!(
                    "{}: no palette in it; using default colours",
                    path.display()
                );
                Theme::default()
            }),
        }
    }
}

impl std::fmt::Display for ColorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(p) => write!(f, "{}", p.display()),
            Self::Noctalia(p) => write!(f, "Noctalia's palette ({})", p.display()),
        }
    }
}

/// Colours from Noctalia's palette file, which defines lines like
/// `local primary = "rgb(ffb4a7)"`. The accent comes from it; the surfaces
/// stay tidings' own for the mode (a wallpaper-derived surface can make the
/// glass illegible), and the mode is read from how light Noctalia's surface
/// is. `None` if there is no primary colour.
#[must_use]
pub fn from_noctalia(palette: &str) -> Option<Theme> {
    let color = |name: &str| {
        palette.lines().find_map(|line| {
            let value = line
                .trim()
                .strip_prefix("local ")?
                .strip_prefix(name)?
                .trim_start()
                .strip_prefix('=')?
                .trim();
            let hex = value.strip_prefix("\"rgb(")?.strip_suffix(")\"")?;
            Rgb::from_hex(&format!("#{hex}"))
        })
    };
    let primary = color("primary")?;
    let light = color("surface").is_some_and(|s| s.luminance() > 0.5);
    let base = if light { Theme::light() } else { Theme::dark() };
    Some(Theme { primary, ..base })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidings_core::theme::Mode;

    const DARK: &str = include_str!("../tests/fixtures/noctalia.lua");

    #[test]
    fn noctalia_palettes_give_the_accent_and_the_mode() {
        let t = from_noctalia(DARK).unwrap();
        assert_eq!(t.mode, Mode::Dark);
        assert_eq!(t.primary, Rgb(0xff, 0xb4, 0xa7), "upper-case hex too");
        assert_eq!(t.surface, Theme::dark().surface, "tidings' own surfaces");

        let light = DARK.replace("rgb(1f0f0d)", "rgb(fff8f6)");
        let t = from_noctalia(&light).unwrap();
        assert_eq!((t.mode, t.surface), (Mode::Light, Theme::light().surface));

        assert_eq!(from_noctalia("local surface = \"rgb(000000)\""), None);
        assert_eq!(from_noctalia("local primary = rgb(ffffff)"), None);
        assert_eq!(from_noctalia("local primary_dim = \"rgb(ffffff)\""), None);
    }

    #[test]
    fn picking_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("colors.json");
        let palette = dir.path().join("noctalia.lua");
        let pick = |choice: Option<&str>| {
            ColorSource::pick(choice.map(Path::new), json.clone(), Some(palette.clone()))
        };
        // Nothing there yet: colors.json, watched for when it appears.
        assert_eq!(pick(None), ColorSource::File(json.clone()));
        // Noctalia's palette, when there is no colours file.
        std::fs::write(&palette, DARK).unwrap();
        assert_eq!(pick(None), ColorSource::Noctalia(palette.clone()));
        // A colours file always wins.
        std::fs::write(&json, "{}").unwrap();
        assert_eq!(pick(None), ColorSource::File(json.clone()));
        // Unless Noctalia is asked for; any other value is a file.
        assert_eq!(
            pick(Some("noctalia")),
            ColorSource::Noctalia(palette.clone())
        );
        assert_eq!(
            pick(Some("/x/mine.json")),
            ColorSource::File("/x/mine.json".into())
        );
        // Asked for without knowing where it would be: the colours file.
        assert_eq!(
            ColorSource::pick(Some(Path::new("noctalia")), json.clone(), None),
            ColorSource::File(json.clone())
        );
    }

    #[test]
    fn loading_falls_back_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let palette = dir.path().join("noctalia.lua");
        assert_eq!(
            ColorSource::Noctalia(palette.clone()).load(),
            Theme::default()
        );
        std::fs::write(&palette, "nothing here").unwrap();
        assert_eq!(
            ColorSource::Noctalia(palette.clone()).load(),
            Theme::default()
        );
        std::fs::write(&palette, DARK).unwrap();
        assert_eq!(
            ColorSource::Noctalia(palette.clone()).load().primary,
            Rgb(0xff, 0xb4, 0xa7)
        );
        let json = dir.path().join("colors.json");
        std::fs::write(&json, "{ nope").unwrap();
        assert_eq!(ColorSource::File(json).load(), Theme::default());
        // Unreadable (a directory): defaults too.
        assert_eq!(
            ColorSource::File(dir.path().into()).load(),
            Theme::default()
        );
    }
}
