//! Text measurement seam between pure layout and the font engine.
//!
//! Layout needs to know how tall wrapped text is and what the ellipsized
//! string looks like. The real implementation shapes with cosmic-text
//! ([`crate::render::TextEngine`]); tests use [`MonoMeasure`], a deterministic
//! fixed-advance fake.

use crate::style::{TextStyle, Type};

/// Result of fitting text into a box.
#[derive(Debug, Clone, PartialEq)]
pub struct TextFit {
    /// The string to draw: the input, or a prefix ending in "…" if it was
    /// clamped. Single-line styles have newlines folded into spaces.
    pub text: String,
    /// Width of the widest laid-out line.
    pub width: f32,
    /// Height of all laid-out lines.
    pub height: f32,
    pub lines: u32,
}

/// Measures text for layout. Implementations may cache.
pub trait TextMeasure {
    /// Fits `text` into `max_width`, wrapping at word boundaries and
    /// clamping to `max_lines` with a trailing ellipsis.
    fn fit(&mut self, text: &str, style: TextStyle, max_width: f32, max_lines: u32) -> TextFit;

    /// Height of one line of `style`, in logical px.
    fn line_height(&self, style: TextStyle) -> f32;
}

/// Normalises text before shaping: trims, and for single-line styles folds
/// line breaks and runs of whitespace into single spaces.
#[must_use]
pub fn normalise(text: &str, style: TextStyle) -> String {
    if style.single_line() {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        text.lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned()
    }
}

/// Finds the longest prefix (cut on a char boundary) that `fits`, appends an
/// ellipsis, and returns it. `fits` must be monotonic in prefix length.
pub fn ellipsize(text: &str, mut fits: impl FnMut(&str) -> bool) -> String {
    let bounds: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let candidate = |n: usize| {
        let end = bounds.get(n).copied().unwrap_or(text.len());
        format!("{}…", text[..end].trim_end())
    };
    // Binary search for the largest n in 0..=len with candidate(n) fitting.
    let (mut lo, mut hi) = (0usize, bounds.len());
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(&candidate(mid)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    candidate(lo)
}

/// Deterministic fake at the default type: every char advances
/// `0.5 * size`, greedy word wrap.
#[derive(Debug, Default, Clone, Copy)]
pub struct MonoMeasure;

impl MonoMeasure {
    fn advance(style: TextStyle) -> f32 {
        Type::default().size(style) * 0.5
    }

    /// Greedy word-wrap returning line widths.
    fn wrap(text: &str, style: TextStyle, max_width: f32) -> Vec<f32> {
        let adv = Self::advance(style);
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut cur = 0.0f32;
            for word in para.split(' ') {
                let w = crate::geom::u32_to_f32(
                    u32::try_from(word.chars().count()).unwrap_or(u32::MAX),
                ) * adv;
                let with_space = if cur > 0.0 { cur + adv + w } else { w };
                if with_space <= max_width || cur == 0.0 {
                    cur = with_space;
                } else {
                    lines.push(cur);
                    cur = w;
                }
            }
            lines.push(cur);
        }
        lines
    }
}

impl TextMeasure for MonoMeasure {
    fn fit(&mut self, text: &str, style: TextStyle, max_width: f32, max_lines: u32) -> TextFit {
        let text = normalise(text, style);
        let max_lines = max_lines.max(1) as usize;
        let fits = |s: &str| Self::wrap(s, style, max_width).len() <= max_lines;
        let text = if fits(&text) {
            text
        } else {
            ellipsize(&text, fits)
        };
        let lines = Self::wrap(&text, style, max_width);
        let n = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        TextFit {
            width: lines.iter().copied().fold(0.0, f32::max).min(max_width),
            height: crate::geom::u32_to_f32(n) * self.line_height(style),
            lines: n,
            text,
        }
    }

    fn line_height(&self, style: TextStyle) -> f32 {
        Type::default().line_height(style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_single_line_folds_whitespace() {
        assert_eq!(normalise("  a\n b\t c ", TextStyle::Meta), "a b c");
        assert_eq!(normalise(" a \nb\n", TextStyle::Body), "a\nb");
    }

    #[test]
    fn ellipsize_finds_longest_prefix() {
        let s = ellipsize("hello world", |c| c.chars().count() <= 6);
        assert_eq!(s, "hello…");
        let s = ellipsize("abc", |c| c.chars().count() <= 1);
        assert_eq!(s, "…");
    }

    #[test]
    fn mono_wraps_and_clamps() {
        let mut m = MonoMeasure;
        // Body: 13px -> 6.5 px per char. 65 px = 10 chars per line.
        let fit = m.fit("aaaa bbbb cccc dddd eeee", TextStyle::Body, 65.0, 2);
        assert_eq!(fit.lines, 2);
        assert!(fit.text.ends_with('…'));
        assert!((fit.height - 2.0 * m.line_height(TextStyle::Body)).abs() < 1e-3);
        let fit = m.fit("short", TextStyle::Body, 65.0, 2);
        assert_eq!(fit.text, "short");
        assert_eq!(fit.lines, 1);
    }
}
