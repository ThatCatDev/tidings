//! Notification card layout, shared by panel cards, stacks and toasts.

use tidings_core::model::{Action, Urgency};
use tidings_core::view::CardView;

use crate::geom::{Point, Rect, snap};
use crate::scene::{Align, GlyphKind, HitTarget, Node, Scene};
use crate::style::{Fill, Role, TextStyle, fills, metrics as m};
use crate::text::{TextFit, TextMeasure};

/// How a card is presented.
#[derive(Debug, Clone)]
pub struct CardOpts {
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
    /// kept clear of text and the thumbnail, for a corner button.
    pub corner_reserve: f32,
}

impl CardSpec {
    /// A panel card: full-width rows, no corner button.
    #[must_use]
    pub fn panel(width: f32, body_lines: u32) -> Self {
        Self {
            width,
            body_lines,
            corner_reserve: 0.0,
        }
    }
}

/// A measured card, positioned relative to its own top-left corner.
#[derive(Debug, Clone)]
pub struct CardLayout {
    pub width: f32,
    pub height: f32,
    icon: Rect,
    thumb: Option<Rect>,
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
    } = spec;
    let pad = m::CARD_PADDING;
    let text_x = pad + m::ICON_SIZE + m::ICON_GAP;
    let has_thumb = card.icon.is_some() && card.image.is_some();
    let col_right = width - pad;
    let text_w = col_right
        - text_x
        - if has_thumb {
            m::ICON_SIZE + m::ICON_GAP
        } else {
            0.0
        };

    // Row 1: app name (left) and time (right, spanning the thumbnail column).
    let meta_h = TextStyle::Meta.line_height();
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
    let thumb = has_thumb.then(|| {
        // Below a corner button if there is one.
        let below_button = if corner_reserve > 0.0 {
            corner_reserve + CORNER_GAP
        } else {
            0.0
        };
        Rect::new(
            col_right - m::ICON_SIZE,
            (pad + meta_h + m::LINE_GAP).max(below_button),
            m::ICON_SIZE,
            m::ICON_SIZE,
        )
    });
    let content_bottom = text_bottom
        .max(pad + m::ICON_SIZE)
        .max(thumb.map_or(0.0, |t| t.bottom()));

    // The icon sits centred against the text block, like macOS.
    let icon_y = snap(pad + ((content_bottom - pad) - m::ICON_SIZE) / 2.0, scale);
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
        height: snap(bottom + pad, scale),
        icon,
        thumb,
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

/// First letter of the app name, used when an icon cannot be shown.
#[must_use]
pub fn fallback_letter(app_name: &str) -> char {
    app_name
        .chars()
        .find(|c| c.is_alphanumeric())
        .and_then(|c| c.to_uppercase().next())
        .unwrap_or('•')
}

/// Corner radius for a card of `height`: a true pill while the card is short,
/// capped at [`m::CARD_RADIUS_MAX`] once it grows.
#[must_use]
pub fn card_radius(height: f32) -> f32 {
    (height / 2.0).min(m::CARD_RADIUS_MAX)
}

/// Emits a measured panel card at `origin` into `scene`: its tinted fill and
/// outline, the content, and the hover "×". Hits are added only when they
/// intersect `hit_clip`.
pub fn emit_card(
    scene: &mut Scene,
    card: &CardView,
    lay: &CardLayout,
    origin: Point,
    opts: &CardOpts,
    hit_clip: Rect,
) {
    let bounds = Rect::new(origin.x, origin.y, lay.width, lay.height);
    let stroke = if card.urgency == Urgency::Critical {
        fills::CRITICAL_OUTLINE
    } else {
        fills::CARD_OUTLINE
    };
    scene.push(Node::RoundRect {
        rect: bounds,
        radius: card_radius(lay.height),
        fill: Some(if opts.hovered {
            fills::CARD_HOVER
        } else {
            fills::CARD
        }),
        stroke: Some(stroke),
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
    opts: &CardOpts,
    hit_clip: Rect,
) {
    let at = |r: Rect| r.translate(origin.x, origin.y);
    let bounds = Rect::new(origin.x, origin.y, lay.width, lay.height);
    let letter = fallback_letter(&card.app_name);
    scene.push(Node::Image {
        rect: at(lay.icon),
        radius: m::ICON_RADIUS,
        image: card.icon.clone().or_else(|| card.image.clone()),
        fallback: letter,
    });
    if let Some(thumb) = lay.thumb {
        scene.push(Node::Image {
            rect: at(thumb),
            radius: 8.0,
            image: card.image.clone(),
            fallback: letter,
        });
    }
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

    let radius = card_radius(lay.height);
    add_hit(scene, bounds, radius, opts.body_target.clone(), hit_clip);
    for (r, f, key) in &lay.actions {
        let target = HitTarget::Action {
            id: card.id,
            key: key.clone(),
        };
        let hovered = opts.hovered_target.as_ref() == Some(&target);
        pill(scene, at(*r), f, hovered, target, hit_clip);
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
    rect: Rect,
    label: &TextFit,
    hovered: bool,
    target: HitTarget,
    clip: Rect,
) {
    let radius = rect.h / 2.0;
    scene.push(Node::RoundRect {
        rect,
        radius,
        fill: Some(if hovered {
            fills::PILL_HOVER
        } else {
            fills::PILL
        }),
        stroke: None,
    });
    let lh = TextStyle::Button.line_height();
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
    let g = radius * 0.8;
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
}
