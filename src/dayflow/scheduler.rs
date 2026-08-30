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

use crate::dayflow::models::EntryProvenance;
use crate::dayflow::models::{ChunkSummary, RollingContext, TimelineEntry};
use crate::dayflow::window::ClosedWindow;

/// A window waiting to be summarised.
///
/// Dropping one taken from [`SummaryScheduler::next_due`] without calling
/// [`SummaryScheduler::succeeded`] or [`SummaryScheduler::failed`] loses that
/// slice of the day — exactly what "retry, never drop" forbids. NOT enforced:
/// `next_due` returns an `Option`, which std already marks `#[must_use]`, and
/// the real hazard is a caller that BINDS the window and then early-returns,
/// which no attribute can see. Closing it properly needs a drop-guard or an
/// in-flight slot that `succeeded`/`failed` clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWindow {
    /// The window itself.
    pub window: ClosedWindow,
    /// How many summarisation attempts have been made.
    pub attempts: u32,
    /// When the next attempt is allowed, if the last one failed.
    pub retry_after: Option<DateTime<Utc>>,
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
    /// Windows are inserted in TIME order, not arrival order. They do not arrive
    /// sorted: sequence counters are per display, and a window closes on its END
    /// while the queue is keyed by its START — so a short pause-truncated window
    /// on one display can close before a longer window that began earlier on
    /// another. A blind `push_back` therefore hands windows out in the wrong
    /// order with no failure involved at all, and [`SummaryScheduler::failed`]'s
    /// sorted reinsert cannot repair an order `enqueue` never established.
    pub fn enqueue(&mut self, window: ClosedWindow) {
        self.insert_in_time_order(PendingWindow { window, attempts: 0, retry_after: None });
    }

    /// Insert at the window's position in time. Single seam, so the queue has
    /// exactly one ordering rule.
    fn insert_in_time_order(&mut self, pending: PendingWindow) {
        let key = Self::order_key(&pending);
        let idx = self
            .queue
            .iter()
            .position(|p| Self::order_key(p) > key)
            .unwrap_or(self.queue.len());
        self.queue.insert(idx, pending);
    }

    /// `display_id` is part of the key: sequences are per display, so two
    /// displays' first windows both carry sequence 0 and would otherwise tie,
    /// making their relative order arbitrary.
    fn order_key(p: &PendingWindow) -> (DateTime<Utc>, u32, u64) {
        (p.window.start_wall, p.window.display_id, p.window.sequence)
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
    /// A window with no samples is settled here and never handed out: there is
    /// nothing to summarise, and passing it to a caller would spend a perception
    /// call per idle window.
    pub fn next_due(&mut self, now: DateTime<Utc>) -> Option<PendingWindow> {
        loop {
            let idx = self
                .queue
                .iter()
                .position(|p| p.retry_after.is_none_or(|t| now >= t))?;
            let mut p = self.queue.remove(idx)?;
            if p.window.sample_count == 0 {
                self.settled += 1;
                continue;
            }
            p.attempts += 1;
            return Some(p);
        }
    }

    /// Record that a window was summarised, advancing the rolling context.
    pub fn succeeded(&mut self, summary: &ChunkSummary) {
        self.context = crate::dayflow::summarizer::advance_context(&self.context, summary);
        self.settled += 1;
    }

    /// Record a failed attempt and requeue the window with backoff.
    ///
    /// The window is reinserted at its position in TIME, not at the front. A
    /// front-push is correct only for a window that came from the head; one
    /// taken from deeper in the queue jumps ahead of its own predecessors, and
    /// the inversion compounds with each failure. During an outage spanning
    /// several windows that rebuilds the timeline backwards and threads the
    /// rolling context in reverse, breaking FR-016.
    pub fn failed(&mut self, mut pending: PendingWindow, now: DateTime<Utc>) {
        self.failures += 1;
        let backoff = Self::backoff_for(pending.attempts);
        pending.retry_after = Some(now + backoff);
        self.insert_in_time_order(pending);
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
        // Provenance is attached by the caller that HAS the regions — this
        // function sees only a summary and a window. Left None rather than
        // guessed: an invented layout is indistinguishable from a measured one.
        provenance: None,
    }
}

/// The provenance of a window's text: the region it principally came from.
///
/// # Why ONE region and not all of them
///
/// `TimelineEntry::provenance` is a single `Option<EntryProvenance>` and the
/// timeline's columns are single-valued, so an entry carries one region's
/// identity — not the whole layout. The one chosen is **rank 0 in the
/// deterministic reading order**, which is the region a reader's eye reaches
/// first and the one the summary leads with.
///
/// The layout is not lost: `parent_region_id` carries the cascade edge, so a
/// stored row can be walked back up to the window it sat in, and
/// `reading_order` says where it sat among its siblings.
///
/// Returns `None` for an empty region set — a window that was read whole has no
/// region to attribute to, and inventing one would make a whole-frame read look
/// like a measured layout.
pub fn provenance_from_regions(regions: &[crate::regions::Region]) -> Option<EntryProvenance> {
    let order = crate::regions::reading_order(regions);
    let first = *order.first()?;
    let r = regions.get(first)?;
    Some(EntryProvenance {
        region_id: r.identity(),
        bbox_x: r.bbox.x,
        bbox_y: r.bbox.y,
        bbox_w: r.bbox.w,
        bbox_h: r.bbox.h,
        parent_region_id: r
            .parent
            .and_then(|i| regions.get(i as usize))
            .map(crate::regions::Region::identity),
        display_id: r.display_id,
        // Rank within THIS capture's reading order. Zero by construction here;
        // carried explicitly so a reader of the stored row does not have to
        // know that, and so the field means the same thing everywhere.
        reading_order: 0,
    })
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
            clock_anomaly: false,
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
    fn several_windows_failing_in_one_outage_still_drain_in_time_order() {
        // The single-window test above cannot distinguish a front-push from a
        // time-ordered reinsert. With three windows down in one outage a
        // front-push drains them [2,1,0] — the timeline is rebuilt BACKWARDS and
        // the rolling context threads in reverse, so every prompt in that
        // stretch carries the wrong predecessor (FR-016).
        let mut s = SummaryScheduler::new();
        for i in 0..3u64 {
            let from = (i as i64) * 600;
            s.enqueue(win(i, from, from + 600, 4));
        }

        // provider is down: each window is taken and fails, in time order
        for _ in 0..3 {
            let p = s.next_due(at(1_800)).expect("all three are due");
            s.failed(p, at(1_800));
        }
        assert_eq!(s.pending(), 3, "a failure requeues, never drops");

        // provider recovers well after every backoff has elapsed
        let mut drained = Vec::new();
        while let Some(p) = s.next_due(at(100_000)) {
            drained.push(p.window.sequence);
            s.succeeded(&summary(p.window.sequence as usize));
        }
        assert_eq!(drained, vec![0, 1, 2], "an outage must not reverse the timeline");
    }

    #[test]
    fn a_partial_outage_still_drains_in_time_order() {
        // The all-fail test above cannot tell a sorted reinsert from a plain
        // push_back: when EVERY window fails once, push_back rotates the queue
        // exactly back into sorted order. Leaving one window un-failed breaks
        // that symmetry — push_back drains [2,0,1].
        let mut s = SummaryScheduler::new();
        for i in 0..3u64 {
            let from = (i as i64) * 600;
            s.enqueue(win(i, from, from + 600, 4));
        }

        for _ in 0..2 {
            let p = s.next_due(at(1_800)).expect("windows 0 and 1 are due");
            s.failed(p, at(1_800));
        }
        // window 2 is left pending, never attempted

        let mut drained = Vec::new();
        while let Some(p) = s.next_due(at(100_000)) {
            drained.push(p.window.sequence);
            s.succeeded(&summary(p.window.sequence as usize));
        }
        assert_eq!(drained, vec![0, 1, 2], "a retried window must not land behind a fresh one");
    }

    #[test]
    fn windows_arriving_out_of_order_are_still_handed_out_in_time_order() {
        // Windows do NOT arrive sorted, with no failure involved: a window
        // closes on its END while the queue is keyed by its START, and sequence
        // counters are per display. A short pause-truncated window on display 0
        // can therefore close before a longer window that BEGAN earlier on
        // display 1.
        let mut s = SummaryScheduler::new();
        let mut late_start = win(0, 1_000, 1_100, 3);
        late_start.display_id = 0;
        let mut early_start = win(0, 900, 1_500, 3);
        early_start.display_id = 1;

        s.enqueue(late_start); // closes first...
        s.enqueue(early_start); // ...but began later

        let first = s.next_due(at(2_000)).unwrap();
        assert_eq!(
            first.window.start_wall,
            at(900),
            "the earlier-starting window must come first however it arrived"
        );
    }

    #[test]
    fn two_displays_first_windows_do_not_tie() {
        // Sequences are per display, so both displays' first window is sequence
        // 0. Without display_id in the key they compare equal and their relative
        // order is whatever the insert scan happens to do.
        let mut s = SummaryScheduler::new();
        let mut b = win(0, 600, 1_200, 2);
        b.display_id = 1;
        let mut a = win(0, 600, 1_200, 2);
        a.display_id = 0;
        s.enqueue(b);
        s.enqueue(a);

        let one = s.next_due(at(2_000)).unwrap();
        let two = s.next_due(at(2_000)).unwrap();
        assert_eq!(
            (one.window.display_id, two.window.display_id),
            (0, 1),
            "identical start times must break ties deterministically by display"
        );
    }

    #[test]
    fn backoff_grows_but_is_capped() {
        // "grows and is capped" is satisfied by a LINEAR schedule too, which
        // hammers a down provider far harder than documented. Pin the doubling.
        assert_eq!(SummaryScheduler::backoff_for(0), chrono::Duration::seconds(30));
        assert_eq!(SummaryScheduler::backoff_for(1), chrono::Duration::seconds(60));
        assert_eq!(SummaryScheduler::backoff_for(2), chrono::Duration::seconds(120));
        assert_eq!(SummaryScheduler::backoff_for(3), chrono::Duration::seconds(240));
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
        // The scheduler must do this ITSELF. The previous version of this test
        // called settled_empty() by hand and so verified only its own
        // choreography: every empty-window behaviour could be deleted and it
        // still passed.
        let mut s = SummaryScheduler::new();
        s.enqueue(win(0, 0, 600, 0));
        s.enqueue(win(1, 600, 1200, 4));

        let due = s.next_due(at(1_200)).expect("the window with samples is due");
        assert_eq!(
            due.window.sequence, 1,
            "an empty window must never be handed out: summarising it spends a \
             perception call to describe nothing"
        );
        assert_eq!(s.settled_count(), 1, "but it is still accounted for");
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
