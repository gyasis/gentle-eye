//! The content-identity gate: decide whether a freshly sampled frame is
//! meaningfully different from the previous one.
//!
//! # Two strategies, both shipped
//!
//! Two working systems solved this problem differently, and the difference is
//! not cosmetic — each is robust to a failure mode the other is blind to:
//!
//! | | [`GateStrategy::Magnitude`] (Lookout) | [`GateStrategy::Proportion`] (videolocr) |
//! |---|---|---|
//! | measures | mean absolute difference | fraction of pixels that changed at all |
//! | catches | a large SUBTLE shift — a theme change, a dimmed screen | a small INTENSE change — a cursor, a spinner, one edited line |
//! | blind to | a tiny region changing violently | a whole screen shifting slightly |
//! | tuned at | `6.0` (0–255 scale) | `0.4` fraction |
//!
//! Rather than pick one and inherit its blind spot, both are implemented and
//! selectable, plus [`GateStrategy::Either`], which fires when *either* signal
//! trips. `Either` is the default: for an all-day recorder a false "changed"
//! costs one wasted perception pass, while a false "unchanged" loses the moment
//! permanently.
//!
//! # Fail open
//!
//! Every path here fails toward KEEPING the frame. Dayflow cannot re-capture
//! yesterday, so a gate that errs toward dropping turns any bug into silent data
//! loss. This is the rule videolocr's `_frame_is_informative` states outright:
//! *"FAIL-OPEN: on any error we KEEP the frame — never risk MISSING code."*

use serde::{Deserialize, Serialize};

/// Which change-detection strategy the gate applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GateStrategy {
    /// Mean absolute difference (Lookout). Robust to large subtle shifts.
    Magnitude,
    /// Fraction of pixels that changed at all (videolocr). Robust to small
    /// intense changes.
    Proportion,
    /// Changed if EITHER signal trips. The default — it inherits neither
    /// strategy's blind spot, at the cost of occasionally re-perceiving a frame
    /// that had not meaningfully changed.
    #[default]
    Either,
    /// Changed only if BOTH trip. Cheapest, and the most likely to miss
    /// something; offered for a machine where perception cost dominates.
    Both,
}

/// Why the gate reached its verdict — recorded on the sample so a quiet day is
/// explainable after the fact rather than merely empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateVerdict {
    /// First frame seen for this display; nothing to compare against.
    FirstSight,
    /// Meaningfully different from the previous frame.
    Changed,
    /// Same picture as the previous frame.
    Unchanged,
    /// Uniform/blank — below the content-std floor, nothing worth perceiving.
    Blank,
    /// Comparison could not be performed. **Treated as changed** (fail open).
    Indeterminate,
}

impl GateVerdict {
    /// Whether this verdict means the frame should be perceived.
    pub fn should_perceive(self) -> bool {
        !matches!(self, Self::Unchanged | Self::Blank)
    }

    /// The reason string recorded in `dayflow_samples.skip_reason`.
    pub fn skip_reason(self) -> Option<&'static str> {
        match self {
            Self::Unchanged => Some("unchanged"),
            Self::Blank => Some("blank"),
            _ => None,
        }
    }
}

/// Mean absolute difference between two greyscale buffers (Lookout's method).
///
/// Differing lengths return [`f64::INFINITY`] — a resolution change is a large
/// change, and must never read as "no change".
pub fn mean_abs_diff(prev: &[u8], cur: &[u8]) -> f64 {
    if prev.len() != cur.len() || cur.is_empty() {
        return f64::INFINITY;
    }
    let sum: u64 = prev
        .iter()
        .zip(cur)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    sum as f64 / cur.len() as f64
}

/// Fraction of pixels that differ at all (videolocr's method).
///
/// Differing lengths return `1.0` — everything changed — for the same reason
/// [`mean_abs_diff`] returns infinity.
pub fn changed_fraction(prev: &[u8], cur: &[u8]) -> f64 {
    if prev.len() != cur.len() || cur.is_empty() {
        return 1.0;
    }
    let changed = prev.iter().zip(cur).filter(|(a, b)| a != b).count();
    changed as f64 / cur.len() as f64
}

/// Population standard deviation of a greyscale buffer. Below the configured
/// floor the frame is uniform — a blank screen, a wallpaper, a screensaver.
pub fn gray_std(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let n = bytes.len() as f64;
    let mean = bytes.iter().map(|b| f64::from(*b)).sum::<f64>() / n;
    let var = bytes
        .iter()
        .map(|b| {
            let d = f64::from(*b) - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    var.sqrt()
}

/// Apply the gate to a newly sampled greyscale buffer.
///
/// `prev` is `None` on first sight. Both thresholds are supplied so the caller's
/// configuration stays the single source of truth.
pub fn evaluate(
    prev: Option<&[u8]>,
    cur: &[u8],
    strategy: GateStrategy,
    magnitude_threshold: f64,
    proportion_threshold: f64,
    content_std_floor: f64,
) -> GateVerdict {
    // An empty buffer means the capture or downscale failed. Fail OPEN: we
    // cannot tell whether anything changed, so we must not claim it did not.
    if cur.is_empty() {
        return GateVerdict::Indeterminate;
    }
    if gray_std(cur) < content_std_floor {
        return GateVerdict::Blank;
    }
    let Some(prev) = prev else {
        return GateVerdict::FirstSight;
    };

    let by_magnitude = mean_abs_diff(prev, cur) > magnitude_threshold;
    let by_proportion = changed_fraction(prev, cur) > proportion_threshold;

    let changed = match strategy {
        GateStrategy::Magnitude => by_magnitude,
        GateStrategy::Proportion => by_proportion,
        GateStrategy::Either => by_magnitude || by_proportion,
        GateStrategy::Both => by_magnitude && by_proportion,
    };
    if changed {
        GateVerdict::Changed
    } else {
        GateVerdict::Unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAG: f64 = 6.0; // Lookout GATE_CHANGE
    const PROP: f64 = 0.4; // videolocr change_threshold
    const STD: f64 = 8.0; // Lookout CONTENT_STD

    /// A buffer with enough variation to clear the content-std floor.
    fn textured(n: usize, bias: u8) -> Vec<u8> {
        (0..n)
            .map(|i| ((i * 37) % 256) as u8 ^ bias)
            .collect()
    }

    #[test]
    fn each_strategy_catches_what_the_other_misses() {
        let n = 10_000;
        let base = textured(n, 0);

        // A: a LARGE SUBTLE shift — every pixel moves a little (theme change).
        let mut subtle = base.clone();
        for b in subtle.iter_mut() {
            *b = b.saturating_add(12);
        }
        assert!(mean_abs_diff(&base, &subtle) > MAG, "magnitude sees it");
        assert!(
            changed_fraction(&base, &subtle) > PROP,
            "proportion also sees an every-pixel shift"
        );

        // B: a SMALL INTENSE change — 1% of pixels swing hard (a cursor).
        let mut intense = base.clone();
        for b in intense.iter_mut().take(n / 100) {
            *b = b.wrapping_add(200);
        }
        assert!(
            mean_abs_diff(&base, &intense) < MAG,
            "magnitude is BLIND to a 1% intense change: {}",
            mean_abs_diff(&base, &intense)
        );
        assert!(
            changed_fraction(&base, &intense) < PROP,
            "proportion at 0.4 is also blind to a 1% change"
        );

        // C: a moderate-area change that magnitude misses but proportion catches:
        // half the pixels move by a small amount.
        let mut half = base.clone();
        for b in half.iter_mut().take(n / 2) {
            *b = b.saturating_add(3);
        }
        let m = mean_abs_diff(&base, &half);
        let p = changed_fraction(&base, &half);
        assert!(m < MAG, "magnitude misses it (mean only {m})");
        assert!(p > PROP, "proportion catches it ({p} of pixels moved)");
        // ...and this is exactly why Either exists:
        assert_eq!(
            evaluate(Some(&base), &half, GateStrategy::Magnitude, MAG, PROP, STD),
            GateVerdict::Unchanged
        );
        assert_eq!(
            evaluate(Some(&base), &half, GateStrategy::Either, MAG, PROP, STD),
            GateVerdict::Changed,
            "Either must not inherit Magnitude's blind spot"
        );
    }

    #[test]
    fn identical_frames_are_unchanged_under_every_strategy() {
        let a = textured(5_000, 0);
        for s in [
            GateStrategy::Magnitude,
            GateStrategy::Proportion,
            GateStrategy::Either,
            GateStrategy::Both,
        ] {
            assert_eq!(
                evaluate(Some(&a), &a, s, MAG, PROP, STD),
                GateVerdict::Unchanged,
                "{s:?} must report an identical frame unchanged"
            );
        }
    }

    #[test]
    fn a_resolution_change_can_never_read_as_unchanged() {
        // The dangerous case: buffers of different length. Comparing them
        // element-wise would silently compare a prefix.
        let a = textured(5_000, 0);
        let b = textured(9_000, 0);
        assert_eq!(mean_abs_diff(&a, &b), f64::INFINITY);
        assert_eq!(changed_fraction(&a, &b), 1.0);
        for s in [GateStrategy::Magnitude, GateStrategy::Proportion, GateStrategy::Both] {
            assert_eq!(evaluate(Some(&a), &b, s, MAG, PROP, STD), GateVerdict::Changed);
        }
    }

    #[test]
    fn the_gate_fails_open() {
        // An empty buffer means capture or downscale failed. We cannot tell
        // whether anything changed, so we must NOT claim it did not.
        let prev = textured(5_000, 0);
        let verdict = evaluate(Some(&prev), &[], GateStrategy::Either, MAG, PROP, STD);
        assert_eq!(verdict, GateVerdict::Indeterminate);
        assert!(
            verdict.should_perceive(),
            "fail open: an indeterminate gate must KEEP the frame — dayflow cannot re-capture yesterday"
        );
    }

    #[test]
    fn a_blank_screen_is_recognised_and_skipped() {
        let blank = vec![17u8; 5_000]; // uniform → std 0
        assert!(gray_std(&blank) < STD);
        let v = evaluate(None, &blank, GateStrategy::Either, MAG, PROP, STD);
        assert_eq!(v, GateVerdict::Blank);
        assert!(!v.should_perceive());
        assert_eq!(v.skip_reason(), Some("blank"));
    }

    #[test]
    fn first_sight_is_always_perceived() {
        let a = textured(5_000, 0);
        let v = evaluate(None, &a, GateStrategy::Either, MAG, PROP, STD);
        assert_eq!(v, GateVerdict::FirstSight);
        assert!(v.should_perceive());
        assert_eq!(v.skip_reason(), None);
    }

    #[test]
    fn every_verdict_has_a_definite_perceive_answer() {
        // No verdict may be ambiguous — a sample is either perceived or has a
        // recorded reason it was not.
        for v in [
            GateVerdict::FirstSight,
            GateVerdict::Changed,
            GateVerdict::Unchanged,
            GateVerdict::Blank,
            GateVerdict::Indeterminate,
        ] {
            assert_eq!(
                v.should_perceive(),
                v.skip_reason().is_none(),
                "{v:?}: perceived iff there is no skip reason"
            );
        }
    }
}
