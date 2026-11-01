//! App actor tests with recording collaborators and a fake clock.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use tidings_core::clock::FakeClock;
use tidings_core::gesture::GestureKind;
use tidings_core::model::{Action, Urgency};
use tidings_core::theme::Rgb;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Sig {
    Action(u32, String),
    Closed(u32, CloseReason),
}

#[derive(Clone, Default)]
struct Rec {
    ui: Arc<Mutex<Vec<UiUpdate>>>,
    signals: Arc<Mutex<Vec<Sig>>>,
    panel_open: Arc<Mutex<Vec<bool>>>,
}

impl UiHandle for Rec {
    fn send(&self, update: UiUpdate) {
        self.ui.lock().unwrap().push(update);
    }
}

impl SignalSink for Rec {
    fn action_invoked(&self, id: NotificationId, key: &str) {
        self.signals
            .lock()
            .unwrap()
            .push(Sig::Action(id.0, key.to_owned()));
    }

    fn notification_closed(&self, id: NotificationId, reason: CloseReason) {
        self.signals.lock().unwrap().push(Sig::Closed(id.0, reason));
    }
}

impl PanelStateSink for Rec {
    fn set_panel_open(&self, open: bool) {
        self.panel_open.lock().unwrap().push(open);
    }
}

struct Harness {
    app: App,
    rec: Rec,
    clock: FakeClock,
}

impl Harness {
    fn new() -> Self {
        let rec = Rec::default();
        let clock = FakeClock::at_unix(1_800_000_000);
        let deps = AppDeps {
            clock: Arc::new(clock.clone()),
            ui: Box::new(rec.clone()),
            signals: Box::new(rec.clone()),
            panel_state: Box::new(rec.clone()),
        };
        let app = App::new(deps, ToastPolicy::default(), Theme::default());
        Self { app, rec, clock }
    }

    fn notify(&mut self, n: Notification) -> u32 {
        self.notify_with(n, 0, false)
    }

    fn notify_with(&mut self, n: Notification, replaces_id: u32, resident: bool) -> u32 {
        let (tx, mut rx) = oneshot::channel();
        let incoming = Incoming {
            replaces_id,
            notification: n,
            resident,
        };
        assert!(
            self.app
                .handle(AppEvent::Notify(Box::new(incoming), tx))
                .is_continue()
        );
        rx.try_recv().unwrap().0
    }

    fn ipc(&mut self, req: Request) -> Response {
        let (tx, mut rx) = oneshot::channel();
        assert!(self.app.handle(AppEvent::Ipc(req, tx)).is_continue());
        rx.try_recv().unwrap()
    }

    fn value(&mut self, req: Request) -> String {
        match self.ipc(req) {
            Response::Ok(Some(v)) => v,
            other => panic!("expected a value, got {other:?}"),
        }
    }

    fn ui(&mut self, ev: UiEvent) {
        assert!(self.app.handle(AppEvent::Ui(ev)).is_continue());
    }

    fn tick_after(&mut self, d: Duration) {
        self.clock.advance(d);
        assert!(self.app.handle(AppEvent::Tick).is_continue());
    }

    fn view(&self) -> ViewModel {
        self.rec
            .ui
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|u| match u {
                UiUpdate::View(v) => Some((**v).clone()),
                _ => None,
            })
            .unwrap()
    }

    fn toast_ids(&self) -> Vec<u32> {
        self.view().toasts.iter().map(|t| t.card.id.0).collect()
    }

    fn signals(&self) -> Vec<Sig> {
        std::mem::take(&mut *self.rec.signals.lock().unwrap())
    }

    fn take_ui(&self) -> Vec<UiUpdate> {
        std::mem::take(&mut *self.rec.ui.lock().unwrap())
    }

    fn panel_commands(&self) -> Vec<PanelCommand> {
        self.take_ui()
            .into_iter()
            .filter_map(|u| match u {
                UiUpdate::Panel(c) => Some(c),
                _ => None,
            })
            .collect()
    }
}

fn notif(app: &str) -> Notification {
    Notification {
        id: NotificationId(0),
        app_name: app.to_owned(),
        app_icon: None,
        image: None,
        summary: format!("from {app}"),
        body: String::new(),
        actions: Vec::new(),
        has_default_action: false,
        urgency: Urgency::Normal,
        expire_timeout_ms: None,
        desktop_entry: None,
        transient: false,
        received_at: SystemTime::UNIX_EPOCH,
    }
}

fn with(f: impl FnOnce(&mut Notification)) -> Notification {
    let mut n = notif("app");
    f(&mut n);
    n
}

#[test]
fn startup_sends_theme_and_empty_view() {
    let h = Harness::new();
    let ui = h.take_ui();
    assert!(matches!(ui[0], UiUpdate::Theme(_)));
    assert_eq!(ui[1], UiUpdate::View(Box::default()));
    assert_eq!(*h.rec.panel_open.lock().unwrap(), [false]);
}

#[test]
fn notify_assigns_ids_stamps_time_and_toasts() {
    let mut h = Harness::new();
    assert_eq!(h.notify(notif("a")), 1);
    h.clock.advance(Duration::from_secs(120));
    assert_eq!(h.notify(notif("b")), 2);
    let view = h.view();
    assert_eq!(view.groups.len(), 2);
    assert_eq!(view.groups[0].key, "b");
    assert_eq!(view.groups[1].cards[0].time_label, "2m");
    assert_eq!(h.toast_ids(), [2, 1]);
    assert_eq!(h.value(Request::Count), "2");
}

#[test]
fn replace_keeps_id_and_position_and_reshows_toast() {
    let mut h = Harness::new();
    let first = h.notify(notif("a"));
    h.notify(notif("b"));
    h.tick_after(Duration::from_secs(7)); // both toasts expire
    assert_eq!(h.toast_ids(), Vec::<u32>::new());
    let id = h.notify_with(with(|n| n.summary = "v2".into()), first, false);
    assert_eq!(id, first);
    let view = h.view();
    let keys: Vec<_> = view.groups.iter().map(|g| g.key.as_str()).collect();
    assert_eq!(
        keys,
        ["b", "app"],
        "replacement moved to app's group, b stays newest"
    );
    assert_eq!(h.toast_ids(), [first]);
    assert_eq!(view.toasts[0].card.summary, "v2");
    assert_eq!(h.value(Request::Count), "2");
    // Unknown replaces_id: a fresh id.
    assert_eq!(h.notify_with(notif("c"), 999, false), 3);
}

#[test]
fn toasts_expire_by_urgency_and_stay_in_history() {
    let mut h = Harness::new();
    let low = h.notify(with(|n| n.urgency = Urgency::Low));
    let normal = h.notify(notif("n"));
    let critical = h.notify(with(|n| n.urgency = Urgency::Critical));
    let custom = h.notify(with(|n| n.expire_timeout_ms = Some(1500)));
    let never = h.notify(with(|n| n.expire_timeout_ms = Some(0)));
    assert_eq!(h.app.next_wakeup(), Some(Duration::from_millis(1500)));
    h.tick_after(Duration::from_millis(1499));
    assert_eq!(h.toast_ids(), [never, custom, critical, normal]);
    h.tick_after(Duration::from_millis(1));
    assert_eq!(
        h.toast_ids(),
        [never, critical, normal],
        "custom timeout, and low pushed off"
    );
    let _ = low;
    h.tick_after(Duration::from_millis(4500));
    assert_eq!(h.toast_ids(), [never, critical]);
    h.tick_after(Duration::from_secs(3600));
    assert_eq!(h.toast_ids(), [never, critical]);
    assert_eq!(
        h.value(Request::Count),
        "5",
        "expired toasts stay in the panel"
    );
    assert_eq!(h.signals(), [], "nothing was closed");
}

#[test]
fn transient_notifications_are_dropped_when_their_toast_goes() {
    let mut h = Harness::new();
    let id = h.notify(with(|n| n.transient = true));
    assert_eq!(h.value(Request::Count), "1");
    h.tick_after(Duration::from_secs(6));
    assert_eq!(h.signals(), [Sig::Closed(id, CloseReason::Expired)]);
    assert_eq!(h.value(Request::Count), "0");
}

#[test]
fn max_four_toasts_and_overflowing_transients_are_closed() {
    let mut h = Harness::new();
    let transient = h.notify(with(|n| n.transient = true));
    for _ in 0..4 {
        h.notify(notif("x"));
    }
    assert_eq!(h.toast_ids(), [5, 4, 3, 2]);
    assert_eq!(h.signals(), [Sig::Closed(transient, CloseReason::Expired)]);
    assert_eq!(h.value(Request::Count), "4");
}

#[test]
fn dnd_suppresses_all_but_critical() {
    let mut h = Harness::new();
    assert_eq!(h.value(Request::Dnd(Some(DndChange::On))), "on");
    assert!(h.view().dnd);
    let quiet = h.notify(notif("a"));
    let loud = h.notify(with(|n| n.urgency = Urgency::Critical));
    let gone = h.notify(with(|n| n.transient = true));
    assert_eq!(h.toast_ids(), [loud]);
    assert_eq!(h.signals(), [Sig::Closed(gone, CloseReason::Expired)]);
    assert_eq!(h.value(Request::Count), "2", "suppressed but kept: {quiet}");
    assert_eq!(h.value(Request::Dnd(None)), "on");
    assert_eq!(h.value(Request::Dnd(Some(DndChange::Toggle))), "off");
    h.ui(UiEvent::ToggleDnd);
    assert_eq!(h.value(Request::Dnd(None)), "on");
    assert_eq!(h.value(Request::Dnd(Some(DndChange::Off))), "off");
    assert!(!h.view().dnd);
}

#[test]
fn hover_pauses_expiry() {
    let mut h = Harness::new();
    let id = h.notify(notif("a"));
    h.clock.advance(Duration::from_secs(5));
    h.ui(UiEvent::ToastHover {
        id: NotificationId(id),
        hovered: true,
    });
    assert!(h.view().toasts[0].paused);
    assert_eq!(
        h.app.next_wakeup(),
        Some(LABEL_REFRESH),
        "only the label refresh is pending"
    );
    h.tick_after(Duration::from_secs(60));
    assert_eq!(h.toast_ids(), [id]);
    h.ui(UiEvent::ToastHover {
        id: NotificationId(id),
        hovered: false,
    });
    h.tick_after(Duration::from_millis(999));
    assert_eq!(h.toast_ids(), [id]);
    h.tick_after(Duration::from_millis(1));
    assert_eq!(h.toast_ids(), Vec::<u32>::new());
}

#[test]
fn activate_invokes_default_then_dismisses() {
    let mut h = Harness::new();
    let with_default = h.notify(with(|n| n.has_default_action = true));
    let without = h.notify(notif("b"));
    h.ui(UiEvent::Activate(NotificationId(with_default)));
    h.ui(UiEvent::Activate(NotificationId(without)));
    h.ui(UiEvent::Activate(NotificationId(999)));
    assert_eq!(
        h.signals(),
        [
            Sig::Action(with_default, "default".into()),
            Sig::Closed(with_default, CloseReason::Dismissed),
            Sig::Closed(without, CloseReason::Dismissed),
        ]
    );
    assert_eq!(h.value(Request::Count), "0");
}

#[test]
fn resident_notifications_survive_actions() {
    let mut h = Harness::new();
    let n = with(|n| {
        n.has_default_action = true;
        n.actions = vec![Action {
            key: "pause".into(),
            label: "Pause".into(),
        }];
    });
    let id = h.notify_with(n, 0, true);
    h.ui(UiEvent::InvokeAction {
        id: NotificationId(id),
        key: "pause".into(),
    });
    h.ui(UiEvent::Activate(NotificationId(id)));
    assert_eq!(
        h.signals(),
        [
            Sig::Action(id, "pause".into()),
            Sig::Action(id, "default".into())
        ]
    );
    assert_eq!(h.value(Request::Count), "1");
    assert_eq!(
        h.toast_ids(),
        Vec::<u32>::new(),
        "toast is dismissed though"
    );
}

#[test]
fn actions_dismiss_and_clear() {
    let mut h = Harness::new();
    let a1 = h.notify(notif("a"));
    let b = h.notify(notif("b"));
    let a2 = h.notify(notif("a"));
    let c = h.notify(notif("c"));
    h.ui(UiEvent::InvokeAction {
        id: NotificationId(b),
        key: "reply".into(),
    });
    assert_eq!(
        h.signals(),
        [
            Sig::Action(b, "reply".into()),
            Sig::Closed(b, CloseReason::Dismissed)
        ]
    );
    h.ui(UiEvent::ToggleGroup { key: "a".into() });
    assert!(
        h.view()
            .groups
            .iter()
            .find(|g| g.key == "a")
            .unwrap()
            .expanded
    );
    h.ui(UiEvent::ClearGroup { key: "a".into() });
    assert_eq!(
        h.signals(),
        [
            Sig::Closed(a2, CloseReason::Dismissed),
            Sig::Closed(a1, CloseReason::Dismissed)
        ]
    );
    h.ui(UiEvent::Dismiss(NotificationId(c)));
    h.ui(UiEvent::Dismiss(NotificationId(c)));
    assert_eq!(h.signals(), [Sig::Closed(c, CloseReason::Dismissed)]);
    h.notify(notif("d"));
    h.notify(notif("e"));
    assert_eq!(h.ipc(Request::ClearAll), Response::Ok(None));
    assert_eq!(
        h.signals(),
        [
            Sig::Closed(6, CloseReason::Dismissed),
            Sig::Closed(5, CloseReason::Dismissed)
        ]
    );
    let view = h.view();
    assert_eq!((view.groups.len(), view.toasts.len()), (0, 0));
}

#[test]
fn close_notification_uses_reason_3() {
    let mut h = Harness::new();
    let id = h.notify(notif("a"));
    assert!(
        h.app
            .handle(AppEvent::Close(NotificationId(id)))
            .is_continue()
    );
    assert!(
        h.app
            .handle(AppEvent::Close(NotificationId(id)))
            .is_continue()
    );
    assert_eq!(h.signals(), [Sig::Closed(id, CloseReason::ClosedByCall)]);
}

#[test]
fn gestures_drive_the_panel_and_report_state() {
    let mut h = Harness::new();
    h.take_ui();
    let gesture = |h: &mut Harness, g| assert!(h.app.handle(AppEvent::Gesture(g)).is_continue());
    gesture(&mut h, GestureEvent::Begin(GestureKind::Open));
    gesture(&mut h, GestureEvent::Update { progress: 0.15 });
    assert_eq!(h.value(Request::IsOpen), "false");
    gesture(
        &mut h,
        GestureEvent::End {
            progress: 0.2,
            velocity: 0.0,
        },
    );
    assert_eq!(
        h.panel_commands(),
        [PanelCommand::Track(0.5), PanelCommand::AnimateTo(1.0)]
    );
    assert_eq!(h.value(Request::IsOpen), "true");
    assert_eq!(*h.rec.panel_open.lock().unwrap(), [false, true]);
    // An Open gesture while open is ignored; Close works.
    gesture(&mut h, GestureEvent::Begin(GestureKind::Open));
    gesture(&mut h, GestureEvent::Update { progress: 0.1 });
    assert_eq!(h.panel_commands(), []);
    gesture(&mut h, GestureEvent::Begin(GestureKind::Close));
    gesture(&mut h, GestureEvent::Update { progress: 0.03 });
    gesture(&mut h, GestureEvent::Cancel);
    let cmds = h.panel_commands();
    assert_eq!(
        cmds.last(),
        Some(&PanelCommand::AnimateTo(1.0)),
        "cancel restores open"
    );
    assert_eq!(h.value(Request::IsOpen), "true");
}

#[test]
fn ipc_and_ui_panel_commands() {
    let mut h = Harness::new();
    h.take_ui();
    assert_eq!(h.ipc(Request::Open), Response::Ok(None));
    assert_eq!(h.ipc(Request::Toggle), Response::Ok(None));
    assert_eq!(h.ipc(Request::Toggle), Response::Ok(None));
    h.ui(UiEvent::RequestClose);
    assert_eq!(h.ipc(Request::Close), Response::Ok(None));
    assert_eq!(
        h.panel_commands(),
        [1.0, 0.0, 1.0, 0.0, 0.0]
            .map(PanelCommand::AnimateTo)
            .to_vec()
    );
    assert_eq!(
        *h.rec.panel_open.lock().unwrap(),
        [false, true, false, true, false]
    );
    h.ipc(Request::Gesture(GestureEvent::Begin(GestureKind::Open)));
    h.ipc(Request::Gesture(GestureEvent::End {
        progress: 0.0,
        velocity: 1.0,
    }));
    assert_eq!(h.value(Request::IsOpen), "true");
    h.ui(UiEvent::PanelSettled { open: false });
    assert_eq!(h.value(Request::IsOpen), "false");
}

#[test]
fn unchanged_views_are_not_resent() {
    let mut h = Harness::new();
    h.notify(with(|n| n.expire_timeout_ms = Some(0)));
    h.take_ui();
    h.tick_after(Duration::from_secs(10));
    assert_eq!(h.take_ui(), [], "label still \"now\"");
    h.tick_after(Duration::from_secs(60));
    let ui = h.take_ui();
    assert_eq!(ui.len(), 1);
    let UiUpdate::View(v) = &ui[0] else {
        panic!("{ui:?}")
    };
    assert_eq!(v.groups[0].cards[0].time_label, "1m");
    h.ui(UiEvent::ToggleGroup { key: "nope".into() });
    assert_eq!(h.take_ui(), []);
}

#[test]
fn wakeups_cover_expiry_and_label_refresh() {
    let mut h = Harness::new();
    assert_eq!(h.app.next_wakeup(), None, "nothing stored, nothing to do");
    h.notify(with(|n| n.urgency = Urgency::Critical));
    assert_eq!(h.app.next_wakeup(), Some(LABEL_REFRESH));
    h.notify(with(|n| n.urgency = Urgency::Low));
    assert_eq!(h.app.next_wakeup(), Some(Duration::from_secs(4)));
}

#[test]
fn theme_and_shutdown_reach_the_ui() {
    let mut h = Harness::new();
    h.take_ui();
    let theme = Theme {
        primary: Rgb(1, 2, 3),
        ..Theme::default()
    };
    assert!(
        h.app
            .handle(AppEvent::Theme(Box::new(theme.clone())))
            .is_continue()
    );
    assert!(h.app.handle(AppEvent::Shutdown).is_break());
    assert_eq!(
        h.take_ui(),
        [UiUpdate::Theme(Box::new(theme)), UiUpdate::Shutdown]
    );
}

#[tokio::test]
async fn run_loop_expires_toasts_on_its_own_timer() {
    let rec = Rec::default();
    // Real time drives the actor's sleep; the fake clock only needs to agree.
    let clock = FakeClock::at_unix(0);
    let deps = AppDeps {
        clock: Arc::new(clock.clone()),
        ui: Box::new(rec.clone()),
        signals: Box::new(rec.clone()),
        panel_state: Box::new(rec.clone()),
    };
    let policy = ToastPolicy {
        normal_timeout: Duration::from_millis(20),
        ..ToastPolicy::default()
    };
    let app = App::new(deps, policy, Theme::default());
    let (tx, rx) = mpsc::unbounded_channel();
    let actor = tokio::spawn(app.run(rx));
    let (reply, id) = oneshot::channel();
    let incoming = Incoming {
        replaces_id: 0,
        notification: with(|n| n.transient = true),
        resident: false,
    };
    tx.send(AppEvent::Notify(Box::new(incoming), reply))
        .unwrap();
    let id = id.await.unwrap();
    clock.advance(Duration::from_millis(20));
    tokio::time::timeout(Duration::from_secs(5), async {
        while rec.signals.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        *rec.signals.lock().unwrap(),
        [Sig::Closed(id.0, CloseReason::Expired)]
    );
    drop(tx);
    actor.await.unwrap();
}
