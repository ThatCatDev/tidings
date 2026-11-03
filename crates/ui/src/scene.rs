//! The positioned element tree produced by layout and consumed by painting
//! and hit-testing.
//!
//! A [`Scene`] is plain data: drawing primitives in paint order plus a list
//! of hit regions (topmost last). It is compared between frames to compute
//! damage, so every node is `PartialEq`.

use tidings_core::model::{Image, NotificationId};
use tidings_core::ui::UiEvent;

use crate::geom::{Point, Rect, Size};
use crate::style::{Fill, Stroke, TextStyle};

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
    /// Rounded rectangle, optionally with an inside stroke.
    RoundRect {
        rect: Rect,
        radius: f32,
        fill: Option<Fill>,
        stroke: Option<Stroke>,
    },
    /// A stacked-group "sheet": a rounded rect with the card in front of it
    /// (`cutout`) cut away, so translucent fills never double up.
    Sheet {
        rect: Rect,
        radius: f32,
        fill: Fill,
        stroke: Option<Stroke>,
        cutout: Rect,
        cutout_radius: f32,
    },
    /// A soft drop shadow: `rect` (a rounded rect) Gaussian-blurred, shifted
    /// down by `offset_y`. `fill` is the opacity of the unblurred shape, so
    /// the shadow is half that at the edge and fades out over `spread`
    /// (three standard deviations). Nothing is painted inside `rect` itself, so a
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
        stroke: Option<Stroke>,
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
    /// Multiplies the opacity of following nodes by the given factor (0..=1)
    /// until the matching [`Node::PopOpacity`]. Nests multiplicatively.
    PushOpacity(f32),
    PopOpacity,
    /// A sub-scene painted once into a sprite (reused while it is unchanged)
    /// and composited; see [`Scene::layer`].
    Layer(Box<Layer>),
}

/// A cached sprite: `scene` (local coordinates, origin at the sprite's
/// top-left) composited with its top-left at `rect`'s.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// Where the sprite lands, in the enclosing scene's coordinates.
    pub rect: Rect,
    pub opacity: f32,
    pub scene: Scene,
}

impl Node {
    /// This node moved by `(dx, dy)`.
    #[must_use]
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        let mut n = self.clone();
        match &mut n {
            Self::RoundRect { rect, .. }
            | Self::Text { rect, .. }
            | Self::Image { rect, .. }
            | Self::Glyph { rect, .. }
            | Self::Shadow { rect, .. }
            | Self::PushClip(rect) => *rect = rect.translate(dx, dy),
            Self::Sheet { rect, cutout, .. } => {
                *rect = rect.translate(dx, dy);
                *cutout = cutout.translate(dx, dy);
            }
            Self::Circle { center, .. } => *center = Point::new(center.x + dx, center.y + dy),
            Self::Layer(l) => l.rect = l.rect.translate(dx, dy),
            Self::PopClip | Self::PushOpacity(_) | Self::PopOpacity => {}
        }
        n
    }

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
            Self::Layer(l) => return Some(l.rect),
            Self::PushClip(_) | Self::PopClip | Self::PushOpacity(_) | Self::PopOpacity => {
                return None;
            }
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
    /// A card's "Reply" button: opens its reply field.
    StartReply(NotificationId),
    /// The reply field itself: clicking it takes the keyboard back.
    ReplyField(NotificationId),
    /// The reply field's send button.
    SendReply(NotificationId),
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
    /// The event a completed click on this target emits, if any. The reply
    /// targets emit none: the UI handles them itself (see
    /// [`Self::reply_click`]).
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
            Self::StartReply(_) | Self::ReplyField(_) | Self::SendReply(_) | Self::Background => {
                return None;
            }
        })
    }

    /// What a click on a reply target asks of the UI, if this is one.
    #[must_use]
    pub fn reply_click(&self) -> Option<ReplyClick> {
        match self {
            Self::StartReply(id) | Self::ReplyField(id) => Some(ReplyClick::Focus(*id)),
            Self::SendReply(id) => Some(ReplyClick::Send(*id)),
            _ => None,
        }
    }

    /// The notification card this target belongs to (for card hover).
    #[must_use]
    pub fn card(&self) -> Option<NotificationId> {
        match self {
            Self::Activate(id)
            | Self::Dismiss(id)
            | Self::Action { id, .. }
            | Self::StartReply(id)
            | Self::ReplyField(id)
            | Self::SendReply(id)
            | Self::ExpandToast(id)
            | Self::CollapseToast(id) => Some(*id),
            _ => None,
        }
    }

    /// Whether the pointer should show the "hand" cursor over this target.
    #[must_use]
    pub fn is_clickable(&self) -> bool {
        !matches!(self, Self::Background | Self::ReplyField(_))
    }

    /// Whether this is a text field (the pointer shows an I-beam).
    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(self, Self::ReplyField(_))
    }
}

/// A click on one of a card's reply targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyClick {
    /// Open the card's reply field (or give it the keyboard back).
    Focus(NotificationId),
    /// Send what is typed.
    Send(NotificationId),
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

    /// Emits whatever `f` pushes at `alpha` opacity. Hit regions are
    /// unaffected; fully opaque content gets no opacity group and fully
    /// transparent content paints nothing.
    pub fn with_opacity(&mut self, alpha: f32, f: impl FnOnce(&mut Self)) {
        if alpha >= 1.0 {
            f(self);
            return;
        }
        if alpha <= 0.0 {
            let painted = self.nodes.len();
            f(self);
            self.nodes.truncate(painted);
            return;
        }
        self.push(Node::PushOpacity(alpha));
        f(self);
        self.push(Node::PopOpacity);
    }

    /// Emits what `f` draws as a cached sprite placed at `at`, faded by
    /// `opacity`.
    ///
    /// `f` draws in local coordinates (as if `at` were the origin) and gets
    /// `hit_clip` translated to match; its hit regions are moved back into
    /// this scene. Because the local drawing does not depend on `at`, moving
    /// or fading the sprite reuses its pixels instead of repainting. The
    /// sprite's edge is kept on `scale`'s physical pixel grid, so a layer
    /// at a whole-pixel `at` paints exactly where direct drawing would.
    pub fn layer(
        &mut self,
        at: Point,
        opacity: f32,
        scale: f32,
        hit_clip: Rect,
        f: impl FnOnce(&mut Self, Rect),
    ) {
        let mut local = Scene::new(Size::default());
        f(&mut local, hit_clip.translate(-at.x, -at.y));
        self.hits.extend(local.hits.drain(..).map(|h| Hit {
            rect: h.rect.translate(at.x, at.y),
            ..h
        }));
        if opacity <= 0.0 {
            return;
        }
        let Some(extent) = local
            .nodes
            .iter()
            .filter_map(Node::bounds)
            .reduce(|a, b| a.union(&b))
        else {
            return;
        };
        let grid = |v: f32| (v * scale).floor() / scale;
        let (x0, y0) = (grid(extent.x), grid(extent.y));
        let size = Size::new((extent.right() - x0).ceil(), (extent.bottom() - y0).ceil());
        let scene = Scene {
            size,
            nodes: local.nodes.iter().map(|n| n.translated(-x0, -y0)).collect(),
            hits: Vec::new(),
        };
        self.push(Node::Layer(Box::new(Layer {
            rect: Rect::new(at.x + x0, at.y + y0, size.w, size.h),
            opacity: opacity.min(1.0),
            scene,
        })));
    }

    /// Every node with layers expanded in place (in this scene's
    /// coordinates), so tests can inspect what is drawn.
    #[cfg(test)]
    #[must_use]
    pub fn flat_nodes(&self) -> Vec<Node> {
        let mut out = Vec::with_capacity(self.nodes.len());
        for n in &self.nodes {
            match n {
                Node::Layer(l) => out.extend(
                    l.scene
                        .flat_nodes()
                        .iter()
                        .map(|c| c.translated(l.rect.x, l.rect.y)),
                ),
                other => out.push(other.clone()),
            }
        }
        out
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
    /// A size change or a change to the clip/opacity structure (which alters
    /// how unchanged nodes look) damages everything.
    #[must_use]
    pub fn damage_to(&self, next: &Scene) -> Vec<Rect> {
        let full = Rect::new(0.0, 0.0, next.size.w, next.size.h);
        if self.size != next.size || structure(&self.nodes) != structure(&next.nodes) {
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
        let gone: Vec<&Node> = self
            .nodes
            .iter()
            .filter(|n| !next.nodes.contains(n))
            .collect();
        let new: Vec<&Node> = next
            .nodes
            .iter()
            .filter(|n| !self.nodes.contains(n))
            .collect();
        // A layer redrawn in place damages only what changed inside it.
        let in_place = |a: &Node, b: &Node| match (a, b) {
            (Node::Layer(a), Node::Layer(b))
                if a.rect == b.rect && a.opacity.to_bits() == b.opacity.to_bits() =>
            {
                Some(
                    a.scene
                        .damage_to(&b.scene)
                        .into_iter()
                        .map(|r| r.translate(a.rect.x, a.rect.y))
                        .collect::<Vec<_>>(),
                )
            }
            _ => None,
        };
        for a in &gone {
            match new.iter().find_map(|b| in_place(a, b)) {
                Some(inner) => inner.into_iter().for_each(&mut add),
                None => a.bounds().into_iter().for_each(&mut add),
            }
        }
        for b in new
            .iter()
            .filter(|b| !gone.iter().any(|a| in_place(a, b).is_some()))
        {
            b.bounds().into_iter().for_each(&mut add);
        }
        out.into_iter().filter_map(|r| r.intersect(&full)).collect()
    }
}

/// The nodes that change how others paint.
fn structure(nodes: &[Node]) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|n| {
            matches!(
                n,
                Node::PushClip(_) | Node::PopClip | Node::PushOpacity(_) | Node::PopOpacity
            )
        })
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
    fn opacity_groups_nest_and_fade_damages_everything() {
        let mut a = Scene::new(Size::new(100.0, 100.0));
        a.with_opacity(1.0, |s| s.push(rr(0.0)));
        assert_eq!(a.nodes, vec![rr(0.0)], "opaque content needs no group");
        let mut gone = Scene::new(Size::new(100.0, 100.0));
        gone.with_opacity(0.0, |s| {
            s.push(rr(0.0));
            s.hit(Rect::new(0.0, 0.0, 5.0, 5.0), 0.0, HitTarget::ClearAll);
        });
        assert!(gone.nodes.is_empty(), "invisible content paints nothing");
        assert_eq!(gone.hits.len(), 1, "but stays clickable");
        let mut b = Scene::new(Size::new(100.0, 100.0));
        b.with_opacity(0.5, |s| s.push(rr(0.0)));
        assert_eq!(
            b.nodes,
            vec![Node::PushOpacity(0.5), rr(0.0), Node::PopOpacity]
        );
        let mut c = Scene::new(Size::new(100.0, 100.0));
        c.with_opacity(0.6, |s| s.push(rr(0.0)));
        assert_eq!(b.damage_to(&c), vec![Rect::new(0.0, 0.0, 100.0, 100.0)]);
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

    #[test]
    fn every_clickable_target_maps_to_its_event() {
        let id = NotificationId(3);
        let key = || "mail".to_owned();
        let cases = [
            (
                HitTarget::Action { id, key: key() },
                UiEvent::InvokeAction { id, key: key() },
            ),
            (HitTarget::Dismiss(id), UiEvent::Dismiss(id)),
            (
                HitTarget::CollapseGroup(key()),
                UiEvent::ToggleGroup { key: key() },
            ),
            (
                HitTarget::ClearGroup(key()),
                UiEvent::ClearGroup { key: key() },
            ),
            (HitTarget::ClearAll, UiEvent::ClearAll),
            (HitTarget::ToggleDnd, UiEvent::ToggleDnd),
        ];
        for (target, event) in cases {
            assert_eq!(target.click_event(), Some(event), "{target:?}");
            assert!(target.is_clickable() && !target.is_text());
            assert_eq!(target.reply_click(), None);
        }
        // Card-level targets hover their card; the rest belong to none.
        assert_eq!(HitTarget::Action { id, key: key() }.card(), Some(id));
        assert_eq!(HitTarget::ExpandToast(id).card(), Some(id));
        assert_eq!(HitTarget::ToggleDnd.card(), None);
    }

    #[test]
    fn reply_targets_are_handled_by_the_ui() {
        let id = NotificationId(4);
        let start = HitTarget::StartReply(id);
        let field = HitTarget::ReplyField(id);
        let send = HitTarget::SendReply(id);
        for t in [&start, &field, &send] {
            assert_eq!(t.click_event(), None, "{t:?} emits nothing itself");
            assert_eq!(t.card(), Some(id), "{t:?} belongs to its card");
        }
        assert_eq!(start.reply_click(), Some(ReplyClick::Focus(id)));
        assert_eq!(field.reply_click(), Some(ReplyClick::Focus(id)));
        assert_eq!(send.reply_click(), Some(ReplyClick::Send(id)));
        // The field is for typing: an I-beam, not a hand.
        assert!(field.is_text() && !field.is_clickable());
        assert!(start.is_clickable() && send.is_clickable());
        assert!(!HitTarget::Background.is_clickable());
        assert_eq!(HitTarget::ExpandGroup(String::new()).card(), None);
    }

    #[test]
    fn translated_moves_every_positioned_part() {
        let fill = Fill::solid(Role::Surface);
        let sheet = Node::Sheet {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            radius: 2.0,
            fill,
            stroke: None,
            cutout: Rect::new(1.0, 1.0, 5.0, 5.0),
            cutout_radius: 1.0,
        };
        let Node::Sheet { rect, cutout, .. } = sheet.translated(3.0, 4.0) else {
            panic!("still a sheet");
        };
        assert_eq!(
            (rect, cutout),
            (
                Rect::new(3.0, 4.0, 10.0, 10.0),
                Rect::new(4.0, 5.0, 5.0, 5.0)
            )
        );

        let circle = Node::Circle {
            center: Point::new(5.0, 5.0),
            radius: 2.0,
            fill,
            stroke: None,
        };
        let moved = circle.translated(-1.0, 2.0);
        assert!(matches!(moved, Node::Circle { center, .. } if center == Point::new(4.0, 7.0)));

        let layer = Node::Layer(Box::new(Layer {
            rect: Rect::new(0.0, 0.0, 4.0, 4.0),
            opacity: 1.0,
            scene: Scene::new(Size::new(4.0, 4.0)),
        }));
        let moved = layer.translated(2.0, 2.0);
        assert_eq!(moved.bounds(), Some(Rect::new(2.0, 2.0, 4.0, 4.0)));

        let shadow = Node::Shadow {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            radius: 2.0,
            spread: 4.0,
            offset_y: 1.0,
            fill,
        };
        assert!(matches!(
            shadow.translated(1.0, 2.0),
            Node::Shadow { rect, .. } if rect == Rect::new(1.0, 2.0, 10.0, 10.0)
        ));

        for n in [Node::PopClip, Node::PushOpacity(0.5), Node::PopOpacity] {
            assert_eq!(n.translated(5.0, 5.0), n);
        }
    }

    #[test]
    fn bounds_cover_what_a_node_paints() {
        let fill = Fill::solid(Role::Surface);
        let circle = Node::Circle {
            center: Point::new(10.0, 10.0),
            radius: 4.0,
            fill,
            stroke: None,
        };
        // Plus a pixel of anti-aliasing.
        assert_eq!(circle.bounds(), Some(Rect::new(5.0, 5.0, 10.0, 10.0)));
        let shadow = Node::Shadow {
            rect: Rect::new(10.0, 10.0, 20.0, 10.0),
            radius: 4.0,
            spread: 5.0,
            offset_y: 2.0,
            fill,
        };
        // Shifted down, grown by the spread and the fringe.
        assert_eq!(shadow.bounds(), Some(Rect::new(4.0, 6.0, 32.0, 22.0)));
        let text = Node::Text {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            text: "x".into(),
            style: TextStyle::Body,
            fill,
            align: Align::Left,
            wrap_width: 10.0,
        };
        assert_eq!(text.bounds(), Some(Rect::new(-3.0, -3.0, 16.0, 16.0)));
        for n in [
            Node::PushClip(Rect::new(0.0, 0.0, 1.0, 1.0)),
            Node::PopClip,
            Node::PushOpacity(0.5),
            Node::PopOpacity,
        ] {
            assert_eq!(n.bounds(), None, "{n:?} paints nothing itself");
        }
    }

    #[test]
    fn layers_keep_hits_and_skip_invisible_or_empty_content() {
        let target = HitTarget::ClearAll;
        let hit = |s: &mut Scene| s.hit(Rect::new(1.0, 1.0, 2.0, 2.0), 0.0, HitTarget::ClearAll);
        let at = Point::new(10.0, 20.0);
        let clip = Rect::new(0.0, 0.0, 100.0, 100.0);

        // Faded out: clickable, but nothing painted.
        let mut s = Scene::new(Size::new(100.0, 100.0));
        s.layer(at, 0.0, 1.0, clip, |s, _| {
            s.push(rr(0.0));
            hit(s);
        });
        assert!(s.nodes.is_empty(), "faded out: nothing painted");
        assert_eq!(s.hit_test(Point::new(12.0, 22.0)), Some(&target));

        // Only structure nodes: nothing to paint either.
        let mut s = Scene::new(Size::new(100.0, 100.0));
        s.layer(at, 1.0, 1.0, clip, |s, local_clip| {
            assert_eq!(local_clip, Rect::new(-10.0, -20.0, 100.0, 100.0));
            s.push(Node::PushClip(local_clip));
            s.push(Node::PopClip);
        });
        assert!(s.nodes.is_empty(), "no painted content: no layer");

        // Painted content becomes one layer on the pixel grid.
        let mut s = Scene::new(Size::new(100.0, 100.0));
        s.layer(at, 2.0, 1.6, clip, |s, _| s.push(rr(0.0)));
        let [Node::Layer(l)] = s.nodes.as_slice() else {
            panic!("one layer: {:?}", s.nodes);
        };
        assert!((l.opacity - 1.0).abs() < f32::EPSILON, "opacity capped");
        // The rect's 1 px fringe starts at -1, snapped down to the 1/1.6 grid.
        assert!((l.rect.x - (10.0 - 1.25)).abs() < 1e-4, "{:?}", l.rect);
        assert_eq!(s.flat_nodes(), vec![rr(0.0).translated(10.0, 20.0)]);
    }

    #[test]
    fn a_layer_changed_in_place_damages_only_its_changed_part() {
        let frame = |x: f32| {
            let mut s = Scene::new(Size::new(200.0, 100.0));
            s.layer(Point::new(50.0, 0.0), 1.0, 1.0, Rect::default(), |s, _| {
                s.push(rr(0.0));
                s.push(rr(x));
                s.push(Node::RoundRect {
                    rect: Rect::new(0.0, 80.0, 100.0, 10.0),
                    radius: 0.0,
                    fill: None,
                    stroke: None,
                });
            });
            s
        };
        let (a, b) = (frame(40.0), frame(60.0));
        let damage = a.damage_to(&b);
        // Not the whole layer (x 49..151): just where the moved rect was
        // (89..101) and is now (109..121), in scene coordinates.
        assert_eq!(damage.len(), 2, "{damage:?}");
        assert!(
            damage.iter().all(|r| r.x >= 88.0 && r.right() <= 122.0),
            "{damage:?}"
        );

        // A layer that moves damages its old and new place in full.
        let mut moved = Scene::new(Size::new(200.0, 100.0));
        moved.layer(Point::new(0.0, 0.0), 1.0, 1.0, Rect::default(), |s, _| {
            s.push(rr(0.0));
        });
        let damage = a.damage_to(&moved);
        assert!(damage.iter().any(|r| r.right() >= 150.0), "{damage:?}");
    }
}
