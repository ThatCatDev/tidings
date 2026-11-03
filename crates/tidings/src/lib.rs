//! The tidings daemon: D-Bus notification server, App actor, IPC and wiring.
//!
//! The binary (`main.rs`) is a thin CLI over this library, which integration
//! tests drive directly against a private bus.

pub mod app;
pub mod clock;
pub mod colors;
pub mod config_file;
pub mod daemon;
pub mod dbus;
pub mod ipc;
pub mod null_ui;
pub mod theme_watch;
pub mod themes;
