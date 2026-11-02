//! Theme files, and the look settings `config.toml` can override.
//!
//! A theme file is TOML with an optional `name` and `description` and the
//! sections below; `config.toml` accepts the same sections under `[theme]`
//! (`[theme.panel]`, ...). Both are parsed into a [`LookPatch`]: only the
//! keys that were written, already validated. Patches are layered onto
//! [`Look::default`] (the "glass" theme): first the chosen theme, then the
//! config's overrides.
//!
//! ```toml
//! name = "Solid"
//! [panel]
//! opacity = 1.0
//! radius = 14
//! border_width = 1
//! border_color = "outline"
//! ```

use std::ops::RangeInclusive;

use serde::Deserialize;
use toml::Spanned;

use super::{Checker, ConfigError, ConfigErrors, line_column, narrow};
use crate::look::{Border, Look, Paint, SHADOW_ROOM};
use crate::theme::{Mode, Rgb};

/// The themes that ship with tidings: `(name, file contents)`. The first is
/// the default.
pub const BUILTIN: [(&str, &str); 9] = [
    ("glass", include_str!("../../../../themes/glass.toml")),
    ("frosted", include_str!("../../../../themes/frosted.toml")),
    ("clear", include_str!("../../../../themes/clear.toml")),
    ("solid", include_str!("../../../../themes/solid.toml")),
    ("flat", include_str!("../../../../themes/flat.toml")),
    ("paper", include_str!("../../../../themes/paper.toml")),
    ("nord", include_str!("../../../../themes/nord.toml")),
    ("mocha", include_str!("../../../../themes/mocha.toml")),
    ("terminal", include_str!("../../../../themes/terminal.toml")),
];

/// The theme used when the config names none.
pub const DEFAULT_THEME: &str = BUILTIN[0].0;

/// The built-in theme called `name`, if there is one.
#[must_use]
pub fn builtin(name: &str) -> Option<&'static str> {
    BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

/// Whether `name` can name a theme: letters, digits, `-` and `_` (so it is
/// never a path).
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Accepted ranges, also quoted in error messages.
mod limits {
    use std::ops::RangeInclusive;

    pub const OPACITY: RangeInclusive<f64> = 0.0..=1.0;
    pub const RADIUS: RangeInclusive<f64> = 0.0..=40.0;
    pub const BORDER: RangeInclusive<f64> = 0.0..=4.0;
    pub const SHADOW_OPACITY: RangeInclusive<f64> = 0.0..=0.6;
    pub const SHADOW_LEN: RangeInclusive<f64> = 0.0..=12.0;
    pub const FONT_SCALE: RangeInclusive<f64> = 0.75..=1.5;
    pub const LINE_HEIGHT: RangeInclusive<f64> = 1.0..=2.0;
}

// ---- Field kinds -------------------------------------------------------------
//
// Each setting has a kind that fixes its TOML type, its checked type and how
// it is checked. The `section!` macro below uses these to declare a raw
// (serde) struct, a checked patch struct and the check between them.

/// TOML type of a field of each kind.
macro_rules! raw_ty {
    (number $($r:tt)*) => { Spanned<f64> };
    (flag) => { bool };
    ($other:ident) => { Spanned<String> };
}

/// Checked type of a field of each kind.
macro_rules! value_ty {
    (number $($r:tt)*) => {
        f32
    };
    (flag) => {
        bool
    };
    (paint) => {
        Paint
    };
    (color) => {
        Rgb
    };
    (mode) => {
        Mode
    };
    (text) => {
        String
    };
}

/// Checks one present raw value; `None` (with an error recorded) if bad.
macro_rules! check_value {
    ($c:ident, $v:ident, $key:expr, number ($range:expr)) => {
        $c.in_range(&$v, $key, &$range).map(narrow)
    };
    ($c:ident, $v:ident, $key:expr, flag) => {{
        let _ = (&$c, $key);
        Some($v)
    }};
    ($c:ident, $v:ident, $key:expr, paint) => {
        $c.paint(&$v, $key)
    };
    ($c:ident, $v:ident, $key:expr, color) => {
        $c.color(&$v, $key)
    };
    ($c:ident, $v:ident, $key:expr, mode) => {
        $c.mode(&$v, $key)
    };
    ($c:ident, $v:ident, $key:expr, text) => {
        $c.text(&$v, $key)
    };
}

/// Declares a look section: its raw struct, its patch and the check.
macro_rules! section {
    (
        $(#[$doc:meta])*
        $raw:ident => $patch:ident, $name:literal {
            $($field:ident: $kind:ident $(($range:expr))?),* $(,)?
        }
    ) => {
        #[derive(Debug, Default, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $raw {
            $($field: Option<raw_ty!($kind $(($range))?)>,)*
        }

        $(#[$doc])*
        #[derive(Debug, Clone, Default, PartialEq)]
        pub struct $patch {
            $(pub $field: Option<value_ty!($kind $(($range))?)>,)*
        }

        impl $raw {
            fn check(self, c: &mut Checker<'_>, prefix: &str) -> $patch {
                $patch {
                    $($field: self.$field.and_then(|v| {
                        let key = format!("{prefix}{}.{}", $name, stringify!($field));
                        check_value!(c, v, &key, $kind $(($range))?)
                    }),)*
                }
            }
        }
    };
}

section! {
    /// `[font]`.
    RawFont => FontPatch, "font" {
        family: text,
        scale: number(limits::FONT_SCALE),
        line_height: number(limits::LINE_HEIGHT),
    }
}

section! {
    /// `[palette]`: colours that replace those from `colors.json`.
    RawPalette => PalettePatch, "palette" {
        mode: mode,
        surface: color,
        surface_variant: color,
        on_surface: color,
        on_surface_variant: color,
        primary: color,
        on_primary: color,
        error: color,
        on_error: color,
        outline: color,
    }
}

section! {
    /// `[panel]`.
    RawPanel => PanelPatch, "panel" {
        opacity: number(limits::OPACITY),
        radius: number(limits::RADIUS),
        border_width: number(limits::BORDER),
        border_color: paint,
        border_opacity: number(limits::OPACITY),
        blur: flag,
    }
}

section! {
    /// `[card]`.
    RawCard => CardPatch, "card" {
        radius: number(limits::RADIUS),
        color: paint,
        opacity: number(limits::OPACITY),
        hover_opacity: number(limits::OPACITY),
        border_width: number(limits::BORDER),
        border_color: paint,
        border_opacity: number(limits::OPACITY),
    }
}

section! {
    /// `[toast]`.
    RawToast => ToastPatch, "toast" {
        radius: number(limits::RADIUS),
        color: paint,
        opacity: number(limits::OPACITY),
        hover_opacity: number(limits::OPACITY),
        expanded_opacity: number(limits::OPACITY),
        sheen: number(limits::OPACITY),
        border_width: number(limits::BORDER),
        border_color: paint,
        border_opacity: number(limits::OPACITY),
        blur: flag,
    }
}

section! {
    /// `[shadow]`: the toasts' drop shadow.
    RawShadow => ShadowPatch, "shadow" {
        color: paint,
        opacity: number(limits::SHADOW_OPACITY),
        size: number(limits::SHADOW_LEN),
        offset: number(limits::SHADOW_LEN),
    }
}

section! {
    /// `[button]`.
    RawButton => ButtonPatch, "button" {
        radius: number(limits::RADIUS),
        color: paint,
        opacity: number(limits::OPACITY),
        hover_opacity: number(limits::OPACITY),
    }
}

impl Checker<'_> {
    fn in_range(
        &mut self,
        v: &Spanned<f64>,
        key: &str,
        range: &RangeInclusive<f64>,
    ) -> Option<f64> {
        let n = *v.get_ref();
        if range.contains(&n) {
            Some(n)
        } else {
            let msg = format!("{n} is out of range ({} to {})", range.start(), range.end());
            self.error(&v.span(), key, msg);
            None
        }
    }

    fn paint(&mut self, v: &Spanned<String>, key: &str) -> Option<Paint> {
        let p = Paint::parse(v.get_ref());
        if p.is_none() {
            let msg = format!(
                "{:?} is not a colour; use #rrggbb or one of: {}",
                v.get_ref(),
                Paint::NAMES.join(", ")
            );
            self.error(&v.span(), key, msg);
        }
        p
    }

    fn color(&mut self, v: &Spanned<String>, key: &str) -> Option<Rgb> {
        let c = Rgb::from_hex(v.get_ref().trim()).filter(|_| v.get_ref().trim().starts_with('#'));
        if c.is_none() {
            let msg = format!("{:?} is not a colour; use #rrggbb", v.get_ref());
            self.error(&v.span(), key, msg);
        }
        c
    }

    fn mode(&mut self, v: &Spanned<String>, key: &str) -> Option<Mode> {
        match v.get_ref().trim() {
            "dark" => Some(Mode::Dark),
            "light" => Some(Mode::Light),
            other => {
                let msg = format!(r#"{other:?} is not a mode; use "dark" or "light""#);
                self.error(&v.span(), key, msg);
                None
            }
        }
    }

    fn text(&mut self, v: &Spanned<String>, key: &str) -> Option<String> {
        let t = v.get_ref().trim();
        if t.is_empty() {
            self.error(&v.span(), key, "must not be empty".to_owned());
            None
        } else {
            Some(t.to_owned())
        }
    }
}

/// The look sections as read, before checking.
#[derive(Debug, Default)]
pub(super) struct RawLook {
    font: RawFont,
    palette: RawPalette,
    panel: RawPanel,
    card: RawCard,
    toast: RawToast,
    shadow: RawShadow,
    button: RawButton,
}

/// Declares a raw file struct holding the look sections beside other
/// top-level keys, with a conversion to [`RawLook`].
macro_rules! with_look_sections {
    ($(#[$attr:meta])* struct $name:ident { $($extra:tt)* }) => {
        $(#[$attr])*
        struct $name {
            $($extra)*
            font: RawFont,
            palette: RawPalette,
            panel: RawPanel,
            card: RawCard,
            toast: RawToast,
            shadow: RawShadow,
            button: RawButton,
        }

        impl $name {
            fn split_look(self) -> (RawLook, Self) {
                let look = RawLook {
                    font: self.font,
                    palette: self.palette,
                    panel: self.panel,
                    card: self.card,
                    toast: self.toast,
                    shadow: self.shadow,
                    button: self.button,
                };
                let rest = Self {
                    font: RawFont::default(),
                    palette: RawPalette::default(),
                    panel: RawPanel::default(),
                    card: RawCard::default(),
                    toast: RawToast::default(),
                    shadow: RawShadow::default(),
                    button: RawButton::default(),
                    ..self
                };
                (look, rest)
            }
        }
    };
}

with_look_sections! {
    /// `[theme]` in `config.toml`.
    #[derive(Debug, Default, Deserialize)]
    #[serde(deny_unknown_fields, default)]
    struct RawThemeSection {
        name: Option<Spanned<String>>,
        colors: Option<Spanned<String>>,
    }
}

with_look_sections! {
    /// A theme file.
    #[derive(Debug, Default, Deserialize)]
    #[serde(deny_unknown_fields, default)]
    struct RawThemeFile {
        name: Option<Spanned<String>>,
        description: Option<Spanned<String>>,
    }
}

/// `[theme]` from `config.toml`, split into its own keys and the look.
#[derive(Debug, Default)]
pub(super) struct ThemeSection {
    pub name: Option<Spanned<String>>,
    pub colors: Option<Spanned<String>>,
    pub look: RawLook,
}

/// Deserializes the `[theme]` section (used by the config's raw struct).
pub(super) fn deserialize_section<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<ThemeSection, D::Error> {
    let (look, rest) = RawThemeSection::deserialize(d)?.split_look();
    Ok(ThemeSection {
        name: rest.name,
        colors: rest.colors,
        look,
    })
}

/// The look settings a file wrote, checked; layered with [`LookPatch::apply`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LookPatch {
    pub font: FontPatch,
    pub palette: PalettePatch,
    pub panel: PanelPatch,
    pub card: CardPatch,
    pub toast: ToastPatch,
    pub shadow: ShadowPatch,
    pub button: ButtonPatch,
}

impl RawLook {
    /// Checks every value; keys are reported as `{prefix}section.key`.
    pub(super) fn check(self, c: &mut Checker<'_>, prefix: &str) -> LookPatch {
        let shadow_spans = (
            self.shadow.size.as_ref().map(Spanned::span),
            self.shadow.offset.as_ref().map(Spanned::span),
        );
        let patch = LookPatch {
            font: self.font.check(c, prefix),
            palette: self.palette.check(c, prefix),
            panel: self.panel.check(c, prefix),
            card: self.card.check(c, prefix),
            toast: self.toast.check(c, prefix),
            shadow: self.shadow.check(c, prefix),
            button: self.button.check(c, prefix),
        };
        // The shadow must fit in the room left around each toast.
        let s = &patch.shadow;
        let d = Look::default().toast.shadow;
        if s.size.unwrap_or(d.size) + s.offset.unwrap_or(d.offset) > SHADOW_ROOM {
            let (key, span) = match shadow_spans {
                (_, Some(span)) if s.offset.is_some() => ("shadow.offset", span),
                (Some(span), _) => ("shadow.size", span),
                (_, Some(span)) => ("shadow.offset", span),
                (None, None) => unreachable!("a default shadow fits"),
            };
            let msg = format!("shadow.size + shadow.offset must be at most {SHADOW_ROOM}");
            c.error(&span, &format!("{prefix}{key}"), msg);
        }
        patch
    }
}

fn set<T: Clone>(slot: &mut T, value: Option<&T>) {
    if let Some(v) = value {
        *slot = v.clone();
    }
}

fn set_border(b: &mut Border, width: Option<f32>, paint: Option<Paint>, opacity: Option<f32>) {
    set(&mut b.width, width.as_ref());
    set(&mut b.paint, paint.as_ref());
    set(&mut b.opacity, opacity.as_ref());
}

impl LookPatch {
    /// Writes every setting this patch has onto `look`.
    pub fn apply(&self, look: &mut Look) {
        let f = &self.font;
        if let Some(family) = &f.family {
            look.font.family = Some(family.clone());
        }
        set(&mut look.font.scale, f.scale.as_ref());
        set(&mut look.font.line_height, f.line_height.as_ref());

        let p = &self.palette;
        let c = &mut look.colors;
        for (value, slot) in [
            (p.surface, &mut c.surface),
            (p.surface_variant, &mut c.surface_variant),
            (p.on_surface, &mut c.on_surface),
            (p.on_surface_variant, &mut c.on_surface_variant),
            (p.primary, &mut c.primary),
            (p.on_primary, &mut c.on_primary),
            (p.error, &mut c.error),
            (p.on_error, &mut c.on_error),
            (p.outline, &mut c.outline),
        ] {
            if value.is_some() {
                *slot = value;
            }
        }
        if p.mode.is_some() {
            c.mode = p.mode;
        }

        let p = &self.panel;
        let panel = &mut look.panel;
        if p.opacity.is_some() {
            panel.opacity = p.opacity;
        }
        set(&mut panel.radius, p.radius.as_ref());
        set(&mut panel.blur, p.blur.as_ref());
        set_border(
            &mut panel.border,
            p.border_width,
            p.border_color,
            p.border_opacity,
        );

        let p = &self.card;
        let card = &mut look.card;
        set(&mut card.radius, p.radius.as_ref());
        set(&mut card.paint, p.color.as_ref());
        set(&mut card.opacity, p.opacity.as_ref());
        set(&mut card.hover_opacity, p.hover_opacity.as_ref());
        set_border(
            &mut card.border,
            p.border_width,
            p.border_color,
            p.border_opacity,
        );

        let p = &self.toast;
        let toast = &mut look.toast;
        set(&mut toast.radius, p.radius.as_ref());
        set(&mut toast.paint, p.color.as_ref());
        set(&mut toast.opacity, p.opacity.as_ref());
        set(&mut toast.hover_opacity, p.hover_opacity.as_ref());
        set(&mut toast.expanded_opacity, p.expanded_opacity.as_ref());
        set(&mut toast.sheen, p.sheen.as_ref());
        set(&mut toast.blur, p.blur.as_ref());
        set_border(
            &mut toast.border,
            p.border_width,
            p.border_color,
            p.border_opacity,
        );

        let p = &self.shadow;
        let shadow = &mut toast.shadow;
        set(&mut shadow.paint, p.color.as_ref());
        set(&mut shadow.opacity, p.opacity.as_ref());
        set(&mut shadow.size, p.size.as_ref());
        set(&mut shadow.offset, p.offset.as_ref());
        // Layered patches can each fit and still overflow together.
        shadow.offset = shadow.offset.min(SHADOW_ROOM - shadow.size);

        let p = &self.button;
        let button = &mut look.button;
        set(&mut button.radius, p.radius.as_ref());
        set(&mut button.paint, p.color.as_ref());
        set(&mut button.opacity, p.opacity.as_ref());
        set(&mut button.hover_opacity, p.hover_opacity.as_ref());
    }
}

/// A parsed theme file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThemeFile {
    /// Display name, if the file gives one.
    pub name: Option<String>,
    pub description: Option<String>,
    pub look: LookPatch,
}

impl ThemeFile {
    /// Parses and checks a theme file's contents.
    pub fn from_toml(src: &str) -> Result<Self, ConfigErrors> {
        let raw: RawThemeFile = toml::from_str(src).map_err(|e| {
            let (line, column) = e.span().map_or((0, 0), |span| line_column(src, span.start));
            ConfigErrors(vec![ConfigError {
                line,
                column,
                key: None,
                message: e.message().trim().to_owned(),
            }])
        })?;
        let (look, rest) = raw.split_look();
        let mut c = Checker {
            src,
            errors: Vec::new(),
        };
        let file = Self {
            name: rest.name.and_then(|v| c.text(&v, "name")),
            description: rest.description.and_then(|v| c.text(&v, "description")),
            look: look.check(&mut c, ""),
        };
        c.finish(file)
    }

    /// [`Look::default`] with this theme applied.
    #[must_use]
    pub fn look(&self) -> Look {
        let mut look = Look::default();
        self.look.apply(&mut look);
        look
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::look::Paint;

    fn theme(src: &str) -> ThemeFile {
        ThemeFile::from_toml(src).unwrap_or_else(|e| panic!("{e}"))
    }

    fn errors(src: &str) -> Vec<ConfigError> {
        match ThemeFile::from_toml(src) {
            Ok(t) => panic!("expected errors, got {t:?}"),
            Err(ConfigErrors(e)) => e,
        }
    }

    #[test]
    fn glass_is_the_default_look() {
        assert_eq!(
            builtin(DEFAULT_THEME).map(|s| theme(s).look()),
            Some(Look::default())
        );
        assert_eq!(ThemeFile::from_toml(""), Ok(ThemeFile::default()));
    }

    #[test]
    fn every_builtin_theme_parses_and_differs() {
        let mut seen: Vec<Look> = Vec::new();
        for (name, src) in BUILTIN {
            assert!(is_valid_name(name));
            let t = ThemeFile::from_toml(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(t.name.is_some() && t.description.is_some(), "{name}");
            let look = t.look();
            assert!(!seen.contains(&look), "{name} repeats another theme");
            seen.push(look);
        }
    }

    #[test]
    fn reads_every_section() {
        let t = theme(
            r##"
            name = "Test"
            [font]
            family = "Inter"
            scale = 1.1
            line_height = 1.5
            [palette]
            mode = "light"
            surface = "#ffffff"
            [panel]
            opacity = 0.9
            radius = 4
            border_width = 2
            border_color = "#112233"
            border_opacity = 1
            blur = false
            [card]
            radius = 6
            color = "surface_variant"
            opacity = 1
            [toast]
            radius = 8
            sheen = 0
            blur = false
            [shadow]
            color = "primary"
            opacity = 0.5
            size = 4
            offset = 2
            [button]
            radius = 3
            hover_opacity = 0.3
            "##,
        );
        let look = t.look();
        assert_eq!(t.name.as_deref(), Some("Test"));
        assert_eq!(look.font.family.as_deref(), Some("Inter"));
        assert!((look.font.line_height - 1.5).abs() < 1e-6);
        assert_eq!(look.colors.mode, Some(Mode::Light));
        assert_eq!(look.colors.surface, Some(Rgb(255, 255, 255)));
        assert_eq!(look.colors.primary, None);
        assert_eq!(look.panel.opacity, Some(0.9));
        assert_eq!(look.panel.border.paint, Paint::Fixed(Rgb(0x11, 0x22, 0x33)));
        assert!((look.panel.border.width - 2.0).abs() < 1e-6);
        assert!(!look.panel.blur && !look.toast.blur);
        assert_eq!(look.card.paint, Paint::SurfaceVariant);
        assert!((look.card.radius - 6.0).abs() < 1e-6);
        // Unset keys keep the glass defaults.
        assert!((look.card.hover_opacity - 0.11).abs() < 1e-6);
        assert_eq!(look.toast.shadow.paint, Paint::Primary);
        assert!((look.toast.shadow.size - 4.0).abs() < 1e-6);
        assert!((look.button.radius - 3.0).abs() < 1e-6);
    }

    #[test]
    fn bad_values_are_reported_with_positions() {
        let e = errors(
            "[panel]\nradius = 99\nborder_color = \"red\"\n[palette]\nmode = \"sepia\"\nprimary = \"tint\"\n[font]\nfamily = \" \"\n",
        );
        let keys: Vec<_> = e
            .iter()
            .map(|e| (e.line, e.key.clone().unwrap_or_default()))
            .collect();
        assert_eq!(
            keys,
            vec![
                (2, "panel.radius".to_owned()),
                (3, "panel.border_color".to_owned()),
                (5, "palette.mode".to_owned()),
                (6, "palette.primary".to_owned()),
                (8, "font.family".to_owned()),
            ]
        );
        assert!(e[0].message.contains("out of range (0 to 40)"), "{}", e[0]);
        assert!(e[1].message.contains("#rrggbb or one of"), "{}", e[1]);

        let e = errors("[card]\nradiuss = 3\n");
        assert!(e[0].message.contains("unknown field `radiuss`"), "{}", e[0]);
        let e = errors("[cards]\n");
        assert!(e[0].message.contains("unknown field `cards`"), "{}", e[0]);
    }

    #[test]
    fn shadow_must_fit_around_the_toast() {
        let e = errors("[shadow]\nsize = 10\n");
        assert_eq!(e[0].key.as_deref(), Some("shadow.size"));
        let e = errors("[shadow]\nsize = 8\noffset = 6\n");
        assert_eq!(e[0].key.as_deref(), Some("shadow.offset"));
        assert!(e[0].message.contains("at most 12"), "{}", e[0]);
        assert!(ThemeFile::from_toml("[shadow]\nsize = 12\noffset = 0\n").is_ok());

        // Two patches that each fit are clamped together.
        let mut look = Look::default();
        theme("[shadow]\nsize = 4\noffset = 8\n")
            .look
            .apply(&mut look);
        theme("[shadow]\nsize = 10\noffset = 2\n")
            .look
            .apply(&mut look);
        assert!(look.toast.shadow.size + look.toast.shadow.offset <= SHADOW_ROOM);
    }

    #[test]
    fn config_overrides_layer_over_the_theme() {
        let c = Config::from_toml(
            "[theme]\nname = \"flat\"\n[theme.card]\nradius = 2\n[theme.palette]\nprimary = \"#ff0000\"\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.theme.name.as_deref(), Some("flat"));
        let mut look = theme(builtin("flat").unwrap_or_default()).look();
        let flat_panel = look.panel;
        c.theme.look.apply(&mut look);
        assert!((look.card.radius - 2.0).abs() < 1e-6);
        assert_eq!(look.colors.primary, Some(Rgb(255, 0, 0)));
        assert_eq!(look.panel, flat_panel);

        let Err(ConfigErrors(e)) = Config::from_toml("[theme.card]\nopacity = 2\n") else {
            panic!("expected an error");
        };
        assert_eq!(e[0].key.as_deref(), Some("theme.card.opacity"));
        let Err(ConfigErrors(e)) = Config::from_toml("[theme]\nname = \"../x\"\n") else {
            panic!("expected an error");
        };
        assert_eq!(e[0].key.as_deref(), Some("theme.name"));
    }
}
