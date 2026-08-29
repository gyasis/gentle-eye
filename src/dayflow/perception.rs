//! The two-tier perception ladder (US3).
//!
//! Dayflow's per-sample work is **text extraction**, which a small OCR model
//! does cheaply. Understanding — what category of activity this was, what it
//! meant — is a different and far more expensive question. Running the reasoning
//! model on every sample of an eight-hour day would cost roughly two orders of
//! magnitude more than the day is worth (research R19), so the ladder starts at
//! the text tier and escalates only when a caller explicitly asks a question the
//! text tier cannot answer.
//!
//! # The tier is the CALLER's declaration, never a guess
//!
//! [`PerceptionKind`] is supplied by the caller and dispatched on directly. The
//! router never inspects the prompt to decide which model to use. Sniffing would
//! make cost depend on incidental wording — the same request escalating or not
//! depending on whether someone wrote "read" or "explain" — and make the spend
//! unpredictable and untestable. An explicit kind is auditable: every escalation
//! has a caller who asked for it.
//!
//! # Fail-open
//!
//! Every gate here fails OPEN (research R13): on any error the sample is KEPT.
//! A gate erring toward keeping costs one wasted perception call; one erring
//! toward dropping loses a slice of the day silently, and dayflow cannot
//! re-capture yesterday.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::config::DayflowIntent;
use crate::contracts::traits::{AnalysisResult, VisionProvider};
use crate::dayflow::errors::DayflowError;
use crate::security::rate_limiter::RateLimiter;

/// Which question is being asked of the ladder.
///
/// Supplied by the caller. The router dispatches on this and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerceptionKind {
    /// "What text is on this screen?" — the per-sample default.
    Text,
    /// "What was this activity, and what did it mean?" — escalates.
    Reason,
}

/// Which tier actually served a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The cheap OCR tier.
    Text,
    /// The expensive reasoning tier.
    Reason,
}

/// One escalation to the reasoning tier, with the reason it happened (FR-007/010).
///
/// Recorded so the answer to "why was today expensive?" is data rather than
/// inference. An escalation with no reason is a bug, not a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escalation {
    /// When it happened.
    pub at: DateTime<Utc>,
    /// Why the caller needed the reasoning tier.
    pub reason: String,
    /// The tier that served it.
    pub served_by: Tier,
}

/// Rate-limit key for Dayflow's own perception traffic.
///
/// Deliberately distinct from the interactive `analyze_video` key: a day of
/// background sampling must never exhaust the bucket a user's live request
/// draws from, and a burst of live requests must never starve the timeline.
/// Two keys in one limiter would still share a capacity, so these are separate
/// limiters entirely.
pub const DAYFLOW_KEY: &str = "dayflow:perception";

/// Rate-limit key for user-initiated analysis.
pub const INTERACTIVE_KEY: &str = "interactive:analyze";

/// The window Dayflow's perception budget is measured over.
///
/// NOT one minute. Coarse all-day tracking samples every 3 minutes (D10), so a
/// per-minute budget floors at "one interval" for every interval longer than a
/// minute — making a 3-minute cadence ask for exactly as much as a 1-minute one
/// and defeating the point of deriving the number at all.
pub const BUDGET_WINDOW: std::time::Duration = std::time::Duration::from_secs(600);

/// The perception budget Dayflow's sampling needs over [`BUDGET_WINDOW`].
///
/// Derived from the sampling shape rather than picked: one call per region per
/// display per interval, doubled for retry headroom. A budget that is merely
/// "large" would let a misconfiguration bill an entire day before anything
/// noticed.
pub fn dayflow_budget(interval_secs: u64, displays: u32, max_regions_per_segment: u32) -> u32 {
    let per_interval = displays.max(1).saturating_mul(max_regions_per_segment.max(1));
    let intervals = (BUDGET_WINDOW.as_secs_f64() / interval_secs.max(1) as f64).ceil() as u32;
    per_interval.saturating_mul(intervals.max(1)).saturating_mul(2)
}

/// Routes a perception request to the tier that should answer it.
pub struct PerceptionRouter {
    text: Arc<dyn VisionProvider>,
    reason: Arc<dyn VisionProvider>,
    escalations: Mutex<Vec<Escalation>>,
    limiter: RateLimiter,
}

impl PerceptionRouter {
    /// Build a router over the two configured tiers.
    pub fn new(
        text: Arc<dyn VisionProvider>,
        reason: Arc<dyn VisionProvider>,
        budget: u32,
    ) -> Self {
        Self {
            text,
            reason,
            escalations: Mutex::new(Vec::new()),
            limiter: RateLimiter::new(f64::from(budget), BUDGET_WINDOW),
        }
    }

    /// Perceive one image at the tier the caller asked for.
    ///
    /// `reason` is required for [`PerceptionKind::Reason`] and is what lands in
    /// the escalation record.
    pub async fn perceive(
        &self,
        kind: PerceptionKind,
        image: &Path,
        prompt: &str,
        reason: &str,
    ) -> Result<(AnalysisResult, Tier), DayflowError> {
        self.limiter
            .check(DAYFLOW_KEY)
            .map_err(|e| DayflowError::Perception(format!("dayflow perception budget: {e}")))?;

        let (provider, tier) = match kind {
            PerceptionKind::Text => (&self.text, Tier::Text),
            PerceptionKind::Reason => (&self.reason, Tier::Reason),
        };

        if tier == Tier::Reason {
            if reason.trim().is_empty() {
                return Err(DayflowError::Invalid(
                    "an escalation to the reasoning tier must state its reason".into(),
                ));
            }
            self.record_escalation(reason, tier);
        }

        let result = provider
            .analyze_image(image, prompt)
            .await
            .map_err(|e| DayflowError::Perception(e.to_string()))?;
        Ok((result, tier))
    }

    fn record_escalation(&self, reason: &str, served_by: Tier) {
        let record = Escalation { at: Utc::now(), reason: reason.to_string(), served_by };
        tracing::info!(
            reason = %record.reason,
            served_by = ?served_by,
            "dayflow perception escalated to the reasoning tier"
        );
        // A poisoned lock must not lose the record: the Vec is plain data, so
        // recovering the guard is safe, and dropping an escalation degrades the
        // FR-007/010 audit trail exactly where it is most needed.
        match self.escalations.lock() {
            Ok(mut v) => v.push(record),
            Err(poisoned) => poisoned.into_inner().push(record),
        }
    }

    /// Every escalation so far, in order.
    pub fn escalations(&self) -> Vec<Escalation> {
        // `unwrap_or_default()` here would answer "nothing escalated" after a
        // single unrelated panic — a false-green answer to "why was today
        // expensive?", worse than losing one record.
        match self.escalations.lock() {
            Ok(v) => v.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

// ─── T052/T053: Content intent — rolling OCR aggregation ───────────────────

/// How many recent blocks the aggregator compares against.
///
/// videolocr's window (research R13). Bounded on purpose: an unbounded history
/// would make every sample O(day) and grow without limit across eight hours.
const HISTORY: usize = 5;

/// At or above this, a capture is the SAME material seen again — merge into it.
///
/// **Calibrated to this metric, not imported.** videolocr uses 0.85, but against
/// difflib's CHARACTER-level `SequenceMatcher`; [`similarity`] here is
/// LINE-level, and a threshold does not survive a change of metric. Measured on
/// the two cases that must be separated:
///
/// | case | line similarity | must |
/// |---|---|---|
/// | a pane scrolled 2 lines in a 10-line view (80% shared) | 0.80 | MERGE |
/// | a screen sharing half its lines with another | 0.50 | SEPARATE |
///
/// 0.65 sits between them with margin on both sides. Importing 0.85 unchanged
/// put the threshold ABOVE the legitimate scroll case, so every sample of a
/// scrolling document started a new block — the exact failure T052 exists to
/// prevent.
///
/// This is also the ONLY merge gate. videolocr's weaker "comparable" floor at
/// 0.3 was ported as a second merge arm and that was a mistake caught by
/// mutation testing: two screens sharing 30% of their lines are mostly
/// DIFFERENT, and merging them builds a document that is majority unrelated
/// material. In videolocr 0.3 gates whether two chunks are worth *diffing*, not
/// whether to fold them together.
const SAME_BLOCK: f64 = 0.65;

// T053 specifies "append a block only below 0.95 similarity". That gate is
// SUBSUMED here rather than implemented: line-level `diff_merge` adds only lines
// the block does not already have, so a near-identical capture contributes
// nothing by construction and needs no threshold to stop it. It was implemented
// as an explicit branch first; mutation testing showed removing the branch
// changed no behaviour at all, which is the definition of a constant no
// behaviour depends on. Deleted rather than wrapped in a test that would only
// have asserted the optimization was taken.

/// The longest run of consecutive lines common to `block` and `incoming`,
/// as `(index in block, index in incoming, length)`.
///
/// **Contiguity is the whole point.** A pane that scrolled shares a CONTIGUOUS
/// run with what came before. Two unrelated screens of the same application
/// share SCATTERED lines — the menu bar, the status bar, the file tree, line
/// numbers — and a set-based measure cannot tell those apart: a realistic
/// full-screen capture is mostly chrome, so any two screens of one editor score
/// high enough to merge, folding a day's separate documents into one
/// interleaved blob.
fn longest_common_run(block: &[&str], incoming: &[&str]) -> (usize, usize, usize) {
    // Classic LCSubstring DP, one row at a time.
    let (mut best_b, mut best_i, mut best_len) = (0usize, 0usize, 0usize);
    let mut prev = vec![0usize; incoming.len() + 1];
    let mut cur = vec![0usize; incoming.len() + 1];
    for (bi, b) in block.iter().enumerate() {
        for (ii, i) in incoming.iter().enumerate() {
            cur[ii + 1] = if b == i { prev[ii] + 1 } else { 0 };
            if cur[ii + 1] > best_len {
                best_len = cur[ii + 1];
                best_b = bi + 1 - best_len;
                best_i = ii + 1 - best_len;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.iter_mut().for_each(|v| *v = 0);
    }
    (best_b, best_i, best_len)
}

/// Split text into the comparable lines: trimmed, blanks dropped.
///
/// Blank lines are dropped for COMPARISON only — [`merge_scroll`] preserves the
/// original text, because in a captured document a blank line is structure.
fn lines_of(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

/// How much of `incoming` is a contiguous continuation of `block`, in `[0, 1]`.
///
/// **Asymmetric on purpose.** The obvious choice is a symmetric ratio
/// (`2·common / (len(a)+len(b))`), and it is WRONG here — a fact found by
/// running five samples instead of two. The block GROWS with every merge while
/// a capture stays one screenful, so a symmetric score decays mechanically as
/// the document accumulates even when each capture overlaps its tail perfectly:
/// 0.80, 0.73, 0.67, **0.62 → splits**. Every long document eventually
/// fragments and the threshold only decides when. The question that actually
/// matters is "is this capture material I already have?", which is a property of
/// the INCOMING text and stays stable however large the block gets.
///
/// Line-based rather than character-based because OCR of a scrolling pane
/// re-reads whole lines: character drift within a line is noise, a new line is
/// signal. That does make this brittle to OCR that perturbs many lines per
/// sample — see research.md R24.
pub fn coverage(block: &str, incoming: &str) -> f64 {
    let lb = lines_of(block);
    let li = lines_of(incoming);
    if li.is_empty() {
        return 0.0;
    }
    let (_, _, len) = longest_common_run(&lb, &li);
    len as f64 / li.len() as f64
}

/// Merge `incoming` into `block`, dropping only the contiguous run they share.
///
/// **Not a set-union.** Deduplicating by line VALUE loses every legitimately
/// repeated line — a closing brace, a blank separator, a repeated table row —
/// so for Content intent, where the merged text IS the deliverable, any source
/// file or table was silently corrupted. Only the lines proven to be the SAME
/// OCCURRENCE (the contiguous overlap) are skipped; everything else in
/// `incoming` is kept, in order.
pub fn merge_scroll(block: &str, incoming: &str) -> String {
    let bl = lines_of(block);
    let il = lines_of(incoming);
    if il.is_empty() {
        return block.to_string();
    }
    let (_, inc_start, len) = longest_common_run(&bl, &il);
    if len == 0 {
        return format!("{block}\n{incoming}");
    }
    let mut out = block.trim_end().to_string();
    // Everything before the overlap (a scroll-up) and after it (a scroll-down)
    // is new material. Appended in arrival order: putting it in document order
    // needs on-screen geometry, which is US4's job, not this function's.
    for line in il[..inc_start].iter().chain(il[inc_start + len..].iter()) {
        out.push('\n');
        out.push_str(line);
    }
    out
}

/// Rolling aggregation of OCR text across samples (Content intent only).
///
/// Under [`DayflowIntent::Activity`] this is never constructed: activity
/// tracking wants a description of what happened, not a transcript, and paying
/// to accumulate verbatim text all day would be cost for no benefit.
#[derive(Debug, Default)]
pub struct TextAggregator {
    blocks: Vec<String>,
}

impl TextAggregator {
    /// An empty aggregator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorb one capture, returning the index of the block it landed in.
    ///
    /// FAIL-OPEN (research R13): text that matches nothing becomes its own
    /// block rather than being discarded. There is no path here that drops a
    /// capture — a wrong merge costs some duplication, a drop loses the only
    /// record of that moment.
    pub fn absorb(&mut self, text: &str) -> usize {
        if text.trim().is_empty() {
            self.blocks.push(text.to_string());
            return self.blocks.len() - 1;
        }

        let start = self.blocks.len().saturating_sub(HISTORY);
        let best = self.blocks[start..]
            .iter()
            .enumerate()
            .map(|(i, b)| (start + i, coverage(b, text)))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        match best {
            // The same material seen again as the pane scrolled: grow it.
            Some((i, score)) if score >= SAME_BLOCK => {
                self.blocks[i] = merge_scroll(&self.blocks[i], text);
                i
            }
            _ => {
                self.blocks.push(text.to_string());
                self.blocks.len() - 1
            }
        }
    }

    /// The accumulated blocks.
    pub fn blocks(&self) -> &[String] {
        &self.blocks
    }
}

/// Build an aggregator only when the intent calls for one.
///
/// Returns `None` under [`DayflowIntent::Activity`] so the aggregator cannot be
/// invoked by accident: absence is a stronger guarantee than a disabled flag.
pub fn aggregator_for(intent: DayflowIntent) -> Option<TextAggregator> {
    intent.aggregates_text().then(TextAggregator::new)
}

// ─── T031: routing per-segment summarisation through the ladder ───────────

/// Summarise one segment through the perception ladder.
///
/// The text tier reads every sample; the reasoning tier is consulted **once per
/// segment**, not once per sample. That ratio is the entire cost argument: a
/// 15-minute segment sampled every 3 minutes is 5 text calls and 1 reason call,
/// where naive per-sample reasoning would be 5 reason calls. Over an eight-hour
/// day that is the difference between a feature that can run all day and one
/// that cannot.
///
/// Returns the raw reasoning-tier text; the caller parses it into a
/// [`ChunkSummary`] with the summarizer's existing parser.
pub async fn summarize_segment_via_ladder(
    router: &PerceptionRouter,
    samples: &[std::path::PathBuf],
    reason_prompt: &str,
) -> Result<String, DayflowError> {
    if samples.is_empty() {
        return Err(DayflowError::Invalid(
            "cannot summarise a segment with no samples".into(),
        ));
    }

    let mut extracted = Vec::with_capacity(samples.len());
    for path in samples {
        let (result, tier) = router
            .perceive(PerceptionKind::Text, path, "Transcribe all visible text.", "")
            .await?;
        debug_assert_eq!(tier, Tier::Text);
        extracted.push(result.analysis_text);
    }

    // The reasoning tier sees the segment's accumulated text plus its last
    // frame — one call, carrying everything the text tier already paid for.
    let joined = extracted.join("\n---\n");
    let prompt = format!("{reason_prompt}\n\nTEXT EXTRACTED FROM THIS SEGMENT:\n{joined}");
    let last = samples.last().expect("checked non-empty above");
    let (result, _) = router
        .perceive(
            PerceptionKind::Reason,
            last,
            &prompt,
            "segment category and meaning",
        )
        .await?;
    Ok(result.analysis_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::errors::VisionError;
    use crate::contracts::traits::TimeRange;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts calls so a test can prove which tier was actually used — the
    /// question "did this escalate?" is answered by the provider's call count,
    /// not by anything the router reports about itself.
    struct CountingProvider {
        name: &'static str,
        calls: Arc<AtomicUsize>,
        /// Every prompt this provider was given. Without capturing these, a
        /// regression that discards the extracted transcript is invisible: the
        /// call COUNTS stay identical and only the content changes.
        prompts: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl VisionProvider for CountingProvider {
        async fn analyze_video(
            &self,
            _: &Path,
            _: &str,
            _: Option<TimeRange>,
        ) -> Result<AnalysisResult, VisionError> {
            unreachable!("dayflow perception never analyses video")
        }
        async fn analyze_image(&self, _: &Path, prompt: &str) -> Result<AnalysisResult, VisionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut p) = self.prompts.lock() {
                p.push(prompt.to_string());
            }
            Ok(AnalysisResult {
                request_id: uuid::Uuid::new_v4(),
                analysis_text: format!("served by {}", self.name),
                provider: self.name.to_string(),
                model_used: self.name.to_string(),
                processing_time_ms: 1,
                token_count: None,
                completed_at: Utc::now(),
            })
        }
        async fn health_check(&self) -> Result<(), VisionError> {
            Ok(())
        }
        fn name(&self) -> &'static str {
            self.name
        }
        fn max_video_size(&self) -> u64 {
            0
        }
        fn supports_native_video(&self) -> bool {
            false
        }
        fn model(&self) -> &str {
            self.name
        }
    }

    fn prompts() -> Arc<Mutex<Vec<String>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn router() -> (PerceptionRouter, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let (r, t, rc, _, _) = router_capturing();
        (r, t, rc)
    }

    /// Same router, but the caller also gets the two prompt logs.
    #[allow(clippy::type_complexity)]
    fn router_capturing() -> (
        PerceptionRouter,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let (t, r) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (tp, rp) = (prompts(), prompts());
        let router = PerceptionRouter::new(
            Arc::new(CountingProvider {
                name: "text-tier",
                calls: t.clone(),
                prompts: tp.clone(),
            }),
            Arc::new(CountingProvider {
                name: "reason-tier",
                calls: r.clone(),
                prompts: rp.clone(),
            }),
            1_000,
        );
        (router, t, r, tp, rp)
    }

    #[tokio::test]
    async fn a_text_request_never_touches_the_reason_tier() {
        // The whole cost argument rests on this: an eight-hour day is ~160
        // samples, and routing even a fraction of them to the reasoning tier
        // costs two orders of magnitude more than the day is worth.
        let (router, text_calls, reason_calls) = router();
        for _ in 0..5 {
            router
                .perceive(PerceptionKind::Text, Path::new("/tmp/x.png"), "read it", "")
                .await
                .unwrap();
        }
        assert_eq!(text_calls.load(Ordering::SeqCst), 5);
        assert_eq!(
            reason_calls.load(Ordering::SeqCst),
            0,
            "the expensive tier must be untouched by ordinary sampling"
        );
    }

    #[tokio::test]
    async fn the_tier_follows_the_declared_kind_not_the_prompt() {
        // Sniffing the prompt would make cost depend on incidental wording —
        // the same request escalating or not on "read" versus "explain". These
        // two prompts are deliberately swapped relative to their kinds.
        let (router, text_calls, reason_calls) = router();

        let (_, tier) = router
            .perceive(
                PerceptionKind::Text,
                Path::new("/tmp/x.png"),
                "explain what this activity means and why",
                "",
            )
            .await
            .unwrap();
        assert_eq!(tier, Tier::Text, "a reasoning-SOUNDING prompt must not escalate");

        let (_, tier) = router
            .perceive(
                PerceptionKind::Reason,
                Path::new("/tmp/x.png"),
                "read the text",
                "categorising the segment",
            )
            .await
            .unwrap();
        assert_eq!(tier, Tier::Reason, "an OCR-sounding prompt must not de-escalate");

        assert_eq!(text_calls.load(Ordering::SeqCst), 1);
        assert_eq!(reason_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_escalation_is_recorded_with_its_reason_and_a_normal_interval_records_none() {
        // FR-007/010: "why was today expensive?" must be answerable from data.
        // Both halves matter — a recorder that logs everything is as useless as
        // one that logs nothing.
        let (router, _, _) = router();
        for _ in 0..3 {
            router
                .perceive(PerceptionKind::Text, Path::new("/tmp/x.png"), "read", "")
                .await
                .unwrap();
        }
        assert!(router.escalations().is_empty(), "ordinary sampling is not an escalation");

        router
            .perceive(
                PerceptionKind::Reason,
                Path::new("/tmp/x.png"),
                "categorise",
                "segment category requested",
            )
            .await
            .unwrap();

        let esc = router.escalations();
        assert_eq!(esc.len(), 1, "exactly one record per escalation");
        assert_eq!(esc[0].reason, "segment category requested");
        assert_eq!(esc[0].served_by, Tier::Reason);
    }

    #[tokio::test]
    async fn dayflow_traffic_cannot_exhaust_the_interactive_bucket() {
        // A day of background sampling must never starve a live user request.
        // Separate LIMITERS, not two keys in one: two keys still share a
        // capacity, so a shared limiter would let one drain the other.
        let interactive = RateLimiter::per_minute(10);
        let (router, _, _) = {
            let t = Arc::new(AtomicUsize::new(0));
            let r = Arc::new(AtomicUsize::new(0));
            (
                PerceptionRouter::new(
                    Arc::new(CountingProvider {
                        name: "text-tier",
                        calls: t.clone(),
                        prompts: prompts(),
                    }),
                    Arc::new(CountingProvider {
                        name: "reason-tier",
                        calls: r.clone(),
                        prompts: prompts(),
                    }),
                    3,
                ),
                t,
                r,
            )
        };

        // Exhaust dayflow's budget entirely.
        for _ in 0..3 {
            router
                .perceive(PerceptionKind::Text, Path::new("/tmp/x.png"), "read", "")
                .await
                .unwrap();
        }
        let refused = router
            .perceive(PerceptionKind::Text, Path::new("/tmp/x.png"), "read", "")
            .await;
        assert!(refused.is_err(), "dayflow's own budget must actually bind");

        // The interactive bucket is untouched by that exhaustion.
        for _ in 0..10 {
            interactive.check(INTERACTIVE_KEY).expect("live requests still served");
        }
    }

    #[tokio::test]
    async fn the_budget_charges_the_expensive_tier_too() {
        // Every budget test drove Text calls only, so exempting the REASONING
        // tier — the one that is two orders of magnitude more expensive (R19) —
        // from the budget would have passed the whole suite. That defeats the
        // budget's stated purpose exactly where it matters most.
        let t = Arc::new(AtomicUsize::new(0));
        let r = Arc::new(AtomicUsize::new(0));
        let router = PerceptionRouter::new(
            Arc::new(CountingProvider { name: "text-tier", calls: t.clone(), prompts: prompts() }),
            Arc::new(CountingProvider { name: "reason-tier", calls: r.clone(), prompts: prompts() }),
            2,
        );

        for _ in 0..2 {
            router
                .perceive(PerceptionKind::Reason, Path::new("/tmp/x.png"), "why", "categorising")
                .await
                .unwrap();
        }
        let refused = router
            .perceive(PerceptionKind::Reason, Path::new("/tmp/x.png"), "why", "categorising")
            .await;
        assert!(refused.is_err(), "reason-tier calls must be charged to the budget");
        assert_eq!(r.load(Ordering::SeqCst), 2, "and the refused one never reached the model");

        // and a Text call is refused too — one budget, both tiers
        assert!(router
            .perceive(PerceptionKind::Text, Path::new("/tmp/x.png"), "read", "")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn an_escalation_without_a_reason_is_refused() {
        // The file's own doc says "an escalation with no reason is a bug, not a
        // log line" — previously enforced only by the tests' good manners.
        let (router, _, r) = router();
        let err = router
            .perceive(PerceptionKind::Reason, Path::new("/tmp/x.png"), "why", "   ")
            .await;
        assert!(err.is_err(), "an unexplained escalation must not be billable");
        assert_eq!(r.load(Ordering::SeqCst), 0, "and must not reach the model");
        assert!(router.escalations().is_empty());
    }

    #[test]
    fn the_budget_is_derived_from_the_sampling_shape() {
        // A budget that is merely "large" would let a misconfiguration bill a
        // whole day before anything noticed. Coarse all-day tracking must ask
        // for far less than a fine-grained focused session.
        // The window must be long enough to express the COARSE cadence: with a
        // per-minute window, ceil(60/180) and ceil(60/60) both clamp to 1 and a
        // 3-minute interval asks for exactly as much as a 1-minute one. That
        // was a real bug this assertion caught.
        let coarse = dayflow_budget(180, 1, 4);
        let fine = dayflow_budget(60, 1, 4);
        assert!(fine > coarse, "a shorter interval needs a bigger budget: {fine} vs {coarse}");

        let one_display = dayflow_budget(60, 1, 4);
        let three = dayflow_budget(60, 3, 4);
        assert_eq!(three, one_display * 3, "budget scales with displays");

        let few = dayflow_budget(60, 1, 2);
        let many = dayflow_budget(60, 1, 8);
        assert_eq!(many, few * 4, "and with the region cap that bounds work at the source");

        // Ratios alone leave the MAGNITUDE unpinned — halving or 10x-ing the
        // whole budget passed every assertion above. Pin the derivation:
        // 600s window / 180s interval = 4 intervals, x1 display x4 regions
        // x2 retry headroom = 32.
        assert_eq!(dayflow_budget(180, 1, 4), 32, "600/180 -> 4 intervals x 4 regions x 2");
        assert_eq!(dayflow_budget(60, 2, 3), 120, "600/60 -> 10 x 2 displays x 3 regions x 2");

        assert!(dayflow_budget(0, 0, 0) > 0, "degenerate config must not deadlock");
    }

    // ─── T052/T053: content aggregation ───────────────────────────────────

    /// One screenful of a pane scrolled down by `offset` lines.
    fn scrolled(offset: usize, height: usize) -> String {
        (offset..offset + height).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_scrolling_pane_yields_one_growing_block_not_n_near_duplicates() {
        // The reason Content intent exists: sampling a document being read
        // produces heavily overlapping captures. Stored naively that is N
        // near-identical blobs; the useful artifact is one document.
        let mut agg = TextAggregator::new();
        for step in 0..5 {
            agg.absorb(&scrolled(step * 2, 10));
        }

        assert_eq!(agg.blocks().len(), 1, "overlapping captures are one document");
        let doc = &agg.blocks()[0];
        // Every line seen across all five captures survives...
        for i in 0..18 {
            assert!(doc.contains(&format!("line {i}\n")) || doc.ends_with(&format!("line {i}")),
                    "line {i} was captured and must be preserved");
        }
        // ...exactly once.
        assert_eq!(
            doc.lines().filter(|l| *l == "line 5").count(),
            1,
            "a line seen in four consecutive captures must not appear four times"
        );
    }

    #[test]
    fn an_unrelated_screen_starts_its_own_block() {
        // The symmetric risk: if everything merged, switching from a document
        // to a terminal would fold two unrelated things into one transcript.
        let mut agg = TextAggregator::new();
        agg.absorb(&scrolled(0, 10));
        let idx = agg.absorb("$ cargo test\nrunning 380 tests\nall green");
        assert_eq!(idx, 1);
        assert_eq!(agg.blocks().len(), 2, "unrelated material must not be merged");
        assert!(!agg.blocks()[0].contains("cargo test"));
    }

    #[test]
    fn a_partly_overlapping_but_mostly_different_screen_is_not_merged() {
        // The case the "unrelated screen" test above cannot reach: it shares
        // ZERO lines, so it passes under any threshold from 0.0 up. A capture
        // sharing ~half its lines is the one that discriminates — merging it
        // builds a document that is majority unrelated material.
        let mut agg = TextAggregator::new();
        agg.absorb(&scrolled(0, 10));

        // 10 lines, 5 shared with the block above → similarity ≈ 0.5.
        let half: String = (5..10)
            .map(|i| format!("line {i}"))
            .chain((0..5).map(|i| format!("unrelated {i}")))
            .collect::<Vec<_>>()
            .join("\n");
        let score = coverage(&agg.blocks()[0], &half);
        // 0.3 is videolocr's "comparable" floor — the value that was wrongly
        // ported here as a second merge arm. The fixture sits above it and
        // below SAME_BLOCK, which is exactly the band that discriminates.
        assert!(
            (0.3..SAME_BLOCK).contains(&score),
            "the fixture must sit between the two thresholds to discriminate: {score}"
        );

        assert_eq!(agg.absorb(&half), 1, "a half-different screen is its own material");
        assert_eq!(agg.blocks().len(), 2);
        assert!(!agg.blocks()[0].contains("unrelated 0"), "and must not pollute the first");
    }

    #[test]
    fn a_repeat_of_the_same_screen_does_not_grow_the_block() {
        // A static screen sampled repeatedly is the common case for a coarse
        // all-day cadence. It must not accumulate anything.
        let mut agg = TextAggregator::new();
        let screen = scrolled(0, 10);
        agg.absorb(&screen);
        let before = agg.blocks()[0].clone();
        for _ in 0..4 {
            agg.absorb(&screen);
        }
        assert_eq!(agg.blocks().len(), 1);
        assert_eq!(agg.blocks()[0], before, "an unchanged screen adds nothing");
    }

    #[test]
    fn the_comparison_history_is_bounded() {
        // An unbounded history makes every sample O(day) and grows without
        // limit across eight hours. Blocks older than the window are no longer
        // compared against, so distinct material stays distinct.
        let mut agg = TextAggregator::new();
        let first = "alpha unique one\nalpha unique two\nalpha unique three";
        agg.absorb(first);
        for i in 0..HISTORY {
            agg.absorb(&format!("filler {i} a\nfiller {i} b\nfiller {i} c"));
        }
        // The first block has fallen out of the window: an identical capture
        // now starts a new block rather than matching it.
        // EXACT count, not `> HISTORY`: after 1 seed + 5 fillers the length is
        // already 6, so `> HISTORY` holds whether the probe merges into the
        // out-of-window block (bounding broken, 6) or starts a new one
        // (correct, 7). The old assertion performed the right experiment and
        // then asserted nothing about its outcome.
        agg.absorb(first);
        assert_eq!(
            agg.blocks().len(),
            HISTORY + 2,
            "the seed has fallen out of the window, so an identical capture \
             starts a new block instead of scanning the whole day"
        );
    }

    #[test]
    fn every_capture_survives_absorption_including_degenerate_ones() {
        // FAIL-OPEN (R13). There must be no input for which absorb() silently
        // discards. A wrong merge costs duplication; a drop loses the only
        // record of that moment, and dayflow cannot re-capture yesterday.
        let mut agg = TextAggregator::new();
        let long = "x".repeat(5000);
        let three = scrolled(0, 3);
        let inputs = ["", "   ", "\n\n", "single", &three, "\u{2028}odd", &long];
        for (n, text) in inputs.iter().enumerate() {
            let idx = agg.absorb(text);
            assert!(idx < agg.blocks().len(), "input {n} must land in a real block");
        }
        assert!(!agg.blocks().is_empty());
    }

    #[test]
    fn activity_intent_has_no_aggregator_at_all() {
        // Absence is a stronger guarantee than a disabled flag: under Activity
        // there is no object to invoke by accident. Running Content all day
        // would be expensive for no benefit (D12).
        assert!(aggregator_for(DayflowIntent::Activity).is_none());
        assert!(aggregator_for(DayflowIntent::Content).is_some());
    }

    #[test]
    fn the_merge_threshold_separates_a_scroll_from_a_different_screen() {
        // The calibration itself, asserted — so a future change to `similarity`
        // that shifts the scale fails HERE, naming the reason, instead of
        // silently turning one document back into N blocks.
        let view = scrolled(0, 10);

        let scrolled_two = coverage(&view, &scrolled(2, 10));
        assert!(
            scrolled_two >= SAME_BLOCK,
            "a 2-line scroll shares 80% of the screen and must merge: {scrolled_two}"
        );

        let half: String = (5..10)
            .map(|i| format!("line {i}"))
            .chain((0..5).map(|i| format!("other {i}")))
            .collect::<Vec<_>>()
            .join("\n");
        let half_score = coverage(&view, &half);
        assert!(
            half_score < SAME_BLOCK,
            "a half-different screen must not merge: {half_score}"
        );
        assert!(
            scrolled_two - half_score > 0.2,
            "the threshold needs margin on both sides, not a knife edge"
        );
    }

    #[test]
    fn coverage_is_bounded_ordered_and_stable_as_the_block_grows() {
        let a = scrolled(0, 10);
        assert_eq!(coverage(&a, &a), 1.0, "fully contained is 1.0");
        assert_eq!(coverage(&a, "totally different content here"), 0.0, "disjoint is 0.0");
        assert_eq!(coverage(&a, ""), 0.0, "empty incoming covers nothing");
        assert_eq!(coverage("", &a), 0.0);

        let mostly = coverage(&a, &scrolled(1, 10));
        let barely = coverage(&a, &scrolled(8, 10));
        assert!(mostly > barely, "more overlap must score higher: {mostly} vs {barely}");

        // The property a symmetric metric cannot have: the score for a capture
        // does NOT decay just because the block accumulated more material.
        // The block must grow with material UNRELATED to this capture. Growing
        // it with scrolled(0, 40) would legitimately raise the score, since
        // that genuinely contains more of the capture — a wrong fixture, not a
        // wrong metric.
        let small = scrolled(0, 10);
        let grown = format!(
            "{}\n{}",
            small,
            (0..30).map(|i| format!("elsewhere {i}")).collect::<Vec<_>>().join("\n")
        );
        let capture = scrolled(2, 10);
        assert_eq!(
            coverage(&small, &capture),
            coverage(&grown, &capture),
            "coverage must not depend on how much unrelated-to-this-capture text \
             the block has accumulated"
        );
    }

    #[test]
    fn merge_keeps_the_union_without_duplicating_the_overlap() {
        // T053's stated property, asserted directly rather than through the
        // aggregator, so a merge bug cannot hide behind the matching logic.
        assert_eq!(merge_scroll("a\nb\nc", "b\nc\nd\ne"), "a\nb\nc\nd\ne");
        assert_eq!(merge_scroll("a\nb", "a\nb"), "a\nb", "a full repeat adds nothing");
        assert_eq!(merge_scroll("", "a\nb"), "\na\nb", "empty base keeps everything");
    }

    #[test]
    fn merging_preserves_legitimately_repeated_lines() {
        // Deduplicating by line VALUE loses every repeated line — a closing
        // brace, a blank separator, a repeated table row. For Content intent
        // the merged text IS the deliverable, so that silently corrupts every
        // source file and every table. The earlier set-union merge did exactly
        // this, and no fixture caught it because they all used globally unique
        // "line {i}" text.
        let block = "fn a() {\n    one();\n}";
        let incoming = "}\nfn b() {\n    two();\n}";
        let merged = merge_scroll(block, incoming);

        assert_eq!(
            merged.lines().filter(|l| l.trim() == "}").count(),
            2,
            "both closing braces must survive: {merged}"
        );
        assert!(merged.contains("fn a()") && merged.contains("fn b()"));
        assert!(merged.contains("two();"), "no incoming content may be dropped");
    }

    #[test]
    fn two_documents_behind_the_same_ui_chrome_do_not_merge() {
        // A realistic full-screen capture is mostly chrome — menu bar, file
        // tree, status bar — which is IDENTICAL between any two screens of the
        // same application. A set-based measure scores those at 0.80 and folds
        // a day's separate files into one interleaved blob. A contiguous run
        // tells them apart because chrome is SCATTERED, not consecutive.
        let chrome_top = "File  Edit  View\nEXPLORER\n  src/";
        let chrome_bottom = "Ln 1, Col 1    UTF-8\n$ cargo build\n   Compiling gentle-eye";
        let one = format!("{chrome_top}\nfn alpha() {{\n    first();\n{chrome_bottom}");
        let two = format!("{chrome_top}\nfn beta() {{\n    second();\n{chrome_bottom}");

        let score = coverage(&one, &two);
        assert!(
            score < SAME_BLOCK,
            "shared chrome must not read as the same document: {score}"
        );

        let mut agg = TextAggregator::new();
        agg.absorb(&one);
        agg.absorb(&two);
        assert_eq!(agg.blocks().len(), 2, "two files behind one UI are two documents");
    }

    #[tokio::test]
    async fn a_segment_costs_one_reason_call_however_many_samples_it_holds() {
        // The entire cost argument. If this ratio ever becomes per-sample, an
        // eight-hour day costs ~two orders of magnitude more (R19) — and
        // nothing else in the system would report that it had happened.
        let (router, text_calls, reason_calls) = router();
        let samples: Vec<std::path::PathBuf> =
            (0..5).map(|i| std::path::PathBuf::from(format!("/tmp/s{i}.png"))).collect();

        let out = summarize_segment_via_ladder(&router, &samples, "categorise this segment")
            .await
            .unwrap();

        assert_eq!(text_calls.load(Ordering::SeqCst), 5, "every sample is read");
        assert_eq!(
            reason_calls.load(Ordering::SeqCst),
            1,
            "but the segment is reasoned about ONCE"
        );
        assert_eq!(router.escalations().len(), 1);
        assert!(out.contains("reason-tier"), "the summary comes from the reasoning tier");
    }

    #[tokio::test]
    async fn the_one_reason_call_carries_what_every_text_call_extracted() {
        // Without this, discarding the transcript is INVISIBLE: the call counts
        // are unchanged and the reasoning tier silently summarises an entire
        // segment from one frame — same cost, worse output, and the text tier's
        // work becomes pure waste.
        let (router, _, _, _, reason_prompts) = router_capturing();
        let samples: Vec<std::path::PathBuf> =
            (0..4).map(|i| std::path::PathBuf::from(format!("/tmp/s{i}.png"))).collect();

        summarize_segment_via_ladder(&router, &samples, "categorise this segment")
            .await
            .unwrap();

        let seen = reason_prompts.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let prompt = &seen[0];
        assert!(prompt.contains("categorise this segment"), "the caller's ask survives");
        assert_eq!(
            prompt.matches("served by text-tier").count(),
            4,
            "every sample's extracted text must reach the reasoning tier: {prompt}"
        );
    }

    #[tokio::test]
    async fn a_longer_segment_does_not_cost_more_reasoning() {
        // The discriminating case: a per-sample implementation passes the test
        // above only by coincidence if sample count happens to be 1. Two
        // different lengths pin the ratio.
        for n in [1usize, 12] {
            let (router, text_calls, reason_calls) = router();
            let samples: Vec<std::path::PathBuf> =
                (0..n).map(|i| std::path::PathBuf::from(format!("/tmp/s{i}.png"))).collect();
            summarize_segment_via_ladder(&router, &samples, "categorise").await.unwrap();
            assert_eq!(text_calls.load(Ordering::SeqCst), n);
            assert_eq!(reason_calls.load(Ordering::SeqCst), 1, "still one, at n={n}");
        }
    }

    #[tokio::test]
    async fn an_empty_segment_is_refused_rather_than_billed() {
        // Sending an empty segment to the reasoning tier would spend a call to
        // describe nothing, and return invention with no grounding.
        let (router, text_calls, reason_calls) = router();
        let err = summarize_segment_via_ladder(&router, &[], "categorise").await;
        assert!(err.is_err());
        assert_eq!(text_calls.load(Ordering::SeqCst), 0);
        assert_eq!(reason_calls.load(Ordering::SeqCst), 0, "nothing is billed");
        assert!(router.escalations().is_empty());
    }
}
