//! Serves a test page that sends Chrome web notifications, to try tidings'
//! inline replies, action buttons and site names by hand.
//!
//! Chrome only gives notifications buttons and reply fields when a service
//! worker shows them, and service workers need a secure origin, which
//! `localhost` is (a `file://` page is not). So this serves the page on
//! `http://localhost:8317` (or the port given as the first argument):
//!
//! ```sh
//! cargo run -p tidings --example web_notify
//! ```

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

const INDEX: &str = include_str!("web_notify/index.html");
const WORKER: &str = include_str!("web_notify/sw.js");

fn main() -> std::io::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8317);
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("open http://localhost:{port} in Chrome (Ctrl+C to stop)");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = serve(stream) {
                    eprintln!("request failed: {e}");
                }
            }
            Err(e) => eprintln!("connection failed: {e}"),
        }
    }
    Ok(())
}

/// Answers one request: the page, its service worker, or 404.
fn serve(mut stream: TcpStream) -> std::io::Result<()> {
    let mut request_line = String::new();
    BufReader::new(&stream).read_line(&mut request_line)?;
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let path = path.split('?').next().unwrap_or(path);
    let (status, kind, body) = match path {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", INDEX),
        "/sw.js" => ("200 OK", "text/javascript; charset=utf-8", WORKER),
        _ => ("404 Not Found", "text/plain; charset=utf-8", "not found\n"),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}
