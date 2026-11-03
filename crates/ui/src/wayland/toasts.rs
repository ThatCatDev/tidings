//! Toast stack state: per-toast layouts and pixmaps, motion, hover, and
//! per-frame composition. Independent of Wayland objects.

use std::time::Instant;

use tidings_core::model::NotificationId;
use tidings_core::view::ToastView;
use tiny_skia::Pixmap;

use crate::anim::{Timing, ToastMotion};
use crate::compose::ReplyDraft;
use crate::geom::{Point, Rect, f32_to_i32};
use crate::layout::{ToastLayout, layout_toast, stack_offsets};
use crate::render::{Painter, blit};
use crate::scene::HitTarget;
use crate::style::{Palette, metrics as m};

/// Toast surface width: the inset stack plus the gap to the screen edge.
pub const SURFACE_WIDTH: f32 = m::TOAST_INSET + m::TOAST_WIDTH + m::TOAST_MARGIN_RIGHT;

#[derive(Debug)]
struct Item {
    view: ToastView,
    layout: ToastLayout,
    pixmap: Option<Pixmap>,
    /// Scale and hover the pixmap was rendered for.
    rendered: Option<(u32, Option<HitTarget>, Option<ReplyDraft>)>,
    target_y: f32,
}

/// The toast stack's renderer-side state.
#[derive(Debug, Default)]
pub struct ToastStack {
    motion: ToastMotion,
    items: Vec<Item>,
    /// Removed toasts still fading out.
    ghosts: Vec<Item>,
    /// Hovered toast and the target under the pointer.
    pub hovered: Option<(NotificationId, HitTarget)>,
    /// The reply being typed into a toast, if any.
    pub reply: Option<ReplyDraft>,
}

impl ToastStack {
    /// An empty stack whose toasts move with `timing`.
    #[must_use]
    pub fn new(timing: Timing) -> Self {
        Self {
            motion: ToastMotion::new(timing),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.ghosts.is_empty()
    }

    /// Drops cached pixels so the next sync repaints (theme change).
    pub fn invalidate_paint(&mut self) {
        for i in self.items.iter_mut().chain(&mut self.ghosts) {
            i.rendered = None;
        }
    }

    fn hovered_target(&self, id: NotificationId) -> Option<HitTarget> {
        self.hovered
            .as_ref()
            .filter(|(h, _)| *h == id)
            .map(|(_, t)| t.clone())
    }

    /// Applies a new toast list (newest first): re-lays out changed toasts,
    /// renders them, and starts enter/move/leave motion.
    pub fn sync(
        &mut self,
        toasts: &[ToastView],
        painter: &mut Painter,
        palette: &Palette,
        scale: f32,
        now: Instant,
    ) {
        let mut old = std::mem::take(&mut self.items);
        for t in toasts {
            let id = t.card.id;
            let hovered = self.hovered_target(id);
            let mut item = match old.iter().position(|i| i.layout.id == id) {
                Some(pos) if old[pos].view == *t => old.swap_remove(pos),
                Some(pos) => {
                    old.swap_remove(pos);
                    Self::new_item(t, scale, hovered.as_ref(), painter)
                }
                None => {
                    self.ghosts.retain(|g| g.layout.id != id);
                    Self::new_item(t, scale, hovered.as_ref(), painter)
                }
            };
            let draft = self.reply.as_ref().filter(|d| d.id == id);
            let key = (scale.to_bits(), hovered.clone(), draft.cloned());
            if item.rendered.as_ref() != Some(&key) {
                item.layout = layout_toast(
                    t,
                    scale,
                    hovered.as_ref(),
                    draft,
                    &painter.style,
                    &mut painter.text,
                );
                item.pixmap = painter.paint(&item.layout.scene, palette, scale);
                item.rendered = Some(key);
            }
            self.items.push(item);
        }
        let heights: Vec<f32> = self.items.iter().map(|i| i.layout.scene.size.h).collect();
        for (item, y) in self.items.iter_mut().zip(stack_offsets(&heights)) {
            item.target_y = y;
        }
        let targets: Vec<_> = self
            .items
            .iter()
            .map(|i| (i.layout.id, i.target_y))
            .collect();
        let gone = self.motion.sync(&targets, now);
        self.ghosts
            .extend(old.into_iter().filter(|i| gone.contains(&i.layout.id)));
        if self
            .hovered
            .as_ref()
            .is_some_and(|(id, _)| !self.items.iter().any(|i| i.layout.id == *id))
        {
            self.hovered = None;
        }
    }

    fn new_item(
        t: &ToastView,
        scale: f32,
        hovered: Option<&HitTarget>,
        painter: &mut Painter,
    ) -> Item {
        Item {
            view: t.clone(),
            layout: layout_toast(t, scale, hovered, None, &painter.style, &mut painter.text),
            pixmap: None,
            rendered: None,
            target_y: 0.0,
        }
    }

    /// Re-renders for a new hover state; returns whether anything changed.
    pub fn set_hover(
        &mut self,
        hovered: Option<(NotificationId, HitTarget)>,
        painter: &mut Painter,
        palette: &Palette,
        scale: f32,
        now: Instant,
    ) -> bool {
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        let views: Vec<ToastView> = self.items.iter().map(|i| i.view.clone()).collect();
        self.sync(&views, painter, palette, scale, now);
        true
    }

    /// Height the surface needs at `now` (logical px).
    #[must_use]
    pub fn needed_height(&self, now: Instant) -> f32 {
        let frames = self.motion.frames(now);
        let mut h: f32 = 0.0;
        for i in &self.items {
            h = h.max(i.target_y + i.layout.scene.size.h);
        }
        for f in frames {
            if let Some(i) = self.find(f.id) {
                h = h.max(f.y + i.layout.scene.size.h);
            }
        }
        h.ceil()
    }

    fn find(&self, id: NotificationId) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| i.layout.id == id)
            .or_else(|| self.ghosts.iter().find(|i| i.layout.id == id))
    }

    /// Removes finished leave animations.
    pub fn reap(&mut self, now: Instant) {
        let done = self.motion.reap(now);
        self.ghosts.retain(|g| !done.contains(&g.layout.id));
    }

    #[must_use]
    pub fn is_animating(&self, now: Instant) -> bool {
        self.motion.is_animating(now)
    }

    /// Composes the stack into a `phys`-sized frame.
    #[must_use]
    pub fn compose(&self, now: Instant, scale: f32, phys: (u32, u32)) -> Option<Pixmap> {
        let mut frame = Pixmap::new(phys.0, phys.1)?;
        for f in self.motion.frames(now) {
            let Some(pm) = self.find(f.id).and_then(|i| i.pixmap.as_ref()) else {
                continue;
            };
            let x = f32_to_i32((f.dx * scale).round());
            let y = f32_to_i32((f.y * scale).round());
            blit(&mut frame, pm, x, y, f.alpha);
        }
        Some(frame)
    }

    /// Each visible toast's glass where it is drawn at `now`, for the
    /// compositor to blur behind. Toasts mostly faded out are left out, so
    /// their blur does not outlive them.
    #[must_use]
    pub fn glass(&self, now: Instant) -> Vec<super::blur::Glass> {
        self.motion
            .frames(now)
            .into_iter()
            .filter(|f| f.alpha >= 0.5)
            .filter_map(|f| {
                let item = self.find(f.id)?;
                Some(super::blur::Glass {
                    rect: item.layout.card.translate(f.dx, f.y),
                    radius: item.layout.radius,
                })
            })
            .collect()
    }

    /// Card rects of live toasts at their resting slots (input region).
    #[must_use]
    pub fn input_rects(&self) -> Vec<Rect> {
        self.items
            .iter()
            .map(|i| {
                i.layout
                    .card
                    .translate(0.0, i.target_y)
                    .outset(m::CLOSE_RADIUS * 0.6)
            })
            .collect()
    }

    /// Hit-tests a surface-local point against live toasts.
    #[must_use]
    pub fn hit(&self, p: Point) -> Option<(NotificationId, HitTarget)> {
        self.items.iter().find_map(|i| {
            let local = Point::new(p.x, p.y - i.target_y);
            i.layout
                .scene
                .hit_test(local)
                .map(|t| (i.layout.id, t.clone()))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use super::*;
    use crate::layout::tests::sample_card;

    fn painter() -> std::sync::MutexGuard<'static, Painter> {
        static P: OnceLock<Mutex<Painter>> = OnceLock::new();
        P.get_or_init(|| Mutex::new(Painter::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn toast(id: u32) -> ToastView {
        ToastView {
            card: sample_card(id, "Mail"),
            paused: false,
            expanded: false,
        }
    }

    #[test]
    fn stack_composes_newest_on_top_and_hits() {
        let mut s = ToastStack::default();
        let mut painter = painter();
        let palette = Palette::default();
        let t0 = Instant::now();
        s.sync(&[toast(2), toast(1)], &mut painter, &palette, 1.0, t0);
        let later = t0 + Duration::from_secs(1);
        assert!(!s.is_animating(later));
        let h = s.needed_height(later);
        assert!(h > 2.0 * m::TOAST_HEIGHT + m::TOAST_GAP);
        let rects = s.input_rects();
        assert_eq!(rects.len(), 2);
        assert!(rects[0].bottom() < rects[1].bottom());
        let top = rects[0].center();
        assert_eq!(s.hit(top).map(|(id, _)| id), Some(NotificationId(2)));
        let frame = s.compose(later, 1.0, (380, crate::geom::f32_to_u32(h)));
        let frame = frame.unwrap_or_else(|| panic!("frame"));
        let (cx, cy) = (
            crate::geom::f32_to_u32(top.x),
            crate::geom::f32_to_u32(top.y),
        );
        // The toast's glass is painted there (text may add to it, never less).
        let glass = crate::style::Style::default().toast.alpha * 255.0 * 0.9;
        assert!(
            frame
                .pixel(cx, cy)
                .is_some_and(|p| f32::from(p.alpha()) >= glass)
        );
    }

    #[test]
    fn hovering_rerenders_the_toast_and_ends_when_it_goes() {
        let mut s = ToastStack::new(Timing::default());
        assert!(s.is_empty());
        let mut painter = painter();
        let palette = Palette::default();
        let t0 = Instant::now();
        s.sync(&[toast(1)], &mut painter, &palette, 1.0, t0);
        let before = s.items[0].pixmap.clone();
        let hover = Some((NotificationId(1), HitTarget::Activate(NotificationId(1))));
        assert!(s.set_hover(hover.clone(), &mut painter, &palette, 1.0, t0));
        assert!(
            !s.set_hover(hover.clone(), &mut painter, &palette, 1.0, t0),
            "no change"
        );
        assert_eq!(s.hovered, hover);
        assert_ne!(s.items[0].pixmap, before, "hover tints the glass");
        assert_eq!(s.items.len(), 1);
        // The hovered toast is gone, so nothing is hovered.
        s.sync(&[], &mut painter, &palette, 1.0, t0);
        assert_eq!(s.hovered, None);
    }

    #[test]
    fn a_changed_toast_is_laid_out_again_in_place() {
        let mut s = ToastStack::default();
        let mut painter = painter();
        let palette = Palette::default();
        let t0 = Instant::now();
        s.sync(&[toast(1)], &mut painter, &palette, 1.0, t0);
        let later = t0 + Duration::from_secs(1);
        let compact = s.needed_height(later);
        let expanded = ToastView {
            expanded: true,
            ..toast(1)
        };
        s.sync(
            std::slice::from_ref(&expanded),
            &mut painter,
            &palette,
            1.0,
            later,
        );
        assert_eq!(s.items.len(), 1);
        assert!(s.ghosts.is_empty(), "replaced, not removed");
        assert_eq!(s.items[0].view, expanded);
        let settled = later + Duration::from_secs(1);
        assert!(
            s.needed_height(settled) > compact + 20.0,
            "expanded is taller"
        );
    }

    #[test]
    fn invalidated_toasts_are_rendered_again() {
        let mut s = ToastStack::default();
        let mut painter = painter();
        let palette = Palette::default();
        let t0 = Instant::now();
        s.sync(&[toast(2), toast(1)], &mut painter, &palette, 1.0, t0);
        s.sync(&[toast(2)], &mut painter, &palette, 1.0, t0);
        assert_eq!((s.items.len(), s.ghosts.len()), (1, 1));
        s.invalidate_paint();
        assert!(
            s.items
                .iter()
                .chain(&s.ghosts)
                .all(|i| i.rendered.is_none())
        );
        s.sync(&[toast(2)], &mut painter, &palette, 1.0, t0);
        assert!(s.items[0].rendered.is_some() && s.items[0].pixmap.is_some());
    }

    #[test]
    fn a_returning_toast_replaces_its_fading_ghost() {
        let mut s = ToastStack::default();
        let mut painter = painter();
        let palette = Palette::default();
        let t0 = Instant::now();
        s.sync(&[toast(1)], &mut painter, &palette, 1.0, t0);
        s.sync(&[], &mut painter, &palette, 1.0, t0);
        assert_eq!(s.ghosts.len(), 1);
        s.sync(&[toast(1)], &mut painter, &palette, 1.0, t0);
        assert!(s.ghosts.is_empty());
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn removed_toast_fades_out_then_disappears() {
        let mut s = ToastStack::default();
        let mut painter = painter();
        let palette = Palette::default();
        let t0 = Instant::now();
        s.sync(&[toast(1)], &mut painter, &palette, 1.0, t0);
        let t1 = t0 + Duration::from_secs(1);
        s.sync(&[], &mut painter, &palette, 1.0, t1);
        assert!(!s.is_empty(), "ghost kept while fading");
        assert!(
            s.hit(Point::new(100.0, 40.0)).is_none(),
            "ghosts are not interactive"
        );
        s.reap(t1 + Duration::from_secs(1));
        assert!(s.is_empty());
    }
}
