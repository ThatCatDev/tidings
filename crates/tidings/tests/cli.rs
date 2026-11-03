//! Tests of the `tidings` binary itself: `ctl` exit codes, and `daemon`
//! refusing to replace a running notification daemon or failing cleanly.

#![allow(clippy::unwrap_used)] // test helpers

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Output;

use common::TestDaemon;

/// Runs the binary with a throwaway `HOME` and `envs` added (off the async thread).
async fn run(args: &'static [&'static str], envs: Vec<(&'static str, PathBuf)>) -> Output {
    tokio::task::spawn_blocking(move || run_blocking(args, envs))
        .await
        .unwrap()
}

fn run_blocking(args: &[&str], envs: Vec<(&'static str, PathBuf)>) -> Output {
    let home = tempfile::tempdir().unwrap();
    common::tidings_cmd(home.path())
        .args(args)
        .envs(envs)
        .output()
        .unwrap()
}

/// A stand-in daemon on `path` that reads one request and answers `reply`
/// verbatim (or hangs up when it is `None`). Returns the request it got.
fn fake_daemon(path: &Path, reply: Option<&'static str>) -> std::thread::JoinHandle<String> {
    let listener = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        if let Some(reply) = reply {
            stream.write_all(reply.as_bytes()).unwrap();
        }
        request
    })
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[tokio::test]
async fn ctl_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.sock");
    let out = run(&["ctl", "count"], vec![("TIDINGS_SOCKET", missing)]).await;
    assert_eq!(out.status.code(), Some(2), "daemon not running");
    assert!(stderr(&out).contains("not running"));

    let t = TestDaemon::start().await;
    let sock = t.socket.clone();
    let out = run(&["ctl", "count"], vec![("TIDINGS_SOCKET", sock.clone())]).await;
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "0\n");
    let out = run(
        &["ctl", "dnd", "on"],
        vec![("TIDINGS_SOCKET", sock.clone())],
    )
    .await;
    assert_eq!(String::from_utf8_lossy(&out.stdout), "on\n");
    let out = run(&["ctl", "toggle"], vec![("TIDINGS_SOCKET", sock.clone())]).await;
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "no value, nothing printed");
    let out = run(
        &["ctl", "gesture", "update", "-0.2"],
        vec![("TIDINGS_SOCKET", sock.clone())],
    )
    .await;
    assert_eq!(out.status.code(), Some(0));
    let out = run(&["ctl", "dnd", "maybe"], vec![("TIDINGS_SOCKET", sock)]).await;
    assert_eq!(
        out.status.code(),
        Some(1),
        "usage errors are errors, not 'not running'"
    );
    t.shutdown().await;
}

#[test]
fn ctl_finds_the_socket_in_the_runtime_dir() {
    let dir = tempfile::tempdir().unwrap();
    let server = fake_daemon(&dir.path().join("tidings.sock"), Some("ok 3\n"));
    let out = run_blocking(
        &["ctl", "count"],
        vec![("XDG_RUNTIME_DIR", dir.path().to_owned())],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "3\n");
    assert_eq!(server.join().unwrap(), "count\n");
}

#[test]
fn ctl_without_a_runtime_dir_is_an_error() {
    let out = run_blocking(&["ctl", "count"], vec![]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("XDG_RUNTIME_DIR is not set"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn ctl_reports_the_daemons_error() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("t.sock");
    let server = fake_daemon(&sock, Some("err no can do\n"));
    let out = run_blocking(&["ctl", "open"], vec![("TIDINGS_SOCKET", sock)]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "tidings: no can do\n");
    assert_eq!(server.join().unwrap(), "open\n");
}

#[test]
fn ctl_reports_a_malformed_reply() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("t.sock");
    let server = fake_daemon(&sock, Some("what?\n"));
    let out = run_blocking(&["ctl", "count"], vec![("TIDINGS_SOCKET", sock)]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a broken daemon is not 'not running'"
    );
    assert!(
        stderr(&out).contains("malformed response"),
        "{}",
        stderr(&out)
    );
    server.join().unwrap();
}

#[test]
fn ctl_reports_a_daemon_that_hangs_up() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("t.sock");
    let server = fake_daemon(&sock, None);
    let out = run_blocking(&["ctl", "count"], vec![("TIDINGS_SOCKET", sock)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).starts_with("tidings: ipc: "),
        "{}",
        stderr(&out)
    );
    server.join().unwrap();
}

#[test]
fn ctl_reports_an_unusable_socket_path() {
    // Longer than a Unix socket address can hold: not "not running", an error.
    let long = PathBuf::from(format!("/tmp/{}.sock", "x".repeat(200)));
    let out = run_blocking(&["ctl", "count"], vec![("TIDINGS_SOCKET", long)]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).starts_with("tidings: ipc: "),
        "{}",
        stderr(&out)
    );
}

#[test]
fn daemon_without_a_runtime_dir_is_an_error() {
    // No subcommand runs the daemon.
    let out = run_blocking(&[], vec![]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("XDG_RUNTIME_DIR is not set"),
        "{}",
        stderr(&out)
    );
}

#[tokio::test]
async fn daemon_refuses_to_replace_a_running_daemon() {
    let t = TestDaemon::start().await;
    let runtime_dir = tempfile::tempdir().unwrap();
    let bus = PathBuf::from(&t.bus.address);
    let out = run(
        &["daemon", "--no-gestures"],
        vec![
            // Only this child sees the private bus as its session bus.
            ("DBUS_SESSION_BUS_ADDRESS", bus),
            ("XDG_RUNTIME_DIR", runtime_dir.path().to_owned()),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("already owned"), "{}", stderr(&out));
    // The running daemon is untouched.
    let proxy = t.proxy().await;
    assert_eq!(common::simple_notify(&proxy, "a", "b").await, 1);
    t.shutdown().await;
}

#[tokio::test]
async fn daemon_without_a_compositor_fails_cleanly() {
    let bus = common::PrivateBus::start();
    let home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    // A broken config is logged, not fatal.
    let config = home.path().join(".config/tidings/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "[toasts]\nmax_visible = 99\n").unwrap();

    let mut cmd = common::tidings_cmd(home.path());
    cmd.args(["daemon", "--no-gestures"])
        .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
        .env("XDG_RUNTIME_DIR", runtime_dir.path())
        .env("TIDINGS_COLORS", home.path().join("colors.json"));
    let out = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("toasts.max_visible"),
        "config error logged: {err}"
    );
    assert!(err.contains("cannot start the UI"), "{err}");
    assert!(
        !runtime_dir.path().join("tidings.sock").exists(),
        "the IPC socket is removed"
    );
}

#[tokio::test]
async fn daemon_uses_the_configured_theme_and_colours() {
    let bus = common::PrivateBus::start();
    let home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    let config = home.path().join(".config/tidings/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        "[theme]\nname = \"nord\"\ncolors = \"~/my-colors.json\"\n",
    )
    .unwrap();

    let mut cmd = common::tidings_cmd(home.path());
    cmd.args(["daemon", "--no-gestures"])
        .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
        .env("XDG_RUNTIME_DIR", runtime_dir.path())
        .env("TIDINGS_LOG", "info");
    let out = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "no compositor");
    let err = stderr(&out);
    assert!(
        err.contains(&format!("config: {}", config.display())),
        "{err}"
    );
    assert!(err.contains("theme: nord"), "{err}");
    assert!(
        err.contains(&format!(
            "colours: {}",
            home.path().join("my-colors.json").display()
        )),
        "{err}"
    );
}

#[tokio::test]
async fn daemon_follows_noctalia_without_a_colours_file() {
    let bus = common::PrivateBus::start();
    let home = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    let palette = home.path().join(".config/hypr/noctalia.lua");
    std::fs::create_dir_all(palette.parent().unwrap()).unwrap();
    std::fs::write(&palette, include_str!("fixtures/noctalia.lua")).unwrap();

    let mut cmd = common::tidings_cmd(home.path());
    cmd.args(["daemon", "--no-gestures"])
        .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
        .env("XDG_RUNTIME_DIR", runtime_dir.path())
        .env("TIDINGS_LOG", "info");
    let out = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    let err = stderr(&out);
    assert!(
        err.contains(&format!(
            "colours: Noctalia's palette ({})",
            palette.display()
        )),
        "{err}"
    );
}
