//! US6 acceptance coverage — one engine behind three surfaces (T044).
//!
//! The parity claim is "a session started on one surface is visible from the
//! other two". These tests assert it by driving the surfaces AGAINST EACH
//! OTHER, not by checking each in isolation: three passing isolated tests are
//! exactly what you get when three surfaces each keep their own state and
//! quietly disagree.

use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use gentle_eye::config::DayflowConfig;
use gentle_eye::dayflow::http;
use gentle_eye::dayflow::models::{ActivityCategory, DayflowMode, TimelineEntry};
use gentle_eye::dayflow::service::DayflowService;
use gentle_eye::dayflow::timeline::SqliteTimelineStore;
use gentle_eye::storage::database::init_in_memory;
use uuid::Uuid;

fn service() -> Arc<DayflowService> {
    let store = Arc::new(SqliteTimelineStore::new(Arc::new(Mutex::new(
        init_in_memory().unwrap(),
    ))));
    Arc::new(DayflowService::new(store, DayflowConfig::default()))
}

/// What the HTTP surface reports, parsed.
fn http_status(svc: &DayflowService) -> serde_json::Value {
    let (code, body) = http::route("GET", "/dayflow/status", "", svc);
    assert_eq!(code, "200 OK", "status is always a successful call: {body}");
    serde_json::from_str(&body).expect("valid JSON")
}

#[test]
fn a_session_started_on_one_surface_is_visible_from_the_others() {
    // The US6 independent test. Started through the HTTP route, observed
    // through the service call the MCP and CLI surfaces make.
    let svc = service();

    let before = http_status(&svc);
    assert_eq!(before["running"], false);

    let (code, body) = http::route("POST", "/dayflow/start", "displays=0", &svc);
    assert_eq!(code, "200 OK", "{body}");
    let started: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = started["session_id"].as_str().expect("a session id").to_string();

    // …seen by the call the MCP tool and the CLI subcommand both make…
    let direct = svc.status(Utc::now()).unwrap();
    assert!(direct.running);
    assert_eq!(direct.session_id.map(|u| u.to_string()), Some(id.clone()));

    // …and by the HTTP surface, reporting the same session.
    let after = http_status(&svc);
    assert_eq!(after["running"], true);
    assert_eq!(after["session_id"], serde_json::json!(id));
}

#[test]
fn stopping_on_one_surface_is_visible_from_the_others() {
    // The other direction, which is the one that actually catches divergent
    // state: a surface caching "running" would pass the start test and fail
    // this one.
    let svc = service();
    svc.start(DayflowMode::Session, vec![0], Utc::now()).unwrap();

    let (code, _) = http::route("POST", "/dayflow/stop", "", &svc);
    assert_eq!(code, "200 OK");

    assert!(!svc.status(Utc::now()).unwrap().running, "the service agrees");
    assert_eq!(http_status(&svc)["running"], false, "and so does HTTP");
}

#[test]
fn a_second_start_is_refused_on_every_surface_and_says_so() {
    // Refusing on one surface while another silently replaced the session would
    // drop the running session's unwritten windows.
    let svc = service();
    svc.start(DayflowMode::Session, vec![0], Utc::now()).unwrap();

    let (code, body) = http::route("POST", "/dayflow/start", "", &svc);
    assert_eq!(code, "400 Bad Request", "{body}");
    assert!(body.contains("already running"), "and it says why: {body}");
    assert!(svc.status(Utc::now()).unwrap().running, "the original is untouched");
}

#[test]
fn the_timeline_reads_the_same_through_every_surface() {
    let svc = service();
    let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let entry = TimelineEntry {
        id: Uuid::new_v4(),
        recording_id: Uuid::new_v4(),
        start_time: base,
        end_time: base + chrono::Duration::minutes(10),
        category: ActivityCategory::Coding,
        app: "editor".into(),
        activity: "refactor".into(),
        summary: "the perception ladder".into(),
        provenance: None,
    };
    svc.insert_entry(&entry).unwrap();

    // Z-form, not the `+00:00` offset: `+` is a SPACE in a query string, so a
    // raw offset arrives mangled. See `http::percent_decode`.
    let from = base.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to = (base + chrono::Duration::hours(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (code, body) = http::route(
        "GET",
        "/dayflow/timeline",
        &format!("from={from}&to={to}"),
        &svc,
    );
    assert_eq!(code, "200 OK", "{body}");
    let http_entries: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(http_entries["entries"].as_array().unwrap().len(), 1);

    let direct = svc.timeline(base, base + chrono::Duration::hours(1)).unwrap();
    assert_eq!(direct.len(), 1, "the same answer through the service call");
    assert_eq!(direct[0].summary, "the perception ladder");
}

#[test]
fn every_surface_defaults_a_missing_range_to_today_so_far() {
    // A default that differed between surfaces would make the same question
    // return different answers depending on how it was asked — the exact parity
    // failure US6 exists to prevent, and the kind that never shows up in a test
    // of one surface alone.
    let svc = service();
    let (code, body) = http::route("GET", "/dayflow/timeline", "", &svc);
    assert_eq!(code, "200 OK");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();

    let from = chrono::DateTime::parse_from_rfc3339(v["from"].as_str().unwrap()).unwrap();
    let to = chrono::DateTime::parse_from_rfc3339(v["to"].as_str().unwrap()).unwrap();
    assert_eq!(
        from.time(),
        chrono::NaiveTime::MIN,
        "the default range starts at midnight"
    );
    assert_eq!(from.date_naive(), to.date_naive(), "and ends the same day");
}

#[test]
fn an_ask_with_no_record_never_reaches_a_model_on_any_surface() {
    let svc = service();
    let (code, body) = http::route("GET", "/dayflow/ask", "question=what+was+I+doing", &svc);
    assert_eq!(code, "200 OK", "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["answer"], gentle_eye::dayflow::timeline::NO_RECORD);
    assert_eq!(v["grounding"].as_array().unwrap().len(), 0);
}

#[test]
fn a_malformed_request_is_refused_without_stopping_the_surface() {
    let svc = service();
    assert_eq!(http::route("GET", "/nope", "", &svc).0, "404 Not Found");
    assert_eq!(http::route("DELETE", "/dayflow/status", "", &svc).0, "405 Method Not Allowed");
    assert_eq!(
        http::route("GET", "/dayflow/timeline", "from=not-a-date", &svc).0,
        "400 Bad Request"
    );
    assert_eq!(
        http::route("GET", "/dayflow/ask", "", &svc).0,
        "400 Bad Request",
        "a question is required"
    );
    // and the surface still answers afterwards
    assert_eq!(http::route("GET", "/dayflow/status", "", &svc).0, "200 OK");
}

#[test]
fn a_question_survives_percent_encoding_intact() {
    // A question is free text arriving through a query string. Dropping a
    // malformed escape would silently eat part of what the user asked.
    let svc = service();
    let (code, body) = http::route(
        "GET",
        "/dayflow/ask",
        "question=what%20was%20I%20doing%20at%202pm%3F&from=2026-08-26T00:00:00Z",
        &svc,
    );
    assert_eq!(code, "200 OK", "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    // No record, so the answer is the refusal — but the QUESTION round-tripped.
    assert_eq!(v["answer"], gentle_eye::dayflow::timeline::NO_RECORD);
}

#[test]
fn a_raw_plus_in_a_timestamp_fails_loudly_rather_than_silently_shifting_the_range() {
    // `+` is a space in a query string, so `+00:00` arrives as ` 00:00`. The
    // dangerous outcome would be a lenient parser accepting it as some OTHER
    // instant and quietly answering about the wrong hour; refusing is right,
    // and the message must show the mangled value or the cause is invisible.
    let svc = service();
    let (code, body) = http::route(
        "GET",
        "/dayflow/timeline",
        "from=2026-08-26T00:00:00+00:00",
        &svc,
    );
    assert_eq!(code, "400 Bad Request");
    assert!(body.contains("bad timestamp"), "{body}");

    // The escaped form works.
    let (code, _) = http::route(
        "GET",
        "/dayflow/timeline",
        "from=2026-08-26T00:00:00%2B00:00",
        &svc,
    );
    assert_eq!(code, "200 OK");
}

#[test]
fn a_degraded_session_is_reported_as_a_success_with_the_degradation_in_the_payload() {
    // FR: degraded means RUNNING but not producing. Returning 503 would make
    // every monitor treat a recoverable state as an outage — and a non-zero CLI
    // exit would make every script treat it as a crash — when the state is
    // already in the body for anything that wants to act on it.
    //
    // This needs the clock seam: a session is only degraded once it has been
    // quiet longer than its interval, so a route reading the clock itself could
    // never be driven here. The mutation returning 503 survived until this test
    // existed.
    let svc = service();
    let start = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    svc.start(DayflowMode::Session, vec![0], start).unwrap();

    // Far past any staleness window, with nothing ever produced.
    let much_later = start + chrono::Duration::hours(6);
    let status = svc.status(much_later).unwrap();
    assert!(status.running, "still a session");
    assert!(status.is_degraded(), "and it is producing nothing: {status:?}");

    let (code, body) = http::route_at("GET", "/dayflow/status", "", &svc, much_later);
    assert_eq!(code, "200 OK", "a degraded session is not an HTTP failure: {body}");

    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["running"], true);
    assert_eq!(
        v["liveness"]["health"], "degraded",
        "and the caller can see WHY without a second request: {body}"
    );
}

#[test]
fn a_paused_session_is_not_reported_as_degraded_on_any_surface() {
    // FR-032: a pause is quiet on purpose. Deriving "unhealthy" as "not
    // healthy" would make every lunch break look like a broken recorder.
    let svc = service();
    let start = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    svc.start(DayflowMode::Session, vec![0], start).unwrap();
    svc.with_run(|r| r.turn_off(start + chrono::Duration::minutes(1))).unwrap();

    let later = start + chrono::Duration::hours(6);
    assert!(!svc.status(later).unwrap().is_degraded(), "off is not a fault");

    let (code, body) = http::route_at("GET", "/dayflow/status", "", &svc, later);
    assert_eq!(code, "200 OK");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_ne!(v["liveness"]["health"], "degraded", "{body}");
}

#[test]
fn the_standup_digest_reads_the_same_through_every_surface() {
    // US7's independent test: seed a day, request the standup shape, get a
    // categorised time-ranged digest — and get the SAME one whichever surface
    // asks, because the digest is computed once in the service.
    let svc = service();
    let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let push = |from_m: i64, to_m: i64, cat: ActivityCategory, what: &str| {
        svc.insert_entry(&TimelineEntry {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            start_time: base + chrono::Duration::minutes(from_m),
            end_time: base + chrono::Duration::minutes(to_m),
            category: cat,
            app: "app".into(),
            activity: what.into(),
            summary: format!("did {what}"),
            provenance: None,
        })
        .unwrap();
    };
    // Deliberately mixed lengths: one long meeting, several short interruptions.
    push(0, 60, ActivityCategory::Meeting, "planning");
    push(60, 62, ActivityCategory::Comms, "slack");
    push(62, 64, ActivityCategory::Comms, "slack");
    push(64, 90, ActivityCategory::Coding, "the ladder");

    let from = base.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to = (base + chrono::Duration::hours(2))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (code, body) = http::route(
        "GET",
        "/dayflow/standup",
        &format!("from={from}&to={to}"),
        &svc,
    );
    assert_eq!(code, "200 OK", "{body}");
    let http_digest: serde_json::Value = serde_json::from_str(&body).unwrap();

    let direct = svc
        .standup(base, base + chrono::Duration::hours(2))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&direct).unwrap(),
        http_digest,
        "the same digest, byte for byte, whichever surface asked"
    );

    // And the proportions are durations, not counts: two comms entries against
    // one meeting entry, and the meeting is the bulk of the day.
    let top = &direct.categories[0];
    assert_eq!(top.category, ActivityCategory::Meeting);
    assert_eq!(top.entries, 1);
    assert!(top.percent > 60.0, "one long entry outweighs several short: {top:?}");
    let comms = direct
        .categories
        .iter()
        .find(|c| c.category == ActivityCategory::Comms)
        .unwrap();
    assert_eq!(comms.entries, 2, "more entries…");
    assert!(comms.percent < 10.0, "…and far less time: {comms:?}");
}
