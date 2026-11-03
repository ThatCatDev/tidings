//! End-to-end tests of the daemon core on a private D-Bus, with a `NullUi`
//! and a fake clock.

mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::{TestDaemon, config, next, simple_notify, start_daemon};
use tidings::app::AppEvent;
use tidings::daemon::DaemonError;
use tidings::ipc::proto::{DndChange, Request, Response};
use tidings_core::clock::FakeClock;
use tidings_core::gesture::{GestureEvent, GestureKind};
use tidings_core::model::{Image, NotificationId};
use tidings_core::theme::Rgb;
use tidings_core::ui::{PanelCommand, UiEvent, UiUpdate};
use tidings_core::view::ViewModel;
use zbus::{OwnedVariant, Variant};

fn view(update: &UiUpdate) -> Option<ViewModel> {
    match update {
        UiUpdate::View(v) => Some((**v).clone()),
        _ => None,
    }
}

#[tokio::test]
async fn capabilities_and_server_information() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    let caps = proxy.get_capabilities().await.unwrap();
    for cap in [
        "actions",
        "body",
        "body-hyperlinks",
        "body-markup",
        "icon-static",
        "inline-reply",
        "persistence",
        "x-kde-origin-name",
    ] {
        assert!(caps.iter().any(|c| c == cap), "missing {cap}: {caps:?}");
    }
    let (name, vendor, version, spec) = proxy.get_server_information().await.unwrap();
    assert_eq!(
        (name.as_str(), vendor.as_str(), spec.as_str()),
        ("tidings", "tidings", "1.2")
    );
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    t.shutdown().await;
}

#[tokio::test]
async fn notify_replace_and_close() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    let mut closed = proxy.receive_notification_closed().await.unwrap();

    let id = proxy
        .notify(
            "Firefox",
            0,
            "firefox",
            "Hello",
            "<b>bold</b> &amp; plain",
            &[],
            HashMap::new(),
            -1,
        )
        .await
        .unwrap();
    assert_eq!(id, 1);
    let v = t
        .wait_ui(|u| view(u).filter(|v| !v.toasts.is_empty()))
        .await;
    assert_eq!(v.toasts[0].card.body, "bold & plain");
    assert_eq!(
        v.toasts[0].card.icon,
        Some(Image::IconName("firefox".into()))
    );

    let other = simple_notify(&proxy, "Slack", "ping").await;
    assert_eq!(other, 2);
    let replaced = proxy
        .notify(
            "Firefox",
            id,
            "",
            "Hello again",
            "",
            &[],
            HashMap::new(),
            -1,
        )
        .await
        .unwrap();
    assert_eq!(replaced, id, "replaces in place");
    let v = t
        .wait_ui(|u| {
            view(u).filter(|v| {
                v.toasts
                    .first()
                    .is_some_and(|t| t.card.summary == "Hello again")
            })
        })
        .await;
    let keys: Vec<_> = v.groups.iter().map(|g| g.key.as_str()).collect();
    assert_eq!(keys, ["Slack", "Firefox"], "the replaced card stays put");
    assert_eq!(t.value(Request::Count).await, "2");

    proxy.close_notification(id).await.unwrap();
    let sig = next(&mut closed).await;
    let args = sig.args().unwrap();
    assert_eq!((args.id, args.reason), (id, 3));
    assert_eq!(t.value(Request::Count).await, "1");
    // Closing an unknown id is not an error.
    proxy.close_notification(424_242).await.unwrap();
    t.shutdown().await;
}

#[tokio::test]
async fn action_invoked_and_dismissed_signals() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    let mut actions = proxy.receive_action_invoked().await.unwrap();
    let mut closed = proxy.receive_notification_closed().await.unwrap();
    let id = proxy
        .notify(
            "app",
            0,
            "",
            "s",
            "",
            &["default", "Open", "later", "Later"],
            HashMap::new(),
            0,
        )
        .await
        .unwrap();
    let card = t
        .wait_ui(|u| view(u).and_then(|v| v.toasts.first().map(|t| t.card.clone())))
        .await;
    assert!(card.clickable);
    assert_eq!(card.actions.len(), 1);
    assert_eq!(card.actions[0].key, "later");

    // The UI reports a click.
    t.daemon()
        .events()
        .send(AppEvent::Ui(UiEvent::Activate(NotificationId(id))))
        .unwrap();
    let sig = next(&mut actions).await;
    let args = sig.args().unwrap();
    assert_eq!((args.id, args.action_key.as_str()), (id, "default"));
    let sig = next(&mut closed).await;
    let args = sig.args().unwrap();
    assert_eq!((args.id, args.reason), (id, 2));

    // A resident notification keeps living after its action.
    let mut hints = HashMap::new();
    hints.insert(
        "resident",
        OwnedVariant::try_from(Variant::Bool(true)).unwrap(),
    );
    let resident = proxy
        .notify("app", 0, "", "r", "", &["stop", "Stop"], hints, 0)
        .await
        .unwrap();
    t.daemon()
        .events()
        .send(AppEvent::Ui(UiEvent::InvokeAction {
            id: NotificationId(resident),
            key: "stop".into(),
        }))
        .unwrap();
    let sig = next(&mut actions).await;
    let args = sig.args().unwrap();
    assert_eq!((args.id, args.action_key.as_str()), (resident, "stop"));
    assert_eq!(t.value(Request::Count).await, "1");
    t.shutdown().await;
}

#[tokio::test]
async fn inline_reply_is_signalled() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    let mut replied = proxy.receive_notification_replied().await.unwrap();
    let mut closed = proxy.receive_notification_closed().await.unwrap();
    let mut hints = HashMap::new();
    hints.insert(
        "x-kde-reply-placeholder-text",
        OwnedVariant::try_from(Variant::from("Message")).unwrap(),
    );
    let id = proxy
        .notify(
            "chat",
            0,
            "",
            "Ada",
            "6?",
            &["inline-reply", "Reply"],
            hints,
            0,
        )
        .await
        .unwrap();
    let card = t
        .wait_ui(|u| view(u).and_then(|v| v.toasts.first().map(|t| t.card.clone())))
        .await;
    assert_eq!(card.actions.len(), 0);
    assert_eq!(
        card.reply.and_then(|r| r.placeholder).as_deref(),
        Some("Message")
    );

    t.daemon()
        .events()
        .send(AppEvent::Ui(UiEvent::Reply {
            id: NotificationId(id),
            text: "yes 👍".into(),
        }))
        .unwrap();
    let sig = next(&mut replied).await;
    let args = sig.args().unwrap();
    assert_eq!((args.id, args.text.as_str()), (id, "yes 👍"));
    let sig = next(&mut closed).await;
    assert_eq!(sig.args().unwrap().reason, 2);
    t.shutdown().await;
}

#[tokio::test]
async fn toast_expiry_follows_the_fake_clock() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    let mut closed = proxy.receive_notification_closed().await.unwrap();
    let mut hints = HashMap::new();
    hints.insert(
        "transient",
        OwnedVariant::try_from(Variant::Bool(true)).unwrap(),
    );
    let transient = proxy
        .notify("a", 0, "", "gone soon", "", &[], hints, -1)
        .await
        .unwrap();
    let kept = simple_notify(&proxy, "b", "stays").await;
    t.wait_ui(|u| view(u).filter(|v| v.toasts.len() == 2)).await;

    t.clock.advance(Duration::from_millis(5999));
    t.daemon().events().send(AppEvent::Tick).unwrap();
    assert_eq!(t.value(Request::Count).await, "2", "not yet");
    t.clock.advance(Duration::from_millis(1));
    t.daemon().events().send(AppEvent::Tick).unwrap();

    let sig = next(&mut closed).await;
    let args = sig.args().unwrap();
    assert_eq!(
        (args.id, args.reason),
        (transient, 1),
        "transient expires for good"
    );
    let v = t.wait_ui(|u| view(u).filter(|v| v.toasts.is_empty())).await;
    let ids: Vec<_> = v
        .groups
        .iter()
        .flat_map(|g| g.cards.iter().map(|c| c.id.0))
        .collect();
    assert_eq!(ids, [kept], "expired toast stays in the panel");
    t.shutdown().await;
}

#[tokio::test]
async fn expanded_toast_suspends_expiry() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    let id = simple_notify(&proxy, "a", "read me").await;
    t.wait_ui(|u| view(u).filter(|v| v.toasts.len() == 1)).await;

    t.clock.advance(Duration::from_secs(1));
    let expand = |expanded| {
        AppEvent::Ui(UiEvent::SetToastExpanded {
            id: NotificationId(id),
            expanded,
        })
    };
    t.daemon().events().send(expand(true)).unwrap();
    t.wait_ui(|u| view(u).filter(|v| v.toasts.first().is_some_and(|t| t.expanded)))
        .await;

    // Long past the 6 s expiry: still toasting.
    t.clock.advance(Duration::from_secs(60));
    t.daemon().events().send(AppEvent::Tick).unwrap();
    t.daemon().events().send(expand(false)).unwrap();
    let v = t
        .wait_ui(|u| view(u).filter(|v| v.toasts.first().is_some_and(|t| !t.expanded)))
        .await;
    assert_eq!(v.toasts[0].card.id, NotificationId(id));

    // Collapsed: the 5 s that remained run out.
    t.clock.advance(Duration::from_secs(5));
    t.daemon().events().send(AppEvent::Tick).unwrap();
    t.wait_ui(|u| view(u).filter(|v| v.toasts.is_empty())).await;
    assert_eq!(t.value(Request::Count).await, "1", "kept in the panel");
    t.shutdown().await;
}

#[tokio::test]
async fn image_data_hint_round_trips_as_rgba() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    // 2x1 RGB with 2 bytes of row padding.
    let image = Variant::from((
        2i32,
        1i32,
        8i32,
        false,
        8i32,
        3i32,
        vec![1u8, 2, 3, 4, 5, 6, 0, 0],
    ));
    let mut hints = HashMap::new();
    hints.insert("image-data", OwnedVariant::try_from(image).unwrap());
    hints.insert("urgency", OwnedVariant::try_from(Variant::U8(2)).unwrap());
    hints.insert(
        "desktop-entry",
        OwnedVariant::try_from(Variant::from("org.example.App")).unwrap(),
    );
    proxy
        .notify("Example", 0, "", "pic", "", &[], hints, -1)
        .await
        .unwrap();
    let v = t
        .wait_ui(|u| view(u).filter(|v| !v.groups.is_empty()))
        .await;
    assert_eq!(v.groups[0].key, "org.example.App");
    let card = &v.groups[0].cards[0];
    assert_eq!(card.urgency, tidings_core::model::Urgency::Critical);
    assert_eq!(
        card.image,
        Some(Image::Rgba {
            width: 2,
            height: 1,
            pixels: vec![1, 2, 3, 255, 4, 5, 6, 255].into()
        })
    );
    t.shutdown().await;
}

#[tokio::test]
async fn ipc_count_clear_all_and_dnd() {
    let t = TestDaemon::start().await;
    let proxy = t.proxy().await;
    let mut closed = proxy.receive_notification_closed().await.unwrap();
    assert_eq!(t.value(Request::Count).await, "0");
    let a = simple_notify(&proxy, "a", "1").await;
    let b = simple_notify(&proxy, "b", "2").await;
    assert_eq!(t.value(Request::Count).await, "2");
    assert_eq!(t.ctl(Request::ClearAll).await, Response::Ok(None));
    let mut got = vec![];
    for _ in 0..2 {
        let sig = next(&mut closed).await;
        let args = sig.args().unwrap();
        got.push((args.id, args.reason));
    }
    got.sort_unstable();
    assert_eq!(got, [(a, 2), (b, 2)]);
    assert_eq!(t.value(Request::Count).await, "0");

    assert_eq!(t.value(Request::Dnd(None)).await, "off");
    assert_eq!(t.value(Request::Dnd(Some(DndChange::On))).await, "on");
    simple_notify(&proxy, "c", "quiet").await;
    let v = t
        .wait_ui(|u| view(u).filter(|v| v.dnd && !v.groups.is_empty()))
        .await;
    assert!(v.toasts.is_empty(), "DND suppresses the toast");
    assert_eq!(t.value(Request::Dnd(Some(DndChange::Toggle))).await, "off");
    t.shutdown().await;
}

#[tokio::test]
async fn ipc_panel_and_gesture_flow() {
    let t = TestDaemon::start().await;
    let panel = |u: &UiUpdate| match u {
        UiUpdate::Panel(c) => Some(*c),
        _ => None,
    };
    assert_eq!(t.value(Request::IsOpen).await, "false");
    t.ctl(Request::Gesture(GestureEvent::Begin(GestureKind::Open)))
        .await;
    t.ctl(Request::Gesture(GestureEvent::Update { progress: 0.15 }))
        .await;
    assert_eq!(t.wait_ui(panel).await, PanelCommand::Track(0.5));
    assert_eq!(
        t.value(Request::IsOpen).await,
        "false",
        "not open until released"
    );
    t.ctl(Request::Gesture(GestureEvent::End {
        progress: 0.1,
        velocity: 1.0,
    }))
    .await;
    assert_eq!(
        t.wait_ui(panel).await,
        PanelCommand::AnimateTo(1.0),
        "flick opens"
    );
    assert_eq!(t.value(Request::IsOpen).await, "true");

    t.ctl(Request::Gesture(GestureEvent::Begin(GestureKind::Close)))
        .await;
    t.ctl(Request::Gesture(GestureEvent::Update { progress: 0.03 }))
        .await;
    t.ctl(Request::Gesture(GestureEvent::End {
        progress: 0.03,
        velocity: 0.0,
    }))
    .await;
    assert_eq!(t.wait_ui(panel).await, PanelCommand::Track(0.9));
    assert_eq!(
        t.wait_ui(panel).await,
        PanelCommand::AnimateTo(1.0),
        "springs back open"
    );
    assert_eq!(t.value(Request::IsOpen).await, "true");

    assert_eq!(t.ctl(Request::Toggle).await, Response::Ok(None));
    assert_eq!(t.wait_ui(panel).await, PanelCommand::AnimateTo(0.0));
    assert_eq!(t.value(Request::IsOpen).await, "false");
    t.ctl(Request::Open).await;
    t.ctl(Request::Close).await;
    assert_eq!(t.wait_ui(panel).await, PanelCommand::AnimateTo(1.0));
    assert_eq!(t.wait_ui(panel).await, PanelCommand::AnimateTo(0.0));
    t.shutdown().await;
}

#[tokio::test]
async fn ipc_rejects_malformed_requests() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let t = TestDaemon::start().await;
    let stream = tokio::net::UnixStream::connect(&t.socket).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"frobnicate\ncount\n").await.unwrap();
    assert!(
        lines
            .next_line()
            .await
            .unwrap()
            .unwrap()
            .starts_with("err ")
    );
    assert_eq!(
        lines.next_line().await.unwrap().unwrap(),
        "ok 0",
        "connection stays usable"
    );
    t.shutdown().await;
}

#[tokio::test]
async fn theme_file_changes_reach_the_ui() {
    let t = TestDaemon::start().await;
    let initial = t
        .wait_ui(|u| match u {
            UiUpdate::Theme(theme) => Some((**theme).clone()),
            _ => None,
        })
        .await;
    assert_eq!(
        initial,
        tidings_core::theme::Theme::default(),
        "missing file → defaults"
    );
    std::fs::write(&t.colors, r##"{"mode":"dark","primary":"#123456"}"##).unwrap();
    let theme = t
        .wait_ui(|u| match u {
            UiUpdate::Theme(theme) => Some((**theme).clone()),
            _ => None,
        })
        .await;
    assert_eq!(theme.primary, Rgb(0x12, 0x34, 0x56));
    t.shutdown().await;
}

#[tokio::test]
async fn second_daemon_refuses_and_first_keeps_the_name() {
    let t = TestDaemon::start().await;
    let other_dir = tempfile::tempdir().unwrap();
    let result = start_daemon(config(&t.bus, other_dir.path()), &FakeClock::at_unix(0)).await;
    assert!(matches!(result, Err(DaemonError::NameTaken)), "{result:?}");
    // The original still answers.
    let proxy = t.proxy().await;
    assert_eq!(simple_notify(&proxy, "a", "still here").await, 1);
    t.shutdown().await;
}

#[tokio::test]
async fn shutdown_releases_name_and_socket() {
    let t = TestDaemon::start().await;
    assert!(t.socket.exists());
    let socket = t.socket.clone();
    let bus = t.bus.address.clone();
    let TestDaemon {
        daemon,
        dir,
        bus: private_bus,
        ..
    } = t;
    daemon.unwrap().shutdown().await;
    assert!(!socket.exists(), "socket removed");
    // The name is free again: a new daemon can start on the same bus.
    let mut cfg = config(&private_bus, dir.path());
    cfg.bus = tidings::daemon::Bus::Address(bus);
    let (again, _ui) = start_daemon(cfg, &FakeClock::at_unix(0)).await.unwrap();
    again.shutdown().await;
}
