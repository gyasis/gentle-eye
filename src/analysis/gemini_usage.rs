//! Gemini spend accounting, shared with the `gemini` CLI.
//!
//! Every Gemini generation call appends one JSON line to
//! `~/.local/share/gemini-cli/usage.jsonl`, in the shape the `gemini` CLI
//! writes, so one log answers "what did Gemini cost" for both tools. With
//! `GEMINI_BURST_ID` set, a call is refused once the spend logged under that
//! burst has reached the cap in `bursts/<id>.json`.
//!
//! The directory is a parameter of every function here, so the logic is tested
//! against a temp dir and never against the real log.

use crate::contracts::errors::VisionError;
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// USD per 1M tokens, standard tier; output rates also bill thinking tokens.
/// Source: https://aistudio.google.com/docs/pricing, the table the `gemini`
/// CLI carries in `PRICES`. `long` rates apply when the prompt exceeds
/// [`LONG_PROMPT_TOKENS`]. A model that is not listed has no price here.
struct Price {
    model: &'static str,
    input: f64,
    output: f64,
    long: Option<(f64, f64)>,
}

const PRICES: &[Price] = &[
    Price { model: "gemini-3-flash-preview", input: 0.50, output: 3.00, long: None },
    Price { model: "gemini-3.1-flash-lite", input: 0.25, output: 1.50, long: None },
    Price { model: "gemini-3.5-flash-lite", input: 0.30, output: 2.50, long: None },
    Price { model: "gemini-3.6-flash", input: 0.75, output: 3.75, long: None },
    Price { model: "gemini-3.8-flash", input: 0.75, output: 3.75, long: None },
    Price { model: "gemini-3.1-pro-preview", input: 2.00, output: 12.00, long: Some((4.00, 18.00)) },
];

const LONG_PROMPT_TOKENS: u64 = 200_000;

/// Token counts of one call, from the response's `usageMetadata`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub thinking: u64,
}

/// One line of `usage.jsonl`. Field names and order match the `gemini` CLI.
#[derive(Debug, Clone, Serialize)]
pub struct UsageRecord {
    pub ts: String,
    pub cmd: String,
    pub cwd: String,
    pub model: String,
    #[serde(rename = "in")]
    pub input: u64,
    pub out: u64,
    pub thinking: u64,
    pub usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst: Option<String>,
    pub tool: &'static str,
    /// The concrete model the API reports it served, when it differs from the
    /// requested name (an alias such as `gemini-flash-latest`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
}

/// The directory the `gemini` CLI keeps its usage log and bursts in.
pub fn default_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().join(".local/share/gemini-cli"))
}

/// The burst id from `GEMINI_BURST_ID`, if one is set.
pub fn burst_id_from_env() -> Option<String> {
    std::env::var("GEMINI_BURST_ID").ok().filter(|b| !b.trim().is_empty())
}

fn usage_log(dir: &Path) -> PathBuf {
    dir.join("usage.jsonl")
}

/// Cost of one call from its token counts, or `None` when `model` has no
/// listed price.
pub fn price_usd(model: &str, usage: TokenUsage) -> Option<f64> {
    let p = PRICES.iter().find(|p| p.model == model)?;
    let (pin, pout) = match p.long {
        Some(long) if usage.input > LONG_PROMPT_TOKENS => long,
        _ => (p.input, p.output),
    };
    let usd = (usage.input as f64 * pin + (usage.output + usage.thinking) as f64 * pout) / 1e6;
    Some((usd * 1e6).round() / 1e6)
}

/// Token counts and the served model version from a `generateContent`
/// response body. `None` when the body carries no `usageMetadata`.
pub fn parse_usage(body: &str) -> Option<(TokenUsage, Option<String>)> {
    let v: Value = serde_json::from_str(body).ok()?;
    let u = v.get("usageMetadata")?;
    let count = |key: &str| u.get(key).and_then(Value::as_u64).unwrap_or(0);
    let usage = TokenUsage {
        input: count("promptTokenCount"),
        output: count("candidatesTokenCount"),
        thinking: count("thoughtsTokenCount"),
    };
    let version = v.get("modelVersion").and_then(Value::as_str).map(str::to_string);
    Some((usage, version))
}

/// Build the log line for one call. The price comes from the requested model
/// name, else from the model version the API reports; with neither listed the
/// cost is `None`.
pub fn usage_record(
    cmd: &str,
    model: &str,
    usage: TokenUsage,
    model_version: Option<&str>,
    burst: Option<&str>,
) -> UsageRecord {
    let usd = price_usd(model, usage)
        .or_else(|| model_version.and_then(|served| price_usd(served, usage)));
    UsageRecord {
        ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string(),
        cmd: cmd.to_string(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        model: model.to_string(),
        input: usage.input,
        out: usage.output,
        thinking: usage.thinking,
        usd,
        burst: burst.map(str::to_string),
        tool: "gentle-eye",
        model_version: model_version.filter(|served| *served != model).map(str::to_string),
    }
}

/// Append one record to `<dir>/usage.jsonl`.
pub fn append_usage(dir: &Path, record: &UsageRecord) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(usage_log(dir))?
        .write_all(line.as_bytes())
}

/// USD already logged under one burst id. A call without a price counts as 0.
pub fn burst_spent(dir: &Path, burst_id: &str) -> f64 {
    let Ok(file) = std::fs::File::open(usage_log(dir)) else {
        return 0.0;
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.contains(burst_id))
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter(|r| r.get("burst").and_then(Value::as_str) == Some(burst_id))
        .map(|r| r.get("usd").and_then(Value::as_f64).unwrap_or(0.0))
        .sum()
}

/// With a burst id, refuse the call once that burst's logged spend has
/// reached its cap. Without one, every call passes.
pub fn burst_guard(dir: &Path, burst_id: Option<&str>) -> Result<(), VisionError> {
    let Some(bid) = burst_id else {
        return Ok(());
    };
    let meta = dir.join("bursts").join(format!("{bid}.json"));
    let cap = std::fs::read_to_string(&meta)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.get("cap_usd").and_then(Value::as_f64))
        .ok_or_else(|| {
            VisionError::SpendCapReached(format!(
                "GEMINI_BURST_ID={bid} has no readable cap_usd in {}; start the burst with \
                 `gemini-burst start` first.",
                meta.display()
            ))
        })?;
    let spent = burst_spent(dir, bid);
    if spent >= cap {
        return Err(VisionError::SpendCapReached(format!(
            "burst {bid} reached its cap: {spent:.4} of {cap} USD spent. Stop, or start a new \
             burst with a higher cap."
        )));
    }
    Ok(())
}

/// Log one finished `generateContent` call from its response body. Logging
/// never fails the call: a problem is reported on stderr.
pub fn log_call(dir: &Path, cmd: &str, model: &str, body: &str, burst: Option<&str>) {
    let Some((usage, served)) = parse_usage(body) else {
        eprintln!("[gentle-eye] gemini usage not logged: the response carried no usageMetadata");
        return;
    };
    let record = usage_record(cmd, model, usage, served.as_deref(), burst);
    if record.usd.is_none() {
        eprintln!(
            "[gentle-eye] no price listed for gemini model '{model}': tokens logged with usd null \
             (in={} out={} thinking={})",
            usage.input, usage.output, usage.thinking
        );
    }
    if let Err(e) = append_usage(dir, &record) {
        eprintln!("[gentle-eye] gemini usage log not written: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USAGE: TokenUsage = TokenUsage { input: 1684, output: 896, thinking: 1369 };

    fn start_burst(dir: &Path, id: &str, cap: f64) {
        std::fs::create_dir_all(dir.join("bursts")).unwrap();
        std::fs::write(
            dir.join("bursts").join(format!("{id}.json")),
            format!(r#"{{"cap_usd": {cap}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn prices_a_listed_model_and_bills_thinking_as_output() {
        // (1684 * 0.75 + (896 + 1369) * 3.75) / 1e6
        assert_eq!(price_usd("gemini-3.8-flash", USAGE), Some(0.009757));
    }

    #[test]
    fn long_prompts_use_the_long_rates_only_where_listed() {
        let long = TokenUsage { input: 300_000, output: 1000, thinking: 0 };
        assert_eq!(price_usd("gemini-3.1-pro-preview", long), Some(1.218));
        assert_eq!(price_usd("gemini-3.8-flash", long), Some(0.22875));
    }

    #[test]
    fn an_unlisted_model_has_no_price() {
        assert_eq!(price_usd("gemini-flash-latest", USAGE), None);
        let record = usage_record("analyze image", "gemini-flash-latest", USAGE, None, None);
        assert_eq!(record.usd, None);
        assert_eq!(serde_json::to_value(&record).unwrap()["usd"], Value::Null);
    }

    #[test]
    fn an_alias_is_priced_from_the_served_model_version() {
        let record = usage_record(
            "analyze image",
            "gemini-flash-latest",
            USAGE,
            Some("gemini-3.8-flash"),
            None,
        );
        assert_eq!(record.usd, Some(0.009757));
        assert_eq!(record.model, "gemini-flash-latest");
        assert_eq!(record.model_version.as_deref(), Some("gemini-3.8-flash"));
    }

    #[test]
    fn parses_usage_metadata_and_model_version() {
        let body = r#"{
            "candidates":[{"content":{"parts":[{"text":"x"}]}}],
            "usageMetadata":{"promptTokenCount":1684,"candidatesTokenCount":896,
                             "thoughtsTokenCount":1369,"totalTokenCount":3949},
            "modelVersion":"gemini-3.8-flash"
        }"#;
        let (usage, served) = parse_usage(body).unwrap();
        assert_eq!(usage, USAGE);
        assert_eq!(served.as_deref(), Some("gemini-3.8-flash"));
        assert!(parse_usage(r#"{"candidates":[]}"#).is_none());
    }

    #[test]
    fn the_usage_line_has_the_gemini_cli_shape() {
        let dir = tempfile::tempdir().unwrap();
        let record = usage_record("analyze image", "gemini-3.8-flash", USAGE, None, Some("b1"));
        append_usage(dir.path(), &record).unwrap();

        let raw = std::fs::read_to_string(dir.path().join("usage.jsonl")).unwrap();
        assert!(raw.ends_with('\n'));
        assert_eq!(raw.lines().count(), 1);
        let line: Value = serde_json::from_str(raw.trim_end()).unwrap();
        let keys: Vec<&str> = line.as_object().unwrap().keys().map(String::as_str).collect();
        for key in ["ts", "cmd", "cwd", "model", "in", "out", "thinking", "usd", "burst", "tool"] {
            assert!(keys.contains(&key), "missing {key} in {raw}");
        }
        assert_eq!(line["cmd"], "analyze image");
        assert_eq!(line["model"], "gemini-3.8-flash");
        assert_eq!(line["in"], 1684);
        assert_eq!(line["out"], 896);
        assert_eq!(line["thinking"], 1369);
        assert_eq!(line["usd"], 0.009757);
        assert_eq!(line["burst"], "b1");
        assert_eq!(line["tool"], "gentle-eye");
        assert!(line["ts"].as_str().unwrap().ends_with("+00:00"));
        assert!(line.get("model_version").is_none());
    }

    #[test]
    fn a_line_without_a_burst_omits_the_field() {
        let record = usage_record("analyze video", "gemini-3.8-flash", USAGE, None, None);
        assert!(serde_json::to_value(&record).unwrap().get("burst").is_none());
    }

    #[test]
    fn burst_spend_sums_only_that_burst() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("usage.jsonl"),
            concat!(
                r#"{"model":"a","usd":0.25,"burst":"b1"}"#, "\n",
                r#"{"model":"a","usd":0.5,"burst":"b2"}"#, "\n",
                r#"{"model":"a","usd":null,"burst":"b1"}"#, "\n",
                "not json b1\n",
                r#"{"model":"b1","usd":9.0}"#, "\n",
                r#"{"model":"a","usd":0.125,"burst":"b1"}"#, "\n",
            ),
        )
        .unwrap();
        assert_eq!(burst_spent(dir.path(), "b1"), 0.375);
        assert_eq!(burst_spent(dir.path(), "absent"), 0.0);
    }

    #[test]
    fn no_burst_id_means_no_guard() {
        let dir = tempfile::tempdir().unwrap();
        assert!(burst_guard(dir.path(), None).is_ok());
    }

    #[test]
    fn a_burst_under_its_cap_passes() {
        let dir = tempfile::tempdir().unwrap();
        start_burst(dir.path(), "b1", 1.0);
        let record = usage_record("analyze image", "gemini-3.8-flash", USAGE, None, Some("b1"));
        append_usage(dir.path(), &record).unwrap();
        assert!(burst_guard(dir.path(), Some("b1")).is_ok());
    }

    #[test]
    fn a_burst_at_its_cap_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        start_burst(dir.path(), "b1", 0.5);
        std::fs::write(
            dir.path().join("usage.jsonl"),
            concat!(
                r#"{"usd":0.25,"burst":"b1"}"#, "\n",
                r#"{"usd":0.25,"burst":"b1"}"#, "\n",
            ),
        )
        .unwrap();
        let err = burst_guard(dir.path(), Some("b1")).unwrap_err();
        assert!(matches!(err, VisionError::SpendCapReached(_)));
        let msg = err.to_string();
        assert!(msg.contains("b1") && msg.contains("0.5000") && msg.contains("cap"), "{msg}");
    }

    #[test]
    fn a_burst_with_no_cap_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = burst_guard(dir.path(), Some("unstarted")).unwrap_err();
        assert!(matches!(err, VisionError::SpendCapReached(_)));
        assert!(err.to_string().contains("gemini-burst start"));
    }

    #[test]
    fn log_call_writes_one_line_per_response() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"{"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5}}"#;
        log_call(dir.path(), "analyze image", "gemini-3.8-flash", body, Some("b1"));
        log_call(dir.path(), "analyze image", "gemini-3.8-flash", "{}", Some("b1"));
        let raw = std::fs::read_to_string(dir.path().join("usage.jsonl")).unwrap();
        assert_eq!(raw.lines().count(), 1);
        let line: Value = serde_json::from_str(raw.trim_end()).unwrap();
        assert_eq!(line["thinking"], 0);
        assert_eq!(line["burst"], "b1");
    }
}
