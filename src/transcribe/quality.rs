//! The information content of a piece of text.
//!
//! > "Is this text real content, or a reader that has broken down?"
//!
//! # Why entropy, and never length
//!
//! A vision model reading a screen sometimes loops: it emits one line
//! thousands of times and returns 24k characters for a screen that holds one.
//! Measured on real readings (research.md M3, one good frame and one looped
//! frame from the same model):
//!
//! | metric                 | GOOD  | LOOPED | separation |
//! |------------------------|-------|--------|------------|
//! | length                 | 1,000 | 24,875 | 25×        |
//! | unique lines / total   | 0.955 | 0.003  | **300×**   |
//! | unique tokens / total  | 0.781 | 0.004  | 195×       |
//! | zlib compression ratio | 0.610 | 0.0072 | **85×**    |
//!
//! Length separates the populations by only 25×, and worse, **length is a
//! legitimate property of the content**: a dense page of code is genuinely
//! long, so a character ceiling would truncate real material. No legitimate
//! text has 0.3% unique lines. That is why [`TextQuality`] carries **no length
//! field** — not even as "extra information". A length field invites exactly
//! the character-ceiling mistake M3 rules out, and a caller who wants the
//! length has the text (D015-4).
//!
//! # Scores, not a verdict
//!
//! Nothing here says "rejected". The reject threshold is the caller's: the
//! measured populations are two regimes, not a threshold to tune, and any sane
//! threshold lands between them — which is exactly why the tool need not
//! choose one (D015-4, and the module-level principle in `super`).
//!
//! # Empty text is not a failure
//!
//! An empty screen is a normal thing to read. It scores as empty — every ratio
//! is `0.0`, see [`quality`] — and the caller decides whether an empty screen
//! was expected. It is never an error.
//!
//! This primitive is pure: no OpenCV, no feature gate, no I/O.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// The information content of a piece of text, as three ratios in `0.0..=1.0`.
///
/// Serialises (`serde`) so an agent on any harness can read it from a shell.
/// Nothing here is a verdict, and nothing here is a length — see the module
/// header for why both are deliberate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct TextQuality {
    /// Estimated compressed size ÷ original size, in bytes. **Lower means
    /// more repetitive.** Real content sits around 0.5–0.7; a looped reading
    /// collapses towards 0.01. The estimate is defined precisely at
    /// `compression_ratio` in this file; a caller's threshold is against THAT
    /// definition, which reproduces zlib's two regimes but is not numerically
    /// zlib.
    pub compression_ratio: f64,
    /// Distinct non-blank lines (trimmed) ÷ all non-blank lines. The strongest
    /// separator M3 measured (300×): real content is nearly all distinct lines,
    /// a loop is one line over and over.
    pub unique_line_ratio: f64,
    /// Distinct whitespace-separated tokens ÷ all tokens. Catches a loop that
    /// varies its line breaks but not its words.
    pub unique_token_ratio: f64,
}

/// Score `text`. Deterministic: the same text always yields the same scores.
///
/// # Empty input
///
/// Text with no non-blank line scores `0.0` on every ratio. The scores measure
/// information content and an empty reading carries none, so `0.0` is the
/// honest value — the same region a looped reading lands in. That is by
/// design: the scores do not distinguish "empty" from "degenerate" because a
/// caller who needs to knows more than the scores do — it has the text, and
/// `text.trim().is_empty()` is the question to ask. Encoding it here would be
/// the first step back towards a length field.
///
/// The alternative, `1.0` ("vacuously all unique"), would let an empty reading
/// pass any threshold as if it were rich content, which hides the case the
/// caller is supposed to decide.
///
/// "Empty" is decided ONCE, here, as `text.trim().is_empty()`, so the three
/// scores cannot disagree about it: whitespace-only text has no lines and no
/// tokens, and the compression estimate would otherwise score the mix of
/// space, tab and newline bytes as (a little) information it is not.
pub fn quality(text: &str) -> TextQuality {
    if text.trim().is_empty() {
        return TextQuality {
            compression_ratio: 0.0,
            unique_line_ratio: 0.0,
            unique_token_ratio: 0.0,
        };
    }
    TextQuality {
        compression_ratio: compression_ratio(text.as_bytes()),
        unique_line_ratio: unique_line_ratio(text),
        unique_token_ratio: unique_token_ratio(text),
    }
}

/// Distinct ÷ total, or `0.0` when there is nothing to count.
fn distinct_ratio<'a>(items: impl Iterator<Item = &'a str>) -> f64 {
    let mut total = 0usize;
    let mut seen: HashSet<&str> = HashSet::new();
    for item in items {
        total += 1;
        seen.insert(item);
    }
    if total == 0 {
        0.0
    } else {
        seen.len() as f64 / total as f64
    }
}

/// Lines are compared TRIMMED, and blank lines are not lines. A reader that
/// indents inconsistently has not produced new information, and a run of
/// blank lines is padding, not content — counting either would let padding
/// masquerade as variety.
fn unique_line_ratio(text: &str) -> f64 {
    distinct_ratio(text.lines().map(str::trim).filter(|l| !l.is_empty()))
}

fn unique_token_ratio(text: &str) -> f64 {
    distinct_ratio(text.split_whitespace())
}

// ── Compression estimate ─────────────────────────────────────────────────────
//
// M3 measured zlib, and zlib (DEFLATE) is two mechanisms: LZ77 back-references
// for repeated byte runs, and Huffman coding for whatever remains. This
// estimate models both without pulling a compression crate in for a number
// that is a proxy anyway:
//
//   * LZ77, greedy, over a 32 KiB window with DEFLATE's own match bounds
//     (3..=258 bytes). A back-reference is charged a flat MATCH_BITS.
//   * Each literal byte is charged the order-0 Shannon entropy of the whole
//     text's byte histogram — an idealised Huffman code, no table overhead.
//   * A match is taken only when it is cheaper than the literals it replaces,
//     so the estimate never exceeds the literal-only cost, and the ratio never
//     exceeds 1.0.
//
// No container overhead (no zlib header/trailer, no block headers), so the
// number is a property of the text alone and a tiny input does not score
// above 1.0. Consequence: real content scores near zlib's figure (≈0.6 in
// M3), a loop collapses the same way zlib does (every 258-byte run costs 3
// bytes), and the two regimes are preserved — but the absolute values are an
// ESTIMATE, not zlib's output. A caller sets its threshold against this
// definition, which is why it is spelled out here.

/// DEFLATE's sliding window.
const WINDOW: usize = 32 * 1024;
/// DEFLATE's minimum and maximum match lengths.
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// How many earlier occurrences of a 3-byte prefix to examine before settling
/// for the best seen. Bounds the work on repetitive input; zlib's default level
/// uses the same figure.
const MAX_CHAIN: usize = 128;
/// Cost of one back-reference: roughly a length code plus a 15-bit distance,
/// before entropy coding.
const MATCH_BITS: f64 = 24.0;

/// Estimated compressed bytes ÷ original bytes, per the scheme above.
/// `0.0` for empty input (nothing to compress, nothing to compare).
fn compression_ratio(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    estimated_compressed_bits(bytes) / 8.0 / bytes.len() as f64
}

/// Order-0 Shannon entropy of the byte histogram, in bits per byte.
/// A text of one repeated byte has zero entropy: it carries no information.
fn literal_bits(bytes: &[u8]) -> f64 {
    let mut hist = [0usize; 256];
    for &b in bytes {
        hist[usize::from(b)] += 1;
    }
    let n = bytes.len() as f64;
    hist.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Greedy LZ77 pass with a hash chain on 3-byte prefixes, charging literals at
/// the order-0 entropy and matches at `MATCH_BITS`. Returns total bits.
///
/// The hash map is only ever looked up by key, never iterated, so the result
/// does not depend on hashing order: the same bytes always cost the same bits.
fn estimated_compressed_bits(bytes: &[u8]) -> f64 {
    let n = bytes.len();
    let per_literal = literal_bits(bytes);
    // Most recent position of each 3-byte prefix, and for each position the
    // previous position with the same prefix — zlib's head/prev tables.
    let mut head: HashMap<[u8; 3], usize> = HashMap::new();
    let mut prev: Vec<Option<usize>> = vec![None; n];
    let key = |p: usize| [bytes[p], bytes[p + 1], bytes[p + 2]];

    let mut bits = 0.0_f64;
    let mut i = 0usize;
    while i < n {
        let mut best_len = 0usize;
        if i + MIN_MATCH <= n {
            let max_len = (n - i).min(MAX_MATCH);
            let mut cand = head.get(&key(i)).copied();
            let mut chain = 0usize;
            while let Some(c) = cand {
                if i - c > WINDOW || chain >= MAX_CHAIN {
                    break;
                }
                // Overlap (c + len reaching past i) is allowed: that is how a
                // single repeated byte compresses, and the bytes compared are
                // all already emitted.
                let mut len = 0usize;
                while len < max_len && bytes[c + len] == bytes[i + len] {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    if len == max_len {
                        break;
                    }
                }
                cand = prev[c];
                chain += 1;
            }
        }
        let take_match = best_len >= MIN_MATCH && MATCH_BITS < best_len as f64 * per_literal;
        let step = if take_match {
            bits += MATCH_BITS;
            best_len
        } else {
            bits += per_literal;
            1
        };
        // Every position covered by this token enters the dictionary, matched
        // or not, so later matches can start anywhere — as zlib does.
        for (p, slot) in prev.iter_mut().enumerate().skip(i).take(step) {
            if p + MIN_MATCH <= n {
                *slot = head.insert(key(p), p);
            }
        }
        i += step;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny deterministic generator, so the "good" population is varied
    /// without depending on a random crate or on the test's own text.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
        fn ident(&mut self) -> String {
            let len = 3 + (self.next() % 8) as usize;
            (0..len)
                .map(|_| char::from(b'a' + (self.next() % 26) as u8))
                .collect()
        }
        /// One line shaped like code: keywords repeat, identifiers do not.
        fn code_line(&mut self) -> String {
            format!(
                "let {} = {}({}, {});\n",
                self.ident(),
                self.ident(),
                self.ident(),
                self.ident()
            )
        }
    }

    /// `lines` distinct code-shaped lines — the GOOD population.
    fn good_reading(lines: usize, seed: u64) -> String {
        let mut g = Lcg(seed);
        (0..lines).map(|_| g.code_line()).collect()
    }

    /// One line, `times` over — the LOOPED population: a reader that broke
    /// down and repeated itself, which is what M3's f_0019 looked like.
    fn looped_reading(times: usize) -> String {
        Lcg(7).code_line().repeat(times)
    }

    /// Every ratio is 0.0 for empty input, and whitespace-only IS empty: a
    /// screen of blank lines carries no more information than no screen.
    /// Never an error — the caller decides whether empty was expected.
    #[test]
    fn empty_text_scores_as_empty_not_as_an_error() {
        for text in ["", "   ", "\n\n\n", " \t\n  \n"] {
            let q = quality(text);
            assert_eq!(
                q,
                TextQuality {
                    compression_ratio: 0.0,
                    unique_line_ratio: 0.0,
                    unique_token_ratio: 0.0,
                },
                "{text:?} must score as empty"
            );
        }
    }

    /// THE PROPERTY THIS PRIMITIVE EXISTS FOR (M3, D015-4): the scores put a
    /// real reading and a looped reading in different REGIMES, far apart.
    ///
    /// Pinned here on synthetic populations, so the assertions are on
    /// separation and ordering rather than M3's exact floats (0.955 vs 0.003
    /// unique lines, 0.610 vs 0.0072 compression), which a synthetic fixture
    /// will not reproduce to the digit. The margins asserted (≥ 10× on every
    /// score) are an order of magnitude looser than M3 measured (85×–300×), so
    /// a regression that closed the gap by even that much would fail.
    #[test]
    fn a_real_reading_and_a_looped_reading_are_separated_by_a_wide_margin() {
        let good = quality(&good_reading(40, 1));
        let looped = quality(&looped_reading(500));

        // Ordering: every score is higher for real content.
        assert!(good.unique_line_ratio > looped.unique_line_ratio, "{good:?} vs {looped:?}");
        assert!(good.unique_token_ratio > looped.unique_token_ratio, "{good:?} vs {looped:?}");
        assert!(good.compression_ratio > looped.compression_ratio, "{good:?} vs {looped:?}");

        // Regions: real content is nearly all distinct and barely compresses;
        // a loop is almost nothing distinct and compresses to almost nothing.
        assert!(good.unique_line_ratio > 0.9, "{good:?}");
        assert!(looped.unique_line_ratio < 0.01, "{looped:?}");
        assert!(good.unique_token_ratio > 0.5, "{good:?}");
        assert!(looped.unique_token_ratio < 0.01, "{looped:?}");
        assert!(good.compression_ratio > 0.3, "{good:?}");
        assert!(looped.compression_ratio < 0.05, "{looped:?}");

        // Separation: two regimes, not a threshold to tune.
        for (name, g, l) in [
            ("unique_line_ratio", good.unique_line_ratio, looped.unique_line_ratio),
            ("unique_token_ratio", good.unique_token_ratio, looped.unique_token_ratio),
            ("compression_ratio", good.compression_ratio, looped.compression_ratio),
        ] {
            assert!(
                g / l > 10.0,
                "{name} must separate the populations by ≥10× (M3 measured 85×–300×); \
                 got {g} vs {l} = {:.1}×",
                g / l
            );
        }
    }

    /// LENGTH ALONE DOES NOT SEPARATE THEM — the reason there is no length
    /// field. Here the real reading is LONGER than the looped one (a dense
    /// page of code is legitimately long), so a character ceiling that
    /// rejected the loop would reject the real content too, while every score
    /// still puts the long real reading in the good region.
    #[test]
    fn length_does_not_separate_the_populations_but_the_scores_do() {
        let long_good_text = good_reading(1500, 2);
        let looped_text = looped_reading(500);
        assert!(
            long_good_text.len() > looped_text.len(),
            "fixture: the real reading must be the LONGER one ({} vs {} bytes)",
            long_good_text.len(),
            looped_text.len()
        );

        let long_good = quality(&long_good_text);
        let looped = quality(&looped_text);
        assert!(long_good.unique_line_ratio > 0.9, "{long_good:?}");
        assert!(long_good.unique_token_ratio > 0.5, "{long_good:?}");
        assert!(long_good.compression_ratio > 0.3, "{long_good:?}");
        assert!(long_good.compression_ratio / looped.compression_ratio > 10.0);
        assert!(long_good.unique_line_ratio / looped.unique_line_ratio > 10.0);
    }

    /// The serialised form carries exactly the three ratios and NOTHING that
    /// measures length. A `length`, `len`, `chars`, `bytes`, `lines` or
    /// `tokens` field added "for information" fails here — that is the
    /// character-ceiling mistake M3 rules out (T007: NO length field).
    #[test]
    fn serialises_the_three_ratios_and_no_length() {
        let v = serde_json::to_value(quality("a b\nc d\n")).expect("serialises");
        let obj = v.as_object().expect("an object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["compression_ratio", "unique_line_ratio", "unique_token_ratio"],
            "exactly three ratios, no length, no verdict"
        );
    }

    /// Same text, same scores. The compression estimate uses a hash map, so
    /// this pins that its result never depends on hashing order.
    #[test]
    fn is_deterministic() {
        let text = good_reading(200, 3) + &looped_reading(50);
        assert_eq!(quality(&text), quality(&text));
    }

    /// Repetition LOWERS the compression ratio — the direction the whole
    /// estimate rests on. A function returning a constant passes the
    /// separation test's ordering only by luck of fixture; this pins the
    /// mechanism on one text against its own repeat.
    #[test]
    fn repeating_a_text_lowers_its_compression_ratio() {
        let once = good_reading(20, 4);
        let many = once.repeat(50);
        let a = quality(&once).compression_ratio;
        let b = quality(&many).compression_ratio;
        assert!(b < a, "repeat must compress better: once={a}, x50={b}");
        assert!(a / b > 5.0, "and substantially so: {:.1}×", a / b);
        assert!((0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b));
    }

    /// A text of a single repeated byte carries no information: zero entropy,
    /// and the estimate says so rather than charging a byte per literal.
    #[test]
    fn a_single_repeated_byte_has_no_information() {
        let q = quality(&"a".repeat(1000));
        assert_eq!(q.compression_ratio, 0.0, "{q:?}");
    }

    /// Lines are compared trimmed and blank lines do not count: indentation
    /// drift and padding are not variety.
    #[test]
    fn lines_are_trimmed_and_blank_lines_are_not_lines() {
        let q = quality("  hello\n\nhello\t\n\n\n  world\n");
        // "hello" twice (same line, different whitespace) + "world": 2 of 3.
        assert!((q.unique_line_ratio - 2.0 / 3.0).abs() < 1e-12, "{q:?}");
        assert_eq!(quality("x\n").unique_line_ratio, 1.0);
    }

    /// Tokens split on any whitespace, so a loop that varies its line breaks
    /// but not its words is still caught by the token ratio.
    #[test]
    fn tokens_catch_a_loop_that_rewraps_its_lines() {
        let q = quality("alpha beta\ngamma alpha\nbeta gamma\n");
        assert!((q.unique_token_ratio - 0.5).abs() < 1e-12, "{q:?}");
        assert_eq!(q.unique_line_ratio, 1.0, "{q:?}");
    }
}
