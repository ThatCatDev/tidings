//! The positioned element tree produced by layout and consumed by painting
//! and hit-testing.
//!
//! A [`Scene`] is plain data: drawing primitives in paint order plus a list
//! of hit regions (topmost last). It is compared between frames to compute
//! damage, so every node is `PartialEq`.

use tidings_core::model::{Image, NotificationId};
use tidings_core::ui::UiEvent;

use crate::geom::{Point, Rect, Size};
use crate::style::{Fill, TextStyle};

/// Horizontal text alignment inside a text node's rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Vector glyphs drawn without fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphKind {
    /// "×" cross.
    Close,
    /// Crescent moon (Do Not Disturb).
    Moon,
    /// "⌄" stroke chevron (expand).
    ChevronDown,
    /// "⌃" stroke chevron (collapse).
    ChevronUp,
}

/// A letter tile standing in for a missing icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monogram {
    pub letter: char,
    /// Index into [`crate::style::MONOGRAM_TINTS`] (wrapping).
    pub tint: u8,
}

/// One drawing primitive.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// Rounded rectangle, optionally with a 1 px inside stroke.
    RoundRect {
        rect: Rect,
        radius: f32,
        fill: Option<Fill>,
        stroke: Option<Fill>,
    },
    /// A stacked-group "sheet": a rounded rect with the card in front of it
    /// (`cutout`) cut away, so translucent fills never double up.
    Sheet {
        rect: Rect,
        radius: f32,
        fill: Fill,
        stroke: Option<Fill>,
        cutout: Rect,
        cutout_radius: f32,
    },
    /// A soft drop shadow around a rounded `rect`: `fill` at its peak right
    /// at the shape's edge, fading to nothing `spread` beyond it, shifted
    /// down by `offset_y`. Nothing is painted inside `rect` itself, so a
    /// translucent shape on top never darkens.
    Shadow {
        rect: Rect,
        radius: f32,
        spread: f32,
        offset_y: f32,
        fill: Fill,
    },
    Circle {
        center: Point,
        radius: f32,
        fill: Fill,
        stroke: Option<Fill>,
    },
    /// Pre-fitted text (see [`crate::text::TextMeasure::fit`]).
    Text {
        rect: Rect,
        text: String,
        style: TextStyle,
        fill: Fill,
        align: Align,
        /// Wrap width the text was fitted at (needed to shape identically).
        wrap_width: f32,
    },
    /// An icon or image, cover-fitted and rounded. If it cannot be loaded,
    /// the painter draws the `fallback` letter tile instead.
    Image {
        rect: Rect,
        radius: f32,
        image: Option<Image>,
        fallback: Monogram,
    },
    Glyph {
        rect: Rect,
        kind: GlyphKind,
        fill: Fill,
    },
    /// Restricts following nodes to `rect` until the matching [`Node::PopClip`].
    PushClip(Rect),
    PopClip,
}

impl Node {
    /// Bounds this node may paint into (including anti-aliasing fringe).
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        let r = match self {
            Self::RoundRect { rect, .. }
            | Self::Sheet { rect, .. }
            | Self::Text { rect, .. }
            | Self::Image { rect, .. }
            | Self::Glyph { rect, .. } => *rect,
            Self::Circle { center, radius, .. } => Rect::new(
                center.x - radius,
                center.y - radius,
                radius * 2.0,
                radius * 2.0,
            ),
            Self::Shadow {
                rect,
                spread,
                offset_y,
                ..
            } => rect.translate(0.0, *offset_y).outset(*spread),
            Self::PushClip(_) | Self::PopClip => return None,
        };
        // Text descenders/ascenders may exceed the line box slightly.
        let pad = if matches!(self, Self::Text { .. }) {
            3.0
        } else {
            1.0
        };
        Some(r.outset(pad))
    }
}

/// What a click/hover on a region means.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HitTarget {
    /// Card body: activate the notification.
    Activate(NotificationId),
    Action {
        id: NotificationId,
        key: String,
    },
    Dismiss(NotificationId),
    /// Collapsed stack: expand the group.
    ExpandGroup(String),
    /// A collapsed toast's expand (⌄) button.
    ExpandToast(NotificationId),
    /// An expanded toast's collapse (⌃) button.
    CollapseToast(NotificationId),
    /// "Show less" on an expanded group.
    CollapseGroup(String),
    ClearGroup(String),
    ClearAll,
    ToggleDnd,
    /// Inert panel background: swallows clicks without an event.
    Background,
}

impl HitTarget {
    /// The event a completed click on this target emits, if any.
    #[must_use]
    pub fn click_event(&self) -> Option<UiEvent> {
        Some(match self {
            Self::Activate(id) => UiEvent::Activate(*id),
            Self::Action { id, key } => UiEvent::InvokeAction {
                id: *id,
                key: key.clone(),
            },
            Self::Dismiss(id) => UiEvent::Dismiss(*id),
            Self::ExpandGroup(key) | Self::CollapseGroup(key) => {
                UiEvent::ToggleGroup { key: key.clone() }
            }
            Self::ExpandToast(id) => UiEvent::SetToastExpanded {
                id: *id,
                expanded: true,
            },
            Self::CollapseToast(id) => UiEvent::SetToastExpanded {
                id: *id,
                expanded: false,
            },
            Self::ClearGroup(key) => UiEvent::ClearGroup { key: key.clone() },
            Self::ClearAll => UiEvent::ClearAll,
            Self::ToggleDnd => UiEvent::ToggleDnd,
            Self::Background => return None,
        })
    }

    /// The notification card this target belongs to (for card hover).
    #[must_use]
    pub fn card(&self) -> Option<NotificationId> {
        match self {
            Self::Activate(id)
            | Self::Dismiss(id)
            | Self::Action { id, .. }
            | Self::ExpandToast(id)
            | Self::CollapseToast(id) => Some(*id),
            _ => None,
        }
    }

    /// Whether the pointer should show the "hand" cursor over this target.
    #[must_use]
    pub fn is_clickable(&self) -> bool {
        !matches!(self, Self::Background)
    }
}

/// A clickable region. Later entries are on top.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub rect: Rect,
    pub target: HitTarget,
    /// Corner radius, so clicks just outside a rounded corner miss.
    pub radius: f32,
}

/// Layout output: paint list plus hit regions, in logical pixels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub size: Size,
    pub nodes: Vec<Node>,
    pub hits: Vec<Hit>,
}

impl Scene {
    #[must_use]
    pub fn new(size: Size) -> Self {
        Self {
            size,
            nodes: Vec::new(),
            hits: Vec::new(),
        }
    }

    pub fn push(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub fn hit(&mut self, rect: Rect, radius: f32, target: HitTarget) {
        self.hits.push(Hit {
            rect,
            target,
            radius,
        });
    }

    /// Topmost hit target under `p`.
    #[must_use]
    pub fn hit_test(&self, p: Point) -> Option<&HitTarget> {
        self.hits
            .iter()
            .rev()
            .find(|h| rounded_contains(&h.rect, h.radius, p))
            .map(|h| &h.target)
    }

    /// Rects that differ between `self` (previous frame) and `next`.
    ///
    /// Nodes present in only one of the two scenes contribute their bounds.
    /// A size change or a clip-structure change damages everything.
    #[must_use]
    pub fn damage_to(&self, next: &Scene) -> Vec<Rect> {
        let full = Rect::new(0.0, 0.0, next.size.w, next.size.h);
        if self.size != next.size || clips(&self.nodes) != clips(&next.nodes) {
            return vec![full];
        }
        let mut out: Vec<Rect> = Vec::new();
        let mut add = |r: Rect| {
            // Merge into an overlapping rect to keep the list short.
            if let Some(existing) = out.iter_mut().find(|e| e.intersect(&r).is_some()) {
                *existing = existing.union(&r);
            } else {
                out.push(r);
            }
        };
        for n in self.nodes.iter().filter(|n| !next.nodes.contains(n)) {
            n.bounds().into_iter().for_each(&mut add);
        }
        for n in next.nodes.iter().filter(|n| !self.nodes.contains(n)) {
            n.bounds().into_iter().for_each(&mut add);
        }
        out.into_iter().filter_map(|r| r.intersect(&full)).collect()
    }
}

fn clips(nodes: &[Node]) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|n| matches!(n, Node::PushClip(_) | Node::PopClip))
        .collect()
}

/// Point-in-rounded-rect test.
#[must_use]
pub fn rounded_contains(rect: &Rect, radius: f32, p: Point) -> bool {
    if !rect.contains(p) {
        return false;
    }
    let r = radius.min(rect.w / 2.0).min(rect.h / 2.0);
    if r <= 0.0 {
        return true;
    }
    let cx = p.x.clamp(rect.x + r, rect.right() - r);
    let cy = p.y.clamp(rect.y + r, rect.bottom() - r);
    let (dx, dy) = (p.x - cx, p.y - cy);
    dx * dx + dy * dy <= r * r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Fill, Role};

    fn rr(x: f32) -> Node {
        Node::RoundRect {
            rect: Rect::new(x, 0.0, 10.0, 10.0),
            radius: 2.0,
            fill: Some(Fill::solid(Role::Surface)),
            stroke: None,
        }
    }

    #[test]
    fn hit_test_prefers_topmost() {
        let mut s = Scene::new(Size::new(100.0, 100.0));
        s.hit(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            0.0,
            HitTarget::Background,
        );
        s.hit(Rect::new(10.0, 10.0, 20.0, 20.0), 0.0, HitTarget::ClearAll);
        assert_eq!(
            s.hit_test(Point::new(15.0, 15.0)),
            Some(&HitTarget::ClearAll)
        );
        assert_eq!(
            s.hit_test(Point::new(50.0, 50.0)),
            Some(&HitTarget::Background)
        );
        assert_eq!(s.hit_test(Point::new(150.0, 50.0)), None);
    }

    #[test]
    fn rounded_corners_miss() {
        let r = Rect::new(0.0, 0.0, 40.0, 40.0);
        assert!(!rounded_contains(&r, 16.0, Point::new(1.0, 1.0)));
        assert!(rounded_contains(&r, 16.0, Point::new(20.0, 1.0)));
        assert!(rounded_contains(&r, 16.0, Point::new(5.0, 5.0)));
    }

    #[test]
    fn damage_only_covers_changed_nodes() {
        let mut a = Scene::new(Size::new(100.0, 100.0));
        a.push(rr(0.0));
        a.push(rr(50.0));
        let mut b = a.clone();
        assert_eq!(a.damage_to(&b), Vec::new());
        b.nodes[1] = rr(60.0);
        let d = a.damage_to(&b);
        assert_eq!(d.len(), 1);
        assert!(d[0].x >= 48.0 && d[0].right() <= 72.0);
    }

    #[test]
    fn damage_full_on_resize() {
        let a = Scene::new(Size::new(100.0, 100.0));
        let b = Scene::new(Size::new(100.0, 120.0));
        assert_eq!(a.damage_to(&b), vec![Rect::new(0.0, 0.0, 100.0, 120.0)]);
    }

    #[test]
    fn click_events_map() {
        let id = NotificationId(7);
        assert_eq!(
            HitTarget::Activate(id).click_event(),
            Some(UiEvent::Activate(id))
        );
        assert_eq!(
            HitTarget::ExpandGroup("k".into()).click_event(),
            Some(UiEvent::ToggleGroup { key: "k".into() })
        );
        assert_eq!(HitTarget::Background.click_event(), None);
        assert_eq!(
            HitTarget::ExpandToast(id).click_event(),
            Some(UiEvent::SetToastExpanded { id, expanded: true })
        );
        assert_eq!(
            HitTarget::CollapseToast(id).click_event(),
            Some(UiEvent::SetToastExpanded {
                id,
                expanded: false
            })
        );
        assert_eq!(HitTarget::CollapseToast(id).card(), Some(id));
        assert_eq!(HitTarget::Dismiss(id).card(), Some(id));
        assert_eq!(HitTarget::ClearAll.card(), None);
    }
}
