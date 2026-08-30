//! W7 gate — `build_sources`, the spec→sources factory, on its platform-free
//! branches. The Displays and Window branches open real platform handles (an
//! X11 connection) and stay integration-only; the Input and Target-stream
//! branches are pure construction and are pinned here.
//!
//! Own test binary because the Target branch reads `TargetStore` from `$HOME`
//! — the override must not race the library's in-process unit tests (same
//! isolation rule as tests/target_store_disk.rs).

use gentle_eye::dayflow::source::{build_sources, SourceSpec};
use gentle_eye::target::model::{NormRect, Target, TargetSource};
use gentle_eye::target::store::TargetStore;

/// An input spec builds exactly one source: an `InputSource` whose identity is
/// the URL (a reconnect is the SAME input; a day must not split) at ordinal 0
/// (D014-2: a single non-display source occupies the `display_id` position).
#[test]
fn an_input_spec_builds_one_input_source_with_the_url_as_identity() {
    let dir = tempfile::tempdir().unwrap();
    let spec = SourceSpec::Input { url: "rtsp://cam.local/live".into() };
    let built = build_sources(&spec, dir.path()).expect("an input needs no platform handle");
    assert_eq!(built.len(), 1, "one URL, one source");
    let id = built[0].identity();
    assert_eq!(id.kind, "input");
    assert_eq!(id.key, "rtsp://cam.local/live");
    assert_eq!(built[0].ordinal(), 0);
}

/// An UNRESOLVED display spec is refused, not enumerated: the run's ordinals
/// were already derived from one enumeration, and a second one here could
/// disagree — the run says [0,1,2] while the thread builds a different set,
/// and samples file under ordinals no run window owns, silently.
#[test]
fn an_unresolved_display_spec_is_refused_not_reenumerated() {
    let dir = tempfile::tempdir().unwrap();
    let err = match build_sources(&SourceSpec::Displays { indices: Vec::new() }, dir.path()) {
        Err(e) => e,
        Ok(built) => panic!("empty indices must be refused, got {} sources", built.len()),
    };
    assert!(
        err.contains("ONE enumeration"),
        "the error must say WHY resolution belongs to the caller: {err}"
    );
}

/// A stream-backed target resolves to a `NamedTargetSource` over an
/// `InputSource`. The crop is applied ONCE — by `NamedTargetSource` via
/// `crop_bgra` on the decoded frame; the inner `InputSource` calls
/// `capture_stream_frame` (no ffmpeg `crop=` filter), so a double crop is
/// impossible by construction. Identity is the target's NAME: its rectangle
/// can be edited and it remains the same target.
#[test]
fn a_stream_backed_target_resolves_to_a_named_target_over_an_input() {
    let tmp = tempfile::tempdir().unwrap();
    // Safe: this binary owns the process's HOME (see the module doc).
    std::env::set_var("HOME", tmp.path());

    let mut store = TargetStore::load().unwrap();
    store.add(Target::new(
        "qa-panel",
        TargetSource::Stream { url: "rtsp://cam.local/live".into() },
        NormRect::new(0.25, 0.25, 0.5, 0.5),
    ));
    store.save().unwrap();

    let scratch = tempfile::tempdir().unwrap();
    let built = build_sources(&SourceSpec::Target { name: "qa-panel".into() }, scratch.path())
        .expect("a stream target needs no platform handle");
    assert_eq!(built.len(), 1);
    let id = built[0].identity();
    assert_eq!(id.kind, "target", "the outer source is the target, not the raw input");
    assert_eq!(id.key, "qa-panel", "identity is the NAME, stable across a rectangle edit");
    assert_eq!(built[0].ordinal(), 0);

    // A target that does not exist is an error naming the target, not a
    // session that silently records nothing.
    let missing = match build_sources(&SourceSpec::Target { name: "no-such".into() }, scratch.path()) {
        Err(e) => e,
        Ok(built) => panic!("a missing target must refuse to build, got {} sources", built.len()),
    };
    assert!(missing.contains("no-such"), "the error names WHICH target: {missing}");
}
