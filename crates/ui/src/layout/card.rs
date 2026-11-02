//! Notification card layout, shared by panel cards, stacks and toasts.

use tidings_core::model::{Action, Image, Urgency};
use tidings_core::view::CardView;

use crate::geom::{Point, Rect, snap};
use crate::scene::{Align, GlyphKind, HitTarget, Monogram, Node, Scene};
use crate::style::{Fill, MONOGRAM_TINTS, Role, Style, TextStyle, fills, metrics as m};
use crate::text::{TextFit, TextMeasure};

/// How a card is presented.
#[derive(Debug, Clone)]
pub struct CardOpts<'a> {
    /// The theme's shapes and fills.
    pub style: &'a Style,
    /// Click target for the card body.
    pub body_target: HitTarget,
    /// Target for the hover "×" button.
    pub close_target: HitTarget,
    /// Whether the card is hovered (shows the "×", lightens the fill).
    pub hovered: bool,
    /// The currently hovered target, for pill hover feedback.
    pub hovered_target: Option<HitTarget>,
}

/// What to measure a card for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardSpec {
    pub width: f32,
    /// Body line clamp.
    pub body_lines: u32,
    /// Side of a square in the card's top-right corner (from the outer edge)
    /// kept clear of text, for a corner button.
    pub corner_reserve: f32,
    /// Where the icon sits against the text block.
    pub icon_align: IconAlign,
    /// Padding under the last row (the other sides use [`m::CARD_PADDING`]).
    pub pad_bottom: f32,
}

/// Vertical placement of a card's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconAlign {
    /// Centred against the text block, like macOS panel cards.
    Center,
    /// Level with the first (app name) row.
    Top,
}

impl CardSpec {
    /// A panel card: full-width rows, no corner button.
    #[must_use]
    pub fn panel(width: f32, body_lines: u32) -> Self {
        Self {
            width,
            body_lines,
            corner_reserve: 0.0,
            icon_align: IconAlign::Center,
            pad_bottom: m::CARD_PADDING,
        }
    }
}

/// A measured card, positioned relative to its own top-left corner.
#[derive(Debug, Clone)]
pub struct CardLayout {
    pub width: f32,
    pub height: f32,
    icon: Rect,
    app: (Rect, TextFit),
    time: (Rect, TextFit),
    summary: Option<(Rect, TextFit)>,
    body: Option<(Rect, TextFit)>,
    actions: Vec<(Rect, TextFit, String)>,
}

/// Space between text and a corner-reserved button.
const CORNER_GAP: f32 = 4.0;

fn fit_rect(m: &mut dyn TextMeasure, text: &str, style: TextStyle, w: f32, lines: u32) -> TextFit {
    m.fit(text, style, w.max(1.0), lines)
}

/// Measures a panel card of `width` (pure; positions are card-relative).
pub fn measure_card(
    card: &CardView,
    width: f32,
    body_lines: u32,
    scale: f32,
    meas: &mut dyn TextMeasure,
) -> CardLayout {
    measure_card_with(card, CardSpec::panel(width, body_lines), scale, meas)
}

/// Measures a card to `spec` (pure; positions are card-relative).
pub fn measure_card_with(
    card: &CardView,
    spec: CardSpec,
    scale: f32,
    meas: &mut dyn TextMeasure,
) -> CardLayout {
    let CardSpec {
        width,
        body_lines,
        corner_reserve,
        icon_align,
        pad_bottom,
    } = spec;
    let pad = m::CARD_PADDING;
    let text_x = pad + m::ICON_SIZE + m::ICON_GAP;
    let col_right = width - pad;
    let text_w = col_right - text_x;

    // Row 1: app name (left) and time (right).
    let meta_h = meas.line_height(TextStyle::Meta);
    // Rows starting inside the corner reserve stop short of it.
    let clear_right = width - corner_reserve - CORNER_GAP;
    let row_right = if pad < corner_reserve {
        col_right.min(clear_right)
    } else {
        col_right
    };
    let time = fit_rect(
        meas,
        &card.time_label,
        TextStyle::Meta,
        (row_right - text_x) / 2.0,
        1,
    );
    let time_rect = Rect::new(row_right - time.width, pad, time.width, meta_h);
    let app_w = (row_right - text_x - time.width - 8.0).max(1.0);
    let app = fit_rect(meas, &card.app_name, TextStyle::Meta, app_w, 1);
    let app_rect = Rect::new(text_x, pad, app_w, meta_h);

    let mut y = pad + meta_h + m::LINE_GAP;
    let summary = (!card.summary.trim().is_empty()).then(|| {
        let w = if y < corner_reserve {
            text_w.min(clear_right - text_x)
        } else {
            text_w
        };
        let f = fit_rect(meas, &card.summary, TextStyle::Summary, w, m::SUMMARY_LINES);
        let r = Rect::new(text_x, y, w, f.height);
        y += f.height + m::LINE_GAP;
        (r, f)
    });
    let body = (!card.body.trim().is_empty()).then(|| {
        let f = fit_rect(meas, &card.body, TextStyle::Body, text_w, body_lines);
        let r = Rect::new(text_x, y, text_w, f.height);
        y += f.height + m::LINE_GAP;
        (r, f)
    });
    let text_bottom = y - m::LINE_GAP;
    let content_bottom = text_bottom.max(pad + m::ICON_SIZE);

    let icon_y = match icon_align {
        IconAlign::Center => snap(pad + ((content_bottom - pad) - m::ICON_SIZE) / 2.0, scale),
        IconAlign::Top => pad,
    };
    let icon = Rect::new(pad, icon_y, m::ICON_SIZE, m::ICON_SIZE);

    let (actions, bottom) = measure_actions(
        &card.actions,
        (text_x, col_right),
        content_bottom,
        scale,
        meas,
    );

    CardLayout {
        width,
        height: snap(bottom + pad_bottom, scale),
        icon,
        app: (app_rect, app),
        time: (time_rect, time),
        summary,
        body,
        actions,
    }
}

/// Action pills of equal width, in rows under the text column `(left,
/// right)` starting below `top`. Returns the pills and the last row's bottom
/// (`top` if there are none).
fn measure_actions(
    list: &[Action],
    (left, right): (f32, f32),
    top: f32,
    scale: f32,
    meas: &mut dyn TextMeasure,
) -> (Vec<(Rect, TextFit, String)>, f32) {
    let mut actions = Vec::new();
    let mut bottom = top;
    for row in list.chunks(m::ACTIONS_PER_ROW) {
        let n = crate::geom::u32_to_f32(u32::try_from(row.len()).unwrap_or(1));
        let pill_w = snap((right - left - (n - 1.0) * m::ACTION_GAP) / n, scale);
        let row_y = bottom + m::ACTION_GAP + 2.0;
        let mut x = left;
        for action in row {
            let label = &action.label;
            let f = fit_rect(
                meas,
                label,
                TextStyle::Button,
                pill_w - 2.0 * m::PILL_PAD_X,
                1,
            );
            actions.push((
                Rect::new(x, row_y, pill_w, m::PILL_HEIGHT),
                f,
                action.key.clone(),
            ));
            x += pill_w + m::ACTION_GAP;
        }
        bottom = row_y + m::PILL_HEIGHT;
    }
    (actions, bottom)
}

fn text(rect: Rect, fit: &TextFit, style: TextStyle, fill: Fill, align: Align) -> Node {
    Node::Text {
        rect,
        text: fit.text.clone(),
        style,
        fill,
        align,
        wrap_width: rect.w,
    }
}

/// Which images an avatar shows: `(main, badge)`.
///
/// When a notification carries both an image and an app icon, the image is
/// the avatar and the app icon a small badge on it, as macOS does for web
/// notifications. Chrome, for one, sends its own logo as `app_icon` and the
/// site's icon (an `IRCCloud` or Gmail logo) as `image-path`: the site is what the
/// reader needs to recognise, the browser only says where it came from.
#[must_use]
pub fn avatar_images(card: &CardView) -> (Option<Image>, Option<Image>) {
    match (&card.image, &card.icon) {
        (Some(image), Some(icon)) => (Some(image.clone()), Some(icon.clone())),
        (image, icon) => (image.clone().or_else(|| icon.clone()), None),
    }
}

/// Paints a round avatar in `rect`, with the app badge when there is one.
pub fn emit_avatar(scene: &mut Scene, card: &CardView, rect: Rect) {
    let (main, badge) = avatar_images(card);
    let letter = monogram(&card.app_name);
    scene.push(Node::Image {
        rect,
        radius: rect.w / 2.0,
        image: main,
        fallback: letter,
    });
    if let Some(badge) = badge {
        let size = (rect.w * m::BADGE_SCALE).round();
        let at = Point::new(
            rect.right() - size * m::BADGE_OVERHANG,
            rect.bottom() - size * m::BADGE_OVERHANG,
        );
        let ring = m::BADGE_RING;
        scene.push(Node::RoundRect {
            rect: Rect::new(
                at.x - ring,
                at.y - ring,
                size + 2.0 * ring,
                size + 2.0 * ring,
            ),
            radius: size / 2.0 + ring,
            fill: Some(fills::BADGE_RING),
            stroke: None,
        });
        scene.push(Node::Image {
            rect: Rect::new(at.x, at.y, size, size),
            radius: size / 2.0,
            image: Some(badge),
            fallback: letter,
        });
    }
}

/// The letter tile shown for `app_name` when its icon cannot be: the first
/// letter, on a pastel picked from a stable hash of the name, so an app
/// always gets the same colour (across runs, too) and different apps tend to
/// differ.
#[must_use]
pub fn monogram(app_name: &str) -> Monogram {
    // FNV-1a: tiny, stable across builds (unlike std's randomised hasher).
    let hash = app_name
        .trim()
        .to_lowercase()
        .bytes()
        .fold(0x811c_9dc5_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
    let n = u32::try_from(MONOGRAM_TINTS.len()).unwrap_or(1);
    Monogram {
        letter: fallback_letter(app_name),
        tint: u8::try_from(hash % n).unwrap_or(0),
    }
}

/// First letter of the app name, used when an icon cannot be shown.
#[must_use]
pub fn fallback_letter(app_name: &str) -> char {
    app_name
        .chars()
        .find(|c| c.is_alphanumeric())
        .and_then(|c| c.to_uppercase().next())
        .unwrap_or('•')
}

/// Emits a measured panel card at `origin` into `scene`: its tinted fill and
/// outline, the content, and the hover "×". Hits are added only when they
/// intersect `hit_clip`.
pub fn emit_card(
    scene: &mut Scene,
    card: &CardView,
    lay: &CardLayout,
    origin: Point,
    opts: &CardOpts<'_>,
    hit_clip: Rect,
) {
    let style = opts.style;
    let bounds = Rect::new(origin.x, origin.y, lay.width, lay.height);
    let stroke = if card.urgency == Urgency::Critical {
        Some(style.critical)
    } else {
        style.card_border
    };
    scene.push(Node::RoundRect {
        rect: bounds,
        radius: style.card_radius(lay.height),
        fill: Some(if opts.hovered {
            style.card_hover
        } else {
            style.card
        }),
        stroke,
    });
    emit_card_content(scene, card, lay, origin, opts, hit_clip);
    if opts.hovered {
        close_button(
            scene,
            Point::new(origin.x + m::CLOSE_OFFSET, origin.y + m::CLOSE_OFFSET),
            m::CLOSE_RADIUS,
            opts.close_target.clone(),
            opts.hovered_target.as_ref() == Some(&opts.close_target),
            hit_clip,
        );
    }
}

/// Emits a measured card's content without any background: icon, texts, the
/// body hit region and the action pills.
pub fn emit_card_content(
    scene: &mut Scene,
    card: &CardView,
    lay: &CardLayout,
    origin: Point,
    opts: &CardOpts<'_>,
    hit_clip: Rect,
) {
    let at = |r: Rect| r.translate(origin.x, origin.y);
    let bounds = Rect::new(origin.x, origin.y, lay.width, lay.height);
    emit_avatar(scene, card, at(lay.icon));
    let muted = Fill::solid(Role::OnSurfaceVariant);
    let strong = Fill::solid(Role::OnSurface);
    scene.push(text(
        at(lay.app.0),
        &lay.app.1,
        TextStyle::Meta,
        muted,
        Align::Left,
    ));
    scene.push(text(
        at(lay.time.0),
        &lay.time.1,
        TextStyle::Meta,
        muted,
        Align::Right,
    ));
    if let Some((r, f)) = &lay.summary {
        scene.push(text(at(*r), f, TextStyle::Summary, strong, Align::Left));
    }
    if let Some((r, f)) = &lay.body {
        scene.push(text(at(*r), f, TextStyle::Body, muted, Align::Left));
    }

    let radius = opts.style.card_radius(lay.height);
    add_hit(scene, bounds, radius, opts.body_target.clone(), hit_clip);
    for (r, f, key) in &lay.actions {
        let target = HitTarget::Action {
            id: card.id,
            key: key.clone(),
        };
        let hovered = opts.hovered_target.as_ref() == Some(&target);
        pill(scene, opts.style, at(*r), f, hovered, target, hit_clip);
    }
}

/// Adds a hit region clipped to `clip` (dropped if fully outside).
pub fn add_hit(scene: &mut Scene, rect: Rect, radius: f32, target: HitTarget, clip: Rect) {
    if let Some(r) = rect.intersect(&clip) {
        // Keep the corner radius only when nothing was clipped away.
        let radius = if r == rect { radius } else { 0.0 };
        scene.hit(r, radius, target);
    }
}

/// A text pill button.
pub fn pill(
    scene: &mut Scene,
    style: &Style,
    rect: Rect,
    label: &TextFit,
    hovered: bool,
    target: HitTarget,
    clip: Rect,
) {
    let radius = style.button_radius(rect.h);
    scene.push(Node::RoundRect {
        rect,
        radius,
        fill: Some(style.button_fill(hovered)),
        stroke: None,
    });
    let lh = style.text.line_height(TextStyle::Button);
    let tr = Rect::new(rect.x, rect.y + (rect.h - lh) / 2.0, rect.w, lh);
    scene.push(Node::Text {
        rect: tr,
        text: label.text.clone(),
        style: TextStyle::Button,
        fill: Fill::solid(Role::OnSurface),
        align: Align::Center,
        wrap_width: (rect.w - 2.0 * m::PILL_PAD_X).max(1.0),
    });
    add_hit(scene, rect, radius, target, clip);
}

/// A round "×" button.
pub fn close_button(
    scene: &mut Scene,
    center: Point,
    radius: f32,
    target: HitTarget,
    hovered: bool,
    clip: Rect,
) {
    scene.push(Node::Circle {
        center,
        radius,
        fill: if hovered {
            fills::CLOSE_BG_HOVER
        } else {
            fills::CLOSE_BG
        },
        stroke: Some(fills::CLOSE_RING),
    });
    let g = radius * m::CLOSE_GLYPH;
    scene.push(Node::Glyph {
        rect: Rect::new(center.x - g / 2.0, center.y - g / 2.0, g, g),
        kind: GlyphKind::Close,
        fill: Fill::solid(if hovered {
            Role::OnSurface
        } else {
            Role::OnSurfaceVariant
        }),
    });
    let hit = Rect::new(
        center.x - radius,
        center.y - radius,
        radius * 2.0,
        radius * 2.0,
    );
    add_hit(scene, hit, radius, target, clip);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::tests::sample_card;
    use crate::text::MonoMeasure;

    #[test]
    fn card_height_grows_with_body_and_actions() {
        let mut meas = MonoMeasure;
        let mut c = sample_card(1, "Mail");
        c.body.clear();
        c.actions.clear();
        let plain = measure_card(&c, 356.0, 2, 1.0, &mut meas);
        // Padding + icon dominates a one-line card.
        assert!((plain.height - (2.0 * m::CARD_PADDING + m::ICON_SIZE)).abs() < 1.0);

        let mut with_body = c.clone();
        with_body.body = "word ".repeat(80);
        let b = measure_card(&with_body, 356.0, 2, 1.0, &mut meas);
        assert!(b.height > plain.height);
        assert_eq!(b.body.as_ref().map(|(_, f)| f.lines), Some(2));

        let expanded = measure_card(&with_body, 356.0, 8, 1.0, &mut meas);
        assert!(expanded.height > b.height);

        let mut with_actions = c.clone();
        with_actions.actions = sample_card(1, "x").actions;
        let a = measure_card(&with_actions, 356.0, 2, 1.0, &mut meas);
        assert!(a.height >= plain.height + m::PILL_HEIGHT);
        assert_eq!(a.actions.len(), 2);
        // Equal-width pills stay inside the card.
        assert!(
            a.actions
                .iter()
                .all(|(r, ..)| r.right() <= 356.0 - m::CARD_PADDING + 0.5)
        );
    }

    #[test]
    fn fallback_letter_is_uppercase_alnum() {
        assert_eq!(fallback_letter("  firefox"), 'F');
        assert_eq!(fallback_letter("—"), '•');
    }

    #[test]
    fn monogram_is_stable_per_app_and_varies_between_apps() {
        let a = monogram("Thunderbird");
        assert_eq!(a, monogram("Thunderbird"));
        assert_eq!(a, monogram("  thunderbird "), "case and padding ignored");
        assert_eq!(a.letter, 'T');
        // FNV-1a of the folded name: the same in every build and run.
        let fnv = "thunderbird".bytes().fold(0x811c_9dc5_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
        assert_eq!(u32::from(a.tint), fnv % 6);
        let apps = [
            "Signal",
            "Thunderbird",
            "Battery",
            "Firefox",
            "Calculator",
            "Slack",
        ];
        let tints: std::collections::HashSet<u8> = apps.iter().map(|a| monogram(a).tint).collect();
        assert!(tints.len() >= 3, "{tints:?}");
        assert!(tints.iter().all(|&t| usize::from(t) < MONOGRAM_TINTS.len()));
    }

    #[test]
    fn avatar_carries_the_apps_monogram() {
        let c = sample_card(1, "Signal");
        let mut scene = Scene::default();
        emit_avatar(&mut scene, &c, Rect::new(0.0, 0.0, 40.0, 40.0));
        assert!(
            scene.nodes.iter().any(
                |n| matches!(n, Node::Image { fallback, .. } if *fallback == monogram("Signal"))
            )
        );
    }

    #[test]
    fn icon_is_centred_on_panel_cards_and_top_aligned_on_request() {
        let mut meas = MonoMeasure;
        let mut c = sample_card(1, "Mail");
        c.body = "word ".repeat(80);
        let spec = CardSpec::panel(356.0, 4);
        let centred = measure_card_with(&c, spec, 1.0, &mut meas);
        let text_mid = f32::midpoint(
            m::CARD_PADDING,
            centred.body.as_ref().map_or(0.0, |(r, _)| r.bottom()),
        );
        assert!((centred.icon.center().y - text_mid).abs() < 1.0);
        assert!(centred.icon.y > m::CARD_PADDING + 1.0);

        let top = CardSpec {
            icon_align: IconAlign::Top,
            ..spec
        };
        let topped = measure_card_with(&c, top, 1.0, &mut meas);
        assert!((topped.icon.y - topped.app.0.y).abs() < f32::EPSILON);
        assert_eq!(topped.height, centred.height);
    }

    #[test]
    fn bottom_padding_follows_the_spec() {
        let mut meas = MonoMeasure;
        let c = sample_card(1, "Mail");
        let spec = CardSpec::panel(356.0, 2);
        let a = measure_card_with(&c, spec, 1.0, &mut meas);
        let b = measure_card_with(
            &c,
            CardSpec {
                pad_bottom: spec.pad_bottom + 2.0,
                ..spec
            },
            1.0,
            &mut meas,
        );
        assert!((b.height - a.height - 2.0).abs() < f32::EPSILON);
    }

    fn img(name: &str) -> Image {
        Image::IconName(name.into())
    }

    #[test]
    fn site_image_is_the_avatar_and_app_icon_the_badge() {
        // What Chrome sends for a web notification (captured off D-Bus): its
        // own logo as app_icon, the site's icon as image-path.
        let mut c = sample_card(1, "Google Chrome");
        c.icon = Some(Image::Path("/tmp/chrome/logo.png".into()));
        c.image = Some(Image::Path("/tmp/chrome/icon.png".into()));
        let (main, badge) = avatar_images(&c);
        assert_eq!(main, c.image);
        assert_eq!(badge, c.icon);
    }

    #[test]
    fn a_single_image_is_the_avatar_without_a_badge() {
        let mut c = sample_card(1, "App");
        c.icon = Some(img("firefox"));
        c.image = None;
        assert_eq!(avatar_images(&c), (Some(img("firefox")), None));
        c.icon = None;
        c.image = Some(img("photo"));
        assert_eq!(avatar_images(&c), (Some(img("photo")), None));
        c.image = None;
        assert_eq!(avatar_images(&c), (None, None));
    }

    #[test]
    fn badge_sits_on_the_avatar_bottom_right_above_a_ring() {
        let mut c = sample_card(1, "Google Chrome");
        c.icon = Some(img("google-chrome"));
        c.image = Some(img("irccloud"));
        let avatar = Rect::new(10.0, 10.0, 40.0, 40.0);
        let mut scene = Scene::default();
        emit_avatar(&mut scene, &c, avatar);
        let images: Vec<(Rect, Option<Image>)> = scene
            .nodes
            .iter()
            .filter_map(|n| match n {
                Node::Image { rect, image, .. } => Some((*rect, image.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0], (avatar, Some(img("irccloud"))));
        let (badge, badge_img) = &images[1];
        assert_eq!(badge_img, &Some(img("google-chrome")));
        assert!(badge.w < avatar.w / 2.0);
        // Overlaps the avatar's bottom-right corner and hangs past it.
        assert!(badge.x > avatar.center().x && badge.y > avatar.center().y);
        assert!(badge.right() > avatar.right() && badge.bottom() > avatar.bottom());
        assert!(
            scene
                .nodes
                .iter()
                .any(|n| matches!(n, Node::RoundRect { rect, .. }
                if rect.x < badge.x && rect.right() > badge.right())),
            "a ring sits behind the badge"
        );
    }
}
