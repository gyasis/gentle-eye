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
        // A read timeout, because `read_line` on a client that connects and
        // says nothing blocks FOREVER — and this loop is single-threaded, so
        // one silent socket froze the timeline surface for everyone until that
        // client died.
        //
        // ⚠ A MITIGATION, NOT A FIX. The loop is still single-threaded, so N
        // silent clients serialise into ~5N seconds of stall for a legitimate
        // caller (measured: one silent client delays the next request by 5.06s),
        // and an honest client sending a large request line over more than five
        // seconds is cut off. Thread-per-connection is the real answer;
        // localhost-only makes this survivable, not correct.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));

        // And a panic guard. One bad request must not end the server: without
        // this, a single panic anywhere in parsing unwinds out of the loop and
        // every later request goes unanswered, with nothing logged to say why.
        let svc = Arc::clone(&service);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            handle(stream, &svc)
        }));
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "dayflow http request failed"),
            Err(_) => tracing::error!("dayflow http request PANICKED; surface continues"),
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
    let mutating = matches!(path, "/dayflow/start" | "/dayflow/stop");
    let allowed = if mutating { method == "POST" } else { method == "GET" };
    if !allowed {
        return (
            "405 Method Not Allowed",
            json_err(if mutating {
                "start and stop change state and require POST"
            } else {
                "only GET is served for reads"
            }),
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
        "/dayflow/timeline" => match crate::dayflow::service::resolve_range(
            param(query, "from").as_deref(),
            param(query, "to").as_deref(),
            now,
        ) {
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
            match crate::dayflow::service::resolve_range(
                param(query, "from").as_deref(),
                param(query, "to").as_deref(),
                now,
            ) {
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
                // BYTE slice, not a str slice. `&s[i+1..i+3]` panics when those
                // two bytes fall inside a multi-byte character — and a request
                // line is any valid UTF-8, so `?question=%aé` was a one-line
                // remote kill for the whole surface.
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or(()) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    // A malformed escape is kept verbatim rather than dropped:
                    // silently eating part of a user's question is worse than
                    // showing them the stray percent sign.
                    Err(()) => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percent_escape_straddling_a_multibyte_character_does_not_panic() {
        // `&s[i+1..i+3]` on a str panics when those bytes fall inside a
        // multi-byte character — and a request line is any valid UTF-8, so
        // `?question=%aé` was a one-line remote kill for the whole surface.
        // Every one of these used to abort the serving thread.
        for hostile in ["%aé", "%é", "%\u{00e9}\u{00e9}", "a%bécd", "%🎉x", "100%é"] {
            let decoded = percent_decode(hostile);
            assert!(!decoded.is_empty(), "{hostile:?} decoded to nothing");
        }
    }

    #[test]
    fn decoding_is_faithful_and_keeps_what_it_cannot_decode() {
        assert_eq!(percent_decode("what%20was%20I%20doing"), "what was I doing");
        assert_eq!(percent_decode("a+b"), "a b", "+ is a space in a query string");
        assert_eq!(percent_decode("100%"), "100%", "a bare trailing percent is kept");
        assert_eq!(percent_decode("%zz"), "%zz", "a malformed escape is kept verbatim");
        assert_eq!(percent_decode("%2B"), "+", "and an escaped plus is a plus");
        assert_eq!(percent_decode(""), "");
    }

    #[test]
    fn state_changing_routes_refuse_a_safe_method() {
        // GET is safe by definition, and anything that speculatively fetches
        // localhost URLs — a browser preconnect, a link checker, a monitoring
        // probe — would otherwise be able to stop someone's recording.
        let store = std::sync::Arc::new(crate::dayflow::timeline::SqliteTimelineStore::new(
            std::sync::Arc::new(std::sync::Mutex::new(
                crate::storage::database::init_in_memory().unwrap(),
            )),
        ));
        let svc = DayflowService::new(store, crate::config::DayflowConfig::default());

        assert_eq!(route("GET", "/dayflow/start", "", &svc).0, "405 Method Not Allowed");
        assert_eq!(route("GET", "/dayflow/stop", "", &svc).0, "405 Method Not Allowed");
        // …while reads stay available over GET.
        assert_eq!(route("GET", "/dayflow/status", "", &svc).0, "200 OK");
        assert_eq!(route("GET", "/dayflow/timeline", "", &svc).0, "200 OK");
    }
}
