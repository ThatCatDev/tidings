//! `tidings ctl` side of the IPC socket (blocking; one request per call).

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::ipc::proto::{Request, Response};

/// How long to wait for the daemon's reply.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Why a request could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// No daemon is listening (exit code 2).
    #[error("tidings daemon is not running (no socket at {0})")]
    NotRunning(String),
    /// Socket I/O failed.
    #[error("ipc: {0}")]
    Io(#[from] io::Error),
    /// The daemon sent a malformed response.
    #[error("ipc: {0}")]
    Protocol(#[from] crate::ipc::proto::ParseError),
}

/// Sends one request and returns the daemon's response.
pub fn send(path: &Path, request: Request) -> Result<Response, ClientError> {
    let mut stream = UnixStream::connect(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
            ClientError::NotRunning(path.display().to_string())
        }
        _ => ClientError::Io(e),
    })?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(format!("{request}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    if line.is_empty() {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }
    Ok(Response::parse(&line)?)
}
