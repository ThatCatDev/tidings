//! Panel state: openness motion, interaction, cached content and per-frame
//! composition. Independent of Wayland objects so it can be tested offscreen.

use std::time::Instant;

use tidings_core::view::ViewModel;
use tiny_skia::Pixmap;

use crate::anim::{GroupMotion, PanelMotion, panel_opacity};
use crate::geom::{Point, PxRect, Rect, Size, f32_to_i32, i32_to_f32};
use crate::layout::{Interaction, PanelLayout, layout_panel};
use crate::render::{Painter, blit};
use crate::scene::HitTarget;
use crate::style::{Palette, metrics as m};

/// Panel surface width: the sheet plus the right margin it slides across.
pub const SURFACE_WIDTH: f32 = m::PANEL_WIDTH + m::PANEL_MARGIN_RIGHT;

/// What changed since the last composed frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Composed {
    offset_px: i32,
    opacity: f32,
    size: (u32, u32),
}

/// The panel's renderer-side state.
#[derive(Debug, Default)]
pub struct PanelView {
    pub motion: PanelMotion,
    /// Groups fanning out of (or folding into) their stacks.
    pub groups: GroupMotion,
    pub ui: Interaction,
    layout: Option<PanelLayout>,
    /// The sheet rendered at `content_key`.
    content: Option<Pixmap>,
    content_key: Option<(u32, Size)>,
    /// Layout must be redone (view, hover or scroll changed).
    pub needs_layout: bool,
    /// Sheet-space rects changed since the last compose.
    damage: Vec<Rect>,
    composed: Option<Composed>,
}

impl PanelView {
    /// Horizontal offset of the sheet inside the surface, in logical px.
    #[must_use]
    pub fn offset(&self) -> f32 {
        (1.0 - self.motion.openness()) * SURFACE_WIDTH
    }

    /// Forgets everything tied to a mapped surface (called on unmap).
    pub fn reset_surface_state(&mut self) {
        self.layout = None;
        self.content = None;
        self.content_key = None;
        self.composed = None;
        self.damage.clear();
        self.ui = Interaction::default();
    }

    /// Drops cached pixels so the next refresh repaints (theme change).
    pub fn invalidate_paint(&mut self) {
        self.content_key = None;
    }

    /// Advances group morphs to `now`, then re-lays out and repaints the
    /// sheet if anything it depends on changed.
    pub fn refresh(
        &mut self,
        view: &ViewModel,
        painter: &mut Painter,
        palette: &Palette,
        surface_h: f32,
        scale: f32,
        now: Instant,
    ) {
        self.groups.sync(
            view.groups.iter().map(|g| (g.key.as_str(), g.expanded)),
            now,
        );
        let morph = self.groups.frame(now);
        if morph != self.ui.morph {
            self.ui.morph = morph;
            self.needs_layout = true;
        }
        let size = Size::new(m::PANEL_WIDTH, surface_h);
        let key = (scale.to_bits(), size);
        if !self.needs_layout && self.content_key == Some(key) && self.content.is_some() {
            return;
        }
        let lay = layout_panel(view, size, scale, &self.ui, &mut painter.text);
        self.ui.scroll = lay.scroll;
        let damage = match (&self.layout, self.content_key == Some(key)) {
            (Some(old), true) => old.scene.damage_to(&lay.scene),
            _ => vec![Rect::new(0.0, 0.0, size.w, size.h)],
        };
        if damage.is_empty() && self.content.is_some() {
            self.layout = Some(lay);
            self.needs_layout = false;
            return;
        }
        self.content = painter.paint(&lay.scene, palette, scale);
        self.content_key = Some(key);
        self.damage.extend(damage);
        self.layout = Some(lay);
        self.needs_layout = false;
    }

    /// Whether another frame is needed: the panel is sliding or a group is
    /// morphing.
    #[must_use]
    pub fn is_animating(&self, now: Instant) -> bool {
        self.motion.is_animating() || self.groups.is_animating(now)
    }

    /// Composes the surface frame (`phys` = surface size in device px).
    /// Returns the frame and the damaged device-pixel rects.
    pub fn compose(&mut self, scale: f32, phys: (u32, u32)) -> Option<(Pixmap, Vec<PxRect>)> {
        let content = self.content.as_ref()?;
        let mut frame = Pixmap::new(phys.0, phys.1)?;
        let offset_px = f32_to_i32((self.offset() * scale).round());
        let opacity = panel_opacity(self.motion.openness());
        blit(&mut frame, content, offset_px, 0, opacity);
        let now = Composed {
            offset_px,
            opacity,
            size: phys,
        };
        let damage = if self.composed == Some(now) {
            let off = i32_to_f32(offset_px) / scale;
            self.damage
                .iter()
                .map(|r| r.translate(off, 0.0).to_physical(scale))
                .collect()
        } else {
            // Moved or faded: everything changed.
            vec![PxRect {
                x: 0,
                y: 0,
                w: phys.0,
                h: phys.1,
            }]
        };
        self.composed = Some(now);
        self.damage.clear();
        Some((frame, damage))
    }

    /// The visible part of the sheet, in surface coordinates.
    #[must_use]
    pub fn input_rect(&self, surface_h: f32) -> Rect {
        let x = self.offset();
        Rect::new(
            x,
            0.0,
            (m::PANEL_WIDTH).min(SURFACE_WIDTH - x).max(0.0),
            surface_h,
        )
    }

    /// Hit-tests a surface-local point.
    #[must_use]
    pub fn hit(&self, p: Point) -> Option<HitTarget> {
        let lay = self.layout.as_ref()?;
        lay.scene
            .hit_test(Point::new(p.x - self.offset(), p.y))
            .cloned()
    }

    /// Scrolls the list by `dy` logical px; returns whether it moved.
    pub fn scroll_by(&mut self, dy: f32) -> bool {
        let max = self.layout.as_ref().map_or(0.0, |l| l.max_scroll);
        let next = (self.ui.scroll + dy).clamp(0.0, max);
        let moved = (next - self.ui.scroll).abs() > f32::EPSILON;
        if moved {
            self.ui.scroll = next;
            self.needs_layout = true;
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    use super::*;

    fn painter() -> std::sync::MutexGuard<'static, Painter> {
        static P: OnceLock<Mutex<Painter>> = OnceLock::new();
        P.get_or_init(|| Mutex::new(Painter::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn half_open_panel_draws_only_visible_part() {
        let mut p = PanelView::default();
        p.motion.track(0.5, Instant::now());
        let mut painter = painter();
        p.refresh(
            &ViewModel::default(),
            &mut painter,
            &Palette::default(),
            400.0,
            1.0,
            Instant::now(),
        );
        let (frame, damage) = p
            .compose(1.0, (388, 400))
            .unwrap_or_else(|| panic!("frame"));
        assert_eq!(damage.len(), 1, "first frame damages everything");
        // Left of the offset is transparent; right of it has glass.
        assert_eq!(
            frame
                .pixel(100, 200)
                .map(tiny_skia::PremultipliedColorU8::alpha),
            Some(0)
        );
        assert!(frame.pixel(250, 200).is_some_and(|c| c.alpha() > 0));
        let r = p.input_rect(400.0);
        assert!((r.x - 194.0).abs() < 0.01 && (r.w - 194.0).abs() < 0.01);
        assert_eq!(p.hit(Point::new(100.0, 200.0)), None);
        assert_eq!(p.hit(Point::new(300.0, 300.0)), Some(HitTarget::Background));
    }

    #[test]
    fn unchanged_hover_frames_damage_little() {
        let mut p = PanelView::default();
        p.motion.track(1.0, Instant::now());
        let mut painter = painter();
        let palette = Palette::default();
        let view = ViewModel::default();
        p.refresh(&view, &mut painter, &palette, 400.0, 1.0, Instant::now());
        p.compose(1.0, (388, 400));
        // Hovering DND only repaints the pill.
        p.ui.hovered = Some(HitTarget::ToggleDnd);
        p.needs_layout = true;
        p.refresh(&view, &mut painter, &palette, 400.0, 1.0, Instant::now());
        let (_, damage) = p
            .compose(1.0, (388, 400))
            .unwrap_or_else(|| panic!("frame"));
        assert_eq!(damage.len(), 1);
        assert!(damage[0].w < 80 && damage[0].h < 60, "{damage:?}");
    }
}
