//! Wiring: connects the D-Bus server, IPC socket, theme watcher, gesture
//! reader and UI to one App actor.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use tidings_core::clock::Clock;
use tidings_core::config::Config;
use tidings_core::ui::{UiEvent, UiHandle};
use tidings_gesture::{GestureReader, ReaderConfig, RecognizerConfig, TrackpadOpener};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::{Connection, connection};

use crate::app::{App, AppDeps, AppEvent, AppSender, PanelStateSink};
use crate::dbus::server::{BUS_NAME, DbusSignals, NotificationServer, OBJECT_PATH};
use crate::ipc::server::{BindError, IpcServer};
use crate::theme_watch::{POLL_INTERVAL, ThemeWatcher};

/// Which bus to own the notifications name on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bus {
    /// `$DBUS_SESSION_BUS_ADDRESS`.
    Session,
    /// An explicit address (tests use a private bus).
    Address(String),
}

/// Daemon settings.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Bus to serve on.
    pub bus: Bus,
    /// IPC socket location.
    pub socket_path: PathBuf,
    /// `None` uses the built-in colours and watches nothing.
    pub colors_path: Option<PathBuf>,
    /// How often the colours file is polled.
    pub theme_poll: Duration,
    /// The user's settings (`config.toml`).
    pub settings: Config,
}

impl DaemonConfig {
    /// Defaults for `socket_path`, everything else standard.
    #[must_use]
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            bus: Bus::Session,
            socket_path,
            colors_path: None,
            theme_poll: POLL_INTERVAL,
            settings: Config::default(),
        }
    }
}

/// Startup failures.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// Connecting or serving failed.
    #[error("D-Bus: {0}")]
    Bus(#[from] zbus::Error),
    #[error(
        "{BUS_NAME} is already owned by another notification daemon; stop it first \
         (tidings never replaces a running daemon)"
    )]
    /// Another daemon owns the notifications name.
    NameTaken,
    /// The IPC socket could not be bound.
    #[error(transparent)]
    Ipc(#[from] BindError),
    /// The gesture thread could not be spawned.
    #[error("cannot start the gesture reader: {0}")]
    Gesture(std::io::Error),
    /// The UI failed to start.
    #[error("cannot start the UI: {0}")]
    Ui(Box<dyn std::error::Error + Send + Sync>),
}

/// Creates the UI, given the channel it reports interactions on.
pub type UiFactory = Box<
    dyn FnOnce(
        std_mpsc::Sender<UiEvent>,
    ) -> Result<Box<dyn UiHandle>, Box<dyn std::error::Error + Send + Sync>>,
>;

/// A running daemon.
#[derive(Debug)]
pub struct Daemon {
    events: AppSender,
    actor: JoinHandle<()>,
    background: Vec<JoinHandle<()>>,
    connection: Connection,
}

/// Panel state goes nowhere when gestures are off.
struct NoGestures;

impl PanelStateSink for NoGestures {
    fn set_panel_open(&self, _open: bool) {}
}

impl PanelStateSink for GestureReader {
    fn set_panel_open(&self, open: bool) {
        GestureReader::set_panel_open(self, open);
    }
}

impl Daemon {
    /// Owns the bus name (refusing if taken), binds the IPC socket, then
    /// starts the UI, gesture reader, theme watcher and actor.
    pub async fn start(
        config: DaemonConfig,
        clock: Arc<dyn Clock>,
        ui_factory: UiFactory,
    ) -> Result<Self, DaemonError> {
        let (events, rx) = mpsc::unbounded_channel();

        let builder = match &config.bus {
            Bus::Session => connection::Builder::session(),
            Bus::Address(address) => connection::Builder::address(address.as_str()),
        };
        let connection = builder
            .serve_at(OBJECT_PATH, NotificationServer::new(events.clone()))
            .build()
            .await?;
        // DO_NOT_QUEUE without REPLACE_EXISTING: fail rather than queue or take over.
        match connection
            .request_name_with_flags(BUS_NAME, RequestNameFlags::DO_NOT_QUEUE)
            .await
        {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
            Ok(RequestNameReply::Exists | RequestNameReply::InQueue)
            | Err(zbus::Error::NameTaken) => {
                return Err(DaemonError::NameTaken);
            }
            Err(e) => return Err(e.into()),
        }
        tracing::info!("owning {BUS_NAME}");

        let ipc = IpcServer::bind(&config.socket_path).await?;
        tracing::info!("ipc socket: {}", config.socket_path.display());

        let (ui_tx, ui_rx) = std_mpsc::channel();
        let ui = ui_factory(ui_tx).map_err(DaemonError::Ui)?;
        bridge_ui_events(ui_rx, events.clone()).map_err(|e| DaemonError::Ui(Box::new(e)))?;

        let gestures = config.settings.gestures;
        let panel_state: Box<dyn PanelStateSink> = if gestures.enabled {
            let tx = events.clone();
            let reader = GestureReader::spawn(
                TrackpadOpener::default(),
                ReaderConfig {
                    recognizer: RecognizerConfig {
                        edge: gestures.edge_zone,
                        ..RecognizerConfig::default()
                    },
                    ..ReaderConfig::default()
                },
                move |g| {
                    if tx.send(AppEvent::Gesture(g)).is_ok() {
                        std::ops::ControlFlow::Continue(())
                    } else {
                        std::ops::ControlFlow::Break(())
                    }
                },
            )
            .map_err(DaemonError::Gesture)?;
            Box::new(reader)
        } else {
            Box::new(NoGestures)
        };

        let mut background = Vec::new();
        let theme = match config.colors_path {
            Some(path) => {
                let (theme, watcher) = ThemeWatcher::load(path);
                background.push(tokio::spawn(watcher.run(config.theme_poll, events.clone())));
                theme
            }
            None => tidings_core::theme::Theme::default(),
        };
        background.push(tokio::spawn(ipc.serve(events.clone())));

        let signals = DbusSignals::spawn(&connection)?;
        let app = App::new(
            AppDeps {
                clock,
                ui,
                signals: Box::new(signals),
                panel_state,
            },
            config.settings.toasts,
            gestures.swipe,
            theme,
        );
        let actor = tokio::spawn(app.run(rx));
        Ok(Self {
            events,
            actor,
            background,
            connection,
        })
    }

    /// A sender into the actor (tests use it to inject ticks and UI events).
    #[must_use]
    pub fn events(&self) -> AppSender {
        self.events.clone()
    }

    /// The daemon's bus connection.
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Whether the actor has stopped (e.g. after `Shutdown`).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.actor.is_finished()
    }

    /// Stops the actor, then the background tasks; releases the bus name and
    /// removes the socket.
    pub async fn shutdown(self) {
        let _ = self.events.send(AppEvent::Shutdown);
        if let Err(e) = self.actor.await {
            tracing::error!("app actor failed: {e}");
        }
        for task in self.background {
            task.abort();
            // Aborted tasks report cancellation; the IPC server's drop removes the socket.
            let _ = task.await;
        }
        if let Err(e) = self.connection.release_name(BUS_NAME).await {
            tracing::debug!("releasing {BUS_NAME}: {e}");
        }
    }
}

/// Forwards UI interactions (reported on a plain channel from the UI
/// thread) into the actor. The thread ends when the UI drops its sender or
/// the actor is gone.
fn bridge_ui_events(rx: std_mpsc::Receiver<UiEvent>, events: AppSender) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("tidings-ui-events".into())
        .spawn(move || {
            while let Ok(ev) = rx.recv() {
                if events.send(AppEvent::Ui(ev)).is_err() {
                    break;
                }
            }
        })?;
    Ok(())
}
