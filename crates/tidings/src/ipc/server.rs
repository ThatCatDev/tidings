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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::AsyncReadExt as _;
    use tokio::sync::mpsc;

    use super::*;

    /// Sends `bytes` on a fresh connection and returns everything the server
    /// writes back before it closes the connection.
    async fn exchange(path: &Path, bytes: &[u8]) -> String {
        let mut stream = UnixStream::connect(path).await.unwrap();
        stream.write_all(bytes).await.unwrap();
        stream.shutdown().await.unwrap();
        let mut reply = String::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_string(&mut reply))
            .await
            .unwrap()
            .unwrap();
        reply
    }

    #[tokio::test]
    async fn bind_replaces_a_stale_socket_but_not_a_live_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sock");
        // A socket file left behind by a daemon that died: nobody listens.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let server = IpcServer::bind(&path).await.unwrap();

        let err = IpcServer::bind(&path).await.unwrap_err();
        assert!(matches!(&err, BindError::InUse(p) if *p == path), "{err}");
        assert!(path.exists(), "the live socket is kept");

        drop(server);
        assert!(!path.exists(), "dropping the server removes its socket");
    }

    #[tokio::test]
    async fn bind_reports_an_unusable_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("t.sock");
        let err = IpcServer::bind(&path).await.unwrap_err();
        assert!(
            matches!(&err, BindError::Io { path: p, .. } if *p == path),
            "{err}"
        );
        assert!(err.to_string().starts_with("cannot bind "), "{err}");
    }

    #[tokio::test]
    async fn requests_are_answered_in_order_and_bad_ones_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sock");
        let server = IpcServer::bind(&path).await.unwrap();
        let (events, mut rx) = mpsc::unbounded_channel();
        let serving = tokio::spawn(server.serve(events));
        let actor = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let AppEvent::Ipc(request, reply) = event {
                    let _ = reply.send(Response::Ok(Some(request.to_string())));
                }
            }
        });

        let reply = exchange(&path, b"count\nbogus\ndnd on\n").await;
        assert!(
            reply.starts_with("ok count\nerr ") && reply.ends_with("\nok dnd on\n"),
            "{reply:?}"
        );
        // A line that is not text ends that connection, not the server.
        assert_eq!(exchange(&path, b"\xff\xfe\n").await, "");
        assert_eq!(exchange(&path, b"is-open\n").await, "ok is-open\n");

        serving.abort();
        actor.abort();
    }

    #[tokio::test]
    async fn requests_during_shutdown_get_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sock");
        let server = IpcServer::bind(&path).await.unwrap();
        let (events, mut rx) = mpsc::unbounded_channel();
        let serving = tokio::spawn(server.serve(events));

        // The actor takes the request but stops before answering it.
        let (reply, ()) = tokio::join!(exchange(&path, b"count\n"), async {
            drop(rx.recv().await);
        });
        assert_eq!(reply, "err daemon is shutting down\n");

        // The actor is gone: requests are refused, and the server stops
        // after that connection.
        drop(rx);
        assert_eq!(
            exchange(&path, b"count\n").await,
            "err daemon is shutting down\n"
        );
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .unwrap()
            .unwrap();
    }
}
