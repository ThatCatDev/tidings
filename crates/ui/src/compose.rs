//! The inline reply field's editing model: a single line of text with a
//! caret, the edits keys make to it, and which key makes which edit.
//!
//! Pure and Wayland-free apart from the key names, so every key binding is
//! unit-tested. The Wayland layer owns one [`LineEditor`] per draft and
//! applies an [`Edit`] for each key press.

use smithay_client_toolkit::seat::keyboard::Keysym;
use tidings_core::model::NotificationId;

/// Longest reply kept, in chars. Typing or pasting past it is cut off.
pub const MAX_CHARS: usize = 4000;

/// One change to a [`LineEditor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Type or paste text at the caret (line breaks become spaces).
    Insert(String),
    /// Delete the char before the caret.
    Backspace,
    /// Delete the char after the caret.
    Delete,
    /// Delete back to the start of the word before the caret.
    DeleteWordBack,
    /// Delete up to the end of the word after the caret.
    DeleteWordForward,
    /// Delete everything before the caret.
    DeleteToStart,
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

/// What a key press asks of the reply field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCommand {
    Edit(Edit),
    /// Paste the clipboard.
    Paste,
    /// Send the reply.
    Submit,
    /// Close the field.
    Cancel,
}

/// The modifiers that change what a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// The command for a key press: `keysym` with `mods` held, which typed
/// `utf8` (if anything). `None` for keys the field ignores.
#[must_use]
pub fn command_for(keysym: Keysym, utf8: Option<&str>, mods: Mods) -> Option<KeyCommand> {
    use Edit as E;
    let edit = |e| Some(KeyCommand::Edit(e));
    match keysym {
        Keysym::Return | Keysym::KP_Enter => return Some(KeyCommand::Submit),
        Keysym::Escape => return Some(KeyCommand::Cancel),
        Keysym::BackSpace if mods.ctrl || mods.alt => return edit(E::DeleteWordBack),
        Keysym::BackSpace => return edit(E::Backspace),
        Keysym::Delete | Keysym::KP_Delete if mods.ctrl || mods.alt => {
            return edit(E::DeleteWordForward);
        }
        Keysym::Delete | Keysym::KP_Delete => return edit(E::Delete),
        Keysym::Left | Keysym::KP_Left if mods.ctrl || mods.alt => return edit(E::WordLeft),
        Keysym::Right | Keysym::KP_Right if mods.ctrl || mods.alt => return edit(E::WordRight),
        Keysym::Left | Keysym::KP_Left => return edit(E::Left),
        Keysym::Right | Keysym::KP_Right => return edit(E::Right),
        Keysym::Home | Keysym::KP_Home | Keysym::Up => return edit(E::Home),
        Keysym::End | Keysym::KP_End | Keysym::Down => return edit(E::End),
        Keysym::Insert if mods.shift => return Some(KeyCommand::Paste),
        _ => {}
    }
    if mods.ctrl {
        // Readline-style bindings, as in a terminal or a GTK entry.
        return match keysym {
            Keysym::v | Keysym::V => Some(KeyCommand::Paste),
            Keysym::a | Keysym::A => edit(E::Home),
            Keysym::e | Keysym::E => edit(E::End),
            Keysym::u | Keysym::U => edit(E::DeleteToStart),
            Keysym::w | Keysym::W => edit(E::DeleteWordBack),
            _ => None,
        };
    }
    if mods.alt {
        return None;
    }
    utf8.filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .map(|s| KeyCommand::Edit(E::Insert(s.to_owned())))
}

/// A single line of text and a caret, which always sits on a char boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineEditor {
    text: String,
    /// Byte offset of the caret.
    caret: usize,
}

impl LineEditor {
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Byte offset of the caret in [`Self::text`].
    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Applies `edit`; returns whether the text or caret changed.
    pub fn apply(&mut self, edit: &Edit) -> bool {
        match edit {
            Edit::Insert(s) => self.insert(s),
            Edit::Backspace => self.delete_to(self.prev_char()),
            Edit::Delete => self.delete_to(self.next_char()),
            Edit::DeleteWordBack => self.delete_to(self.prev_word()),
            Edit::DeleteWordForward => self.delete_to(self.next_word()),
            Edit::DeleteToStart => self.delete_to(0),
            Edit::Left => self.move_to(self.prev_char()),
            Edit::Right => self.move_to(self.next_char()),
            Edit::WordLeft => self.move_to(self.prev_word()),
            Edit::WordRight => self.move_to(self.next_word()),
            Edit::Home => self.move_to(0),
            Edit::End => self.move_to(self.text.len()),
        }
    }

    fn insert(&mut self, s: &str) -> bool {
        let room = MAX_CHARS.saturating_sub(self.text.chars().count());
        // One line: line breaks and tabs become spaces, other controls go.
        let clean: String = s
            .chars()
            .filter_map(|c| match c {
                '\n' | '\r' | '\t' => Some(' '),
                c if c.is_control() => None,
                c => Some(c),
            })
            .take(room)
            .collect();
        if clean.is_empty() {
            return false;
        }
        self.text.insert_str(self.caret, &clean);
        self.caret += clean.len();
        true
    }

    /// Deletes between the caret and `to` (either side); the caret ends at
    /// the start of the deleted range.
    fn delete_to(&mut self, to: usize) -> bool {
        let (start, end) = if to < self.caret {
            (to, self.caret)
        } else {
            (self.caret, to)
        };
        if start == end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.caret = start;
        true
    }

    fn move_to(&mut self, to: usize) -> bool {
        let moved = to != self.caret;
        self.caret = to;
        moved
    }

    fn prev_char(&self) -> usize {
        self.text[..self.caret]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_char(&self) -> usize {
        self.text[self.caret..]
            .chars()
            .next()
            .map_or(self.caret, |c| self.caret + c.len_utf8())
    }

    /// Start of the word before the caret, skipping spaces first.
    fn prev_word(&self) -> usize {
        self.text[..self.caret]
            .trim_end_matches(|c| !is_word(c))
            .trim_end_matches(is_word)
            .len()
    }

    /// End of the word after the caret, skipping spaces first.
    fn next_word(&self) -> usize {
        let tail = &self.text[self.caret..];
        let rest = tail
            .trim_start_matches(|c| !is_word(c))
            .trim_start_matches(is_word);
        self.caret + tail.len() - rest.len()
    }
}

/// Word characters, for word-wise moves and deletes.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// The reply field as layout sees it: which card, what is typed, and
/// whether it has the keyboard (and so shows a caret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyDraft {
    pub id: NotificationId,
    pub text: String,
    /// Byte offset of the caret in `text`.
    pub caret: usize,
    pub focused: bool,
}

impl ReplyDraft {
    /// The draft for `id` from `editor`.
    #[must_use]
    pub fn new(id: NotificationId, editor: &LineEditor, focused: bool) -> Self {
        Self {
            id,
            text: editor.text().to_owned(),
            caret: editor.caret(),
            focused,
        }
    }

    /// Whether there is anything to send.
    #[must_use]
    pub fn can_send(&self) -> bool {
        !self.text.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str, caret: usize) -> LineEditor {
        LineEditor {
            text: text.to_owned(),
            caret,
        }
    }

    fn after(text: &str, caret: usize, edit: &Edit) -> (String, usize) {
        let mut e = editor(text, caret);
        e.apply(edit);
        (e.text, e.caret)
    }

    #[test]
    fn typing_inserts_at_the_caret() {
        let mut e = LineEditor::default();
        assert!(e.apply(&Edit::Insert("héllo".into())));
        assert_eq!((e.text(), e.caret()), ("héllo", 6));
        e.apply(&Edit::Home);
        e.apply(&Edit::Insert("👋 ".into()));
        assert_eq!((e.text(), e.caret()), ("👋 héllo", 5));
        // Line breaks and tabs become spaces; other controls are dropped.
        e.apply(&Edit::End);
        e.apply(&Edit::Insert("\na\tb\u{7}".into()));
        assert_eq!(e.text(), "👋 héllo a b");
        assert!(!e.apply(&Edit::Insert("\u{7}".into())), "nothing to insert");
    }

    #[test]
    fn inserting_stops_at_the_limit() {
        let mut e = LineEditor::default();
        e.apply(&Edit::Insert("x".repeat(MAX_CHARS - 1)));
        e.apply(&Edit::Insert("éé".into()));
        assert_eq!(e.text().chars().count(), MAX_CHARS);
        assert!(e.text().ends_with('é'));
        assert!(!e.apply(&Edit::Insert("y".into())));
    }

    #[test]
    fn char_moves_and_deletes_respect_utf8() {
        assert_eq!(after("aé", 3, &Edit::Backspace), ("a".into(), 1));
        assert_eq!(after("aé", 1, &Edit::Delete), ("a".into(), 1));
        assert_eq!(after("aé", 3, &Edit::Left), ("aé".into(), 1));
        assert_eq!(after("aé", 1, &Edit::Right), ("aé".into(), 3));
        // At the ends nothing happens.
        let mut e = editor("ab", 0);
        assert!(!e.apply(&Edit::Backspace));
        assert!(!e.apply(&Edit::Left));
        let mut e = editor("ab", 2);
        assert!(!e.apply(&Edit::Delete));
        assert!(!e.apply(&Edit::Right));
    }

    #[test]
    fn word_moves_skip_spaces_then_a_word() {
        let s = "see you  at 6";
        assert_eq!(after(s, s.len(), &Edit::WordLeft).1, 12);
        assert_eq!(after(s, 12, &Edit::WordLeft).1, 9);
        assert_eq!(after(s, 9, &Edit::WordLeft).1, 4);
        assert_eq!(after(s, 2, &Edit::WordLeft).1, 0);
        assert_eq!(after(s, 0, &Edit::WordRight).1, 3);
        assert_eq!(after(s, 3, &Edit::WordRight).1, 7);
        assert_eq!(after(s, 7, &Edit::WordRight).1, 11);
        assert_eq!(after("don't go", 0, &Edit::WordRight).1, 5);
        assert_eq!(after("a, b", 4, &Edit::WordLeft).1, 3);
    }

    #[test]
    fn word_and_line_deletes() {
        assert_eq!(
            after("see you at", 10, &Edit::DeleteWordBack),
            ("see you ".into(), 8)
        );
        assert_eq!(
            after("see you at", 3, &Edit::DeleteWordForward),
            ("see at".into(), 3)
        );
        assert_eq!(after("see you", 4, &Edit::DeleteToStart), ("you".into(), 0));
        assert_eq!(after("one", 3, &Edit::DeleteWordBack), (String::new(), 0));
    }

    #[test]
    fn keys_map_to_commands() {
        let none = Mods::default();
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        let cmd = |k, s: Option<&str>, m| command_for(k, s, m);
        let edit = |e| Some(KeyCommand::Edit(e));
        assert_eq!(
            cmd(Keysym::Return, Some("\r"), none),
            Some(KeyCommand::Submit)
        );
        assert_eq!(cmd(Keysym::KP_Enter, None, none), Some(KeyCommand::Submit));
        assert_eq!(cmd(Keysym::Escape, None, none), Some(KeyCommand::Cancel));
        assert_eq!(
            cmd(Keysym::a, Some("a"), none),
            edit(Edit::Insert("a".into()))
        );
        assert_eq!(
            cmd(Keysym::A, Some("A"), shift),
            edit(Edit::Insert("A".into()))
        );
        assert_eq!(
            cmd(Keysym::BackSpace, Some("\u{8}"), none),
            edit(Edit::Backspace)
        );
        assert_eq!(
            cmd(Keysym::BackSpace, None, ctrl),
            edit(Edit::DeleteWordBack)
        );
        assert_eq!(
            cmd(Keysym::Delete, None, ctrl),
            edit(Edit::DeleteWordForward)
        );
        assert_eq!(cmd(Keysym::Left, None, ctrl), edit(Edit::WordLeft));
        assert_eq!(cmd(Keysym::Right, None, none), edit(Edit::Right));
        assert_eq!(cmd(Keysym::Home, None, none), edit(Edit::Home));
        assert_eq!(
            cmd(Keysym::v, Some("\u{16}"), ctrl),
            Some(KeyCommand::Paste)
        );
        assert_eq!(cmd(Keysym::V, None, ctrl), Some(KeyCommand::Paste));
        assert_eq!(cmd(Keysym::Insert, None, shift), Some(KeyCommand::Paste));
        assert_eq!(cmd(Keysym::Delete, None, none), edit(Edit::Delete));
        assert_eq!(cmd(Keysym::KP_Left, None, none), edit(Edit::Left));
        assert_eq!(cmd(Keysym::Up, None, none), edit(Edit::Home));
        assert_eq!(cmd(Keysym::End, None, none), edit(Edit::End));
        assert_eq!(cmd(Keysym::Down, None, none), edit(Edit::End));
        // Readline's Ctrl+A / Ctrl+E, in either case.
        assert_eq!(cmd(Keysym::A, None, ctrl), edit(Edit::Home));
        assert_eq!(cmd(Keysym::e, None, ctrl), edit(Edit::End));
        assert_eq!(cmd(Keysym::u, None, ctrl), edit(Edit::DeleteToStart));
        assert_eq!(cmd(Keysym::w, None, ctrl), edit(Edit::DeleteWordBack));
        // Other shortcuts and bare modifiers do nothing.
        assert_eq!(cmd(Keysym::q, Some("\u{11}"), ctrl), None);
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        assert_eq!(cmd(Keysym::x, Some("x"), alt), None);
        assert_eq!(cmd(Keysym::Shift_L, None, shift), None);
        assert_eq!(cmd(Keysym::Tab, Some("\t"), none), None);
    }

    #[test]
    fn drafts_only_send_real_text() {
        let id = NotificationId(1);
        assert!(!ReplyDraft::new(id, &editor("  ", 2), true).can_send());
        assert!(ReplyDraft::new(id, &editor(" hi", 3), false).can_send());
    }
}
