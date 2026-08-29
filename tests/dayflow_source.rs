//! Capture-source abstraction: availability, identity, and the display source.

use gentle_eye::dayflow::sampler::DropReason;
use gentle_eye::dayflow::source::{Availability, SourceIdentity};
use gentle_eye::dayflow::window::PauseCause;

/// The three states must lead to three DIFFERENT outcomes, not merely be three
/// different enum variants. `ALL.contains(x)` style membership checks were a
/// repeated false pass in 013 (R25) — they cannot fail. This asserts the
/// mapping each state drives.
#[test]
fn availability_states_drive_distinct_outcomes() {
    // Available warrants NO gap. A gap is a recorded claim that capture
    // stopped; writing one for a healthy source invents a fact.
    assert_eq!(Availability::Available.gap_cause(), None);
    assert_eq!(
        Availability::Occluded.gap_cause(),
        Some(PauseCause::SourceOccluded)
    );
    assert_eq!(
        Availability::Ended.gap_cause(),
        Some(PauseCause::SourceEnded)
    );

    // The distinction that actually costs something if lost: retry.
    assert!(Availability::Available.retryable());
    assert!(Availability::Occluded.retryable());
    assert!(
        !Availability::Ended.retryable(),
        "an ended source retried every tick spins forever on a window that is gone"
    );

    // All three outcomes differ pairwise, so no two states are interchangeable.
    let outcomes = [
        (Availability::Available.gap_cause(), Availability::Available.retryable()),
        (Availability::Occluded.gap_cause(), Availability::Occluded.retryable()),
        (Availability::Ended.gap_cause(), Availability::Ended.retryable()),
    ];
    for i in 0..outcomes.len() {
        for j in (i + 1)..outcomes.len() {
            assert_ne!(outcomes[i], outcomes[j], "states {i} and {j} are indistinguishable");
        }
    }
}

/// An ended source must not auto-resume. This is the same rule `UserOff`
/// carries, reached for a different reason.
#[test]
fn ended_source_pause_is_not_automatic() {
    assert!(!PauseCause::SourceEnded.is_automatic());
    assert!(PauseCause::SourceOccluded.is_automatic());
}

/// A per-source failure is a DROP, not a gap. 013 separated these deliberately:
/// a gap says capture stopped, which is false while other sources produce.
#[test]
fn source_failure_has_its_own_drop_reason() {
    assert_eq!(DropReason::SourceUnavailable.label(), "source_unavailable");
    for other in [DropReason::MalformedFrame, DropReason::WriteFailed] {
        assert_ne!(DropReason::SourceUnavailable.label(), other.label());
    }
}

/// Position is not identity. A window dragged to another monitor is the same
/// source; if it were not, a day's work would split in two at the drag.
#[test]
fn identity_survives_a_move() {
    let before = SourceIdentity::new("window", "org.gnome.Terminal");
    let after = SourceIdentity::new("window", "org.gnome.Terminal");
    assert_eq!(before.hash(), after.hash());

    // A different window is a different source.
    let other = SourceIdentity::new("window", "firefox");
    assert_ne!(before.hash(), other.hash());

    // Two KINDS sharing a key must not collide.
    let as_target = SourceIdentity::new("target", "org.gnome.Terminal");
    assert_ne!(before.hash(), as_target.hash());
}

/// The separator matters: without it, ("win","dow") and ("window","") would
/// concatenate to the same bytes and hash identically.
#[test]
fn identity_cannot_be_forged_by_splitting_at_the_boundary() {
    let a = SourceIdentity::new("win", "dow");
    let b = SourceIdentity::new("window", "");
    assert_ne!(a.hash(), b.hash());
}

/// The id is written to disk, so its VALUE is part of the contract. A pinned
/// value is what makes a hash change fail loudly here instead of silently
/// rebinding every stored source on a toolchain upgrade (013/R31).
#[test]
fn identity_hash_value_is_pinned() {
    let id = SourceIdentity::new("display", "DP-1");
    assert_eq!(
        id.hash(),
        0x434d_cc3d_75a7_294c,
        "the on-disk source id changed — every stored row would stop matching"
    );
}

// ── the packing seam and the display source ──────────────────────────────────

use gentle_eye::dayflow::source::display::tightly_packed;

/// A padded capture must lose its padding, row by row — a sheared image passes
/// every dimension check, so this is asserted on bytes, not lengths alone.
#[test]
fn padded_rows_are_repacked_and_tight_ones_pass_through() {
    // 2x2 image, 8-byte rows, padded to a 12-byte stride.
    let row0 = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let row1 = [9u8, 10, 11, 12, 13, 14, 15, 16];
    let mut padded = Vec::new();
    padded.extend_from_slice(&row0);
    padded.extend_from_slice(&[0xAA; 4]); // padding
    padded.extend_from_slice(&row1);
    padded.extend_from_slice(&[0xBB; 4]); // padding
    let packed = tightly_packed(&padded, 2, 2);
    assert_eq!(packed.len(), 16, "2x2 BGRA is 16 tight bytes");
    assert_eq!(&packed[..8], &row0, "row 0 must survive without its padding");
    assert_eq!(&packed[8..], &row1, "row 1 must start where row 0 ends, not at the stride");

    // Already tight: byte-identical passthrough.
    let tight: Vec<u8> = (0u8..16).collect();
    assert_eq!(tightly_packed(&tight, 2, 2), tight);
}

/// A short buffer must not panic; the truncation is visible in the length.
/// (The private copy this replaced in `tests/dayflow_live.rs` had already
/// drifted by exactly this bounds check — the drift IS the argument for one
/// implementation.)
#[test]
fn a_short_final_row_truncates_instead_of_panicking() {
    // 14 bytes for a claimed 2x2 (row = 8): stride floors to 7, so row 1 would
    // need bytes 7..15 of a 14-byte buffer. The unguarded copy this replaced
    // panics on that slice; the guarded one keeps the complete row and stops.
    let short = vec![7u8; 14];
    let packed = tightly_packed(&short, 2, 2);
    assert_eq!(packed.len(), 8, "only the complete row survives");
}

/// LIVE (needs a real display, nothing else): the trait path and the direct
/// path `tests/dayflow_live.rs` drives must agree on everything observable
/// without a running loop — dimensions, packing, ordinal and identity.
///
/// What this does NOT prove, stated so nobody reads more into T005's DONE
/// line than exists: sample FILENAMES are minted by the `Sampler` from the
/// `display_id` it is handed, and no loop feeds it from a source yet (W4/T006);
/// filename identity therefore reduces to `ordinal() == index`, which IS
/// asserted here. Region equality is deferred behind the `Region::display_id`
/// producer gap (T010/T011).
#[test]
#[ignore = "live: needs a capturable display"]
fn display_source_matches_the_direct_capture_path() {
    use gentle_eye::capture::screen::ScreenCapturer;
    use gentle_eye::dayflow::source::{CaptureSource, DisplaySource};

    let displays = gentle_eye::capture::display::DisplayManager::list_available()
        .expect("display enumeration");
    assert!(!displays.is_empty(), "no capturable display — run on a machine with a screen");

    for d in &displays {
        let index = d.index as u32;
        let mut source = DisplaySource::new(index).expect("open via the trait");
        let mut direct = ScreenCapturer::new(d.index).expect("open directly");

        let frame = source.next_frame().expect("frame via the trait");
        let (w, h) = (direct.width(), direct.height());
        assert_eq!((frame.width, frame.height), (w as u32, h as u32), "dimensions must agree");
        assert_eq!(
            frame.bgra.len(),
            w * h * 4,
            "the trait hands the sampler TIGHT bgra — padded rows shear the image"
        );
        let raw = direct.capture_frame(std::time::Duration::from_secs(2)).expect("direct frame");
        assert_eq!(
            tightly_packed(&raw, w, h).len(),
            frame.bgra.len(),
            "both paths pack through the same function to the same size"
        );

        // The durable key's middle field: same value the old path passed, so
        // sample filenames (d{display_id}_w{seq}...) cannot change.
        assert_eq!(source.ordinal(), index, "ordinal IS the display index (D014-2)");
        let id = source.identity();
        assert_eq!((id.kind, id.key.as_str()), ("display", index.to_string().as_str()));
        assert_eq!(
            source.availability(),
            gentle_eye::dayflow::source::Availability::Available,
            "a source that just produced a frame is Available"
        );
    }
}
