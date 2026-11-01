//! Pure layout: `(ViewModel, size, scale, hover) -> Scene`.
//!
//! Nothing here touches Wayland or pixels. Text is measured through the
//! [`TextMeasure`] seam, so tests run with a deterministic fake.

pub mod card;
pub mod toast;

use tidings_core::view::{CardView, GroupView, ViewModel};

use crate::geom::{Point, Rect, Size, snap};
use crate::scene::{Align, GlyphKind, HitTarget, Node, Scene};
use crate::style::{Fill, Role, TextStyle, fills, metrics as m};
use crate::text::TextMeasure;
use card::{CardOpts, add_hit, close_button, emit_card, measure_card, pill};
pub use toast::{ToastLayout, layout_toast, stack_offsets};

/// Interaction state that affects layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Interaction {
    /// Target under the pointer.
    pub hovered: Option<HitTarget>,
    /// Vertical scroll offset of the panel list, in logical px.
    pub scroll: f32,
}

/// Panel layout result.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelLayout {
    pub scene: Scene,
    /// Largest valid scroll offset for the current content.
    pub max_scroll: f32,
    /// The scroll offset actually used (clamped).
    pub scroll: f32,
}

/// Lays out the panel sheet. `size` is the panel's glass rect (380 x height);
/// the scene's origin is the sheet's top-left corner.
pub fn layout_panel(
    view: &ViewModel,
    size: Size,
    scale: f32,
    ui: &Interaction,
    meas: &mut dyn TextMeasure,
) -> PanelLayout {
    let mut scene = Scene::new(size);
    let sheet = Rect::new(0.0, 0.0, size.w, size.h);
    scene.push(Node::RoundRect {
        rect: sheet,
        radius: m::PANEL_RADIUS,
        fill: Some(fills::GLASS),
        stroke: Some(fills::OUTLINE),
    });
    scene.hit(sheet, m::PANEL_RADIUS, HitTarget::Background);
    header(&mut scene, view, size, ui, meas);

    // The list viewport starts a little above the first card so a hovered
    // card's overhanging "×" is not clipped.
    let top_slack = 8.0;
    let viewport = Rect::new(
        0.0,
        m::HEADER_HEIGHT - top_slack,
        size.w,
        (size.h - m::HEADER_HEIGHT + top_slack - m::PANEL_PADDING).max(0.0),
    );
    let col_w = size.w - 2.0 * m::PANEL_PADDING;

    if view.groups.is_empty() {
        let fit = meas.fit("No Notifications", TextStyle::Empty, col_w, 1);
        let h = TextStyle::Empty.line_height();
        scene.push(Node::Text {
            rect: Rect::new(
                m::PANEL_PADDING,
                snap(
                    viewport.y + (viewport.h - h) / 2.0 - m::HEADER_HEIGHT / 2.0,
                    scale,
                ),
                col_w,
                h,
            ),
            text: fit.text,
            style: TextStyle::Empty,
            fill: Fill::solid(Role::OnSurfaceVariant),
            align: Align::Center,
            wrap_width: col_w,
        });
        return PanelLayout {
            scene,
            max_scroll: 0.0,
            scroll: 0.0,
        };
    }

    // Measure everything first to know the content height and clamp scroll.
    let groups: Vec<GroupLayout> = view
        .groups
        .iter()
        .map(|g| GroupLayout::measure(g, col_w, scale, meas))
        .collect();
    let gaps = crate::geom::u32_to_f32(u32::try_from(groups.len().saturating_sub(1)).unwrap_or(0));
    let content_h = top_slack + groups.iter().map(|g| g.height).sum::<f32>() + gaps * m::CARD_GAP;
    let max_scroll = (content_h - viewport.h).max(0.0);
    let scroll = snap(ui.scroll.clamp(0.0, max_scroll), scale);

    scene.push(Node::PushClip(viewport));
    let mut y = viewport.y + top_slack - scroll;
    for (g, lay) in view.groups.iter().zip(&groups) {
        // Skip groups entirely outside the viewport (cheap culling).
        let bounds = Rect::new(m::PANEL_PADDING, y, col_w, lay.height);
        if bounds
            .outset(m::CLOSE_RADIUS)
            .intersect(&viewport)
            .is_some()
        {
            lay.emit(&mut scene, g, Point::new(m::PANEL_PADDING, y), ui, viewport);
        }
        y += lay.height + m::CARD_GAP;
    }
    scene.push(Node::PopClip);
    PanelLayout {
        scene,
        max_scroll,
        scroll,
    }
}

fn header(
    scene: &mut Scene,
    view: &ViewModel,
    size: Size,
    ui: &Interaction,
    meas: &mut dyn TextMeasure,
) {
    let row_cy = m::HEADER_HEIGHT / 2.0 + 2.0;
    let full = Rect::new(0.0, 0.0, size.w, size.h);
    let right = size.w - m::PANEL_PADDING - 4.0;
    let pill_y = row_cy - m::PILL_HEIGHT / 2.0;
    let mut x = right;

    if !view.groups.is_empty() {
        let fit = meas.fit("Clear all", TextStyle::Button, 200.0, 1);
        let w = (fit.width + 2.0 * m::PILL_PAD_X).ceil();
        x -= w;
        let hovered = ui.hovered == Some(HitTarget::ClearAll);
        pill(
            scene,
            Rect::new(x, pill_y, w, m::PILL_HEIGHT),
            &fit,
            hovered,
            HitTarget::ClearAll,
            full,
        );
        x -= m::ACTION_GAP;
    }

    // Do Not Disturb: a moon pill, filled with the accent colour when on.
    let w = 40.0;
    x -= w;
    let dnd = Rect::new(x, pill_y, w, m::PILL_HEIGHT);
    let hovered = ui.hovered == Some(HitTarget::ToggleDnd);
    let dnd_fill = match (view.dnd, hovered) {
        (true, _) => fills::PILL_ON,
        (false, true) => fills::PILL_HOVER,
        (false, false) => fills::PILL,
    };
    scene.push(Node::RoundRect {
        rect: dnd,
        radius: m::PILL_HEIGHT / 2.0,
        fill: Some(dnd_fill),
        stroke: None,
    });
    let g = 14.0;
    scene.push(Node::Glyph {
        rect: Rect::new(dnd.center().x - g / 2.0, dnd.center().y - g / 2.0, g, g),
        kind: GlyphKind::Moon,
        fill: Fill::solid(if view.dnd {
            Role::OnPrimary
        } else {
            Role::OnSurfaceVariant
        }),
    });
    add_hit(scene, dnd, m::PILL_HEIGHT / 2.0, HitTarget::ToggleDnd, full);

    let title_x = m::PANEL_PADDING + 8.0;
    let title_w = (x - 8.0 - title_x).max(1.0);
    let fit = meas.fit("Notifications", TextStyle::Title, title_w, 1);
    let lh = TextStyle::Title.line_height();
    scene.push(Node::Text {
        rect: Rect::new(title_x, row_cy - lh / 2.0, title_w, lh),
        text: fit.text,
        style: TextStyle::Title,
        fill: Fill::solid(Role::OnSurface),
        align: Align::Left,
        wrap_width: title_w,
    });
}

/// A measured group in one of its three presentations.
#[derive(Debug, Clone)]
enum GroupShape {
    Single(card::CardLayout),
    /// Collapsed stack: the top card plus `sheets` peeking below.
    Stack {
        top: card::CardLayout,
        sheets: usize,
    },
    /// Header row followed by every card.
    Expanded {
        title: crate::text::TextFit,
        less: crate::text::TextFit,
        cards: Vec<card::CardLayout>,
    },
}

#[derive(Debug, Clone)]
struct GroupLayout {
    shape: GroupShape,
    height: f32,
}

impl GroupLayout {
    fn measure(g: &GroupView, w: f32, scale: f32, meas: &mut dyn TextMeasure) -> Self {
        let lines = if g.expanded {
            m::BODY_LINES_EXPANDED
        } else {
            m::BODY_LINES_COLLAPSED
        };
        match g.cards.as_slice() {
            [] => Self {
                shape: GroupShape::Expanded {
                    title: meas.fit(&g.app_name, TextStyle::GroupTitle, w, 1),
                    less: meas.fit("Show less", TextStyle::Button, w, 1),
                    cards: Vec::new(),
                },
                height: 0.0,
            },
            [one] => {
                let c = measure_card(one, w, lines, scale, meas);
                Self {
                    height: c.height,
                    shape: GroupShape::Single(c),
                }
            }
            [top, rest @ ..] if !g.expanded => {
                let c = measure_card(top, w, lines, scale, meas);
                let sheets = rest.len().min(m::MAX_SHEETS);
                Self {
                    height: c.height
                        + m::SHEET_PEEK
                            * crate::geom::u32_to_f32(u32::try_from(sheets).unwrap_or(0)),
                    shape: GroupShape::Stack { top: c, sheets },
                }
            }
            cards => {
                let cards: Vec<_> = cards
                    .iter()
                    .map(|c| measure_card(c, w, lines, scale, meas))
                    .collect();
                let gaps = crate::geom::u32_to_f32(u32::try_from(cards.len()).unwrap_or(0));
                let height = m::GROUP_HEADER_HEIGHT
                    + cards.iter().map(|c| c.height).sum::<f32>()
                    + gaps * m::ACTION_GAP;
                Self {
                    shape: GroupShape::Expanded {
                        title: meas.fit(&g.app_name, TextStyle::GroupTitle, w - 140.0, 1),
                        less: meas.fit("Show less", TextStyle::Button, 120.0, 1),
                        cards,
                    },
                    height,
                }
            }
        }
    }

    fn emit(&self, scene: &mut Scene, g: &GroupView, origin: Point, ui: &Interaction, clip: Rect) {
        let card_hovered =
            |c: &CardView| ui.hovered.as_ref().and_then(HitTarget::card) == Some(c.id);
        match &self.shape {
            GroupShape::Single(lay) => {
                let Some(c) = g.cards.first() else { return };
                let opts = CardOpts {
                    body_target: HitTarget::Activate(c.id),
                    close_target: HitTarget::Dismiss(c.id),
                    hovered: card_hovered(c),
                    hovered_target: ui.hovered.clone(),
                };
                emit_card(scene, c, lay, origin, &opts, clip);
            }
            GroupShape::Stack { top, sheets } => {
                let Some(c) = g.cards.first() else { return };
                let expand = HitTarget::ExpandGroup(g.key.clone());
                let clear = HitTarget::ClearGroup(g.key.clone());
                let hovered = ui
                    .hovered
                    .as_ref()
                    .is_some_and(|h| *h == expand || *h == clear);

                emit_sheets(scene, origin, top, *sheets, &expand, clip);
                let opts = CardOpts {
                    body_target: expand,
                    close_target: clear,
                    hovered,
                    hovered_target: ui.hovered.clone(),
                };
                emit_card(scene, c, top, origin, &opts, clip);
            }
            GroupShape::Expanded { title, less, cards } => {
                if cards.is_empty() {
                    return;
                }
                let w = cards.first().map_or(0.0, |c| c.width);
                let row_cy = origin.y + m::GROUP_HEADER_HEIGHT / 2.0 - 3.0;
                let lh = TextStyle::GroupTitle.line_height();
                scene.push(Node::Text {
                    rect: Rect::new(origin.x + 6.0, row_cy - lh / 2.0, w - 140.0, lh),
                    text: title.text.clone(),
                    style: TextStyle::GroupTitle,
                    fill: Fill::solid(Role::OnSurface),
                    align: Align::Left,
                    wrap_width: w - 140.0,
                });
                let close_r = 12.0;
                let close_c = Point::new(origin.x + w - close_r - 2.0, row_cy);
                let clear = HitTarget::ClearGroup(g.key.clone());
                close_button(
                    scene,
                    close_c,
                    close_r,
                    clear.clone(),
                    ui.hovered.as_ref() == Some(&clear),
                    clip,
                );
                let pill_h = 24.0;
                let pw = (less.width + 2.0 * m::PILL_PAD_X).ceil();
                let collapse = HitTarget::CollapseGroup(g.key.clone());
                pill(
                    scene,
                    Rect::new(
                        close_c.x - close_r - 8.0 - pw,
                        row_cy - pill_h / 2.0,
                        pw,
                        pill_h,
                    ),
                    less,
                    ui.hovered.as_ref() == Some(&collapse),
                    collapse,
                    clip,
                );
                let mut y = origin.y + m::GROUP_HEADER_HEIGHT;
                for (c, lay) in g.cards.iter().zip(cards) {
                    let opts = CardOpts {
                        body_target: HitTarget::Activate(c.id),
                        close_target: HitTarget::Dismiss(c.id),
                        hovered: card_hovered(c),
                        hovered_target: ui.hovered.clone(),
                    };
                    emit_card(scene, c, lay, Point::new(origin.x, y), &opts, clip);
                    y += lay.height + m::ACTION_GAP;
                }
            }
        }
    }
}

/// Paints the "sheets" peeking below a collapsed stack, deepest first.
fn emit_sheets(
    scene: &mut Scene,
    origin: Point,
    top: &card::CardLayout,
    sheets: usize,
    expand: &HitTarget,
    clip: Rect,
) {
    let card_rect = Rect::new(origin.x, origin.y, top.width, top.height);
    let radius = card::card_radius(top.height);
    for i in (1..=sheets).rev() {
        let level = crate::geom::u32_to_f32(u32::try_from(i).unwrap_or(1));
        let inset = m::SHEET_INSET * level;
        let rect = Rect::new(
            origin.x + inset,
            origin.y + top.height - 2.0 * radius,
            top.width - 2.0 * inset,
            2.0 * radius + m::SHEET_PEEK * level,
        );
        scene.push(Node::Sheet {
            rect,
            radius: radius - 2.0 * level,
            fill: if i == 1 {
                fills::SHEET_1
            } else {
                fills::SHEET_2
            },
            stroke: Some(fills::CARD_OUTLINE),
            cutout: card_rect,
            cutout_radius: radius,
        });
        add_hit(scene, rect, radius, expand.clone(), clip);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::SystemTime;

    use tidings_core::model::{Action, NotificationId, Urgency};

    use super::*;
    use crate::text::MonoMeasure;

    pub fn sample_card(id: u32, app: &str) -> CardView {
        CardView {
            id: NotificationId(id),
            app_name: app.into(),
            icon: None,
            image: None,
            summary: format!("Summary {id}"),
            body: "Body text that is reasonably long to wrap onto another line".into(),
            time_label: "now".into(),
            actions: vec![
                Action {
                    key: "reply".into(),
                    label: "Reply".into(),
                },
                Action {
                    key: "archive".into(),
                    label: "Archive".into(),
                },
            ],
            clickable: true,
            urgency: Urgency::Normal,
        }
    }

    fn group(key: &str, ids: &[u32], expanded: bool) -> GroupView {
        GroupView {
            key: key.into(),
            app_name: key.into(),
            icon: None,
            cards: ids.iter().map(|&i| sample_card(i, key)).collect(),
            expanded,
        }
    }

    fn view(groups: Vec<GroupView>) -> ViewModel {
        let _ = SystemTime::now();
        ViewModel {
            groups,
            toasts: Vec::new(),
            dnd: false,
        }
    }

    const SIZE: Size = Size::new(380.0, 900.0);

    fn targets(scene: &Scene) -> Vec<HitTarget> {
        scene.hits.iter().map(|h| h.target.clone()).collect()
    }

    fn rect_of(scene: &Scene, t: &HitTarget) -> Rect {
        scene
            .hits
            .iter()
            .find(|h| &h.target == t)
            .map_or_else(|| panic!("no hit for {t:?}"), |h| h.rect)
    }

    #[test]
    fn empty_state_has_message_and_no_clear_all() {
        let lay = layout_panel(
            &view(vec![]),
            SIZE,
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        assert!(
            lay.scene
                .nodes
                .iter()
                .any(|n| matches!(n, Node::Text { text, .. } if text == "No Notifications"))
        );
        assert!(!targets(&lay.scene).contains(&HitTarget::ClearAll));
        assert!(targets(&lay.scene).contains(&HitTarget::ToggleDnd));
    }

    #[test]
    fn header_hits_resolve() {
        let lay = layout_panel(
            &view(vec![group("a", &[1], false)]),
            SIZE,
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        let s = &lay.scene;
        let clear = rect_of(s, &HitTarget::ClearAll);
        assert_eq!(s.hit_test(clear.center()), Some(&HitTarget::ClearAll));
        let dnd = rect_of(s, &HitTarget::ToggleDnd);
        assert!(dnd.right() < clear.x, "DND sits left of Clear all");
        assert_eq!(
            s.hit_test(Point::new(30.0, 8.0)),
            Some(&HitTarget::Background)
        );
    }

    #[test]
    fn single_card_click_activates_and_actions_hit() {
        let lay = layout_panel(
            &view(vec![group("a", &[1], false)]),
            SIZE,
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        let s = &lay.scene;
        let card = rect_of(s, &HitTarget::Activate(NotificationId(1)));
        assert_eq!(
            s.hit_test(Point::new(card.x + 60.0, card.y + 20.0)),
            Some(&HitTarget::Activate(NotificationId(1)))
        );
        let reply = HitTarget::Action {
            id: NotificationId(1),
            key: "reply".into(),
        };
        assert_eq!(s.hit_test(rect_of(s, &reply).center()), Some(&reply));
        // No close button until hovered.
        assert!(!targets(s).contains(&HitTarget::Dismiss(NotificationId(1))));
    }

    #[test]
    fn hover_reveals_close_button_on_top() {
        let ui = Interaction {
            hovered: Some(HitTarget::Activate(NotificationId(1))),
            scroll: 0.0,
        };
        let lay = layout_panel(
            &view(vec![group("a", &[1], false)]),
            SIZE,
            1.0,
            &ui,
            &mut MonoMeasure,
        );
        let s = &lay.scene;
        let card = rect_of(s, &HitTarget::Activate(NotificationId(1)));
        let close = rect_of(s, &HitTarget::Dismiss(NotificationId(1)));
        // The "×" sits in the card's top-left corner, its centre inside the
        // pill's rounded corner, and wins the hit test over the card body.
        let c = close.center();
        let r = card::card_radius(card.h);
        let corner = Point::new(card.x + r, card.y + r);
        assert!((c.x - corner.x).hypot(c.y - corner.y) < r);
        assert_eq!(
            s.hit_test(close.center()),
            Some(&HitTarget::Dismiss(NotificationId(1)))
        );
    }

    #[test]
    fn collapsed_stack_expands_and_shows_sheets() {
        let lay = layout_panel(
            &view(vec![group("chat", &[3, 2, 1], false)]),
            SIZE,
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        let s = &lay.scene;
        let sheets = s
            .nodes
            .iter()
            .filter(|n| matches!(n, Node::Sheet { .. }))
            .count();
        assert_eq!(sheets, 2);
        let expand = HitTarget::ExpandGroup("chat".into());
        let top = s
            .hits
            .iter()
            .rev()
            .find(|h| h.target == expand)
            .map(|h| h.rect);
        let top = top.unwrap_or_default();
        assert_eq!(
            s.hit_test(Point::new(top.x + 100.0, top.y + 20.0)),
            Some(&expand)
        );
        // Lower cards are not individually clickable while collapsed.
        assert!(!targets(s).contains(&HitTarget::Activate(NotificationId(2))));
        // The peeking sheets also expand.
        let sheet_hit = s
            .hits
            .iter()
            .find(|h| h.target == expand)
            .map(|h| h.rect)
            .unwrap_or_default();
        assert_eq!(
            s.hit_test(Point::new(sheet_hit.center().x, sheet_hit.bottom() - 2.0)),
            Some(&expand)
        );
    }

    #[test]
    fn two_card_stack_has_one_sheet() {
        let lay = layout_panel(
            &view(vec![group("chat", &[2, 1], false)]),
            SIZE,
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        assert_eq!(
            lay.scene
                .nodes
                .iter()
                .filter(|n| matches!(n, Node::Sheet { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn expanded_group_has_header_controls_and_all_cards() {
        let lay = layout_panel(
            &view(vec![group("chat", &[3, 2, 1], true)]),
            SIZE,
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        let t = targets(&lay.scene);
        assert!(t.contains(&HitTarget::CollapseGroup("chat".into())));
        assert!(t.contains(&HitTarget::ClearGroup("chat".into())));
        for id in [1, 2, 3] {
            assert!(t.contains(&HitTarget::Activate(NotificationId(id))));
        }
        let a3 = rect_of(&lay.scene, &HitTarget::Activate(NotificationId(3)));
        let a2 = rect_of(&lay.scene, &HitTarget::Activate(NotificationId(2)));
        assert!(a2.y > a3.bottom(), "cards stack downwards in order");
    }

    #[test]
    fn groups_stack_with_gap_newest_first() {
        let lay = layout_panel(
            &view(vec![group("a", &[2], false), group("b", &[1], false)]),
            SIZE,
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        let a = rect_of(&lay.scene, &HitTarget::Activate(NotificationId(2)));
        let b = rect_of(&lay.scene, &HitTarget::Activate(NotificationId(1)));
        assert!((b.y - a.bottom() - m::CARD_GAP).abs() < 0.01);
        assert!((a.x - m::PANEL_PADDING).abs() < 0.01);
        assert!((a.w - (380.0 - 2.0 * m::PANEL_PADDING)).abs() < 0.01);
    }

    #[test]
    fn overflow_scrolls_and_clips_hits() {
        let groups: Vec<_> = (0..20)
            .map(|i| group(&format!("g{i}"), &[i + 1], false))
            .collect();
        let v = view(groups);
        let lay = layout_panel(
            &v,
            Size::new(380.0, 600.0),
            1.0,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        assert!(lay.max_scroll > 0.0);
        // Every hit stays inside the panel.
        assert!(
            lay.scene
                .hits
                .iter()
                .all(|h| h.rect.bottom() <= 600.0 + 0.01)
        );
        // Scrolling past the end clamps.
        let ui = Interaction {
            hovered: None,
            scroll: 1e6,
        };
        let end = layout_panel(&v, Size::new(380.0, 600.0), 1.0, &ui, &mut MonoMeasure);
        assert!((end.scroll - end.max_scroll).abs() < 0.01);
        assert!(targets(&end.scene).contains(&HitTarget::Activate(NotificationId(20))));
        assert!(!targets(&end.scene).contains(&HitTarget::Activate(NotificationId(1))));
    }

    #[test]
    fn layout_snaps_to_device_pixels() {
        let lay = layout_panel(
            &view(vec![group("a", &[1], false)]),
            SIZE,
            1.6,
            &Interaction::default(),
            &mut MonoMeasure,
        );
        let card = rect_of(&lay.scene, &HitTarget::Activate(NotificationId(1)));
        let h = card.h * 1.6;
        assert!((h - h.round()).abs() < 1e-3);
    }
}
