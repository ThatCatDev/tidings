//! A UI that renders nothing.
//!
//! Used for headless runs (until `tidings-ui` is wired in) and in tests,
//! where an optional tap lets the test observe every [`UiUpdate`].

use std::sync::mpsc;

use tidings_core::ui::{UiHandle, UiUpdate};

/// Discards UI updates, optionally forwarding copies to a tap.
#[derive(Debug, Default)]
pub struct NullUi {
    tap: Option<mpsc::Sender<UiUpdate>>,
}

impl NullUi {
    /// A UI that drops everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A UI that forwards every update to `tap`.
    #[must_use]
    pub fn with_tap(tap: mpsc::Sender<UiUpdate>) -> Self {
        Self { tap: Some(tap) }
    }
}

impl UiHandle for NullUi {
    fn send(&self, update: UiUpdate) {
        tracing::trace!(?update, "null ui");
        if let Some(tap) = &self.tap {
            // A test that stopped listening is not an error.
            let _ = tap.send(update);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_sees_updates_and_plain_null_drops_them() {
        NullUi::new().send(UiUpdate::Shutdown);
        let (tx, rx) = mpsc::channel();
        let ui = NullUi::with_tap(tx);
        ui.send(UiUpdate::Shutdown);
        assert_eq!(rx.try_recv(), Ok(UiUpdate::Shutdown));
        drop(rx);
        ui.send(UiUpdate::Shutdown); // receiver gone: ignored
    }
}
