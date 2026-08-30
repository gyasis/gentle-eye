//! Attaching to a running daemon (D014-15).
//!
//! A surface that builds its own `DayflowService` gets a PRIVATE session: it
//! reports "not running" while a daemon is capturing, and its `start` creates a
//! second session nothing else can see. This module is how the CLI and MCP
//! reach the one that is actually running.
//!
//! Hand-rolled over `TcpStream` for the same reason the server is: the surface
//! speaks five routes to localhost, and a dependency for that is paid for on
//! every build forever.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::contracts::errors::DayflowError;

/// A handle to a daemon's HTTP surface.
pub struct DaemonClient {
    port: u16,
    timeout: Duration,
}

impl DaemonClient {
    /// Talk to the daemon on `port`.
    pub fn new(port: u16) -> Self {
        // Short, because this is localhost and a surface that hangs is worse
        // than one that says the daemon is unreachable: a CLI blocking on a
        // dead socket looks like a hung capture.
        Self { port, timeout: Duration::from_secs(3) }
    }

    /// Find the running daemon from its state file, if there is one.
    ///
    /// Returns `None` when no daemon is running, no port is published, or the
    /// published port does not answer — all of which mean "there is nothing to
    /// attach to", and the caller falls back to a local engine. A STALE port
    /// answered by something else is the one case this cannot detect; the
    /// health probe below is what bounds it.
    pub fn discover(store: &crate::dayflow::daemon::DaemonStateStore) -> Option<Self> {
        let port = store.load().ok()??.port?;
        let c = Self::new(port);
        c.get("/dayflow/status").ok().map(|_| c)
    }

    /// GET a route, returning the body.
    pub fn get(&self, path: &str) -> Result<String, DayflowError> {
        self.request("GET", path)
    }

    /// POST a route, returning the body.
    pub fn post(&self, path: &str) -> Result<String, DayflowError> {
        self.request("POST", path)
    }

    fn request(&self, method: &str, path: &str) -> Result<String, DayflowError> {
        let addr = format!("127.0.0.1:{}", self.port);
        let mut stream = TcpStream::connect(&addr)
            .map_err(|e| DayflowError::Invalid(format!("daemon at {addr} unreachable: {e}")))?;
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .map_err(|e| DayflowError::Invalid(format!("writing to the daemon: {e}")))?;

        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader
            .read_line(&mut status)
            .map_err(|e| DayflowError::Invalid(format!("reading the daemon's reply: {e}")))?;
        // Drain headers to the blank line.
        loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| DayflowError::Invalid(format!("reading headers: {e}")))?;
            if n == 0 || line.trim().is_empty() {
                break;
            }
        }
        let mut body = String::new();
        reader
            .read_to_string_compat(&mut body)
            .map_err(|e| DayflowError::Invalid(format!("reading the body: {e}")))?;

        if !status.contains(" 200 ") {
            // The body carries the daemon's own message; surfacing the status
            // line alone would replace a specific error with a number.
            return Err(DayflowError::Invalid(format!(
                "daemon refused {method} {path}: {} {body}",
                status.trim()
            )));
        }
        Ok(body)
    }
}

/// `read_to_string` on a `BufReader<TcpStream>` without pulling in a trait
/// import at every call site.
trait ReadToStringCompat {
    fn read_to_string_compat(&mut self, out: &mut String) -> std::io::Result<usize>;
}

impl<R: std::io::Read> ReadToStringCompat for R {
    fn read_to_string_compat(&mut self, out: &mut String) -> std::io::Result<usize> {
        std::io::Read::read_to_string(self, out)
    }
}
