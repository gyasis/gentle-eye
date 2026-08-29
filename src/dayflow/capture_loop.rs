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
use crate::dayflow::source::{Availability, CaptureSource};
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

    /// How many acquisition attempts the sampler may make per interval.
    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts;
        self
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

    /// A handle to the whole-frame-read counter, for a caller that owns the
    /// loop on another thread and must still report it.
    pub fn read_whole_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.read_whole)
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
                    entries.push(crate::dayflow::scheduler::entry_from(
                        recording_id,
                        &pending.window,
                        &summary,
                    ));
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

    /// Take one frame from every live source and advance the pipeline.
    ///
    /// Failure of one source never ends the tick: the others are still asked,
    /// and the failure is recorded as a per-source drop. A drop is not a gap —
    /// a gap claims capture stopped, which is false while another source is
    /// still producing (D014-9).
    pub fn tick(&mut self, run: &mut DayflowRun, now: DateTime<Utc>) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let read_whole = Arc::clone(&self.read_whole);

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
