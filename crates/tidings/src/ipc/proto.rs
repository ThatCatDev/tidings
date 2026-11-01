//! The line-based IPC protocol between `tidings ctl` and the daemon.
//!
//! Requests are one line each, e.g. `dnd toggle` or `gesture end 0.4 1.2`.
//! Responses are `ok`, `ok <value>` or `err <message>`.

use std::fmt;

use tidings_core::gesture::{GestureEvent, GestureKind};

/// Do Not Disturb change requested by `dnd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndChange {
    /// Turn on.
    On,
    /// Turn off.
    Off,
    /// Flip.
    Toggle,
}

/// A `tidings ctl` command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    /// Open the panel if closed, else close it.
    Toggle,
    /// Open the panel.
    Open,
    /// Close the panel.
    Close,
    /// Dismiss every notification.
    ClearAll,
    /// `None` queries the current state.
    Dnd(Option<DndChange>),
    /// Number of stored notifications.
    Count,
    /// Whether the panel is open.
    IsOpen,
    /// Injects a gesture event, as if from the trackpad (for tests).
    Gesture(GestureEvent),
}

/// A malformed request line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(String);

fn err<T>(msg: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError(msg.into()))
}

fn number(s: Option<&str>, what: &str) -> Result<f32, ParseError> {
    match s.map(str::parse::<f32>) {
        Some(Ok(v)) if v.is_finite() => Ok(v),
        _ => err(format!("expected a number for {what}")),
    }
}

impl Request {
    /// Parses one request line.
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or_default();
        let request = match cmd {
            "toggle" => Self::Toggle,
            "open" => Self::Open,
            "close" => Self::Close,
            "clear-all" => Self::ClearAll,
            "count" => Self::Count,
            "is-open" => Self::IsOpen,
            "dnd" => Self::Dnd(match words.next() {
                None => None,
                Some("on") => Some(DndChange::On),
                Some("off") => Some(DndChange::Off),
                Some("toggle") => Some(DndChange::Toggle),
                Some(other) => return err(format!("dnd: expected on|off|toggle, got {other:?}")),
            }),
            "gesture" => Self::Gesture(match words.next() {
                Some("begin") => match words.next() {
                    Some("open") => GestureEvent::Begin(GestureKind::Open),
                    Some("close") => GestureEvent::Begin(GestureKind::Close),
                    _ => return err("gesture begin: expected open|close"),
                },
                Some("update") => GestureEvent::Update {
                    progress: number(words.next(), "P")?,
                },
                Some("end") => GestureEvent::End {
                    progress: number(words.next(), "P")?,
                    velocity: number(words.next(), "V")?,
                },
                Some("cancel") => GestureEvent::Cancel,
                _ => return err("gesture: expected begin|update|end|cancel"),
            }),
            "" => return err("empty request"),
            other => return err(format!("unknown command {other:?}")),
        };
        match words.next() {
            Some(extra) => err(format!("unexpected argument {extra:?}")),
            None => Ok(request),
        }
    }
}

impl fmt::Display for Request {
    /// The wire form, parseable by [`Request::parse`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toggle => f.write_str("toggle"),
            Self::Open => f.write_str("open"),
            Self::Close => f.write_str("close"),
            Self::ClearAll => f.write_str("clear-all"),
            Self::Count => f.write_str("count"),
            Self::IsOpen => f.write_str("is-open"),
            Self::Dnd(None) => f.write_str("dnd"),
            Self::Dnd(Some(DndChange::On)) => f.write_str("dnd on"),
            Self::Dnd(Some(DndChange::Off)) => f.write_str("dnd off"),
            Self::Dnd(Some(DndChange::Toggle)) => f.write_str("dnd toggle"),
            Self::Gesture(GestureEvent::Begin(GestureKind::Open)) => {
                f.write_str("gesture begin open")
            }
            Self::Gesture(GestureEvent::Begin(GestureKind::Close)) => {
                f.write_str("gesture begin close")
            }
            Self::Gesture(GestureEvent::Update { progress }) => {
                write!(f, "gesture update {progress}")
            }
            Self::Gesture(GestureEvent::End { progress, velocity }) => {
                write!(f, "gesture end {progress} {velocity}")
            }
            Self::Gesture(GestureEvent::Cancel) => f.write_str("gesture cancel"),
        }
    }
}

/// The daemon's reply to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Success, optionally with a value to print.
    Ok(Option<String>),
    /// Failure with a message.
    Err(String),
}

impl Response {
    /// Parses a response line.
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        let line = line.trim_end_matches(['\r', '\n']);
        match line.split_once(' ') {
            None if line == "ok" => Ok(Self::Ok(None)),
            Some(("ok", value)) => Ok(Self::Ok(Some(value.to_owned()))),
            Some(("err", msg)) => Ok(Self::Err(msg.to_owned())),
            None if line == "err" => Ok(Self::Err(String::new())),
            _ => err(format!("malformed response {line:?}")),
        }
    }
}

impl fmt::Display for Response {
    /// The wire form (without the newline). Newlines in values are flattened.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok(None) => f.write_str("ok"),
            Self::Ok(Some(v)) => write!(f, "ok {}", v.replace('\n', " ")),
            Self::Err(msg) => write!(f, "err {}", msg.replace('\n', " ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command_and_round_trips() {
        let cases = [
            ("toggle", Request::Toggle),
            ("open", Request::Open),
            ("close", Request::Close),
            ("clear-all", Request::ClearAll),
            ("count", Request::Count),
            ("is-open", Request::IsOpen),
            ("dnd", Request::Dnd(None)),
            ("dnd on", Request::Dnd(Some(DndChange::On))),
            ("dnd off", Request::Dnd(Some(DndChange::Off))),
            ("dnd toggle", Request::Dnd(Some(DndChange::Toggle))),
            (
                "gesture begin open",
                Request::Gesture(GestureEvent::Begin(GestureKind::Open)),
            ),
            (
                "gesture begin close",
                Request::Gesture(GestureEvent::Begin(GestureKind::Close)),
            ),
            (
                "gesture update 0.25",
                Request::Gesture(GestureEvent::Update { progress: 0.25 }),
            ),
            (
                "gesture update -0.1",
                Request::Gesture(GestureEvent::Update { progress: -0.1 }),
            ),
            (
                "gesture end 0.4 -1.5",
                Request::Gesture(GestureEvent::End {
                    progress: 0.4,
                    velocity: -1.5,
                }),
            ),
            ("gesture cancel", Request::Gesture(GestureEvent::Cancel)),
        ];
        for (line, want) in cases {
            assert_eq!(Request::parse(line), Ok(want), "{line}");
            assert_eq!(
                Request::parse(&want.to_string()),
                Ok(want),
                "round trip {line}"
            );
        }
        assert_eq!(Request::parse("  count \n"), Ok(Request::Count));
    }

    #[test]
    fn rejects_bad_requests() {
        for bad in [
            "",
            "bogus",
            "count extra",
            "dnd maybe",
            "gesture",
            "gesture begin",
            "gesture begin sideways",
            "gesture update",
            "gesture update x",
            "gesture update NaN",
            "gesture update inf",
            "gesture end 0.4",
            "gesture cancel now",
        ] {
            assert!(Request::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn responses_round_trip() {
        for r in [
            Response::Ok(None),
            Response::Ok(Some("3".into())),
            Response::Err("no such thing".into()),
        ] {
            assert_eq!(Response::parse(&r.to_string()), Ok(r));
        }
        assert_eq!(
            Response::parse("ok true\n"),
            Ok(Response::Ok(Some("true".into())))
        );
        assert_eq!(Response::Err("a\nb".into()).to_string(), "err a b");
        assert!(Response::parse("maybe").is_err());
        assert!(Response::parse("").is_err());
    }
}
