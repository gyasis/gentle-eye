//! The capture driver — the thing that actually runs Dayflow.
//!
//! Every open limitation in `docs/DAYFLOW_LIMITATIONS.md` traced to this file
//! not existing: the pipeline was complete and nothing drove it.
//!
//! # What this owns, and what it does not
//!
//! The loop owns SEQUENCING and TIMING — when to take a frame, in what order,
//! what to do when one fails, when a closed window enters the summary queue. It
//! owns **no policy**: windowing belongs to [`DayflowRun`], gating to
//! [`Sampler`], retry order to [`SummaryScheduler`], budget refusal to the
//! perception router. Re-implementing any of those here would let the loop
//! bypass the component whose tests claim to cover it (013/R29).
//!
//! # The clock is a parameter
//!
//! No `Utc::now()` appears in any decision path. A rule with the clock inside
//! the function is undefended by construction: the state that matters is hours
//! of wall-clock away and no test can reach it (013/R36).
//!
//! # Module name
//!
//! `loop` is a Rust keyword, so `src/dayflow/loop.rs` would need `mod r#loop`
//! and `dayflow::r#loop::` at every call site. Named `capture_loop` instead.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use chrono::Utc;

use crate::dayflow::engine::DayflowRun;
use crate::dayflow::sampler::{DropReason, SampleDrop, SampleRecord, SampleRequest, Sampler};
use crate::dayflow::models::{ChunkRef, TimelineEntry};
use crate::dayflow::scheduler::SummaryScheduler;
use crate::dayflow::summarizer::ChunkSummarizer;
use crate::dayflow::source::{Availability, CaptureSource, SourceIdentity};
use crate::dayflow::window::ClosedWindow;
use crate::regions::Region;

/// One source's result for one tick.
#[derive(Debug, Clone)]
pub struct SourceTick {
    /// The source's ordinal — the `display_id` position on samples.
    pub ordinal: u32,
    /// The sample, when a frame was obtained and offered to the gate.
    pub record: Option<SampleRecord>,
    /// The regions the source reported for this frame.
    ///
    /// `None` means the source had no cascade to ask, and the segment will read
    /// the whole frame. Carried out of the tick so the caller can write the
    /// sidecar (T010) and count the whole-frame read; a loop that discarded
    /// this would make the degradation invisible, which is the FR-103 failure.
    pub regions: Option<Vec<Region>>,
    /// Set when the frame could not be obtained.
    pub failure: Option<Availability>,
}

/// What one tick did.
#[derive(Debug, Clone, Default)]
pub struct TickOutcome {
    /// Per-source results, in source order.
    pub sources: Vec<SourceTick>,
    /// Windows closed by this tick's samples.
    pub closed: Vec<ClosedWindow>,
    /// Sources that ended and will not be asked again.
    pub retired: Vec<u32>,
}

impl TickOutcome {
    /// How many frames were obtained and offered to the gate.
    pub fn frames_taken(&self) -> usize {
        self.sources.iter().filter(|s| s.record.is_some()).count()
    }

    /// How many sources read the whole frame because no cascade answered.
    pub fn whole_frame_reads(&self) -> usize {
        self.sources
            .iter()
            .filter(|s| s.record.is_some() && s.regions.is_none())
            .count()
    }
}

/// Drives capture sources on a cadence and feeds the pipeline.
pub struct CaptureLoop {
    sources: Vec<Box<dyn CaptureSource>>,
    sampler: Sampler,
    scheduler: SummaryScheduler,
    sample_dir: PathBuf,
    max_attempts: u32,
    retired: HashSet<u32>,
    /// Samples that will be read as a WHOLE FRAME because no usable sidecar
    /// reached disk beside them.
    ///
    /// Counted HERE, at capture, for IMMEDIACY: `status` must show the
    /// degradation while the session runs, and summarisation happens minutes
    /// later (or never, if no window has settled yet) — a count taken only at
    /// read time is zero for exactly the period an operator is looking.
    /// NOT because capture sees more: a failed sidecar write leaves NO file,
    /// so the ladder's read-time count catches it too, and the ladder alone
    /// can see a sidecar that corrupted after a successful write.
    ///
    /// Shared so the service can report it while the thread owns the loop.
    read_whole: Arc<AtomicU64>,
    /// Samples written per window, keyed by the DURABLE identity
    /// `(ordinal, sequence)` — never the filename (013/R34).
    ///
    /// Retention needs to know what a window's bytes actually are, and only the
    /// loop sees a sample at the moment it is written. Rebuilding this by
    /// listing the directory later would guess at which window a file belonged
    /// to from its name, which is the identity mistake this key exists to
    /// prevent.
    samples: std::collections::HashMap<(u32, u64), Vec<PathBuf>>,
    /// Windows whose summary landed, so retention may touch them.
    summarized: HashSet<(u32, u64)>,
    /// Closed windows awaiting or past summarisation.
    closed: Vec<ClosedWindow>,
}

impl CaptureLoop {
    /// A loop over `sources`, writing samples under `sample_dir`.
    pub fn new(
        sources: Vec<Box<dyn CaptureSource>>,
        sampler: Sampler,
        sample_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            sources,
            sampler,
            scheduler: SummaryScheduler::new(),
            sample_dir: sample_dir.into(),
            max_attempts: 2,
            retired: HashSet::new(),
            read_whole: Arc::new(AtomicU64::new(0)),
            samples: std::collections::HashMap::new(),
            summarized: HashSet::new(),
            closed: Vec::new(),
        }
    }

    /// Use a caller-owned whole-frame-read counter instead of a private one.
    ///
    /// Needed because the loop runs on the capture thread while `status()` is
    /// answered on another: a private counter would read zero to every surface.
    pub fn with_read_whole_counter(mut self, counter: Arc<AtomicU64>) -> Self {
        self.read_whole = counter;
        self
    }

    /// Every source's identity, ordinal and CURRENT availability.
    ///
    /// A retired source is still described — a session that lost its window
    /// must be able to say so, and dropping it from the list would make a dead
    /// source indistinguishable from one that was never configured.
    pub fn describe(&self) -> Vec<(SourceIdentity, u32, Availability)> {
        self.sources
            .iter()
            .map(|s| {
                let ordinal = s.ordinal();
                let a = if self.retired.contains(&ordinal) {
                    Availability::Ended
                } else {
                    s.availability()
                };
                (s.identity(), ordinal, a)
            })
            .collect()
    }

    /// The source ordinals still being asked.
    pub fn active_ordinals(&self) -> Vec<u32> {
        self.sources
            .iter()
            .map(|s| s.ordinal())
            .filter(|o| !self.retired.contains(o))
            .collect()
    }

    /// The summary queue, so the caller can settle due windows.
    pub fn scheduler(&mut self) -> &mut SummaryScheduler {
        &mut self.scheduler
    }

    /// How many samples will be read as whole frames.
    pub fn samples_read_whole(&self) -> u64 {
        self.read_whole.load(Ordering::SeqCst)
    }

    /// Drops recorded so far.
    pub fn drops(&self) -> &[SampleDrop] {
        self.sampler.drops()
    }

    /// Where a source's sample for this tick is written.
    pub fn sample_dir(&self) -> &Path {
        &self.sample_dir
    }

    /// Adopt samples a DEAD process left in the sample directory (T022).
    ///
    /// The loop's ownership map (`samples`/`closed`/`summarized`) is in-memory
    /// and dies with the process. Without adoption, a restarted daemon reads
    /// every pre-restart window as unknown: its samples are invisible to
    /// `segments()`, so retention can never reclaim them, and disk grows
    /// without bound across restarts — a real leak in a recorder designed to
    /// run all day, every day.
    ///
    /// Adoption reconstructs each orphaned window from the samples themselves
    /// (the naming rule is [`sample_prefix`]'s, parsed by its own inverse
    /// [`parse_sample_filename`](crate::dayflow::sampler::parse_sample_filename)
    /// so the two cannot drift), enqueues it for summarisation, and records
    /// its files — so the pre-restart tail is SUMMARISED and then reclaimed
    /// through the normal retention gate, not deleted unsummarised and not
    /// leaked.
    ///
    /// **Only call this when resuming the SAME session.** The entries a
    /// settle produces are filed under the session the loop serves; adopting
    /// another session's samples would attribute its screen to this one.
    /// Windows the loop already owns are never re-adopted.
    ///
    /// The window's walls are the first and last sample instants — the only
    /// evidence that survived. `end_wall` therefore understates the window by
    /// up to one sampling interval; honest, and marked by `CloseReason::Stopped`,
    /// which is what actually ended it.
    pub fn adopt_orphans(&mut self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.sample_dir) else {
            return 0;
        };
        type OrphanFiles = Vec<(DateTime<Utc>, PathBuf)>;
        let mut orphans: std::collections::BTreeMap<(u32, u64), OrphanFiles> =
            std::collections::BTreeMap::new();
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let Some((display, sequence, taken_at)) =
                crate::dayflow::sampler::parse_sample_filename(name)
            else {
                continue;
            };
            let key = (display, sequence);
            if self.samples.contains_key(&key) || self.summarized.contains(&key) {
                continue; // already owned by THIS loop
            }
            orphans.entry(key).or_default().push((taken_at, path));
        }
        let adopted = orphans.len();
        for ((display_id, sequence), mut files) in orphans {
            files.sort();
            let start_wall = files.first().map(|(t, _)| *t).unwrap_or_default();
            let end_wall = files.last().map(|(t, _)| *t).unwrap_or(start_wall);
            let window = ClosedWindow {
                display_id,
                sequence,
                start_wall,
                end_wall,
                sample_count: files.len() as u32,
                clock_anomaly: false,
                last_sample_at: Some(end_wall),
                reason: crate::dayflow::window::CloseReason::Stopped,
            };
            self.samples.insert(
                (display_id, sequence),
                files.into_iter().map(|(_, p)| p).collect(),
            );
            self.closed.push(window.clone());
            self.scheduler.enqueue(window);
        }
        adopted
    }

    /// Summarise every window that is due, returning the timeline entries.
    ///
    /// Called between ticks so summarisation happens DURING the session rather
    /// than only at stop (FR-025). A failure requeues the window with backoff
    /// and produces NO entry: retry-never-drop means a window is never marked
    /// summarised because the model was briefly unavailable.
    ///
    /// `now` is a parameter for the same reason `tick`'s is: the retry schedule
    /// is the thing most worth testing, and it is hours away in wall-clock.
    pub async fn settle_due(
        &mut self,
        summarizer: &dyn ChunkSummarizer,
        recording_id: uuid::Uuid,
        now: DateTime<Utc>,
    ) -> Vec<TimelineEntry> {
        let mut entries = Vec::new();
        while let Some(pending) = self.scheduler.next_due(now) {
            let key = (pending.window.display_id, pending.window.sequence);
            let chunk = ChunkRef {
                index: pending.window.sequence as usize,
                // The sample DIRECTORY, not a per-window artifact — samples are
                // loose PNGs, and the window's own files are found by the
                // (display_id, sequence) prefix, which `RoutedChunkSummarizer`
                // resolves via `sampler::sample_prefix`. A summarizer that
                // treats `path` as a single video file (`VisionChunkSummarizer`
                // does) would analyse a directory and must NOT be handed to
                // this loop.
                path: self.sample_dir.clone(),
                start_wall: pending.window.start_wall,
                end_wall: pending.window.end_wall,
                display_id: pending.window.display_id,
                sequence: pending.window.sequence,
                // Eviction is gated on `summarized`, never on age or budget
                // pressure — a chunk not yet summarised must survive.
                summarized: false,
            };
            let prior = self.scheduler.context().clone();
            match summarizer.summarize_chunk(&chunk, &prior).await {
                Ok(summary) => {
                    let mut entry = crate::dayflow::scheduler::entry_from(
                        recording_id,
                        &pending.window,
                        &summary,
                    );
                    // T019: the regions this window's text actually came from,
                    // read back from the sidecars the loop wrote beside its
                    // samples. Before this, every entry stored `provenance:
                    // NULL` — the cascade ran, the crops were read, and where
                    // the text came from was thrown away at the last step.
                    entry.provenance = self.provenance_for(&key);
                    entries.push(entry);
                    self.summarized.insert(key);
                    self.scheduler.succeeded(&summary);
                }
                Err(_) => self.scheduler.failed(pending, now),
            }
        }
        entries
    }

    /// Write a kept sample's regions beside it, as the ladder expects.
    ///
    /// Only for a sample that reached disk: a skipped or dropped frame has no
    /// path, and a sidecar with no sample would be read by nothing.
    ///
    /// `None` regions write NO file. That is the honest encoding — the ladder
    /// reads a missing sidecar as "no cascade answered" and counts the
    /// whole-frame read. Writing an empty array instead would claim the cascade
    /// ran and found nothing, which is a different fact and would hide the
    /// degradation `samples_read_whole` exists to surface (D014-3, FR-103).
    ///
    /// A write failure is logged, never raised: the sample itself is already
    /// safe on disk, and losing a segment because its sidecar could not be
    /// written would trade a degraded read for no data at all. Every gate in
    /// this pipeline fails open.
    fn write_sidecar(counter: &AtomicU64, record: &SampleRecord, regions: Option<&[Region]>) {
        let Some(path) = record.path.as_ref() else {
            // Nothing on disk: no sidecar to write, and nothing to read whole.
            return;
        };
        let Some(regions) = regions else {
            // No cascade answered. The missing file IS the honest encoding, and
            // this sample will be read whole — count it.
            counter.fetch_add(1, Ordering::SeqCst);
            return;
        };
        let target = crate::dayflow::perception::regions_path(path);
        match serde_json::to_string(regions) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&target, json) {
                    // The regions existed but did not reach disk, so the ladder
                    // will read the whole frame. Same visible outcome as no
                    // cascade, so it must land in the same counter.
                    counter.fetch_add(1, Ordering::SeqCst);
                    tracing::warn!(error = %e, path = %target.display(),
                        "could not write region sidecar; the segment will read whole frames");
                }
            }
            Err(e) => {
                counter.fetch_add(1, Ordering::SeqCst);
                tracing::warn!(error = %e, "regions could not be serialised");
            }
        }
    }

    /// The provenance for a window: the regions beside its samples.
    ///
    /// Reads the sidecars the loop wrote at capture time rather than
    /// re-detecting. Re-detecting at summarise time would describe a DIFFERENT
    /// moment than the pixels do — the screen has moved on — which is the same
    /// reason the sidecar exists at all.
    fn provenance_for(&self, key: &(u32, u64)) -> Option<crate::dayflow::models::EntryProvenance> {
        let samples = self.samples.get(key)?;
        // The FIRST sample of the window. Its regions are the arrangement the
        // window opened with; a later sample may describe a layout the summary
        // does not lead with.
        let first = samples.first()?;
        let side = crate::dayflow::perception::regions_path(first);
        let raw = std::fs::read_to_string(side).ok()?;
        let regions: Vec<crate::regions::Region> = serde_json::from_str(&raw).ok()?;
        crate::dayflow::scheduler::provenance_from_regions(&regions)
    }

    /// The retention view of every window this loop has closed.
    ///
    /// `summarized` is taken from what actually SETTLED, never from age or
    /// from the window merely having closed — eviction is gated on a summary
    /// existing, and that gate is the whole of retention's safety.
    pub fn segments(&self) -> Vec<crate::dayflow::retention::SegmentRecord> {
        self.closed
            .iter()
            .map(|w| {
                let key = (w.display_id, w.sequence);
                let raw: Vec<PathBuf> = self.samples.get(&key).cloned().unwrap_or_default();
                let raw_bytes = raw
                    .iter()
                    .filter_map(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum();
                crate::dayflow::retention::SegmentRecord {
                    sequence: w.sequence,
                    display_id: w.display_id,
                    closed_at: w.end_wall,
                    summarized: self.summarized.contains(&key),
                    raw,
                    warm_artifact: None,
                    raw_bytes,
                    warm_bytes: 0,
                }
            })
            .collect()
    }

    /// Run retention on the session's own segments and execute the plan.
    ///
    /// Returns the decisions, INCLUDING the ones that left a segment alone and
    /// why: a sweep that reported only its actions would make "nothing needed
    /// reclaiming" and "everything was refused" look identical, which is
    /// exactly the question asked when the disk fills up anyway.
    pub fn sweep_retention(
        &mut self,
        cfg: &crate::dayflow::retention::RetentionConfig,
        now: DateTime<Utc>,
    ) -> Vec<crate::dayflow::retention::Decision> {
        use crate::dayflow::retention::Action;
        let segments = self.segments();
        let decisions = crate::dayflow::retention::plan(&segments, now, cfg);
        // Scoped to the sample directory, deliberately. `reclaim_file` refuses
        // any path outside its roots, so a corrupted or hostile record cannot
        // turn a retention sweep into deleting arbitrary files.
        let validator =
            crate::security::path_validator::PathValidator::new(self.sample_dir.clone());
        for d in &decisions {
            let key = (d.display_id, d.sequence);
            match d.action {
                Action::Shrink => {
                    if let Some(seg) = segments.iter().find(|s| s.key() == key) {
                        for f in &seg.raw {
                            // Best-effort: a file that cannot be reclaimed is
                            // logged, never fatal. Losing the session because
                            // one sample would not delete would trade a full
                            // disk for no recording at all.
                            if let Err(e) = crate::dayflow::retention::reclaim_file(f, &validator) {
                                tracing::warn!(error = %e, "could not reclaim a sample");
                            }
                            // The region sidecar the loop wrote BESIDE this
                            // sample (T010). It describes pixels that no longer
                            // exist once the sample is gone, and nothing else
                            // ever deletes it — leaving it would grow one
                            // orphan JSON per reclaimed sample for the life of
                            // the directory. `reclaim_file` treats
                            // already-absent as success, so samples that never
                            // had a sidecar (whole-frame reads) cost nothing.
                            let side = crate::dayflow::perception::regions_path(f);
                            if let Err(e) = crate::dayflow::retention::reclaim_file(&side, &validator) {
                                tracing::warn!(error = %e, "could not reclaim a region sidecar");
                            }
                        }
                        self.samples.remove(&key);
                    }
                }
                // The warm artifact is produced by `shrink`, which this loop
                // does not run yet — nothing to drop.
                Action::DropWarm | Action::Keep(_) => {}
            }
        }
        decisions
    }

    /// Take one frame from every live source and advance the pipeline.
    ///
    /// Failure of one source never ends the tick: the others are still asked,
    /// and the failure is recorded as a per-source drop. A drop is not a gap —
    /// a gap claims capture stopped, which is false while another source is
    /// still producing (D014-9).
    pub fn tick(&mut self, run: &mut DayflowRun, now: DateTime<Utc>) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let read_whole = Arc::clone(&self.read_whole);
        let mut kept_samples: Vec<((u32, u64), PathBuf)> = Vec::new();

        for source in &mut self.sources {
            let ordinal = source.ordinal();
            if self.retired.contains(&ordinal) {
                continue;
            }

            // BEFORE on_sample: this sample belongs to the window that is open
            // now, and on_sample may close it.
            let sequence = run.current_sequence(ordinal);

            match source.next_frame() {
                Ok(frame) => {
                    // A frame arrived: lift a SOURCE pause BEFORE recording it
                    // — `on_sample` is a no-op on a paused run, so resuming
                    // after the loop would silently uncount the first frame of
                    // every recovery. Lifts ONLY `SourceOccluded`: a frame
                    // says nothing about the user's idle/lock state.
                    run.sources_recovered(now);
                    let regions = source.regions_for(&frame);
                    // No re-acquire closure: the source is already borrowed
                    // for this frame. A source that can retry does so behind
                    // `next_frame`; the sampler's own re-acquire path stays
                    // available to callers that own the capturer directly.
                    let observed = self.sampler.observe_with_reacquire(
                        SampleRequest {
                            display_id: ordinal,
                            sequence,
                            frame: frame.as_raw(),
                            taken_at: now,
                            dir: &self.sample_dir,
                            max_attempts: self.max_attempts,
                        },
                        |_| None,
                    );
                    match observed {
                        Ok(record) => {
                            // T010: write the region sidecar BESIDE the sample
                            // the gate kept. The ladder has consumed this file
                            // since 013 and nothing ever produced it — and
                            // because that read fails open, its absence was
                            // invisible: whole-frame reads, every test green,
                            // crop-before-extract entirely absent.
                            Self::write_sidecar(&read_whole, &record, regions.as_deref());
                            if let Some(path) = record.path.clone() {
                                kept_samples.push(((ordinal, sequence), path));
                            }
                            if let Some(closed) = run.on_sample(ordinal, now) {
                                outcome.closed.push(closed);
                            }
                            outcome.sources.push(SourceTick {
                                ordinal,
                                record: Some(record),
                                regions,
                                failure: None,
                            });
                        }
                        Err(_) => {
                            // The gate failed open upstream; the frame was
                            // wanted and not kept, which is a drop, not a skip.
                            self.sampler.record_drop(SampleDrop {
                                display_id: ordinal,
                                sequence,
                                at: now,
                                reason: DropReason::WriteFailed,
                                attempts: self.max_attempts,
                                recovered: false,
                            });
                            outcome.sources.push(SourceTick {
                                ordinal,
                                record: None,
                                regions,
                                failure: None,
                            });
                        }
                    }
                }
                Err(_) => {
                    let availability = source.availability();
                    self.sampler.record_drop(SampleDrop {
                        display_id: ordinal,
                        sequence,
                        at: now,
                        reason: DropReason::SourceUnavailable,
                        attempts: 1,
                        recovered: false,
                    });
                    if !availability.retryable() {
                        self.retired.insert(ordinal);
                        outcome.retired.push(ordinal);
                    }
                    outcome.sources.push(SourceTick {
                        ordinal,
                        record: None,
                        regions: None,
                        failure: Some(availability),
                    });
                }
            }
        }

        // The SESSION-WIDE arm of FR-113 (D014-9's second row). A drop is
        // per-source; a gap says capture stopped for the whole session — and
        // it did exactly when NO source produced a frame this tick for
        // availability reasons. One occluded source among producers is NOT a
        // gap (that would claim the session stopped while others record), but
        // for the single-source sessions this feature exists for, the watched
        // window minimising IS the session pausing — and a watched window
        // that QUIT must leave a gap saying so, not silence: `SourceEnded` is
        // what lets health read the difference between quiet-on-purpose and
        // dead (models.rs maps it to Degraded, a mapping nothing reached
        // until this wire existed).
        let any_frame = outcome.sources.iter().any(|s| s.failure.is_none());
        if !any_frame && !self.sources.is_empty() {
            let all_ended = self
                .sources
                .iter()
                .all(|s| self.retired.contains(&s.ordinal()));
            let session = if all_ended {
                // Nothing will ever produce again; `SourceEnded` is not
                // automatic, so this gap does not lift on its own — the
                // condition "clears" never happens for a retired source.
                Availability::Ended
            } else if !outcome.sources.is_empty() {
                // Every source still being asked failed this tick (any
                // produced frame would have made `any_frame` true).
                Availability::Occluded
            } else {
                // No live source was asked yet none are retired — a shape
                // that cannot occur today (every live source pushes a
                // SourceTick). Fail toward "no gap" rather than inventing
                // one: `Available` warrants none (D014-9).
                Availability::Available
            };
            if let Some(cause) = session.gap_cause() {
                outcome.closed.extend(run.sources_unavailable(cause, now));
            }
        }

        for (key, path) in kept_samples {
            self.samples.entry(key).or_default().push(path);
        }
        self.closed.extend(outcome.closed.iter().cloned());

        // Closed windows enter the queue as they close, so summarisation runs
        // DURING the session rather than only at stop (FR-025).
        for window in &outcome.closed {
            self.scheduler.enqueue(window.clone());
        }

        // The wire `sync_drops_from` exists for, actually connected: its own
        // doc warns that "a caller that samples and never syncs would report
        // zero holes while dropping frames" — which is exactly what this loop
        // did until the W4 gate. Without this line, `status` liveness reports
        // frames_dropped = 0 forever, however many intervals the sources lose.
        run.sync_drops_from(&self.sampler);

        outcome
    }
}
