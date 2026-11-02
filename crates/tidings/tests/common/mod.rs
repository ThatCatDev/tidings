//! Integration-test harness: a private `dbus-daemon`, the daemon core with a
//! tapped `NullUi` and a fake clock, and a client proxy.
//!
//! Nothing here touches the real session bus: every connection is made to
//! the private bus's explicit address.

#![allow(dead_code)] // each test crate uses a different subset
#![allow(clippy::unwrap_used)] // test harness: failing loudly is the point

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use tempfile::TempDir;
use tidings::daemon::{Bus, Daemon, DaemonConfig, DaemonError};
use tidings::ipc::client;
use tidings::ipc::proto::{Request, Response};
use tidings::null_ui::NullUi;
use tidings_core::clock::FakeClock;
use tidings_core::ui::UiUpdate;
use zbus::{Connection, OwnedVariant, connection};

/// A throwaway `dbus-daemon` with no service activation, killed on drop.
pub struct PrivateBus {
    child: Child,
    pub address: String,
    _dir: TempDir,
}

impl PrivateBus {
    pub fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("bus");
        let config = dir.path().join("bus.conf");
        // No <servicedir>: a call to an unowned name must never auto-start
        // the desktop's real notification daemon on this bus.
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                socket.display()
            ),
        )
        .unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("dbus-daemon is installed");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let address = line.trim().to_owned();
        assert!(
            address.starts_with("unix:path="),
            "unexpected address {address:?}"
        );
        Self {
            child,
            address,
            _dir: dir,
        }
    }

    pub async fn client(&self) -> Connection {
        connection::Builder::address(self.address.as_str())
            .build()
            .await
            .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Client side of `org.freedesktop.Notifications`.
#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait Notifications {
    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, OwnedVariant>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_server_information(&self) -> zbus::Result<(String, String, String, String)>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// The daemon core on a private bus with a tapped `NullUi` and a fake clock.
pub struct TestDaemon {
    pub daemon: Option<Daemon>,
    pub clock: FakeClock,
    pub ui: std_mpsc::Receiver<UiUpdate>,
    pub socket: PathBuf,
    pub colors: PathBuf,
    pub dir: TempDir,
    pub bus: PrivateBus,
}

pub fn config(bus: &PrivateBus, dir: &Path) -> DaemonConfig {
    let mut config = DaemonConfig::new(dir.join("tidings.sock"));
    config.bus = Bus::Address(bus.address.clone());
    config.colors_path = Some(dir.join("colors.json"));
    config.theme_poll = Duration::from_millis(20);
    config.settings.gestures.enabled = false;
    config
}

/// Starts the daemon core with a tapped `NullUi`.
pub async fn start_daemon(
    config: DaemonConfig,
    clock: &FakeClock,
) -> Result<(Daemon, std_mpsc::Receiver<UiUpdate>), DaemonError> {
    let (tap, ui) = std_mpsc::channel();
    let daemon = Daemon::start(
        config,
        Arc::new(clock.clone()),
        Box::new(move |_events| Ok(Box::new(NullUi::with_tap(tap)))),
    )
    .await?;
    Ok((daemon, ui))
}

impl TestDaemon {
    pub async fn start() -> Self {
        let bus = PrivateBus::start();
        let dir = tempfile::tempdir().unwrap();
        let clock = FakeClock::at_unix(1_800_000_000);
        let config = config(&bus, dir.path());
        let socket = config.socket_path.clone();
        let colors = dir.path().join("colors.json");
        let (daemon, ui) = start_daemon(config, &clock).await.unwrap();
        Self {
            daemon: Some(daemon),
            clock,
            ui,
            socket,
            colors,
            dir,
            bus,
        }
    }

    pub fn daemon(&self) -> &Daemon {
        self.daemon.as_ref().unwrap()
    }

    pub async fn proxy(&self) -> NotificationsProxy<'static> {
        NotificationsProxy::new(&self.bus.client().await)
            .await
            .unwrap()
    }

    /// One IPC request over the real socket.
    pub async fn ctl(&self, request: Request) -> Response {
        let socket = self.socket.clone();
        tokio::task::spawn_blocking(move || client::send(&socket, request).unwrap())
            .await
            .unwrap()
    }

    /// `ctl` for requests that return a value.
    pub async fn value(&self, request: Request) -> String {
        match self.ctl(request).await {
            Response::Ok(Some(v)) => v,
            other => panic!("expected a value, got {other:?}"),
        }
    }

    /// Waits for a UI update matching `pred`, skipping others.
    pub async fn wait_ui<T>(&self, mut pred: impl FnMut(&UiUpdate) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match self.ui.try_recv() {
                Ok(update) => {
                    if let Some(t) = pred(&update) {
                        return t;
                    }
                }
                Err(std_mpsc::TryRecvError::Empty) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "timed out waiting for a UI update"
                    );
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Err(e) => panic!("ui tap closed: {e}"),
            }
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(d) = self.daemon.take() {
            d.shutdown().await;
        }
    }
}

/// Notify with no actions/hints and default timeout.
pub async fn simple_notify(proxy: &NotificationsProxy<'_>, app: &str, summary: &str) -> u32 {
    proxy
        .notify(app, 0, "", summary, "", &[], HashMap::new(), -1)
        .await
        .unwrap()
}

/// Awaits a stream item with a timeout.
pub async fn next<S: futures_util::Stream + Unpin>(stream: &mut S) -> S::Item {
    use futures_util::StreamExt as _;
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timed out waiting for a signal")
        .expect("signal stream ended")
}
