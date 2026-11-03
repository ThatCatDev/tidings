//! Asking the compositor to blur what is behind the glass, through
//! `ext-background-effect-v1`.
//!
//! Compositors that support it (Hyprland, niri 26.04 and newer, others as
//! they adopt it) then blur behind tidings with no configuration. The blur
//! region follows each glass shape, rounded corners included, so neither the
//! transparent margin nor the shadow is blurred. Compositors without the
//! protocol fall back on their own rules (see the layer namespaces).

use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
};
use smithay_client_toolkit::reexports::protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};

use super::App;
use crate::geom::{Rect, f32_to_i32};

/// A rounded rectangle of glass, in surface-local logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glass {
    pub rect: Rect,
    pub radius: f32,
}

/// Whole-pixel rectangles `(x, y, w, h)` covering `glass`: a band through
/// the middle and, near the top and bottom, one row per pixel inset to follow
/// the corner's curve (rows with the same inset merged). A Wayland region is
/// made of rectangles, so this is how it gets round corners.
#[must_use]
#[allow(
    clippy::many_single_char_names,
    reason = "x, y, w, h and r are the usual names for a rect and its radius"
)]
pub fn region_rects(glass: &[Glass]) -> Vec<(i32, i32, i32, i32)> {
    let mut out = Vec::new();
    for g in glass {
        let x = f32_to_i32(g.rect.x.round());
        let y = f32_to_i32(g.rect.y.round());
        let w = f32_to_i32(g.rect.w.round());
        let h = f32_to_i32(g.rect.h.round());
        if w <= 0 || h <= 0 {
            continue;
        }
        let r = f32_to_i32(g.radius.round()).clamp(0, w.min(h) / 2);
        // One inset per row of the corner, from the outermost row in.
        let insets: Vec<i32> = (0..r)
            .map(|row| {
                let rf = crate::geom::i32_to_f32(r);
                let dy = rf - crate::geom::i32_to_f32(row) - 0.5;
                f32_to_i32((rf - (rf * rf - dy * dy).max(0.0).sqrt()).ceil())
            })
            .collect();
        let mut row = 0;
        while row < r {
            let inset = insets[usize::try_from(row).unwrap_or(0)];
            let start = row;
            while row < r && insets[usize::try_from(row).unwrap_or(0)] == inset {
                row += 1;
            }
            let rows = row - start;
            let width = w - 2 * inset;
            if width > 0 {
                out.push((x + inset, y + start, width, rows));
                out.push((x + inset, y + h - start - rows, width, rows));
            }
        }
        if h > 2 * r {
            out.push((x, y + r, w, h - 2 * r));
        }
    }
    out
}

/// The blur manager and whether the compositor says it can blur now.
#[derive(Debug)]
pub struct BlurManager {
    pub manager: ExtBackgroundEffectManagerV1,
    pub supported: bool,
}

impl BlurManager {
    /// Starts blurring behind `surface`.
    pub fn effect_for(
        &self,
        surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        qh: &QueueHandle<App>,
    ) -> ExtBackgroundEffectSurfaceV1 {
        self.manager.get_background_effect(surface, qh, ())
    }
}

/// Sets `effect`'s blur region to `glass`. Like the input region it applies
/// with the surface's next commit.
pub fn set_region(
    effect: &ExtBackgroundEffectSurfaceV1,
    compositor: &CompositorState,
    glass: &[Glass],
) {
    match Region::new(compositor) {
        Ok(region) => {
            for (x, y, w, h) in region_rects(glass) {
                region.add(x, y, w, h);
            }
            effect.set_blur_region(Some(region.wl_region()));
        }
        Err(e) => log::warn!("tidings-ui: cannot create blur region: {e}"),
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for App {
    fn event(
        app: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event
            && let Some(blur) = app.blur.as_mut()
        {
            blur.supported = matches!(flags, WEnum::Value(f) if f.contains(Capability::Blur));
            log::debug!("tidings-ui: compositor blur on request: {}", blur.supported);
        }
    }
}

delegate_noop!(App: ExtBackgroundEffectSurfaceV1);

#[cfg(test)]
mod tests {
    use super::*;

    fn area(rects: &[(i32, i32, i32, i32)]) -> i32 {
        rects.iter().map(|&(_, _, w, h)| w * h).sum()
    }

    fn covers(rects: &[(i32, i32, i32, i32)], px: i32, py: i32) -> bool {
        rects
            .iter()
            .any(|&(x, y, w, h)| px >= x && px < x + w && py >= y && py < y + h)
    }

    #[test]
    fn square_glass_is_one_rect() {
        let r = region_rects(&[Glass {
            rect: Rect::new(4.0, 6.0, 100.0, 40.0),
            radius: 0.0,
        }]);
        assert_eq!(r, [(4, 6, 100, 40)]);
    }

    #[test]
    fn rounded_corners_are_cut_away() {
        let rect = Rect::new(0.0, 0.0, 200.0, 80.0);
        let r = region_rects(&[Glass { rect, radius: 20.0 }]);
        // The centre and the middle of each edge are covered...
        for (x, y) in [(100, 40), (100, 0), (100, 79), (0, 40), (199, 40)] {
            assert!(covers(&r, x, y), "({x}, {y}) should be blurred");
        }
        // ...the very corners are not, and nothing spills outside.
        for (x, y) in [(0, 0), (199, 0), (0, 79), (199, 79), (2, 2)] {
            assert!(!covers(&r, x, y), "({x}, {y}) is outside the curve");
        }
        assert!(
            r.iter()
                .all(|&(x, y, w, h)| x >= 0 && y >= 0 && x + w <= 200 && y + h <= 80)
        );
        // About the area of a rounded rect: w*h - (4 - pi) r^2.
        let ideal = 200.0 * 80.0 - (4.0 - std::f32::consts::PI) * 400.0;
        let got = crate::geom::i32_to_f32(area(&r));
        assert!((got - ideal).abs() < 0.03 * ideal, "{got} vs {ideal}");
        // No pixel is covered twice.
        let mut seen = std::collections::HashSet::new();
        for &(x, y, w, h) in &r {
            for py in y..y + h {
                for px in x..x + w {
                    assert!(seen.insert((px, py)), "({px}, {py}) twice");
                }
            }
        }
    }

    #[test]
    fn pills_and_empty_glass() {
        // A radius over half the height is a pill, not an error.
        let r = region_rects(&[Glass {
            rect: Rect::new(0.0, 0.0, 300.0, 58.0),
            radius: 99.0,
        }]);
        assert!(covers(&r, 150, 29) && !covers(&r, 0, 0));
        let empty = Glass {
            rect: Rect::new(0.0, 0.0, 0.0, 10.0),
            radius: 4.0,
        };
        assert_eq!(region_rects(&[empty]), []);
        // Several pieces of glass make one region.
        let two = [
            Glass {
                rect: Rect::new(0.0, 0.0, 50.0, 50.0),
                radius: 0.0,
            },
            Glass {
                rect: Rect::new(0.0, 60.0, 50.0, 50.0),
                radius: 0.0,
            },
        ];
        assert_eq!(region_rects(&two).len(), 2);
    }
}
