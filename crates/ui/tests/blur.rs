//! Blur on request (`ext-background-effect-v1`), against a fake compositor
//! that offers it (the headless sway in the end-to-end tests does not).

mod fake;

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use fake::{Blur, FakeCompositor, OUTPUT, Rect4};
use tidings_core::config::AnimationConfig;
use tidings_core::look::Look;
use tidings_core::model::{NotificationId, Urgency};
use tidings_core::ui::{PanelCommand, UiEvent, UiHandle, UiUpdate};
use tidings_core::view::{CardView, ToastView, ViewModel};
use tidings_ui::style::{Style, metrics as m};
use tidings_ui::{Display, spawn_on};

fn ui(fake: &FakeCompositor, look: &Look) -> (Box<dyn UiHandle>, Receiver<UiEvent>) {
    let (tx, rx) = channel();
    let instant = AnimationConfig {
        enabled: false,
        ..AnimationConfig::default()
    };
    let ui = spawn_on(&Display::Socket(fake.socket().into()), tx, instant, look)
        .unwrap_or_else(|e| panic!("{e}"));
    (ui, rx)
}

fn one_toast() -> ViewModel {
    let card = CardView {
        id: NotificationId(1),
        app_name: "Chat".into(),
        icon: None,
        image: None,
        summary: "Ada".into(),
        body: "Are we still on for 6?".into(),
        time_label: "now".into(),
        actions: Vec::new(),
        reply: None,
        origin: None,
        clickable: true,
        urgency: Urgency::Normal,
    };
    ViewModel {
        toasts: vec![ToastView {
            card,
            paused: false,
            expanded: false,
        }],
        ..ViewModel::default()
    }
}

fn covers(rects: &[Rect4], (px, py): (i32, i32)) -> bool {
    rects
        .iter()
        .any(|&(x, y, w, h)| px >= x && px < x + w && py >= y && py < y + h)
}

/// The smallest rect holding all of `rects`.
fn bounds(rects: &[Rect4]) -> Rect4 {
    let x0 = rects.iter().map(|r| r.0).min().unwrap_or(0);
    let y0 = rects.iter().map(|r| r.1).min().unwrap_or(0);
    let x1 = rects.iter().map(|r| r.0 + r.2).max().unwrap_or(0);
    let y1 = rects.iter().map(|r| r.1 + r.3).max().unwrap_or(0);
    (x0, y0, x1 - x0, y1 - y0)
}

#[allow(clippy::cast_possible_truncation, reason = "small layout constants")]
fn px(v: f32) -> i32 {
    v.round() as i32
}

#[test]
fn toasts_and_the_panel_ask_for_blur_shaped_like_their_glass() {
    let fake = FakeCompositor::start(Blur::Offered);
    let (ui, events) = ui(&fake, &Look::default());
    ui.send(UiUpdate::View(Box::new(one_toast())));

    // The toast: exactly its rounded card, not the shadow margin around it.
    let rec = fake.wait_for(|r| r.last_blur("tidings-toast").is_some());
    assert!(rec.effects_created >= 1);
    let rects = rec.last_blur("tidings-toast").cloned().unwrap_or_default();
    let inset = px(m::TOAST_INSET);
    assert_eq!(
        bounds(&rects),
        (inset, inset, px(m::TOAST_WIDTH), px(m::TOAST_HEIGHT))
    );
    let mid = (
        inset + px(m::TOAST_WIDTH) / 2,
        inset + px(m::TOAST_HEIGHT) / 2,
    );
    assert!(covers(&rects, mid));
    assert!(!covers(&rects, (inset, inset)), "a pill's corner is cut");
    assert!(!covers(&rects, (0, 0)), "the shadow margin is not blurred");

    // The panel: its sheet, full height, with the theme's corner radius.
    ui.send(UiUpdate::Panel(PanelCommand::AnimateTo(1.0)));
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).ok(),
        Some(UiEvent::PanelSettled { open: true })
    );
    let rec = fake.wait_for(|r| r.last_blur("tidings-panel").is_some());
    let rects = rec.last_blur("tidings-panel").cloned().unwrap_or_default();
    // The fake gives the panel the whole output height.
    let full = i32::try_from(OUTPUT.1).unwrap_or(0);
    assert_eq!(bounds(&rects), (0, 0, px(m::PANEL_WIDTH), full));
    let radius = px(Style::new(&Look::default()).panel_radius);
    assert!(radius > 2, "the default panel is rounded");
    assert!(!covers(&rects, (0, 0)) && covers(&rects, (radius, radius)));
    // The catcher behind the panel is never blurred.
    assert!(rec.namespaces.iter().any(|n| n == "tidings-catcher"));
    assert!(rec.last_blur("tidings-catcher").is_none());

    // Surfaces going away take their blur objects with them.
    ui.send(UiUpdate::Shutdown);
    let rec = fake.wait_for(|r| r.effects_destroyed == r.effects_created);
    assert!(rec.effects_destroyed >= 2);
}

#[test]
fn a_theme_without_blur_asks_for_none() {
    let fake = FakeCompositor::start(Blur::Offered);
    let mut look = Look::default();
    look.panel.blur = false;
    look.toast.blur = false;
    let (ui, _events) = ui(&fake, &look);
    ui.send(UiUpdate::View(Box::new(one_toast())));
    let rec = fake.wait_for(|r| r.namespaces.iter().any(|n| n == "tidings-toast-noblur"));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(rec.effects_created, 0);
    assert_eq!(fake.record().blur, []);
}

#[test]
fn a_compositor_that_cannot_blur_gets_no_regions() {
    let fake = FakeCompositor::start(Blur::Refused);
    let (ui, _events) = ui(&fake, &Look::default());
    ui.send(UiUpdate::View(Box::new(one_toast())));
    fake.wait_for(|r| r.namespaces.iter().any(|n| n == "tidings-toast"));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(fake.record().blur, []);
}
