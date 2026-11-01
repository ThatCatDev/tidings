//! Reduces notification body markup to plain text.
//!
//! The spec's body markup is a small XML-ish subset (`<b>`, `<i>`, `<u>`,
//! `<a href>`, `<img>`) with XML entities. Clients often send stray `<` or
//! `&` unescaped, so this is a forgiving scanner rather than a parser: a `<`
//! only starts a tag when it looks like one and has a closing `>`.

/// Strips tags and decodes entities. `<br>` becomes a newline.
#[must_use]
pub fn strip_markup(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(i) = rest.find(['<', '&']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if rest.starts_with('<') {
            if let Some(len) = tag_len(rest) {
                if is_line_break(&rest[..len]) {
                    out.push('\n');
                }
                rest = &rest[len..];
            } else {
                out.push('<');
                rest = &rest[1..];
            }
        } else {
            if let Some((ch, len)) = decode_entity(rest) {
                out.push(ch);
                rest = &rest[len..];
            } else {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Length of the tag at the start of `s` (which starts with `<`), if it is one.
fn tag_len(s: &str) -> Option<usize> {
    let next = s[1..].chars().next()?;
    if !(next.is_ascii_alphabetic() || next == '/' || next == '!' || next == '?') {
        return None;
    }
    let end = s.find('>')?;
    // A second `<` before the `>` means this `<` was literal text.
    if s[1..end].contains('<') {
        return None;
    }
    Some(end + 1)
}

fn is_line_break(tag: &str) -> bool {
    let name = tag[1..tag.len() - 1].trim_end_matches('/').trim();
    name.eq_ignore_ascii_case("br")
}

/// Decodes the entity at the start of `s` (which starts with `&`).
fn decode_entity(s: &str) -> Option<(char, usize)> {
    let (end, _) = s.char_indices().take(12).find(|&(_, c)| c == ';')?;
    let name = &s[1..end];
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((ch, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags() {
        assert_eq!(strip_markup("<b>Bold</b> and <i>it</i>"), "Bold and it");
        assert_eq!(
            strip_markup(r#"see <a href="https://x.org/?a=1&amp;b=2">link</a>"#),
            "see link"
        );
        assert_eq!(
            strip_markup(r#"<img src="x.png" alt="pic"/>after"#),
            "after"
        );
        assert_eq!(
            strip_markup("one<br>two<br/>three<BR />four"),
            "one\ntwo\nthree\nfour"
        );
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            strip_markup("a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;"),
            "a & b <c> \"d\" 'e'"
        );
        assert_eq!(strip_markup("&#65;&#x42;&#X43;"), "ABC");
        assert_eq!(strip_markup("x&nbsp;y"), "x\u{a0}y");
    }

    #[test]
    fn tolerates_stray_characters() {
        assert_eq!(strip_markup("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2");
        assert_eq!(strip_markup("a <3 b"), "a <3 b");
        assert_eq!(strip_markup("x < <b>y</b>"), "x < y");
        assert_eq!(strip_markup("unclosed <b"), "unclosed <b");
        assert_eq!(strip_markup("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(
            strip_markup("&bogus; &#xZZ; &#1114112; &"),
            "&bogus; &#xZZ; &#1114112; &"
        );
        assert_eq!(strip_markup("trailing <"), "trailing <");
        assert_eq!(strip_markup("ünïcödé <i>ok</i>"), "ünïcödé ok");
        assert_eq!(strip_markup("&ééééééééééé;"), "&ééééééééééé;");
    }
}
