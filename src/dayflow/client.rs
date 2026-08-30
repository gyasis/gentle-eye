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
    /// Time allowed for the TCP connect ALONE. Short and separate from the
    /// read budget (the D014-12 lesson): a flat budget cannot tell a dead
    /// endpoint from a slow response — a dead port must fail in about a
    /// second, while a legitimate slow route (a summarising daemon under
    /// load) deserves the full read window.
    connect_timeout: Duration,
    /// Time allowed for the response once connected.
    read_timeout: Duration,
}

/// What probing the state file found — three distinguishable situations, not
/// two. "No daemon" and "a daemon record whose port does not answer" call for
/// different behaviour: the first is a clean local fallback, the second is a
/// crashed daemon's leftovers OR a wedged daemon, and silently pretending it
/// is the first hides exactly the divergence D014-15 exists to prevent.
pub enum Discovery {
    /// No state file, or no published port: nothing was ever serving.
    NoDaemon,
    /// A record names `port` but nothing answered there — a crashed daemon's
    /// stale record, or a live daemon too wedged to answer. The caller should
    /// SAY SO before falling back to a local view.
    Stale { port: u16 },
    /// A daemon answered; attach to it.
    Live(DaemonClient),
}

impl DaemonClient {
    /// Talk to the daemon on `port`.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            connect_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(10),
        }
    }

    /// Probe the state file: is a daemon serving, gone, or unresponsive?
    pub fn probe(store: &crate::dayflow::daemon::DaemonStateStore) -> Discovery {
        let Some(state) = store.load().ok().flatten() else {
            return Discovery::NoDaemon;
        };
        let Some(port) = state.port else {
            return Discovery::NoDaemon;
        };
        let c = Self::new(port);
        match c.get("/dayflow/status") {
            Ok(_) => Discovery::Live(c),
            Err(_) => Discovery::Stale { port },
        }
    }

    /// Find the running daemon from its state file, if there is one.
    ///
    /// `None` collapses both "nothing was serving" and "a record that did not
    /// answer" — use [`DaemonClient::probe`] when the caller should report the
    /// difference (the CLI does).
    pub fn discover(store: &crate::dayflow::daemon::DaemonStateStore) -> Option<Self> {
        match Self::probe(store) {
            Discovery::Live(c) => Some(c),
            Discovery::NoDaemon | Discovery::Stale { .. } => None,
        }
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
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], self.port));
        // `connect_timeout`, NOT bare `connect`: a bare connect has no budget
        // of its own, and a wedged listener (accept queue full, host asleep)
        // hangs the CLI for the OS default — minutes — while looking like a
        // hung capture.
        let mut stream = TcpStream::connect_timeout(&addr, self.connect_timeout)
            .map_err(|e| DayflowError::Invalid(format!("daemon at {addr} unreachable: {e}")))?;
        stream.set_read_timeout(Some(self.read_timeout)).ok();
        stream.set_write_timeout(Some(self.read_timeout)).ok();
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

/// Percent-encode a query value for the daemon's routes.
///
/// ONE copy, here beside the client that sends it, used by both attaching
/// surfaces (CLI and MCP) — a second encoder is how the two surfaces send
/// different bytes for the same question (the R37/R40 duplicate-drift class).
/// Small and local on purpose: the alternative is a dependency for five routes
/// against localhost.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
