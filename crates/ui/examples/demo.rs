//! Live demo of the UI on the running compositor (no D-Bus involved).
//!
//! `cargo run -p tidings-ui --example demo --release -- [mode] [hold-secs]`
//!
//! Modes: `all` (default: toasts slide in, then the panel slides open),
//! `toasts` (the compact stack, the second toast expanded), `panel`, `half`
//! (panel tracked to 50%), `expanded` (stack expanded), `empty`, `light`
//! (light theme). UI events are printed.
//!
//! `-- states <dir>` renders one toast collapsed, hovered and expanded to
//! `<dir>/toast-*.png` offscreen (hover cannot be forced on a live surface).

use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, SystemTime};

use tidings_core::model::{Action, Image, NotificationId, Urgency};
use tidings_core::theme::{Mode, Rgb, Theme};
use tidings_core::ui::{PanelCommand, UiUpdate};
use tidings_core::view::{CardView, GroupView, ToastView, ViewModel};

fn card(id: u32, app: &str, icon: Option<&str>, summary: &str, body: &str, time: &str) -> CardView {
    CardView {
        id: NotificationId(id),
        app_name: app.into(),
        icon: icon.map(|i| Image::IconName(i.into())),
        image: None,
        summary: summary.into(),
        body: body.into(),
        time_label: time.into(),
        actions: Vec::new(),
        clickable: true,
        urgency: Urgency::Normal,
    }
}

fn action(key: &str, label: &str) -> Action {
    Action {
        key: key.into(),
        label: label.into(),
    }
}

/// A soft diagonal gradient, standing in for an `image-data` thumbnail.
fn gradient(size: u8) -> Image {
    let n = u16::from(size);
    let mut px = Vec::new();
    for y in 0..n {
        for x in 0..n {
            // t in 0..=255 along the diagonal.
            let t = u8::try_from((x + y) * 255 / (2 * n)).unwrap_or(255);
            px.extend_from_slice(&[255 - t, 120 + t / 3, t, 255]);
        }
    }
    Image::Rgba {
        width: u32::from(size),
        height: u32::from(size),
        pixels: px.into(),
    }
}

fn sample(expanded: bool) -> ViewModel {
    let mut mail = card(
        12,
        "Thunderbird",
        None,
        "Quarterly planning — agenda",
        "Hi James, attached is the draft agenda for Thursday. Could you add the Hyprland roadmap items before the meeting? Thanks!",
        "2m",
    );
    mail.actions = vec![
        action("reply", "Reply"),
        action("archive", "Archive"),
        action("spam", "Mark as Spam"),
    ];
    let mut photo = card(
        11,
        "Firefox",
        Some("firefox"),
        "Download complete",
        "sunset-over-the-harbour.png (4.2 MB)",
        "5m",
    );
    photo.image = Some(gradient(96));
    photo.actions = vec![action("open", "Open"), action("show", "Show in Folder")];
    let chat = [
        card(
            10,
            "Signal",
            None,
            "Alex",
            "Are we still on for lunch tomorrow? I found a great new ramen place near the office.",
            "now",
        ),
        card(9, "Signal", None, "Alex", "Also bring the adapter", "12m"),
        card(8, "Signal", None, "Sam", "🎉 Happy birthday!", "1h"),
    ];
    let mut crit = card(
        7,
        "Battery",
        None,
        "Battery critically low",
        "5% remaining. Plug in your charger now.",
        "20m",
    );
    crit.urgency = Urgency::Critical;
    let crit_toast = crit.clone();
    let calc = card(
        5,
        "Calculator",
        Some("org.gnome.Calculator"),
        "Result copied",
        "42",
        "Yesterday",
    );
    ViewModel {
        groups: vec![
            group("Signal", chat.to_vec(), expanded),
            group("Thunderbird", vec![mail.clone()], false),
            group("Firefox", vec![photo.clone()], false),
            group("Battery", vec![crit], false),
            group("Calculator", vec![calc], false),
        ],
        toasts: vec![
            toast(chat[0].clone(), false),
            toast(mail, true),
            toast(photo, false),
            toast(crit_toast, false),
        ],
        dnd: false,
    }
}

fn toast(card: CardView, expanded: bool) -> ToastView {
    ToastView {
        card,
        paused: expanded,
        expanded,
    }
}

fn group(app: &str, cards: Vec<CardView>, expanded: bool) -> GroupView {
    GroupView {
        key: app.to_lowercase(),
        app_name: app.into(),
        icon: None,
        cards,
        expanded,
    }
}

fn light_theme() -> Theme {
    Theme {
        mode: Mode::Light,
        surface: Rgb(0xf4, 0xf2, 0xf8),
        surface_variant: Rgb(0xe4, 0xe1, 0xec),
        on_surface: Rgb(0x1b, 0x1b, 0x21),
        on_surface_variant: Rgb(0x5d, 0x5b, 0x68),
        primary: Rgb(0x3e, 0x5b, 0xc9),
        on_primary: Rgb(0xff, 0xff, 0xff),
        error: Rgb(0xba, 0x1a, 0x1a),
        on_error: Rgb(0xff, 0xff, 0xff),
        outline: Rgb(0x9a, 0x97, 0xa4),
        surface_alpha: 0.72,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "all".into());
    if mode == "states" {
        let dir = args.next().unwrap_or_else(|| ".".into());
        return render_states(std::path::Path::new(&dir));
    }

    let hold = Duration::from_secs_f32(args.next().and_then(|s| s.parse().ok()).unwrap_or(3.0));

    let (tx, rx) = mpsc::channel();
    let ui = tidings_ui::spawn(tx)?;
    std::thread::spawn(move || {
        for ev in rx {
            println!("ui event: {ev:?}");
        }
    });
    let _ = SystemTime::now();

    if mode == "light" {
        ui.send(UiUpdate::Theme(Box::new(light_theme())));
    }
    let mut view = sample(mode == "expanded");
    if mode == "empty" {
        view = ViewModel::default();
    }
    if mode != "toasts" && mode != "all" {
        view.toasts.clear();
    }
    ui.send(UiUpdate::View(Box::new(view)));

    match mode.as_str() {
        "toasts" => sleep(hold),
        "half" => {
            ui.send(UiUpdate::Panel(PanelCommand::Track(0.5)));
            sleep(hold);
            ui.send(UiUpdate::Panel(PanelCommand::AnimateTo(0.0)));
            sleep(Duration::from_millis(600));
        }
        "all" => {
            sleep(Duration::from_millis(1500));
            open_and_hold(&*ui, hold);
        }
        _ => open_and_hold(&*ui, hold),
    }
    ui.send(UiUpdate::Shutdown);
    sleep(Duration::from_millis(100));
    Ok(())
}

/// Simulates a finger drag to 40%, then releases into a spring-open.
fn open_and_hold(ui: &dyn tidings_core::ui::UiHandle, hold: Duration) {
    for i in 1..=12u8 {
        ui.send(UiUpdate::Panel(PanelCommand::Track(f32::from(i) / 30.0)));
        sleep(Duration::from_millis(12));
    }
    ui.send(UiUpdate::Panel(PanelCommand::AnimateTo(1.0)));
    sleep(hold);
    ui.send(UiUpdate::Panel(PanelCommand::AnimateTo(0.0)));
    sleep(Duration::from_millis(600));
}

/// Renders the mail toast's three states over a dark backdrop at the
/// laptop's 1.6 scale.
fn render_states(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use tidings_ui::layout::layout_toast;
    use tidings_ui::render::Painter;
    use tidings_ui::scene::HitTarget;
    use tidings_ui::style::Palette;

    let scale = 1.6;
    let mut painter = Painter::new();
    let palette = Palette::default();
    let view = sample(false);
    let mail = view
        .toasts
        .iter()
        .find(|t| t.card.id == NotificationId(12))
        .ok_or("no mail toast")?;
    let hover = HitTarget::Activate(mail.card.id);
    let states = [
        ("collapsed", toast(mail.card.clone(), false), None),
        ("hover", toast(mail.card.clone(), false), Some(&hover)),
        ("expanded", toast(mail.card.clone(), true), None),
    ];
    for (name, t, hovered) in states {
        let lay = layout_toast(&t, scale, hovered, &mut painter.text);
        let (w, h) = tidings_ui::render::physical_size(&lay.scene, scale);
        let mut pm = tiny_skia::Pixmap::new(w, h).ok_or("empty toast")?;
        pm.fill(tiny_skia::Color::from_rgba8(0x1b, 0x12, 0x14, 0xff));
        painter.paint_into(&mut pm, &lay.scene, &palette, scale);
        let path = dir.join(format!("toast-{name}.png"));
        pm.save_png(&path)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
