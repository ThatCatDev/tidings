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

    /// Cards are pills: corner radius is half the card height, capped here so
    /// a tall card reads as a card with generous corners, not a lozenge. See
    /// [`crate::layout::card::card_radius`].
    pub const CARD_RADIUS_MAX: f32 = 28.0;
    pub const CARD_PADDING: f32 = 12.0;
    pub const CARD_GAP: f32 = 10.0;
    pub const ICON_SIZE: f32 = 36.0;
    /// Round icon badge.
    pub const ICON_RADIUS: f32 = ICON_SIZE / 2.0;
    pub const ICON_GAP: f32 = 10.0;
    pub const LINE_GAP: f32 = 2.0;

    pub const PILL_HEIGHT: f32 = 28.0;
    pub const PILL_PAD_X: f32 = 12.0;
    pub const ACTION_GAP: f32 = 8.0;
    pub const ACTIONS_PER_ROW: usize = 3;

    pub const CLOSE_RADIUS: f32 = 10.0;
    /// Close-button centre offset from a card's top-left corner.
    /// Far enough in that the button's centre stays inside a
    /// [`CARD_RADIUS_MAX`] corner.
    pub const CLOSE_OFFSET: f32 = 10.0;

    /// Visible strip of each stacked "sheet" below a collapsed group.
    pub const SHEET_PEEK: f32 = 8.0;
    /// Horizontal inset per sheet level.
    pub const SHEET_INSET: f32 = 10.0;
    pub const MAX_SHEETS: usize = 2;

    pub const GROUP_HEADER_HEIGHT: f32 = 30.0;

    pub const TOAST_WIDTH: f32 = 360.0;
    /// Transparent margin around each toast inside the toast surface, leaving
    /// room for the drop shadow and the hover "×" that overhangs the icon.
    /// Must not exceed [`TOAST_MARGIN_RIGHT`]: the right inset lives inside
    /// the gap to the screen edge.
    pub const TOAST_INSET: f32 = 12.0;
    /// A collapsed toast is a fixed-height pill, whatever its text.
    pub const TOAST_HEIGHT: f32 = 58.0;
    /// Half the height: a true pill.
    pub const TOAST_RADIUS: f32 = TOAST_HEIGHT / 2.0;
    /// Round icon of a collapsed toast, vertically centred.
    pub const TOAST_ICON_SIZE: f32 = 40.0;
    /// Icon inset from the pill's left edge (and, centred, from top/bottom).
    pub const TOAST_ICON_INSET: f32 = (TOAST_HEIGHT - TOAST_ICON_SIZE) / 2.0;
    /// Gap between the icon, the text column and the expand button.
    pub const TOAST_GAP_X: f32 = 10.0;
    pub const TOAST_PAD_RIGHT: f32 = 14.0;
    /// Gap between the summary row and the body line.
    pub const TOAST_LINE_GAP: f32 = 1.0;
    /// Gap between the summary and the time label on the summary row.
    pub const TOAST_TIME_GAP: f32 = 8.0;
    /// Hover "×" on a toast: radius, and its centre's offset from the toast's
    /// top-left corner (it overlaps the icon's top-left, overhanging by 4).
    pub const TOAST_CLOSE_RADIUS: f32 = 11.0;
    pub const TOAST_CLOSE_OFFSET: f32 = TOAST_CLOSE_RADIUS - 4.0;
    /// Round expand/collapse (chevron) button.
    pub const CHEVRON_BUTTON: f32 = 28.0;
    /// Chevron stroke icon size inside the button.
    pub const CHEVRON_ICON: f32 = 14.0;
    /// Collapse button inset from an expanded toast's top-right corner.
    pub const CHEVRON_CORNER_INSET: f32 = 10.0;
    /// "There are actions inside" dot on the expand button.
    pub const ACTIONS_DOT_RADIUS: f32 = 3.5;
    /// Body line clamp in an expanded toast.
    pub const BODY_LINES_TOAST_EXPANDED: u32 = 6;
    pub const TOAST_MARGIN_TOP: i32 = 52;
    pub const TOAST_MARGIN_RIGHT: f32 = 12.0;
    pub const TOAST_GAP: f32 = 10.0;

    /// Body line clamp in collapsed panel cards.
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
    /// Collapsed toast summary: one line, newlines folded.
    SummaryLine,
    /// Collapsed toast body: one line, newlines folded.
    BodyLine,
    /// Collapsed toast time label.
    Time,
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
            Self::GroupTitle | Self::Summary | Self::SummaryLine => 14.0,
            Self::Body | Self::BodyLine => 13.0,
            Self::Time => 11.5,
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
            Self::Title | Self::Summary | Self::SummaryLine | Self::GroupTitle => Weight::Semibold,
            Self::Button | Self::Empty => Weight::Medium,
            Self::IconLetter => Weight::Bold,
            Self::Meta | Self::Body | Self::BodyLine | Self::Time => Weight::Regular,
        }
    }

    /// Single-line styles fold newlines into spaces before shaping.
    #[must_use]
    pub fn single_line(self) -> bool {
        matches!(
            self,
            Self::Title
                | Self::Meta
                | Self::GroupTitle
                | Self::Button
                | Self::IconLetter
                | Self::SummaryLine
                | Self::BodyLine
                | Self::Time
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
    /// Pure black, for drop shadows.
    Shadow,
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

    use super::toast_glass as g;
    pub const TOAST: Fill = Fill::new(Role::Surface, g::TOAST_ALPHA);
    pub const TOAST_HOVER: Fill = Fill::new(Role::Surface, g::TOAST_HOVER_ALPHA);
    pub const TOAST_EXPANDED: Fill = Fill::new(Role::Surface, g::TOAST_EXPANDED_ALPHA);
    /// Lightening sheen over a hovered or expanded toast's glass.
    pub const TOAST_SHEEN: Fill = Fill::new(Role::Tint, g::TOAST_SHEEN_ALPHA);
    pub const TOAST_RIM: Fill = Fill::new(Role::Tint, g::TOAST_RIM_ALPHA);
    pub const TOAST_SHADOW: Fill = Fill::new(Role::Shadow, g::SHADOW_ALPHA);
    /// "There are actions inside" dot on a toast's expand button.
    pub const ACTIONS_DOT: Fill = Fill::solid(Role::Primary);
}

/// Toast glass: much more transparent than the panel, because Hyprland blurs
/// what is behind it.
///
/// Hyprland's layer rule `ignore_alpha = T` blurs only behind pixels whose
/// alpha is above `T`. The toast surface holds two kinds of translucent
/// pixels: the glass body (at least [`TOAST_ALPHA`]) and the soft drop
/// shadow around it (at most [`SHADOW_ALPHA`]). Setting `T` between the two
/// (`SHADOW_ALPHA < T < TOAST_ALPHA`, e.g. 0.4) blurs behind the body but not
/// behind the shadow, which would otherwise show as a blurred halo. Keep that
/// gap when tuning these values. The shadow is cut out under the body, so it
/// never adds to the body's alpha.
pub mod toast_glass {
    /// A resting collapsed toast; the lowest body alpha on the surface.
    pub const TOAST_ALPHA: f32 = 0.46;
    /// A hovered collapsed toast (≈ 0.56 once the sheen is on top).
    pub const TOAST_HOVER_ALPHA: f32 = 0.54;
    /// An expanded toast (≈ 0.52 once the sheen is on top).
    pub const TOAST_EXPANDED_ALPHA: f32 = 0.50;
    /// Tint layered over hovered/expanded glass so it reads lighter, not
    /// just more opaque.
    pub const TOAST_SHEEN_ALPHA: f32 = 0.04;
    /// The 1 px light rim around every toast.
    pub const TOAST_RIM_ALPHA: f32 = 0.16;
    /// Peak opacity of the drop shadow, right at the toast's edge. Must stay
    /// below Hyprland's `ignore_alpha` (and so below [`TOAST_ALPHA`]).
    pub const SHADOW_ALPHA: f32 = 0.28;
    /// How far the shadow fades out beyond the toast's edge (logical px).
    pub const SHADOW_SPREAD: f32 = 9.0;
    /// Downward shift of the shadow (logical px).
    pub const SHADOW_OFFSET_Y: f32 = 3.0;

    // The ignore_alpha gap, and the shadow fitting inside the toast inset.
    const _: () = assert!(SHADOW_ALPHA < 0.35 && SHADOW_ALPHA + 0.1 < TOAST_ALPHA);
    const _: () = assert!(
        SHADOW_SPREAD + SHADOW_OFFSET_Y <= super::metrics::TOAST_INSET
            && super::metrics::TOAST_INSET <= super::metrics::TOAST_MARGIN_RIGHT
    );
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
            Role::Shadow => Rgb(0, 0, 0),
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
