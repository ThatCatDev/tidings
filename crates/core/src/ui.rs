//! CONTRACT: messages between the daemon and the UI thread.
//!
//! The UI runs its own Wayland event loop on a dedicated thread. The daemon
//! drives it through a `UiHandle` and receives `UiEvent`s back.

use crate::model::NotificationId;
use crate::theme::Theme;
use crate::view::ViewModel;

/// Panel position commands. Openness is 0.0 (hidden) ..= 1.0 (fully out).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelCommand {
    /// Finger tracking: jump to this openness this frame, no animation.
    Track(f32),
    /// Animate from the current openness to `target` (0.0 or 1.0).
    AnimateTo(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UiUpdate {
    View(Box<ViewModel>),
    Panel(PanelCommand),
    Theme(Box<Theme>),
    Shutdown,
}

/// User interactions reported by the UI. The daemon owns all state changes.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// Card body clicked: invoke "default" (if any) and dismiss.
    Activate(NotificationId),
    InvokeAction {
        id: NotificationId,
        key: String,
    },
    /// The user sent `text` from a card's inline reply field.
    Reply {
        id: NotificationId,
        text: String,
    },
    Dismiss(NotificationId),
    ClearGroup {
        key: String,
    },
    ToggleGroup {
        key: String,
    },
    ClearAll,
    ToggleDnd,
    ToastHover {
        id: NotificationId,
        hovered: bool,
    },
    /// The toast's expand/collapse button was clicked. Expanded toasts show
    /// the full card and do not expire; collapsing resumes the countdown with
    /// the time that remained. Carries the desired state (not a toggle) so a
    /// double click racing a view update cannot flip it back.
    SetToastExpanded {
        id: NotificationId,
        expanded: bool,
    },
    /// Outside click, Esc, or close button.
    RequestClose,
    /// The panel finished an animation; `open` is the resting state.
    PanelSettled {
        open: bool,
    },
}

/// Daemon-side handle to the UI thread.
pub trait UiHandle: Send {
    fn send(&self, update: UiUpdate);
}
