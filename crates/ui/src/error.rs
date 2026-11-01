//! Error type for starting and running the UI thread.

use smithay_client_toolkit::reexports::client::{
    ConnectError, DispatchError, globals::GlobalError,
};

/// Errors from the UI thread's setup or event loop.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    #[error("cannot connect to the Wayland display: {0}")]
    Connect(#[from] ConnectError),
    #[error("cannot read the Wayland registry: {0}")]
    Registry(#[from] GlobalError),
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
