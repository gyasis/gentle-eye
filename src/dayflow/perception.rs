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
//
// There is deliberately NO `INTERACTIVE_KEY` here. One was defined, and it named
// an isolation nothing implements: no interactive bucket exists anywhere in the
// tree, so the test "asserting" the isolation constructed its own local limiter
// and checked it — an assertion that cannot fail, describing protection that is
// absent. When a real interactive limiter exists, isolation follows from it
// being a SEPARATE limiter, not from a key.

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
        let (provider, tier) = match kind {
            PerceptionKind::Text => (&self.text, Tier::Text),
            PerceptionKind::Reason => (&self.reason, Tier::Reason),
        };

        // VALIDATE BEFORE SPENDING. Charging the budget first meant a malformed
        // request consumed a token it could never use, so a caller could starve
        // the day's real work with requests that were refused anyway.
        if tier == Tier::Reason && reason.trim().is_empty() {
            return Err(DayflowError::Invalid(
                "an escalation to the reasoning tier must state its reason".into(),
            ));
        }

        self.limiter
            .check(DAYFLOW_KEY)
            .map_err(|e| DayflowError::Perception(format!("dayflow perception budget: {e}")))?;

        if tier == Tier::Reason {
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

/// At or above this fraction UNCHANGED, a capture is the same screen with an
/// edit in it — however novel the changed sliver is.
///
/// Deliberately much higher than [`SAME_BLOCK`]: this evidence is "almost
/// nothing moved", and at a third of the screen changed that claim is false.
/// The judgement it encodes: a capture 90% identical to one already held is
/// the same screen, and merging it costs a stray line inside one block, where
/// splitting it costs a near-duplicate copy of the WHOLE screen every interval
/// — which is the exact waste the aggregator exists to prevent.
const STABLE_SCREEN: f64 = 0.90;

// T053 specifies "append a block only below 0.95 similarity". That gate is
// SUBSUMED here rather than implemented: line-level `diff_merge` adds only lines
// the block does not already have, so a near-identical capture contributes
// nothing by construction and needs no threshold to stop it. It was implemented
// as an explicit branch first; mutation testing showed removing the branch
// changed no behaviour at all, which is the definition of a constant no
// behaviour depends on. Deleted rather than wrapped in a test that would only
// have asserted the optimization was taken.

/// The longest run of consecutive lines common to `a` and `b`, as
/// `(start index in b, length)`. Comparison is on trimmed lines.
fn longest_common_run(a: &[&str], b: &[&str]) -> (usize, usize) {
    let (mut best_b, mut best_len) = (0usize, 0usize);
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for x in a {
        for (bi, y) in b.iter().enumerate() {
            cur[bi + 1] = if x.trim() == y.trim() { prev[bi] + 1 } else { 0 };
            if cur[bi + 1] > best_len {
                best_len = cur[bi + 1];
                best_b = bi + 1 - best_len;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.iter_mut().for_each(|v| *v = 0);
    }
    (best_b, best_len)
}

/// Split two captures into `(frame prefix len, content_a, content_b, frame suffix len)`.
///
/// **This separates the window from what is inside it.** Every realistic capture
/// is an application window: a menu bar, a file tree, a status bar, a terminal —
/// UI CHROME that is byte-identical between any two screens of that app and
/// pinned to the same position in every capture. What is IN the window scrolls.
///
/// Neither earlier metric survived that. A set measure scored two different
/// files behind one editor at 0.80 and merged them. A single contiguous run
/// scored a genuine scroll at 0.50 and split it, because chrome interrupts the
/// run at BOTH content boundaries — reintroducing the exact failure T052 exists
/// to prevent. Each fixture passed only because it lacked the other's
/// ingredient.
///
/// Chrome is positionally STABLE and content MOVES, so strip the positionally
/// identical head and tail first and compare only what is left.
fn frame_split<'a>(
    a: &'a [&'a str],
    b: &'a [&'a str],
) -> (usize, &'a [&'a str], &'a [&'a str], usize) {
    let p = a.iter().zip(b).take_while(|(x, y)| x.trim() == y.trim()).count();
    let s = a[p..]
        .iter()
        .rev()
        .zip(b[p..].iter().rev())
        .take_while(|(x, y)| x.trim() == y.trim())
        .count();
    (p, &a[p..a.len() - s], &b[p..b.len() - s], s)
}

/// How much of `incoming`'s CONTENT the `block` already holds, in `[0, 1]`.
///
/// **Asymmetric on purpose.** A symmetric ratio decays mechanically as the block
/// grows — 0.80, 0.73, 0.67, 0.62 → splits — so every long document eventually
/// fragments and the threshold only decides when. The question that matters is
/// "is this capture material I already have?", a property of the INCOMING text,
/// which stays stable however large the block gets.
///
/// Line-based because OCR of a scrolling pane re-reads whole lines: drift within
/// a line is noise, a new line is signal. It is therefore brittle to an OCR
/// misread MID-RUN, which splits the run — see research.md R25.
pub fn coverage(block: &str, incoming: &str) -> f64 {
    let lb: Vec<&str> = block.lines().collect();
    let li: Vec<&str> = incoming.lines().collect();
    if li.is_empty() {
        return 0.0;
    }
    let (prefix, content_b, content_i, suffix) = frame_split(&lb, &li);
    if content_i.is_empty() {
        // Everything in the capture is already held: the same screen, or one
        // scrolled back to material we have.
        return 1.0;
    }
    if content_b.is_empty() && !lb.is_empty() {
        // The block has NOTHING between the shared head and tail, so the
        // capture strictly EXTENDS it — nothing was replaced, only added. That
        // is a document being written into, or a terminal filling up before it
        // first scrolls, and it is the same screen no matter how much was
        // added. Deciding it by "how much changed" instead would make the
        // answer depend on the window's height: two new lines is 5% of a
        // 40-line view and 20% of a 10-line one, and the same edit cannot be
        // the same screen on one monitor and a new document on another.
        //
        // `!lb.is_empty()` matters: an EMPTY block also has an empty content
        // region, but it holds no evidence about anything and must score 0.
        return 1.0;
    }

    // TWO kinds of evidence, and the maximum of them.
    //
    // The run answers "did this SCROLL from what I have?". On its own that is
    // the wrong question for every change that is not a scroll: typing at the
    // bottom of a file, an OCR misread of one line, a cursor or clock ticking
    // in a status bar. In all of those the changed region is NOVEL by
    // definition, so a run-only score is exactly 0.0 and each sample forks a
    // near-duplicate copy of the whole screen — and a mostly-static screen is
    // the most common state of an all-day capture, so that is the common case,
    // not the pathological one.
    //
    // The unchanged fraction answers the other question: "is most of this
    // screen already what I have?". A capture whose changed region is a small
    // slice of the screen is the same screen with an edit in it. One whose
    // changed region is most of the screen is different material — unless the
    // run says it scrolled.
    // The two bars DIFFER, because the two questions differ. A scroll may
    // replace most of the screen and still be the same document, so the run
    // bar is SAME_BLOCK. An edit is a small perturbation — if a third of the
    // screen changed, that is a different screen, not a typo. Using one bar for
    // both would merge two different files whenever chrome happened to be a
    // large share of the capture.
    let unchanged = (prefix + suffix) as f64 / li.len() as f64;
    if unchanged >= STABLE_SCREEN {
        return unchanged;
    }
    let (_, len) = longest_common_run(content_b, content_i);
    len as f64 / content_i.len() as f64
}

/// Merge `incoming` into `block`, keeping the frame once and appending only new content.
///
/// **Not a set-union, and not a trim.** Deduplicating by line VALUE loses every
/// legitimately repeated line — a closing brace, a blank separator, a repeated
/// table row — and for Content intent the merged text IS the deliverable. Lines
/// are appended EXACTLY as captured: comparison trims, but a merge that trimmed
/// would strip the indentation from every appended line, which in Python is not
/// a cosmetic loss.
pub fn merge_scroll(block: &str, incoming: &str) -> String {
    let lb: Vec<&str> = block.lines().collect();
    let li: Vec<&str> = incoming.lines().collect();
    if li.is_empty() {
        return block.to_string();
    }
    let (_, content_b, content_i, suffix) = frame_split(&lb, &li);
    if content_i.is_empty() {
        return block.to_string();
    }
    let (run_start, run_len) = longest_common_run(content_b, content_i);

    // Rebuild as everything up to the frame suffix, then the incoming content
    // that is not the shared run, then the frame suffix back on — so a status
    // bar stays at the bottom instead of being buried mid-document.
    let mut out: Vec<&str> = lb[..lb.len() - suffix].to_vec();
    if run_len == 0 {
        out.extend(content_i);
    } else {
        out.extend(&content_i[..run_start]);
        out.extend(&content_i[run_start + run_len..]);
    }
    out.extend(&lb[lb.len() - suffix..]);
    out.join("\n")
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

// ─── T027: crop before extract (FR-011) ───────────────────────────────────

/// Crop a captured frame to its regions, at FULL resolution, in reading order.
///
/// **Never send the whole frame downscaled.** A 4K screen shrunk to fit a
/// model's input is unreadable as text — and worse than unreadable, it is
/// SILENTLY unreadable: the model returns confident nonsense rather than an
/// error. Two side-by-side panes downscaled together also let the model read
/// straight across the gutter, interleaving two documents line by line into
/// text that looks plausible and belongs to neither.
///
/// So the text tier sees one full-resolution crop per region, and the caller
/// keeps them apart. `max_regions` bounds the work at the source, which is what
/// the perception budget is derived from.
///
/// `display_id` is the display this FRAME came from. Region boxes are
/// display-LOCAL (see [`crate::regions::Region::display_id`]), so a region from
/// another display indexes into these pixels as if it belonged here — cropping
/// the wrong content, confidently and with no error. Those are skipped.
pub fn crop_regions(
    frame: &Path,
    regions: &[crate::regions::Region],
    out_dir: &Path,
    max_regions: usize,
    display_id: u32,
) -> Result<Vec<std::path::PathBuf>, DayflowError> {
    let img = image::open(frame)
        .map_err(|e| DayflowError::Invalid(format!("open {}: {e}", frame.display())))?;
    let (fw, fh) = (img.width(), img.height());

    std::fs::create_dir_all(out_dir)
        .map_err(|e| DayflowError::Internal(format!("create {}: {e}", out_dir.display())))?;

    let mut out = Vec::new();
    for (n, r) in regions.iter().take(max_regions).enumerate() {
        // Regions carry their own display, and bboxes are display-LOCAL, so a
        // region from another screen would index straight into these pixels as
        // if it belonged here.
        if r.display_id != display_id {
            continue;
        }
        let (rx, ry) = (r.bbox.x as i64, r.bbox.y as i64);

        // Real INTERSECTION, not a clamp. Clamping `x` into the frame drags a
        // box that is entirely off-screen onto the edge and yields a 1x1 crop
        // of a corner pixel — content belonging to no region, fed to OCR under
        // that region's name, burning a budgeted call on garbage. A region that
        // does not overlap this frame has nothing to say about it.
        let x0 = rx.max(0);
        let y0 = ry.max(0);
        let x1 = (rx + r.bbox.w as i64).min(fw as i64);
        let y1 = (ry + r.bbox.h as i64).min(fh as i64);
        if x1 <= x0 || y1 <= y0 {
            tracing::warn!(
                region = n,
                display = r.display_id,
                "region does not intersect this frame; skipped rather than cropped to an edge"
            );
            continue;
        }
        let (x, y) = (x0 as u32, y0 as u32);
        let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);

        // `crop_imm` borrows; `crop` needs `&mut self` and so forced a clone of
        // the whole decoded frame per region — ~33 MB at 4K, times max_regions,
        // times every sample of the day, to produce a sub-image.
        let cropped = img.crop_imm(x, y, w, h);
        let path = out_dir.join(format!("r{n:02}_{x}_{y}_{w}x{h}.png"));
        cropped
            .save(&path)
            .map_err(|e| DayflowError::Internal(format!("write {}: {e}", path.display())))?;
        out.push(path);
    }
    Ok(out)
}

// ─── T029: residency policy (FR-008/013) ──────────────────────────────────

/// What one segment's perception actually cost (FR-013).
///
/// Recorded per segment INCLUDING any model reload, because the reload is the
/// thing residency exists to avoid: a latency figure that excluded it would
/// report the policy as free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentLatency {
    /// Samples in the segment.
    pub samples: usize,
    /// Perception calls actually made — text calls (one per region crop, or one
    /// per frame when no regions were captured) plus the single reasoning call.
    ///
    /// NOT equal to `samples`: cropping turns one sample into one call per
    /// region, which is exactly what the budget is derived from, so reporting
    /// samples as if they were calls would understate the spend.
    pub perception_calls: usize,
    /// Wall time for the segment's perception.
    pub total: std::time::Duration,
}

impl SegmentLatency {
    /// Mean wall time per perception call.
    ///
    /// The useful comparison for residency: a segment whose first call paid a
    /// cold load shows a higher mean than one that ran entirely warm, without
    /// needing a hardcoded absolute threshold that is only valid for one model
    /// on one machine.
    pub fn mean_call(&self) -> std::time::Duration {
        self.total.checked_div(self.perception_calls.max(1) as u32).unwrap_or_default()
    }
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
    display_id: u32,
    max_regions: usize,
) -> Result<(String, SegmentLatency), DayflowError> {
    if samples.is_empty() {
        return Err(DayflowError::Invalid(
            "cannot summarise a segment with no samples".into(),
        ));
    }

    let started = std::time::Instant::now();
    let mut extracted = Vec::with_capacity(samples.len());

    for path in samples {
        // CROP BEFORE EXTRACT (T027/FR-011). The regions detected when this
        // frame was captured are written beside it; without them there is no
        // choice but to read the whole frame, which on a 4K screen is silently
        // unreadable — the model returns confident nonsense rather than an
        // error, and two side-by-side panes get read straight across the
        // gutter into text belonging to neither.
        //
        // FAIL-OPEN: no sidecar means read the frame whole. A missing region
        // file must degrade the reading, never drop the sample — dayflow
        // cannot re-capture yesterday.
        let targets = match load_regions_beside(path) {
            Some(regions) if !regions.is_empty() => {
                let dir = path.with_extension("crops");
                match crop_regions(path, &regions, &dir, max_regions, display_id) {
                    Ok(crops) if !crops.is_empty() => crops,
                    Ok(_) => vec![path.clone()],
                    Err(e) => {
                        tracing::warn!(error = %e, sample = %path.display(),
                            "cropping failed; reading the whole frame instead");
                        vec![path.clone()]
                    }
                }
            }
            _ => vec![path.clone()],
        };

        for target in &targets {
            let (result, tier) = router
                .perceive(PerceptionKind::Text, target, "Transcribe all visible text.", "")
                .await?;
            debug_assert_eq!(tier, Tier::Text);
            extracted.push(result.analysis_text);
        }
    }

    // The reasoning tier sees the segment's accumulated text plus its LAST
    // frame — the one carrying the segment's end state, which is what a
    // question about "what was I doing" is usually asking about.
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

    Ok((
        result.analysis_text,
        SegmentLatency {
            samples: samples.len(),
            perception_calls: extracted.len() + 1,
            total: started.elapsed(),
        },
    ))
}

/// Where a sample's detected regions live, if the capture wrote them.
///
/// A sidecar rather than a parameter because the region cascade runs at CAPTURE
/// time while summarisation happens later, at segment close — by then the
/// screen has moved on, and re-detecting would describe a different moment than
/// the pixels do.
pub fn regions_path(sample: &Path) -> std::path::PathBuf {
    sample.with_extension("regions.json")
}

fn load_regions_beside(sample: &Path) -> Option<Vec<crate::regions::Region>> {
    let p = regions_path(sample);
    let raw = std::fs::read_to_string(&p).ok()?;
    match serde_json::from_str(&raw) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, path = %p.display(),
                "region sidecar unreadable; reading the whole frame");
            None
        }
    }
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
            // DISTINCT PER CALL. With identical text every call, a regression
            // that sends the LAST sample's text N times is indistinguishable
            // from sending all N — the assertion counts occurrences and both
            // give the same number.
            let n = self.calls.load(Ordering::SeqCst);
            Ok(AnalysisResult {
                request_id: uuid::Uuid::new_v4(),
                analysis_text: format!("served by {} #{n}", self.name),
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
    async fn the_dayflow_budget_actually_binds() {
        // This used to also "prove" isolation from an interactive bucket by
        // constructing a fresh local limiter and checking it ten times — which
        // succeeds by construction under ANY source mutation, and described a
        // bucket that does not exist. Only the half that can fail remains.
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

        // And it must not have SPENT anything: a request refused for being
        // malformed that still burns a token lets bad callers starve the day's
        // real work. Budget 1, so a surviving token is provable.
        let (small, _, r2) = {
            let t = Arc::new(AtomicUsize::new(0));
            let r2 = Arc::new(AtomicUsize::new(0));
            (
                PerceptionRouter::new(
                    Arc::new(CountingProvider {
                        name: "text-tier",
                        calls: t.clone(),
                        prompts: prompts(),
                    }),
                    Arc::new(CountingProvider {
                        name: "reason-tier",
                        calls: r2.clone(),
                        prompts: prompts(),
                    }),
                    1,
                ),
                t,
                r2,
            )
        };
        assert!(small
            .perceive(PerceptionKind::Reason, Path::new("/tmp/x.png"), "why", "")
            .await
            .is_err());
        small
            .perceive(PerceptionKind::Reason, Path::new("/tmp/x.png"), "why", "categorising")
            .await
            .expect("the refused request must not have consumed the only token");
        assert_eq!(r2.load(Ordering::SeqCst), 1);
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

    // ─── T027 / T029 ──────────────────────────────────────────────────────

    fn two_pane_frame(dir: &Path) -> std::path::PathBuf {
        // A 400x200 frame: left pane solid red, right pane solid blue, with a
        // black gutter between. Colour stands in for text — what matters is
        // that a crop of the left pane must contain NO blue, which is the
        // pixel-level form of "the model never read across the gutter".
        let mut img = image::RgbaImage::new(400, 200);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = if x < 190 {
                image::Rgba([255, 0, 0, 255])
            } else if x < 210 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            };
        }
        let p = dir.join("frame.png");
        img.save(&p).unwrap();
        p
    }

    fn region_at(x: u32, y: u32, w: u32, h: u32) -> crate::regions::Region {
        crate::regions::Region::new(
            crate::target::model::PixelRect { x, y, w, h },
            crate::regions::Source::Wm,
            crate::regions::Granularity::Pane,
            0.8,
        )
    }

    #[test]
    fn each_pane_is_cropped_at_full_resolution_and_never_read_across_the_gutter() {
        let dir = tempfile::tempdir().unwrap();
        let frame = two_pane_frame(dir.path());
        let regions = vec![region_at(0, 0, 190, 200), region_at(210, 0, 190, 200)];

        let crops = crop_regions(&frame, &regions, &dir.path().join("crops"), 8, 0).unwrap();
        assert_eq!(crops.len(), 2, "one crop per pane, in reading order");

        let left = image::open(&crops[0]).unwrap().to_rgba8();
        let right = image::open(&crops[1]).unwrap().to_rgba8();

        // FULL resolution: the crop is the region's real size, not a downscale.
        assert_eq!((left.width(), left.height()), (190, 200));
        assert_eq!((right.width(), right.height()), (190, 200));

        // The column scramble: a downscaled full frame lets a model read
        // straight across the gutter and interleave two documents into text
        // that looks plausible and belongs to neither.
        assert!(
            left.pixels().all(|p| p.0[2] == 0),
            "the left pane's crop must contain nothing from the right pane"
        );
        assert!(
            right.pixels().all(|p| p.0[0] == 0),
            "and vice versa"
        );
    }

    #[test]
    fn a_region_partly_off_the_frame_is_trimmed_and_one_entirely_off_is_skipped() {
        // Regions can outlive the frame they are applied to — a window
        // straddling two displays, or a stale box after a resize.
        //
        // The earlier version of this test asserted that an ENTIRELY off-frame
        // region "clamps to a 1px sliver, not an error", and enshrined a bug:
        // clamping x into the frame drags an off-screen box onto the edge and
        // yields a crop of a corner pixel, which is then fed to OCR under that
        // region's name. That is content belonging to no region, a budgeted
        // call spent on garbage, and a miniature of the cross-pane
        // contamination cropping exists to prevent. A region that does not
        // overlap the frame has nothing to say about it.
        let dir = tempfile::tempdir().unwrap();
        let frame = two_pane_frame(dir.path());
        let regions = vec![
            region_at(300, 100, 400, 400),   // overlaps: trim to the frame
            region_at(0, 0, 100, 100),       // wholly inside
            region_at(9_000, 9_000, 50, 50), // entirely outside: skip
        ];

        let crops = crop_regions(&frame, &regions, &dir.path().join("crops"), 8, 0).unwrap();
        assert_eq!(crops.len(), 2, "the non-intersecting region yields NO crop");

        let trimmed = image::open(&crops[0]).unwrap();
        assert_eq!(
            (trimmed.width(), trimmed.height()),
            (100, 100),
            "the overlapping region is trimmed to the intersection"
        );
        assert!(
            !crops.iter().any(|p| p.to_string_lossy().contains("1x1")),
            "no 1x1 corner-pixel crop may be produced: {crops:?}"
        );
    }

    #[test]
    fn a_region_from_another_display_never_crops_this_frame() {
        // Region bboxes are display-LOCAL, so a region belonging to another
        // screen indexes straight into these pixels as if it were here —
        // cropping the wrong content, confidently and with no error.
        let dir = tempfile::tempdir().unwrap();
        let frame = two_pane_frame(dir.path());
        let mut other = region_at(0, 0, 100, 100);
        other.display_id = 1;
        let mine = region_at(210, 0, 100, 100);

        let crops =
            crop_regions(&frame, &[other, mine], &dir.path().join("crops"), 8, 0).unwrap();
        assert_eq!(crops.len(), 1, "only this display's region is cropped");
        let img = image::open(&crops[0]).unwrap().to_rgba8();
        assert!(img.pixels().all(|p| p.0[0] == 0), "and it is the RIGHT region's pixels");
    }

    #[test]
    fn the_region_cap_bounds_the_work_at_the_source() {
        // This is the same number the perception budget is derived from, so a
        // cap that did not bind would make the budget a fiction.
        let dir = tempfile::tempdir().unwrap();
        let frame = two_pane_frame(dir.path());
        let regions: Vec<_> = (0..20).map(|i| region_at(i * 10, 0, 50, 50)).collect();
        let crops = crop_regions(&frame, &regions, &dir.path().join("crops"), 4, 0).unwrap();
        assert_eq!(crops.len(), 4, "never more than max_regions crops");
    }

    #[test]
    fn residency_is_sized_by_the_segment_cadence_not_the_sample_interval() {
        // The bug this pins: dayflow does NOT perceive per sample. A segment's
        // text calls fire back-to-back at segment close, so the gap the model
        // must survive to stay warm is the gap between SEGMENTS (~900s), not
        // the sample interval (~180s). Sized from the sample interval, the
        // window expired long before the next burst — `Resident` held memory
        // AND paid every cold load, while reporting itself as residency.
        use crate::config::ResidencyPolicy;
        let segment = std::time::Duration::from_secs(900);
        let sample_interval = std::time::Duration::from_secs(180);

        let ka = ResidencyPolicy::Resident.keep_alive(segment).unwrap();
        assert_eq!(ka, "1860s", "twice the segment cadence plus margin");
        let secs: u64 = ka.trim_end_matches('s').parse().unwrap();
        assert!(
            secs > segment.as_secs(),
            "a window that does not outlast one segment cannot bridge two"
        );
        assert!(
            secs > 4 * sample_interval.as_secs(),
            "sizing from the sample interval would expire mid-gap: {secs}s"
        );

        assert_eq!(ResidencyPolicy::OnDemand.keep_alive(segment), None);
        assert_eq!(ResidencyPolicy::Off.keep_alive(segment).as_deref(), Some("0"));
        assert_eq!(ResidencyPolicy::default(), ResidencyPolicy::OnDemand);
    }

    #[test]
    fn segment_latency_counts_calls_not_samples() {
        // Cropping turns one sample into one call per region, which is exactly
        // what the budget is derived from — so reporting samples as if they
        // were calls would understate the spend by the region factor.
        let l = SegmentLatency {
            samples: 5,
            perception_calls: 21, // 5 samples x 4 regions + 1 reasoning call
            total: std::time::Duration::from_millis(4_200),
        };
        assert_ne!(l.perception_calls, l.samples);
        assert_eq!(l.mean_call(), std::time::Duration::from_millis(200));

        // A segment that paid a cold load shows a higher mean than one that ran
        // warm — no hardcoded absolute threshold, which would only ever be
        // valid for one model on one machine.
        let warm = SegmentLatency {
            samples: 5,
            perception_calls: 21,
            total: std::time::Duration::from_millis(1_000),
        };
        assert!(l.mean_call() > warm.mean_call());
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
        assert_eq!(merge_scroll("", "a\nb"), "a\nb", "empty base keeps everything");
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

    /// One captured application window: chrome, then content, then chrome.
    ///
    /// EVERY realistic capture has this shape. The two fixtures this replaced
    /// each had only half of it — a scroll with no chrome, and chrome with no
    /// scroll — so each passed only because it lacked the other's ingredient,
    /// and the metric was wrong in both directions at once.
    fn windowed(content: &[String]) -> String {
        let mut v = vec![
            "File  Edit  View".to_string(),
            "EXPLORER".to_string(),
            "  src/".to_string(),
        ];
        v.extend(content.iter().cloned());
        v.extend([
            "Ln 1, Col 1    UTF-8".to_string(),
            "$ cargo build".to_string(),
            "   Compiling gentle-eye".to_string(),
        ]);
        v.join("\n")
    }

    fn doc_lines(offset: usize, height: usize) -> Vec<String> {
        (offset..offset + height).map(|i| format!("    doc line {i}")).collect()
    }

    #[test]
    fn a_document_scrolling_inside_a_window_is_one_block_chrome_and_all() {
        // The case both earlier fixtures missed. Chrome interrupts the shared
        // run at BOTH content boundaries, so a single-contiguous-run metric
        // scores every sample at 0.50 and produces five blocks — the exact
        // failure T052 exists to prevent, reintroduced by the fix for the
        // opposite direction.
        let mut agg = TextAggregator::new();
        for step in 0..5 {
            agg.absorb(&windowed(&doc_lines(step * 2, 10)));
        }
        assert_eq!(
            agg.blocks().len(),
            1,
            "a document scrolling inside a window is ONE document: {:#?}",
            agg.blocks()
        );

        let doc = &agg.blocks()[0];
        for i in 0..18 {
            assert_eq!(
                doc.lines().filter(|l| l.trim() == format!("doc line {i}")).count(),
                1,
                "line {i} must appear exactly once in {doc}"
            );
        }
        // Appended lines keep their indentation. Comparison trims; a merge that
        // trimmed would corrupt every Python file it ever captured.
        assert_eq!(
            doc.lines().filter(|l| l.starts_with("    doc line")).count(),
            18,
            "every appended content line keeps its original indentation"
        );
        assert_eq!(doc.matches("EXPLORER").count(), 1, "chrome is kept once");
        assert!(
            doc.trim_end().ends_with("Compiling gentle-eye"),
            "the frame suffix stays at the end, not buried mid-document"
        );
    }

    #[test]
    fn a_document_being_written_stays_one_block() {
        // Append-at-bottom growth: typing into a file, or a terminal filling up
        // before it first scrolls. The top of the capture never moves, so a
        // run-only metric classifies ALL prior content as frame, leaving only
        // the new lines to compare — which match nothing, score 0.0, and fork a
        // near-duplicate copy of the whole screen every interval.
        let mut agg = TextAggregator::new();
        for written in 1..=5 {
            let body: Vec<String> =
                (0..8 + written * 2).map(|i| format!("    code line {i}")).collect();
            agg.absorb(&windowed(&body));
        }
        assert_eq!(
            agg.blocks().len(),
            1,
            "a file being typed into is ONE document: {:#?}",
            agg.blocks()
        );
        let doc = &agg.blocks()[0];
        for i in 0..18 {
            assert_eq!(
                doc.lines().filter(|l| l.trim() == format!("code line {i}")).count(),
                1,
                "line {i} exactly once"
            );
        }
    }

    #[test]
    fn ocr_jitter_on_a_static_screen_does_not_fork_a_block() {
        // The most common all-day state at a coarse cadence is a screen that
        // barely changes, and OCR misreading one line per sample is the
        // EXPECTED condition on it, not a pathological one. Under a run-only
        // metric the changed region is exactly the misread line, which matches
        // nothing — coverage 0.0, five near-duplicate full-screen blocks.
        let mut agg = TextAggregator::new();
        for sample in 0..5 {
            let body: Vec<String> = (0..20)
                .map(|i| {
                    if i == 9 {
                        // the flubbed line differs every sample
                        format!("    the qu1ck br0wn f0x {sample}")
                    } else {
                        format!("    stable line {i}")
                    }
                })
                .collect();
            agg.absorb(&windowed(&body));
        }
        assert_eq!(
            agg.blocks().len(),
            1,
            "OCR jitter must not fork the screen: {:#?}",
            agg.blocks()
        );
        // Fail-open: every variant is KEPT rather than any being dropped, since
        // we cannot know which reading was correct.
        let doc = &agg.blocks()[0];
        assert_eq!(
            doc.lines().filter(|l| l.contains("qu1ck")).count(),
            5,
            "every reading is kept — dropping one would discard the only record"
        );
    }

    #[test]
    fn a_status_bar_ticking_does_not_fork_a_block() {
        // The smallest possible change, in the frame itself: a clock or a
        // cursor position updating while nothing else moves.
        let mut agg = TextAggregator::new();
        let body: Vec<String> = (0..15).map(|i| format!("    line {i}")).collect();
        for minute in 0..5 {
            let mut v = vec!["File  Edit  View".to_string()];
            v.extend(body.iter().cloned());
            v.push(format!("Ln 1, Col 1    14:0{minute}"));
            agg.absorb(&v.join("\n"));
        }
        assert_eq!(agg.blocks().len(), 1, "a ticking clock is not a new document");
    }

    #[test]
    fn two_documents_behind_the_same_window_do_not_merge() {
        // The opposite error. Chrome is identical between ANY two screens of one
        // app, so only the content may decide.
        let one = windowed(&["    fn alpha() {".into(), "        first();".into()]);
        let two = windowed(&["    fn beta() {".into(), "        second();".into()]);

        let score = coverage(&one, &two);
        assert!(score < SAME_BLOCK, "different files behind one window: {score}");

        let mut agg = TextAggregator::new();
        agg.absorb(&one);
        agg.absorb(&two);
        assert_eq!(agg.blocks().len(), 2, "two files behind one UI are two documents");
        assert!(!agg.blocks()[0].contains("beta"));
    }

    #[test]
    fn a_wide_sidebar_cannot_carry_two_unrelated_screens_together() {
        // Chrome is the MAJORITY of the capture — what a file tree or terminal
        // scrollback actually looks like — and it is perfectly contiguous, so
        // "chrome is scattered" is no defence. The content still decides.
        let tree: Vec<String> = (0..12).map(|i| format!("  file_{i}.rs")).collect();
        let mk = |body: &[&str]| {
            let mut v = tree.clone();
            v.extend(body.iter().map(|s| s.to_string()));
            v.join("\n")
        };
        let alpha: Vec<&str> = vec![
            "fn alpha() {", "    let a = 1;", "    let b = 2;", "    step_one();",
            "    step_two();", "    step_three();", "    finish();", "    done();",
            "    return a + b;", "}",
        ];
        let beta: Vec<&str> = vec![
            "struct Beta {", "    field_x: u32,", "    field_y: String,", "}",
            "impl Beta {", "    fn render(&self) {", "        draw();", "        flush();",
            "    }", "}",
        ];
        let score = coverage(&mk(&alpha), &mk(&beta));
        assert!(
            score < SAME_BLOCK,
            "12 lines of identical sidebar must not carry 10 lines of different content: {score}"
        );
    }

    #[test]
    fn a_screen_that_is_almost_entirely_identical_is_the_same_screen() {
        // The boundary of the judgement above, asserted rather than left
        // implicit. At ONE line differing out of thirteen, this deliberately
        // merges: splitting would store a near-duplicate copy of the whole
        // screen every interval, which is precisely the waste the aggregator
        // exists to prevent, and the cost of merging is one stray line inside
        // an otherwise correct block.
        let tree: Vec<String> = (0..12).map(|i| format!("  file_{i}.rs")).collect();
        let mk = |body: &str| {
            let mut v = tree.clone();
            v.push(body.to_string());
            v.join("\n")
        };
        let score = coverage(&mk("fn alpha() {}"), &mk("fn beta() {}"));
        assert!(
            score >= STABLE_SCREEN,
            "12 of 13 lines identical is the same screen, not a new document: {score}"
        );
    }

    #[tokio::test]
    async fn a_segment_costs_one_reason_call_however_many_samples_it_holds() {
        // The entire cost argument. If this ratio ever becomes per-sample, an
        // eight-hour day costs ~two orders of magnitude more (R19) — and
        // nothing else in the system would report that it had happened.
        let (router, text_calls, reason_calls) = router();
        let samples: Vec<std::path::PathBuf> =
            (0..5).map(|i| std::path::PathBuf::from(format!("/tmp/s{i}.png"))).collect();

        let (out, latency) = summarize_segment_via_ladder(
            &router, &samples, "categorise this segment", 0, 8,
        )
        .await
        .unwrap();
        assert_eq!(latency.samples, 5);
        assert_eq!(latency.perception_calls, 6, "5 text reads + 1 reasoning call");

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

        summarize_segment_via_ladder(&router, &samples, "categorise this segment", 0, 8)
            .await
            .unwrap();

        let seen = reason_prompts.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let prompt = &seen[0];
        assert!(prompt.contains("categorise this segment"), "the caller's ask survives");
        for n in 1..=4 {
            assert_eq!(
                prompt.matches(&format!("served by text-tier #{n}")).count(),
                1,
                "sample {n}'s own extracted text must reach the reasoning tier: {prompt}"
            );
        }
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
            summarize_segment_via_ladder(&router, &samples, "categorise", 0, 8).await.unwrap();
            assert_eq!(text_calls.load(Ordering::SeqCst), n);
            assert_eq!(reason_calls.load(Ordering::SeqCst), 1, "still one, at n={n}");
        }
    }

    #[tokio::test]
    async fn an_empty_segment_is_refused_rather_than_billed() {
        // Sending an empty segment to the reasoning tier would spend a call to
        // describe nothing, and return invention with no grounding.
        let (router, text_calls, reason_calls) = router();
        let err = summarize_segment_via_ladder(&router, &[], "categorise", 0, 8).await;
        assert!(err.is_err());
        assert_eq!(text_calls.load(Ordering::SeqCst), 0);
        assert_eq!(reason_calls.load(Ordering::SeqCst), 0, "nothing is billed");
        assert!(router.escalations().is_empty());
    }
}
