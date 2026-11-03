//! Error type for starting and running the UI thread.

use smithay_client_toolkit::reexports::client::{
    ConnectError, DispatchError, globals::GlobalError,
};

/// Errors from the UI thread's setup or event loop.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("cannot connect to the Wayland display: {0}")]
    Connect(#[from] ConnectError),
    #[error("cannot connect to the Wayland socket {path}: {source}")]
    Socket {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("cannot read the Wayland registry: {0}")]
    Registry(#[from] GlobalError),
    #[error(
        "this compositor does not support wlr-layer-shell, which tidings needs \
         to show its panel and toasts (GNOME does not; KDE Plasma, Hyprland, \
         sway, niri, river, labwc, Wayfire and COSMIC do)"
    )]
    NoLayerShell,
    #[error("required Wayland global is missing: {0}")]
    MissingGlobal(&'static str),
    #[error("Wayland dispatch failed: {0}")]
    Dispatch(#[from] DispatchError),
    #[error("event loop error: {0}")]
    EventLoop(String),
    #[error("shared-memory buffer error: {0}")]
    Buffer(String),
    #[error("cannot spawn the UI thread: {0}")]
    Thread(#[from] std::io::Error),
    #[error("the UI thread exited during startup")]
    Startup,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_layer_shell_says_what_to_do() {
        let msg = UiError::NoLayerShell.to_string();
        assert!(
            msg.contains("wlr-layer-shell") && msg.contains("GNOME"),
            "{msg}"
        );
    }
}
