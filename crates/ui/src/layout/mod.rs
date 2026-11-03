//! Pure layout: `(ViewModel, size, scale, hover) -> Scene`.
//!
//! Nothing here touches Wayland or pixels. Text is measured through the
//! [`TextMeasure`] seam, so tests run with a deterministic fake.

pub mod card;
pub mod toast;

use tidings_core::view::{CardView, GroupView, ViewModel};

use crate::compose::ReplyDraft;
use crate::geom::{Point, Rect, Size, snap};
use crate::scene::{Align, GlyphKind, HitTarget, Node, Scene};
use crate::style::{Fill, Role, Style, TextStyle, fills, metrics as m};
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
    /// Groups mid expand/collapse, by key: 0 = stacked, 1 = listed.
    pub morph: Vec<(String, f32)>,
    /// The reply being typed into a card, if any.
    pub reply: Option<ReplyDraft>,
}

impl Interaction {
    fn morph_of(&self, key: &str) -> Option<f32> {
        self.morph
            .iter()
            .find(|(k, _)| k == key)
            .map(|&(_, t)| t.clamp(0.0, 1.0))
    }
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
    style: &Style,
    meas: &mut dyn TextMeasure,
) -> PanelLayout {
    let mut scene = Scene::new(size);
    let sheet = Rect::new(0.0, 0.0, size.w, size.h);
    // The glass is the largest fill; as a sprite it is painted once.
    scene.layer(Point::new(0.0, 0.0), 1.0, scale, sheet, |s, _| {
        s.push(Node::RoundRect {
            rect: sheet,
            radius: style.panel_radius,
            fill: Some(style.glass),
            stroke: style.panel_border,
        });
    });
    scene.hit(sheet, style.panel_radius, HitTarget::Background);
    let full = Rect::new(0.0, 0.0, size.w, size.h);
    scene.layer(Point::new(0.0, 0.0), 1.0, scale, full, |s, _| {
        header(s, view, size, ui, style, meas);
    });

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
        let h = meas.line_height(TextStyle::Empty);
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
        .map(|g| {
            GroupLayout::measure(
                g,
                col_w,
                scale,
                ui.morph_of(&g.key),
                ui.reply.as_ref(),
                meas,
            )
        })
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
            let at = Point::new(m::PANEL_PADDING, y);
            lay.emit(
                &mut scene,
                g,
                at,
                ui,
                Viewport {
                    clip: viewport,
                    scale,
                    style,
                },
            );
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
    style: &Style,
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
            style,
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
        (false, hovered) => style.button_fill(hovered),
    };
    let dnd_radius = style.button_radius(m::PILL_HEIGHT);
    scene.push(Node::RoundRect {
        rect: dnd,
        radius: dnd_radius,
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
    add_hit(scene, dnd, dnd_radius, HitTarget::ToggleDnd, full);

    let title_x = m::PANEL_PADDING + 8.0;
    let title_w = (x - 8.0 - title_x).max(1.0);
    let fit = meas.fit("Notifications", TextStyle::Title, title_w, 1);
    let lh = meas.line_height(TextStyle::Title);
    scene.push(Node::Text {
        rect: Rect::new(title_x, row_cy - lh / 2.0, title_w, lh),
        text: fit.text,
        style: TextStyle::Title,
        fill: Fill::solid(Role::OnSurface),
        align: Align::Left,
        wrap_width: title_w,
    });
}

/// A measured group in one of its presentations.
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
        header: GroupHeader,
        cards: Vec<card::CardLayout>,
    },
    /// Between the two: the cards behind slide out from under the top card
    /// (or back) while the header fades in and the sheets fade out.
    /// `t` is 0 for the stack and 1 for the list.
    Morph {
        header: GroupHeader,
        cards: Vec<card::CardLayout>,
        sheets: usize,
        t: f32,
    },
}

/// An expanded group's header row: app name, "Show less" and "×".
#[derive(Debug, Clone)]
struct GroupHeader {
    title: crate::text::TextFit,
    less: crate::text::TextFit,
    /// Row width (the column width).
    width: f32,
}

impl GroupHeader {
    /// Room the "Show less" pill and the "×" keep clear of the title.
    const CONTROLS_W: f32 = 140.0;

    fn measure(app_name: &str, w: f32, meas: &mut dyn TextMeasure) -> Self {
        Self {
            title: meas.fit(
                app_name,
                TextStyle::GroupTitle,
                (w - Self::CONTROLS_W).max(1.0),
                1,
            ),
            less: meas.fit("Show less", TextStyle::Button, 120.0, 1),
            width: w,
        }
    }

    /// Emits the row as a sprite at `origin`, faded by `opacity`.
    fn sprite(
        &self,
        scene: &mut Scene,
        view: Viewport<'_>,
        key: &str,
        origin: Point,
        ui: &Interaction,
        opacity: f32,
    ) {
        scene.layer(origin, opacity, view.scale, view.clip, |s, clip| {
            self.emit(s, view.style, key, Point::new(0.0, 0.0), ui, clip);
        });
    }

    fn emit(
        &self,
        scene: &mut Scene,
        style: &Style,
        key: &str,
        origin: Point,
        ui: &Interaction,
        clip: Rect,
    ) {
        let w = self.width;
        let title_w = w - Self::CONTROLS_W;
        let row_cy = origin.y + m::GROUP_HEADER_HEIGHT / 2.0 - 3.0;
        let lh = style.text.line_height(TextStyle::GroupTitle);
        scene.push(Node::Text {
            rect: Rect::new(origin.x + 6.0, row_cy - lh / 2.0, title_w, lh),
            text: self.title.text.clone(),
            style: TextStyle::GroupTitle,
            fill: Fill::solid(Role::OnSurface),
            align: Align::Left,
            wrap_width: title_w,
        });
        let close_r = 12.0;
        let close_c = Point::new(origin.x + w - close_r - 2.0, row_cy);
        let clear = HitTarget::ClearGroup(key.to_owned());
        close_button(
            scene,
            close_c,
            close_r,
            clear.clone(),
            ui.hovered.as_ref() == Some(&clear),
            clip,
        );
        let pill_h = 24.0;
        let pw = (self.less.width + 2.0 * m::PILL_PAD_X).ceil();
        let collapse = HitTarget::CollapseGroup(key.to_owned());
        pill(
            scene,
            style,
            Rect::new(
                close_c.x - close_r - 8.0 - pw,
                row_cy - pill_h / 2.0,
                pw,
                pill_h,
            ),
            &self.less,
            ui.hovered.as_ref() == Some(&collapse),
            collapse,
            clip,
        );
    }
}

#[derive(Debug, Clone)]
struct GroupLayout {
    shape: GroupShape,
    height: f32,
}

/// `n` as a float (counts here are tiny).
fn count(n: usize) -> f32 {
    crate::geom::u32_to_f32(u32::try_from(n).unwrap_or(u32::MAX))
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Group morph progress by which the stack's sheets have faded out.
const MORPH_SHEETS_OUT: f32 = 0.5;
/// Group morph progress at which the emerging cards start to fade in.
const MORPH_CARDS_IN: f32 = 0.45;

/// 0 before `from`, 1 after `to`, linear between.
fn fade(t: f32, from: f32, to: f32) -> f32 {
    ((t - from) / (to - from)).clamp(0.0, 1.0)
}

/// The part of `clip` from `y` down.
fn below(clip: Rect, y: f32) -> Rect {
    let top = y.max(clip.y);
    Rect::new(clip.x, top, clip.w, (clip.bottom() - top).max(0.0))
}

/// Height of a collapsed stack with `top` in front.
fn stack_height(top: &card::CardLayout, sheets: usize) -> f32 {
    top.height + m::SHEET_PEEK * count(sheets)
}

/// Top of each card in an expanded group, from the group's top edge.
fn list_offsets(cards: &[card::CardLayout]) -> Vec<f32> {
    cards
        .iter()
        .scan(m::GROUP_HEADER_HEIGHT, |y, lay| {
            let at = *y;
            *y += lay.height + m::ACTION_GAP;
            Some(at)
        })
        .collect()
}

impl GroupLayout {
    /// Measures `g`; `morph` is its expansion while it animates.
    fn measure(
        g: &GroupView,
        w: f32,
        scale: f32,
        morph: Option<f32>,
        draft: Option<&ReplyDraft>,
        meas: &mut dyn TextMeasure,
    ) -> Self {
        let lines = if g.expanded {
            m::BODY_LINES_EXPANDED
        } else {
            m::BODY_LINES_COLLAPSED
        };
        match g.cards.as_slice() {
            [] => Self {
                shape: GroupShape::Expanded {
                    header: GroupHeader::measure(&g.app_name, w, meas),
                    cards: Vec::new(),
                },
                height: 0.0,
            },
            [one] => {
                let c = measure_card(one, w, lines, draft, scale, meas);
                Self {
                    height: c.height,
                    shape: GroupShape::Single(c),
                }
            }
            [top, rest @ ..] if !g.expanded && morph.is_none() => {
                let c = measure_card(top, w, lines, draft, scale, meas);
                let sheets = rest.len().min(m::MAX_SHEETS);
                Self {
                    height: stack_height(&c, sheets),
                    shape: GroupShape::Stack { top: c, sheets },
                }
            }
            all => {
                let cards: Vec<_> = all
                    .iter()
                    .map(|c| measure_card(c, w, lines, draft, scale, meas))
                    .collect();
                let header = GroupHeader::measure(&g.app_name, w, meas);
                let list_h = m::GROUP_HEADER_HEIGHT
                    + cards.iter().map(|c| c.height).sum::<f32>()
                    + count(cards.len()) * m::ACTION_GAP;
                match (morph, cards.first()) {
                    (Some(t), Some(top)) => {
                        let sheets = (cards.len() - 1).min(m::MAX_SHEETS);
                        Self {
                            height: lerp(stack_height(top, sheets), list_h, t),
                            shape: GroupShape::Morph {
                                header,
                                cards,
                                sheets,
                                t,
                            },
                        }
                    }
                    _ => Self {
                        shape: GroupShape::Expanded { header, cards },
                        height: list_h,
                    },
                }
            }
        }
    }

    fn emit(
        &self,
        scene: &mut Scene,
        g: &GroupView,
        origin: Point,
        ui: &Interaction,
        view: Viewport<'_>,
    ) {
        let card_hovered =
            |c: &CardView| ui.hovered.as_ref().and_then(HitTarget::card) == Some(c.id);
        let list_opts = |c: &CardView| CardOpts {
            style: view.style,
            body_target: HitTarget::Activate(c.id),
            close_target: HitTarget::Dismiss(c.id),
            hovered: card_hovered(c),
            hovered_target: ui.hovered.clone(),
        };
        let expand = HitTarget::ExpandGroup(g.key.clone());
        let clear = HitTarget::ClearGroup(g.key.clone());
        let stack_opts = CardOpts {
            style: view.style,
            body_target: expand.clone(),
            close_target: clear.clone(),
            hovered: ui
                .hovered
                .as_ref()
                .is_some_and(|h| *h == expand || *h == clear),
            hovered_target: ui.hovered.clone(),
        };
        match &self.shape {
            GroupShape::Single(lay) => {
                let Some(c) = g.cards.first() else { return };
                card_sprite(scene, view, c, lay, origin, &list_opts(c), 1.0);
            }
            GroupShape::Stack { top, sheets } => {
                let Some(c) = g.cards.first() else { return };
                sheets_sprite(scene, view, origin, top, *sheets, &expand, 1.0);
                card_sprite(scene, view, c, top, origin, &stack_opts, 1.0);
            }
            GroupShape::Expanded { header, cards } => {
                if cards.is_empty() {
                    return;
                }
                header.sprite(scene, view, &g.key, origin, ui, 1.0);
                for ((c, lay), dy) in g.cards.iter().zip(cards).zip(list_offsets(cards)) {
                    let at = Point::new(origin.x, origin.y + dy);
                    card_sprite(scene, view, c, lay, at, &list_opts(c), 1.0);
                }
            }
            GroupShape::Morph {
                header,
                cards,
                sheets,
                t,
            } => {
                let (Some(c0), Some(top)) = (g.cards.first(), cards.first()) else {
                    return;
                };
                let t = *t;
                let top_at = Point::new(origin.x, origin.y + m::GROUP_HEADER_HEIGHT * t);
                header.sprite(scene, view, &g.key, origin, ui, t);
                // The sheets hand over to the real cards: they fade out over
                // the first part of the motion, the cards in over the rest.
                let sheets_alpha = 1.0 - fade(t, 0.0, MORPH_SHEETS_OUT);
                let cards_alpha = fade(t, MORPH_CARDS_IN, 1.0);
                sheets_sprite(scene, view, top_at, top, *sheets, &expand, sheets_alpha);
                // Each card starts tucked behind the top card, its bottom
                // edge where its sheet peeks out.
                let tucked_bottom = top_at.y + top.height;
                let tops: Vec<f32> = cards
                    .iter()
                    .zip(list_offsets(cards))
                    .enumerate()
                    .map(|(i, (lay, dy))| {
                        if i == 0 {
                            return top_at.y;
                        }
                        let peek = m::SHEET_PEEK * count(i.min(m::MAX_SHEETS));
                        lerp(tucked_bottom + peek - lay.height, origin.y + dy, t)
                    })
                    .collect();
                // Deepest first, so the top card ends up in front. Each card
                // shows only below the one in front of it, so translucent
                // cards never stack their text.
                for i in (1..cards.len()).rev() {
                    let (Some(c), Some(lay), Some(&y)) =
                        (g.cards.get(i), cards.get(i), tops.get(i))
                    else {
                        continue;
                    };
                    let front_bottom = tops[i - 1] + cards[i - 1].height;
                    let hits = scene.hits.len();
                    scene.push(Node::PushClip(below(view.clip, front_bottom)));
                    let at = Point::new(origin.x, y);
                    card_sprite(scene, view, c, lay, at, &list_opts(c), cards_alpha);
                    scene.push(Node::PopClip);
                    if !g.expanded {
                        // Folding away: nothing to click on.
                        scene.hits.truncate(hits);
                    }
                }
                let opts = if g.expanded {
                    list_opts(c0)
                } else {
                    stack_opts
                };
                card_sprite(scene, view, c0, top, top_at, &opts, 1.0);
            }
        }
    }
}

/// Where groups are drawn: the list viewport (hit regions are clipped to
/// it), the output scale (sprites align to its pixel grid) and the theme.
#[derive(Debug, Clone, Copy)]
struct Viewport<'a> {
    clip: Rect,
    scale: f32,
    style: &'a Style,
}

/// Emits a stack's peeking sheets as a sprite behind a top card at `at`.
fn sheets_sprite(
    scene: &mut Scene,
    view: Viewport<'_>,
    at: Point,
    top: &card::CardLayout,
    sheets: usize,
    expand: &HitTarget,
    opacity: f32,
) {
    scene.layer(at, opacity, view.scale, view.clip, |s, clip| {
        emit_sheets(
            s,
            view.style,
            Point::new(0.0, 0.0),
            top,
            sheets,
            expand,
            clip,
        );
    });
}

/// Emits a card as a cached sprite (see [`Scene::layer`]) at `at`, so
/// moving or fading it during a group morph reuses its pixels.
fn card_sprite(
    scene: &mut Scene,
    view: Viewport<'_>,
    card: &CardView,
    lay: &card::CardLayout,
    at: Point,
    opts: &CardOpts<'_>,
    opacity: f32,
) {
    scene.layer(at, opacity, view.scale, view.clip, |s, clip| {
        emit_card(s, card, lay, Point::new(0.0, 0.0), opts, clip);
    });
}

/// Paints the "sheets" peeking below a collapsed stack, deepest first.
fn emit_sheets(
    scene: &mut Scene,
    style: &Style,
    origin: Point,
    top: &card::CardLayout,
    sheets: usize,
    expand: &HitTarget,
    clip: Rect,
) {
    let card_rect = Rect::new(origin.x, origin.y, top.width, top.height);
    let radius = style.card_radius(top.height);
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
            radius: (radius - 2.0 * level).max(0.0),
            fill: style.sheets[usize::from(i != 1)],
            stroke: style.card_border,
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
            reply: None,
            origin: None,
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
            &Style::default(),
            &mut MonoMeasure,
        );
        assert!(
            lay.scene
                .flat_nodes()
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
            &Style::default(),
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
            &Style::default(),
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
            ..Interaction::default()
        };
        let lay = layout_panel(
            &view(vec![group("a", &[1], false)]),
            SIZE,
            1.0,
            &ui,
            &Style::default(),
            &mut MonoMeasure,
        );
        let s = &lay.scene;
        let card = rect_of(s, &HitTarget::Activate(NotificationId(1)));
        let close = rect_of(s, &HitTarget::Dismiss(NotificationId(1)));
        // The "×" sits in the card's top-left corner, its centre inside the
        // pill's rounded corner, and wins the hit test over the card body.
        let c = close.center();
        let r = Style::default().card_radius(card.h);
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
            &Style::default(),
            &mut MonoMeasure,
        );
        let s = &lay.scene;
        let sheets = s
            .flat_nodes()
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

    /// Top of the "mail" card laid out below a "chat" group in `expanded`
    /// state, mid-morph at `t` if given; plus the scene.
    fn below_chat(expanded: bool, t: Option<f32>) -> (f32, Scene) {
        let ui = Interaction {
            morph: t.map(|t| vec![("chat".to_owned(), t)]).unwrap_or_default(),
            ..Interaction::default()
        };
        let lay = layout_panel(
            &view(vec![
                group("chat", &[3, 2, 1], expanded),
                group("mail", &[9], false),
            ]),
            Size::new(380.0, 2000.0),
            1.0,
            &ui,
            &Style::default(),
            &mut MonoMeasure,
        );
        let y = lay
            .scene
            .hits
            .iter()
            .find(|h| h.target == HitTarget::Activate(NotificationId(9)))
            .map_or(f32::NAN, |h| h.rect.y);
        (y, lay.scene)
    }

    fn has_opacity(s: &Scene) -> bool {
        s.nodes.iter().any(|n| match n {
            Node::PushOpacity(_) => true,
            Node::Layer(l) => l.opacity < 1.0,
            _ => false,
        })
    }

    #[test]
    fn group_morph_meets_both_rest_layouts() {
        let (stack_y, stack) = below_chat(false, None);
        let (list_y, list) = below_chat(true, None);
        assert!(!has_opacity(&stack) && !has_opacity(&list));
        assert!(list_y > stack_y + 50.0);

        let (y0, _) = below_chat(true, Some(0.0));
        let (y1, end) = below_chat(true, Some(1.0));
        assert!((y0 - stack_y).abs() < 0.01, "{y0} vs {stack_y}");
        assert!((y1 - list_y).abs() < 0.01, "{y1} vs {list_y}");
        assert!(!has_opacity(&end), "fully expanded needs no fade");

        // Halfway, the group below sits between, and things are fading.
        let (mid_y, mid) = below_chat(true, Some(0.5));
        assert!(mid_y > stack_y && mid_y < list_y);
        assert!(has_opacity(&mid));
        assert!(targets(&mid).contains(&HitTarget::Activate(NotificationId(2))));
        assert!(targets(&mid).contains(&HitTarget::CollapseGroup("chat".into())));
    }

    #[test]
    fn emerging_cards_fade_in_late_below_the_card_in_front() {
        let drawn = |s: &Scene, text: &str| {
            s.flat_nodes()
                .iter()
                .any(|n| matches!(n, Node::Text { text: t, .. } if t == text))
        };
        let (_, early) = below_chat(true, Some(0.3));
        assert!(!drawn(&early, "Summary 2"), "not yet faded in");
        let (_, late) = below_chat(true, Some(0.8));
        assert!(drawn(&late, "Summary 2"));
        // Card 2 (drawn last of the emerging cards) is clipped to below
        // card 3, wherever card 3 is.
        let top_bottom = late
            .hits
            .iter()
            .find(|h| h.target == HitTarget::Activate(NotificationId(3)))
            .map_or(f32::NAN, |h| h.rect.bottom());
        let clip_top = late.nodes.iter().rev().find_map(|n| match n {
            Node::PushClip(r) if r.y > 60.0 => Some(r.y),
            _ => None,
        });
        assert!(
            clip_top.is_some_and(|y| (y - top_bottom).abs() < 1.0),
            "{clip_top:?} vs {top_bottom}"
        );
    }

    #[test]
    fn folding_group_cards_are_not_clickable() {
        let (_, s) = below_chat(false, Some(0.5));
        let t = targets(&s);
        assert!(!t.contains(&HitTarget::Activate(NotificationId(2))));
        assert!(t.contains(&HitTarget::ExpandGroup("chat".into())));
    }

    #[test]
    fn two_card_stack_has_one_sheet() {
        let lay = layout_panel(
            &view(vec![group("chat", &[2, 1], false)]),
            SIZE,
            1.0,
            &Interaction::default(),
            &Style::default(),
            &mut MonoMeasure,
        );
        assert_eq!(
            lay.scene
                .flat_nodes()
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
            &Style::default(),
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
            &Style::default(),
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
            &Style::default(),
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
            ..Interaction::default()
        };
        let end = layout_panel(
            &v,
            Size::new(380.0, 600.0),
            1.0,
            &ui,
            &Style::default(),
            &mut MonoMeasure,
        );
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
            &Style::default(),
            &mut MonoMeasure,
        );
        let card = rect_of(&lay.scene, &HitTarget::Activate(NotificationId(1)));
        let h = card.h * 1.6;
        assert!((h - h.round()).abs() < 1e-3);
    }
}
