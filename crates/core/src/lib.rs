//! Shared domain types for tidings.
//!
//! This crate is the contract between the daemon (D-Bus server, gesture input,
//! app state) and the UI (Wayland rendering). It has no I/O and no platform
//! dependencies. The modules marked CONTRACT are fixed: the daemon and the UI
//! are built in parallel against them, so change them only in coordination.
//!
//! Implementation modules (store, policy, ...) are added beside these.

pub mod gesture; // CONTRACT
pub mod model; // CONTRACT
pub mod theme; // CONTRACT
pub mod ui; // CONTRACT
pub mod view; // CONTRACT

pub mod clock;
pub mod panel;
pub mod policy;
pub mod store;
pub mod theme_file;
pub mod time_label;
pub mod toasts;
pub mod view_builder;
