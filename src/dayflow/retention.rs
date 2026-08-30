//! Tiered retention: hot → warm → cold, with a disk-budget guard (US5).
//!
//! Hot (raw samples) → Warm (a timelapse plus the extracted text) → Cold (the
//! timeline entry alone, permanent). A budget guard evicts oldest raw first,
//! then oldest warm — and never the timeline.
//!
//! # The rule the whole module exists to enforce
//!
//! **Reclaiming storage may never destroy the only record of a period.** A
//! window that was not summarised still holds the only evidence of what
//! happened then; dropping it converts a backend outage into permanent data
//! loss that nothing downstream can distinguish from a genuinely idle hour.
//! So eviction is gated on `summarized`, not on age, and the timeline itself is
//! never a candidate at all.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::dayflow::errors::DayflowError;

/// How long each tier keeps its artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionConfig {
    /// How long raw samples are kept before AGE alone will shrink them.
    ///
    /// Not a floor: budget pressure shrinks a summarised window of any age.
    /// Stating it as a guarantee would be a promise the eviction path does not
    /// keep — the only real floor is `summarized`.
    pub hot: Duration,
    /// How long the shrunk artifact is kept before age alone will drop it.
    /// Same caveat as [`RetentionConfig::hot`].
    pub warm: Duration,
    /// Total bytes the raw + warm artifacts may occupy.
    pub budget_bytes: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            // A day of raw samples, so "what was I doing this morning" can still
            // be re-read at full fidelity, and a fortnight of timelapses.
            hot: Duration::from_secs(24 * 60 * 60),
            warm: Duration::from_secs(14 * 24 * 60 * 60),
            budget_bytes: 20 * 1024 * 1024 * 1024,
        }
    }
}

impl RetentionConfig {
    /// Build the planner's config from the user-facing policy
    /// (`config::RetentionConfig`).
    ///
    /// This is the ONE join between the two structs. Until the W8 gate wired
    /// it, `config::RetentionConfig` had zero readers outside the config
    /// module — a retention policy the user could set and nothing would ever
    /// consult, the same "expressible but inert" shape T020 fixed for
    /// residency.
    pub fn from_policy(p: &crate::config::RetentionConfig) -> Self {
        Self {
            hot: Duration::from_secs(u64::from(p.hot_grace_hours) * 60 * 60),
            warm: Duration::from_secs(u64::from(p.warm_days) * 24 * 60 * 60),
            budget_bytes: p.disk_budget_bytes,
        }
    }
}

/// Which tier a window's artifacts currently belong in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Raw samples on disk.
    Hot,
    /// Shrunk to a timelapse plus retained text.
    Warm,
    /// Timeline entry only — permanent, never evicted.
    Cold,
}

/// One window's storage state, as retention sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRecord {
    /// Durable identity: `(session, display, sequence)` — never the filename.
    pub sequence: u64,
    /// Which display produced it.
    pub display_id: u32,
    /// When the window closed.
    pub closed_at: DateTime<Utc>,
    /// Whether a summary was actually written for it.
    ///
    /// The single most consequential field in this module: everything eviction
    /// is allowed to do is gated on it.
    pub summarized: bool,
    /// Raw sample files, if they still exist.
    pub raw: Vec<PathBuf>,
    /// The shrunk artifact, if the window has been through [`shrink`].
    pub warm_artifact: Option<PathBuf>,
    /// Bytes the raw samples occupy.
    pub raw_bytes: u64,
    /// Bytes the warm artifact occupies.
    pub warm_bytes: u64,
}

impl SegmentRecord {
    /// The durable identity: display and sequence, never the filename.
    pub fn key(&self) -> (u32, u64) {
        (self.display_id, self.sequence)
    }

    /// Bytes a shrink would actually free.
    ///
    /// NET of the timelapse it writes to the same disk. Crediting the full
    /// `raw_bytes` lets the plan declare the budget met while real usage lands
    /// up to the SC-008 ceiling over it — self-correcting only on the next run,
    /// and then by dropping warm artifacts that should never have been needed.
    pub fn freed_by_shrink(&self) -> u64 {
        self.raw_bytes.saturating_sub((self.raw_bytes as f64 * WARM_BUDGET_RATIO) as u64)
    }

    /// The tier this window is in right now.
    ///
    /// Derived from the RECORD of what is on disk, not from age: a window whose
    /// raw samples are gone is warm however recent it is, and one never shrunk
    /// is hot however old. Deriving it from age would report a state the
    /// filesystem does not have — which retention then acts on by deleting.
    ///
    /// "Record of", not the disk itself: this reads the struct's own fields, so
    /// it is only as true as whoever maintains them. That is why [`shrink`]
    /// takes `&mut self` and updates the record itself rather than trusting a
    /// caller to remember — a stale record schedules a shrink whose encode
    /// reads files that are already gone.
    pub fn tier(&self) -> Tier {
        if !self.raw.is_empty() {
            // Raw AND a warm artifact means a crash between writing the
            // timelapse and clearing the samples. Hot is the safe reading — the
            // raw frames are still the better record — and [`shrink`] deletes
            // the stale timelapse before writing its replacement, so the
            // half-finished one cannot be orphaned on disk forever with no
            // record pointing at it.
            Tier::Hot
        } else if self.warm_artifact.is_some() {
            Tier::Warm
        } else {
            Tier::Cold
        }
    }

    /// Whether this window may be shrunk at `now` under `cfg`.
    ///
    /// **Never before it is summarised**, whatever its age. Shrinking discards
    /// the raw frames, and the summary is what replaces them; doing it first
    /// destroys the input to a step that has not run.
    pub fn may_shrink(&self, now: DateTime<Utc>, cfg: &RetentionConfig) -> bool {
        self.summarized && self.tier() == Tier::Hot && self.age(now) >= cfg.hot
    }

    /// Whether the warm artifact may be dropped at `now` under `cfg`.
    pub fn may_drop_warm(&self, now: DateTime<Utc>, cfg: &RetentionConfig) -> bool {
        self.summarized && self.tier() == Tier::Warm && self.age(now) >= cfg.warm
    }

    /// How long ago this window closed. Zero if the clock moved backwards.
    pub fn age(&self, now: DateTime<Utc>) -> Duration {
        (now - self.closed_at).to_std().unwrap_or(Duration::ZERO)
    }

    /// Bytes this window currently occupies.
    pub fn bytes(&self) -> u64 {
        self.raw_bytes.saturating_add(self.warm_bytes)
    }
}

/// Why a reclaim step refused to touch a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It was never summarised — its raw samples are the only record.
    NotSummarized,
    /// There is nothing in that tier to reclaim.
    NothingToReclaim,
    /// Reclaiming it was not necessary — the budget was already satisfied.
    WithinBudget,
}

/// One reclaim decision, with the reason it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Which display the window came from.
    ///
    /// Part of the key, not decoration: sequences are per display (the daemon
    /// keeps a counter each), so display 0 and display 1 both emit 0, 1, 2 …
    /// Keying decisions on `sequence` alone silently collapsed the two on every
    /// multi-monitor machine — one window per colliding sequence was never
    /// actioned, its bytes were credited to the budget anyway, and the ledger
    /// reported it as merely too recent.
    pub display_id: u32,
    /// The window, within that display.
    pub sequence: u64,
    /// What retention decided.
    pub action: Action,
}

/// The durable identity of a window, as retention keys it.
type WindowKey = (u32, u64);

/// What retention decided to do with a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Replace the raw samples with a timelapse plus retained text.
    Shrink,
    /// Drop the warm artifact; the timeline entry remains.
    DropWarm,
    /// Leave it alone, for this reason.
    Keep(Refusal),
}

/// Plan the reclaim steps needed to bring `segments` under budget at `now`.
///
/// Returns a decision for EVERY segment, including the ones left alone and why.
/// A planner that returned only its actions would make "nothing was reclaimed"
/// and "everything was refused" look identical, which is exactly the question
/// an operator asks when the disk fills up anyway.
///
/// # Order
///
/// Age-expired work first, then, if still over budget, oldest raw and finally
/// oldest warm. Raw before warm because raw is both larger and the more
/// reconstructible: the timelapse plus text is what a person actually reads
/// back, and it is a tenth of the size (SC-008).
pub fn plan(
    segments: &[SegmentRecord],
    now: DateTime<Utc>,
    cfg: &RetentionConfig,
) -> Vec<Decision> {
    let mut decisions: Vec<Decision> = Vec::with_capacity(segments.len());
    let mut freed: u64 = 0;
    let total: u64 = segments.iter().map(SegmentRecord::bytes).sum();

    // Oldest first, deterministically: sequence breaks ties so two windows that
    // closed in the same instant are still ordered the same way on every run.
    let mut order: Vec<&SegmentRecord> = segments.iter().collect();
    order.sort_by_key(|s| (s.closed_at, s.key()));

    // Pass 1 — age. What has simply timed out of its tier.
    let mut planned: std::collections::HashMap<WindowKey, Action> =
        std::collections::HashMap::new();
    for s in &order {
        if s.may_shrink(now, cfg) {
            freed += s.freed_by_shrink();
            planned.insert(s.key(), Action::Shrink);
        } else if s.may_drop_warm(now, cfg) {
            freed += s.warm_bytes;
            planned.insert(s.key(), Action::DropWarm);
        }
    }

    // Pass 2 — budget. Only if age alone did not get us under.
    //
    // ALL reclaimable raw first, oldest-first, and only then warm. Sweeping
    // both tiers in one oldest-first pass looks equivalent and is not: the
    // oldest window is often already warm, so a single pass drops the
    // timelapse a person actually reads back while a hundred megabytes of raw
    // frames — larger, and already superseded by a summary — sit untouched.
    for tier in [Tier::Hot, Tier::Warm] {
        for s in &order {
            if total.saturating_sub(freed) <= cfg.budget_bytes {
                break;
            }
            if planned.contains_key(&s.key()) || s.tier() != tier {
                continue;
            }
            // The gate. An unsummarised window's raw samples are the ONLY
            // record of that period, so being over budget is not a reason to
            // destroy them — it is a reason to stop capturing, which is a
            // decision for a layer that can tell the user.
            if !s.summarized {
                continue;
            }
            match tier {
                Tier::Hot => {
                    freed += s.freed_by_shrink();
                    planned.insert(s.key(), Action::Shrink);
                }
                Tier::Warm => {
                    freed += s.warm_bytes;
                    planned.insert(s.key(), Action::DropWarm);
                }
                Tier::Cold => {}
            }
        }
    }

    for s in segments {
        let action = planned.remove(&s.key()).unwrap_or_else(|| {
            Action::Keep(if !s.summarized {
                Refusal::NotSummarized
            } else if s.tier() == Tier::Cold {
                Refusal::NothingToReclaim
            } else {
                // Always `WithinBudget` here, and that is not a shortcut: pass 2
                // exhausts EVERY summarised Hot/Warm segment before it can end
                // over budget, so an unplanned summarised segment can only exist
                // because the budget was met. Age is never the binding
                // constraint — pass 2 ignores it — so a "too recent" label would
                // name something that was not protecting the window. The enum no
                // longer carries that variant rather than leaving a reason the
                // planner cannot produce.
                Refusal::WithinBudget
            })
        });
        decisions.push(Decision { display_id: s.display_id, sequence: s.sequence, action });
    }
    decisions
}

/// Delete a file that retention decided to reclaim.
///
/// Every path goes through the validator before deletion. Retention is the only
/// part of dayflow that removes data, and the paths it acts on come from a
/// database that other processes write — so "the path came from our own
/// records" is not a safety argument. A traversal outside the capture directory
/// must fail loudly rather than delete.
pub fn reclaim_file(
    path: &Path,
    validator: &crate::security::path_validator::PathValidator,
) -> Result<(), DayflowError> {
    let safe = validator.validate(path).map_err(|e| {
        DayflowError::Retention(format!("refusing to delete {}: {e}", path.display()))
    })?;
    match std::fs::remove_file(&safe) {
        Ok(()) => Ok(()),
        // Already gone is the goal state, not a failure: a retry after a
        // partial run must not abort the rest of the sweep.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(DayflowError::Retention(format!("delete {}: {e}", safe.display()))),
    }
}

/// What a window keeps after shrinking.
///
/// The raw frames go; a timelapse and the EXTRACTED TEXT stay. Keeping the text
/// is the point: a timelapse answers "what did this look like", and the text
/// answers "what did it say" — which is the question a timeline is asked, and
/// the only one the raw frames were ever needed for once a summary exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmArtifact {
    /// The timelapse standing in for the raw samples.
    pub timelapse: PathBuf,
    /// Text extracted from those samples, retained verbatim.
    pub text: String,
    /// Bytes the timelapse occupies.
    pub bytes: u64,
}

/// The share of the raw bytes a warm artifact may occupy (SC-008).
pub const WARM_BUDGET_RATIO: f64 = 0.10;

/// Shrink a window: raw samples out, timelapse plus text in.
///
/// `encode` produces the timelapse from the raw samples and returns its size;
/// it is a parameter so the retention RULES — which are the part that must not
/// be got wrong — are testable without ffmpeg.
///
/// Refuses a window that was not summarised, whatever its age: shrinking
/// discards the frames the summary is made from.
pub fn shrink<F>(
    segment: &mut SegmentRecord,
    extracted_text: &str,
    validator: &crate::security::path_validator::PathValidator,
    mut encode: F,
) -> Result<WarmArtifact, DayflowError>
where
    F: FnMut(&[PathBuf]) -> Result<(PathBuf, u64), DayflowError>,
{
    if !segment.summarized {
        return Err(DayflowError::Retention(format!(
            "refusing to shrink window {} before it is summarised: its raw samples \
             are the only record of that period",
            segment.sequence
        )));
    }
    if segment.raw.is_empty() {
        return Err(DayflowError::Retention(format!(
            "window {} has no raw samples to shrink",
            segment.sequence
        )));
    }

    let (timelapse, bytes) = encode(&segment.raw)?;

    // The WRITE path is validated like the delete path. "The encoder is our
    // own closure" is no more a safety argument than "the path came from our
    // own records" is for deletes: the timelapse destination is built from
    // configuration and records other processes write. A path outside the
    // capture directory refuses the shrink — raw samples intact — rather than
    // install a record pointing outside the tree retention manages. The stray
    // file itself is deliberately left where the encoder put it: deleting
    // outside the root is exactly what the validator forbids.
    if let Err(e) = validator.validate(&timelapse) {
        return Err(DayflowError::Retention(format!(
            "refusing timelapse for window {} at {}: {e}",
            segment.sequence,
            timelapse.display()
        )));
    }

    // SC-008 checked HERE, before anything is deleted. A timelapse bigger than
    // the budget means the encode did not do what shrinking is for, and
    // deleting the raw frames on the strength of it would spend the only copy
    // to save nothing.
    let ceiling = (segment.raw_bytes as f64 * WARM_BUDGET_RATIO) as u64;
    if bytes > ceiling {
        // The encode already wrote it. Leaving it behind accumulates
        // over-ceiling files inside the module whose job is freeing space.
        let _ = reclaim_file(&timelapse, validator);
        return Err(DayflowError::Retention(format!(
            "timelapse for window {} is {bytes} bytes, over the {ceiling} ceiling \
             ({:.0}% of {} raw) — refusing to delete the raw samples for it",
            segment.sequence,
            WARM_BUDGET_RATIO * 100.0,
            segment.raw_bytes
        )));
    }

    // RECORD FIRST, then the disk.
    //
    // The obvious order — delete, then update — has no safe failure point: an
    // error partway through leaves files gone while the record still lists
    // them, so `tier()` reports Hot for a window whose samples do not exist and
    // the next plan schedules a shrink whose encode is handed deleted paths.
    // With a real encoder that wedges the window permanently.
    //
    // Pointing the record at the new timelapse first means every failure lands
    // in the mixed raw-and-warm state `tier()` was taught to read as Hot: the
    // raw frames are still there, still the better record, and a retry
    // completes the job.
    let stale = segment.warm_artifact.replace(timelapse.clone());
    segment.warm_bytes = bytes;

    // Best-effort: a leftover timelapse from a crashed earlier attempt would be
    // orphaned otherwise — nothing points at it, so retention could never
    // reclaim it. A failure to delete it must not abort the shrink, or one
    // transient error strands the window forever.
    if let Some(stale) = stale {
        if stale != timelapse {
            if let Err(e) = reclaim_file(&stale, validator) {
                tracing::warn!(error = %e, path = %stale.display(),
                    "could not reclaim a superseded timelapse; it will leak");
            }
        }
    }

    // Now the originals, dropping each from the record as it goes so a partial
    // failure leaves a record that MATCHES the disk rather than one that lies
    // about it in either direction.
    let mut failure: Option<DayflowError> = None;
    segment.raw.retain(|p| match reclaim_file(p, validator) {
        Ok(()) => false,
        Err(e) => {
            failure.get_or_insert(e);
            true
        }
    });
    // Measured, not assumed: whatever survived is what the budget must count.
    segment.raw_bytes = segment
        .raw
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    if let Some(e) = failure {
        return Err(e);
    }

    Ok(WarmArtifact { timelapse, text: extracted_text.to_string(), bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    fn seg(sequence: u64, closed: i64, summarized: bool) -> SegmentRecord {
        SegmentRecord {
            sequence,
            display_id: 0,
            closed_at: at(closed),
            summarized,
            raw: vec![PathBuf::from(format!("/tmp/d0_w{sequence:06}_a.png"))],
            warm_artifact: None,
            raw_bytes: 100 * 1024 * 1024,
            warm_bytes: 0,
        }
    }

    fn warm(sequence: u64, closed: i64, summarized: bool) -> SegmentRecord {
        SegmentRecord {
            raw: Vec::new(),
            warm_artifact: Some(PathBuf::from(format!("/tmp/w{sequence}.mp4"))),
            raw_bytes: 0,
            warm_bytes: 10 * 1024 * 1024,
            ..seg(sequence, closed, summarized)
        }
    }

    fn cfg(budget_bytes: u64) -> RetentionConfig {
        RetentionConfig { hot: Duration::from_secs(3600), warm: Duration::from_secs(7200), budget_bytes }
    }

    fn action_for(d: &[Decision], sequence: u64) -> &Action {
        on_display(d, 0, sequence)
    }

    fn on_display(d: &[Decision], display_id: u32, sequence: u64) -> &Action {
        &d.iter()
            .find(|x| x.display_id == display_id && x.sequence == sequence)
            .expect("a decision per segment")
            .action
    }

    #[test]
    fn an_unsummarised_window_is_never_reclaimed_however_old_or_over_budget() {
        // THE rule. Its raw samples are the only record of that period, so
        // reclaiming them turns a backend outage into permanent data loss that
        // nothing downstream can distinguish from a genuinely idle hour.
        let segments = vec![seg(1, 0, false)];
        // Ancient, and the budget is a single byte.
        let d = plan(&segments, at(10_000_000), &cfg(1));
        assert_eq!(*action_for(&d, 1), Action::Keep(Refusal::NotSummarized));
    }

    #[test]
    fn being_over_budget_does_not_relax_that_rule() {
        // The dangerous shape: a disk emergency is exactly when a "just free
        // something" path gets added. Every window here is unsummarised, so the
        // correct outcome is to free NOTHING and stay over budget — the fix for
        // that is to stop capturing, which is a decision for a layer that can
        // tell the user.
        let segments: Vec<_> = (1..=10).map(|i| seg(i, i as i64 * 60, false)).collect();
        let d = plan(&segments, at(10_000_000), &cfg(1));
        assert!(
            d.iter().all(|x| x.action == Action::Keep(Refusal::NotSummarized)),
            "not one unsummarised window may be reclaimed to make room"
        );
    }

    #[test]
    fn a_summarised_window_shrinks_once_it_is_old_enough() {
        let segments = vec![seg(1, 0, true)];
        // Not yet: hot is 3600s.
        assert_eq!(
            *action_for(&plan(&segments, at(100), &cfg(u64::MAX)), 1),
            Action::Keep(Refusal::WithinBudget),
            "not old enough AND not needed — the budget reason is the true one"
        );
        // Now.
        assert_eq!(*action_for(&plan(&segments, at(4_000), &cfg(u64::MAX)), 1), Action::Shrink);
    }

    #[test]
    fn eviction_takes_raw_before_warm_and_oldest_first() {
        // Raw before warm because raw is both larger and the more
        // reconstructible: the timelapse plus text is what a person reads back.
        let segments = vec![
            warm(1, 0, true),   // oldest, already warm
            seg(2, 100, true),  // raw, older
            seg(3, 200, true),  // raw, newer
        ];
        // Budget forces exactly one raw shrink: total 210MB, budget 150MB.
        let d = plan(&segments, at(500), &cfg(150 * 1024 * 1024));
        assert_eq!(*action_for(&d, 2), Action::Shrink, "the OLDER raw goes first");
        assert_eq!(*action_for(&d, 3), Action::Keep(Refusal::WithinBudget), "the newer stays");
        assert_eq!(
            *action_for(&d, 1),
            Action::Keep(Refusal::WithinBudget),
            "the warm artifact is not touched while raw remains reclaimable"
        );
    }

    #[test]
    fn warm_is_only_dropped_once_raw_has_been_taken() {
        // Same three segments, but the budget is small enough that shrinking
        // the raw is not enough.
        let segments = vec![warm(1, 0, true), seg(2, 100, true), seg(3, 200, true)];
        let d = plan(&segments, at(500), &cfg(1));
        assert_eq!(*action_for(&d, 2), Action::Shrink);
        assert_eq!(*action_for(&d, 3), Action::Shrink);
        assert_eq!(*action_for(&d, 1), Action::DropWarm, "and only then the warm one");
    }

    #[test]
    fn a_planner_reports_a_decision_for_every_segment_including_the_untouched() {
        // "Nothing was reclaimed" and "everything was refused" are the same
        // observation to a planner that returns only its actions — and they are
        // the two answers an operator needs to tell apart when the disk fills
        // up anyway.
        let segments = vec![seg(1, 0, false), seg(2, 0, true), warm(3, 0, true)];
        let d = plan(&segments, at(50), &cfg(u64::MAX));
        assert_eq!(d.len(), segments.len());
        assert_eq!(*action_for(&d, 1), Action::Keep(Refusal::NotSummarized));
        assert_eq!(*action_for(&d, 2), Action::Keep(Refusal::WithinBudget));
        assert_eq!(*action_for(&d, 3), Action::Keep(Refusal::WithinBudget));
    }

    #[test]
    fn the_tier_is_read_from_disk_not_inferred_from_age() {
        // A window whose raw samples are gone is warm however recent it is, and
        // one never shrunk is hot however old. Deriving the tier from age would
        // report a state the filesystem does not have.
        let ancient_but_raw = seg(1, 0, true);
        assert_eq!(ancient_but_raw.tier(), Tier::Hot);
        let recent_but_shrunk = warm(2, 999_999, true);
        assert_eq!(recent_but_shrunk.tier(), Tier::Warm);

        let cold = SegmentRecord { warm_artifact: None, warm_bytes: 0, ..warm(3, 0, true) };
        assert_eq!(cold.tier(), Tier::Cold);
        // and cold is never a candidate for anything
        let d = plan(&[cold], at(10_000_000), &cfg(1));
        assert_eq!(*action_for(&d, 3), Action::Keep(Refusal::NothingToReclaim));
    }

    #[test]
    fn shrinking_before_summarising_is_refused_however_old_the_window_is() {
        // Shrinking discards the raw frames and the summary is what replaces
        // them, so doing it first destroys the input to a step that has not run.
        let s = seg(1, 0, false);
        assert!(!s.may_shrink(at(10_000_000), &cfg(u64::MAX)));
        let done = seg(2, 0, true);
        assert!(done.may_shrink(at(10_000_000), &cfg(u64::MAX)));
    }

    #[test]
    fn a_backwards_clock_does_not_make_a_window_ancient() {
        // `now - closed_at` is negative after a clock step; a saturating
        // conversion that wrapped would make a fresh window look days old and
        // hand it straight to eviction.
        let s = seg(1, 10_000, true);
        assert_eq!(s.age(at(0)), Duration::ZERO);
        assert!(!s.may_shrink(at(0), &cfg(u64::MAX)), "and it is not reclaimable");
    }


    fn tmp_segment(dir: &Path, sequence: u64, summarized: bool, n: usize) -> SegmentRecord {
        let raw: Vec<PathBuf> = (0..n)
            .map(|i| {
                let p = dir.join(format!("d0_w{sequence:06}_{i}.png"));
                std::fs::write(&p, vec![0u8; 1_000_000]).unwrap();
                p
            })
            .collect();
        SegmentRecord {
            raw_bytes: (n * 1_000_000) as u64,
            raw,
            ..seg(sequence, 0, summarized)
        }
    }


    #[test]
    fn two_displays_sharing_a_sequence_are_two_different_windows() {
        // Sequences are PER DISPLAY — the daemon keeps a counter each — so
        // display 0 and display 1 both emit 0, 1, 2… Keying decisions on
        // `sequence` alone collapsed them: one window per colliding sequence
        // was never actioned, its bytes were credited to the budget anyway,
        // and the ledger reported it as merely too recent.
        let a = SegmentRecord { display_id: 0, ..seg(7, 0, true) };
        let b = SegmentRecord { display_id: 1, ..seg(7, 0, true) };

        let d = plan(&[a, b], at(10_000), &cfg(u64::MAX));
        assert_eq!(d.len(), 2, "two windows, two decisions");
        assert_eq!(*on_display(&d, 0, 7), Action::Shrink, "display 0's window is handled");
        assert_eq!(*on_display(&d, 1, 7), Action::Shrink, "and so is display 1's");
    }

    #[test]
    fn a_warm_artifact_expires_by_age_alone_with_the_budget_untouched() {
        // The "fortnight of timelapses" policy. Every other warm-drop assertion
        // goes through the BUDGET path, so deleting this branch entirely left
        // the whole `cfg.warm` policy with zero verification — and the mutation
        // survived the suite.
        let segments = vec![warm(1, 0, true)];
        let big_budget = cfg(u64::MAX);

        assert_eq!(
            *action_for(&plan(&segments, at(1_000), &big_budget), 1),
            Action::Keep(Refusal::WithinBudget),
            "before its warm window elapses"
        );
        assert_eq!(
            *action_for(&plan(&segments, at(8_000), &big_budget), 1),
            Action::DropWarm,
            "after it, with no budget pressure at all"
        );
    }

    #[test]
    fn age_reclaim_counts_toward_the_budget_so_nothing_extra_is_taken() {
        // The two passes share `freed`. Without that, pass 2 believes nothing
        // has been reclaimed and takes a second, younger window it did not need
        // — over-evicting silently. Every earlier test used a budget of
        // u64::MAX or 1, so this interaction was never exercised.
        let old_enough = seg(1, 0, true);      // 100MB, age-expired
        let young = seg(2, 9_500, true);       // 100MB, far too recent
        let segments = vec![old_enough, young];

        // Total 200MB. Shrinking window 1 frees 90MB (net of its timelapse),
        // which brings us under a 150MB budget.
        let d = plan(&segments, at(10_000), &cfg(150 * 1024 * 1024));
        assert_eq!(*action_for(&d, 1), Action::Shrink, "the age-expired one goes");
        assert_eq!(
            *action_for(&d, 2),
            Action::Keep(Refusal::WithinBudget),
            "and the young one is NOT taken as well"
        );
    }

    #[test]
    fn a_shrink_is_credited_net_of_the_timelapse_it_writes() {
        // Crediting the full raw_bytes lets the plan declare the budget met
        // while real usage lands up to the SC-008 ceiling over it — correcting
        // itself only next run, and then by dropping warm artifacts that should
        // never have been needed.
        let s = seg(1, 0, true);
        assert!(s.freed_by_shrink() < s.raw_bytes, "the timelapse stays on the same disk");
        assert_eq!(
            s.freed_by_shrink(),
            s.raw_bytes - (s.raw_bytes as f64 * WARM_BUDGET_RATIO) as u64
        );
    }

    #[test]
    fn a_window_holding_both_raw_and_warm_bytes_is_accounted_for_correctly() {
        // Every fixture had one tier or the other, so any raw/warm confusion in
        // the accounting was undetectable. A record with both is what a crash
        // between writing the timelapse and clearing the samples leaves behind.
        let mixed = SegmentRecord {
            warm_artifact: Some(PathBuf::from("/tmp/half-done.mp4")),
            warm_bytes: 7 * 1024 * 1024,
            ..seg(1, 0, true)
        };
        assert_eq!(mixed.tier(), Tier::Hot, "raw frames are the better record");
        assert_eq!(mixed.bytes(), mixed.raw_bytes + mixed.warm_bytes, "both counted");
        assert!(
            mixed.freed_by_shrink() < mixed.bytes(),
            "shrinking it cannot free the warm bytes it is also holding"
        );
    }


    #[test]
    fn a_failure_reclaiming_the_stale_timelapse_does_not_strand_the_window() {
        // The bug the &mut refactor introduced: `take()` ran BEFORE the fallible
        // reclaim, so a transient delete error erased the record's only pointer
        // to the stale file — orphaning it — while the raw samples were already
        // gone. The record then claimed Hot for a window whose samples did not
        // exist, and every retry handed the encoder deleted paths.
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());

        // A stale artifact OUTSIDE the validator root: reclaiming it will fail,
        // which is exactly the transient error being simulated.
        let elsewhere = tempfile::tempdir().unwrap();
        let stale = elsewhere.path().join("stale.mp4");
        std::fs::write(&stale, b"stale").unwrap();

        let mut s = SegmentRecord {
            warm_artifact: Some(stale.clone()),
            warm_bytes: 1,
            ..tmp_segment(dir.path(), 1, true, 3)
        };
        let fresh = dir.path().join("fresh.mp4");
        std::fs::write(&fresh, b"x").unwrap();

        let art = shrink(&mut s, "t", &v, |_| Ok((fresh.clone(), 1_000)))
            .expect("an unreclaimable stale artifact must not fail the shrink");

        assert_eq!(art.timelapse, fresh);
        assert_eq!(
            s.warm_artifact.as_deref(),
            Some(fresh.as_path()),
            "the record points at the NEW timelapse, never at nothing"
        );
        assert_eq!(s.tier(), Tier::Warm, "and the shrink completed");
        assert!(s.raw.is_empty());
    }

    #[test]
    fn a_partial_delete_leaves_a_record_that_matches_the_disk() {
        // If some raw samples cannot be removed, the record must list exactly
        // the ones that survived — not all of them (the next encode reads
        // deleted paths) and not none of them (the bytes vanish from the budget
        // while still occupying disk).
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let mut s = tmp_segment(dir.path(), 1, true, 3);

        // One sample is outside the root, so its reclaim is refused.
        let elsewhere = tempfile::tempdir().unwrap();
        let stubborn = elsewhere.path().join("stubborn.png");
        std::fs::write(&stubborn, vec![0u8; 1_000_000]).unwrap();
        s.raw.push(stubborn.clone());
        s.raw_bytes += 1_000_000;

        let fresh = dir.path().join("f.mp4");
        std::fs::write(&fresh, b"x").unwrap();
        let err = shrink(&mut s, "t", &v, |_| Ok((fresh.clone(), 1_000)));
        assert!(err.is_err(), "the failure is reported, not swallowed");

        assert_eq!(s.raw, vec![stubborn.clone()], "only the survivor is listed");
        assert_eq!(s.raw_bytes, 1_000_000, "and its bytes are measured, not assumed");
        assert!(stubborn.exists());
        assert_eq!(s.tier(), Tier::Hot, "recoverable: a retry finishes the job");
        assert_eq!(
            s.warm_artifact.as_deref(),
            Some(fresh.as_path()),
            "the new timelapse is already recorded, so it cannot be orphaned"
        );
    }

    #[test]
    fn the_warm_tier_is_also_evicted_oldest_first() {
        // Every warm test had at most ONE warm segment, so sweeping the tier
        // newest-first survived the suite — and that mutant destroys the most
        // recent timelapse, the one a person is most likely to read back.
        let segments = vec![warm(1, 0, true), warm(2, 500, true)];
        // 20MB total, budget forces exactly one drop.
        let d = plan(&segments, at(600), &cfg(15 * 1024 * 1024));
        assert_eq!(*action_for(&d, 1), Action::DropWarm, "the OLDER timelapse goes");
        assert_eq!(*action_for(&d, 2), Action::Keep(Refusal::WithinBudget), "the newer stays");
    }

    #[test]
    fn the_net_credit_is_load_bearing_inside_the_ten_percent_margin() {
        // The unit test above pins the FORMULA; this pins that `plan` uses it.
        // Reverting both call sites to the gross figure survived the suite,
        // because no fixture put the budget inside the gross/net gap — the
        // extremes problem again, in a 10%-wide band.
        //
        // Two 100MB windows, budget 105MB. Net: each shrink frees 90MB, so one
        // is not enough (110 > 105) and BOTH must go. Gross: the first appears
        // to free 100MB, the plan stops, and real disk lands at 110MB — over.
        //
        // Evaluated BEFORE either is age-expired (hot is 3600s), so the budget
        // path is what decides. An earlier version used t=10_000, where pass 1
        // shrinks both unconditionally and the arithmetic under test never
        // runs — the mutation survived it.
        let segments = vec![seg(1, 0, true), seg(2, 100, true)];
        let d = plan(&segments, at(1_000), &cfg(105 * 1024 * 1024));
        assert_eq!(*action_for(&d, 1), Action::Shrink);
        assert_eq!(
            *action_for(&d, 2),
            Action::Shrink,
            "a gross credit would stop after one and leave the disk over budget"
        );
    }

    #[test]
    fn a_stale_timelapse_is_removed_rather_than_orphaned() {
        // Overwriting `warm_artifact` without deleting the old file leaves it
        // on disk with nothing pointing at it — so retention, whose whole job
        // is freeing space, could never reclaim it.
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let stale = dir.path().join("stale.mp4");
        std::fs::write(&stale, b"orphan-me").unwrap();

        let mut s = SegmentRecord {
            warm_artifact: Some(stale.clone()),
            warm_bytes: 1,
            ..tmp_segment(dir.path(), 1, true, 3)
        };
        let fresh = dir.path().join("fresh.mp4");
        std::fs::write(&fresh, b"x").unwrap();
        shrink(&mut s, "t", &v, |_| Ok((fresh.clone(), 1_000))).unwrap();

        assert!(!stale.exists(), "the half-finished timelapse is reclaimed");
        assert!(fresh.exists());
        assert_eq!(s.warm_artifact.as_deref(), Some(fresh.as_path()));
    }

    #[test]
    fn an_over_ceiling_timelapse_is_not_left_on_disk() {
        // The encode already wrote it. Leaving it behind accumulates
        // over-ceiling files inside the module whose job is freeing space.
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let mut s = tmp_segment(dir.path(), 1, true, 5);
        let big = dir.path().join("too-big.mp4");
        std::fs::write(&big, b"x").unwrap();

        assert!(shrink(&mut s, "t", &v, |_| Ok((big.clone(), 4_000_000))).is_err());
        assert!(!big.exists(), "the rejected timelapse is cleaned up");
        assert!(s.raw.iter().all(|p| p.exists()), "and the raw samples still survive");
    }

    #[test]
    fn the_tier_and_age_boundaries_are_inclusive() {
        // `age == hot` and `remaining == budget` are the values a real sweep
        // hits every cycle, and neither was exercised.
        let s = seg(1, 0, true);
        let c = cfg(u64::MAX);
        assert!(!s.may_shrink(at(3_599), &c), "one second short");
        assert!(s.may_shrink(at(3_600), &c), "exactly at the boundary DOES shrink");

        // remaining == budget is within budget, not over it
        let segments = vec![seg(1, 0, true)];
        let exactly = plan(&segments, at(100), &cfg(100 * 1024 * 1024));
        assert_eq!(*action_for(&exactly, 1), Action::Keep(Refusal::WithinBudget));
    }

    #[test]
    fn shrinking_replaces_the_raw_samples_and_keeps_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let mut s = tmp_segment(dir.path(), 1, true, 5);
        let lapse = dir.path().join("w1.mp4");
        std::fs::write(&lapse, vec![0u8; 200_000]).unwrap();

        let art = shrink(&mut s, "the text that was on screen", &v, |_| Ok((lapse.clone(), 200_000)))
            .expect("a summarised window shrinks");

        assert_eq!(art.text, "the text that was on screen", "the text is what a timeline is asked");
        assert!(s.raw.iter().all(|p| !p.exists()), "the raw samples are gone");
        assert!(art.timelapse.exists(), "and the replacement is on disk");
    }

    #[test]
    fn the_warm_artifact_is_at_most_a_tenth_of_the_raw_it_replaces() {
        // SC-008, and checked BEFORE anything is deleted: a timelapse over the
        // ceiling means the encode did not do what shrinking is for, and
        // deleting the raw frames on the strength of it spends the only copy to
        // save nothing.
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let mut s = tmp_segment(dir.path(), 2, true, 5); // 5 MB raw
        let raw_before = s.raw_bytes;

        let ok = dir.path().join("small.mp4");
        std::fs::write(&ok, b"x").unwrap();
        let art = shrink(&mut s, "t", &v, |_| Ok((ok.clone(), 400_000))).unwrap();
        assert!(
            (art.bytes as f64) <= raw_before as f64 * WARM_BUDGET_RATIO,
            "{} vs {raw_before}",
            art.bytes
        );
        // and the record maintains itself — the caller cannot forget
        assert_eq!(s.tier(), Tier::Warm);
        assert_eq!(s.raw_bytes, 0);
        assert_eq!(s.warm_bytes, art.bytes);

        // and an oversized one is refused with the raw samples INTACT
        let mut s2 = tmp_segment(dir.path(), 3, true, 5);
        let big = dir.path().join("big.mp4");
        std::fs::write(&big, b"x").unwrap();
        let err = shrink(&mut s2, "t", &v, |_| Ok((big.clone(), 4_000_000)));
        assert!(err.is_err());
        assert!(
            s2.raw.iter().all(|p| p.exists()),
            "an over-budget timelapse must not cost the raw samples"
        );
    }

    #[test]
    fn shrinking_an_unsummarised_window_is_refused_and_costs_it_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let mut s = tmp_segment(dir.path(), 4, false, 3);
        let mut encoded = false;
        let err = shrink(&mut s, "t", &v, |_| {
            encoded = true;
            Ok((dir.path().join("x.mp4"), 1))
        });
        assert!(err.is_err());
        assert!(!encoded, "and the encode is not even attempted");
        assert!(s.raw.iter().all(|p| p.exists()), "every sample survives");
    }

    #[test]
    fn a_failed_encode_leaves_every_raw_sample_in_place() {
        // The dangerous ordering is delete-then-encode. Nothing may be removed
        // until a verified replacement exists.
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let mut s = tmp_segment(dir.path(), 5, true, 4);
        let err = shrink(&mut s, "t", &v, |_| {
            Err(DayflowError::Retention("ffmpeg exploded".into()))
        });
        assert!(err.is_err());
        assert!(s.raw.iter().all(|p| p.exists()), "a failed encode costs nothing");
    }

    #[test]
    fn end_to_end_bytes_fall_while_every_window_stays_accounted_for() {
        // T040: summarise -> shrink -> evict, with nothing lost from the record.
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());

        let mut segments: Vec<SegmentRecord> = (1..=6)
            .map(|i| SegmentRecord {
                closed_at: at(i as i64 * 100),
                ..tmp_segment(dir.path(), i, i % 3 != 0, 5) // every third is unsummarised
            })
            .collect();

        let before: u64 = segments.iter().map(SegmentRecord::bytes).sum();
        let plan_now = at(1_000_000);
        let decisions = plan(&segments, plan_now, &cfg(1));
        assert_eq!(decisions.len(), segments.len(), "a decision for every window");

        for d in &decisions {
            if d.action != Action::Shrink {
                continue;
            }
            let i = segments.iter().position(|s| s.sequence == d.sequence).unwrap();
            let lapse = dir.path().join(format!("w{}.mp4", d.sequence));
            std::fs::write(&lapse, b"x").unwrap();
            // No manual record fix-up: `shrink` maintains it. Doing it here as
            // well would mask a regression in exactly that behaviour.
            shrink(&mut segments[i], "kept text", &v, |_| Ok((lapse.clone(), 100_000))).unwrap();
        }

        let after: u64 = segments.iter().map(SegmentRecord::bytes).sum();
        assert!(after < before, "storage actually falls: {before} -> {after}");

        // Every unsummarised window still holds every one of its samples.
        for s in segments.iter().filter(|s| !s.summarized) {
            assert_eq!(s.tier(), Tier::Hot);
            assert!(s.raw.iter().all(|p| p.exists()), "window {} lost data", s.sequence);
        }
        // And the ledger accounts for every window — asserted against a fresh
        // plan rather than against `segments.len()`, which is a local Vec that
        // was never resized and so could not have failed.
        let after_plan = plan(&segments, plan_now, &cfg(1));
        assert_eq!(after_plan.len(), 6);
        let keys: std::collections::HashSet<_> =
            after_plan.iter().map(|d| (d.display_id, d.sequence)).collect();
        assert_eq!(keys.len(), 6, "six distinct windows, none collapsed");
        assert!(after_plan.iter().all(|d| d.action != Action::Shrink), "the raw is gone");

        // Now the EVICT leg, executed against real files rather than planned.
        // T040 is "summarise -> shrink -> evict" and the evict half was
        // planner-only, so a DropWarm that failed to delete anything would have
        // passed.
        let warm_before = segments.iter().filter(|s| s.tier() == Tier::Warm).count();
        assert!(warm_before > 0, "there is something to evict");
        let mut dropped = 0;
        for d in &after_plan {
            if d.action != Action::DropWarm {
                continue;
            }
            let i = segments
                .iter()
                .position(|s| s.key() == (d.display_id, d.sequence))
                .unwrap();
            let art = segments[i].warm_artifact.take().unwrap();
            reclaim_file(&art, &v).unwrap();
            assert!(!art.exists(), "the timelapse is actually gone from disk");
            segments[i].warm_bytes = 0;
            dropped += 1;
        }
        assert!(dropped > 0, "the evict leg ran");

        let finally: u64 = segments.iter().map(SegmentRecord::bytes).sum();
        assert!(finally < after, "evicting freed more: {after} -> {finally}");

        // Through all of it, every unsummarised window kept every sample.
        for s in segments.iter().filter(|s| !s.summarized) {
            assert!(s.raw.iter().all(|p| p.exists()), "window {} lost data", s.sequence);
        }
    }

    #[test]
    fn a_timelapse_written_outside_the_capture_directory_refuses_the_shrink() {
        // The WRITE-path twin of the delete test below. The escape target is a
        // directory this process could genuinely write to — with no validation
        // the shrink would complete, install the outside path in the record,
        // and delete the raw samples. So completing-vs-refusing is the CODE
        // deciding, not the OS.
        let dir = tempfile::tempdir().unwrap();
        let v = crate::security::path_validator::PathValidator::new(dir.path());
        let mut s = tmp_segment(dir.path(), 1, true, 3);

        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("escaped.mp4");
        std::fs::write(&outside, b"x").unwrap();

        let err = shrink(&mut s, "t", &v, |_| Ok((outside.clone(), 1_000)));
        assert!(err.is_err(), "a timelapse outside the capture dir must refuse the shrink");
        assert!(
            err.unwrap_err().to_string().contains("refusing timelapse"),
            "refused as an escape, not as some later failure"
        );
        assert!(s.raw.iter().all(|p| p.exists()), "the raw samples are untouched");
        assert_eq!(s.warm_artifact, None, "the record never points outside the tree");
        assert!(outside.exists(), "and the stray file is left, never deleted outside the root");
    }

    #[test]
    fn deleting_outside_the_capture_directory_is_refused() {
        // The paths come from a database other processes write, so "it came
        // from our own records" is not a safety argument.
        let dir = tempfile::tempdir().unwrap();
        let validator = crate::security::path_validator::PathValidator::new(dir.path());

        let inside = dir.path().join("sample.png");
        std::fs::write(&inside, b"x").unwrap();
        reclaim_file(&inside, &validator).expect("a file inside the capture dir is reclaimable");
        assert!(!inside.exists());

        // The file outside must be one the OS would happily delete, in a
        // directory this process owns. An earlier version pointed at
        // /etc/passwd, which fails with permission-denied whether or not the
        // validator runs — so the assertion passed while proving only that the
        // KERNEL refused. Removing the validation entirely survived the test.
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("someone-elses-file.png");
        std::fs::write(&outside, b"not ours").unwrap();

        let err = reclaim_file(&outside, &validator);
        assert!(err.is_err(), "a path outside the capture dir must be refused");
        assert!(outside.exists(), "and left alone, though we could have deleted it");

        // and traversal back out of the capture dir is refused too
        let traversal = dir.path().join("../escape.png");
        assert!(reclaim_file(&traversal, &validator).is_err());
    }

    #[test]
    fn reclaiming_an_already_deleted_file_is_not_an_error() {
        // A retry after a partial sweep must not abort the rest of it: already
        // gone is the goal state.
        let dir = tempfile::tempdir().unwrap();
        let validator = crate::security::path_validator::PathValidator::new(dir.path());
        let gone = dir.path().join("never-existed.png");
        assert!(reclaim_file(&gone, &validator).is_ok());
    }
}
