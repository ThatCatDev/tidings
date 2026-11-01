//! Unix-socket IPC: the daemon side ([`server`]), the `tidings ctl` side
//! ([`client`]) and the shared line protocol ([`proto`]).

pub mod client;
pub mod proto;
pub mod server;

use std::path::PathBuf;

/// Socket file name inside `$XDG_RUNTIME_DIR`.
pub const SOCKET_NAME: &str = "tidings.sock";

/// The IPC socket path: `$TIDINGS_SOCKET`, else `$XDG_RUNTIME_DIR/tidings.sock`.
#[must_use]
pub fn socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TIDINGS_SOCKET").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|p| !p.is_empty())
        .map(|dir| PathBuf::from(dir).join(SOCKET_NAME))
}
