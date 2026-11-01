//! The `org.freedesktop.Notifications` interface (spec 1.2) and signal emitter.
//!
//! Method handlers only translate arguments and forward them to the App
//! actor; all state lives there.

use std::collections::HashMap;
use std::hash::BuildHasher;

use tidings_core::model::{Action, CloseReason, Notification, NotificationId};
use tokio::sync::{mpsc, oneshot};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedVariant, fdo, interface};

use crate::app::{AppEvent, AppSender, Incoming, SignalSink};
use crate::dbus::hints::{image_from_str, parse_hints};
use crate::dbus::markup::strip_markup;

/// Well-known bus name.
pub const BUS_NAME: &str = "org.freedesktop.Notifications";
/// Object path the interface is served at.
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
/// Interface name.
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// Server capabilities (see DESIGN.md).
pub const CAPABILITIES: [&str; 6] = [
    "actions",
    "body",
    "body-hyperlinks",
    "body-markup",
    "icon-static",
    "persistence",
];

/// The D-Bus object. Holds only a sender to the App actor.
#[derive(Debug)]
pub struct NotificationServer {
    events: AppSender,
}

impl NotificationServer {
    /// A server forwarding calls to `events`.
    #[must_use]
    pub fn new(events: AppSender) -> Self {
        Self { events }
    }

    fn send(&self, event: AppEvent) -> fdo::Result<()> {
        self.events
            .send(event)
            .map_err(|_| fdo::Error::Failed("tidings is shutting down".into()))
    }
}

/// Builds the actor's view of a `Notify` call.
#[allow(clippy::too_many_arguments)] // mirrors the D-Bus method signature
#[must_use]
pub fn incoming<S: BuildHasher>(
    app_name: String,
    replaces_id: u32,
    app_icon: &str,
    summary: String,
    body: &str,
    actions: &[String],
    hints: &HashMap<String, OwnedVariant, S>,
    expire_timeout: i32,
) -> Incoming {
    let hints = parse_hints(hints);
    let mut has_default_action = false;
    let mut parsed = Vec::new();
    for pair in actions.as_chunks::<2>().0 {
        if pair[0] == "default" {
            has_default_action = true;
        } else {
            parsed.push(Action {
                key: pair[0].clone(),
                label: pair[1].clone(),
            });
        }
    }
    let notification = Notification {
        // Assigned by the actor.
        id: NotificationId(0),
        app_name,
        app_icon: image_from_str(app_icon).or(hints.icon_data),
        image: hints.image,
        summary,
        body: strip_markup(body),
        actions: parsed,
        has_default_action,
        urgency: hints.urgency,
        expire_timeout_ms: u32::try_from(expire_timeout).ok(),
        desktop_entry: hints.desktop_entry,
        transient: hints.transient,
        received_at: std::time::UNIX_EPOCH,
    };
    Incoming {
        replaces_id,
        notification,
        resident: hints.resident,
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    /// `GetCapabilities() -> as`
    #[allow(clippy::unused_self)] // zbus methods take a receiver
    fn get_capabilities(&self) -> Vec<String> {
        CAPABILITIES.iter().map(|&c| c.to_owned()).collect()
    }

    /// `Notify(susssasa{sv}i) -> u`
    #[allow(clippy::too_many_arguments)] // fixed by the spec
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> fdo::Result<u32> {
        let incoming = incoming(
            app_name,
            replaces_id,
            &app_icon,
            summary,
            &body,
            &actions,
            &hints,
            expire_timeout,
        );
        let (tx, rx) = oneshot::channel();
        self.send(AppEvent::Notify(Box::new(incoming), tx))?;
        let id = rx
            .await
            .map_err(|_| fdo::Error::Failed("tidings is shutting down".into()))?;
        Ok(id.0)
    }

    /// `CloseNotification(u)`
    fn close_notification(&self, id: u32) -> fdo::Result<()> {
        self.send(AppEvent::Close(NotificationId(id)))
    }

    /// `GetServerInformation() -> (ssss)`
    #[allow(clippy::unused_self)] // zbus methods take a receiver
    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "tidings".to_owned(),
            "tidings".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
            "1.2".to_owned(),
        )
    }
}

/// A signal to emit.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    ActionInvoked(u32, String),
    Closed(u32, u32),
}

/// [`SignalSink`] that queues signals to a task emitting them in order.
#[derive(Debug, Clone)]
pub struct DbusSignals {
    queue: mpsc::UnboundedSender<Signal>,
}

impl DbusSignals {
    /// Spawns the emitter task on the current tokio runtime.
    pub fn spawn(connection: &Connection) -> zbus::Result<Self> {
        let emitter = SignalEmitter::new(connection, OBJECT_PATH)?.into_owned();
        let (queue, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(signal) = rx.recv().await {
                let result = match &signal {
                    Signal::ActionInvoked(id, key) => {
                        emitter
                            .emit(INTERFACE, "ActionInvoked", &(*id, key.as_str()))
                            .await
                    }
                    Signal::Closed(id, reason) => {
                        emitter
                            .emit(INTERFACE, "NotificationClosed", &(*id, *reason))
                            .await
                    }
                };
                if let Err(e) = result {
                    tracing::warn!("failed to emit {signal:?}: {e}");
                }
            }
        });
        Ok(Self { queue })
    }

    fn push(&self, signal: Signal) {
        // Only fails once the emitter task is gone, i.e. during shutdown.
        let _ = self.queue.send(signal);
    }
}

impl SignalSink for DbusSignals {
    fn action_invoked(&self, id: NotificationId, key: &str) {
        self.push(Signal::ActionInvoked(id.0, key.to_owned()));
    }

    fn notification_closed(&self, id: NotificationId, reason: CloseReason) {
        self.push(Signal::Closed(id.0, reason as u32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbus::hints::tests::image_variant;
    use tidings_core::model::{Image, Urgency};
    use zbus::Variant;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn notify_arguments_are_translated() {
        let hints: HashMap<String, OwnedVariant> = [
            ("urgency", Variant::U8(2)),
            ("desktop-entry", Variant::from("firefox")),
            ("resident", Variant::Bool(true)),
            ("transient", Variant::Bool(true)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), OwnedVariant::try_from(v).unwrap()))
        .collect();
        let inc = incoming(
            "Firefox".into(),
            7,
            "firefox",
            "Download done".into(),
            "<b>file.zip</b> &amp; more",
            &strings(&["default", "Open", "show", "Show in folder", "dangling"]),
            &hints,
            -1,
        );
        assert_eq!(inc.replaces_id, 7);
        assert!(inc.resident);
        let n = inc.notification;
        assert_eq!(n.app_name, "Firefox");
        assert_eq!(n.app_icon, Some(Image::IconName("firefox".into())));
        assert_eq!(n.body, "file.zip & more");
        assert!(n.has_default_action);
        assert_eq!(
            n.actions,
            vec![Action {
                key: "show".into(),
                label: "Show in folder".into()
            }]
        );
        assert_eq!(n.urgency, Urgency::Critical);
        assert_eq!(n.expire_timeout_ms, None);
        assert_eq!(n.desktop_entry.as_deref(), Some("firefox"));
        assert!(n.transient);
    }

    #[test]
    fn timeouts_and_icon_fallback() {
        let hints: HashMap<String, OwnedVariant> = [(
            "icon_data".to_owned(),
            OwnedVariant::try_from(image_variant(1, 1, 3, false, vec![1, 2, 3])).unwrap(),
        )]
        .into_iter()
        .collect();
        let inc = incoming("a".into(), 0, "", "s".into(), "", &[], &hints, 0);
        assert_eq!(inc.notification.expire_timeout_ms, Some(0));
        assert!(matches!(
            inc.notification.app_icon,
            Some(Image::Rgba { .. })
        ));
        assert!(!inc.notification.has_default_action);
        let inc = incoming(
            "a".into(),
            0,
            "",
            "s".into(),
            "",
            &[],
            &HashMap::new(),
            2500,
        );
        assert_eq!(inc.notification.expire_timeout_ms, Some(2500));
        assert_eq!(inc.notification.app_icon, None);
        assert_eq!(inc.notification.urgency, Urgency::Normal);
    }
}
