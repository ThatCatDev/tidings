//! Daemon side of the IPC socket: one request line in, one response line out,
//! any number of requests per connection.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

use crate::app::{AppEvent, AppSender};
use crate::ipc::proto::{Request, Response};

/// A bound IPC socket. The socket file is removed on drop.
#[derive(Debug)]
pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
}

/// Errors binding the socket.
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    /// A live daemon is already listening there.
    #[error("another tidings instance is listening on {0}")]
    InUse(PathBuf),
    /// Creating the socket failed.
    #[error("cannot bind {path}: {source}")]
    Io {
        /// The socket path.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
}

impl IpcServer {
    /// Binds `path`, replacing a stale socket file but never a live one.
    pub async fn bind(path: &Path) -> Result<Self, BindError> {
        let io_err = |source| BindError::Io {
            path: path.to_owned(),
            source,
        };
        if path.exists() {
            if UnixStream::connect(path).await.is_ok() {
                return Err(BindError::InUse(path.to_owned()));
            }
            std::fs::remove_file(path).map_err(io_err)?;
        }
        let listener = UnixListener::bind(path).map_err(io_err)?;
        Ok(Self {
            listener,
            path: path.to_owned(),
        })
    }

    /// Accepts connections until the actor goes away, forwarding requests to it.
    pub async fn serve(self, events: AppSender) {
        loop {
            match self.listener.accept().await {
                Ok((stream, _)) => {
                    let events = events.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle(stream, events).await {
                            tracing::debug!("ipc connection: {e}");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("ipc accept failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            if events.is_closed() {
                return;
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn handle(stream: UnixStream, events: AppSender) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match Request::parse(&line) {
            Err(e) => Response::Err(e.to_string()),
            Ok(request) => {
                let (tx, rx) = oneshot::channel();
                if events.send(AppEvent::Ipc(request, tx)).is_err() {
                    Response::Err("daemon is shutting down".into())
                } else {
                    rx.await
                        .unwrap_or_else(|_| Response::Err("daemon is shutting down".into()))
                }
            }
        };
        write.write_all(format!("{response}\n").as_bytes()).await?;
    }
    Ok(())
}
