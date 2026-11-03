//! A minimal fake compositor for testing the UI's use of protocols that the
//! headless sway in the end-to-end tests lacks (blur on request).
//!
//! It offers just what the UI needs to start and draw (`wl_compositor`,
//! `wl_shm`, `zwlr_layer_shell_v1`) plus `ext_background_effect_manager_v1`,
//! configures every layer surface as soon as it is committed, answers frame
//! callbacks, and records what the UI asks for: the layer namespaces and
//! every blur region, as applied on commit. Nothing is drawn.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
};
use wayland_protocols_wlr::layer_shell::v1::server::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};
use wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_callback::WlCallback,
    wl_compositor::{self, WlCompositor},
    wl_region::{self, WlRegion},
    wl_shm::{self, WlShm},
    wl_shm_pool::{self, WlShmPool},
    wl_surface::{self, WlSurface},
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, ListeningSocket, New,
    Resource,
};

/// The size a layer surface gets when it leaves a dimension to the
/// compositor (anchored to both edges).
pub const OUTPUT: (u32, u32) = (1280, 800);

/// A rect in surface-local logical px: `(x, y, w, h)`.
pub type Rect4 = (i32, i32, i32, i32);

/// What the compositor says about blurring on request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blur {
    /// The protocol is there and can blur.
    Offered,
    /// The protocol is there, but says it cannot blur (now).
    Refused,
}

/// A blur region the UI set, applied by a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlurSet {
    /// The layer namespace of the surface.
    pub namespace: String,
    /// The region, or `None` for "no blur".
    pub rects: Option<Vec<Rect4>>,
}

/// What the UI asked of the compositor so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    /// Layer namespaces, in the order the surfaces were made.
    pub namespaces: Vec<String>,
    pub effects_created: usize,
    pub effects_destroyed: usize,
    pub blur: Vec<BlurSet>,
}

impl Record {
    /// The last blur region applied on the surface with `namespace`.
    #[must_use]
    pub fn last_blur(&self, namespace: &str) -> Option<&Vec<Rect4>> {
        self.blur
            .iter()
            .rev()
            .find(|b| b.namespace == namespace)
            .and_then(|b| b.rects.as_ref())
    }
}

type Shared = Arc<Mutex<Record>>;

/// The compositor running on its own thread, stopped on drop.
pub struct FakeCompositor {
    socket: PathBuf,
    record: Shared,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

impl FakeCompositor {
    pub fn start(blur: Blur) -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let socket = dir.path().join("wayland-fake");
        let listener =
            ListeningSocket::bind_absolute(socket.clone()).unwrap_or_else(|e| panic!("{e}"));
        let record = Shared::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (record, stop) = (record.clone(), stop.clone());
            std::thread::spawn(move || serve(&listener, blur, record, &stop))
        };
        Self {
            socket,
            record,
            stop,
            thread: Some(thread),
            _dir: dir,
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Waits until the record satisfies `done`, then returns it.
    #[track_caller]
    pub fn wait_for(&self, done: impl Fn(&Record) -> bool) -> Record {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let now = self.record();
            if done(&now) {
                return now;
            }
            assert!(Instant::now() < deadline, "never happened; got {now:#?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn record(&self) -> Record {
        self.record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for FakeCompositor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(listener: &ListeningSocket, blur: Blur, record: Shared, stop: &AtomicBool) {
    let mut display: Display<Server> = Display::new().unwrap_or_else(|e| panic!("{e}"));
    let dh = display.handle();
    dh.create_global::<Server, WlCompositor, ()>(6, ());
    dh.create_global::<Server, WlShm, ()>(1, ());
    dh.create_global::<Server, ZwlrLayerShellV1, ()>(4, ());
    dh.create_global::<Server, ExtBackgroundEffectManagerV1, Blur>(1, blur);
    let mut state = Server { record, serial: 0 };
    while !stop.load(Ordering::Relaxed) {
        if let Ok(Some(stream)) = listener.accept() {
            let _ = display.handle().insert_client(stream, Arc::new(Peer));
        }
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Per-client data: nothing to keep.
struct Peer;

impl ClientData for Peer {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

struct Server {
    record: Shared,
    serial: u32,
}

impl Server {
    fn record(&self) -> std::sync::MutexGuard<'_, Record> {
        self.record.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A surface's state, as far as the tests care.
#[derive(Default)]
struct SurfaceState {
    layer: Option<ZwlrLayerSurfaceV1>,
    namespace: String,
    size: (u32, u32),
    /// A configure is owed on the next commit (new surface, or new size).
    needs_configure: bool,
    frames: Vec<WlCallback>,
    buffer: Option<WlBuffer>,
    /// A blur region set since the last commit: `Some(None)` is "no blur".
    #[allow(
        clippy::option_option,
        reason = "unset vs set to nothing, as in the protocol"
    )]
    pending_blur: Option<Option<Vec<Rect4>>>,
}

type SurfaceData = Mutex<SurfaceState>;

fn surface_state(surface: &WlSurface) -> std::sync::MutexGuard<'_, SurfaceState> {
    surface
        .data::<SurfaceData>()
        .unwrap_or_else(|| panic!("surface without state"))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

impl GlobalDispatch<WlCompositor, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WlCompositor>,
        (): &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<WlCompositor, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlCompositor,
        request: wl_compositor::Request,
        (): &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                init.init(id, SurfaceData::default());
            }
            wl_compositor::Request::CreateRegion { id } => {
                init.init(id, Mutex::new(Vec::<Rect4>::new()));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegion, Mutex<Vec<Rect4>>> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlRegion,
        request: wl_region::Request,
        rects: &Mutex<Vec<Rect4>>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_region::Request::Add {
            x,
            y,
            width,
            height,
        } = request
        {
            rects
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((x, y, width, height));
        }
    }
}

impl Dispatch<WlSurface, SurfaceData> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        surface: &WlSurface,
        request: wl_surface::Request,
        _: &SurfaceData,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Frame { callback } => {
                let cb = init.init(callback, ());
                surface_state(surface).frames.push(cb);
            }
            wl_surface::Request::Attach { buffer, .. } => {
                surface_state(surface).buffer = buffer;
            }
            wl_surface::Request::Commit => {
                let mut s = surface_state(surface);
                if s.needs_configure
                    && let Some(layer) = s.layer.clone()
                {
                    s.needs_configure = false;
                    let w = if s.size.0 == 0 { OUTPUT.0 } else { s.size.0 };
                    let h = if s.size.1 == 0 { OUTPUT.1 } else { s.size.1 };
                    state.serial += 1;
                    layer.configure(state.serial, w, h);
                }
                for cb in s.frames.drain(..) {
                    cb.done(0);
                }
                if let Some(buffer) = s.buffer.take() {
                    buffer.release();
                }
                if let Some(rects) = s.pending_blur.take() {
                    let namespace = s.namespace.clone();
                    drop(s);
                    state.record().blur.push(BlurSet { namespace, rects });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlCallback,
        _: <WlCallback as Resource>::Request,
        (): &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<WlShm, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WlShm>,
        (): &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let shm = init.init(resource, ());
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
        shm.format(wl_shm::Format::Abgr8888);
    }
}

impl Dispatch<WlShm, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlShm,
        request: wl_shm::Request,
        (): &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, .. } = request {
            init.init(id, ());
        }
    }
}

impl Dispatch<WlShmPool, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlShmPool,
        request: wl_shm_pool::Request,
        (): &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer { id, .. } = request {
            init.init(id, ());
        }
    }
}

impl Dispatch<WlBuffer, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlBuffer,
        _: wl_buffer::Request,
        (): &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<ZwlrLayerShellV1, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwlrLayerShellV1>,
        (): &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwlrLayerShellV1,
        request: zwlr_layer_shell_v1::Request,
        (): &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_layer_shell_v1::Request::GetLayerSurface {
            id,
            surface,
            namespace,
            ..
        } = request
        {
            let layer = init.init(id, surface.clone());
            let mut s = surface_state(&surface);
            s.layer = Some(layer);
            s.namespace.clone_from(&namespace);
            s.needs_configure = true;
            drop(s);
            state.record().namespaces.push(namespace);
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, WlSurface> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrLayerSurfaceV1,
        request: zwlr_layer_surface_v1::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_layer_surface_v1::Request::SetSize { width, height } = request {
            let mut s = surface_state(surface);
            if s.size != (width, height) {
                s.size = (width, height);
                s.needs_configure = true;
            }
        }
    }
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, Blur> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        blur: &Blur,
        init: &mut DataInit<'_, Self>,
    ) {
        let manager = init.init(resource, ());
        manager.capabilities(match blur {
            Blur::Offered => Capability::Blur,
            Blur::Refused => Capability::empty(),
        });
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtBackgroundEffectManagerV1,
        request: ext_background_effect_manager_v1::Request,
        (): &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_background_effect_manager_v1::Request::GetBackgroundEffect { id, surface } =
            request
        {
            init.init(id, surface);
            state.record().effects_created += 1;
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, WlSurface> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtBackgroundEffectSurfaceV1,
        request: ext_background_effect_surface_v1::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_background_effect_surface_v1::Request::SetBlurRegion { region } => {
                let rects = region.map(|r| {
                    r.data::<Mutex<Vec<Rect4>>>()
                        .map(|rects| rects.lock().unwrap_or_else(PoisonError::into_inner).clone())
                        .unwrap_or_default()
                });
                surface_state(surface).pending_blur = Some(rects);
            }
            ext_background_effect_surface_v1::Request::Destroy => {
                state.record().effects_destroyed += 1;
            }
            _ => {}
        }
    }
}
