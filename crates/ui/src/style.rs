//! Visual constants and the theme → colour mapping.
//!
//! Layout refers to colours by [`Role`] (plus an alpha), never by value, so a
//! theme change only needs a repaint, not a re-layout. [`Palette`] resolves
//! roles against the current [`Theme`].

use tidings_core::theme::{Mode, Rgb, Theme};

/// Logical-pixel metrics for the panel, cards and toasts.
pub mod metrics {
    /// Panel width (the visible glass sheet).
    pub const PANEL_WIDTH: f32 = 380.0;
    /// Gap between the panel and the screen's right edge. The panel surface
    /// extends over this gap so the slide clips at the screen edge.
    pub const PANEL_MARGIN_RIGHT: f32 = 8.0;
    pub const PANEL_MARGIN_TOP: i32 = 52;
    pub const PANEL_MARGIN_BOTTOM: i32 = 8;
    pub const PANEL_RADIUS: f32 = 20.0;
    /// Inset of the card column inside the panel.
    pub const PANEL_PADDING: f32 = 12.0;
    pub const HEADER_HEIGHT: f32 = 56.0;

    pub const CARD_RADIUS: f32 = 16.0;
    pub const CARD_PADDING: f32 = 12.0;
    pub const CARD_GAP: f32 = 10.0;
    pub const ICON_SIZE: f32 = 36.0;
    pub const ICON_RADIUS: f32 = 9.0;
    pub const ICON_GAP: f32 = 10.0;
    pub const LINE_GAP: f32 = 2.0;

    pub const PILL_HEIGHT: f32 = 28.0;
    pub const PILL_PAD_X: f32 = 12.0;
    pub const ACTION_GAP: f32 = 8.0;
    pub const ACTIONS_PER_ROW: usize = 3;

    pub const CLOSE_RADIUS: f32 = 10.0;
    /// Close-button centre offset from a card's top-left corner.
    pub const CLOSE_OFFSET: f32 = 5.0;

    /// Visible strip of each stacked "sheet" below a collapsed group.
    pub const SHEET_PEEK: f32 = 8.0;
    /// Horizontal inset per sheet level.
    pub const SHEET_INSET: f32 = 10.0;
    pub const MAX_SHEETS: usize = 2;

    pub const GROUP_HEADER_HEIGHT: f32 = 30.0;

    pub const TOAST_WIDTH: f32 = 360.0;
    /// Transparent margin around the toast stack inside the toast surface,
    /// leaving room for the hover close button that overhangs a card corner.
    pub const TOAST_INSET: f32 = 8.0;
    pub const TOAST_MARGIN_TOP: i32 = 52;
    pub const TOAST_MARGIN_RIGHT: f32 = 12.0;
    pub const TOAST_GAP: f32 = 10.0;

    /// Body line clamp in collapsed cards and toasts.
    pub const BODY_LINES_COLLAPSED: u32 = 2;
    /// Body line clamp in expanded groups.
    pub const BODY_LINES_EXPANDED: u32 = 8;
    pub const SUMMARY_LINES: u32 = 2;
}

/// Named text styles. Sizes and weights live here so layout and the text
/// engine agree, and so styles can be hashed for the shaping cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextStyle {
    /// "Notifications".
    Title,
    /// App name in a card's top row.
    Meta,
    /// Expanded-group header app name.
    GroupTitle,
    Summary,
    Body,
    /// Pill buttons.
    Button,
    /// Fallback icon letter.
    IconLetter,
    /// "No Notifications".
    Empty,
}

/// Font weight used by a [`TextStyle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
    Bold,
}

impl TextStyle {
    /// Font size in logical pixels.
    #[must_use]
    pub fn size(self) -> f32 {
        match self {
            Self::Title => 16.0,
            Self::Meta | Self::Button => 12.0,
            Self::GroupTitle | Self::Summary => 14.0,
            Self::Body => 13.0,
            Self::IconLetter => 18.0,
            Self::Empty => 15.0,
        }
    }

    /// Line height in logical pixels.
    #[must_use]
    pub fn line_height(self) -> f32 {
        (self.size() * 1.3).round()
    }

    #[must_use]
    pub fn weight(self) -> Weight {
        match self {
            Self::Title | Self::Summary | Self::GroupTitle => Weight::Semibold,
            Self::Button | Self::Empty => Weight::Medium,
            Self::IconLetter => Weight::Bold,
            Self::Meta | Self::Body => Weight::Regular,
        }
    }

    /// Single-line styles fold newlines into spaces before shaping.
    #[must_use]
    pub fn single_line(self) -> bool {
        matches!(
            self,
            Self::Title | Self::Meta | Self::GroupTitle | Self::Button | Self::IconLetter
        )
    }
}

/// A colour slot resolved against the theme at paint time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Surface,
    OnSurface,
    OnSurfaceVariant,
    Primary,
    OnPrimary,
    Error,
    Outline,
    /// Lightening tint for cards: `on_surface` in dark mode, white in light.
    Tint,
    /// Glass background: `surface` at the theme's `surface_alpha` (times the
    /// fill alpha).
    Glass,
}

/// A role plus opacity: what layout asks the painter to use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    pub role: Role,
    pub alpha: f32,
}

impl Fill {
    #[must_use]
    pub const fn new(role: Role, alpha: f32) -> Self {
        Self { role, alpha }
    }

    #[must_use]
    pub const fn solid(role: Role) -> Self {
        Self { role, alpha: 1.0 }
    }
}

/// Fills used by layout, collected so the look is tuned in one place.
pub mod fills {
    use super::{Fill, Role};

    pub const CARD: Fill = Fill::new(Role::Tint, 0.075);
    pub const CARD_HOVER: Fill = Fill::new(Role::Tint, 0.11);
    pub const SHEET_1: Fill = Fill::new(Role::Tint, 0.07);
    pub const SHEET_2: Fill = Fill::new(Role::Tint, 0.05);
    pub const PILL: Fill = Fill::new(Role::Tint, 0.10);
    pub const PILL_HOVER: Fill = Fill::new(Role::Tint, 0.17);
    pub const PILL_ON: Fill = Fill::solid(Role::Primary);
    pub const OUTLINE: Fill = Fill::new(Role::Outline, 0.35);
    pub const CARD_OUTLINE: Fill = Fill::new(Role::Tint, 0.06);
    pub const CRITICAL_OUTLINE: Fill = Fill::new(Role::Error, 0.7);
    pub const CLOSE_BG: Fill = Fill::new(Role::Surface, 0.96);
    pub const CLOSE_BG_HOVER: Fill = Fill::solid(Role::Surface);
    pub const CLOSE_RING: Fill = Fill::new(Role::Outline, 0.6);
    pub const ICON_FALLBACK: Fill = Fill::new(Role::Primary, 0.9);
    pub const GLASS: Fill = Fill::solid(Role::Glass);
}

/// Premultiplied-ready RGBA colour (straight alpha, 0..=1 channels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// Resolves [`Role`]s against a [`Theme`].
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    theme: Theme,
}

impl Palette {
    #[must_use]
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    #[must_use]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Glass background alpha for the panel and toasts, clamped to 0..=1.
    #[must_use]
    pub fn surface_alpha(&self) -> f32 {
        self.theme.surface_alpha.clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn rgb(&self, role: Role) -> Rgb {
        let t = &self.theme;
        match role {
            Role::Surface | Role::Glass => t.surface,
            Role::OnSurface => t.on_surface,
            Role::OnSurfaceVariant => t.on_surface_variant,
            Role::Primary => t.primary,
            Role::OnPrimary => t.on_primary,
            Role::Error => t.error,
            Role::Outline => t.outline,
            Role::Tint => match t.mode {
                Mode::Dark => t.on_surface,
                Mode::Light => Rgb(255, 255, 255),
            },
        }
    }

    /// Resolves a fill to a straight-alpha colour.
    #[must_use]
    pub fn resolve(&self, fill: Fill) -> Rgba {
        let Rgb(r, g, b) = self.rgb(fill.role);
        // In light mode a white tint needs far more opacity to read as a
        // lighter card on a light glass sheet.
        let boost = if fill.role == Role::Tint && self.theme.mode == Mode::Light {
            5.0
        } else {
            1.0
        };
        let alpha = if fill.role == Role::Glass {
            fill.alpha * self.surface_alpha()
        } else {
            fill.alpha * boost
        };
        Rgba {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: alpha.clamp(0.0, 1.0),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new(default_theme())
    }
}

/// Built-in dark theme used until the daemon sends one.
#[must_use]
pub fn default_theme() -> Theme {
    Theme {
        mode: Mode::Dark,
        surface: Rgb(0x1c, 0x1b, 0x22),
        surface_variant: Rgb(0x2b, 0x29, 0x33),
        on_surface: Rgb(0xe8, 0xe6, 0xf0),
        on_surface_variant: Rgb(0xa9, 0xa6, 0xb6),
        primary: Rgb(0x9f, 0xb4, 0xff),
        on_primary: Rgb(0x10, 0x1c, 0x4a),
        error: Rgb(0xff, 0x8a, 0x80),
        on_error: Rgb(0x4a, 0x0b, 0x06),
        outline: Rgb(0x6e, 0x6b, 0x7a),
        surface_alpha: 0.62,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_resolve_from_theme() {
        let p = Palette::default();
        let c = p.resolve(Fill::new(Role::Primary, 0.5));
        assert!((c.r - f32::from(0x9f_u8) / 255.0).abs() < 1e-6);
        assert!((c.a - 0.5).abs() < 1e-6);
    }

    #[test]
    fn tint_follows_mode() {
        let mut theme = default_theme();
        assert_eq!(
            Palette::new(theme.clone()).rgb(Role::Tint),
            theme.on_surface
        );
        theme.mode = Mode::Light;
        let p = Palette::new(theme);
        assert_eq!(p.rgb(Role::Tint), Rgb(255, 255, 255));
        assert!(p.resolve(fills::CARD).a > fills::CARD.alpha);
    }

    #[test]
    fn surface_alpha_is_clamped() {
        let mut theme = default_theme();
        theme.surface_alpha = 3.0;
        assert!((Palette::new(theme).surface_alpha() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn single_line_styles() {
        assert!(TextStyle::Meta.single_line());
        assert!(!TextStyle::Body.single_line());
        assert!(TextStyle::Body.line_height() > TextStyle::Body.size());
    }
}
