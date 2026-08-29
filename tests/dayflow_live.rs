//! T051 — LIVE end-to-end validation of Dayflow (`#[ignore]`d, never in CI).
//!
//! A real capture session across every attached display, through real sample
//! PNGs, the real two-tier perception ladder (text tier + reasoning tier over
//! the Atelier governor's ollama lane), a real on-disk SQLite timeline — ending
//! in a grounded answer to "what was I doing at \<the time it just recorded\>?".
//! This is the check a green `cargo test` cannot give: every stage below talks
//! to the actual screen, the actual models and the actual database.
//!
//! # How to run (by hand)
//!
//! ```text
//! GE_DAYFLOW_ENDPOINT=http://<governor-host>:8799/llm/ollama \
//!   ./.tooling/bin/cargo test --test dayflow_live -- --ignored --nocapture
//! ```
//!
//! `<governor-host>` is the Atelier governor machine (the LAN address is
//! deliberately not written into this public repo — it lives in `~/dev/.env`).
//! Optional overrides:
//!
//! | var | default | source |
//! |---|---|---|
//! | `GE_DAYFLOW_TEXT_MODEL` | `deepseek-ocr:latest` | research R27 (the measured text tier) |
//! | `GE_DAYFLOW_REASON_MODEL` | `ornith-1.5-9b:latest` | research R5 (the reasoning tier) |
//!
//! # Failure policy — loud, specific, never silently green
//!
//! An `#[ignore]`d test that quietly passes when its environment is missing
//! certifies nothing while looking like it certified everything. So every
//! missing precondition PANICS naming exactly what to set or fix:
//! no `GE_DAYFLOW_ENDPOINT`, an unreachable governor, a model that is not
//! pulled, no capturable display — each is a distinct, named failure.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use gentle_eye::analysis::OllamaProvider;
use gentle_eye::config::{DayflowConfig, DeltaConfig};
use gentle_eye::contracts::traits::VisionConfig;
use gentle_eye::dayflow::models::{ChunkRef, DayflowMode, RollingContext};
use gentle_eye::dayflow::perception::PerceptionRouter;
use gentle_eye::dayflow::sampler::{RawFrame, Sampler};
use gentle_eye::dayflow::scheduler::entry_from;
use gentle_eye::dayflow::source::display::tightly_packed;
use gentle_eye::dayflow::service::DayflowService;
use gentle_eye::dayflow::summarizer::{ChunkSummarizer, RoutedChunkSummarizer};
use gentle_eye::dayflow::timeline::SqliteTimelineStore;

/// The endpoint, or a panic that says exactly what to export.
fn require_endpoint() -> String {
    std::env::var("GE_DAYFLOW_ENDPOINT").unwrap_or_else(|_| {
        panic!(
            "\n\nGE_DAYFLOW_ENDPOINT is not set — this live test cannot run.\n\
             Set it to the Atelier governor's ollama lane, e.g.\n\
             \n    GE_DAYFLOW_ENDPOINT=http://<governor-host>:8799/llm/ollama \\\n\
             \n      ./.tooling/bin/cargo test --test dayflow_live -- --ignored --nocapture\n\n\
             (The host lives in ~/dev/.env; it is deliberately not hardcoded here.)\n"
        )
    })
}

fn text_model() -> String {
    std::env::var("GE_DAYFLOW_TEXT_MODEL").unwrap_or_else(|_| "deepseek-ocr:latest".into())
}

fn reason_model() -> String {
    std::env::var("GE_DAYFLOW_REASON_MODEL").unwrap_or_else(|_| "ornith-1.5-9b:latest".into())
}

/// Verify the governor lane answers and both tiers' models are pulled.
///
/// Fails LOUDLY per missing piece: an unreachable endpoint and an unpulled
/// model are different problems with different fixes, and a combined "setup
/// failed" would send whoever runs this to the wrong one.
async fn preflight(endpoint: &str, text: &str, reason: &str) {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("http client");
    let tags: serde_json::Value = client
        .get(format!("{}/api/tags", endpoint.trim_end_matches('/')))
        .send()
        .await
        .unwrap_or_else(|e| {
            panic!(
                "\n\nThe governor lane at {endpoint} is unreachable: {e}\n\
                 Check GE_DAYFLOW_ENDPOINT, that the governor is up, and that this\n\
                 machine can reach it.\n"
            )
        })
        .json()
        .await
        .unwrap_or_else(|e| panic!("{endpoint}/api/tags did not return JSON: {e}"));
    let names: Vec<String> = tags["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for model in [text, reason] {
        assert!(
            names.iter().any(|n| n == model),
            "\n\nModel '{model}' is not available on {endpoint}.\n\
             Pull it on the governor host first:  ollama pull {model}\n\
             (models present: {names:?})\n"
        );
    }
}

/// Strip a reasoning model's `<think>…</think>` preamble, if present.
fn strip_think(text: &str) -> String {
    match text.rfind("</think>") {
        Some(i) => text[i + "</think>".len()..].trim().to_string(),
        None => text.trim().to_string(),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: needs GE_DAYFLOW_ENDPOINT (Atelier governor), pulled models, and a real display"]
async fn a_real_session_flows_from_pixels_to_a_grounded_answer() {
    // ── 0. Environment, checked loudly ────────────────────────────────────
    let endpoint = require_endpoint();
    let (text_m, reason_m) = (text_model(), reason_model());
    preflight(&endpoint, &text_m, &reason_m).await;

    let displays = gentle_eye::capture::display::DisplayManager::list_available()
        .unwrap_or_else(|e| {
            panic!("\n\nNo capturable display: {e}\nRun this on a machine with a real screen (not a bare headless box).\n")
        });
    assert!(!displays.is_empty(), "display enumeration returned an empty list");
    println!(
        "[live] {} display(s): {:?}",
        displays.len(),
        displays.iter().map(|d| (d.index, d.width, d.height)).collect::<Vec<_>>()
    );
    if displays.len() < 2 {
        println!(
            "[live] NOTE: only one display attached — the multi-display union \
             path runs, but with a single member per instant."
        );
    }

    // ── 1. A real session over every display, real frames, real samples ──
    let sample_dir = tempfile::tempdir().expect("sample dir");
    let db_dir = tempfile::tempdir().expect("db dir");
    let conn = gentle_eye::storage::database::init_database(&db_dir.path().join("timeline.db"))
        .expect("open timeline db");
    let store = Arc::new(SqliteTimelineStore::new(Arc::new(Mutex::new(conn))));
    let svc = DayflowService::new(store, DayflowConfig::default());

    let display_ids: Vec<u32> = displays.iter().map(|d| d.index as u32).collect();
    let t_start = Utc::now();
    let session_id = svc
        .start(DayflowMode::Session, display_ids.clone(), t_start)
        .expect("start session");
    println!("[live] session {session_id} started over displays {display_ids:?}");

    let mut sampler = Sampler::new(DeltaConfig::default());
    let mut capturers: Vec<gentle_eye::capture::screen::ScreenCapturer> = display_ids
        .iter()
        .map(|&id| {
            gentle_eye::capture::screen::ScreenCapturer::new(id as usize).unwrap_or_else(|e| {
                panic!("\n\nCannot open display {id} for capture: {e}\n(Wayland boxes need the X11 backend; check $DISPLAY.)\n")
            })
        })
        .collect();

    const ROUNDS: usize = 3;
    for round in 0..ROUNDS {
        for (i, &display_id) in display_ids.iter().enumerate() {
            let cap = &mut capturers[i];
            let (w, h) = (cap.width(), cap.height());
            let raw = cap
                .capture_frame(std::time::Duration::from_secs(2))
                .unwrap_or_else(|e| panic!("capture_frame failed on display {display_id}: {e}"));
            let packed = tightly_packed(&raw, w, h);
            let now = Utc::now();
            // The first window on each display carries sequence 0 — asserted
            // against the engine's own record at stop, below.
            let rec = sampler
                .observe(
                    display_id,
                    0,
                    RawFrame { bgra: &packed, width: w as u32, height: h as u32 },
                    now,
                    sample_dir.path(),
                )
                .expect("sampler observe");
            svc.with_run(|r| r.on_sample(display_id, now)).expect("on_sample");
            println!(
                "[live] round {round} display {display_id}: {:?} kept={} ({}x{})",
                rec.verdict,
                rec.path.is_some(),
                w,
                h
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    }

    // ── 2. Stop; the engine hands back the real windows it closed ────────
    let t_end = Utc::now();
    let closed = svc.stop(t_end).expect("stop session");
    assert_eq!(
        closed.len(),
        display_ids.len(),
        "one closed window per display — got {closed:?}"
    );
    for w in &closed {
        assert_eq!(w.sequence, 0, "first window per display is sequence 0: {w:?}");
        assert!(
            w.sample_count >= 1,
            "display {} recorded no samples at all",
            w.display_id
        );
    }

    // Every display must have at least ONE sample PNG on disk (later rounds may
    // be delta-gated away on a static screen — that is the gate working).
    for &id in &display_ids {
        let prefix = gentle_eye::dayflow::sampler::sample_prefix(id, 0);
        let n = std::fs::read_dir(sample_dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .count();
        assert!(n >= 1, "no sample PNG written for display {id}");
        println!("[live] display {id}: {n} sample file(s) on disk");
    }

    // ── 3. The real perception ladder over the real samples ──────────────
    let mk = |model: &str| -> Arc<OllamaProvider> {
        Arc::new(
            OllamaProvider::with_url(
                &VisionConfig {
                    provider: "ollama".into(),
                    api_key: None,
                    model: model.to_string(),
                    timeout_seconds: 180,
                    max_video_size_bytes: 0,
                },
                endpoint.clone(),
            )
            .expect("build provider"),
        )
    };
    let router = Arc::new(PerceptionRouter::new(mk(&text_m), mk(&reason_m), 1_000));
    let summarizer = RoutedChunkSummarizer::new(router.clone(), sample_dir.path());

    let recording_id = uuid::Uuid::new_v4();
    let mut context = RollingContext::default();
    for (i, window) in closed.iter().enumerate() {
        let chunk = ChunkRef {
            index: i,
            path: PathBuf::from("unused-live.mp4"),
            start_wall: window.start_wall,
            end_wall: window.end_wall,
            display_id: window.display_id,
            sequence: window.sequence,
            summarized: false,
        };
        let summary = summarizer
            .summarize_chunk(&chunk, &context)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "\n\nPerception failed for display {}'s window: {e}\n\
                     (Both tiers must be reachable through {endpoint}.)\n",
                    window.display_id
                )
            });
        println!(
            "[live] display {} summarised: category={:?} app={:?} activity={:?}",
            window.display_id, summary.category, summary.app, summary.activity
        );
        context = gentle_eye::dayflow::summarizer::advance_context(&context, &summary);
        svc.insert_entry(&entry_from(recording_id, window, &summary))
            .expect("insert entry");
    }

    // Ladder accounting: each segment made at least one text call plus the one
    // reasoning call, and each segment escalated exactly once.
    let latencies = summarizer.latencies();
    assert_eq!(latencies.len(), closed.len());
    for l in &latencies {
        assert!(
            l.perception_calls >= 2,
            "a segment must pay at least one text call and one reasoning call: {l:?}"
        );
        println!(
            "[live] segment: {} samples, {} calls, first {:?}, total {:?}, cold_load={}",
            l.samples,
            l.perception_calls,
            l.first_call,
            l.total,
            l.paid_a_cold_load()
        );
    }
    assert_eq!(
        router.escalations().len(),
        closed.len(),
        "exactly one reasoning escalation per segment"
    );

    // ── 4. The question the feature exists to answer ──────────────────────
    let (from, to) = (
        t_start - chrono::Duration::minutes(1),
        t_end + chrono::Duration::minutes(1),
    );
    let slice = svc.timeline(from, to).expect("timeline");
    assert_eq!(
        slice.entries.len(),
        closed.len(),
        "one timeline entry per closed window"
    );

    // "What was I doing at 2pm?" — asked about the wall-clock minute this test
    // just recorded, so the grounded answer is checkable against reality by
    // the person running it.
    let asked_at = t_start + (t_end - t_start) / 2;
    let question = format!(
        "What was I doing at {}?",
        asked_at.with_timezone(&chrono::Local).format("%H:%M")
    );
    println!("[live] asking: {question}");

    let handle = tokio::runtime::Handle::current();
    let ep = endpoint.clone();
    let rm = reason_m.clone();
    let answer = svc
        .ask(&question, from, to, move |prompt| {
            tokio::task::block_in_place(|| {
                handle.block_on(async move {
                    let client = reqwest::Client::builder()
                        .connect_timeout(std::time::Duration::from_secs(3))
                        .timeout(std::time::Duration::from_secs(180))
                        .build()
                        .expect("http client");
                    let v: serde_json::Value = client
                        .post(format!("{}/api/generate", ep.trim_end_matches('/')))
                        .json(&serde_json::json!({
                            "model": rm,
                            "prompt": prompt,
                            "stream": false,
                        }))
                        .send()
                        .await
                        .unwrap_or_else(|e| panic!("ask_day generate call failed: {e}"))
                        .json()
                        .await
                        .unwrap_or_else(|e| panic!("ask_day response was not JSON: {e}"));
                    strip_think(v["response"].as_str().unwrap_or_default())
                })
            })
        })
        .expect("ask");

    assert!(
        answer.is_grounded(),
        "the answer must rest on recorded entries, never on the model's imagination"
    );
    assert!(!answer.answer.trim().is_empty(), "the model returned an empty answer");
    println!("\n[live] GROUNDED ON {} ENTRIES:", answer.grounding.len());
    for e in &answer.grounding {
        println!(
            "[live]   {} - {}  [{:?}] {} — {}",
            e.start_time.with_timezone(&chrono::Local).format("%H:%M:%S"),
            e.end_time.with_timezone(&chrono::Local).format("%H:%M:%S"),
            e.category,
            e.activity,
            e.summary
        );
    }
    println!("\n[live] ANSWER:\n{}\n", answer.answer.trim());
}
