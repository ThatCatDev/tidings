//! Test fixtures for the end-to-end UI tests: a private headless sway, a
//! virtual pointer on it, and a [`Session`] that runs the UI there and
//! drives it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::PoisonError;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::client::globals::{GlobalListContents, registry_queue_init};
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::{Axis, ButtonState};
use smithay_client_toolkit::reexports::client::protocol::{wl_registry, wl_seat::WlSeat};
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
};
use smithay_client_toolkit::reexports::protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use tidings_core::config::AnimationConfig;
use tidings_core::look::Look;
use tidings_core::ui::{UiEvent, UiHandle, UiUpdate};
use tidings_core::view::{ToastView, ViewModel};
use tidings_ui::compose::ReplyDraft;
use tidings_ui::geom::{Point, Size};
use tidings_ui::layout::{Interaction, layout_panel, layout_toast};
use tidings_ui::render::Painter;
use tidings_ui::scene::{HitTarget, Scene};
use tidings_ui::style::{Style, metrics as m};
use tidings_ui::{Display, spawn_on};

/// The headless output: physical size and scale (fractional, to exercise
/// the viewporter path).
const OUTPUT: (f64, f64) = (1280.0, 800.0);
const SCALE: f64 = 1.5;

/// `BTN_LEFT` from linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;

/// Taken by each [`Compositor`] in turn (see its `_turn`).
static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How long to wait for the UI to answer.
const PATIENCE: Duration = Duration::from_secs(5);

/// The tools the tests need besides sway.
const TOOLS: [&str; 5] = ["sway", "swaymsg", "wtype", "wl-copy", "grim"];

/// The output's size in logical px.
fn logical() -> (f64, f64) {
    (OUTPUT.0 / SCALE, OUTPUT.1 / SCALE)
}

fn installed(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(tool).is_file()))
}

/// A private headless sway, killed on drop.
pub struct Compositor {
    /// Held while the compositor lives: the tests take turns, as several
    /// software-rendering compositors at once starve each other enough
    /// to make timing flaky.
    _turn: std::sync::MutexGuard<'static, ()>,
    child: Child,
    /// A `wtype` sleeping with its virtual keyboard attached, so the seat
    /// always has a keyboard: otherwise each `wtype` would add one, type
    /// before the UI has bound it, and take it away again.
    keyboard: std::sync::Mutex<Option<Child>>,
    dir: tempfile::TempDir,
    socket: PathBuf,
}

impl Compositor {
    /// Starts sway, or returns `None` (skipping the test) when it or the
    /// other tools are missing, unless `TIDINGS_REQUIRE_COMPOSITOR` is set.
    pub fn start() -> Option<Self> {
        let missing: Vec<_> = TOOLS.into_iter().filter(|t| !installed(t)).collect();
        if !missing.is_empty() {
            assert!(
                std::env::var_os("TIDINGS_REQUIRE_COMPOSITOR").is_none(),
                "missing {missing:?}"
            );
            eprintln!("skipping: {missing:?} not installed");
            return None;
        }
        let turn = TURN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // sway's IPC socket path must fit in sun_path, so keep it short.
        let dir = tempfile::Builder::new()
            .prefix("tw")
            .tempdir_in("/tmp")
            .unwrap_or_else(|e| panic!("{e}"));
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|e| panic!("{e}"));
        let config = dir.path().join("config");
        fs::write(
            &config,
            format!(
                "output HEADLESS-1 resolution {}x{} scale {SCALE}\n",
                OUTPUT.0, OUTPUT.1
            ),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let log = fs::File::create(dir.path().join("sway.log")).unwrap_or_else(|e| panic!("{e}"));
        let child = Command::new("sway")
            .arg("-c")
            .arg(&config)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", dir.path())
            .env("XDG_RUNTIME_DIR", dir.path())
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", "pixman")
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap_or_else(|e| panic!("cannot start sway: {e}"));
        let socket = wait_for_socket(dir.path());
        let mut sway = Self {
            _turn: turn,
            child,
            keyboard: std::sync::Mutex::new(None),
            dir,
            socket,
        };
        let holder = sway
            .tool("wtype")
            .args(["-s", "3600000"])
            .spawn()
            .unwrap_or_else(|e| panic!("wtype: {e}"));
        *sway
            .keyboard
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner) = Some(holder);
        Some(sway)
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// A command run against this compositor.
    fn tool(&self, name: &str) -> Command {
        let mut cmd = Command::new(name);
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("XDG_RUNTIME_DIR", self.dir.path())
            .env(
                "WAYLAND_DISPLAY",
                self.socket.file_name().unwrap_or_default(),
            );
        // swaymsg finds sway by its IPC socket, next to the Wayland one.
        if let Some(ipc) = fs::read_dir(self.dir.path())
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("sway-ipc"))
            })
        {
            cmd.env("SWAYSOCK", ipc);
        }
        cmd
    }

    fn run(&self, name: &str, args: &[&str]) -> Vec<u8> {
        let out = self
            .tool(name)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            out.status.success(),
            "{name} {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }

    /// Runs a sway command (`create_output`, `output HEADLESS-1 scale 2`).
    pub fn sway(&self, command: &str) {
        self.run("swaymsg", &[command]);
        std::thread::sleep(Duration::from_millis(300));
    }

    /// Stops the virtual keyboard: the seat loses its keyboard.
    pub fn unplug_keyboard(&self) {
        let holder = self
            .keyboard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut k) = holder {
            let _ = k.kill();
            let _ = k.wait();
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    /// Puts `text` on the clipboard.
    pub fn copy(&self, text: &str) {
        // wl-copy forks to keep serving the clipboard; with output captured
        // the fork would hold the pipe open, so give it none.
        let status = self
            .tool("wl-copy")
            .args(["--", text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap_or_else(|e| panic!("wl-copy: {e}"));
        assert!(status.success(), "wl-copy failed");
    }

    pub fn pixel(&self, at: Point) -> [u8; 3] {
        let geometry = format!("{},{} 1x1", at.x.round(), at.y.round());
        let ppm = self.run("grim", &["-t", "ppm", "-g", &geometry, "-"]);
        let rgb = &ppm[ppm.len() - 3..];
        [rgb[0], rgb[1], rgb[2]]
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        if let Some(k) = self
            .keyboard
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            let _ = k.kill();
            let _ = k.wait();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for_socket(dir: &Path) -> PathBuf {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let found = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("wayland-"))
                    && p.extension().is_none_or(|e| e != "lock")
            });
        if let Some(socket) = found
            && std::os::unix::net::UnixStream::connect(&socket).is_ok()
        {
            return socket;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let log = fs::read_to_string(dir.join("sway.log")).unwrap_or_default();
    panic!("sway did not start:\n{log}");
}

/// Protocol state for the virtual pointer's connection: nothing to track.
struct Quiet;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Quiet {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Quiet: ignore WlSeat);
delegate_noop!(Quiet: ZwlrVirtualPointerManagerV1);
delegate_noop!(Quiet: ignore ZwlrVirtualPointerV1);

/// A pointer moved and clicked from the test, as a mouse would.
pub struct Pointer {
    conn: Connection,
    queue: EventQueue<Quiet>,
    device: ZwlrVirtualPointerV1,
    epoch: Instant,
}

impl Pointer {
    fn connect(socket: &Path) -> Self {
        let stream =
            std::os::unix::net::UnixStream::connect(socket).unwrap_or_else(|e| panic!("{e}"));
        let conn = Connection::from_socket(stream).unwrap_or_else(|e| panic!("{e}"));
        let (globals, mut queue) =
            registry_queue_init::<Quiet>(&conn).unwrap_or_else(|e| panic!("{e}"));
        let qh = queue.handle();
        let seat: WlSeat = globals
            .bind(&qh, 1..=7, ())
            .unwrap_or_else(|e| panic!("no seat: {e}"));
        let manager: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .unwrap_or_else(|e| panic!("no virtual pointer: {e}"));
        let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
        queue
            .roundtrip(&mut Quiet)
            .unwrap_or_else(|e| panic!("{e}"));
        Self {
            conn,
            queue,
            device: pointer,
            epoch: Instant::now(),
        }
    }

    fn time(&self) -> u32 {
        u32::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    fn send(&mut self) {
        self.device.frame();
        self.queue
            .roundtrip(&mut Quiet)
            .unwrap_or_else(|e| panic!("{e}"));
        let _ = self.conn.flush();
        std::thread::sleep(Duration::from_millis(40));
    }

    /// Moves to a logical point on the output.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "points on a 1280x800 output"
    )]
    pub fn move_to(&mut self, x: f64, y: f64) {
        let (w, h) = logical();
        let to = |v: f64, extent: f64, physical: f64| (v / extent * physical) as u32;
        self.device.motion_absolute(
            self.time(),
            to(x, w, OUTPUT.0),
            to(y, h, OUTPUT.1),
            OUTPUT.0 as u32,
            OUTPUT.1 as u32,
        );
        self.send();
    }

    pub fn click(&mut self) {
        self.device
            .button(self.time(), BTN_LEFT, ButtonState::Pressed);
        self.send();
        self.device
            .button(self.time(), BTN_LEFT, ButtonState::Released);
        self.send();
    }

    /// Removes the virtual pointer: the seat loses its pointer.
    pub fn unplug(&mut self) {
        self.device.destroy();
        self.queue
            .roundtrip(&mut Quiet)
            .unwrap_or_else(|e| panic!("{e}"));
        std::thread::sleep(Duration::from_millis(300));
    }

    /// Scrolls by `dy` logical px (positive: down).
    pub fn scroll(&mut self, dy: f64) {
        self.device.axis(self.time(), Axis::VerticalScroll, dy);
        self.send();
    }
}

/// The UI running on a [`Compositor`], with a pointer to drive it.
pub struct Session<'a> {
    sway: &'a Compositor,
    pub ui: Box<dyn UiHandle>,
    events: Receiver<UiEvent>,
    pub pointer: Pointer,
    painter: Painter,
}

impl<'a> Session<'a> {
    /// The UI with animations off, so what the test clicks is where it
    /// will stay.
    pub fn start(sway: &'a Compositor) -> Self {
        Self::with_animation(
            sway,
            AnimationConfig {
                enabled: false,
                ..AnimationConfig::default()
            },
        )
    }

    /// The UI moving as it does for real.
    pub fn animated(sway: &'a Compositor) -> Self {
        Self::with_animation(sway, AnimationConfig::default())
    }

    fn with_animation(sway: &'a Compositor, animation: AnimationConfig) -> Self {
        let _ = env_logger::builder().is_test(true).try_init();
        let (tx, events) = channel();
        let ui = spawn_on(
            &Display::Socket(sway.socket().to_owned()),
            tx,
            animation,
            &Look::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        Self {
            sway,
            ui,
            events,
            pointer: Pointer::connect(sway.socket()),
            painter: Painter::with_style(Style::new(&Look::default())),
        }
    }

    /// Gives the UI time to draw and animations time to finish.
    #[allow(clippy::unused_self, reason = "reads as a step of the session")]
    pub fn settle(&self) {
        std::thread::sleep(Duration::from_millis(450));
    }

    pub fn view(&self, view: ViewModel) {
        self.ui.send(UiUpdate::View(Box::new(view)));
        self.settle();
    }

    /// Waits for an event matching `want`, skipping others.
    #[track_caller]
    pub fn expect(&self, want: impl Fn(&UiEvent) -> bool) -> UiEvent {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(e) if want(&e) => return e,
                Ok(e) => seen.push(e),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    panic!("the expected event never came; got {seen:?}")
                }
            }
        }
    }

    /// Where `target` is on a toast laid out like `t` (the top one), on
    /// screen. `draft` is the reply being typed, if any.
    fn toast_point(
        &mut self,
        t: &ToastView,
        draft: Option<&ReplyDraft>,
        target: &HitTarget,
    ) -> Point {
        #[allow(clippy::cast_possible_truncation, reason = "the scale is 1.5")]
        let scale = SCALE as f32;
        let lay = layout_toast(
            t,
            scale,
            Some(target),
            draft,
            &self.painter.style,
            &mut self.painter.text,
        );
        let surface_w = m::TOAST_INSET + m::TOAST_WIDTH + m::TOAST_MARGIN_RIGHT;
        #[allow(clippy::cast_possible_truncation, reason = "the output is small")]
        let origin = Point::new(
            logical().0 as f32 - surface_w,
            tidings_ui::geom::i32_to_f32(m::TOAST_MARGIN_TOP) - m::TOAST_INSET,
        );
        center_of(&lay.scene, target, origin)
    }

    pub fn hover_toast(&mut self, t: &ToastView, target: &HitTarget) {
        let p = self.toast_point(t, None, target);
        self.pointer.move_to(f64::from(p.x), f64::from(p.y));
    }

    pub fn click_toast(&mut self, t: &ToastView, target: &HitTarget) {
        self.click_toast_with(t, None, target);
    }

    pub fn click_toast_with(
        &mut self,
        t: &ToastView,
        draft: Option<&ReplyDraft>,
        target: &HitTarget,
    ) {
        let p = self.toast_point(t, draft, target);
        self.pointer.move_to(f64::from(p.x), f64::from(p.y));
        self.pointer.click();
    }

    /// Whether the top toast's glass differs from the empty background.
    pub fn toast_is_painted(&self) -> bool {
        #[allow(clippy::cast_possible_truncation, reason = "the output is small")]
        let right = logical().0 as f32 - m::TOAST_MARGIN_RIGHT;
        let inside = Point::new(
            right - 40.0,
            tidings_ui::geom::i32_to_f32(m::TOAST_MARGIN_TOP) + 20.0,
        );
        self.sway.pixel(inside) != self.sway.pixel(Point::new(20.0, 400.0))
    }

    /// Where `target` is in the open panel showing `view`, on screen.
    fn panel_point(
        &mut self,
        view: &ViewModel,
        draft: Option<&ReplyDraft>,
        target: &HitTarget,
    ) -> Point {
        let (w, h) = logical();
        #[allow(clippy::cast_possible_truncation, reason = "the output is small")]
        let (w, h) = (w as f32, h as f32);
        let top = tidings_ui::geom::i32_to_f32(m::PANEL_MARGIN_TOP);
        let bottom = tidings_ui::geom::i32_to_f32(m::PANEL_MARGIN_BOTTOM);
        let ui = Interaction {
            hovered: Some(target.clone()),
            reply: draft.cloned(),
            ..Interaction::default()
        };
        #[allow(clippy::cast_possible_truncation, reason = "the scale is 1.5")]
        let lay = layout_panel(
            view,
            Size::new(m::PANEL_WIDTH, (h - top - bottom).floor()),
            SCALE as f32,
            &ui,
            &self.painter.style,
            &mut self.painter.text,
        );
        let origin = Point::new(w - m::PANEL_WIDTH - m::PANEL_MARGIN_RIGHT, top);
        center_of(&lay.scene, target, origin)
    }

    pub fn hover_panel(&mut self, view: &ViewModel, target: &HitTarget) {
        let p = self.panel_point(view, None, target);
        self.pointer.move_to(f64::from(p.x), f64::from(p.y));
    }

    /// Clicks `target` in the panel while `draft` is being typed.
    pub fn click_panel_with(
        &mut self,
        view: &ViewModel,
        draft: Option<&ReplyDraft>,
        target: &HitTarget,
    ) {
        let p = self.panel_point(view, draft, target);
        self.pointer.move_to(f64::from(p.x), f64::from(p.y));
        self.pointer.click();
    }

    pub fn click_panel(&mut self, view: &ViewModel, target: &HitTarget) {
        self.hover_panel(view, target);
        self.pointer.click();
    }

    fn wtype(&self, args: &[&str]) {
        self.sway.run("wtype", args);
        std::thread::sleep(Duration::from_millis(60));
    }

    /// Types text.
    pub fn keys(&self, texts: &[&str]) {
        for t in texts {
            self.wtype(&["--", t]);
        }
    }

    /// Presses a named key (an xkb keysym name such as `Return`).
    pub fn key(&self, name: &str) {
        self.wtype(&["-k", name]);
    }

    /// Presses `key` with `modifier` (`ctrl`, `shift`, ...) held.
    pub fn chord(&self, modifier: &str, key: &str) {
        self.wtype(&["-M", modifier, key, "-m", modifier]);
    }
}

/// The centre of `target`'s hit region in `scene`, offset by `origin`.
fn center_of(scene: &Scene, target: &HitTarget, origin: Point) -> Point {
    let hit = scene
        .hits
        .iter()
        .rev()
        .find(|h| h.target == *target)
        .unwrap_or_else(|| panic!("no {target:?} in the layout"));
    let c = hit.rect.center();
    Point::new(origin.x + c.x, origin.y + c.y)
}
