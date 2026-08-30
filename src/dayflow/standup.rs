//! Yesterday in one screen (US7) — a categorised, time-ranged digest.
//!
//! # Proportions come from REAL durations
//!
//! The tempting implementation counts entries per category and multiplies by
//! the configured interval. It is wrong, and wrong in a way that reads as
//! plausible: windows genuinely differ in length. A pause truncates one, an
//! interval change resizes the next, a display disconnects mid-window, and the
//! final window of a session ends when the session does. Counting them treats a
//! 40-second stub and a 15-minute block as equal, so a day with one long
//! meeting and many short interruptions reports the interruptions as the bulk
//! of it — a digest that is confidently, legibly wrong.
//!
//! So every figure here is summed from `end_time - start_time` on the entries
//! themselves.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::dayflow::models::{ActivityCategory, TimelineEntry};

/// One category's share of a day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategorySlice {
    /// The category.
    pub category: ActivityCategory,
    /// Wall-clock time in this category, counting overlap within it once.
    pub seconds: i64,
    /// Share of [`Standup::attributed_seconds`], 0–100, rounded to one decimal.
    ///
    /// Each slice rounds independently, so the column can sum to slightly more
    /// or less than 100 — with seven categories, measured drift is at most 0.3.
    pub percent: f64,
    /// How many entries contributed — reported ALONGSIDE the duration, never
    /// instead of it, so a reader can see "eleven short interruptions" and "one
    /// long meeting" as the different things they are.
    pub entries: usize,
    /// The activity labels seen, most time first.
    pub activities: Vec<String>,
}

/// A day, categorised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Standup {
    /// Start of the range digested.
    pub from: DateTime<Utc>,
    /// End of the range digested.
    pub to: DateTime<Utc>,
    /// Wall-clock time actually covered by recordings, counting overlap ONCE.
    ///
    /// NOT the span of the range: a four-hour range with twenty minutes of
    /// entries recorded twenty minutes, and saying otherwise attributes
    /// unrecorded time to whatever happened to be nearby.
    ///
    /// And NOT the sum of entry durations: windows are per display, so a
    /// two-monitor session produces two entries covering the same minute.
    pub recorded_seconds: i64,
    /// Time attributed to categories, which can EXCEED `recorded_seconds` when
    /// two displays were doing different things at once.
    ///
    /// Reported separately rather than reconciled away: the difference is the
    /// concurrency, and hiding it would make a two-monitor day look either
    /// twice as long or half as busy depending on which number was chosen.
    pub attributed_seconds: i64,
    /// Wall-clock span of the range, for contrast with the above.
    pub span_seconds: i64,
    /// Categories, largest share first.
    pub categories: Vec<CategorySlice>,
}

impl Standup {
    /// Whether the range is mostly unrecorded.
    ///
    /// Surfaced because a digest covering 8% of its range is not a summary of
    /// the day — it is a summary of the fragments that were captured, and a
    /// reader who cannot tell those apart will over-trust it.
    pub fn is_sparse(&self) -> bool {
        self.span_seconds > 0
            && (self.recorded_seconds as f64) < (self.span_seconds as f64 * 0.5)
    }
}

/// Build the digest for `entries` over `[from, to)`.
///
/// Entries are CLIPPED to the range before any duration is counted. The store's
/// query is overlap-based, so an entry that began before `from` arrives whole —
/// and counting all of it would attribute activity from before the range to a
/// digest of it. Clipping removes time, never invents it: the part inside the
/// range is time the recorder observed AND the caller asked about, and only
/// that part is counted. An entry left with nothing inside the range still
/// appears in its category's entry count, but contributes no seconds and no
/// activity label — a digest of a range should not name work done outside it.
pub fn digest(entries: &[TimelineEntry], from: DateTime<Utc>, to: DateTime<Utc>) -> Standup {
    /// What one category accumulates: the category, its intervals, and each
    /// activity label with the time it carried.
    struct Bucket {
        category: ActivityCategory,
        intervals: Vec<(DateTime<Utc>, DateTime<Utc>)>,
        entries: usize,
        labels: Vec<(String, i64)>,
    }
    let mut by_category: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut all_intervals: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();

    for e in entries {
        // CLIPPED to the range. The store's query is overlap-based, so an entry
        // that began before `from` arrives whole — and counting all of it
        // attributes activity from BEFORE the range to a digest of it. Clipping
        // does not invent duration, it declines to claim time the caller did
        // not ask about.
        let start = e.start_time.max(from);
        let end = e.end_time.min(to);
        // A backwards entry (a clock step) or one entirely outside the range
        // contributes no time. Its absolute value would manufacture duration
        // out of a glitch.
        let secs = (end - start).num_seconds().max(0);
        if secs > 0 {
            all_intervals.push((start, end));
        }

        let slot = by_category.entry(e.category.wire_name().to_string()).or_insert(Bucket {
            category: e.category,
            intervals: Vec::new(),
            entries: 0,
            labels: Vec::new(),
        });
        slot.entries += 1;
        if secs > 0 {
            slot.intervals.push((start, end));
            // The label rides with the time. An entry that contributed no
            // seconds inside the range must not contribute its activity name
            // either: "what did I do between 2 and 3" answered with something
            // done entirely before 2 is wrong even at zero weight. Unreachable
            // through the overlap-based store query, but `digest` is public.
            slot.labels.push((e.activity.clone(), secs));
        }
    }

    // UNION, not sum. Windows are per DISPLAY, so a two-monitor session
    // produces two entries covering the same wall-clock minute — and summing
    // them reports two minutes of a day that had one. On a mostly-idle
    // two-monitor day that inflation is enough to push the total past the
    // sparse threshold, so the one safeguard against over-trusting the digest
    // is the first thing the bug destroys.
    let recorded = union_seconds(&mut all_intervals);

    let mut categories: Vec<CategorySlice> = by_category
        .into_values()
        .map(|Bucket { category, mut intervals, entries, mut labels }| {
            labels.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut activities: Vec<String> = Vec::new();
            for (label, _) in labels {
                if !label.is_empty() && !activities.contains(&label) {
                    activities.push(label);
                }
            }
            CategorySlice {
                category,
                seconds: union_seconds(&mut intervals),
                percent: 0.0, // filled below, once the attributed total is known
                entries,
                activities,
            }
        })
        .collect();

    // Percentages are shares of ATTRIBUTED time, not of `recorded`: two
    // displays can be doing DIFFERENT things at once, so the category unions
    // can legitimately sum to more than the wall-clock they cover. Dividing by
    // `recorded` would then produce percentages summing past 100 — a number a
    // reader would rightly disbelieve.
    let attributed: i64 = categories.iter().map(|c| c.seconds).sum();
    for c in &mut categories {
        c.percent = if attributed > 0 {
            ((c.seconds as f64 / attributed as f64) * 1000.0).round() / 10.0
        } else {
            0.0
        };
    }

    // Largest share first. Ties land in name order because the BTreeMap yields
    // them that way and `sort_by` is stable — the tie-break below makes that
    // explicit rather than relying on it.
    categories.sort_by(|a, b| {
        b.seconds
            .cmp(&a.seconds)
            .then(a.category.wire_name().cmp(b.category.wire_name()))
    });

    Standup {
        from,
        to,
        recorded_seconds: recorded,
        attributed_seconds: attributed,
        span_seconds: (to - from).num_seconds().max(0),
        categories,
    }
}

/// Total wall-clock seconds covered by `intervals`, counting overlap ONCE.
fn union_seconds(intervals: &mut [(DateTime<Utc>, DateTime<Utc>)]) -> i64 {
    if intervals.is_empty() {
        return 0;
    }
    intervals.sort_by_key(|(s, _)| *s);
    let mut total = 0i64;
    let (mut cur_start, mut cur_end) = intervals[0];
    for &(s, e) in &intervals[1..] {
        if s > cur_end {
            total += (cur_end - cur_start).num_seconds();
            cur_start = s;
            cur_end = e;
        } else if e > cur_end {
            cur_end = e;
        }
    }
    total + (cur_end - cur_start).num_seconds()
}

/// Render the digest as the prose a person reads in a standup.
pub fn render(s: &Standup) -> String {
    if s.categories.is_empty() {
        return "No activity was recorded for that period.".to_string();
    }
    let mut out = String::new();
    if s.is_sparse() {
        // Said FIRST, because a reader who takes the percentages at face value
        // without this line will believe the day is accounted for.
        out.push_str(&format!(
            "Only {} of {} were recorded — this covers the captured fragments, \
             not the whole period.\n\n",
            human(s.recorded_seconds),
            human(s.span_seconds)
        ));
    }
    if s.attributed_seconds > s.recorded_seconds {
        // Otherwise the percentages describe display-time while the total
        // describes wall-clock, and nothing says the two differ.
        out.push_str(&format!(
            "{} of activity across {} of wall-clock — displays were showing \
             different things at once.\n\n",
            human(s.attributed_seconds),
            human(s.recorded_seconds)
        ));
    }
    for c in &s.categories {
        out.push_str(&format!(
            "- {} — {} ({:.1}%, {} entr{})",
            c.category.wire_name(),
            human(c.seconds),
            c.percent,
            c.entries,
            if c.entries == 1 { "y" } else { "ies" }
        ));
        if !c.activities.is_empty() {
            out.push_str(&format!(": {}", c.activities.join(", ")));
        }
        out.push('\n');
    }
    out
}

fn human(seconds: i64) -> String {
    let d = Duration::seconds(seconds);
    let (h, m) = (d.num_hours(), d.num_minutes() % 60);
    match (h, m) {
        (0, 0) => format!("{}s", d.num_seconds()),
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    fn entry(from: i64, to: i64, category: ActivityCategory, activity: &str) -> TimelineEntry {
        TimelineEntry {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            start_time: at(from),
            end_time: at(to),
            category,
            app: "app".into(),
            activity: activity.into(),
            summary: format!("did {activity}"),
            provenance: None,
        }
    }

    #[test]
    fn proportions_come_from_real_durations_not_from_counting_entries() {
        // THE property. One long meeting against many short interruptions: by
        // COUNT the interruptions are 80% of the day, by TIME the meeting is
        // 88% of it. Counting produces a digest that is confidently, legibly
        // wrong — and windows genuinely differ in length, because a pause
        // truncates one, an interval change resizes the next, and the last
        // window of a session ends when the session does.
        let mut entries = vec![entry(0, 3_600, ActivityCategory::Meeting, "planning")];
        for i in 0..4 {
            let s = 3_600 + i * 200;
            entries.push(entry(s, s + 120, ActivityCategory::Comms, "slack"));
        }

        let d = digest(&entries, at(0), at(4_400));
        assert_eq!(d.categories.len(), 2);

        let meeting = &d.categories[0];
        assert_eq!(meeting.category, ActivityCategory::Meeting);
        assert_eq!(meeting.seconds, 3_600);
        assert_eq!(meeting.entries, 1, "one entry…");
        assert!(meeting.percent > 85.0, "…and most of the day: {}%", meeting.percent);

        let comms = &d.categories[1];
        assert_eq!(comms.entries, 4, "four entries…");
        assert_eq!(comms.seconds, 480);
        assert!(comms.percent < 15.0, "…and a small share: {}%", comms.percent);

        // By count alone the ordering would invert.
        assert!(
            meeting.entries < comms.entries && meeting.seconds > comms.seconds,
            "the fixture must actually separate count from duration"
        );
    }

    #[test]
    fn a_day_of_mixed_segment_lengths_totals_the_summed_real_durations() {
        // T046's stated check. Deliberately mixed: 15 min, 40 s, 7 min, 15 min.
        let entries = vec![
            entry(0, 900, ActivityCategory::Coding, "the ladder"),
            entry(900, 940, ActivityCategory::Coding, "a stub"),
            entry(940, 1_360, ActivityCategory::Docs, "the spec"),
            entry(1_360, 2_260, ActivityCategory::Coding, "the ladder"),
        ];
        let d = digest(&entries, at(0), at(2_260));

        assert_eq!(d.recorded_seconds, 900 + 40 + 420 + 900);
        let coding = d.categories.iter().find(|c| c.category == ActivityCategory::Coding).unwrap();
        assert_eq!(coding.seconds, 1_840, "not 3 x the interval");
        let total: i64 = d.categories.iter().map(|c| c.seconds).sum();
        assert_eq!(total, d.recorded_seconds, "the parts sum to the whole");
        // Each slice rounds independently, so the column need not land exactly
        // on 100. The 0.2 in the first version of this assertion was a property
        // of its two-category fixture presented as an invariant; brute force
        // finds seven-way partitions summing to 100.3, so the honest bound is
        // half the number of categories times the rounding step.
        let pct: f64 = d.categories.iter().map(|c| c.percent).sum();
        let bound = 0.05 * d.categories.len() as f64;
        assert!(
            (pct - 100.0).abs() <= bound.max(0.1),
            "percentages sum to {pct}, outside the rounding bound {bound}"
        );
    }

    #[test]
    fn two_displays_covering_the_same_minute_record_one_minute_not_two() {
        // Windows are per DISPLAY, so a two-monitor session produces two
        // entries over the same wall-clock minute. Summing them reports two
        // minutes of a day that had one — and on a mostly-idle two-monitor day
        // that inflation pushes the total past the sparse threshold, so the one
        // safeguard against over-trusting the digest is the first casualty.
        let entries = vec![
            entry(0, 7_200, ActivityCategory::Coding, "display 0"),
            entry(0, 7_200, ActivityCategory::Coding, "display 1"),
        ];
        let d = digest(&entries, at(0), at(28_800)); // an 8-hour range

        assert_eq!(d.recorded_seconds, 7_200, "two hours of wall-clock, not four");
        assert!(
            d.is_sparse(),
            "6 of 8 hours have no recording at all: recorded={} span={}",
            d.recorded_seconds,
            d.span_seconds
        );
        assert!(render(&d).starts_with("Only 2h of 8h"), "{}", render(&d));
    }

    #[test]
    fn two_displays_doing_different_things_report_both_without_inflating_the_clock() {
        // The other half: concurrency is real information. An hour of coding on
        // one screen and an hour of meeting on the other is one hour of
        // wall-clock and two hours of attributed activity, and a digest that
        // reported only one of those numbers would be wrong either way.
        let entries = vec![
            entry(0, 3_600, ActivityCategory::Coding, "the ladder"),
            entry(0, 3_600, ActivityCategory::Meeting, "standup"),
        ];
        let d = digest(&entries, at(0), at(3_600));

        assert_eq!(d.recorded_seconds, 3_600, "one hour actually elapsed");
        assert_eq!(d.attributed_seconds, 7_200, "across two screens");
        let pct: f64 = d.categories.iter().map(|c| c.percent).sum();
        assert!((pct - 100.0).abs() < 0.5, "shares still sum to 100: {pct}");
        assert!(
            render(&d).contains("displays were showing different things at once"),
            "and the reader is told: {}",
            render(&d)
        );
    }

    #[test]
    fn an_entry_straddling_the_range_contributes_only_the_part_inside_it() {
        // The store's query is OVERLAP-based, so an entry that began before
        // `from` arrives whole. Counting all of it attributes activity from
        // before the range to a digest of the range — and a single long
        // straddling entry can push the total past the span.
        let entries = vec![entry(0, 7_200, ActivityCategory::Coding, "started earlier")];
        let d = digest(&entries, at(3_600), at(10_800)); // asked about 1h-3h

        assert_eq!(d.recorded_seconds, 3_600, "only the hour inside the range");
        assert!(
            d.recorded_seconds <= d.span_seconds,
            "recorded {} cannot exceed the span {}",
            d.recorded_seconds,
            d.span_seconds
        );
        assert_eq!(d.categories[0].entries, 1, "and the entry is still reported");
    }

    #[test]
    fn an_entry_entirely_outside_the_range_contributes_nothing() {
        let entries = vec![entry(0, 600, ActivityCategory::Coding, "yesterday")];
        let d = digest(&entries, at(10_000), at(20_000));
        assert_eq!(d.recorded_seconds, 0);
        assert_eq!(d.categories[0].seconds, 0, "no time…");
        assert_eq!(d.categories[0].entries, 1, "…but it is not silently dropped");
    }

    #[test]
    fn an_interval_contained_in_an_earlier_one_does_not_shrink_the_union() {
        // The surviving mutant: making the merge-extend unconditional lets a
        // CONTAINED interval pull the running window's end BACKWARDS, so the
        // next interval looks disjoint and already-counted time is counted
        // short. [0,7200] + [100,200] + [300,7300] is 7300 seconds of coverage;
        // the mutant reports 7200. Every earlier fixture used identical,
        // adjacent or disjoint intervals, none of which can tell the two apart.
        let entries = vec![
            entry(0, 7_200, ActivityCategory::Coding, "editor"),
            entry(100, 200, ActivityCategory::Coding, "terminal"),
            entry(300, 7_300, ActivityCategory::Coding, "editor"),
        ];
        let d = digest(&entries, at(0), at(10_000));
        assert_eq!(d.recorded_seconds, 7_300, "a contained interval must not shrink the merge");
        assert_eq!(d.categories[0].seconds, 7_300);
    }

    #[test]
    fn a_partial_overlap_extends_the_union_past_the_first_interval() {
        // The other direction: never extending at all. [0,100] + [50,150]
        // covers 150 seconds; a union that ignores the overhang reports 100.
        // Identical-interval fixtures (the two-display tests) never fire the
        // extend branch, so this is the first fixture in which it must.
        let entries = vec![
            entry(0, 100, ActivityCategory::Comms, "slack"),
            entry(50, 150, ActivityCategory::Comms, "email"),
        ];
        let d = digest(&entries, at(0), at(1_000));
        assert_eq!(d.recorded_seconds, 150, "the overhang past the first interval counts");
    }

    #[test]
    fn a_zero_second_entry_contributes_no_activity_label() {
        // An entry wholly outside the range keeps its place in the entry count
        // (it is data the store handed over) but must not put its activity name
        // into a digest of a range it did no work in: "what did I do between
        // 2 and 3" answered with yesterday's task is wrong even at zero weight.
        let entries = vec![
            entry(0, 600, ActivityCategory::Coding, "yesterdays-task"),
            entry(10_000, 11_000, ActivityCategory::Coding, "todays-task"),
        ];
        let d = digest(&entries, at(10_000), at(20_000));
        assert_eq!(d.categories[0].entries, 2, "both entries are counted");
        assert_eq!(
            d.categories[0].activities,
            vec!["todays-task".to_string()],
            "only the activity that carried time inside the range is named"
        );
    }

    #[test]
    fn recorded_time_is_not_the_span_of_the_range() {
        // A four-hour range holding twenty minutes of entries recorded twenty
        // minutes. Reporting the span as the total would attribute unrecorded
        // time to whatever happened to be nearby.
        let entries = vec![entry(0, 600, ActivityCategory::Coding, "a")];
        let d = digest(&entries, at(0), at(14_400));
        assert_eq!(d.recorded_seconds, 600);
        assert_eq!(d.span_seconds, 14_400);
        assert!(d.is_sparse(), "and it says so");
        assert!(
            render(&d).starts_with("Only 10m of 4h were recorded"),
            "prominently, before the percentages: {}",
            render(&d)
        );
    }

    #[test]
    fn a_well_covered_day_is_not_flagged_as_sparse() {
        // The other direction: a warning that fires always is noise.
        let entries = vec![entry(0, 3_000, ActivityCategory::Coding, "a")];
        let d = digest(&entries, at(0), at(3_600));
        assert!(!d.is_sparse(), "83% covered is a real digest");
        assert!(!render(&d).contains("Only"));
    }

    #[test]
    fn a_backwards_entry_contributes_no_time_rather_than_negative_or_absolute() {
        // A clock step can produce end < start. Its absolute value would
        // manufacture duration out of a glitch; a negative would corrupt every
        // percentage in the digest.
        let entries = vec![
            entry(0, 600, ActivityCategory::Coding, "real"),
            entry(1_000, 400, ActivityCategory::Docs, "backwards"),
        ];
        let d = digest(&entries, at(0), at(1_000));
        assert_eq!(d.recorded_seconds, 600, "only the real one counts");
        let docs = d.categories.iter().find(|c| c.category == ActivityCategory::Docs).unwrap();
        assert_eq!(docs.seconds, 0);
        assert_eq!(docs.entries, 1, "but it is still REPORTED, not dropped");
    }

    #[test]
    fn the_order_is_the_same_on_every_run() {
        // A digest that reshuffles between two identical requests looks like
        // the day changed.
        let entries = vec![
            entry(0, 600, ActivityCategory::Coding, "a"),
            entry(600, 1_200, ActivityCategory::Docs, "b"),
            entry(1_200, 1_800, ActivityCategory::Comms, "c"),
        ];
        let first = digest(&entries, at(0), at(1_800));
        for _ in 0..20 {
            assert_eq!(digest(&entries, at(0), at(1_800)), first);
        }
        // Equal durations, so SOMETHING must order these deterministically.
        // Note it is the BTreeMap (name-ascending) plus a stable sort that
        // actually does it — deleting the explicit tie-break changes nothing,
        // which a mutation confirmed. It stays because relying on an incidental
        // property of the collection is how the order breaks when the
        // collection changes.
        assert_eq!(
            first.categories.iter().map(|c| c.category.wire_name()).collect::<Vec<_>>(),
            vec!["coding", "comms", "docs"],
            "ties break by name"
        );
    }

    #[test]
    fn activities_are_listed_by_time_and_never_repeated() {
        let entries = vec![
            entry(0, 120, ActivityCategory::Coding, "small thing"),
            entry(120, 1_320, ActivityCategory::Coding, "the big refactor"),
            entry(1_320, 1_500, ActivityCategory::Coding, "small thing"),
        ];
        let d = digest(&entries, at(0), at(1_500));
        let coding = &d.categories[0];
        assert_eq!(
            coding.activities,
            vec!["the big refactor", "small thing"],
            "most time first, each named once"
        );
        assert_eq!(coding.entries, 3, "while the entry count keeps all three");
    }

    #[test]
    fn an_empty_day_says_so_instead_of_rendering_an_empty_table() {
        let d = digest(&[], at(0), at(3_600));
        assert_eq!(d.recorded_seconds, 0);
        assert!(d.categories.is_empty());
        assert_eq!(render(&d), "No activity was recorded for that period.");
    }

    #[test]
    fn every_category_in_the_taxonomy_can_appear_in_a_digest() {
        // T045: the prompt is derived from ActivityCategory::ALL, so a variant
        // added later reaches the model. This asserts the other end — that the
        // digest handles every member rather than silently folding some into
        // Other.
        let entries: Vec<TimelineEntry> = ActivityCategory::ALL
            .iter()
            .enumerate()
            .map(|(i, c)| entry(i as i64 * 100, i as i64 * 100 + 60, *c, "x"))
            .collect();
        let d = digest(&entries, at(0), at(1_000));
        assert_eq!(
            d.categories.len(),
            ActivityCategory::ALL.len(),
            "every taxonomy member is representable"
        );
    }
}
