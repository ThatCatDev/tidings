//! CONTRACT: render-ready snapshot the daemon hands to the UI.
//!
//! The UI never sees the store; it renders exactly what is in a `ViewModel`.
//! Strings are pre-formatted (relative times etc.) so the UI has no policy.

use crate::model::{Action, Image, NotificationId, Urgency};

#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub id: NotificationId,
    pub app_name: String,
    pub icon: Option<Image>,
    pub image: Option<Image>,
    pub summary: String,
    pub body: String,
    /// "now", "5m", "2h", "Yesterday", "Mon", "3 Nov".
    pub time_label: String,
    pub actions: Vec<Action>,
    pub clickable: bool,
    pub urgency: Urgency,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupView {
    /// Stable grouping key (desktop entry or app name).
    pub key: String,
    pub app_name: String,
    pub icon: Option<Image>,
    /// Newest first.
    pub cards: Vec<CardView>,
    /// Multi-card groups render as a stacked card until expanded.
    pub expanded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToastView {
    pub card: CardView,
    /// Expiry is paused while hovered; UI reports hover via `UiEvent`.
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ViewModel {
    /// Newest group first.
    pub groups: Vec<GroupView>,
    /// Newest first, already capped to the max visible count.
    pub toasts: Vec<ToastView>,
    pub dnd: bool,
}
