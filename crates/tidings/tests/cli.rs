//! Tests of the `tidings` binary itself: `ctl` exit codes and `daemon`
//! refusing to replace a running notification daemon.

#![allow(clippy::unwrap_used)] // test helpers

mod common;

use std::path::PathBuf;
use std::process::{Command, Output};

use common::TestDaemon;

/// Runs the binary with `envs` added (off the async thread).
async fn run(args: &'static [&'static str], envs: Vec<(&'static str, PathBuf)>) -> Output {
    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tidings"));
        cmd.args(args).env_remove("TIDINGS_SOCKET").envs(envs);
        cmd.output().unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn ctl_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.sock");
    let out = run(&["ctl", "count"], vec![("TIDINGS_SOCKET", missing)]).await;
    assert_eq!(out.status.code(), Some(2), "daemon not running");
    assert!(String::from_utf8_lossy(&out.stderr).contains("not running"));

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

#[tokio::test]
async fn daemon_refuses_to_replace_a_running_daemon() {
    let t = TestDaemon::start().await;
    let runtime_dir = tempfile::tempdir().unwrap();
    let bus = t.bus.address.clone();
    let colors = t.dir.path().join("colors.json");
    let out = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_tidings"))
            .args(["daemon", "--no-gestures"])
            // Only this child sees the private bus as its session bus.
            .env("DBUS_SESSION_BUS_ADDRESS", bus)
            .env("XDG_RUNTIME_DIR", runtime_dir.path())
            .env("TIDINGS_COLORS", colors)
            .env_remove("TIDINGS_SOCKET")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already owned"), "{stderr}");
    // The running daemon is untouched.
    let proxy = t.proxy().await;
    assert_eq!(common::simple_notify(&proxy, "a", "b").await, 1);
    t.shutdown().await;
}
