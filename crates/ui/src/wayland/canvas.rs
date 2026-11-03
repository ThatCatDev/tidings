//! One layer surface plus everything needed to put pixels on it at a
//! fractional scale: a viewport, the fractional-scale object, an shm pool
//! and frame-callback bookkeeping.

use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::reexports::client::protocol::wl_shm;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerSurface,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use tiny_skia::Pixmap;

use super::{App, SurfaceId};
use crate::UiError;
use crate::geom::{PxRect, Rect, f32_to_i32, physical_len};

/// Static description of a layer surface.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceSpec {
    pub id: SurfaceId,
    pub namespace: &'static str,
    pub layer: Layer,
    pub anchor: Anchor,
    /// Requested logical size; 0 lets the compositor pick (anchored axes).
    pub size: (u32, u32),
    /// top, right, bottom, left.
    pub margin: (i32, i32, i32, i32),
    pub keyboard: KeyboardInteractivity,
}

/// A mapped (or mapping) layer surface.
#[derive(Debug)]
pub struct Canvas {
    pub id: SurfaceId,
    pub layer: LayerSurface,
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    pool: SlotPool,
    buffer: Option<Buffer>,
    format: wl_shm::Format,
    /// Logical size from the last configure.
    pub size: (u32, u32),
    pub scale: f32,
    pub configured: bool,
    /// A frame callback is outstanding; drawing waits for it.
    pub frame_pending: bool,
    /// Something changed and a redraw is due at the next opportunity.
    pub dirty: bool,
    /// The keyboard mode the surface was created with, and the current one.
    default_keyboard: KeyboardInteractivity,
    keyboard: KeyboardInteractivity,
}

impl Canvas {
    pub fn create(app: &App, qh: &QueueHandle<App>, spec: &SurfaceSpec) -> Result<Self, UiError> {
        let surface = app.compositor.create_surface(qh);
        let fractional = app
            .fractional
            .as_ref()
            .map(|m| m.get_fractional_scale(&surface, qh, spec.id));
        let viewport = app
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, qh, ()));
        let layer = app.layer_shell.create_layer_surface(
            qh,
            surface,
            spec.layer,
            Some(spec.namespace),
            None,
        );
        layer.set_anchor(spec.anchor);
        layer.set_size(spec.size.0, spec.size.1);
        let (t, r, b, l) = spec.margin;
        layer.set_margin(t, r, b, l);
        // Position against the screen edges, ignoring bars' exclusive zones:
        // the margins already account for the bar.
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(spec.keyboard);
        // With a viewport, buffers are sized in device pixels and scaled
        // down; without one, fall back to integer buffer scale 1.
        layer.commit();
        let pool =
            SlotPool::new(256 * 256 * 4, &app.shm).map_err(|e| UiError::Buffer(e.to_string()))?;
        Ok(Self {
            id: spec.id,
            layer,
            viewport,
            fractional,
            pool,
            buffer: None,
            format: app.shm_format,
            size: (spec.size.0, spec.size.1),
            scale: if app.viewporter.is_some() {
                app.last_scale
            } else {
                1.0
            },
            configured: false,
            frame_pending: false,
            dirty: true,
            default_keyboard: spec.keyboard,
            keyboard: spec.keyboard,
        })
    }

    /// Updates the preferred scale; returns whether it changed.
    pub fn set_scale(&mut self, scale: f32) -> bool {
        // Without a viewport we cannot present fractional buffers.
        let scale = if self.viewport.is_some() { scale } else { 1.0 };
        let changed = (scale - self.scale).abs() > f32::EPSILON;
        self.scale = scale;
        changed
    }

    /// Logical size as floats.
    pub fn logical(&self) -> (f32, f32) {
        (
            crate::geom::u32_to_f32(self.size.0),
            crate::geom::u32_to_f32(self.size.1),
        )
    }

    /// Buffer size in device pixels.
    pub fn physical(&self) -> (u32, u32) {
        let (w, h) = self.logical();
        (physical_len(w, self.scale), physical_len(h, self.scale))
    }

    /// Ready to draw: configured and not waiting on a frame callback.
    pub fn can_draw(&self) -> bool {
        self.configured && !self.frame_pending
    }

    /// Presents `frame` (must be [`Self::physical`]-sized), damaging only
    /// `damage` (physical px; empty = whole surface), and requests a frame
    /// callback so the next draw is paced by the compositor.
    pub fn present(
        &mut self,
        qh: &QueueHandle<App>,
        frame: &Pixmap,
        damage: &[PxRect],
    ) -> Result<(), UiError> {
        let (w, h) = (frame.width(), frame.height());
        if (w, h) != self.physical() {
            log::debug!("tidings-ui: dropping stale {w}x{h} frame for {:?}", self.id);
            return Ok(());
        }
        let format = self.format;
        let data = self.acquire(w, h)?;
        match format {
            // ABGR8888 is R,G,B,A in memory: tiny-skia's layout, copy as-is.
            wl_shm::Format::Abgr8888 => data.copy_from_slice(frame.data()),
            _ => {
                for (d, s) in data
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(frame.data().as_chunks::<4>().0)
                {
                    d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
                }
            }
        }
        self.attach_and_commit(qh, w, h, damage)
    }

    /// Presents a fully transparent surface (the click catcher). With a
    /// viewport this is a 1x1 buffer stretched to the surface size.
    pub fn present_clear(&mut self, qh: &QueueHandle<App>) -> Result<(), UiError> {
        let (w, h) = if self.viewport.is_some() {
            (1, 1)
        } else {
            self.size
        };
        let data = self.acquire(w.max(1), h.max(1))?;
        data.fill(0);
        self.attach_and_commit(qh, w.max(1), h.max(1), &[])
    }

    fn acquire(&mut self, w: u32, h: u32) -> Result<&mut [u8], UiError> {
        let (wi, hi) = (int(w)?, int(h)?);
        let stride = wi
            .checked_mul(4)
            .ok_or_else(|| UiError::Buffer("stride overflow".into()))?;
        let reusable = self.buffer.as_ref().is_some_and(|b| {
            b.height() == hi && b.stride() == stride && b.canvas(&mut self.pool).is_some()
        });
        if !reusable {
            // A busy or mis-sized buffer is dropped; SCTK destroys it once
            // the compositor releases it.
            let (b, _) = self
                .pool
                .create_buffer(wi, hi, stride, self.format)
                .map_err(|e| UiError::Buffer(e.to_string()))?;
            self.buffer = Some(b);
        }
        self.buffer
            .as_ref()
            .and_then(|b| b.canvas(&mut self.pool))
            .ok_or_else(|| UiError::Buffer("fresh buffer has no canvas".into()))
    }

    fn attach_and_commit(
        &mut self,
        qh: &QueueHandle<App>,
        w: u32,
        h: u32,
        damage: &[PxRect],
    ) -> Result<(), UiError> {
        let surface = self.layer.wl_surface();
        let Some(buffer) = self.buffer.as_ref() else {
            return Ok(());
        };
        buffer
            .attach_to(surface)
            .map_err(|e| UiError::Buffer(e.to_string()))?;
        if damage.is_empty() {
            surface.damage_buffer(0, 0, int(w)?, int(h)?);
        } else {
            for d in damage.iter().filter_map(|d| d.clamp_to(w, h)) {
                surface.damage_buffer(d.x, d.y, int(d.w)?, int(d.h)?);
            }
        }
        if let Some(vp) = &self.viewport {
            vp.set_destination(int(self.size.0.max(1))?, int(self.size.1.max(1))?);
        }
        surface.frame(qh, surface.clone());
        self.frame_pending = true;
        self.dirty = false;
        self.layer.commit();
        Ok(())
    }

    /// Switches the surface's keyboard mode (`None` = the one it was
    /// created with). Takes effect on commit, which this does.
    pub fn set_keyboard(&mut self, mode: Option<KeyboardInteractivity>) {
        let mode = mode.unwrap_or(self.default_keyboard);
        if mode != self.keyboard {
            self.keyboard = mode;
            self.layer.set_keyboard_interactivity(mode);
            self.layer.commit();
        }
    }

    /// Restricts input to `rects` (logical px). An empty list means no input.
    pub fn set_input(&self, compositor: &CompositorState, rects: &[Rect]) {
        log::trace!("tidings-ui: input region of {:?}: {rects:?}", self.id);
        match Region::new(compositor) {
            Ok(region) => {
                for r in rects {
                    let x = f32_to_i32(r.x.floor());
                    let y = f32_to_i32(r.y.floor());
                    let w = f32_to_i32(r.right().ceil()) - x;
                    let h = f32_to_i32(r.bottom().ceil()) - y;
                    if w > 0 && h > 0 {
                        region.add(x, y, w, h);
                    }
                }
                self.layer
                    .wl_surface()
                    .set_input_region(Some(region.wl_region()));
            }
            Err(e) => log::warn!("tidings-ui: cannot create input region: {e}"),
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        if let Some(f) = self.fractional.take() {
            f.destroy();
        }
        if let Some(v) = self.viewport.take() {
            v.destroy();
        }
    }
}

fn int(v: u32) -> Result<i32, UiError> {
    i32::try_from(v).map_err(|_| UiError::Buffer(format!("dimension {v} too large")))
}
