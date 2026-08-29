//! The HTTP surface (T043) — a third adapter over the one engine.
//!
//! Hand-rolled and GET-only, mirroring `preview::gallery`: US6 asks for a
//! surface, not a web framework, and adding a dependency for five routes would
//! be paid for on every build forever.
//!
//! Every handler is the same three lines — parse, call
//! [`DayflowService`](crate::dayflow::service::DayflowService), serialise —
//! because a handler that decides anything is a decision the MCP and CLI
//! surfaces do not make.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::dayflow::service::DayflowService;

/// Bind the dayflow HTTP surface to `127.0.0.1:port` (0 = ephemeral).
///
/// Loopback only, and not configurable: the timeline is a record of everything
/// the user looked at, so binding it to a routable address is not a decision to
/// leave to a config file.
pub fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
}

/// Serve until `listener` is closed.
pub fn serve(listener: TcpListener, service: Arc<DayflowService>) {
    for stream in listener.incoming().flatten() {
        // One bad request must not end the server: a malformed line from a port
        // scanner would otherwise stop the surface for everyone.
        if let Err(e) = handle(stream, &service) {
            tracing::warn!(error = %e, "dayflow http request failed");
        }
    }
}

fn handle(mut stream: TcpStream, service: &DayflowService) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (code, body) = route_at(method, path, query, service, Utc::now());
    write!(
        stream,
        "HTTP/1.1 {code}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Resolve one request to a status line and a JSON body.
///
/// Separate from the socket so the ROUTING — which is the part that can be
/// wrong — is testable without binding a port.
pub fn route(method: &str, path: &str, query: &str, service: &DayflowService) -> (&'static str, String) {
    route_at(method, path, query, service, Utc::now())
}

/// [`route`], with the clock supplied.
///
/// The seam exists because the interesting states are TIME-dependent: a session
/// is degraded only once it has been quiet for longer than its interval, and a
/// route that read the clock itself could not be driven into that state at all.
/// A behaviour no test can reach is a behaviour nothing defends — the
/// degraded-returns-200 rule survived a mutation for exactly that reason.
pub fn route_at(
    method: &str,
    path: &str,
    query: &str,
    service: &DayflowService,
    now: DateTime<Utc>,
) -> (&'static str, String) {
    if method != "GET" && method != "POST" {
        return (
            "405 Method Not Allowed",
            json_err("only GET and POST are served"),
        );
    }
    match path {
        "/dayflow/start" => match start_from_query(query, service, now) {
            Ok(id) => ("200 OK", serde_json::json!({ "session_id": id }).to_string()),
            Err(e) => ("400 Bad Request", json_err(&e)),
        },
        "/dayflow/stop" => match service.stop(now) {
            Ok(closed) => (
                "200 OK",
                serde_json::json!({ "windows_closed": closed.len() }).to_string(),
            ),
            Err(e) => ("409 Conflict", json_err(&e.to_string())),
        },
        // A DEGRADED session still returns 200 with the degradation in the
        // payload. A 503 for "running but not producing" makes every monitor
        // treat a recoverable state as an outage, and the state is right there
        // in the body for anything that wants to act on it.
        "/dayflow/status" => match service.status(now) {
            Ok(s) => ("200 OK", serde_json::to_string(&s).unwrap_or_else(|_| json_err("serialize"))),
            Err(e) => ("500 Internal Server Error", json_err(&e.to_string())),
        },
        "/dayflow/timeline" => match range_from_query(query, now) {
            Err(e) => ("400 Bad Request", json_err(&e)),
            Ok((from, to)) => match service.timeline(from, to) {
                Ok(entries) => (
                    "200 OK",
                    serde_json::json!({
                        "from": from.to_rfc3339(),
                        "to": to.to_rfc3339(),
                        "entries": entries,
                    })
                    .to_string(),
                ),
                Err(e) => ("500 Internal Server Error", json_err(&e.to_string())),
            },
        },
        "/dayflow/ask" => {
            let question = param(query, "question").unwrap_or_default();
            if question.is_empty() {
                return ("400 Bad Request", json_err("question is required"));
            }
            match range_from_query(query, now) {
                Err(e) => ("400 Bad Request", json_err(&e)),
                Ok((from, to)) => match service.ask(&question, from, to, |p| {
                    format!("[no model configured for ask]\n{p}")
                }) {
                    Ok(a) => (
                        "200 OK",
                        serde_json::to_string(&a).unwrap_or_else(|_| json_err("serialize")),
                    ),
                    Err(e) => ("500 Internal Server Error", json_err(&e.to_string())),
                },
            }
        }
        _ => ("404 Not Found", json_err("no such route")),
    }
}

fn start_from_query(query: &str, service: &DayflowService, now: DateTime<Utc>) -> Result<String, String> {
    let mode = match param(query, "mode").as_deref() {
        None | Some("session") => crate::dayflow::models::DayflowMode::Session,
        Some("daemon") => crate::dayflow::models::DayflowMode::Daemon,
        Some(other) => return Err(format!("unknown mode '{other}': use session or daemon")),
    };
    let displays = match param(query, "displays") {
        Some(list) => list
            .split(',')
            .map(|s| s.trim().parse::<u32>().map_err(|e| format!("bad displays: {e}")))
            .collect::<Result<Vec<_>, _>>()?,
        None => vec![0],
    };
    service
        .start(mode, displays, now)
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())
}

/// The same "today so far" default the other two surfaces use.
fn range_from_query(query: &str, now: DateTime<Utc>) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let parse = |s: &str| {
        DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| format!("bad timestamp '{s}': {e}"))
    };
    let to = match param(query, "to") {
        Some(s) => parse(&s)?,
        None => now,
    };
    let from = match param(query, "from") {
        Some(s) => parse(&s)?,
        None => to.date_naive().and_hms_opt(0, 0, 0).map(|d| d.and_utc()).unwrap_or(to),
    };
    if from > to {
        return Err(format!("range starts after it ends: {from} > {to}"));
    }
    Ok((from, to))
}

fn param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

/// Minimal percent-decoding — enough for a question in a query string.
///
/// **`+` decodes to a space**, which is correct form-encoding and a genuine
/// trap for timestamps: a raw RFC-3339 offset `+00:00` arrives as ` 00:00` and
/// fails to parse. Callers should send `Z`-form timestamps (what
/// `to_rfc3339_opts(.., true)` emits) or escape the plus as `%2B`. The error
/// message quotes the mangled value so the cause is visible rather than
/// mysterious.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    // A malformed escape is kept verbatim rather than dropped:
                    // silently eating part of a user's question is worse than
                    // showing them the stray percent sign.
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn json_err(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}
