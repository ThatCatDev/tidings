//! The look of the panel and toasts: shapes, borders, transparency, shadow,
//! type and colour overrides.
//!
//! A [`Look`] is plain, already-validated data. It comes from a theme file
//! (see [`crate::config::theme`]) layered over the defaults, which are the
//! built-in "glass" theme. The UI turns it into paint and geometry; the
//! colour overrides are applied to the [`Theme`] by the daemon.

use crate::theme::{Mode, Rgb, Theme};

/// A colour in a look: a slot of the current colour theme (so it follows
/// the colours file, `colors.json`), or a fixed colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Paint {
    Surface,
    SurfaceVariant,
    OnSurface,
    OnSurfaceVariant,
    Primary,
    OnPrimary,
    Error,
    Outline,
    /// A lightening tint: `on_surface` in dark mode, white in light mode.
    Tint,
    Fixed(Rgb),
}

impl Paint {
    /// The theme slot names [`Paint::parse`] accepts, besides `#rrggbb`.
    pub const NAMES: [&str; 9] = [
        "surface",
        "surface_variant",
        "on_surface",
        "on_surface_variant",
        "primary",
        "on_primary",
        "error",
        "outline",
        "tint",
    ];

    /// A slot name (see [`Paint::NAMES`]) or `#rrggbb`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.starts_with('#') {
            return Rgb::from_hex(text).map(Self::Fixed);
        }
        Some(match text {
            "surface" => Self::Surface,
            "surface_variant" => Self::SurfaceVariant,
            "on_surface" => Self::OnSurface,
            "on_surface_variant" => Self::OnSurfaceVariant,
            "primary" => Self::Primary,
            "on_primary" => Self::OnPrimary,
            "error" => Self::Error,
            "outline" => Self::Outline,
            "tint" => Self::Tint,
            _ => return None,
        })
    }

    /// The colour this paint stands for under `theme`.
    #[must_use]
    pub fn resolve(self, theme: &Theme) -> Rgb {
        match self {
            Self::Surface => theme.surface,
            Self::SurfaceVariant => theme.surface_variant,
            Self::OnSurface => theme.on_surface,
            Self::OnSurfaceVariant => theme.on_surface_variant,
            Self::Primary => theme.primary,
            Self::OnPrimary => theme.on_primary,
            Self::Error => theme.error,
            Self::Outline => theme.outline,
            Self::Tint => match theme.mode {
                Mode::Dark => theme.on_surface,
                Mode::Light => Rgb(255, 255, 255),
            },
            Self::Fixed(rgb) => rgb,
        }
    }
}

/// A line drawn just inside a shape's edge. A zero width draws nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    /// Logical pixels.
    pub width: f32,
    pub paint: Paint,
    pub opacity: f32,
}

/// Text: the font and how big and how far apart lines are.
#[derive(Debug, Clone, PartialEq)]
pub struct Typography {
    /// Font family; `None` picks the system UI font.
    pub family: Option<String>,
    /// Multiplies every text size.
    pub scale: f32,
    /// Line spacing as a multiple of the text size.
    pub line_height: f32,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            family: None,
            scale: 1.0,
            line_height: 1.3,
        }
    }
}

/// The Notification Centre panel's sheet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelLook {
    /// Sheet opacity; `None` follows `surfaceAlpha` in `colors.json`.
    pub opacity: Option<f32>,
    pub radius: f32,
    pub border: Border,
    /// Ask the compositor to blur behind the sheet.
    pub blur: bool,
}

/// Notification cards in the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardLook {
    /// Largest corner radius; a short card is a pill (radius = height / 2)
    /// until it reaches this.
    pub radius: f32,
    pub paint: Paint,
    pub opacity: f32,
    pub hover_opacity: f32,
    pub border: Border,
}

/// A toast's drop shadow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub paint: Paint,
    /// Opacity of the unblurred shape (the shadow is half this at the edge).
    pub opacity: f32,
    /// How far the shadow fades out beyond the toast (logical px).
    pub size: f32,
    /// Downward shift (logical px).
    pub offset: f32,
}

/// Room around each toast for its shadow: `shadow.size + shadow.offset`
/// must fit in it.
pub const SHADOW_ROOM: f32 = 12.0;

/// Toast pop-ups.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToastLook {
    /// Largest corner radius; a collapsed toast is a pill at the default.
    pub radius: f32,
    pub paint: Paint,
    pub opacity: f32,
    pub hover_opacity: f32,
    pub expanded_opacity: f32,
    /// Tint laid over a hovered or expanded toast so it reads lighter.
    pub sheen: f32,
    pub border: Border,
    pub shadow: Shadow,
    /// Ask the compositor to blur behind toasts.
    pub blur: bool,
}

/// Pill buttons (actions, "Clear all", Do Not Disturb, expand).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonLook {
    /// Corner radius; a button is a pill when this reaches half its height.
    pub radius: f32,
    pub paint: Paint,
    pub opacity: f32,
    pub hover_opacity: f32,
}

/// Colours that replace the ones from `colors.json`. Unset ones still
/// follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorOverrides {
    pub mode: Option<Mode>,
    pub surface: Option<Rgb>,
    pub surface_variant: Option<Rgb>,
    pub on_surface: Option<Rgb>,
    pub on_surface_variant: Option<Rgb>,
    pub primary: Option<Rgb>,
    pub on_primary: Option<Rgb>,
    pub error: Option<Rgb>,
    pub on_error: Option<Rgb>,
    pub outline: Option<Rgb>,
}

impl ColorOverrides {
    /// `theme` with these colours in place of its own.
    #[must_use]
    pub fn apply(&self, mut theme: Theme) -> Theme {
        let slots = [
            (self.surface, &mut theme.surface),
            (self.surface_variant, &mut theme.surface_variant),
            (self.on_surface, &mut theme.on_surface),
            (self.on_surface_variant, &mut theme.on_surface_variant),
            (self.primary, &mut theme.primary),
            (self.on_primary, &mut theme.on_primary),
            (self.error, &mut theme.error),
            (self.on_error, &mut theme.on_error),
            (self.outline, &mut theme.outline),
        ];
        for (value, slot) in slots {
            if let Some(v) = value {
                *slot = v;
            }
        }
        if let Some(mode) = self.mode {
            theme.mode = mode;
        }
        theme
    }
}

/// Everything a theme controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub font: Typography,
    pub colors: ColorOverrides,
    pub panel: PanelLook,
    pub card: CardLook,
    pub toast: ToastLook,
    pub button: ButtonLook,
}

impl Look {
    /// The colour theme to paint with: `theme` (from `colors.json`) with
    /// this look's colour overrides and panel opacity applied.
    #[must_use]
    pub fn apply_to(&self, theme: Theme) -> Theme {
        let mut theme = self.colors.apply(theme);
        if let Some(opacity) = self.panel.opacity {
            theme.surface_alpha = opacity;
        }
        theme
    }
}

impl Default for Look {
    /// The "glass" theme: frosted, translucent, pill-shaped.
    fn default() -> Self {
        Self {
            font: Typography::default(),
            colors: ColorOverrides::default(),
            panel: PanelLook {
                opacity: None,
                radius: 20.0,
                border: Border {
                    width: 1.0,
                    paint: Paint::Outline,
                    opacity: 0.35,
                },
                blur: true,
            },
            card: CardLook {
                radius: 28.0,
                paint: Paint::Tint,
                opacity: 0.075,
                hover_opacity: 0.11,
                border: Border {
                    width: 1.0,
                    paint: Paint::Tint,
                    opacity: 0.06,
                },
            },
            toast: ToastLook {
                radius: 29.0,
                paint: Paint::Surface,
                opacity: 0.46,
                hover_opacity: 0.54,
                expanded_opacity: 0.50,
                sheen: 0.04,
                border: Border {
                    width: 1.0,
                    paint: Paint::Tint,
                    opacity: 0.16,
                },
                shadow: Shadow {
                    paint: Paint::Fixed(Rgb(0, 0, 0)),
                    opacity: 0.28,
                    size: 9.0,
                    offset: 3.0,
                },
                blur: true,
            },
            button: ButtonLook {
                radius: 14.0,
                paint: Paint::Tint,
                opacity: 0.10,
                hover_opacity: 0.17,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_parse_names_and_hex() {
        for name in Paint::NAMES {
            assert!(Paint::parse(name).is_some(), "{name}");
        }
        assert_eq!(Paint::parse(" tint "), Some(Paint::Tint));
        assert_eq!(
            Paint::parse("#ff0080"),
            Some(Paint::Fixed(Rgb(255, 0, 128)))
        );
        assert_eq!(Paint::parse("#ff00"), None);
        assert_eq!(Paint::parse("red"), None);
    }

    #[test]
    fn tint_follows_mode() {
        let dark = Theme::dark();
        assert_eq!(Paint::Tint.resolve(&dark), dark.on_surface);
        assert_eq!(Paint::Tint.resolve(&Theme::light()), Rgb(255, 255, 255));
        assert_eq!(Paint::Fixed(Rgb(1, 2, 3)).resolve(&dark), Rgb(1, 2, 3));
    }

    #[test]
    fn overrides_replace_only_what_they_set() {
        let look = Look {
            colors: ColorOverrides {
                mode: Some(Mode::Light),
                primary: Some(Rgb(1, 2, 3)),
                ..ColorOverrides::default()
            },
            panel: PanelLook {
                opacity: Some(0.9),
                ..Look::default().panel
            },
            ..Look::default()
        };
        let t = look.apply_to(Theme::dark());
        assert_eq!(t.mode, Mode::Light);
        assert_eq!(t.primary, Rgb(1, 2, 3));
        assert_eq!(t.surface, Theme::dark().surface);
        assert!((t.surface_alpha - 0.9).abs() < f32::EPSILON);
        // The default look leaves colors.json alone.
        assert_eq!(Look::default().apply_to(Theme::light()), Theme::light());
    }

    #[test]
    fn default_shadow_fits_its_room() {
        let s = Look::default().toast.shadow;
        assert!(s.size + s.offset <= SHADOW_ROOM);
    }
}
