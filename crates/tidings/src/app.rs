//! The App actor: the single owner of all daemon state.
//!
//! Every input — D-Bus calls, gestures, UI interactions, IPC requests,
//! timers, theme changes — arrives as an [`AppEvent`] on one channel and is
//! handled to completion before the next, so there is no shared mutable
//! state. Outputs go through the [`UiHandle`], [`SignalSink`] and
//! [`PanelStateSink`] seams, which tests replace with recorders.

use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use tidings_core::clock::Clock;
use tidings_core::gesture::GestureEvent;
use tidings_core::model::{CloseReason, Notification, NotificationId};
use tidings_core::panel::Panel;
use tidings_core::policy::ToastPolicy;
use tidings_core::store::Store;
use tidings_core::theme::Theme;
use tidings_core::toasts::Toasts;
use tidings_core::ui::{PanelCommand, UiEvent, UiHandle, UiUpdate};
use tidings_core::view::ViewModel;
use tidings_core::view_builder::build_view;
use tokio::sync::{mpsc, oneshot};

use crate::ipc::proto::{DndChange, Request, Response};

/// How often relative time labels are refreshed while anything is stored.
const LABEL_REFRESH: Duration = Duration::from_secs(30);

/// Sender half of the actor's channel. Unbounded so producers on plain
/// threads (gesture reader, UI) never block; the actor drains it quickly.
pub type AppSender = mpsc::UnboundedSender<AppEvent>;

/// A `Notify` call, already parsed. `notification.id` and `received_at` are
/// placeholders; the actor assigns both.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    /// `replaces_id` argument; 0 for a new notification.
    pub replaces_id: u32,
    /// The parsed notification.
    pub notification: Notification,
    /// `resident` hint.
    pub resident: bool,
}

/// Everything the actor reacts to.
#[derive(Debug)]
pub enum AppEvent {
    /// D-Bus `Notify`; replies with the assigned id.
    Notify(Box<Incoming>, oneshot::Sender<NotificationId>),
    /// D-Bus `CloseNotification`.
    Close(NotificationId),
    /// From the trackpad reader (or `ctl gesture`).
    Gesture(GestureEvent),
    /// An interaction reported by the UI.
    Ui(UiEvent),
    /// An IPC request; replies with the response.
    Ipc(Request, oneshot::Sender<Response>),
    /// The colours file changed.
    Theme(Box<Theme>),
    /// Timer wake-up: expire toasts and refresh time labels.
    Tick,
    /// Stop: tell the UI to shut down and end the actor.
    Shutdown,
}

/// Emits the D-Bus signals.
pub trait SignalSink: Send {
    /// `ActionInvoked(id, key)`.
    fn action_invoked(&self, id: NotificationId, key: &str);
    /// `NotificationClosed(id, reason)`.
    fn notification_closed(&self, id: NotificationId, reason: CloseReason);
}

/// Receives the panel's open state (the gesture recogniser needs it).
pub trait PanelStateSink: Send {
    /// The panel is now (heading) open or closed.
    fn set_panel_open(&self, open: bool);
}

/// The actor's collaborators.
pub struct AppDeps {
    /// Time source.
    pub clock: Arc<dyn Clock>,
    /// The UI thread.
    pub ui: Box<dyn UiHandle>,
    /// D-Bus signal emitter.
    pub signals: Box<dyn SignalSink>,
    /// The gesture recogniser's panel-state input.
    pub panel_state: Box<dyn PanelStateSink>,
}

impl fmt::Debug for AppDeps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppDeps")
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

/// All daemon state.
pub struct App {
    store: Store,
    toasts: Toasts,
    panel: Panel,
    policy: ToastPolicy,
    dnd: bool,
    deps: AppDeps,
    last_view: Option<ViewModel>,
    dirty: bool,
    refresh_at: Option<Duration>,
    reported_open: bool,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("stored", &self.store.len())
            .field("toasts", &self.toasts.len())
            .field("panel", &self.panel)
            .field("dnd", &self.dnd)
            .finish_non_exhaustive()
    }
}

impl App {
    /// Creates the actor and pushes the initial theme and (empty) view to the UI.
    #[must_use]
    pub fn new(deps: AppDeps, policy: ToastPolicy, theme: Theme) -> Self {
        let mut app = Self {
            store: Store::default(),
            toasts: Toasts::new(policy.max_visible),
            panel: Panel::default(),
            policy,
            dnd: false,
            deps,
            last_view: None,
            dirty: true,
            refresh_at: None,
            reported_open: false,
        };
        app.deps.ui.send(UiUpdate::Theme(Box::new(theme)));
        app.deps.panel_state.set_panel_open(false);
        app.flush();
        app
    }

    /// Runs the actor until `Shutdown` or until every sender is gone.
    pub async fn run(mut self, mut events: mpsc::UnboundedReceiver<AppEvent>) {
        loop {
            let wait = self
                .next_wakeup()
                .map(|at| at.saturating_sub(self.deps.clock.monotonic()));
            let event = tokio::select! {
                ev = events.recv() => ev,
                () = sleep_or_forever(wait) => Some(AppEvent::Tick),
            };
            let Some(event) = event else { break };
            if self.handle(event).is_break() {
                break;
            }
        }
    }

    /// Monotonic time of the next timer wake-up, if one is needed.
    #[must_use]
    pub fn next_wakeup(&self) -> Option<Duration> {
        let refresh = if self.store.is_empty() {
            None
        } else {
            self.refresh_at
        };
        match (self.toasts.next_deadline(), refresh) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Handles one event, then pushes any resulting view change to the UI.
    pub fn handle(&mut self, event: AppEvent) -> ControlFlow<()> {
        match event {
            AppEvent::Notify(incoming, reply) => {
                let id = self.notify(*incoming);
                // The caller may have gone away; nothing to do then.
                let _ = reply.send(id);
            }
            AppEvent::Close(id) => self.remove(id, CloseReason::ClosedByCall),
            AppEvent::Gesture(g) => self.gesture(g),
            AppEvent::Ui(ev) => self.ui_event(ev),
            AppEvent::Ipc(req, reply) => {
                let response = self.ipc(req);
                let _ = reply.send(response);
            }
            AppEvent::Theme(theme) => self.deps.ui.send(UiUpdate::Theme(theme)),
            AppEvent::Tick => self.tick(),
            AppEvent::Shutdown => {
                self.deps.ui.send(UiUpdate::Shutdown);
                return ControlFlow::Break(());
            }
        }
        self.flush();
        ControlFlow::Continue(())
    }

    fn notify(&mut self, incoming: Incoming) -> NotificationId {
        let Incoming {
            replaces_id,
            mut notification,
            resident,
        } = incoming;
        let replaces = NotificationId(replaces_id);
        let id = if replaces_id != 0 && self.store.contains(replaces) {
            replaces
        } else {
            self.store.allocate_id()
        };
        notification.id = id;
        notification.received_at = self.deps.clock.now();
        let toast = self.policy.should_toast(&notification, self.dnd);
        tracing::debug!(id = id.0, app = %notification.app_name, toast, "notify");
        self.dirty = true;

        if !toast && !self.policy.keeps_history(&notification) {
            // Transient and suppressed: never shown, never kept.
            self.store.remove(id);
            self.toasts.remove(id);
            self.deps
                .signals
                .notification_closed(id, CloseReason::Expired);
            return id;
        }
        let timeout = self.policy.timeout(&notification);
        let outcome = self.store.insert(notification, resident);
        for evicted in outcome.evicted {
            self.toasts.remove(evicted);
            self.deps
                .signals
                .notification_closed(evicted, CloseReason::Undefined);
        }
        if toast {
            let pushed_off = self.toasts.show(id, timeout, self.deps.clock.monotonic());
            for gone in pushed_off {
                self.toast_gone(gone);
            }
        } else {
            self.toasts.remove(id);
        }
        id
    }

    /// A toast left the screen without user action: transient ones are dropped.
    fn toast_gone(&mut self, id: NotificationId) {
        let transient = self
            .store
            .get(id)
            .is_some_and(|e| !self.policy.keeps_history(&e.notification));
        if transient {
            self.remove(id, CloseReason::Expired);
        }
        self.dirty = true;
    }

    fn remove(&mut self, id: NotificationId, reason: CloseReason) {
        self.toasts.remove(id);
        if self.store.remove(id).is_some() {
            self.deps.signals.notification_closed(id, reason);
            self.dirty = true;
        }
    }

    fn remove_all(&mut self, ids: Vec<NotificationId>) {
        for id in ids {
            self.toasts.remove(id);
            self.deps
                .signals
                .notification_closed(id, CloseReason::Dismissed);
        }
        self.dirty = true;
    }

    /// Invokes an action; non-resident notifications are then dismissed.
    fn invoke(&mut self, id: NotificationId, key: &str) {
        let Some(entry) = self.store.get(id) else {
            return;
        };
        let resident = entry.resident;
        self.deps.signals.action_invoked(id, key);
        if resident {
            self.toasts.remove(id);
            self.dirty = true;
        } else {
            self.remove(id, CloseReason::Dismissed);
        }
    }

    fn panel(&mut self, command: PanelCommand) {
        self.deps.ui.send(UiUpdate::Panel(command));
    }

    fn gesture(&mut self, g: GestureEvent) {
        if let Some(command) = self.panel.on_gesture(g) {
            self.panel(command);
        }
    }

    fn ui_event(&mut self, ev: UiEvent) {
        match ev {
            UiEvent::Activate(id) => {
                let Some(entry) = self.store.get(id) else {
                    return;
                };
                if entry.notification.has_default_action {
                    self.invoke(id, "default");
                } else {
                    self.remove(id, CloseReason::Dismissed);
                }
            }
            UiEvent::InvokeAction { id, key } => self.invoke(id, &key),
            UiEvent::Dismiss(id) => self.remove(id, CloseReason::Dismissed),
            UiEvent::ClearGroup { key } => {
                let ids = self.store.clear_group(&key);
                self.remove_all(ids);
            }
            UiEvent::ToggleGroup { key } => self.dirty |= self.store.toggle_group(&key),
            UiEvent::ClearAll => {
                let ids = self.store.clear_all();
                self.remove_all(ids);
            }
            UiEvent::ToggleDnd => self.set_dnd(!self.dnd),
            UiEvent::ToastHover { id, hovered } => {
                let now = self.deps.clock.monotonic();
                self.dirty |= self.toasts.set_hovered(id, hovered, now);
            }
            UiEvent::SetToastExpanded { id, expanded } => {
                let now = self.deps.clock.monotonic();
                self.dirty |= self.toasts.set_expanded(id, expanded, now);
            }
            UiEvent::RequestClose => {
                let command = self.panel.set_open(false);
                self.panel(command);
            }
            UiEvent::PanelSettled { open } => self.panel.settled(open),
        }
    }

    fn set_dnd(&mut self, on: bool) {
        self.dnd = on;
        self.dirty = true;
    }

    fn ipc(&mut self, req: Request) -> Response {
        let command = match req {
            Request::Toggle => Some(self.panel.toggle()),
            Request::Open => Some(self.panel.set_open(true)),
            Request::Close => Some(self.panel.set_open(false)),
            Request::ClearAll => {
                self.ui_event(UiEvent::ClearAll);
                None
            }
            Request::Dnd(change) => {
                match change {
                    Some(DndChange::On) => self.set_dnd(true),
                    Some(DndChange::Off) => self.set_dnd(false),
                    Some(DndChange::Toggle) => self.set_dnd(!self.dnd),
                    None => {}
                }
                return Response::Ok(Some(if self.dnd { "on" } else { "off" }.to_owned()));
            }
            Request::Count => return Response::Ok(Some(self.store.len().to_string())),
            Request::IsOpen => return Response::Ok(Some(self.panel.is_open().to_string())),
            Request::Gesture(g) => {
                self.gesture(g);
                None
            }
        };
        if let Some(command) = command {
            self.panel(command);
        }
        Response::Ok(None)
    }

    fn tick(&mut self) {
        let now = self.deps.clock.monotonic();
        for id in self.toasts.expire(now) {
            self.toast_gone(id);
        }
        // Time labels may have changed; `flush` only sends if they did.
        self.dirty = true;
        self.refresh_at = Some(now + LABEL_REFRESH);
    }

    /// Rebuilds the view if state changed and sends it if it differs from
    /// the last one; reports panel state changes to the recogniser.
    fn flush(&mut self) {
        if self.dirty {
            self.dirty = false;
            let view = build_view(
                &self.store,
                &self.toasts,
                self.dnd,
                self.policy.max_visible,
                self.deps.clock.as_ref(),
            );
            if self.last_view.as_ref() != Some(&view) {
                self.deps.ui.send(UiUpdate::View(Box::new(view.clone())));
                self.last_view = Some(view);
            }
            if self.refresh_at.is_none() {
                self.refresh_at = Some(self.deps.clock.monotonic() + LABEL_REFRESH);
            }
        }
        let open = self.panel.is_open();
        if open != self.reported_open {
            self.reported_open = open;
            self.deps.panel_state.set_panel_open(open);
        }
    }
}

async fn sleep_or_forever(wait: Option<Duration>) {
    match wait {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
