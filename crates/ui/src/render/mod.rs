//! Painting: [`Scene`] → tiny-skia pixmap at a given scale.
//!
//! Shapes are built directly in physical pixels so edges stay crisp at
//! fractional scales; text and images are rasterised at physical size too.

mod image;
mod sprite;
mod text;

pub use self::image::{ImageCache, Shape};
pub use self::sprite::{SpriteCache, blend};
pub use self::text::TextEngine;

use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pattern,
    Pixmap, PremultipliedColorU8, SpreadMode, Stroke, Transform,
};

use crate::geom::{PxRect, Rect, f32_to_i32, f32_to_u32, physical_len, u32_to_f32};
use crate::scene::{Align, GlyphKind, Layer, Monogram, Node, Scene};
use crate::style::{Fill, Palette, Rgba, TextStyle, fills, monogram_colors};

/// Owns the font, image and layer-sprite caches used for painting.
#[derive(Debug, Default)]
pub struct Painter {
    pub text: TextEngine,
    pub images: ImageCache,
    pub sprites: SpriteCache,
}

/// Physical pixel size of `scene` at `scale`.
#[must_use]
pub fn physical_size(scene: &Scene, scale: f32) -> (u32, u32) {
    (
        physical_len(scene.size.w, scale),
        physical_len(scene.size.h, scale),
    )
}

impl Painter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: TextEngine::new(),
            images: ImageCache::new(),
            sprites: SpriteCache::default(),
        }
    }

    /// Composites a layer's cached sprite, painting it first on a miss.
    fn layer(&mut self, pm: &mut Pixmap, layer: &Layer, ctx: &Ctx<'_>) {
        let (palette, scale) = (ctx.palette, ctx.scale);
        // Taken out so a miss can paint with `self`; layers do not nest in
        // practice, and a nested one would merely go uncached.
        let mut sprites = std::mem::take(&mut self.sprites);
        if let Some(sprite) = sprites.get_or_paint(&layer.scene, palette, scale, || {
            self.paint(&layer.scene, palette, scale)
        }) {
            blend(
                pm,
                sprite,
                f32_to_i32((layer.rect.x * scale).round()),
                f32_to_i32((layer.rect.y * scale).round()),
                layer.opacity * ctx.opacity,
                ctx.clip,
            );
        }
        self.sprites = sprites;
    }

    /// Paints `scene` into a fresh transparent pixmap.
    pub fn paint(&mut self, scene: &Scene, palette: &Palette, scale: f32) -> Option<Pixmap> {
        let (w, h) = physical_size(scene, scale);
        let mut pm = Pixmap::new(w, h)?;
        self.paint_into(&mut pm, scene, palette, scale);
        Some(pm)
    }

    /// Paints `scene` over the existing contents of `pm`.
    pub fn paint_into(&mut self, pm: &mut Pixmap, scene: &Scene, palette: &Palette, scale: f32) {
        let mut clips: Vec<PxRect> = Vec::new();
        let mut mask: Option<Mask> = None;
        let mut opacity: Vec<f32> = Vec::new();
        for node in &scene.nodes {
            match node {
                Node::PushClip(r) => {
                    let px = r.to_physical(scale);
                    let px = clips
                        .last()
                        .map_or(Some(px), |outer| intersect_px(outer, &px));
                    clips.push(px.unwrap_or_default());
                    mask = clips.last().and_then(|c| rect_mask(pm, c));
                }
                Node::PopClip => {
                    clips.pop();
                    mask = clips.last().and_then(|c| rect_mask(pm, c));
                }
                Node::PushOpacity(a) => {
                    let outer = opacity.last().copied().unwrap_or(1.0);
                    opacity.push(outer * a.clamp(0.0, 1.0));
                }
                Node::PopOpacity => {
                    opacity.pop();
                }
                other => {
                    let ctx = Ctx {
                        palette,
                        scale,
                        clip: clips.last().copied(),
                        mask: mask.as_ref(),
                        opacity: opacity.last().copied().unwrap_or(1.0),
                    };
                    self.draw_node(pm, other, &ctx);
                }
            }
        }
    }

    fn draw_node(&mut self, pm: &mut Pixmap, node: &Node, ctx: &Ctx<'_>) {
        match node {
            Node::RoundRect {
                rect,
                radius,
                fill,
                stroke,
            } => shape(pm, *rect, *radius, *fill, *stroke, ctx, ctx.mask),
            Node::Sheet {
                rect,
                radius,
                fill,
                stroke,
                cutout,
                cutout_radius,
            } => {
                let m = cutout_mask(pm, *cutout, *cutout_radius, ctx.scale, ctx.clip.as_ref());
                if let Some(m) = m {
                    shape(pm, *rect, *radius, Some(*fill), *stroke, ctx, Some(&m));
                }
            }
            Node::Shadow {
                rect,
                radius,
                spread,
                offset_y,
                fill,
            } => {
                let shadow = Shadow {
                    rect: *rect,
                    radius: *radius,
                    spread: *spread,
                    offset_y: *offset_y,
                };
                shadow.paint(pm, ctx.color(*fill), ctx);
            }
            Node::Circle {
                center,
                radius,
                fill,
                stroke,
            } => {
                let d = radius * 2.0;
                let r = Rect::new(center.x - radius, center.y - radius, d, d);
                shape(pm, r, *radius, Some(*fill), *stroke, ctx, ctx.mask);
            }
            Node::Text {
                rect,
                text,
                style,
                fill,
                align,
                wrap_width,
            } => self.text.draw(
                &mut pm.as_mut(),
                text,
                *style,
                *wrap_width,
                *rect,
                *align,
                ctx.color(*fill),
                ctx.scale,
                ctx.clip,
            ),
            Node::Image {
                rect,
                radius,
                image,
                fallback,
            } => self.image(pm, *rect, *radius, image.as_ref(), *fallback, ctx),
            Node::Glyph { rect, kind, fill } => glyph(
                pm,
                *rect,
                *kind,
                ctx.color(*fill),
                ctx.scale,
                ctx.clip.as_ref(),
                ctx.mask,
            ),
            Node::Layer(layer) => self.layer(pm, layer, ctx),
            Node::PushClip(_) | Node::PopClip | Node::PushOpacity(_) | Node::PopOpacity => {}
        }
    }

    fn image(
        &mut self,
        pm: &mut Pixmap,
        rect: Rect,
        radius: f32,
        image: Option<&tidings_core::model::Image>,
        fallback: Monogram,
        ctx: &Ctx<'_>,
    ) {
        let (scale, mask) = (ctx.scale, ctx.mask);
        // Snap to whole pixels so the cached raster maps 1:1 (no resampling).
        let x = (rect.x * scale).round();
        let y = (rect.y * scale).round();
        let size = physical_len(rect.w, scale);
        let s = u32_to_f32(size);
        let path = rounded_rect_px(x, y, s, s, radius * scale);
        let loaded = image.and_then(|img| self.images.get(img, size));
        if let (Some(src), Some(path)) = (loaded, path.as_ref()) {
            if src.shape == Shape::Icon {
                // An icon is drawn whole, inset: the plate gives it the
                // round silhouette every avatar shares.
                fill_round_rect(pm, rect, radius, ctx.color(fills::ICON_PLATE), scale, mask);
            }
            let shader = Pattern::new(
                src.pixmap.as_ref(),
                SpreadMode::Pad,
                FilterQuality::Nearest,
                ctx.opacity,
                Transform::from_translate(x, y),
            );
            let paint = Paint {
                shader,
                anti_alias: true,
                ..Paint::default()
            };
            pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), mask);
            return;
        }
        let (bg, fg) = monogram_colors(fallback.tint);
        let (bg, fg) = (bg.faded(ctx.opacity), fg.faded(ctx.opacity));
        fill_round_rect(pm, rect, radius, bg, scale, mask);
        let lh = TextStyle::IconLetter.line_height();
        let tr = Rect::new(rect.x, rect.y + (rect.h - lh) / 2.0, rect.w, lh);
        self.text.draw(
            &mut pm.as_mut(),
            &fallback.letter.to_string(),
            TextStyle::IconLetter,
            rect.w,
            tr,
            Align::Center,
            fg,
            scale,
            ctx.clip,
        );
    }
}

/// Composites `src` onto `dst` at physical offset `(x, y)` with `opacity`.
pub fn blit(dst: &mut Pixmap, src: &Pixmap, x: i32, y: i32, opacity: f32) {
    blend(dst, src, x, y, opacity, None);
}

fn color(c: Rgba) -> Color {
    Color::from_rgba(c.r, c.g, c.b, c.a).unwrap_or(Color::TRANSPARENT)
}

fn solid(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(c));
    p.anti_alias = true;
    p
}

fn intersect_px(a: &PxRect, b: &PxRect) -> Option<PxRect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (i64::from(a.x) + i64::from(a.w)).min(i64::from(b.x) + i64::from(b.w));
    let y1 = (i64::from(a.y) + i64::from(a.h)).min(i64::from(b.y) + i64::from(b.h));
    let w = u32::try_from(x1 - i64::from(x0)).ok()?;
    let h = u32::try_from(y1 - i64::from(y0)).ok()?;
    Some(PxRect { x: x0, y: y0, w, h })
}

fn px_rect_path(r: &PxRect) -> Option<Path> {
    let rect = tiny_skia::Rect::from_xywh(
        crate::geom::i32_to_f32(r.x),
        crate::geom::i32_to_f32(r.y),
        u32_to_f32(r.w),
        u32_to_f32(r.h),
    )?;
    Some(PathBuilder::from_rect(rect))
}

fn rect_mask(pm: &Pixmap, clip: &PxRect) -> Option<Mask> {
    let mut m = Mask::new(pm.width(), pm.height())?;
    m.fill_path(
        &px_rect_path(clip)?,
        FillRule::Winding,
        false,
        Transform::identity(),
    );
    Some(m)
}

/// Mask covering everything except a rounded `cutout` (and outside `clip`).
fn cutout_mask(
    pm: &Pixmap,
    cutout: Rect,
    radius: f32,
    scale: f32,
    clip: Option<&PxRect>,
) -> Option<Mask> {
    let mut m = Mask::new(pm.width(), pm.height())?;
    let path = rounded_rect_px(
        cutout.x * scale,
        cutout.y * scale,
        cutout.w * scale,
        cutout.h * scale,
        radius * scale,
    )?;
    m.fill_path(&path, FillRule::Winding, true, Transform::identity());
    m.invert();
    if let Some(c) = clip {
        m.intersect_path(
            &px_rect_path(c)?,
            FillRule::Winding,
            false,
            Transform::identity(),
        );
    }
    Some(m)
}

/// Rounded rectangle path in physical pixels (cubic-approximated corners).
#[allow(clippy::many_single_char_names, reason = "conventional geometry names")]
fn rounded_rect_px(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    // Control-point distance for a quarter circle.
    let k = r * 0.552_284_8;
    let (r0, b0) = (x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(r0 - r, y);
    pb.cubic_to(r0 - r + k, y, r0, y + r - k, r0, y + r);
    pb.line_to(r0, b0 - r);
    pb.cubic_to(r0, b0 - r + k, r0 - r + k, b0, r0 - r, b0);
    pb.line_to(x + r, b0);
    pb.cubic_to(x + r - k, b0, x, b0 - r + k, x, b0 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Per-node paint context: theme, scale, the active clip and opacity.
struct Ctx<'a> {
    palette: &'a Palette,
    scale: f32,
    clip: Option<PxRect>,
    mask: Option<&'a Mask>,
    /// Product of the enclosing opacity groups.
    opacity: f32,
}

impl Ctx<'_> {
    /// `fill` resolved against the theme and faded by the group opacity.
    fn color(&self, fill: Fill) -> Rgba {
        self.palette.resolve(fill).faded(self.opacity)
    }
}

/// A soft shadow approximated by stacked translucent rounded rects, each a
/// little larger, so opacity falls off roughly linearly with distance.
struct Shadow {
    rect: Rect,
    radius: f32,
    spread: f32,
    offset_y: f32,
}

impl Shadow {
    /// Paints the shadow as a Gaussian-blurred copy of the shape (shifted
    /// down by `offset_y`), computed per pixel from the distance to the
    /// shape's rounded edge. At the edge it is half of `peak`, as a real blur
    /// of a solid shape is, and it fades smoothly over `spread` (three
    /// standard deviations): a soft drop shadow, not a dark ring hugging
    /// the outline.
    fn paint(&self, pm: &mut Pixmap, peak: Rgba, ctx: &Ctx<'_>) {
        if peak.a <= 0.0 || self.spread <= 0.0 {
            return;
        }
        // Never paint under the shape itself.
        let Some(mask) = cutout_mask(pm, self.rect, self.radius, ctx.scale, ctx.clip.as_ref())
        else {
            return;
        };
        let scale = ctx.scale;
        let base = self.rect.translate(0.0, self.offset_y);
        let (cx, cy) = (
            (base.x + base.w / 2.0) * scale,
            (base.y + base.h / 2.0) * scale,
        );
        let (hw, hh) = (base.w / 2.0 * scale, base.h / 2.0 * scale);
        let radius = (self.radius * scale).min(hw).min(hh);
        let reach = self.spread * scale;
        let sigma = reach / 3.0;

        let (width, height) = (pm.width(), pm.height());
        let span = |lo: f32, hi: f32, max: u32| {
            (
                f32_to_u32(lo.floor().max(0.0)).min(max),
                f32_to_u32(hi.ceil().max(0.0)).min(max),
            )
        };
        let (x0, x1) = span(cx - hw - reach, cx + hw + reach, width);
        let (y0, y1) = span(cy - hh - reach, cy + hh + reach, height);
        let coverage_mask = mask.data();
        let pixels = pm.pixels_mut();
        for y in y0..y1 {
            for x in x0..x1 {
                let Ok(idx) = usize::try_from(y * width + x) else {
                    continue;
                };
                let inside = coverage_mask.get(idx).copied().unwrap_or(0);
                if inside == 0 {
                    continue;
                }
                let d = rounded_rect_distance(
                    u32_to_f32(x) + 0.5 - cx,
                    u32_to_f32(y) + 0.5 - cy,
                    hw,
                    hh,
                    radius,
                );
                if d > reach {
                    continue;
                }
                let a = peak.a * blurred_edge_coverage(d, sigma) * f32::from(inside) / 255.0;
                if let Some(px) = pixels.get_mut(idx) {
                    *px = blend_over(*px, peak, a);
                }
            }
        }
    }
}

/// Signed distance from a point (relative to the centre) to a rounded rect
/// with half-extents `hw` x `hh` and corner `radius`: negative inside.
fn rounded_rect_distance(x: f32, y: f32, hw: f32, hh: f32, radius: f32) -> f32 {
    let qx = x.abs() - (hw - radius);
    let qy = y.abs() - (hh - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

/// How much of a Gaussian-blurred (standard deviation `sigma`) solid edge
/// covers a point `d` outside it: 0.5 on the edge, toward 0 further out.
fn blurred_edge_coverage(d: f32, sigma: f32) -> f32 {
    0.5 * erfc(d / (sigma * std::f32::consts::SQRT_2))
}

/// Complementary error function (Abramowitz & Stegun 7.1.26, |error| < 1.5e-7).
fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let tail = poly * (-z * z).exp();
    if x >= 0.0 { tail } else { 2.0 - tail }
}

/// Source-over of `color` at opacity `a` onto a premultiplied pixel.
fn blend_over(dst: PremultipliedColorU8, color: Rgba, a: f32) -> PremultipliedColorU8 {
    let a = a.clamp(0.0, 1.0);
    let keep = 1.0 - a;
    let channel = |src: f32, dst: u8| unit_to_u8(src * a + f32::from(dst) / 255.0 * keep);
    let out_a = unit_to_u8(a + f32::from(dst.alpha()) / 255.0 * keep);
    let r = channel(color.r, dst.red()).min(out_a);
    let g = channel(color.g, dst.green()).min(out_a);
    let b = channel(color.b, dst.blue()).min(out_a);
    PremultipliedColorU8::from_rgba(r, g, b, out_a).unwrap_or(dst)
}

/// 0.0..=1.0 to a byte, rounded.
fn unit_to_u8(v: f32) -> u8 {
    u8::try_from(f32_to_u32((v.clamp(0.0, 1.0) * 255.0).round())).unwrap_or(u8::MAX)
}

/// Fill and/or 1 px stroke of a rounded rect.
fn shape(
    pm: &mut Pixmap,
    rect: Rect,
    radius: f32,
    fill: Option<Fill>,
    stroke: Option<Fill>,
    ctx: &Ctx<'_>,
    mask: Option<&Mask>,
) {
    if let Some(f) = fill {
        fill_round_rect(pm, rect, radius, ctx.color(f), ctx.scale, mask);
    }
    if let Some(s) = stroke {
        stroke_round_rect(pm, rect, radius, ctx.color(s), ctx.scale, mask);
    }
}

fn fill_round_rect(
    pm: &mut Pixmap,
    rect: Rect,
    radius: f32,
    c: Rgba,
    scale: f32,
    mask: Option<&Mask>,
) {
    if c.a <= 0.0 {
        return;
    }
    if let Some(path) = rounded_rect_px(
        rect.x * scale,
        rect.y * scale,
        rect.w * scale,
        rect.h * scale,
        radius * scale,
    ) {
        pm.fill_path(
            &path,
            &solid(c),
            FillRule::Winding,
            Transform::identity(),
            mask,
        );
    }
}

/// 1 logical px stroke drawn just inside the shape's edge.
fn stroke_round_rect(
    pm: &mut Pixmap,
    rect: Rect,
    radius: f32,
    c: Rgba,
    scale: f32,
    mask: Option<&Mask>,
) {
    if c.a <= 0.0 {
        return;
    }
    let width = scale.max(1.0);
    let half = width / 2.0;
    let path = rounded_rect_px(
        rect.x * scale + half,
        rect.y * scale + half,
        rect.w * scale - width,
        rect.h * scale - width,
        (radius * scale - half).max(0.0),
    );
    if let Some(path) = path {
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        pm.stroke_path(&path, &solid(c), &stroke, Transform::identity(), mask);
    }
}

#[allow(clippy::many_single_char_names, reason = "conventional geometry names")]
fn glyph(
    pm: &mut Pixmap,
    rect: Rect,
    kind: GlyphKind,
    c: Rgba,
    scale: f32,
    clip: Option<&PxRect>,
    mask: Option<&Mask>,
) {
    let (x, y, w, h) = (
        rect.x * scale,
        rect.y * scale,
        rect.w * scale,
        rect.h * scale,
    );
    match kind {
        GlyphKind::Close => {
            let mut pb = PathBuilder::new();
            pb.move_to(x, y);
            pb.line_to(x + w, y + h);
            pb.move_to(x + w, y);
            pb.line_to(x, y + h);
            let Some(path) = pb.finish() else { return };
            let stroke = Stroke {
                width: 1.6 * scale,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };
            pm.stroke_path(&path, &solid(c), &stroke, Transform::identity(), mask);
        }
        GlyphKind::ChevronDown | GlyphKind::ChevronUp => {
            // A 24-unit icon grid, like the SVG it mirrors: M6 9 l6 6 6-6.
            let u = w.min(h) / 24.0;
            let (top, bottom) = if kind == GlyphKind::ChevronDown {
                (9.0, 15.0)
            } else {
                (15.0, 9.0)
            };
            let mut pb = PathBuilder::new();
            pb.move_to(x + 6.0 * u, y + top * u);
            pb.line_to(x + 12.0 * u, y + bottom * u);
            pb.line_to(x + 18.0 * u, y + top * u);
            let Some(path) = pb.finish() else { return };
            let stroke = Stroke {
                width: 2.5 * u,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            pm.stroke_path(&path, &solid(c), &stroke, Transform::identity(), mask);
        }
        GlyphKind::Moon => {
            // Crescent: a disc with an offset disc cut away.
            let r = w.min(h) / 2.0;
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let Some(mut m) = Mask::new(pm.width(), pm.height()) else {
                return;
            };
            let Some(bite) = PathBuilder::from_circle(cx + r * 0.55, cy - r * 0.45, r * 0.85)
            else {
                return;
            };
            m.fill_path(&bite, FillRule::Winding, true, Transform::identity());
            m.invert();
            if let Some(path) = clip.and_then(px_rect_path) {
                m.intersect_path(&path, FillRule::Winding, false, Transform::identity());
            }
            if let Some(disc) = PathBuilder::from_circle(cx, cy, r) {
                pm.fill_path(
                    &disc,
                    &solid(c),
                    FillRule::Winding,
                    Transform::identity(),
                    Some(&m),
                );
            }
        }
    }
}

/// Approximate equality of a premultiplied pixel and a straight colour.
#[cfg(test)]
fn close_to(p: tiny_skia::PremultipliedColorU8, c: Rgba, tol: u8) -> bool {
    use crate::geom::f32_to_u32;
    let want = |v: f32| f32_to_u32((v * c.a * 255.0).round());
    let ok = |got: u8, w: u32| u32::from(got).abs_diff(w) <= u32::from(tol);
    ok(p.red(), want(c.r))
        && ok(p.green(), want(c.g))
        && ok(p.blue(), want(c.b))
        && ok(p.alpha(), f32_to_u32((c.a * 255.0).round()))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::OnceLock;

    use tidings_core::model::{Action, NotificationId, Urgency};
    use tidings_core::theme::Rgb;
    use tidings_core::view::{CardView, GroupView, ViewModel};

    use super::*;
    use crate::geom::Size;
    use crate::layout::{Interaction, layout_panel};
    use crate::style::{Role, default_theme};

    /// Font scanning is slow; share one painter across tests.
    fn painter() -> std::sync::MutexGuard<'static, Painter> {
        static P: OnceLock<Mutex<Painter>> = OnceLock::new();
        P.get_or_init(|| Mutex::new(Painter::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn view() -> ViewModel {
        let card = |id| CardView {
            id: NotificationId(id),
            app_name: "Mail".into(),
            icon: None,
            image: None,
            summary: "Hello".into(),
            body: "World".into(),
            time_label: "now".into(),
            actions: vec![Action {
                key: "a".into(),
                label: "Open".into(),
            }],
            clickable: true,
            urgency: Urgency::Normal,
        };
        ViewModel {
            groups: vec![GroupView {
                key: "mail".into(),
                app_name: "Mail".into(),
                icon: None,
                cards: vec![card(2), card(1)],
                expanded: false,
            }],
            toasts: vec![],
            dnd: true,
        }
    }

    fn render(theme: tidings_core::theme::Theme, scale: f32) -> (Pixmap, Palette) {
        let mut p = painter();
        let palette = Palette::new(theme);
        let lay = layout_panel(
            &view(),
            Size::new(380.0, 600.0),
            scale,
            &Interaction::default(),
            &mut p.text,
        );
        let pm = p.paint(&lay.scene, &palette, scale);
        (pm.unwrap_or_else(|| panic!("pixmap")), palette)
    }

    #[test]
    fn opacity_groups_fade_what_they_wrap() {
        let mut p = painter();
        let palette = Palette::new(default_theme());
        let square = Node::RoundRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            radius: 0.0,
            fill: Some(Fill::solid(Role::Surface)),
            stroke: None,
        };
        let alpha_at = |scene: &Scene, p: &mut Painter| {
            p.paint(scene, &palette, 1.0)
                .and_then(|pm| pm.pixel(5, 5))
                .map_or(0, tiny_skia::PremultipliedColorU8::alpha)
        };
        let mut s = Scene::new(Size::new(10.0, 10.0));
        s.push(square.clone());
        assert_eq!(alpha_at(&s, &mut p), 255);
        let mut s = Scene::new(Size::new(10.0, 10.0));
        s.with_opacity(0.5, |s| s.with_opacity(0.5, |s| s.push(square.clone())));
        assert!((60..=68).contains(&alpha_at(&s, &mut p)), "nested 0.25");
    }

    #[test]
    fn renders_non_empty_at_physical_size() {
        let (pm, _) = render(default_theme(), 1.6);
        assert_eq!((pm.width(), pm.height()), (608, 960));
        assert!(pm.pixels().iter().filter(|p| p.alpha() > 0).count() > 1000);
        // Corners outside the 20 px radius stay transparent.
        assert_eq!(
            pm.pixel(1, 1).map(tiny_skia::PremultipliedColorU8::alpha),
            Some(0)
        );
    }

    #[test]
    fn theme_colours_are_applied() {
        let mut theme = default_theme();
        theme.surface = Rgb(200, 20, 20);
        theme.surface_alpha = 0.8;
        let (pm, palette) = render(theme, 1.0);
        // Below the content, inside the sheet: pure glass.
        let px = pm.pixel(190, 560);
        let want = palette.resolve(fills::GLASS);
        assert!(
            px.is_some_and(|p| close_to(p, want, 2)),
            "{px:?} vs {want:?}"
        );
    }

    #[test]
    fn blit_respects_opacity() {
        let mut src = Pixmap::new(4, 4).unwrap_or_else(|| panic!("pixmap"));
        src.fill(Color::WHITE);
        let mut dst = Pixmap::new(8, 8).unwrap_or_else(|| panic!("pixmap"));
        blit(&mut dst, &src, 2, 2, 0.5);
        assert_eq!(
            dst.pixel(0, 0).map(tiny_skia::PremultipliedColorU8::alpha),
            Some(0)
        );
        let a = dst
            .pixel(3, 3)
            .map_or(0, tiny_skia::PremultipliedColorU8::alpha);
        assert!((126..=129).contains(&a));
    }

    #[test]
    fn rounded_paths_are_valid() {
        assert!(rounded_rect_px(0.0, 0.0, 10.0, 10.0, 50.0).is_some());
        assert!(rounded_rect_px(0.0, 0.0, 0.0, 10.0, 2.0).is_none());
    }

    #[test]
    fn rounded_rect_distance_is_signed() {
        // 20x10 rect (half-extents 10 x 5), corner radius 2.
        let d = |x, y| rounded_rect_distance(x, y, 10.0, 5.0, 2.0);
        assert!((d(13.0, 0.0) - 3.0).abs() < 1e-4, "right of the edge");
        assert!((d(0.0, -7.0) - 2.0).abs() < 1e-4, "above the edge");
        assert!((d(0.0, 0.0) + 5.0).abs() < 1e-4, "centre is inside");
        // The corner is round: diagonal distance from the corner centre (8, 3).
        let expect = 4.0f32.hypot(4.0) - 2.0;
        assert!((d(12.0, 7.0) - expect).abs() < 1e-4);
    }

    #[test]
    fn blurred_edge_is_half_on_the_edge_and_fades_out() {
        assert!((erfc(0.0) - 1.0).abs() < 1e-6);
        assert!((erfc(1.0) - 0.157_299_2).abs() < 1e-5);
        assert!((erfc(-1.0) - 1.842_700_8).abs() < 1e-5);
        let c = |d| blurred_edge_coverage(d, 3.0);
        assert!((c(0.0) - 0.5).abs() < 1e-6);
        assert!(c(-3.0) > 0.8 && c(3.0) < 0.2);
        assert!(c(9.0) < 0.002, "faded out at three sigma");
        assert!(c(1.0) > c(2.0) && c(2.0) > c(4.0));
    }

    #[test]
    fn shadow_is_a_soft_drop_shadow_not_a_ring() {
        let card = Rect::new(20.0, 20.0, 100.0, 40.0);
        let mut scene = Scene::new(Size::new(140.0, 80.0));
        scene.push(Node::Shadow {
            rect: card,
            radius: 20.0,
            spread: 9.0,
            offset_y: 3.0,
            fill: Fill::new(crate::style::Role::Shadow, 0.3),
        });
        let pm = painter()
            .paint(&scene, &Palette::default(), 1.0)
            .unwrap_or_else(|| panic!("pixmap"));
        let alpha = |x: u32, y: u32| {
            pm.pixel(x, y)
                .map_or(0, tiny_skia::PremultipliedColorU8::alpha)
        };
        // Below the card: strongest next to it, fading steadily outward.
        let below: Vec<u8> = (60..70).map(|y| alpha(70, y)).collect();
        assert!(below.windows(2).all(|w| w[0] >= w[1]), "{below:?}");
        assert!(below[0] <= 77, "no more than ~0.3 even below: {}", below[0]);
        // Offset down: lighter above the card than below it.
        assert!(alpha(70, 18) < alpha(70, 61));
        // Beside the card, 1.5 px out (pixel centre): the blurred-edge value.
        let side = f32::from(alpha(121, 40));
        let expect = 255.0 * 0.3 * blurred_edge_coverage(1.5, 3.0);
        assert!((side - expect).abs() <= 3.0, "{side} vs {expect}");
        // Never under the card itself.
        assert_eq!(alpha(70, 40), 0);
    }
}
