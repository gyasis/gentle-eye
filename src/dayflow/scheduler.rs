//! Real-time scheduling: a window closes, it gets summarised, an entry appears.
//!
//! The requirement this exists for is FR-014 — entries appear **as the day
//! proceeds**, not in a batch after the recording stops. A user asking "what was
//! I doing at 2pm" at 2:30pm must get an answer.
//!
//! # Retry, never drop
//!
//! A window whose summarisation fails is **not** discarded and **not** marked
//! summarised. It goes back on the queue. The reason is asymmetric cost: a retry
//! costs one more perception call, while dropping loses a slice of the day that
//! cannot be recaptured — and would leave a hole nothing later distinguishes
//! from a period of genuine inactivity.
//!
//! This also guards the eviction rule (FR-025): retention only reclaims windows
//! flagged summarised, so a backend outage must never flag one it did not
//! actually summarise.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use crate::dayflow::models::{ChunkSummary, RollingContext, TimelineEntry};
use crate::dayflow::window::ClosedWindow;

/// A window waiting to be summarised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWindow {
    /// The window itself.
    pub window: ClosedWindow,
    /// How many summarisation attempts have been made.
    pub attempts: u32,
    /// When the next attempt is allowed, if the last one failed.
    pub retry_after: Option<DateTime<Utc>>,
}

/// Why a window left the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Summarised and written to the timeline.
    Summarised,
    /// Contained no samples, so there was nothing to summarise.
    Empty,
}

/// The real-time summarisation queue.
///
/// Deliberately holds no provider, no store and no clock: it decides WHAT should
/// be summarised next and WHEN, and the caller performs the work. That keeps the
/// ordering and retry rules — the parts that are easy to get subtly wrong —
/// testable without a model or a database.
#[derive(Debug, Default)]
pub struct SummaryScheduler {
    queue: VecDeque<PendingWindow>,
    context: RollingContext,
    settled: u64,
    failures: u64,
}

impl SummaryScheduler {
    /// An empty scheduler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a freshly closed window.
    ///
    /// A window with no samples is queued too — it is settled immediately by
    /// [`SummaryScheduler::next_due`] rather than silently dropped, so the count
    /// of what happened to every window stays complete.
    pub fn enqueue(&mut self, window: ClosedWindow) {
        self.queue.push_back(PendingWindow { window, attempts: 0, retry_after: None });
    }

    /// How many windows are waiting.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Windows that have left the queue.
    pub fn settled_count(&self) -> u64 {
        self.settled
    }

    /// Summarisation attempts that failed.
    pub fn failure_count(&self) -> u64 {
        self.failures
    }

    /// The rolling context threaded into the next summary.
    pub fn context(&self) -> &RollingContext {
        &self.context
    }

    /// Take the next window due for an attempt at `now`, or `None`.
    ///
    /// Windows whose backoff has not elapsed are skipped without losing their
    /// place: order is preserved so the timeline is built in time order even
    /// when one window is being retried.
    pub fn next_due(&mut self, now: DateTime<Utc>) -> Option<PendingWindow> {
        let idx = self
            .queue
            .iter()
            .position(|p| p.retry_after.is_none_or(|t| now >= t))?;
        let mut p = self.queue.remove(idx)?;
        p.attempts += 1;
        Some(p)
    }

    /// Record that a window was summarised, advancing the rolling context.
    pub fn succeeded(&mut self, summary: &ChunkSummary) {
        self.context = crate::dayflow::summarizer::advance_context(&self.context, summary);
        self.settled += 1;
    }

    /// Record that a window contained nothing to summarise.
    pub fn settled_empty(&mut self) {
        self.settled += 1;
    }

    /// Record a failed attempt and requeue the window with backoff.
    ///
    /// The window returns to the FRONT so time order is preserved: appending it
    /// would let later windows overtake it and produce a timeline whose entries
    /// arrive out of order.
    pub fn failed(&mut self, mut pending: PendingWindow, now: DateTime<Utc>) {
        self.failures += 1;
        let backoff = Self::backoff_for(pending.attempts);
        pending.retry_after = Some(now + backoff);
        self.queue.push_front(pending);
    }

    /// Exponential backoff, capped so a long outage does not push a retry past
    /// the end of the day.
    pub fn backoff_for(attempts: u32) -> chrono::Duration {
        let secs = 30_i64.saturating_mul(1_i64 << attempts.min(6));
        chrono::Duration::seconds(secs.min(1_800))
    }
}

/// Build a timeline entry from a summary and the window it came from.
///
/// The entry's time range is the WINDOW's real span, never the configured
/// interval: windows genuinely differ in length, and an entry claiming a
/// duration it did not have corrupts every later aggregation.
pub fn entry_from(
    recording_id: uuid::Uuid,
    window: &ClosedWindow,
    summary: &ChunkSummary,
) -> TimelineEntry {
    TimelineEntry {
        id: uuid::Uuid::new_v4(),
        recording_id,
        start_time: window.start_wall,
        end_time: window.end_wall,
        category: summary.category,
        app: summary.app.clone(),
        activity: summary.activity.clone(),
        summary: summary.detail.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dayflow::models::ActivityCategory;
    use crate::dayflow::window::CloseReason;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    fn win(seq: u64, from: i64, to: i64, samples: u32) -> ClosedWindow {
        ClosedWindow {
            display_id: 0,
            sequence: seq,
            start_wall: at(from),
            end_wall: at(to),
            sample_count: samples,
            last_sample_at: (samples > 0).then(|| at(to - 1)),
            reason: CloseReason::Boundary,
        }
    }

    fn summary(idx: usize) -> ChunkSummary {
        ChunkSummary {
            chunk_index: idx,
            start_time: at(0),
            end_time: at(600),
            category: ActivityCategory::Coding,
            app: "editor".into(),
            activity: format!("activity {idx}"),
            detail: format!("detail {idx}"),
        }
    }

    #[test]
    fn a_closed_window_becomes_due_immediately() {
        // FR-014: entries appear as the day proceeds, not after it.
        let mut s = SummaryScheduler::new();
        s.enqueue(win(0, 0, 600, 5));
        assert_eq!(s.pending(), 1);
        let due = s.next_due(at(600)).expect("due the moment it closes");
        assert_eq!(due.window.sequence, 0);
        assert_eq!(due.attempts, 1);
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn a_failed_summary_is_retried_and_never_dropped() {
        // The window must NOT be lost and must NOT be flagged summarised — a
        // backend outage would otherwise become permanent data loss, because
        // eviction reclaims anything flagged summarised (FR-025).
        let mut s = SummaryScheduler::new();
        s.enqueue(win(0, 0, 600, 5));

        let p = s.next_due(at(600)).unwrap();
        s.failed(p, at(600));
        assert_eq!(s.pending(), 1, "it went back on the queue");
        assert_eq!(s.settled_count(), 0, "and was NOT counted as done");
        assert_eq!(s.failure_count(), 1);

        // not due immediately — backoff applies
        assert!(s.next_due(at(600)).is_none(), "backoff must be honoured");
        let retried = s.next_due(at(100_000)).expect("due once backoff elapses");
        assert_eq!(retried.attempts, 2, "the attempt count carries across retries");
    }

    #[test]
    fn a_retried_window_keeps_its_place_in_time_order() {
        // Appending a failure would let later windows overtake it, producing a
        // timeline whose entries arrive out of order.
        let mut s = SummaryScheduler::new();
        s.enqueue(win(0, 0, 600, 5));
        s.enqueue(win(1, 600, 1200, 5));

        let first = s.next_due(at(600)).unwrap();
        assert_eq!(first.window.sequence, 0);
        s.failed(first, at(600));

        // window 1 is due now; window 0 is backing off (60s from the failure at
        // 600, so probe BEFORE 660 or window 0 legitimately comes back first)
        let next = s.next_due(at(620)).expect("the queue is not blocked by a backoff");
        assert_eq!(next.window.sequence, 1, "a backing-off window must not stall the rest");

        // once its backoff elapses, window 0 comes back
        let retried = s.next_due(at(100_000)).expect("window 0 returns");
        assert_eq!(retried.window.sequence, 0);
    }

    #[test]
    fn backoff_grows_but_is_capped() {
        let a = SummaryScheduler::backoff_for(1);
        let b = SummaryScheduler::backoff_for(3);
        assert!(b > a, "backoff must grow with attempts");
        let far = SummaryScheduler::backoff_for(30);
        assert!(
            far <= chrono::Duration::minutes(30),
            "an uncapped backoff would push a retry past the end of the day: {far:?}"
        );
        assert!(far >= a);
    }

    #[test]
    fn rolling_context_threads_from_one_window_to_the_next() {
        // D1: each summary sees what came before, so activity spanning a
        // boundary reads as continuous rather than as two unrelated entries.
        let mut s = SummaryScheduler::new();
        assert!(s.context().is_empty(), "nothing to carry at the start");
        s.succeeded(&summary(0));
        let after_first = s.context().summary.clone();
        assert!(!after_first.is_empty(), "the first summary seeds the context");
        s.succeeded(&summary(1));
        assert_ne!(s.context().summary, after_first, "and the second advances it");
    }

    #[test]
    fn a_failed_attempt_does_not_advance_the_rolling_context() {
        // Threading context from a summary that never happened would put
        // invented continuity into the next window's prompt.
        let mut s = SummaryScheduler::new();
        s.enqueue(win(0, 0, 600, 5));
        let p = s.next_due(at(600)).unwrap();
        s.failed(p, at(600));
        assert!(s.context().is_empty(), "a failure must not advance the narrative");
    }

    #[test]
    fn an_entry_carries_the_windows_real_span_not_the_configured_interval() {
        // A window truncated by a pause is 100s long; an entry claiming 600s
        // corrupts every later aggregation.
        let w = win(3, 1_000, 1_100, 2);
        let e = entry_from(uuid::Uuid::new_v4(), &w, &summary(3));
        assert_eq!(e.start_time, at(1_000));
        assert_eq!(e.end_time, at(1_100));
        assert_eq!(
            (e.end_time - e.start_time).num_seconds(),
            100,
            "the entry's span is the WINDOW's, not the interval's"
        );
    }

    #[test]
    fn an_empty_window_settles_without_being_summarised() {
        // A window with no samples has nothing to describe. It must still be
        // accounted for, or the count of what happened to each window is wrong.
        let mut s = SummaryScheduler::new();
        s.enqueue(win(0, 0, 600, 0));
        let due = s.next_due(at(600)).unwrap();
        assert_eq!(due.window.sample_count, 0);
        s.settled_empty();
        assert_eq!(s.settled_count(), 1);
        assert_eq!(s.pending(), 0);
        assert!(s.context().is_empty(), "an empty window contributes no narrative");
    }

    #[test]
    fn the_queue_drains_in_time_order() {
        let mut s = SummaryScheduler::new();
        for i in 0..5u64 {
            let from = (i as i64) * 600;
            s.enqueue(win(i, from, from + 600, 3));
        }
        let mut seen = Vec::new();
        while let Some(p) = s.next_due(at(10_000)) {
            seen.push(p.window.sequence);
            s.succeeded(&summary(p.window.sequence as usize));
        }
        assert_eq!(seen, vec![0, 1, 2, 3, 4], "entries must be built in time order");
        assert_eq!(s.settled_count(), 5);
    }
}
