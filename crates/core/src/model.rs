//! CONTRACT: notification domain model.

use std::time::SystemTime;

/// Server-assigned notification id (`org.freedesktop.Notifications`, never 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NotificationId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// Decoded image for a notification or app icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Image {
    /// Freedesktop icon theme name, e.g. "firefox" or "dialog-information".
    IconName(String),
    /// Absolute path to an image file (png/svg/jpg).
    Path(std::path::PathBuf),
    /// Raw pixels from the `image-data` hint, converted to straight RGBA8.
    Rgba {
        width: u32,
        height: u32,
        pixels: std::sync::Arc<[u8]>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// The action key the client gave. "default" is the click action.
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: NotificationId,
    pub app_name: String,
    pub app_icon: Option<Image>,
    /// `image-data` / `image-path` hint, shown as the large thumbnail.
    pub image: Option<Image>,
    pub summary: String,
    /// Body with markup already reduced to plain text (links/bold dropped).
    pub body: String,
    /// Non-default actions in the order the client sent them.
    pub actions: Vec<Action>,
    /// Whether the client registered a "default" action.
    pub has_default_action: bool,
    pub urgency: Urgency,
    /// Client-requested timeout: `None` = server decides, `Some(0)` = never.
    pub expire_timeout_ms: Option<u32>,
    /// `desktop-entry` hint, used for grouping when present.
    pub desktop_entry: Option<String>,
    /// `transient` hint: never kept in the panel history.
    pub transient: bool,
    pub received_at: SystemTime,
}

/// Why a notification was closed (spec `NotificationClosed` reason codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    ClosedByCall = 3,
    Undefined = 4,
}
