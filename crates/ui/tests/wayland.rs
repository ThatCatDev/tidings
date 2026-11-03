//! End-to-end tests of the Wayland UI: the real UI thread against a private,
//! headless sway, driven by a virtual pointer (from here), `wtype` (keys)
//! and `wl-copy` (the clipboard).
//!
//! Each test starts its own sway in its own runtime directory, so they run
//! in parallel and never touch the session's compositor. Where to click is
//! worked out from the same layout code the UI uses, so the tests follow
//! the design rather than hard-coded pixels.
//!
//! Without sway (or `wtype`/`wl-copy`/`grim`) the tests skip, unless
//! `TIDINGS_REQUIRE_COMPOSITOR` is set, as CI sets it, where they fail.

mod harness;

use std::time::Duration;

use harness::{Compositor, Session};
use tidings_core::model::{Action, NotificationId, Reply, Urgency};
use tidings_core::ui::{PanelCommand, UiEvent, UiUpdate};
use tidings_core::view::{CardView, GroupView, ToastView, ViewModel};
use tidings_ui::scene::HitTarget;

const ID: NotificationId = NotificationId(7);

fn card(id: u32, reply: bool) -> CardView {
    CardView {
        id: NotificationId(id),
        app_name: "Chat".into(),
        icon: None,
        image: None,
        summary: format!("Ada {id}"),
        body: "Are we still on for 6 tonight?".into(),
        time_label: "now".into(),
        actions: vec![Action {
            key: "read".into(),
            label: "Mark as read".into(),
        }],
        reply: reply.then(|| Reply {
            label: "Reply".into(),
            placeholder: Some("Message".into()),
            submit: None,
        }),
        origin: Some("example.com".into()),
        clickable: true,
        urgency: Urgency::Normal,
    }
}

fn toast(expanded: bool) -> ToastView {
    ToastView {
        card: card(ID.0, true),
        paused: false,
        expanded,
    }
}

fn toasts(t: ToastView) -> ViewModel {
    ViewModel {
        groups: vec![GroupView {
            key: "chat".into(),
            app_name: "Chat".into(),
            icon: None,
            cards: vec![t.card.clone()],
            expanded: false,
        }],
        toasts: vec![t],
        dnd: false,
    }
}

fn is_reply(text: &str) -> impl Fn(&UiEvent) -> bool + '_ {
    move |e| matches!(e, UiEvent::Reply { id, text: t } if *id == ID && t == text)
}

#[test]
fn toast_hover_click_expand_and_dismiss() {
    let Some(sway) = Compositor::start() else {
        return;
    };
    let mut s = Session::start(&sway);
    s.view(toasts(toast(false)));

    // Hovering reports the toast (which pauses its expiry) and paints it.
    s.hover_toast(&toast(false), &HitTarget::Activate(ID));
    s.expect(|e| {
        *e == UiEvent::ToastHover {
            id: ID,
            hovered: true,
        }
    });
    assert!(s.toast_is_painted(), "the toast's glass is on screen");

    s.click_toast(&toast(false), &HitTarget::Activate(ID));
    s.expect(|e| *e == UiEvent::Activate(ID));
    s.click_toast(&toast(false), &HitTarget::ExpandToast(ID));
    s.expect(|e| {
        *e == UiEvent::SetToastExpanded {
            id: ID,
            expanded: true,
        }
    });

    s.view(toasts(toast(true)));
    let read = HitTarget::Action {
        id: ID,
        key: "read".into(),
    };
    s.click_toast(&toast(true), &read);
    s.expect(|e| {
        *e == UiEvent::InvokeAction {
            id: ID,
            key: "read".into(),
        }
    });
    s.click_toast(&toast(true), &HitTarget::CollapseToast(ID));
    s.expect(|e| {
        *e == UiEvent::SetToastExpanded {
            id: ID,
            expanded: false,
        }
    });

    s.view(toasts(toast(false)));
    s.click_toast(&toast(false), &HitTarget::Dismiss(ID));
    s.expect(|e| *e == UiEvent::Dismiss(ID));

    // Leaving reports it, and an empty view takes the toast away.
    s.pointer.move_to(10.0, 300.0);
    s.expect(|e| {
        *e == UiEvent::ToastHover {
            id: ID,
            hovered: false,
        }
    });
    s.view(ViewModel::default());
    s.settle();
}

#[test]
fn replying_from_a_toast() {
    let Some(sway) = Compositor::start() else {
        return;
    };
    let mut s = Session::start(&sway);
    let t = toast(true);
    s.view(toasts(t.clone()));

    // Type, fix a typo, send.
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["on my wayy"]);
    s.key("BackSpace");
    s.key("Return");
    s.expect(is_reply("on my way"));

    // Esc throws the draft away: the next reply starts empty.
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["nope"]);
    s.key("Escape");
    s.settle();
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["yes"]);
    // Word and line shortcuts.
    s.chord("ctrl", "a");
    s.keys(&["oh "]);
    s.chord("ctrl", "e");
    s.keys(&[" sure"]);
    s.chord("ctrl", "w");
    s.key("Return");
    s.expect(is_reply("oh yes"));

    // Pasting from the clipboard, then the send button.
    sway.copy("from the clipboard\n");
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.chord("ctrl", "v");
    s.settle();
    let draft = tidings_ui::compose::ReplyDraft {
        id: ID,
        text: "from the clipboard".into(),
        caret: 18,
        focused: true,
    };
    s.click_toast_with(&t, Some(&draft), &HitTarget::SendReply(ID));
    s.expect(is_reply("from the clipboard"));

    // A draft survives the field closing (here: the panel opening).
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["later"]);
    s.open_panel();
    s.close_panel();
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.key("Return");
    s.expect(is_reply("later"));
}

#[test]
fn panel_opens_works_and_closes() {
    let Some(sway) = Compositor::start() else {
        return;
    };
    let mut s = Session::animated(&sway);
    let mut view = toasts(toast(false));
    view.toasts.clear();
    // Enough cards to scroll.
    view.groups.extend((1..=6).map(|i| GroupView {
        key: format!("app{i}"),
        app_name: format!("App {i}"),
        icon: None,
        cards: vec![card(100 + i, false)],
        expanded: false,
    }));
    s.view(view.clone());

    // Finger tracking, then a spring open.
    s.ui.send(UiUpdate::Panel(PanelCommand::Track(0.3)));
    s.settle();
    s.open_panel();

    s.click_panel(&view, &HitTarget::ToggleDnd);
    s.expect(|e| *e == UiEvent::ToggleDnd);
    s.click_panel(&view, &HitTarget::Activate(ID));
    s.expect(|e| *e == UiEvent::Activate(ID));
    s.hover_panel(&view, &HitTarget::Activate(ID));
    s.pointer.scroll(120.0);
    s.settle();
    s.pointer.scroll(-120.0);
    s.settle();

    // Replying works in the panel too.
    s.click_panel(&view, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["from the panel"]);
    s.key("Return");
    s.expect(is_reply("from the panel"));

    // A new theme repaints; Esc closes.
    s.ui.send(UiUpdate::Theme(Box::default()));
    s.settle();
    s.click_panel(&view, &HitTarget::Background);
    s.key("Escape");
    s.expect(|e| *e == UiEvent::RequestClose);

    // So does a click outside the panel.
    s.pointer.move_to(20.0, 300.0);
    s.pointer.click();
    s.expect(|e| *e == UiEvent::RequestClose);
    s.ui.send(UiUpdate::Panel(PanelCommand::AnimateTo(0.0)));
    s.expect(|e| *e == UiEvent::PanelSettled { open: false });
    s.ui.send(UiUpdate::Shutdown);
    std::thread::sleep(Duration::from_millis(100));
}

#[test]
fn a_missing_compositor_is_an_error() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (tx, _rx) = std::sync::mpsc::channel();
    let result = tidings_ui::spawn_on(
        &tidings_ui::Display::Socket(dir.path().join("nothing-here")),
        tx,
        tidings_core::config::AnimationConfig::default(),
        &tidings_core::look::Look::default(),
    );
    let err = result.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("nothing-here"), "{err}");
}

#[test]
fn reply_field_edge_cases() {
    let Some(sway) = Compositor::start() else {
        return;
    };
    let mut s = Session::start(&sway);
    let t = toast(true);
    s.view(toasts(t.clone()));

    // Send does nothing on an empty field; Tab is swallowed.
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    let empty = tidings_ui::compose::ReplyDraft {
        id: ID,
        text: String::new(),
        caret: 0,
        focused: true,
    };
    s.click_toast_with(&t, Some(&empty), &HitTarget::SendReply(ID));
    s.key("Tab");

    // Collapsing the toast closes the field but keeps the draft.
    s.keys(&["kept"]);
    s.view(toasts(toast(false)));
    s.view(toasts(t.clone()));
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&[" and more"]);
    s.key("Return");
    s.expect(is_reply("kept and more"));

    // The notification going away forgets the draft.
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["gone"]);
    s.view(ViewModel::default());
    s.view(toasts(t.clone()));
    s.click_toast(&t, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["fresh"]);
    s.key("Return");
    s.expect(is_reply("fresh"));
}

#[test]
fn replying_to_another_card_moves_the_field() {
    let Some(sway) = Compositor::start() else {
        return;
    };
    let mut s = Session::start(&sway);
    let other = NotificationId(8);
    let mut view = toasts(toast(false));
    view.toasts.clear();
    view.groups[0].cards.push(card(other.0, true));
    view.groups[0].expanded = true;
    s.view(view.clone());
    s.open_panel();

    s.click_panel(&view, &HitTarget::StartReply(ID));
    s.settle();
    s.keys(&["first"]);
    let first = tidings_ui::compose::ReplyDraft {
        id: ID,
        text: "first".into(),
        caret: 5,
        focused: true,
    };
    s.click_panel_with(&view, Some(&first), &HitTarget::StartReply(other));
    s.settle();
    s.keys(&["second"]);
    s.key("Return");
    s.expect(|e| matches!(e, UiEvent::Reply { id, text } if *id == other && text == "second"));
}

#[test]
fn outputs_and_devices_come_and_go() {
    let Some(sway) = Compositor::start() else {
        return;
    };
    let mut s = Session::start(&sway);
    s.view(toasts(toast(false)));
    s.hover_toast(&toast(false), &HitTarget::Activate(ID));
    s.expect(|e| {
        *e == UiEvent::ToastHover {
            id: ID,
            hovered: true,
        }
    });

    // A second monitor, and a new scale (the toasts repaint for it).
    sway.sway("create_output");
    sway.sway("output HEADLESS-1 scale 2");
    s.settle();

    // The panel's monitor going away closes the panel.
    sway.sway("focus output HEADLESS-1");
    s.open_panel();
    sway.sway("output HEADLESS-1 unplug");
    s.expect(|e| *e == UiEvent::RequestClose);

    // Keyboard and mouse unplugged.
    sway.unplug_keyboard();
    s.pointer.unplug();
    s.view(ViewModel::default());
}
