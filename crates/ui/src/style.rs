//! Visual constants, the theme's [`Style`], and the colour mapping.
//!
//! Layout refers to colours by [`Role`] (plus an alpha), never by value, so a
//! colour change only needs a repaint, not a re-layout. [`Palette`] resolves
//! roles against the current [`Theme`]. Shapes, borders, opacities and type
//! come from the user's theme ([`Look`]) as a [`Style`], fixed for the run.

use tidings_core::look::{Border, Look, Paint, SHADOW_ROOM};
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
    /// Inset of the card column inside the panel.
    pub const PANEL_PADDING: f32 = 12.0;
    pub const HEADER_HEIGHT: f32 = 56.0;

    pub const CARD_PADDING: f32 = 12.0;
    pub const CARD_GAP: f32 = 10.0;
    pub const ICON_SIZE: f32 = 36.0;
    /// Round icon badge.
    pub const ICON_RADIUS: f32 = ICON_SIZE / 2.0;
    /// Share of an avatar's diameter an icon with its own outline is drawn
    /// at (see `render::image::Shape::Icon`). At most 1/sqrt(2), so even a
    /// square icon sits fully inside the circle with room around it.
    pub const ICON_CONTENT: f32 = 0.70;
    /// App badge on an avatar (see [`crate::layout::card::avatar_images`]):
    /// its size as a fraction of the avatar, how much of it hangs past the
    /// avatar's bottom-right edge, and the width of the ring that separates
    /// it from the avatar.
    pub const BADGE_SCALE: f32 = 0.44;
    pub const BADGE_OVERHANG: f32 = 0.8;
    pub const BADGE_RING: f32 = 1.5;
    pub const ICON_GAP: f32 = 10.0;
    pub const LINE_GAP: f32 = 2.0;

    pub const PILL_HEIGHT: f32 = 28.0;
    /// The inline reply field (a little taller than a button, which sits
    /// inside it).
    pub const REPLY_FIELD_HEIGHT: f32 = 34.0;
    pub const PILL_PAD_X: f32 = 12.0;
    pub const ACTION_GAP: f32 = 8.0;
    pub const ACTIONS_PER_ROW: usize = 3;

    pub const CLOSE_RADIUS: f32 = 10.0;
    /// Span of the "×" cross as a fraction of its button's radius.
    pub const CLOSE_GLYPH: f32 = 0.5;
    /// Close-button centre offset from a card's top-left corner.
    /// Far enough in that the button's centre stays inside a 28 px corner.
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
    /// Half the height: the most a collapsed toast can be rounded.
    pub const TOAST_RADIUS: f32 = TOAST_HEIGHT / 2.0;
    /// Round icon of a collapsed toast, vertically centred.
    pub const TOAST_ICON_SIZE: f32 = 40.0;
    /// Icon inset from the pill's left edge (and, centred, from top/bottom).
    pub const TOAST_ICON_INSET: f32 = (TOAST_HEIGHT - TOAST_ICON_SIZE) / 2.0;
    /// Gap between the icon, the text column, the time and the expand button.
    pub const TOAST_GAP_X: f32 = 10.0;
    pub const TOAST_PAD_RIGHT: f32 = 14.0;
    /// Gap between the summary row and the body line.
    pub const TOAST_LINE_GAP: f32 = 1.0;
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
    /// Padding under an expanded toast's last row (the other sides use
    /// [`CARD_PADDING`]).
    pub const TOAST_EXPANDED_PAD_BOTTOM: f32 = 14.0;
    pub const TOAST_MARGIN_TOP: i32 = 52;
    pub const TOAST_MARGIN_RIGHT: f32 = 12.0;
    pub const TOAST_GAP: f32 = 10.0;

    /// Body line clamp in collapsed panel cards.
    pub const BODY_LINES_COLLAPSED: u32 = 2;
    /// Body line clamp in expanded groups.
    pub const BODY_LINES_EXPANDED: u32 = 8;
    pub const SUMMARY_LINES: u32 = 2;
}

/// Named text styles. Base sizes and weights live here so layout and the
/// text engine agree, and so styles can be hashed for the shaping cache; the
/// theme's [`Type`] scales them.
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
    /// Font size in logical pixels before the theme's scale.
    #[must_use]
    pub fn base_size(self) -> f32 {
        match self {
            Self::Title | Self::IconLetter => 16.0,
            Self::Meta | Self::Button => 12.0,
            Self::GroupTitle | Self::Summary | Self::SummaryLine => 14.0,
            Self::Body | Self::BodyLine => 13.0,
            Self::Time => 11.5,
            Self::Empty => 15.0,
        }
    }

    #[must_use]
    pub fn weight(self) -> Weight {
        match self {
            Self::Summary | Self::SummaryLine | Self::IconLetter => Weight::Bold,
            Self::Title | Self::GroupTitle => Weight::Semibold,
            Self::Button | Self::Empty => Weight::Medium,
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

/// Text sizes and line spacing for the theme's typography.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Type {
    /// Multiplies every [`TextStyle::base_size`].
    pub scale: f32,
    /// Line spacing as a multiple of the size.
    pub line_height: f32,
}

impl Default for Type {
    fn default() -> Self {
        let t = tidings_core::look::Typography::default();
        Self {
            scale: t.scale,
            line_height: t.line_height,
        }
    }
}

impl Type {
    /// Font size in logical pixels.
    #[must_use]
    pub fn size(self, style: TextStyle) -> f32 {
        style.base_size() * self.scale
    }

    /// Line height in logical pixels (whole pixels, so rows stay crisp).
    #[must_use]
    pub fn line_height(self, style: TextStyle) -> f32 {
        (self.size(style) * self.line_height).round()
    }
}

/// A colour slot resolved against the theme at paint time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Surface,
    SurfaceVariant,
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
    /// A colour that does not follow the theme.
    Fixed(Rgb),
}

impl From<Paint> for Role {
    fn from(p: Paint) -> Self {
        match p {
            Paint::Surface => Self::Surface,
            Paint::SurfaceVariant => Self::SurfaceVariant,
            Paint::OnSurface => Self::OnSurface,
            Paint::OnSurfaceVariant => Self::OnSurfaceVariant,
            Paint::Primary => Self::Primary,
            Paint::OnPrimary => Self::OnPrimary,
            Paint::Error => Self::Error,
            Paint::Outline => Self::Outline,
            Paint::Tint => Self::Tint,
            Paint::Fixed(rgb) => Self::Fixed(rgb),
        }
    }
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

/// An outline drawn just inside a shape's edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub fill: Fill,
    /// Logical pixels.
    pub width: f32,
}

impl Stroke {
    /// A one-pixel outline.
    #[must_use]
    pub const fn hairline(fill: Fill) -> Self {
        Self { fill, width: 1.0 }
    }

    /// A theme border, or `None` when it would not show.
    #[must_use]
    pub fn from_border(b: Border) -> Option<Self> {
        (b.width > 0.0 && b.opacity > 0.0).then(|| Self {
            fill: Fill::new(b.paint.into(), b.opacity),
            width: b.width,
        })
    }
}

/// Fills that do not change with the theme.
pub mod fills {
    use super::{Fill, Role, Stroke};

    /// Ring behind an avatar badge: the surface colour, so the badge reads as
    /// sitting on top of the avatar on any glass.
    pub const BADGE_RING: Fill = Fill::new(Role::Surface, 0.95);
    /// Round plate behind an icon drawn inset in its avatar.
    pub const ICON_PLATE: Fill = Fill::new(Role::Tint, 0.14);
    /// A toggled-on pill (Do Not Disturb).
    pub const PILL_ON: Fill = Fill::solid(Role::Primary);
    pub const CLOSE_BG: Fill = Fill::new(Role::Surface, 0.96);
    pub const CLOSE_BG_HOVER: Fill = Fill::solid(Role::Surface);
    pub const CLOSE_RING: Stroke = Stroke::hairline(Fill::new(Role::Outline, 0.4));
    /// The reply field: a well a little darker than the card around it.
    pub const FIELD: Fill = Fill::new(Role::Surface, 0.5);
    pub const FIELD_RING: Stroke = Stroke::hairline(Fill::new(Role::Outline, 0.35));
    /// The reply field while it has the keyboard (or the pointer).
    pub const FIELD_FOCUS: Stroke = Stroke {
        fill: Fill::new(Role::Primary, 0.9),
        width: 1.5,
    };
    /// The panel sheet: the surface at the theme's glass opacity.
    pub const GLASS: Fill = Fill::solid(Role::Glass);
    /// "There are actions inside" dot on a toast's expand button.
    pub const ACTIONS_DOT: Fill = Fill::solid(Role::Primary);
}

/// A toast's soft drop shadow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowStyle {
    /// Colour and opacity of the unblurred shape.
    pub fill: Fill,
    /// How far it fades out beyond the toast (three standard deviations).
    pub spread: f32,
    /// Downward shift.
    pub offset_y: f32,
}

/// Everything layout needs from the theme, resolved from a [`Look`]: radii,
/// fills, borders, the shadow and type. Colours stay [`Role`]s, so a colour
/// change still only needs a repaint.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub text: Type,
    /// Font family the user asked for (`None`: the system UI font).
    pub family: Option<String>,

    pub panel_radius: f32,
    pub glass: Fill,
    pub panel_border: Option<Stroke>,
    /// Ask the compositor to blur behind the panel.
    pub panel_blur: bool,

    card_radius: f32,
    pub card: Fill,
    pub card_hover: Fill,
    /// The sheets peeking below a collapsed stack, nearest first.
    pub sheets: [Fill; 2],
    pub card_border: Option<Stroke>,
    /// Outline of a critical notification's card or toast.
    pub critical: Stroke,

    toast_radius: f32,
    pub toast: Fill,
    pub toast_hover: Fill,
    pub toast_expanded: Fill,
    /// Lightening tint over a hovered or expanded toast.
    pub toast_sheen: Option<Fill>,
    pub toast_border: Option<Stroke>,
    pub shadow: Option<ShadowStyle>,
    /// Ask the compositor to blur behind toasts.
    pub toast_blur: bool,

    button_radius: f32,
    pub button: Fill,
    pub button_hover: Fill,
}

impl Style {
    #[must_use]
    pub fn new(look: &Look) -> Self {
        let card_role = Role::from(look.card.paint);
        let card = Fill::new(card_role, look.card.opacity);
        let toast_role = Role::from(look.toast.paint);
        let shadow = look.toast.shadow;
        let critical_width = look.card.border.width.max(look.toast.border.width).max(1.0);
        Self {
            text: Type {
                scale: look.font.scale,
                line_height: look.font.line_height,
            },
            family: look.font.family.clone(),

            panel_radius: look.panel.radius,
            glass: fills::GLASS,
            panel_border: Stroke::from_border(look.panel.border),
            panel_blur: look.panel.blur,

            card_radius: look.card.radius,
            card,
            card_hover: Fill::new(card_role, look.card.hover_opacity),
            // Each sheet a little fainter than the card in front of it.
            sheets: [
                Fill::new(card_role, card.alpha * 0.93),
                Fill::new(card_role, card.alpha * 0.67),
            ],
            card_border: Stroke::from_border(look.card.border),
            critical: Stroke {
                fill: Fill::new(Role::Error, 0.7),
                width: critical_width,
            },

            toast_radius: look.toast.radius,
            toast: Fill::new(toast_role, look.toast.opacity),
            toast_hover: Fill::new(toast_role, look.toast.hover_opacity),
            toast_expanded: Fill::new(toast_role, look.toast.expanded_opacity),
            toast_sheen: (look.toast.sheen > 0.0).then(|| Fill::new(Role::Tint, look.toast.sheen)),
            toast_border: Stroke::from_border(look.toast.border),
            shadow: (shadow.opacity > 0.0 && shadow.size + shadow.offset > 0.0).then(|| {
                // Never past the room left around each toast.
                let spread = shadow.size.min(SHADOW_ROOM);
                ShadowStyle {
                    fill: Fill::new(shadow.paint.into(), shadow.opacity),
                    spread,
                    offset_y: shadow.offset.min(SHADOW_ROOM - spread),
                }
            }),
            toast_blur: look.toast.blur,

            button_radius: look.button.radius,
            button: Fill::new(look.button.paint.into(), look.button.opacity),
            button_hover: Fill::new(look.button.paint.into(), look.button.hover_opacity),
        }
    }

    /// Corner radius for a card of `height`: a pill while it is short,
    /// capped at the theme's card radius.
    #[must_use]
    pub fn card_radius(&self, height: f32) -> f32 {
        (height / 2.0).min(self.card_radius)
    }

    /// Corner radius for a toast of `height`. An expanded toast is shaped
    /// like a card, and is never rounder than one.
    #[must_use]
    pub fn toast_radius(&self, height: f32, expanded: bool) -> f32 {
        let r = (height / 2.0).min(self.toast_radius);
        if expanded { r.min(self.card_radius) } else { r }
    }

    /// Corner radius for a button of `height`.
    #[must_use]
    pub fn button_radius(&self, height: f32) -> f32 {
        (height / 2.0).min(self.button_radius)
    }

    /// A button's fill.
    #[must_use]
    pub fn button_fill(&self, hovered: bool) -> Fill {
        if hovered {
            self.button_hover
        } else {
            self.button
        }
    }
}

impl Default for Style {
    /// The default ("glass") look.
    fn default() -> Self {
        Self::new(&Look::default())
    }
}

// The shadow fits in the toast inset, which lives inside the screen-edge gap.
const _: () = assert!(
    SHADOW_ROOM <= metrics::TOAST_INSET && metrics::TOAST_INSET <= metrics::TOAST_MARGIN_RIGHT
);

/// Pastel `(background, letter)` pairs for letter-tile avatars, one picked
/// per app (see [`crate::layout::card::monogram`]). Fixed rather than themed:
/// the dark letter on a light pastel reads on any glass.
pub const MONOGRAM_TINTS: [(Rgb, Rgb); 6] = [
    // Blue.
    (Rgb(0xa8, 0xc7, 0xfa), Rgb(0x13, 0x23, 0x3f)),
    // Yellow.
    (Rgb(0xfd, 0xe6, 0x8a), Rgb(0x42, 0x20, 0x06)),
    // Pink.
    (Rgb(0xfe, 0xca, 0xca), Rgb(0x45, 0x0a, 0x0a)),
    // Orange.
    (Rgb(0xfd, 0xba, 0x74), Rgb(0x43, 0x14, 0x07)),
    // Lime.
    (Rgb(0xd9, 0xf9, 0x9d), Rgb(0x1a, 0x2e, 0x05)),
    // Lavender.
    (Rgb(0xdd, 0xd6, 0xfe), Rgb(0x2e, 0x10, 0x65)),
];

/// The `(background, letter)` colours of monogram tint `index` (wrapping).
#[must_use]
pub fn monogram_colors(index: u8) -> (Rgba, Rgba) {
    let (bg, fg) = MONOGRAM_TINTS[usize::from(index) % MONOGRAM_TINTS.len()];
    (Rgba::opaque(bg), Rgba::opaque(fg))
}

/// Premultiplied-ready RGBA colour (straight alpha, 0..=1 channels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// A fully opaque colour.
    #[must_use]
    pub fn opaque(Rgb(r, g, b): Rgb) -> Self {
        Self {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
    }

    /// The same colour with its alpha multiplied by `k`.
    #[must_use]
    pub fn faded(self, k: f32) -> Self {
        Self {
            a: self.a * k,
            ..self
        }
    }
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

    /// Glass background alpha for the panel, clamped to 0..=1.
    #[must_use]
    pub fn surface_alpha(&self) -> f32 {
        self.theme.surface_alpha.clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn rgb(&self, role: Role) -> Rgb {
        let paint = match role {
            Role::Surface | Role::Glass => Paint::Surface,
            Role::SurfaceVariant => Paint::SurfaceVariant,
            Role::OnSurface => Paint::OnSurface,
            Role::OnSurfaceVariant => Paint::OnSurfaceVariant,
            Role::Primary => Paint::Primary,
            Role::OnPrimary => Paint::OnPrimary,
            Role::Error => Paint::Error,
            Role::Outline => Paint::Outline,
            Role::Tint => Paint::Tint,
            Role::Fixed(rgb) => Paint::Fixed(rgb),
        };
        paint.resolve(&self.theme)
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
    use tidings_core::config::theme::{BUILTIN, ThemeFile};

    use super::metrics as m;
    use super::*;

    #[test]
    fn roles_resolve_from_theme() {
        let p = Palette::default();
        let c = p.resolve(Fill::new(Role::Primary, 0.5));
        assert!((c.r - f32::from(0x9f_u8) / 255.0).abs() < 1e-6);
        assert!((c.a - 0.5).abs() < 1e-6);
        assert_eq!(p.rgb(Role::Fixed(Rgb(1, 2, 3))), Rgb(1, 2, 3));
    }

    #[test]
    fn every_theme_paint_is_the_role_of_the_same_colour() {
        let p = Palette::default();
        let fixed = Paint::Fixed(Rgb(7, 8, 9));
        for paint in [
            Paint::Surface,
            Paint::SurfaceVariant,
            Paint::OnSurface,
            Paint::OnSurfaceVariant,
            Paint::Primary,
            Paint::OnPrimary,
            Paint::Error,
            Paint::Outline,
            Paint::Tint,
            fixed,
        ] {
            assert_eq!(
                p.rgb(Role::from(paint)),
                paint.resolve(p.theme()),
                "{paint:?}"
            );
        }
        // Glass is the surface colour (its alpha is the theme's).
        assert_eq!(p.rgb(Role::Glass), p.theme().surface);
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
        let card = Style::default().card;
        assert!(p.resolve(card).a > card.alpha);
    }

    #[test]
    fn surface_alpha_is_clamped() {
        let mut theme = default_theme();
        theme.surface_alpha = 3.0;
        assert!((Palette::new(theme).surface_alpha() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn default_style_is_the_glass_look() {
        let s = Style::default();
        assert!((s.panel_radius - 20.0).abs() < f32::EPSILON);
        assert_eq!(s.card, Fill::new(Role::Tint, 0.075));
        assert_eq!(s.toast, Fill::new(Role::Surface, 0.46));
        assert_eq!(
            s.panel_border,
            Some(Stroke::hairline(Fill::new(Role::Outline, 0.35)))
        );
        // A collapsed toast is a full pill; tall cards cap at 28.
        assert!((s.toast_radius(m::TOAST_HEIGHT, false) - m::TOAST_RADIUS).abs() < f32::EPSILON);
        assert!((s.toast_radius(200.0, true) - 28.0).abs() < f32::EPSILON);
        assert!((s.card_radius(40.0) - 20.0).abs() < f32::EPSILON);
        assert!((s.button_radius(m::PILL_HEIGHT) - m::PILL_HEIGHT / 2.0).abs() < f32::EPSILON);
        let shadow = s.shadow.unwrap_or_else(|| panic!("glass has a shadow"));
        assert_eq!(shadow.fill, Fill::new(Role::Fixed(Rgb(0, 0, 0)), 0.28));
        assert!(s.panel_blur && s.toast_blur);
        assert!((s.text.line_height(TextStyle::Body) - 17.0).abs() < f32::EPSILON);
    }

    #[test]
    fn themes_change_shape_borders_and_type() {
        let terminal = BUILTIN
            .iter()
            .find(|(n, _)| *n == "terminal")
            .and_then(|(_, src)| ThemeFile::from_toml(src).ok())
            .unwrap_or_else(|| panic!("terminal theme"));
        let s = Style::new(&terminal.look());
        assert!(s.card_radius(100.0).abs() < f32::EPSILON);
        assert!(s.button_radius(m::PILL_HEIGHT).abs() < f32::EPSILON);
        assert_eq!(s.shadow, None);
        assert!(!s.panel_blur && !s.toast_blur);
        assert!((s.toast_border.map_or(0.0, |b| b.width) - 2.0).abs() < f32::EPSILON);
        assert_eq!(s.family.as_deref(), Some("monospace"));
        assert!(s.text.size(TextStyle::Body) < TextStyle::Body.base_size());
        // A zero-width or invisible border is no border.
        let mut look = Look::default();
        look.card.border.width = 0.0;
        look.toast.border.opacity = 0.0;
        let s = Style::new(&look);
        assert_eq!((s.card_border, s.toast_border), (None, None));
    }

    #[test]
    fn summaries_are_bold() {
        assert_eq!(TextStyle::Summary.weight(), Weight::Bold);
        assert_eq!(TextStyle::SummaryLine.weight(), Weight::Bold);
        assert_eq!(TextStyle::IconLetter.weight(), Weight::Bold);
        assert_eq!(TextStyle::Title.weight(), Weight::Semibold);
        assert!((TextStyle::Summary.base_size() - 14.0).abs() < f32::EPSILON);
        assert!((TextStyle::SummaryLine.base_size() - 14.0).abs() < f32::EPSILON);
    }

    #[test]
    fn monogram_colors_wrap_and_are_opaque() {
        let n = u8::try_from(MONOGRAM_TINTS.len()).unwrap_or(u8::MAX);
        assert_eq!(monogram_colors(0), monogram_colors(n));
        for i in 0..n {
            let (bg, fg) = monogram_colors(i);
            assert!((bg.a - 1.0).abs() < f32::EPSILON && (fg.a - 1.0).abs() < f32::EPSILON);
            // Dark letter on a light pastel.
            assert!(bg.r + bg.g + bg.b > 2.0 && fg.r + fg.g + fg.b < 0.8, "{i}");
        }
    }

    #[test]
    fn single_line_styles() {
        let t = Type::default();
        assert!(TextStyle::Meta.single_line());
        assert!(!TextStyle::Body.single_line());
        assert!(t.line_height(TextStyle::Body) > t.size(TextStyle::Body));
    }
}
