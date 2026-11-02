//! The user config file (`config.toml`): its schema, parsing and validation.
//!
//! Parsing happens in two stages:
//!
//! 1. **Shape.** serde reads the TOML into raw structs that mirror the file.
//!    Unknown keys and sections are errors (a typo is never silently
//!    ignored), and every value must have the right TOML type.
//! 2. **Values.** Each raw value is checked (durations parse, numbers are in
//!    range) and turned into the typed [`Config`]. All problems are
//!    collected, not just the first.
//!
//! Raw values keep their byte spans, so every [`ConfigError`] points at a
//! line and column. Every key is optional; a missing key takes its default,
//! so an empty file is the default config.

use std::fmt;
use std::ops::{Range, RangeInclusive};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use toml::Spanned;

pub mod theme;

use crate::panel::Swipe;
use crate::policy::ToastPolicy;

/// A commented config file listing every setting at its default.
pub const TEMPLATE: &str = include_str!("../../../../config.example.toml");

/// Everything the config file controls.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    /// Toast timing and limits.
    pub toasts: ToastPolicy,
    pub gestures: GestureConfig,
    pub animation: AnimationConfig,
    pub theme: ThemeConfig,
}

/// The trackpad edge swipe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureConfig {
    /// Read the trackpad at all.
    pub enabled: bool,
    /// Width of the right-edge zone a swipe must start in (fraction of pad width).
    pub edge_zone: f64,
    /// How far a swipe travels and how fast a flick must be.
    pub swipe: Swipe,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            edge_zone: 0.10,
            swipe: Swipe::default(),
        }
    }
}

/// Motion of the panel, toasts and groups.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationConfig {
    /// `false` makes every change instant.
    pub enabled: bool,
    /// Playback speed: 2.0 is twice as fast, 0.5 half as fast.
    pub speed: f32,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
        }
    }
}

/// The theme, its overrides, and where colours come from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThemeConfig {
    /// The colours file, as written (a leading `~/` is the caller's to
    /// expand). `None` uses the standard location.
    pub colors: Option<PathBuf>,
    /// The theme to use (a file in the themes folder, or a built-in);
    /// `None` is [`theme::DEFAULT_THEME`].
    pub name: Option<String>,
    /// Look settings written in the config itself, layered over the theme.
    pub look: theme::LookPatch,
}

/// Accepted ranges, also quoted in error messages.
mod limits {
    use std::ops::RangeInclusive;
    use std::time::Duration;

    pub const TIMEOUT: RangeInclusive<Duration> =
        Duration::from_millis(500)..=Duration::from_secs(3600);
    pub const MAX_VISIBLE: RangeInclusive<i64> = 1..=10;
    pub const EDGE_ZONE: RangeInclusive<f64> = 0.02..=0.5;
    pub const SWIPE_DISTANCE: RangeInclusive<f64> = 0.05..=1.0;
    pub const FLICK_VELOCITY: RangeInclusive<f64> = 0.05..=10.0;
    pub const SPEED: RangeInclusive<f64> = 0.25..=4.0;
}

/// One problem in a config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// 1-based line of the offending text (0 when unknown).
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
    /// Dotted key, e.g. `toasts.max_visible`, when the problem is a value.
    pub key: Option<String>,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(f, "line {}, column {}: ", self.line, self.column)?;
        }
        if let Some(key) = &self.key {
            write!(f, "{key}: ")?;
        }
        f.write_str(&self.message)
    }
}

/// Every problem found in a config file, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigErrors(pub Vec<ConfigError>);

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigErrors {}

impl Config {
    /// Parses and validates a config file's contents.
    pub fn from_toml(src: &str) -> Result<Self, ConfigErrors> {
        let raw: RawConfig = toml::from_str(src).map_err(|e| {
            let (line, column) = e.span().map_or((0, 0), |span| line_column(src, span.start));
            ConfigErrors(vec![ConfigError {
                line,
                column,
                key: None,
                message: e.message().trim().to_owned(),
            }])
        })?;
        let mut check = Checker {
            src,
            errors: Vec::new(),
        };
        let config = raw.validate(&mut check);
        check.finish(config)
    }
}

// ---- Stage 1: the file's shape --------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawConfig {
    toasts: RawToasts,
    gestures: RawGestures,
    animation: RawAnimation,
    #[serde(deserialize_with = "theme::deserialize_section")]
    theme: theme::ThemeSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawToasts {
    low_timeout: Option<Spanned<DurationText>>,
    normal_timeout: Option<Spanned<DurationText>>,
    critical_timeout: Option<Spanned<DurationText>>,
    max_visible: Option<Spanned<i64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGestures {
    enabled: Option<bool>,
    edge_zone: Option<Spanned<f64>>,
    swipe_distance: Option<Spanned<f64>>,
    flick_velocity: Option<Spanned<f64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnimation {
    enabled: Option<bool>,
    speed: Option<Spanned<f64>>,
}

/// A duration written as text, e.g. `"4s"`, `"1.5s"`, `"250ms"`, `"2m"`.
/// Kept as text here so a bad one is reported with its position.
#[derive(Debug, Clone)]
struct DurationText(String);

impl<'de> Deserialize<'de> for DurationText {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = DurationText;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(r#"a duration in quotes, such as "4s", "1.5s" or "250ms""#)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<DurationText, E> {
                Ok(DurationText(v.to_owned()))
            }
        }
        d.deserialize_str(Visitor)
    }
}

/// Parses `"<number><unit>"` with unit `ms`, `s` or `m`.
fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    let split = text.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = text.split_at(split);
    let value: f64 = number.trim().parse().ok()?;
    let secs = match unit {
        "ms" => value / 1000.0,
        "s" => value,
        "m" => value * 60.0,
        _ => return None,
    };
    Duration::try_from_secs_f64(secs).ok()
}

// ---- Stage 2: values -------------------------------------------------------

/// Collects value errors with their positions.
struct Checker<'a> {
    src: &'a str,
    errors: Vec<ConfigError>,
}

impl Checker<'_> {
    /// `value` if nothing was wrong, else every error in file order.
    fn finish<T>(mut self, value: T) -> Result<T, ConfigErrors> {
        if self.errors.is_empty() {
            Ok(value)
        } else {
            self.errors.sort_by_key(|e| (e.line, e.column));
            Err(ConfigErrors(self.errors))
        }
    }

    fn error(&mut self, span: &Range<usize>, key: &str, message: String) {
        let (line, column) = line_column(self.src, span.start);
        self.errors.push(ConfigError {
            line,
            column,
            key: Some(key.to_owned()),
            message,
        });
    }

    /// A duration within `range`; `"never"` is `None` when `never_ok`.
    fn duration(
        &mut self,
        value: Option<Spanned<DurationText>>,
        key: &str,
        default: Option<Duration>,
        never_ok: bool,
    ) -> Option<Duration> {
        let Some(value) = value else { return default };
        let span = value.span();
        let text = &value.get_ref().0;
        if never_ok && text.trim() == "never" {
            return None;
        }
        let range = limits::TIMEOUT;
        match parse_duration(text) {
            Some(d) if range.contains(&d) => Some(d),
            Some(_) => {
                let msg = format!(
                    "{text:?} is out of range ({:?} to {:?})",
                    range.start(),
                    range.end()
                );
                self.error(&span, key, msg);
                default
            }
            None => {
                let never = if never_ok { r#", or "never""# } else { "" };
                let msg = format!(
                    r#"{text:?} is not a duration; use a number and a unit, such as "4s", "1.5s" or "250ms"{never}"#
                );
                self.error(&span, key, msg);
                default
            }
        }
    }

    /// A number within `range`.
    fn number(
        &mut self,
        value: Option<Spanned<f64>>,
        key: &str,
        default: f64,
        range: &RangeInclusive<f64>,
    ) -> f64 {
        let Some(value) = value else { return default };
        let v = *value.get_ref();
        if range.contains(&v) {
            v
        } else {
            let msg = format!("{v} is out of range ({} to {})", range.start(), range.end());
            self.error(&value.span(), key, msg);
            default
        }
    }

    /// A whole number within `range`.
    fn count(
        &mut self,
        value: Option<Spanned<i64>>,
        key: &str,
        default: usize,
        range: &RangeInclusive<i64>,
    ) -> usize {
        let Some(value) = value else { return default };
        let v = *value.get_ref();
        match usize::try_from(v) {
            Ok(n) if range.contains(&v) => n,
            _ => {
                let msg = format!("{v} is out of range ({} to {})", range.start(), range.end());
                self.error(&value.span(), key, msg);
                default
            }
        }
    }
}

/// `f64` to `f32` for values already range-checked to be small.
#[allow(
    clippy::cast_possible_truncation,
    reason = "callers pass range-checked values far inside f32's range"
)]
fn narrow(v: f64) -> f32 {
    v as f32
}

impl RawConfig {
    fn validate(self, check: &mut Checker<'_>) -> Config {
        let defaults = Config::default();
        let t = self.toasts;
        let toasts = ToastPolicy {
            low_timeout: check
                .duration(
                    t.low_timeout,
                    "toasts.low_timeout",
                    Some(defaults.toasts.low_timeout),
                    false,
                )
                .unwrap_or(defaults.toasts.low_timeout),
            normal_timeout: check
                .duration(
                    t.normal_timeout,
                    "toasts.normal_timeout",
                    Some(defaults.toasts.normal_timeout),
                    false,
                )
                .unwrap_or(defaults.toasts.normal_timeout),
            critical_timeout: check.duration(
                t.critical_timeout,
                "toasts.critical_timeout",
                defaults.toasts.critical_timeout,
                true,
            ),
            max_visible: check.count(
                t.max_visible,
                "toasts.max_visible",
                defaults.toasts.max_visible,
                &limits::MAX_VISIBLE,
            ),
        };

        let g = self.gestures;
        let dg = defaults.gestures;
        let gestures = GestureConfig {
            enabled: g.enabled.unwrap_or(dg.enabled),
            edge_zone: check.number(
                g.edge_zone,
                "gestures.edge_zone",
                dg.edge_zone,
                &limits::EDGE_ZONE,
            ),
            swipe: Swipe {
                full_travel: narrow(check.number(
                    g.swipe_distance,
                    "gestures.swipe_distance",
                    f64::from(dg.swipe.full_travel),
                    &limits::SWIPE_DISTANCE,
                )),
                flick_velocity: narrow(check.number(
                    g.flick_velocity,
                    "gestures.flick_velocity",
                    f64::from(dg.swipe.flick_velocity),
                    &limits::FLICK_VELOCITY,
                )),
            },
        };

        let a = self.animation;
        let da = defaults.animation;
        let animation = AnimationConfig {
            enabled: a.enabled.unwrap_or(da.enabled),
            speed: narrow(check.number(
                a.speed,
                "animation.speed",
                f64::from(da.speed),
                &limits::SPEED,
            )),
        };

        let t = self.theme;
        let name = t.name.and_then(|n| {
            let text = n.get_ref().trim();
            if theme::is_valid_name(text) {
                Some(text.to_owned())
            } else {
                let msg = format!(
                    "{text:?} is not a theme name; use letters, digits, - and _ (see `tidings theme list`)"
                );
                check.error(&n.span(), "theme.name", msg);
                None
            }
        });
        let look = t.look.check(check, "theme.");
        let colors = t.colors.and_then(|c| {
            if c.get_ref().trim().is_empty() {
                check.error(&c.span(), "theme.colors", "must not be empty".to_owned());
                None
            } else {
                Some(PathBuf::from(c.into_inner()))
            }
        });

        Config {
            toasts,
            gestures,
            animation,
            theme: ThemeConfig { colors, name, look },
        }
    }
}

/// 1-based line and character column of byte `offset` in `src`.
fn line_column(src: &str, offset: usize) -> (usize, usize) {
    let before = src.get(..offset).unwrap_or(src);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(src: &str) -> Vec<ConfigError> {
        match Config::from_toml(src) {
            Ok(c) => panic!("expected errors, got {c:?}"),
            Err(ConfigErrors(e)) => e,
        }
    }

    #[test]
    fn empty_file_and_template_are_the_defaults() {
        assert_eq!(Config::from_toml(""), Ok(Config::default()));
        assert_eq!(Config::from_toml(TEMPLATE), Ok(Config::default()));
    }

    #[test]
    fn reads_every_setting() {
        let src = r#"
            [toasts]
            low_timeout = "2.5s"
            normal_timeout = "1m"
            critical_timeout = "750ms"
            max_visible = 2

            [gestures]
            enabled = false
            edge_zone = 0.2
            swipe_distance = 0.5
            flick_velocity = 1

            [animation]
            enabled = false
            speed = 2.0

            [theme]
            colors = "~/themes/tidings.json"
        "#;
        let c = Config::from_toml(src).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.toasts.low_timeout, Duration::from_millis(2500));
        assert_eq!(c.toasts.normal_timeout, Duration::from_secs(60));
        assert_eq!(c.toasts.critical_timeout, Some(Duration::from_millis(750)));
        assert_eq!(c.toasts.max_visible, 2);
        assert!(!c.gestures.enabled);
        assert!((c.gestures.edge_zone - 0.2).abs() < 1e-9);
        assert!((c.gestures.swipe.full_travel - 0.5).abs() < 1e-6);
        assert!((c.gestures.swipe.flick_velocity - 1.0).abs() < 1e-6);
        assert!(!c.animation.enabled);
        assert!((c.animation.speed - 2.0).abs() < 1e-6);
        assert_eq!(c.theme.colors, Some(PathBuf::from("~/themes/tidings.json")));
    }

    #[test]
    fn critical_never_is_the_default_and_can_be_spelled_out() {
        let c = Config::from_toml("[toasts]\ncritical_timeout = \"never\"");
        assert_eq!(c.map(|c| c.toasts.critical_timeout), Ok(None));
        let e = errors("[toasts]\nlow_timeout = \"never\"");
        assert!(e[0].message.contains("not a duration"), "{}", e[0]);
    }

    #[test]
    fn unknown_keys_and_sections_are_errors_with_positions() {
        let e = errors("[toasts]\nmax_visibel = 3\n");
        assert_eq!((e[0].line, e[0].column), (2, 1));
        assert!(
            e[0].message.contains("unknown field `max_visibel`"),
            "{}",
            e[0]
        );

        let e = errors("[toast]\n");
        assert!(e[0].message.contains("unknown field `toast`"), "{}", e[0]);
    }

    #[test]
    fn wrong_types_are_errors_with_positions() {
        let e = errors("[toasts]\nmax_visible = \"four\"\n");
        assert_eq!(e[0].line, 2);
        assert!(e[0].message.contains("invalid type"), "{}", e[0]);

        let e = errors("[toasts]\nlow_timeout = 4\n");
        assert!(e[0].message.contains(r#"such as "4s""#), "{}", e[0]);

        let e = errors("[animation]\nenabled = \"yes\"\n");
        assert!(e[0].message.contains("invalid type"), "{}", e[0]);

        let e = errors("toasts = 3\n");
        assert_eq!(e[0].line, 1);
    }

    #[test]
    fn every_bad_value_is_reported_in_file_order() {
        let src = "[toasts]\nmax_visible = 0\nlow_timeout = \"4 parsecs\"\n\n[animation]\nspeed = 9.0\n\n[gestures]\nedge_zone = nan\n";
        let e = errors(src);
        let summary: Vec<_> = e
            .iter()
            .map(|e| (e.line, e.column, e.key.clone().unwrap_or_default()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (2, 15, "toasts.max_visible".to_owned()),
                (3, 15, "toasts.low_timeout".to_owned()),
                (6, 9, "animation.speed".to_owned()),
                (9, 13, "gestures.edge_zone".to_owned()),
            ]
        );
        assert!(e[0].message.contains("out of range (1 to 10)"), "{}", e[0]);
        assert_eq!(
            e[2].to_string(),
            "line 6, column 9: animation.speed: 9 is out of range (0.25 to 4)"
        );
    }

    #[test]
    fn durations_parse_with_units_only() {
        assert_eq!(parse_duration("250ms"), Some(Duration::from_millis(250)));
        assert_eq!(parse_duration("1.5s"), Some(Duration::from_millis(1500)));
        assert_eq!(parse_duration("2m"), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration(" 3 s "), Some(Duration::from_secs(3)));
        assert_eq!(parse_duration("3"), None);
        assert_eq!(parse_duration("3h"), None);
        assert_eq!(parse_duration("-1s"), None);
        let e = errors("[toasts]\nnormal_timeout = \"2h\"\n");
        assert!(e[0].message.contains("not a duration"), "{}", e[0]);
        let e = errors("[toasts]\nnormal_timeout = \"100ms\"\n");
        assert!(e[0].message.contains("out of range"), "{}", e[0]);
    }
}
