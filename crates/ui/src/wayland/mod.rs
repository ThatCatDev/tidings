//! Wayland plumbing: the UI thread's event loop, surfaces and input routing.
//!
//! [`App`] owns the protocol objects and the three surfaces (panel, catcher,
//! toasts). Drawing is paced by frame callbacks: a change marks a surface
//! dirty and draws immediately if no frame is outstanding, otherwise the
//! next frame callback picks it up.

mod canvas;
mod panel;
mod toasts;

use std::sync::mpsc::Sender;
use std::time::Instant;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::{GlobalList, registry_queue_init};
use smithay_client_toolkit::reexports::client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, registry_handlers,
};
use tidings_core::model::NotificationId;
use tidings_core::ui::{PanelCommand, UiEvent, UiUpdate};
use tidings_core::view::ViewModel;

use crate::UiError;
use crate::anim::Timing;
use crate::geom::{Point, f32_to_u32, u32_to_f32};
use crate::render::Painter;
use crate::scene::HitTarget;
use crate::style::{Palette, Style, metrics as m};
use canvas::{Canvas, SurfaceSpec};
use panel::PanelView;
use toasts::ToastStack;

/// `BTN_LEFT` from linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;

/// Which of our surfaces a protocol object belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceId {
    Panel,
    Catcher,
    Toasts,
}

/// Layer namespaces. Hyprland layer rules match these to blur behind our
/// surfaces; a theme that turns blur off uses the `-noblur` names, which
/// the rules do not match.
fn namespace(blur: bool, blurred: &'static str, plain: &'static str) -> &'static str {
    if blur { blurred } else { plain }
}

fn panel_spec(blur: bool) -> SurfaceSpec {
    SurfaceSpec {
        id: SurfaceId::Panel,
        namespace: namespace(blur, "tidings-panel", "tidings-panel-noblur"),
        layer: Layer::Top,
        anchor: Anchor::TOP.union(Anchor::BOTTOM).union(Anchor::RIGHT),
        size: (f32_to_u32_const(panel::SURFACE_WIDTH), 0),
        margin: (m::PANEL_MARGIN_TOP, 0, m::PANEL_MARGIN_BOTTOM, 0),
        keyboard: KeyboardInteractivity::OnDemand,
    }
}

const CATCHER_SPEC: SurfaceSpec = SurfaceSpec {
    id: SurfaceId::Catcher,
    namespace: "tidings-catcher",
    layer: Layer::Top,
    anchor: Anchor::all(),
    size: (0, 0),
    margin: (0, 0, 0, 0),
    keyboard: KeyboardInteractivity::None,
};

fn toast_spec(height: u32, blur: bool) -> SurfaceSpec {
    SurfaceSpec {
        id: SurfaceId::Toasts,
        namespace: namespace(blur, "tidings-toast", "tidings-toast-noblur"),
        layer: Layer::Overlay,
        anchor: Anchor::TOP.union(Anchor::RIGHT),
        size: (f32_to_u32_const(toasts::SURFACE_WIDTH), height.max(1)),
        // The stack's transparent inset sits inside the margin.
        margin: (m::TOAST_MARGIN_TOP - inset_i32(), 0, 0, 0),
        keyboard: KeyboardInteractivity::None,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the inset is a small constant"
)]
const fn inset_i32() -> i32 {
    m::TOAST_INSET as i32
}

/// `const` float → u32 for the fixed surface widths (small positive constants).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "small positive constants"
)]
const fn f32_to_u32_const(v: f32) -> u32 {
    v as u32
}

/// UI thread state.
pub struct App {
    conn: Connection,
    qh: QueueHandle<App>,
    registry: RegistryState,
    seat: SeatState,
    output: OutputState,
    compositor: CompositorState,
    shm: Shm,
    shm_format: wl_shm::Format,
    layer_shell: LayerShell,
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    events: Sender<UiEvent>,
    palette: Palette,
    view: ViewModel,
    painter: Painter,
    panel_view: PanelView,
    panel: Option<Canvas>,
    catcher: Option<Canvas>,
    toast_stack: ToastStack,
    toasts: Option<Canvas>,
    pointer: Option<ThemedPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Surface and position under the pointer.
    pointer_at: Option<(SurfaceId, Point)>,
    /// Press target, for click = press + release on the same target.
    pressed: Option<(SurfaceId, Option<HitTarget>)>,
    cursor: Option<CursorIcon>,
    /// Most recent fractional scale seen; used for new surfaces.
    last_scale: f32,
    exit: bool,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("panel_mapped", &self.panel.is_some())
            .field("toasts_mapped", &self.toasts.is_some())
            .field("openness", &self.panel_view.motion.openness())
            .finish_non_exhaustive()
    }
}

/// Connects, binds globals and builds the event loop. Returns the loop, the
/// state and the update channel sender.
pub fn setup(
    events: Sender<UiEvent>,
    timing: Timing,
    style: Style,
) -> Result<(EventLoop<'static, App>, App, channel::Sender<UiUpdate>), UiError> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<App>(&conn)?;
    let qh = queue.handle();
    let mut app = App::new(conn.clone(), &globals, &qh, events, timing, style)?;
    // Second roundtrip: receive the shm format list and seat capabilities.
    queue.roundtrip(&mut app)?;
    if app.shm.formats().contains(&wl_shm::Format::Abgr8888) {
        app.shm_format = wl_shm::Format::Abgr8888;
    }
    let event_loop: EventLoop<App> =
        EventLoop::try_new().map_err(|e| UiError::EventLoop(e.to_string()))?;
    WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .map_err(|e| UiError::EventLoop(e.to_string()))?;
    let (tx, rx) = channel::channel::<UiUpdate>();
    insert_channel(&event_loop.handle(), rx)?;

    Ok((event_loop, app, tx))
}

fn insert_channel(handle: &LoopHandle<'static, App>, rx: Channel<UiUpdate>) -> Result<(), UiError> {
    handle
        .insert_source(rx, |event, (), app| match event {
            channel::Event::Msg(update) => app.handle_update(update),
            // The daemon dropped its handle: nothing more will come.
            channel::Event::Closed => app.exit = true,
        })
        .map(|_| ())
        .map_err(|e| UiError::EventLoop(e.to_string()))
}

/// Runs the loop until `Shutdown` or the channel closes.
pub fn run(mut event_loop: EventLoop<'static, App>, mut app: App) {
    while !app.exit {
        if let Err(e) = event_loop.dispatch(None, &mut app) {
            log::error!("tidings-ui: event loop failed: {e}");
            break;
        }
    }
    // Drop surfaces before the connection goes away, then flush.
    app.panel = None;
    app.catcher = None;
    app.toasts = None;
    if let Err(e) = app.conn.flush() {
        log::debug!("tidings-ui: flush on exit failed: {e}");
    }
}

impl App {
    fn new(
        conn: Connection,
        globals: &GlobalList,
        qh: &QueueHandle<Self>,
        events: Sender<UiEvent>,
        timing: Timing,
        style: Style,
    ) -> Result<Self, UiError> {
        let compositor = CompositorState::bind(globals, qh)
            .map_err(|_| UiError::MissingGlobal("wl_compositor"))?;
        let layer_shell = LayerShell::bind(globals, qh)
            .map_err(|_| UiError::MissingGlobal("zwlr_layer_shell_v1"))?;
        let shm = Shm::bind(globals, qh).map_err(|_| UiError::MissingGlobal("wl_shm"))?;
        let fractional = globals
            .bind::<WpFractionalScaleManagerV1, _, _>(qh, 1..=1, ())
            .ok();
        let viewporter = globals.bind::<WpViewporter, _, _>(qh, 1..=1, ()).ok();
        if fractional.is_none() || viewporter.is_none() {
            log::warn!("tidings-ui: no fractional-scale/viewporter support; rendering at scale 1");
        }
        Ok(Self {
            registry: RegistryState::new(globals),
            seat: SeatState::new(globals, qh),
            output: OutputState::new(globals, qh),
            compositor,
            // Decided once the shm format list has arrived (first roundtrip).
            shm_format: wl_shm::Format::Argb8888,
            shm,
            layer_shell,
            fractional,
            viewporter,
            events,
            palette: Palette::default(),
            view: ViewModel::default(),
            painter: Painter::with_style(style),
            panel_view: PanelView::new(timing),
            panel: None,
            catcher: None,
            toast_stack: ToastStack::new(timing),
            toasts: None,
            pointer: None,
            keyboard: None,
            pointer_at: None,
            pressed: None,
            cursor: None,
            last_scale: 1.0,
            exit: false,
            qh: qh.clone(),
            conn,
        })
    }

    fn emit(&self, event: UiEvent) {
        if self.events.send(event).is_err() {
            log::debug!("tidings-ui: daemon is gone; dropping UI event");
        }
    }

    fn handle_update(&mut self, update: UiUpdate) {
        let now = Instant::now();
        match update {
            UiUpdate::View(view) => {
                self.view = *view;
                self.panel_view.needs_layout = true;
                self.sync_toasts(now);
                self.schedule(SurfaceId::Panel);
            }
            UiUpdate::Panel(PanelCommand::Track(x)) => {
                self.panel_view.motion.track(x, now);
                self.after_motion_command();
            }
            UiUpdate::Panel(PanelCommand::AnimateTo(target)) => {
                if let Some(open) = self.panel_view.motion.animate_to(target, now) {
                    self.panel_settled(open);
                } else {
                    self.after_motion_command();
                }
            }
            UiUpdate::Theme(theme) => {
                self.palette = Palette::new(*theme);
                self.panel_view.invalidate_paint();
                self.toast_stack.invalidate_paint();
                self.sync_toasts(now);
                self.schedule(SurfaceId::Panel);
            }
            UiUpdate::Shutdown => self.exit = true,
        }
    }

    fn after_motion_command(&mut self) {
        if self.panel_view.motion.openness() > 0.0 || self.panel_view.motion.is_animating() {
            self.map_panel();
        }
        self.schedule(SurfaceId::Panel);
    }

    fn panel_settled(&mut self, open: bool) {
        self.emit(UiEvent::PanelSettled { open });
        if !open {
            self.unmap_panel();
        }
    }

    /// Maps the catcher, then the panel (so the panel stacks above it).
    fn map_panel(&mut self) {
        if self.panel.is_some() {
            return;
        }
        let qh = self.queue();
        match Canvas::create(self, &qh, &CATCHER_SPEC) {
            Ok(c) => self.catcher = Some(c),
            Err(e) => log::warn!("tidings-ui: cannot create catcher: {e}"),
        }
        match Canvas::create(self, &qh, &panel_spec(self.painter.style.panel_blur)) {
            Ok(c) => self.panel = Some(c),
            Err(e) => {
                log::error!("tidings-ui: cannot create panel: {e}");
                self.catcher = None;
                return;
            }
        }
        // Toasts hide while the panel is out.
        self.toasts = None;
    }

    fn unmap_panel(&mut self) {
        self.panel = None;
        self.catcher = None;
        self.panel_view.reset_surface_state();
        if matches!(
            self.pointer_at,
            Some((SurfaceId::Panel | SurfaceId::Catcher, _))
        ) {
            self.pointer_at = None;
        }
        self.sync_toasts(Instant::now());
    }

    fn queue(&self) -> QueueHandle<Self> {
        self.qh.clone()
    }

    fn canvas_mut(&mut self, id: SurfaceId) -> Option<&mut Canvas> {
        match id {
            SurfaceId::Panel => self.panel.as_mut(),
            SurfaceId::Catcher => self.catcher.as_mut(),
            SurfaceId::Toasts => self.toasts.as_mut(),
        }
    }

    fn id_of(&self, surface: &wl_surface::WlSurface) -> Option<SurfaceId> {
        [
            (SurfaceId::Panel, self.panel.as_ref()),
            (SurfaceId::Catcher, self.catcher.as_ref()),
            (SurfaceId::Toasts, self.toasts.as_ref()),
        ]
        .into_iter()
        .find(|(_, c)| c.is_some_and(|c| c.layer.wl_surface() == surface))
        .map(|(id, _)| id)
    }

    /// Marks a surface dirty and draws now if it is free to.
    fn schedule(&mut self, id: SurfaceId) {
        let Some(c) = self.canvas_mut(id) else { return };
        c.dirty = true;
        if c.can_draw() {
            self.draw(id);
        }
    }

    fn draw(&mut self, id: SurfaceId) {
        let now = Instant::now();
        let result = match id {
            SurfaceId::Panel => self.draw_panel(now),
            SurfaceId::Catcher => self.draw_catcher(),
            SurfaceId::Toasts => self.draw_toasts(now),
        };
        if let Err(e) = result {
            log::error!("tidings-ui: drawing {id:?} failed: {e}");
        }
    }

    fn draw_catcher(&mut self) -> Result<(), UiError> {
        let qh = self.queue();
        let Some(c) = self.catcher.as_mut() else {
            return Ok(());
        };
        if !c.configured {
            return Ok(());
        }
        c.present_clear(&qh)
    }

    fn draw_panel(&mut self, now: Instant) -> Result<(), UiError> {
        let step = self.panel_view.motion.step(now);
        if let Some(open) = step.settled {
            self.panel_settled(open);
            if !open {
                return Ok(());
            }
        }
        // Wait for the catcher to map first so stacking order is right.
        if self.catcher.as_ref().is_some_and(|c| !c.configured) {
            return Ok(());
        }
        let qh = self.queue();
        let Some(canvas) = self.panel.as_mut() else {
            return Ok(());
        };
        if !canvas.configured {
            return Ok(());
        }
        let (_, h) = canvas.logical();
        let scale = canvas.scale;
        self.panel_view
            .refresh(&self.view, &mut self.painter, &self.palette, h, scale, now);
        let Some((frame, damage)) = self.panel_view.compose(scale, canvas.physical()) else {
            return Ok(());
        };
        canvas.present(&qh, &frame, &damage)?;
        canvas.set_input(&self.compositor, &[self.panel_view.input_rect(h)]);
        if self.panel_view.is_animating(now) {
            canvas.dirty = true;
        }
        Ok(())
    }

    fn toast_scale(&self) -> f32 {
        self.toasts.as_ref().map_or(self.last_scale, |c| c.scale)
    }

    fn sync_toasts(&mut self, now: Instant) {
        let scale = self.toast_scale();
        self.toast_stack.sync(
            &self.view.toasts,
            &mut self.painter,
            &self.palette,
            scale,
            now,
        );
        self.ensure_toast_surface(now);
        self.schedule(SurfaceId::Toasts);
    }

    /// Maps the toast surface when there is something to show (and the
    /// panel is closed); unmaps it when empty.
    fn ensure_toast_surface(&mut self, now: Instant) {
        let visible = !self.toast_stack.is_empty() && self.panel.is_none();
        if !visible {
            self.toasts = None;
            return;
        }
        if self.toasts.is_none() {
            let qh = self.queue();
            let h = f32_to_u32(self.toast_stack.needed_height(now));
            match Canvas::create(self, &qh, &toast_spec(h, self.painter.style.toast_blur)) {
                Ok(c) => self.toasts = Some(c),
                Err(e) => log::error!("tidings-ui: cannot create toast surface: {e}"),
            }
        }
    }

    fn draw_toasts(&mut self, now: Instant) -> Result<(), UiError> {
        self.toast_stack.reap(now);
        self.ensure_toast_surface(now);
        let qh = self.queue();
        let Some(canvas) = self.toasts.as_mut() else {
            return Ok(());
        };
        if !canvas.configured {
            return Ok(());
        }
        let animating = self.toast_stack.is_animating(now);
        let needed = f32_to_u32(self.toast_stack.needed_height(now)).max(1);
        // Grow immediately; shrink only once motion has settled.
        if needed > canvas.size.1 || (!animating && needed < canvas.size.1) {
            canvas
                .layer
                .set_size(f32_to_u32_const(toasts::SURFACE_WIDTH), needed);
            canvas.layer.commit();
            canvas.configured = false;
            canvas.dirty = true;
            return Ok(());
        }
        let scale = canvas.scale;
        let Some(frame) = self.toast_stack.compose(now, scale, canvas.physical()) else {
            return Ok(());
        };
        canvas.present(&qh, &frame, &[])?;
        canvas.set_input(&self.compositor, &self.toast_stack.input_rects());
        if animating {
            canvas.dirty = true;
        }
        Ok(())
    }

    fn set_cursor(&mut self, icon: CursorIcon) {
        if self.cursor == Some(icon) {
            return;
        }
        if let Some(p) = &self.pointer {
            if let Err(e) = p.set_cursor(&self.conn, icon) {
                log::debug!("tidings-ui: set_cursor failed: {e}");
                return;
            }
            self.cursor = Some(icon);
        }
    }

    /// Updates hover state for a pointer at `p` on surface `id`.
    fn hover(&mut self, id: SurfaceId, p: Option<Point>) {
        let now = Instant::now();
        let mut clickable = false;
        match id {
            SurfaceId::Panel => {
                let target = p.and_then(|p| self.panel_view.hit(p));
                clickable = target.as_ref().is_some_and(HitTarget::is_clickable);
                if self.panel_view.ui.hovered != target {
                    self.panel_view.ui.hovered = target;
                    self.panel_view.needs_layout = true;
                    self.schedule(SurfaceId::Panel);
                }
            }
            SurfaceId::Toasts => {
                let hit = p.and_then(|p| self.toast_stack.hit(p));
                clickable = hit.as_ref().is_some_and(|(_, t)| t.is_clickable());
                let before = self.toast_stack.hovered.as_ref().map(|(id, _)| *id);
                let after = hit.as_ref().map(|(id, _)| *id);
                let scale = self.toast_scale();
                if self
                    .toast_stack
                    .set_hover(hit, &mut self.painter, &self.palette, scale, now)
                {
                    self.schedule(SurfaceId::Toasts);
                }
                if before != after {
                    self.toast_hover_changed(before, after);
                }
            }
            SurfaceId::Catcher => {}
        }
        if p.is_some() {
            self.set_cursor(if clickable {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
        }
    }

    fn toast_hover_changed(&self, before: Option<NotificationId>, after: Option<NotificationId>) {
        if let Some(id) = before {
            self.emit(UiEvent::ToastHover { id, hovered: false });
        }
        if let Some(id) = after {
            self.emit(UiEvent::ToastHover { id, hovered: true });
        }
    }

    fn target_at(&self, id: SurfaceId, p: Point) -> Option<HitTarget> {
        match id {
            SurfaceId::Panel => self.panel_view.hit(p),
            SurfaceId::Toasts => self.toast_stack.hit(p).map(|(_, t)| t),
            SurfaceId::Catcher => None,
        }
    }

    fn pointer_event(&mut self, id: SurfaceId, event: &PointerEvent) {
        log::trace!(
            "tidings-ui: pointer {:?} on {id:?} at {:?}",
            event.kind,
            event.position
        );
        #[allow(
            clippy::cast_possible_truncation,
            reason = "surface coordinates fit in f32"
        )]
        let p = Point::new(event.position.0 as f32, event.position.1 as f32);
        match event.kind {
            PointerEventKind::Enter { .. } => {
                self.cursor = None;
                self.pointer_at = Some((id, p));
                self.hover(id, Some(p));
            }
            PointerEventKind::Motion { .. } => {
                self.pointer_at = Some((id, p));
                self.hover(id, Some(p));
            }
            PointerEventKind::Leave { .. } => {
                if self.pointer_at.is_some_and(|(s, _)| s == id) {
                    self.pointer_at = None;
                }
                self.pressed = None;
                self.hover(id, None);
            }
            PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                if id == SurfaceId::Catcher {
                    self.emit(UiEvent::RequestClose);
                    return;
                }
                self.pressed = Some((id, self.target_at(id, p)));
            }
            PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                let Some((pid, target)) = self.pressed.take() else {
                    return;
                };
                if pid != id {
                    return;
                }
                let now_target = self.target_at(id, p);
                if let Some(t) = target.filter(|t| Some(t) == now_target.as_ref())
                    && let Some(ev) = t.click_event()
                {
                    self.emit(ev);
                }
            }
            PointerEventKind::Axis { vertical, .. } if id == SurfaceId::Panel => {
                #[allow(clippy::cast_possible_truncation, reason = "scroll deltas are small")]
                let dy = vertical.absolute as f32;
                if dy != 0.0 && self.panel_view.scroll_by(dy) {
                    self.schedule(SurfaceId::Panel);
                    // Content moved under the pointer: refresh hover.
                    let at = self.pointer_at;
                    if let Some((SurfaceId::Panel, p)) = at {
                        self.hover(SurfaceId::Panel, Some(p));
                    }
                }
            }
            _ => {}
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        // Only used when fractional scaling is unavailable.
        if self.fractional.is_some() {
            return;
        }
        if let Some(id) = self.id_of(surface) {
            #[allow(clippy::cast_precision_loss, reason = "integer scale factors are tiny")]
            self.scale_changed(id, factor as f32);
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let Some(id) = self.id_of(surface) else {
            return;
        };
        let Some(c) = self.canvas_mut(id) else { return };
        c.frame_pending = false;
        if c.dirty {
            self.draw(id);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl App {
    fn scale_changed(&mut self, id: SurfaceId, scale: f32) {
        self.last_scale = scale;
        let changed = self.canvas_mut(id).is_some_and(|c| c.set_scale(scale));
        if !changed {
            return;
        }
        if id == SurfaceId::Toasts {
            self.sync_toasts(Instant::now());
        }
        self.schedule(id);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        let Some(id) = self.id_of(layer.wl_surface()) else {
            return;
        };
        log::debug!("tidings-ui: compositor closed {id:?}");
        match id {
            SurfaceId::Panel | SurfaceId::Catcher => {
                self.unmap_panel();
                self.emit(UiEvent::RequestClose);
            }
            SurfaceId::Toasts => self.toasts = None,
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        cfg: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.id_of(layer.wl_surface()) else {
            return;
        };
        let Some(c) = self.canvas_mut(id) else { return };
        let (w, h) = cfg.new_size;
        if w > 0 && h > 0 {
            c.size = (w, h);
        }
        c.configured = true;
        // A configure is not a frame: draw now even if a callback is pending
        // so the new size takes effect immediately.
        c.frame_pending = false;
        self.draw(id);
        if id == SurfaceId::Catcher {
            // The panel waits for the catcher; kick it now.
            self.schedule(SurfaceId::Panel);
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            let surface = self.compositor.create_surface(qh);
            match self.seat.get_pointer_with_theme(
                qh,
                &seat,
                self.shm.wl_shm(),
                surface,
                ThemeSpec::default(),
            ) {
                Ok(p) => self.pointer = Some(p),
                Err(e) => log::warn!("tidings-ui: no pointer: {e}"),
            }
        }
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat.get_keyboard(qh, &seat, None) {
                Ok(k) => self.keyboard = Some(k),
                Err(e) => log::warn!("tidings-ui: no keyboard: {e}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.pointer().release();
        }
        if cap == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if let Some(id) = self.id_of(&event.surface) {
                self.pointer_event(id, event);
            }
        }
    }
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if event.keysym == Keysym::Escape && self.panel.is_some() {
            self.emit(UiEvent::RequestClose);
        }
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

impl Dispatch<WpFractionalScaleV1, SurfaceId> for App {
    fn event(
        app: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        id: &SurfaceId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            app.scale_changed(*id, u32_to_f32(scale) / 120.0);
        }
    }
}

impl Dispatch<WpFractionalScaleManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpViewporter, ()> for App {
    fn event(
        _: &mut Self,
        _: &WpViewporter,
        _: <WpViewporter as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpViewport, ()> for App {
    fn event(
        _: &mut Self,
        _: &WpViewport,
        _: <WpViewport as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_layer!(App);
delegate_registry!(App);
