//! Toast layout: a compact one-line pill, or (expanded) the full card.
//!
//! A collapsed toast is always a [`m::TOAST_HEIGHT`] pill: round icon,
//! one-line summary over one line of body, then the time and a round expand
//! (⌄) button. Hovering tints the glass and reveals a "×" over the icon.
//! An expanded toast is laid out like a panel card, with a collapse (⌃)
//! button in its top-right corner. Both float on their own glass with a soft
//! drop shadow, all styled by the theme (see [`Style`]).

use tidings_core::model::{NotificationId, Urgency};
use tidings_core::view::{CardView, ToastView};

use crate::compose::ReplyDraft;
use crate::geom::{Point, Rect, Size, snap};
use crate::scene::{Align, GlyphKind, HitTarget, Node, Scene};
use crate::style::{Fill, Role, Style, TextStyle, fills, metrics as m};
use crate::text::{TextFit, TextMeasure};

use super::card::{
    CardOpts, CardSpec, IconAlign, add_hit, close_button, emit_avatar, emit_card_content,
    measure_card_with,
};

/// One laid-out toast: a scene sized to the toast plus [`m::TOAST_INSET`] on
/// every side (room for the shadow and the overhanging "×").
#[derive(Debug, Clone, PartialEq)]
pub struct ToastLayout {
    pub id: NotificationId,
    pub scene: Scene,
    /// The toast's own rect within `scene`.
    pub card: Rect,
}

/// Lays out one toast. `hovered` is the target under the pointer, if any;
/// `draft` the reply being typed, which shows on an expanded toast.
pub fn layout_toast(
    toast: &ToastView,
    scale: f32,
    hovered: Option<&HitTarget>,
    draft: Option<&ReplyDraft>,
    style: &Style,
    meas: &mut dyn TextMeasure,
) -> ToastLayout {
    let c = &toast.card;
    let is_hovered = hovered.and_then(HitTarget::card) == Some(c.id);
    let inset = m::TOAST_INSET;
    let origin = Point::new(inset, inset);
    let (height, shape) = if toast.expanded {
        let spec = expanded_spec();
        let lay = measure_card_with(c, spec, draft, scale, meas);
        (lay.height, Shape::Expanded(Box::new(lay)))
    } else {
        let lay = measure_compact(c, scale, meas);
        (m::TOAST_HEIGHT, Shape::Compact(lay))
    };
    let size = Size::new(m::TOAST_WIDTH + 2.0 * inset, height + 2.0 * inset);
    let bounds = Rect::new(inset, inset, m::TOAST_WIDTH, height);
    let mut scene = Scene::new(size);
    let clip = Rect::new(0.0, 0.0, size.w, size.h);
    let radius = style.toast_radius(height, toast.expanded);
    glass(
        &mut scene,
        style,
        c,
        bounds,
        radius,
        toast.expanded,
        is_hovered,
    );

    let opts = CardOpts {
        style,
        body_target: HitTarget::Activate(c.id),
        close_target: HitTarget::Dismiss(c.id),
        hovered: is_hovered,
        hovered_target: hovered.cloned(),
    };
    match &shape {
        Shape::Compact(lay) => {
            emit_compact(&mut scene, c, lay, origin, &opts, clip);
            chevron_button(
                &mut scene,
                style,
                lay.expand.translate(origin.x, origin.y),
                &ChevronSpec {
                    kind: GlyphKind::ChevronDown,
                    target: HitTarget::ExpandToast(c.id),
                    dot: !c.actions.is_empty() || c.reply.is_some(),
                },
                hovered,
                clip,
            );
        }
        Shape::Expanded(lay) => {
            emit_card_content(&mut scene, c, lay, origin, &opts, clip);
            let b = m::CHEVRON_BUTTON;
            let corner = m::CHEVRON_CORNER_INSET;
            chevron_button(
                &mut scene,
                style,
                Rect::new(bounds.right() - corner - b, bounds.y + corner, b, b),
                &ChevronSpec {
                    kind: GlyphKind::ChevronUp,
                    target: HitTarget::CollapseToast(c.id),
                    dot: false,
                },
                hovered,
                clip,
            );
        }
    }
    if is_hovered {
        let off = m::TOAST_CLOSE_OFFSET;
        close_button(
            &mut scene,
            Point::new(bounds.x + off, bounds.y + off),
            m::TOAST_CLOSE_RADIUS,
            opts.close_target.clone(),
            hovered == Some(&opts.close_target),
            clip,
        );
    }
    ToastLayout {
        id: c.id,
        scene,
        card: bounds,
    }
}

/// How an expanded toast's card is measured: panel-card style, a longer body,
/// the icon level with the app-name row, a little more room under the
/// actions, and the top-right corner kept clear for the collapse button.
#[must_use]
pub fn expanded_spec() -> CardSpec {
    CardSpec {
        width: m::TOAST_WIDTH,
        body_lines: m::BODY_LINES_TOAST_EXPANDED,
        corner_reserve: m::CHEVRON_CORNER_INSET + m::CHEVRON_BUTTON,
        icon_align: IconAlign::Top,
        pad_bottom: m::TOAST_EXPANDED_PAD_BOTTOM,
    }
}

/// Vertical offsets (scene top, in toast-surface coordinates) for a stack of
/// toast scenes of the given heights, newest first.
#[must_use]
pub fn stack_offsets(heights: &[f32]) -> Vec<f32> {
    let mut y = 0.0;
    heights
        .iter()
        .map(|h| {
            let top = y;
            // Scenes include the inset on both sides; toasts sit TOAST_GAP apart.
            y += h - 2.0 * m::TOAST_INSET + m::TOAST_GAP;
            top
        })
        .collect()
}

#[derive(Debug, Clone)]
enum Shape {
    Compact(CompactLayout),
    Expanded(Box<super::card::CardLayout>),
}

/// A measured collapsed toast, positioned relative to its top-left corner.
#[derive(Debug, Clone)]
struct CompactLayout {
    icon: Rect,
    summary: Option<(Rect, TextFit)>,
    body: Option<(Rect, TextFit)>,
    /// The time label, vertically centred left of the expand button.
    time: Option<(Rect, TextFit)>,
    expand: Rect,
}

/// Lays out the pill row: icon, text column, time, expand button, each
/// [`m::TOAST_GAP_X`] apart, all vertically centred. The text column takes
/// what is left and ellipsizes.
fn measure_compact(card: &CardView, scale: f32, meas: &mut dyn TextMeasure) -> CompactLayout {
    let h = m::TOAST_HEIGHT;
    let icon_inset = m::TOAST_ICON_INSET;
    let icon = Rect::new(
        icon_inset,
        icon_inset,
        m::TOAST_ICON_SIZE,
        m::TOAST_ICON_SIZE,
    );
    let text_x = icon.right() + m::TOAST_GAP_X;
    let button = m::CHEVRON_BUTTON;
    let expand = Rect::new(
        m::TOAST_WIDTH - m::TOAST_PAD_RIGHT - button,
        snap((h - button) / 2.0, scale),
        button,
        button,
    );
    let mut text_right = expand.x - m::TOAST_GAP_X;

    let time = (!card.time_label.trim().is_empty()).then(|| {
        let max_w = (text_right - text_x) / 2.0;
        let fit = meas.fit(&card.time_label, TextStyle::Time, max_w, 1);
        let lh = meas.line_height(TextStyle::Time);
        let r = Rect::new(
            text_right - fit.width,
            snap((h - lh) / 2.0, scale),
            fit.width,
            lh,
        );
        text_right = r.x - m::TOAST_GAP_X;
        (r, fit)
    });

    let has_summary = !card.summary.trim().is_empty();
    let has_body = !card.body.trim().is_empty();
    let sum_h = meas.line_height(TextStyle::SummaryLine);
    let body_h = meas.line_height(TextStyle::BodyLine);
    let block = match (has_summary, has_body) {
        (true, true) => sum_h + m::TOAST_LINE_GAP + body_h,
        (true, false) => sum_h,
        (false, _) => body_h,
    };
    let w = (text_right - text_x).max(1.0);
    let mut y = snap((h - block) / 2.0, scale);
    let mut line = |text: &str, style: TextStyle, lh: f32| {
        let fit = meas.fit(text, style, w, 1);
        let r = Rect::new(text_x, y, w, lh);
        y += lh + m::TOAST_LINE_GAP;
        (r, fit)
    };
    let summary = has_summary.then(|| line(&card.summary, TextStyle::SummaryLine, sum_h));
    let body = has_body.then(|| line(&card.body, TextStyle::BodyLine, body_h));
    CompactLayout {
        icon,
        summary,
        body,
        time,
        expand,
    }
}

/// The toast's glass: shadow, translucent body, optional sheen, rim.
fn glass(
    scene: &mut Scene,
    style: &Style,
    card: &CardView,
    bounds: Rect,
    radius: f32,
    expanded: bool,
    hovered: bool,
) {
    if let Some(shadow) = style.shadow {
        scene.push(Node::Shadow {
            rect: bounds,
            radius,
            spread: shadow.spread,
            offset_y: shadow.offset_y,
            fill: shadow.fill,
        });
    }
    let body = match (expanded, hovered) {
        (true, _) => style.toast_expanded,
        (false, true) => style.toast_hover,
        (false, false) => style.toast,
    };
    scene.push(Node::RoundRect {
        rect: bounds,
        radius,
        fill: Some(body),
        stroke: None,
    });
    let rim = if card.urgency == Urgency::Critical {
        Some(style.critical)
    } else {
        style.toast_border
    };
    let sheen = style.toast_sheen.filter(|_| expanded || hovered);
    if sheen.is_some() || rim.is_some() {
        scene.push(Node::RoundRect {
            rect: bounds,
            radius,
            fill: sheen,
            stroke: rim,
        });
    }
}

fn emit_compact(
    scene: &mut Scene,
    card: &CardView,
    lay: &CompactLayout,
    origin: Point,
    opts: &CardOpts<'_>,
    clip: Rect,
) {
    let at = |r: Rect| r.translate(origin.x, origin.y);
    emit_avatar(scene, card, at(lay.icon));
    let line = |(r, f): &(Rect, TextFit), style, fill, align| Node::Text {
        rect: at(*r),
        text: f.text.clone(),
        style,
        fill,
        align,
        wrap_width: r.w,
    };
    let muted = Fill::solid(Role::OnSurfaceVariant);
    if let Some(s) = &lay.summary {
        let strong = Fill::solid(Role::OnSurface);
        scene.push(line(s, TextStyle::SummaryLine, strong, Align::Left));
    }
    if let Some(b) = &lay.body {
        scene.push(line(b, TextStyle::BodyLine, muted, Align::Left));
    }
    if let Some(t) = &lay.time {
        scene.push(line(t, TextStyle::Time, muted, Align::Right));
    }
    let bounds = Rect::new(origin.x, origin.y, m::TOAST_WIDTH, m::TOAST_HEIGHT);
    add_hit(
        scene,
        bounds,
        opts.style.toast_radius(m::TOAST_HEIGHT, false),
        opts.body_target.clone(),
        clip,
    );
}

/// What a chevron button shows and does.
struct ChevronSpec {
    kind: GlyphKind,
    target: HitTarget,
    /// Draw the "there are actions inside" dot.
    dot: bool,
}

/// A round expand/collapse button with a stroke chevron.
fn chevron_button(
    scene: &mut Scene,
    style: &Style,
    rect: Rect,
    spec: &ChevronSpec,
    hovered: Option<&HitTarget>,
    clip: Rect,
) {
    let radius = style.button_radius(rect.h);
    let fill = style.button_fill(hovered == Some(&spec.target));
    scene.push(Node::RoundRect {
        rect,
        radius,
        fill: Some(fill),
        stroke: None,
    });
    let g = m::CHEVRON_ICON;
    let c = rect.center();
    scene.push(Node::Glyph {
        rect: Rect::new(c.x - g / 2.0, c.y - g / 2.0, g, g),
        kind: spec.kind,
        fill: Fill::solid(Role::OnSurface),
    });
    if spec.dot {
        let r = m::ACTIONS_DOT_RADIUS;
        scene.push(Node::Circle {
            center: Point::new(rect.right() - r, rect.y + r),
            radius: r,
            fill: fills::ACTIONS_DOT,
            stroke: None,
        });
    }
    add_hit(scene, rect, radius, spec.target.clone(), clip);
}

#[cfg(test)]
mod tests {
    use tidings_core::ui::UiEvent;

    use super::*;
    use crate::layout::tests::sample_card;
    use crate::text::MonoMeasure;

    const ID: NotificationId = NotificationId(9);

    fn toast(card: CardView, expanded: bool) -> ToastView {
        ToastView {
            card,
            paused: expanded,
            expanded,
        }
    }

    fn lay(t: &ToastView, hovered: Option<&HitTarget>) -> ToastLayout {
        layout_toast(t, 1.0, hovered, None, &Style::default(), &mut MonoMeasure)
    }

    fn hovered() -> HitTarget {
        HitTarget::Activate(ID)
    }

    fn hit_rect(s: &Scene, t: &HitTarget) -> Option<Rect> {
        s.hits.iter().find(|h| &h.target == t).map(|h| h.rect)
    }

    fn texts(s: &Scene, style: TextStyle) -> Vec<(Rect, String)> {
        s.nodes
            .iter()
            .filter_map(|n| match n {
                Node::Text {
                    rect,
                    text,
                    style: st,
                    ..
                } if *st == style => Some((*rect, text.clone())),
                _ => None,
            })
            .collect()
    }

    /// Radius of the toast body (the first fill after the shadow).
    fn body_radius(s: &Scene) -> Option<f32> {
        s.nodes.iter().find_map(|n| match n {
            Node::RoundRect {
                radius,
                fill: Some(f),
                ..
            } if f.role == Role::Surface => Some(*radius),
            _ => None,
        })
    }

    #[test]
    fn compact_toast_is_a_fixed_pill_whatever_the_text() {
        for body in ["", "short", &"long words ".repeat(60), "a\nb\nc\nd"] {
            let mut c = sample_card(9, "Mail");
            c.body = body.into();
            for h in [None, Some(hovered())] {
                let t = lay(&toast(c.clone(), false), h.as_ref());
                assert!((t.card.h - 58.0).abs() < f32::EPSILON, "{body:?}");
                assert!((t.card.w - m::TOAST_WIDTH).abs() < f32::EPSILON);
                assert_eq!(body_radius(&t.scene), Some(29.0));
                assert!((t.scene.size.h - 58.0 - 2.0 * m::TOAST_INSET).abs() < f32::EPSILON);
            }
        }
    }

    #[test]
    fn compact_text_is_one_ellipsized_line_each() {
        let mut c = sample_card(9, "Mail");
        c.summary = "A very long summary line that cannot possibly fit".repeat(3);
        c.body = "First paragraph of the body\nsecond paragraph ".repeat(10);
        let t = lay(&toast(c, false), None);
        let sum = texts(&t.scene, TextStyle::SummaryLine);
        let body = texts(&t.scene, TextStyle::BodyLine);
        assert_eq!((sum.len(), body.len()), (1, 1));
        for (r, text) in sum.iter().chain(&body) {
            assert!(text.ends_with('…'), "{text}");
            assert!(!text.contains('\n'));
            assert!(r.right() <= t.card.right() - m::TOAST_PAD_RIGHT + 0.01);
        }
        assert!(
            (sum[0].0.h - MonoMeasure.line_height(TextStyle::SummaryLine)).abs() < f32::EPSILON
        );
        assert!(body[0].0.y > sum[0].0.y, "body under the summary");
        // The icon is a 40 px circle, vertically centred, 9 px in.
        let icon = t.scene.nodes.iter().find_map(|n| match n {
            Node::Image { rect, radius, .. } => Some((*rect, *radius)),
            _ => None,
        });
        let (icon, r) = icon.unwrap_or_else(|| panic!("no icon"));
        assert_eq!(icon.w, 40.0);
        assert_eq!(r, 20.0);
        assert!((icon.x - t.card.x - 9.0).abs() < f32::EPSILON);
        assert!((icon.center().y - t.card.center().y).abs() < 0.01);
    }

    #[test]
    fn at_rest_time_and_expand_button_sit_right_of_the_text() {
        let t = lay(&toast(sample_card(9, "Mail"), false), None);
        let s = &t.scene;
        let expand = hit_rect(s, &HitTarget::ExpandToast(ID));
        let expand = expand.unwrap_or_else(|| panic!("no expand button at rest"));
        assert_eq!((expand.w, expand.h), (28.0, 28.0));
        assert!((expand.center().y - t.card.center().y).abs() < 0.01);
        assert!((expand.right() - (t.card.right() - m::TOAST_PAD_RIGHT)).abs() < 0.01);

        let time = texts(s, TextStyle::Time);
        assert_eq!(time.len(), 1);
        assert_eq!(time[0].1, "now");
        let time = time[0].0;
        assert!((time.right() - (expand.x - m::TOAST_GAP_X)).abs() < 0.01);
        assert!((time.center().y - t.card.center().y).abs() < 0.51);
        // Both text lines stop short of the time.
        let sum = texts(s, TextStyle::SummaryLine);
        let body = texts(s, TextStyle::BodyLine);
        for (r, _) in sum.iter().chain(&body) {
            assert!(r.right() <= time.x - m::TOAST_GAP_X + 0.01, "{r:?}");
        }

        assert_eq!(
            s.hit_test(expand.center()).and_then(HitTarget::click_event),
            Some(UiEvent::SetToastExpanded {
                id: ID,
                expanded: true
            })
        );
        assert_eq!(
            s.hit_test(sum[0].0.center())
                .and_then(HitTarget::click_event),
            Some(UiEvent::Activate(ID))
        );
        assert!(
            hit_rect(s, &HitTarget::Dismiss(ID)).is_none(),
            "× only on hover"
        );
    }

    #[test]
    fn a_toast_without_a_time_label_gives_the_text_its_room() {
        let mut c = sample_card(9, "Mail");
        let with = lay(&toast(c.clone(), false), None);
        c.time_label.clear();
        let without = lay(&toast(c, false), None);
        assert_eq!(texts(&without.scene, TextStyle::Time), Vec::new());
        let w = |t: &ToastLayout| texts(&t.scene, TextStyle::SummaryLine)[0].0.w;
        assert!(w(&without) > w(&with));
        assert!(hit_rect(&without.scene, &HitTarget::ExpandToast(ID)).is_some());
    }

    #[test]
    fn hover_adds_close_and_keeps_time_and_expand() {
        let rest = lay(&toast(sample_card(9, "Mail"), false), None);
        let t = lay(&toast(sample_card(9, "Mail"), false), Some(&hovered()));
        let s = &t.scene;
        assert_eq!(
            texts(s, TextStyle::Time),
            texts(&rest.scene, TextStyle::Time)
        );
        assert_eq!(
            hit_rect(s, &HitTarget::ExpandToast(ID)),
            hit_rect(&rest.scene, &HitTarget::ExpandToast(ID))
        );
        assert_ne!(body_fill(s), body_fill(&rest.scene), "hover tint");

        // The "×" overlaps the icon's top-left and wins the hit test.
        let close = hit_rect(s, &HitTarget::Dismiss(ID));
        let close = close.unwrap_or_else(|| panic!("no close button"));
        assert_eq!(close.w, 22.0);
        let icon = Rect::new(t.card.x + 9.0, t.card.y + 9.0, 40.0, 40.0);
        assert!(close.intersect(&icon).is_some());
        assert!(close.x < icon.x && close.y < icon.y);
        assert_eq!(s.hit_test(close.center()), Some(&HitTarget::Dismiss(ID)));
        // The rest of the pill still activates.
        assert_eq!(
            s.hit_test(Point::new(t.card.center().x, t.card.center().y)),
            Some(&HitTarget::Activate(ID))
        );
    }

    /// Fill of the toast body (the first surface fill after the shadow).
    fn body_fill(s: &Scene) -> Option<Fill> {
        s.nodes.iter().find_map(|n| match n {
            Node::RoundRect { fill: Some(f), .. } if f.role == Role::Surface => Some(*f),
            _ => None,
        })
    }

    fn has_dot(s: &Scene) -> bool {
        s.nodes
            .iter()
            .any(|n| matches!(n, Node::Circle { fill, .. } if *fill == fills::ACTIONS_DOT))
    }

    #[test]
    fn actions_dot_only_when_there_are_actions() {
        for h in [None, Some(hovered())] {
            let with = lay(&toast(sample_card(9, "Mail"), false), h.as_ref());
            assert!(has_dot(&with.scene));
            let mut c = sample_card(9, "Mail");
            c.actions.clear();
            let without = lay(&toast(c, false), h.as_ref());
            assert!(!has_dot(&without.scene));
        }
    }
    #[test]
    fn expanded_toast_is_the_card_with_a_corner_collapse_button() {
        let mut c = sample_card(9, "Mail");
        c.body = "word ".repeat(200);
        let t = lay(&toast(c.clone(), true), None);
        let card = measure_card_with(&c, expanded_spec(), None, 1.0, &mut MonoMeasure);
        assert!((t.card.h - card.height).abs() < f32::EPSILON);
        assert!(t.card.h > 58.0);
        assert_eq!(
            body_radius(&t.scene),
            Some(Style::default().toast_radius(t.card.h, true))
        );
        assert_eq!(body_radius(&t.scene), Some(28.0));
        // Full card content: meta row, a 6-line body, action pills.
        assert_eq!(texts(&t.scene, TextStyle::Meta).len(), 2);
        let body = texts(&t.scene, TextStyle::Body);
        let lines = body[0].0.h / MonoMeasure.line_height(TextStyle::Body);
        assert!((lines - 6.0).abs() < 0.01, "{lines}");
        let reply = HitTarget::Action {
            id: ID,
            key: "reply".into(),
        };
        assert!(hit_rect(&t.scene, &reply).is_some());

        let s = &t.scene;
        let collapse = hit_rect(s, &HitTarget::CollapseToast(ID));
        let collapse = collapse.unwrap_or_else(|| panic!("no collapse button"));
        assert_eq!((collapse.w, collapse.h), (28.0, 28.0));
        assert!((collapse.right() - (t.card.right() - 10.0)).abs() < 0.01);
        assert!((collapse.y - (t.card.y + 10.0)).abs() < 0.01);
        assert_eq!(
            s.hit_test(collapse.center())
                .and_then(HitTarget::click_event),
            Some(UiEvent::SetToastExpanded {
                id: ID,
                expanded: false
            })
        );
        // The meta row keeps clear of the button.
        assert!(
            texts(s, TextStyle::Meta)
                .iter()
                .all(|(r, _)| r.right() <= collapse.x)
        );
        assert!(hit_rect(s, &HitTarget::ExpandToast(ID)).is_none());
        assert_eq!(texts(s, TextStyle::SummaryLine).len(), 0);
        assert_eq!(
            s.hit_test(Point::new(t.card.x + 120.0, t.card.center().y)),
            Some(&HitTarget::Activate(ID))
        );
    }

    #[test]
    fn expanded_icon_is_level_with_the_app_name_row() {
        let mut c = sample_card(9, "Mail");
        c.body = "word ".repeat(200);
        let t = lay(&toast(c, true), None);
        let icon = t.scene.nodes.iter().find_map(|n| match n {
            Node::Image { rect, .. } => Some(*rect),
            _ => None,
        });
        let icon = icon.unwrap_or_else(|| panic!("no icon"));
        let meta = texts(&t.scene, TextStyle::Meta);
        assert!((icon.y - meta[0].0.y).abs() < f32::EPSILON);
        assert!((icon.y - (t.card.y + m::CARD_PADDING)).abs() < f32::EPSILON);
        assert!(icon.center().y < t.card.center().y - 20.0, "not centred");
        // A little more room under the actions than on the other sides.
        let pills = t
            .scene
            .hits
            .iter()
            .filter(|h| matches!(h.target, HitTarget::Action { .. }));
        let last = pills.map(|h| h.rect.bottom()).fold(0.0, f32::max);
        assert!((t.card.bottom() - last - m::TOAST_EXPANDED_PAD_BOTTOM).abs() < 0.01);
    }

    #[test]
    fn hovered_expanded_toast_offers_close() {
        let t = lay(&toast(sample_card(9, "Mail"), true), Some(&hovered()));
        assert!(hit_rect(&t.scene, &HitTarget::Dismiss(ID)).is_some());
        assert!(hit_rect(&t.scene, &HitTarget::CollapseToast(ID)).is_some());
    }

    #[test]
    fn critical_keeps_its_outline() {
        let mut c = sample_card(9, "Mail");
        c.urgency = Urgency::Critical;
        let t = lay(&toast(c, false), None);
        assert!(t.scene.nodes.iter().any(
            |n| matches!(n, Node::RoundRect { stroke: Some(s), .. } if *s == Style::default().critical)
        ));
    }

    #[test]
    fn stack_offsets_follow_heights() {
        let offs = stack_offsets(&[100.0, 80.0, 50.0]);
        assert_eq!(offs[0], 0.0);
        assert!((offs[1] - (100.0 - 2.0 * m::TOAST_INSET + m::TOAST_GAP)).abs() < 0.01);
        assert!(offs[2] > offs[1]);
    }

    #[test]
    fn painted_glass_and_shadow_straddle_ignore_alpha() {
        use crate::render::Painter;
        use crate::style::Palette;
        let mut c = sample_card(9, "Mail");
        c.actions.clear();
        let t = lay(&toast(c, false), None);
        let mut painter = Painter::new();
        let pm = painter.paint(&t.scene, &Palette::default(), 1.0);
        let pm = pm.unwrap_or_else(|| panic!("pixmap"));
        let alpha = |x: f32, y: f32| {
            let (x, y) = (crate::geom::f32_to_u32(x), crate::geom::f32_to_u32(y));
            pm.pixel(x, y)
                .map_or(0, tiny_skia::PremultipliedColorU8::alpha)
        };
        let to_u8 = |a: f32| crate::geom::f32_to_u32((a * 255.0).round());
        // Inside the body, clear of text: the glass alpha.
        let body = u32::from(alpha(t.card.right() - 8.0, t.card.center().y));
        assert!(
            body.abs_diff(to_u8(Style::default().toast.alpha)) <= 2,
            "{body}"
        );
        // Just outside: shadow, visible but below the threshold.
        let shadow = u32::from(alpha(t.card.center().x, t.card.bottom() + 2.0));
        assert!(
            shadow > 0
                && shadow <= to_u8(Style::default().shadow.map_or(0.0, |s| s.fill.alpha)) + 1,
            "{shadow}"
        );
        // Far outside: nothing.
        assert_eq!(alpha(1.0, 1.0), 0);
    }
}
