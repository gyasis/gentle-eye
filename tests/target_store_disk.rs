//! TG2 (T321) — target lifecycle round-trip through disk.
//!
//! Own test binary so the `HOME` override is isolated from the library's
//! in-process unit tests (no env race).

use gentle_eye::target::model::{NormRect, Target, TargetSource};
use gentle_eye::target::store::TargetStore;

#[test]
fn define_use_active_round_trips_through_disk() {
    let tmp = tempfile::tempdir().unwrap();
    // Safe on edition 2021; this test owns the process's HOME.
    std::env::set_var("HOME", tmp.path());

    let mut store = TargetStore::load().unwrap();
    store.add(Target::new(
        "left",
        TargetSource::Display { index: 0 },
        NormRect::new(0.0, 0.0, 0.5, 1.0),
    ));
    store.add(Target::new(
        "right",
        TargetSource::Display { index: 0 },
        NormRect::new(0.5, 0.0, 0.5, 1.0),
    ));
    store.set_active("right").unwrap();
    store.save().unwrap();

    // Fresh load from disk sees both targets and the right active one.
    let reloaded = TargetStore::load().unwrap();
    assert_eq!(reloaded.list().len(), 2);
    assert_eq!(reloaded.active().unwrap().name, "right");

    // The persisted file is where the display catalogue lives.
    let expected = tmp.path().join(".config/gentle-eye/targets.json");
    assert!(expected.exists(), "targets.json should be persisted next to display.json");
}
