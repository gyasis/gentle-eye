//! Primitive 5 — **align**: what was on screen when this was said.
//!
//! Pure interval arithmetic over timestamps: for every utterance of a
//! transcript, the frames whose timestamps fall inside a window around it, each
//! carrying its own sharpness. No OpenCV, no feature gate — this lives in the
//! default build like [`super::frames`].
//!
//! # Why this pairing is worth computing
//!
//! Measured on the two reference recordings (spec.md, Scope Amendment
//! 2026-09-06): every substantive finding came from the CORRELATION of speech
//! and screen, not from either alone. "Concat. I'm going to do HCC. I think
//! that'll work" is a plan; an autocomplete list open on `concat` with a red
//! squiggle on `G.hcc_gap` is a state; together they timestamp the moment a
//! fix was written. Each channel also covers the other's blind spot — speech
//! is full of deixis ("this table here") that only the frames resolve, and a
//! screen filmed from too far back never resolves and only the speech does.
//!
//! # Pairs, not conclusions
//!
//! Whether an utterance EXPLAINS what is on screen is judgement, and belongs
//! to the caller. This module never ranks frames, never scores them for
//! relevance, never picks "the best" one: every frame inside the window is
//! returned, with its sharpness, so the caller can choose the most legible —
//! or all of them. The score passes through; it does not decide.
//!
//! # No pairing is not a failure
//!
//! Silence over a screen change, and speech over a static screen, are both
//! real. An utterance with no frame in its window is a legitimate
//! [`Alignment`] with an empty `frames` list — never an error, never an
//! omission. A transcript row is never dropped for lack of a frame.
//!
//! # The window is the caller's, and it is TWO numbers
//!
//! Speech and action are offset, and in BOTH directions: a speaker says "we're
//! going to change this" and then changes it (speech LEADS the screen — the
//! state that matters comes AFTER the words), or changes it and then says "so
//! that's why it wasn't working" (speech TRAILS the screen — the state that
//! matters came BEFORE the words). Those are different offsets, and they vary
//! per speaker. A single symmetric tolerance cannot express "two seconds
//! before, eight seconds after", so [`AlignOpts`] carries [`AlignOpts::lead_s`]
//! and [`AlignOpts::lag_s`] separately. The defaults are a starting point the
//! caller can see, not a measurement of anyone's speech.
//!
//! # The transcript is INPUT
//!
//! This module does no ASR. VoxStruct is the house tool for producing a
//! transcript — its `segments` are `[{start, end, text, speaker?}]`, which is
//! exactly the [`Utterance`] shape — and a second speech-to-text path here
//! would be the duplication primitive 3 exists to close.
//!
//! # The frames come straight from primitive 1
//!
//! [`FrameRow`] carries `timestamp_s` — ffmpeg's own presentation time for the
//! frame, read back from the extraction — so the rows `extract_frames`
//! returns are aligned as they are. There is deliberately NO helper that
//! infers a time from `index` and a rate: under any [`super::frames::Dedup`]
//! but `None` the index counts KEPT frames, so `index / fps` is plausible,
//! wrong, and undetectable from the rows. A caller with frames from some
//! other source builds `FrameRow`s with the times they actually know.

use super::frames::FrameRow;
use serde::Serialize;

/// One utterance of a transcript: what was said, and when.
///
/// This is VoxStruct's segment shape — `start`, `end`, `text` — with the units
/// in the names. Times are seconds from the start of the recording, `end_s >=
/// start_s`, both finite and non-negative; anything else is refused by
/// [`align`] with [`AlignError::BadUtterance`]. A zero-length utterance
/// (`start_s == end_s`, a single word-level timestamp) is legal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Utterance {
    /// When the speech starts, in seconds from the start of the recording.
    pub start_s: f64,
    /// When it ends. Equal to `start_s` for a point timestamp.
    pub end_s: f64,
    pub text: String,
}

/// Default [`AlignOpts::lead_s`]: 3 s of screen before the utterance starts.
///
/// A starting point, NOT a measurement. It is symmetric with
/// [`DEFAULT_LAG_S`] only because nobody has yet measured a speaker's offset
/// in either direction; the two exist as separate knobs precisely so a caller
/// who has measured one can set it without moving the other.
pub const DEFAULT_LEAD_S: f64 = 3.0;

/// Default [`AlignOpts::lag_s`]: 3 s of screen after the utterance ends. See
/// [`DEFAULT_LEAD_S`].
pub const DEFAULT_LAG_S: f64 = 3.0;

/// The window around each utterance. Both knobs are the caller's, with a
/// documented default.
///
/// A frame at time `t` is inside the window of an utterance when
/// `start_s - lead_s <= t <= end_s + lag_s` (both ends inclusive). The
/// utterance's own duration is always inside: a frame taken WHILE the words
/// were being said is the most literal answer to "what was on screen when
/// this was said", so the window is `[start, end]` widened, never a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignOpts {
    /// How far the window reaches BEFORE the utterance starts, in seconds.
    /// Covers speech that TRAILS the screen: the change was made, then
    /// described ("so that's why it wasn't working"). Finite and
    /// non-negative. Default [`DEFAULT_LEAD_S`].
    pub lead_s: f64,
    /// How far the window reaches AFTER the utterance ends, in seconds.
    /// Covers speech that LEADS the screen: the change was announced, then
    /// made ("I'm going to do HCC"). Finite and non-negative. Default
    /// [`DEFAULT_LAG_S`].
    pub lag_s: f64,
}

impl Default for AlignOpts {
    fn default() -> Self {
        Self { lead_s: DEFAULT_LEAD_S, lag_s: DEFAULT_LAG_S }
    }
}

/// One utterance and every frame inside its window. Serialises (`serde`) so an
/// agent on any harness can read it from a shell. Nothing here is a verdict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alignment {
    pub utterance: Utterance,
    /// `utterance.start_s - lead_s`, clamped at 0 — the recording has no
    /// negative time. Reported so the caller can see the window that was
    /// actually applied without recomputing it.
    pub window_start_s: f64,
    /// `utterance.end_s + lag_s`.
    pub window_end_s: f64,
    /// Every frame with `window_start_s <= timestamp_s <= window_end_s`, in
    /// time order (ties by frame index, then path), each the row primitive 1
    /// produced — `{index, timestamp_s, path, sharpness}`, flat. **May be
    /// empty**, and an empty list is a real answer: nothing was sampled while
    /// this was said. Not ranked, not filtered — the sharpness on each row is
    /// for the CALLER to pick by, and it is comparable within this recording
    /// only.
    pub frames: Vec<FrameRow>,
}

/// Why an alignment could not be produced. Every variant is a STATED failure:
/// none of them surfaces as an empty result.
#[derive(Debug, thiserror::Error)]
pub enum AlignError {
    /// A tolerance that cannot be applied — a negative or non-finite
    /// `lead_s`/`lag_s`. Refused up front, before any work: a NaN compares
    /// false against everything and would make every window empty, which
    /// would then read as "nothing was on screen".
    #[error("bad option: {0}")]
    BadOption(String),
    /// The transcript has no utterances. An empty transcript aligns to
    /// nothing, and an empty result must not be mistaken for "no utterance had
    /// a frame" — that case is a list of rows with empty `frames`.
    #[error("no utterances: the transcript is empty, so there is nothing to align (an utterance with no frames is a row, not an absence)")]
    NoUtterances,
    /// No frames at all. Against zero frames every row would be empty, and
    /// "nothing was sampled" is a fact about the EXTRACTION (which states its
    /// own failures) — it must not come out of this call looking like a
    /// property of the speech.
    #[error("no frames: nothing to align against — every row would be empty, and that is the extraction's failure to state, not this call's")]
    NoFrames,
    /// An utterance whose times are not a span in a recording: non-finite,
    /// negative, or `end_s < start_s`. `index` is its position in the input.
    #[error("bad utterance {index}: {reason}")]
    BadUtterance { index: usize, reason: String },
    /// A frame whose `timestamp_s` is not a position in a recording:
    /// non-finite or negative. `index` is its position in the input slice,
    /// not `FrameRow::index`.
    #[error("bad frame {index}: {reason}")]
    BadFrame { index: usize, reason: String },
}

/// Refuse a window that cannot be applied. See [`AlignError::BadOption`].
fn check_opts(opts: AlignOpts) -> Result<(), AlignError> {
    for (name, v) in [("lead_s", opts.lead_s), ("lag_s", opts.lag_s)] {
        if !v.is_finite() || v < 0.0 {
            return Err(AlignError::BadOption(format!("{name} must be finite and non-negative, got {v}")));
        }
    }
    Ok(())
}

/// Refuse a time that is not a position in a recording.
fn check_seconds(v: f64) -> Result<(), String> {
    if !v.is_finite() {
        return Err(format!("time is not finite ({v})"));
    }
    if v < 0.0 {
        return Err(format!("time is negative ({v}); a recording starts at 0"));
    }
    Ok(())
}

/// For every utterance, the frames inside its window.
///
/// Returns exactly one [`Alignment`] per input utterance — none is ever
/// dropped, and one with no frame in its window is a row with an empty
/// `frames` list. A frame inside the windows of SEVERAL utterances appears
/// under each of them: real speech is dense (two utterances two seconds apart
/// are ordinary), so windows overlap constantly, and the frame on screen
/// during both was on screen during both. Assigning it to only one would make
/// the other row lie. A caller who wants each frame once can dedupe by
/// `frame.index`; the pairs are theirs.
///
/// # Determinism
///
/// Same input, same output, regardless of input order. Utterances come back
/// sorted by `start_s`, then `end_s`, then their position in the input;
/// frames within a row by `timestamp_s`, then `FrameRow::index`, then `path`.
/// No step depends on hash-map iteration.
///
/// # Errors
///
/// Every refusal is stated ([`AlignError`]): a bad window, an empty
/// transcript, an empty frame list, or a time that is not a position in a
/// recording. All inputs are validated BEFORE any pairing, so a bad row late
/// in the input is refused rather than half-aligned.
pub fn align(transcript: &[Utterance], frames: &[FrameRow], opts: AlignOpts) -> Result<Vec<Alignment>, AlignError> {
    check_opts(opts)?;
    if transcript.is_empty() {
        return Err(AlignError::NoUtterances);
    }
    if frames.is_empty() {
        return Err(AlignError::NoFrames);
    }
    for (index, u) in transcript.iter().enumerate() {
        let bad = |reason: String| AlignError::BadUtterance { index, reason };
        check_seconds(u.start_s).map_err(|r| bad(format!("start_s: {r}")))?;
        check_seconds(u.end_s).map_err(|r| bad(format!("end_s: {r}")))?;
        if u.end_s < u.start_s {
            return Err(bad(format!("end_s {} is before start_s {}", u.end_s, u.start_s)));
        }
    }
    for (index, f) in frames.iter().enumerate() {
        check_seconds(f.timestamp_s).map_err(|reason| AlignError::BadFrame { index, reason })?;
    }

    // Every time is finite from here on, so total_cmp is a plain numeric order.
    let mut frames: Vec<&FrameRow> = frames.iter().collect();
    frames.sort_by(|a, b| {
        a.timestamp_s
            .total_cmp(&b.timestamp_s)
            .then(a.index.cmp(&b.index))
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut utterances: Vec<&Utterance> = transcript.iter().collect();
    // Stable, so equal (start, end) keep their input order.
    utterances.sort_by(|a, b| a.start_s.total_cmp(&b.start_s).then(a.end_s.total_cmp(&b.end_s)));

    Ok(utterances
        .into_iter()
        .map(|u| {
            let window_start_s = (u.start_s - opts.lead_s).max(0.0);
            let window_end_s = u.end_s + opts.lag_s;
            // Frames are sorted by time, so the window is one contiguous run:
            // first frame at or after the start, first frame past the end.
            let lo = frames.partition_point(|f| f.timestamp_s < window_start_s);
            let hi = frames.partition_point(|f| f.timestamp_s <= window_end_s);
            Alignment {
                utterance: u.clone(),
                window_start_s,
                window_end_s,
                frames: frames[lo..hi].iter().map(|f| (*f).clone()).collect(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(start_s: f64, end_s: f64, text: &str) -> Utterance {
        Utterance { start_s, end_s, text: text.to_string() }
    }

    fn f(index: usize, timestamp_s: f64, sharpness: f64) -> FrameRow {
        FrameRow { index, timestamp_s, path: format!("f_{index:05}.png").into(), sharpness }
    }

    fn indices(a: &Alignment) -> Vec<usize> {
        a.frames.iter().map(|t| t.index).collect()
    }

    /// The realistic shape from the amendment's evidence: the "Concat. / I'm
    /// going to do HCC" exchange at 12:07 and 12:09, frames at 5-second
    /// intervals over the span the caller selected (12:00–12:35 — transcript-
    /// driven span selection cuts frames to the spans worth processing, while
    /// the transcript covers the whole call), sharpness from M4's measured
    /// range. A later utterance at 14:00 has no frames anywhere near it.
    fn fixture() -> (Vec<Utterance>, Vec<FrameRow>) {
        let transcript = vec![
            u(727.0, 727.8, "Concat."),
            u(729.0, 730.6, "I'm going to do HCC"),
            u(840.0, 842.2, "and that one's fine"),
        ];
        let sharp = [1443.0, 396.0, 1458.0, 507.0, 1450.0, 421.0, 1447.0, 1455.0];
        let frames: Vec<FrameRow> =
            (0..8).map(|k| f(144 + k, 720.0 + 5.0 * k as f64, sharp[k])).collect();
        (transcript, frames)
    }

    /// The pairing the amendment describes: the frame at 12:10 (the 5-second
    /// grid's nearest to 12:09) was on screen during BOTH the 12:07 and the
    /// 12:09 utterance at 3 s lead / 3 s lag, so it appears under both. And
    /// the 14:00 utterance, with no frame within 3 s, is a ROW with an empty
    /// list — not dropped.
    #[test]
    fn a_frame_is_shared_by_overlapping_windows_and_an_empty_row_survives() {
        let (t, fr) = fixture();
        let rows = align(&t, &fr, AlignOpts::default()).unwrap();
        assert_eq!(rows.len(), 3, "one row per utterance, always");

        // "Concat." at 727.0–727.8 → window [724.0, 730.8] → 725 (idx 145), 730 (idx 146)
        assert_eq!(rows[0].utterance.text, "Concat.");
        assert_eq!((rows[0].window_start_s, rows[0].window_end_s), (724.0, 730.8));
        assert_eq!(indices(&rows[0]), vec![145, 146]);

        // "I'm going to do HCC" at 729.0–730.6 → window [726.0, 733.6] → 730 (idx 146)
        assert_eq!(rows[1].utterance.text, "I'm going to do HCC");
        assert_eq!(indices(&rows[1]), vec![146]);
        assert!(
            indices(&rows[0]).contains(&146) && indices(&rows[1]).contains(&146),
            "the 12:10 frame was on screen during both utterances and must appear under both"
        );

        // 14:00 → window [837.0, 845.2] → nothing was sampled there
        assert_eq!(rows[2].utterance.text, "and that one's fine");
        assert!(rows[2].frames.is_empty(), "no pairing is a row, not an omission");

        // Sharpness passes through untouched, unranked: idx 145 scored 396
        // (M4's "all failed" band) and sits BEFORE idx 146's 1458 because the
        // order is time, not legibility. The caller picks.
        let s: Vec<f64> = rows[0].frames.iter().map(|t| t.sharpness).collect();
        assert_eq!(s, vec![396.0, 1458.0]);
    }

    /// The reason the window is two numbers. With the same utterance and the
    /// same frames, lead-only and lag-only windows select DIFFERENT frames;
    /// a single symmetric tolerance could express neither.
    #[test]
    fn lead_and_lag_are_independent_and_asymmetric() {
        let t = vec![u(100.0, 101.0, "so that's why it wasn't working")];
        let fr = vec![f(0, 95.0, 1.0), f(1, 98.0, 1.0), f(2, 100.5, 1.0), f(3, 103.0, 1.0), f(4, 106.0, 1.0)];

        // Speech TRAILING the screen: look back 4 s, not forward at all.
        let back = align(&t, &fr, AlignOpts { lead_s: 4.0, lag_s: 0.0 }).unwrap();
        assert_eq!(indices(&back[0]), vec![1, 2], "window [96, 101]");

        // Speech LEADING the screen: look forward 4 s, not back at all.
        let fwd = align(&t, &fr, AlignOpts { lead_s: 0.0, lag_s: 4.0 }).unwrap();
        assert_eq!(indices(&fwd[0]), vec![2, 3], "window [100, 105]");

        // Zero both ways: only the frame taken DURING the words.
        let during = align(&t, &fr, AlignOpts { lead_s: 0.0, lag_s: 0.0 }).unwrap();
        assert_eq!(indices(&during[0]), vec![2], "the utterance's own span is always inside the window");

        // Lead 2 / lag 6 ≠ lead 6 / lag 2 — the asymmetry is real, not a rename.
        let a = align(&t, &fr, AlignOpts { lead_s: 2.0, lag_s: 6.0 }).unwrap();
        let b = align(&t, &fr, AlignOpts { lead_s: 6.0, lag_s: 2.0 }).unwrap();
        assert_eq!(indices(&a[0]), vec![1, 2, 3, 4]);
        assert_eq!(indices(&b[0]), vec![0, 1, 2, 3]);
        assert_ne!(indices(&a[0]), indices(&b[0]));
    }

    /// Both window ends are inclusive, and the lead is clamped at 0 — a
    /// recording has no negative time, and the report says the window that
    /// was actually applied.
    #[test]
    fn window_ends_are_inclusive_and_never_negative() {
        let t = vec![u(1.0, 2.0, "x")];
        let fr = vec![f(0, 0.0, 1.0), f(1, 4.0, 1.0), f(2, 4.000001, 1.0)];
        let rows = align(&t, &fr, AlignOpts { lead_s: 5.0, lag_s: 2.0 }).unwrap();
        assert_eq!(rows[0].window_start_s, 0.0, "1 - 5 clamps to 0, and is reported as 0");
        assert_eq!(rows[0].window_end_s, 4.0);
        assert_eq!(indices(&rows[0]), vec![0, 1], "t=0 is on the start edge, t=4 on the end edge, t=4.000001 is out");
    }

    /// A zero-length utterance — a word-level timestamp — is legal, and its
    /// window is just the tolerances around the point.
    #[test]
    fn a_point_utterance_is_legal() {
        let t = vec![u(50.0, 50.0, "Concat.")];
        let fr = vec![f(0, 48.0, 1.0), f(1, 50.0, 1.0), f(2, 52.0, 1.0)];
        let rows = align(&t, &fr, AlignOpts { lead_s: 1.0, lag_s: 1.0 }).unwrap();
        assert_eq!(indices(&rows[0]), vec![1]);
        let rows = align(&t, &fr, AlignOpts { lead_s: 2.0, lag_s: 2.0 }).unwrap();
        assert_eq!(indices(&rows[0]), vec![0, 1, 2]);
    }

    /// Order in, order out: shuffled utterances and shuffled frames produce
    /// the same rows, in the same order, as sorted input. Ties between frames
    /// at the same second break by index, never by input position.
    #[test]
    fn unsorted_input_gives_the_same_deterministic_output() {
        let (t, fr) = fixture();
        let sorted = align(&t, &fr, AlignOpts::default()).unwrap();

        let mut t2 = t.clone();
        t2.reverse();
        let mut fr2 = fr.clone();
        fr2.swap(0, 7);
        fr2.swap(2, 5);
        fr2.rotate_left(3);
        let shuffled = align(&t2, &fr2, AlignOpts::default()).unwrap();
        assert_eq!(sorted, shuffled);

        // Two frames at the same second (a caller-supplied duplicate): the
        // lower index comes first whichever was handed in first.
        let t = vec![u(10.0, 10.0, "x")];
        let dup_a = vec![f(7, 10.0, 1.0), f(3, 10.0, 2.0)];
        let dup_b = vec![f(3, 10.0, 2.0), f(7, 10.0, 1.0)];
        let ra = align(&t, &dup_a, AlignOpts::default()).unwrap();
        let rb = align(&t, &dup_b, AlignOpts::default()).unwrap();
        assert_eq!(indices(&ra[0]), vec![3, 7]);
        assert_eq!(ra, rb);

        // Two utterances with identical times keep their input order — the
        // only tie-break that is stable AND meaningful for a transcript.
        let same = vec![u(10.0, 11.0, "first"), u(10.0, 11.0, "second")];
        let rows = align(&same, &dup_a, AlignOpts::default()).unwrap();
        assert_eq!(rows[0].utterance.text, "first");
        assert_eq!(rows[1].utterance.text, "second");
    }

    /// Every refusal is stated. None of these may come back as `Ok(vec![])` or
    /// as rows with empty frame lists — each would be read as "nothing was on
    /// screen", which is a different fact.
    #[test]
    fn every_bad_input_is_a_stated_error() {
        let (t, fr) = fixture();
        let ok = AlignOpts::default();

        assert!(matches!(align(&[], &fr, ok), Err(AlignError::NoUtterances)));
        assert!(matches!(align(&t, &[], ok), Err(AlignError::NoFrames)));

        for bad in [
            AlignOpts { lead_s: -1.0, ..ok },
            AlignOpts { lag_s: -0.5, ..ok },
            AlignOpts { lead_s: f64::NAN, ..ok },
            AlignOpts { lag_s: f64::INFINITY, ..ok },
        ] {
            assert!(matches!(align(&t, &fr, bad), Err(AlignError::BadOption(_))), "{bad:?} must be refused");
        }
        // Options are checked before emptiness, so a bad window is the same
        // first error whatever else is wrong.
        assert!(matches!(align(&[], &[], AlignOpts { lead_s: -1.0, ..ok }), Err(AlignError::BadOption(_))));

        // Utterances: end before start, negative, non-finite — each names its index.
        let cases: [(Utterance, &str); 4] = [
            (u(10.0, 9.0, "backwards"), "end before start"),
            (u(-1.0, 2.0, "negative"), "negative start"),
            (u(1.0, f64::NAN, "nan"), "NaN end"),
            (u(f64::INFINITY, f64::INFINITY, "inf"), "infinite"),
        ];
        for (bad, why) in cases {
            let t2 = vec![t[0].clone(), bad];
            match align(&t2, &fr, ok) {
                Err(AlignError::BadUtterance { index: 1, .. }) => {}
                other => panic!("{why}: expected BadUtterance at index 1, got {other:?}"),
            }
        }

        // Frames: a negative or non-finite time names its position in the input.
        for (bad, why) in [(f(9, -0.001, 1.0), "negative"), (f(9, f64::NAN, 1.0), "NaN")] {
            let fr2 = vec![fr[0].clone(), fr[1].clone(), bad];
            match align(&t, &fr2, ok) {
                Err(AlignError::BadFrame { index: 2, .. }) => {}
                other => panic!("{why}: expected BadFrame at index 2, got {other:?}"),
            }
        }

        // Zero tolerances are legal: they are the edge of the range, not past it.
        check_opts(AlignOpts { lead_s: 0.0, lag_s: 0.0 }).unwrap();
        // And the message says which knob.
        let msg = check_opts(AlignOpts { lead_s: 1.0, lag_s: -2.0 }).unwrap_err().to_string();
        assert!(msg.contains("lag_s") && msg.contains("-2"), "{msg}");
    }

    #[test]
    fn defaults_are_the_documented_constants() {
        let o = AlignOpts::default();
        assert_eq!(o.lead_s, 3.0);
        assert_eq!(o.lag_s, 3.0);
        assert_eq!((DEFAULT_LEAD_S, DEFAULT_LAG_S), (o.lead_s, o.lag_s));
        check_opts(o).unwrap();
    }

    /// The contract's machine-readable requirement: rows serialise with their
    /// field names, frames FLAT — primitive 1's row as it is, not nested under
    /// a `frame` key — an empty `frames` is an empty array, and there is no
    /// verdict field anywhere.
    #[test]
    fn the_output_is_machine_readable_and_carries_no_verdict() {
        let (t, fr) = fixture();
        let rows = align(&t, &fr, AlignOpts::default()).unwrap();
        let v = serde_json::to_value(&rows).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 3);

        let row = arr[0].as_object().unwrap();
        for key in ["utterance", "window_start_s", "window_end_s", "frames"] {
            assert!(row.contains_key(key), "missing {key}: {v}");
        }
        assert_eq!(row["utterance"]["text"], "Concat.");
        assert_eq!(row["utterance"]["start_s"], 727.0);

        let frame = row["frames"][0].as_object().unwrap();
        for key in ["timestamp_s", "index", "path", "sharpness"] {
            assert!(frame.contains_key(key), "missing {key}: {v}");
        }
        assert!(!frame.contains_key("frame"), "frames serialise flat, not nested");
        assert_eq!(frame["timestamp_s"], 725.0);
        assert_eq!(frame["index"], 145);
        assert_eq!(frame["path"], "f_00145.png");
        assert_eq!(frame["sharpness"], 396.0);

        assert_eq!(arr[2]["frames"], serde_json::json!([]), "an empty window is an empty array, present");
        let text = v.to_string();
        for verdict in ["best", "relevant", "score", "explains", "improved"] {
            assert!(!text.contains(verdict), "no verdict field: found {verdict:?} in {text}");
        }
    }
}
