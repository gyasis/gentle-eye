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
