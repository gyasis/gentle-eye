//! The real `ask_day` answerer (T024).
//!
//! Before this, all three surfaces returned `[no model configured for ask]`
//! followed by the prompt — a range WITH records handed back the question
//! instead of an answer, three times over, in three copies of the same stub.
//!
//! One implementation here, so the surfaces cannot drift: an answer that
//! differs by which surface asked is a bug nobody would think to look for.

use std::sync::OnceLock;
use std::time::Duration;

use crate::config::DayflowConfig;

/// SPLIT timeouts, per D014-12. A single flat budget cannot tell a dead
/// endpoint from a legitimate cold load: measured 2026-08-29, the first
/// ask after an idle period failed with a transport error while the
/// governor spent 95 s loading the reasoning model. A short CONNECT
/// fails a wrong URL in a second; a long READ lets a cold load finish.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT: Duration = Duration::from_secs(180);

/// Calls the reasoning tier through the governed lane.
///
/// `Clone` is cheap — the client is an `Arc` around a shared pool — and it is
/// what lets `answer_blocking` hand the SAME pooled connections to its worker
/// thread instead of rebuilding TLS state per question.
#[derive(Clone)]
pub struct ModelAnswerer {
    endpoint: String,
    model: String,
    /// `Err` when the HTTP client could not be built. Kept as a value rather
    /// than unwrapped at construction so a broken TLS backend produces a
    /// STATED ask failure — and never a default client, whose missing
    /// timeouts would silently undo the D014-12 fix.
    client: Result<reqwest::Client, String>,
}

/// What the answerer needs from the environment, resolved once.
///
/// `GE_DAYFLOW_ENDPOINT` is the governed lane (`…:8799/llm/ollama`), the same
/// variable the live test uses — one name, so a working live run and a working
/// `ask` are the same configuration.
pub fn endpoint_from_env() -> Option<String> {
    std::env::var("GE_DAYFLOW_ENDPOINT").ok().filter(|s| !s.trim().is_empty())
}

/// The one client every answerer shares.
///
/// One client, not one per question: the pool (and its TLS session state) is
/// what makes the SECOND ask of the day skip the handshake the first one paid.
/// The build result is cached including failure — a TLS backend that failed
/// once will fail identically every time, and retrying the build per question
/// would only repeat the same stated error more slowly.
fn shared_client() -> Result<reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(READ_TIMEOUT)
                .build()
                .map_err(|e| e.to_string())
        })
        .clone()
}

/// The one runtime every blocking ask runs on.
///
/// SHARED rather than built per call: reqwest's pooled connections are driven
/// by tasks living on the runtime that made the request, so a per-call runtime
/// takes its pool down with it — the next question would find dead pooled
/// connections and pay a fresh handshake (or worse, a spurious transport error
/// that then consumes the one D014-12 retry).
fn ask_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("dayflow-ask")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

impl ModelAnswerer {
    /// Build from the environment and config, or `None` when unconfigured.
    ///
    /// `None` is not a failure: an unconfigured install must still answer
    /// "no model configured" rather than error, because the timeline is
    /// readable without one.
    pub fn from_env(cfg: &DayflowConfig) -> Option<Self> {
        let endpoint = endpoint_from_env()?;
        let model = std::env::var("GE_DAYFLOW_REASON_MODEL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| cfg.perception.reason_model.clone());
        Some(Self::new(endpoint, model))
    }

    /// An answerer against `endpoint`, asking `model`.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into(), model: model.into(), client: shared_client() }
    }

    /// An answerer with explicit budgets and its own client, for tests that
    /// need a timeout to actually FIRE within a test's lifetime.
    #[cfg(test)]
    fn with_timeouts(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        connect: Duration,
        read: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(connect)
            .timeout(read)
            .build()
            .map_err(|e| e.to_string());
        Self { endpoint: endpoint.into(), model: model.into(), client }
    }

    /// Ask, returning prose, or a stated failure.
    ///
    /// Never panics and never propagates an error: `ask_day`'s answerer returns
    /// a `String`, and the grounding is already attached by the caller — so a
    /// failure here must READ as a failure rather than vanish into an empty
    /// answer that looks like a confident "nothing happened".
    pub async fn answer(&self, prompt: &str) -> String {
        let client = match &self.client {
            Ok(c) => c,
            Err(e) => return format!("[ask failed: could not build an HTTP client: {e}]"),
        };
        let url = format!("{}/api/generate", self.endpoint.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
        });

        // ONE retry on a TRANSPORT error, per D014-12. The first question after
        // an idle period is precisely when the model is cold and the lane drops
        // the connection — the common case, not the edge. Two things do NOT
        // retry, deliberately:
        // - a REJECTED request (the model answered; asking again would double
        //   the cost of every refusal), and
        // - a TIMEOUT: the read budget already waited out the whole cold-load
        //   window, so a second identical wait cannot succeed where the first
        //   ran out — it would turn one bounded 3-minute failure into six
        //   unbounded minutes of silence. D014-12 retries the transport
        //   failure, not the timeout.
        let mut last = String::new();
        for attempt in 1..=2 {
            match client.post(&url).json(&body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = match resp.text().await {
                        Ok(t) => t,
                        Err(e) => {
                            return format!(
                                "[ask failed: reading the model's reply from {url} failed: {e}]"
                            )
                        }
                    };
                    if !status.is_success() {
                        return format!("[ask failed: the model returned {status}: {text}]");
                    }
                    return match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(v) => {
                            let raw = v["response"].as_str().unwrap_or_default();
                            let answer = crate::analysis::ollama::strip_reasoning(raw).trim().to_string();
                            if answer.is_empty() {
                                // An empty answer with real grounding would read
                                // as "nothing happened" — say what actually
                                // occurred instead.
                                "[ask failed: the model returned an empty answer]".to_string()
                            } else {
                                answer
                            }
                        }
                        Err(e) => format!("[ask failed: the model's reply was not JSON: {e}]"),
                    };
                }
                Err(e) if e.is_timeout() => {
                    return format!(
                        "[ask failed: no answer from {url} within the {}s read budget: {e}]",
                        READ_TIMEOUT.as_secs()
                    );
                }
                Err(e) => {
                    last = e.to_string();
                    if attempt == 1 {
                        tracing::warn!(error = %last, "ask transport failed; retrying once");
                    }
                }
            }
        }
        format!("[ask failed: could not reach the model at {url}: {last}]")
    }
}

impl ModelAnswerer {
    /// Ask, from a SYNCHRONOUS caller.
    ///
    /// `ask_day`'s answerer is `FnOnce(&str) -> String`, and the three surfaces
    /// that call it are a mix of sync (the HTTP server's request loop) and
    /// async (the CLI, MCP). `block_on` inside an async context panics, so the
    /// call blocks on a plain thread that drives the SHARED runtime — the
    /// thread is a per-question cost of microseconds against a multi-second
    /// model call, but the runtime and client are not rebuilt: the connection
    /// pool survives between questions (see [`ask_runtime`]).
    ///
    /// Blocking an interactive `ask` is the intended cost: the caller asked a
    /// question and is waiting for the answer. It is NOT used on the capture
    /// path, where blocking would stall recording.
    pub fn answer_blocking(&self, prompt: &str) -> String {
        let answerer = self.clone();
        let prompt = prompt.to_string();
        let handle = std::thread::spawn(move || match ask_runtime() {
            Ok(rt) => rt.block_on(answerer.answer(&prompt)),
            Err(e) => format!("[ask failed: no runtime for the model call: {e}]"),
        });
        handle.join().unwrap_or_else(|_| {
            // A panicked answer thread must not take the surface with it: the
            // grounding is already attached, and a stated failure is readable
            // where a propagated panic is a 500 with no explanation.
            "[ask failed: the model call panicked]".to_string()
        })
    }
}

/// The answer for an install with no model configured.
///
/// Says so, and does NOT echo the prompt back. Returning the prompt made a
/// range with records look like it had produced an answer, which is how the
/// stub survived three surfaces unnoticed.
pub const NO_MODEL: &str =
    "[no model configured for ask — set GE_DAYFLOW_ENDPOINT to the governed lane]";

/// Answer through the configured model, or explain why there is none.
///
/// The ONE entry every surface calls. Three copies of a stub is how the
/// placeholder survived unnoticed on three surfaces; one function is how an
/// answer cannot differ by which surface asked.
pub fn answer_or_explain(cfg: &DayflowConfig, prompt: &str) -> String {
    match ModelAnswerer::from_env(cfg) {
        Some(a) => a.answer_blocking(prompt),
        None => NO_MODEL.to_string(),
    }
}

/// Serialises every test (in this crate's test binary) that reads or writes
/// `GE_DAYFLOW_ENDPOINT` / `GE_DAYFLOW_REASON_MODEL`.
///
/// Tests run as parallel THREADS of one process and the environment is process
/// GLOBAL: without this lock, one test's `remove_var` races another's
/// `set_var`, and whichever reads in between sees the other test's state — a
/// flaky pass/fail that depends on scheduler order. Poisoning is deliberately
/// forgiven: a panicked env test must not cascade into every other env test
/// failing on `lock().unwrap()`.
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    /// An unconfigured install says so, and does NOT echo the prompt back.
    ///
    /// Echoing it is what let the stub survive on three surfaces: a range WITH
    /// records handed back the question, and the response was long and prose-ish
    /// enough to look like an answer.
    #[test]
    fn an_unconfigured_install_says_so_without_echoing_the_prompt() {
        let _env = test_env_lock();
        let cfg = DayflowConfig::default();
        // Ensure the env is genuinely absent for this assertion.
        let prior = std::env::var("GE_DAYFLOW_ENDPOINT").ok();
        std::env::remove_var("GE_DAYFLOW_ENDPOINT");

        let prompt = "SECRET-PROMPT-MARKER what did I do today?";
        let out = answer_or_explain(&cfg, prompt);

        if let Some(v) = prior {
            std::env::set_var("GE_DAYFLOW_ENDPOINT", v);
        }

        assert_eq!(out, NO_MODEL);
        assert!(
            !out.contains("SECRET-PROMPT-MARKER"),
            "the prompt was echoed back as if it were an answer: {out}"
        );
    }

    /// An unreachable endpoint produces a STATED failure, not an empty answer
    /// and not a panic. An empty answer with real grounding attached reads as a
    /// confident "nothing happened".
    #[test]
    fn an_unreachable_endpoint_fails_loudly_rather_than_answering_nothing() {
        // Port 1 on loopback: nothing listens, and the connect fails fast
        // because the connect budget is separate from the read budget (D014-12).
        let a = ModelAnswerer::new("http://127.0.0.1:1", "ornith-1.5-9b:latest");
        let started = std::time::Instant::now();
        let out = a.answer_blocking("what did I do today?");

        assert!(out.starts_with("[ask failed"), "a failure must READ as one: {out}");
        assert!(!out.trim().is_empty());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "a dead endpoint took {:?} — the CONNECT budget is not separate from the \
             read budget, so a wrong URL waits out the cold-load window (D014-12)",
            started.elapsed()
        );
    }

    /// A server that accepts connections and hands each accepted socket to
    /// `on_accept`, counting accepts. The COUNT is the observable: it is the
    /// number of HTTP attempts the client actually made.
    fn counting_server(
        on_accept: fn(std::net::TcpStream, &mut Vec<std::net::TcpStream>),
    ) -> (std::net::SocketAddr, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let accepts = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let (accepts_t, done_t) = (accepts.clone(), done.clone());
        std::thread::spawn(move || {
            listener.set_nonblocking(true).expect("nonblocking");
            // Sockets a behaviour wants HELD (open but silent) live here so
            // they are not dropped — dropping would turn a timeout fixture
            // into a transport-error fixture.
            let mut held = Vec::new();
            while !done_t.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        accepts_t.fetch_add(1, Ordering::SeqCst);
                        on_accept(stream, &mut held);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        (addr, accepts, done)
    }

    /// A READ TIMEOUT is a stated failure and is NOT retried (D014-12 retries
    /// the transport failure, not the timeout). The fixture genuinely produces
    /// the condition: the server accepts and then never answers, so the
    /// client's read budget expires. One accept = one attempt; a second accept
    /// would mean the old behaviour (retry-on-any-error) is back, doubling a
    /// 3-minute wait into 6 in production.
    #[test]
    fn a_read_timeout_is_stated_and_not_retried() {
        let (addr, accepts, done) = counting_server(|stream, held| {
            // Hold the socket open and say nothing: the request is sent, no
            // byte of response ever arrives, and only the READ budget can end
            // the wait.
            held.push(stream);
        });

        let a = ModelAnswerer::with_timeouts(
            format!("http://{addr}"),
            "m",
            Duration::from_millis(500),
            Duration::from_millis(700),
        );
        let started = std::time::Instant::now();
        let out = a.answer_blocking("q");
        done.store(true, Ordering::SeqCst);

        assert!(
            out.starts_with("[ask failed: no answer"),
            "a timeout must be stated as one: {out}"
        );
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            1,
            "a timed-out request was RETRIED — that spends the whole read budget twice: {out}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout did not bound the wait: {:?}",
            started.elapsed()
        );
    }

    /// A DROPPED connection — the measured cold-load failure of D014-12 — is
    /// retried exactly once. The fixture produces the condition: the server
    /// accepts and immediately closes, which is a transport error, not a
    /// timeout. Two accepts = the original attempt plus its one retry.
    #[test]
    fn a_dropped_connection_is_retried_exactly_once() {
        let (addr, accepts, done) = counting_server(|stream, _held| {
            // Close immediately: the client sees the connection die with no
            // response — reqwest reports a transport error.
            drop(stream);
        });

        let a = ModelAnswerer::with_timeouts(
            format!("http://{addr}"),
            "m",
            Duration::from_millis(500),
            Duration::from_secs(5),
        );
        let out = a.answer_blocking("q");
        // The retry needs a moment to complete both attempts before we stop
        // counting; answer_blocking already joined, so both are done here.
        done.store(true, Ordering::SeqCst);

        assert!(out.starts_with("[ask failed"), "both attempts failed, stated: {out}");
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            2,
            "a transport error must be retried exactly once (D014-12): {out}"
        );
    }

    /// The retry is on TRANSPORT failure only. A model that answered — even
    /// with a rejection — has spoken, and asking again would double the cost of
    /// every refusal.
    #[test]
    fn the_endpoint_and_model_are_taken_from_the_environment() {
        let _env = test_env_lock();
        let mut cfg = DayflowConfig::default();
        cfg.perception.reason_model = "from-config".into();
        let prior_endpoint = std::env::var("GE_DAYFLOW_ENDPOINT").ok();
        let prior_model = std::env::var("GE_DAYFLOW_REASON_MODEL").ok();

        std::env::set_var("GE_DAYFLOW_ENDPOINT", "http://example.invalid:8799/llm/ollama");
        std::env::remove_var("GE_DAYFLOW_REASON_MODEL");
        let a = ModelAnswerer::from_env(&cfg).expect("configured");
        assert_eq!(a.model, "from-config", "the config supplies the model when env does not");

        std::env::set_var("GE_DAYFLOW_REASON_MODEL", "from-env");
        let b = ModelAnswerer::from_env(&cfg).expect("configured");
        assert_eq!(b.model, "from-env", "the environment overrides the config");
        assert_eq!(b.endpoint, "http://example.invalid:8799/llm/ollama");

        std::env::remove_var("GE_DAYFLOW_ENDPOINT");
        std::env::remove_var("GE_DAYFLOW_REASON_MODEL");
        let unconfigured = ModelAnswerer::from_env(&cfg);

        // Restore whatever the process started with before asserting, so a
        // failure here cannot leave the environment mangled for later tests.
        if let Some(v) = prior_endpoint {
            std::env::set_var("GE_DAYFLOW_ENDPOINT", v);
        }
        if let Some(v) = prior_model {
            std::env::set_var("GE_DAYFLOW_REASON_MODEL", v);
        }
        assert!(
            unconfigured.is_none(),
            "no endpoint means no answerer — an unconfigured install must still READ its timeline"
        );
    }
}
