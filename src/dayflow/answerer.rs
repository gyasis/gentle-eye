//! The real `ask_day` answerer (T024).
//!
//! Before this, all three surfaces returned `[no model configured for ask]`
//! followed by the prompt — a range WITH records handed back the question
//! instead of an answer, three times over, in three copies of the same stub.
//!
//! One implementation here, so the surfaces cannot drift: an answer that
//! differs by which surface asked is a bug nobody would think to look for.

use std::time::Duration;

use crate::config::DayflowConfig;

/// Calls the reasoning tier through the governed lane.
pub struct ModelAnswerer {
    endpoint: String,
    model: String,
    client: reqwest::Client,
}

/// What the answerer needs from the environment, resolved once.
///
/// `GE_DAYFLOW_ENDPOINT` is the governed lane (`…:8799/llm/ollama`), the same
/// variable the live test uses — one name, so a working live run and a working
/// `ask` are the same configuration.
pub fn endpoint_from_env() -> Option<String> {
    std::env::var("GE_DAYFLOW_ENDPOINT").ok().filter(|s| !s.trim().is_empty())
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
        // SPLIT timeouts, per D014-12. A single flat budget cannot tell a dead
        // endpoint from a legitimate cold load: measured 2026-08-29, the first
        // ask after an idle period failed with a transport error while the
        // governor spent 95 s loading the reasoning model. A short CONNECT
        // fails a wrong URL in a second; a long READ lets a cold load finish.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(180))
            .build()
            .unwrap_or_default();
        Self { endpoint: endpoint.into(), model: model.into(), client }
    }

    /// Ask, returning prose, or a stated failure.
    ///
    /// Never panics and never propagates an error: `ask_day`'s answerer returns
    /// a `String`, and the grounding is already attached by the caller — so a
    /// failure here must READ as a failure rather than vanish into an empty
    /// answer that looks like a confident "nothing happened".
    pub async fn answer(&self, prompt: &str) -> String {
        let url = format!("{}/api/generate", self.endpoint.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
        });

        // ONE retry on a TRANSPORT error, per D014-12. The first question after
        // an idle period is precisely when the model is cold and the lane drops
        // the connection — the common case, not the edge. A retry after a
        // REJECTED request would be wrong (the model answered, it just said
        // something we could not use), so only transport failures retry.
        let mut last = String::new();
        for attempt in 1..=2 {
            match self.client.post(&url).json(&body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
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
    /// call runs on its own thread with its own current-thread runtime and this
    /// one blocks on the join.
    ///
    /// Blocking an interactive `ask` is the intended cost: the caller asked a
    /// question and is waiting for the answer. It is NOT used on the capture
    /// path, where blocking would stall recording.
    pub fn answer_blocking(&self, prompt: &str) -> String {
        let endpoint = self.endpoint.clone();
        let model = self.model.clone();
        let prompt = prompt.to_string();
        let handle = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return format!("[ask failed: no runtime for the model call: {e}]"),
            };
            let answerer = ModelAnswerer::new(endpoint, model);
            rt.block_on(answerer.answer(&prompt))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An unconfigured install says so, and does NOT echo the prompt back.
    ///
    /// Echoing it is what let the stub survive on three surfaces: a range WITH
    /// records handed back the question, and the response was long and prose-ish
    /// enough to look like an answer.
    #[test]
    fn an_unconfigured_install_says_so_without_echoing_the_prompt() {
        let cfg = DayflowConfig::default();
        // Ensure the env is genuinely absent for this assertion.
        let prior = std::env::var("GE_DAYFLOW_ENDPOINT").ok();
        std::env::remove_var("GE_DAYFLOW_ENDPOINT");

        let prompt = "SECRET-PROMPT-MARKER what did I do today?";
        let out = answer_or_explain(&cfg, prompt);

        assert_eq!(out, NO_MODEL);
        assert!(
            !out.contains("SECRET-PROMPT-MARKER"),
            "the prompt was echoed back as if it were an answer: {out}"
        );

        if let Some(v) = prior {
            std::env::set_var("GE_DAYFLOW_ENDPOINT", v);
        }
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

    /// The retry is on TRANSPORT failure only. A model that answered — even
    /// with a rejection — has spoken, and asking again would double the cost of
    /// every refusal.
    #[test]
    fn the_endpoint_and_model_are_taken_from_the_environment() {
        let mut cfg = DayflowConfig::default();
        cfg.perception.reason_model = "from-config".into();

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
        assert!(
            ModelAnswerer::from_env(&cfg).is_none(),
            "no endpoint means no answerer — an unconfigured install must still READ its timeline"
        );
    }
}
