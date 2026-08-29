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
    /// Total time spent, summed from real entry durations.
    pub seconds: i64,
    /// Share of the digest's total, 0–100, rounded to one decimal.
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
    /// Total recorded time — the sum of entry durations, NOT the wall-clock
    /// span of the range. A four-hour range with twenty minutes of entries in
    /// it recorded twenty minutes, and saying otherwise would attribute
    /// unrecorded time to whatever happened to be nearby.
    pub recorded_seconds: i64,
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
/// Entries are used as given: clipping them to the range would invent durations
/// the recorder never observed, and the range is the caller's question, not a
/// claim about what was captured.
pub fn digest(entries: &[TimelineEntry], from: DateTime<Utc>, to: DateTime<Utc>) -> Standup {
    /// What one category accumulates while the entries are walked: the
    /// category itself, total seconds, entry count, and each activity label
    /// with the time it carried.
    struct Bucket {
        category: ActivityCategory,
        seconds: i64,
        entries: usize,
        labels: Vec<(String, i64)>,
    }
    let mut by_category: BTreeMap<String, Bucket> = BTreeMap::new();

    for e in entries {
        // A backwards or zero-length entry contributes no time. Taking its
        // absolute value would manufacture duration out of a clock glitch.
        let secs = (e.end_time - e.start_time).num_seconds().max(0);
        let slot = by_category.entry(e.category.wire_name().to_string()).or_insert(Bucket {
            category: e.category,
            seconds: 0,
            entries: 0,
            labels: Vec::new(),
        });
        slot.seconds += secs;
        slot.entries += 1;
        slot.labels.push((e.activity.clone(), secs));
    }

    let recorded: i64 = by_category.values().map(|b| b.seconds).sum();

    let mut categories: Vec<CategorySlice> = by_category
        .into_values()
        .map(|Bucket { category, seconds, entries, mut labels }| {
            labels.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut activities: Vec<String> = Vec::new();
            for (label, _) in labels {
                if !label.is_empty() && !activities.contains(&label) {
                    activities.push(label);
                }
            }
            CategorySlice {
                category,
                seconds,
                percent: if recorded > 0 {
                    ((seconds as f64 / recorded as f64) * 1000.0).round() / 10.0
                } else {
                    0.0
                },
                entries,
                activities,
            }
        })
        .collect();

    // Largest share first, ties broken by name so the order is the same on
    // every run — a digest that reshuffles between two identical requests looks
    // like the day changed.
    categories.sort_by(|a, b| {
        b.seconds
            .cmp(&a.seconds)
            .then(a.category.wire_name().cmp(b.category.wire_name()))
    });

    Standup {
        from,
        to,
        recorded_seconds: recorded,
        span_seconds: (to - from).num_seconds().max(0),
        categories,
    }
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
        let pct: f64 = d.categories.iter().map(|c| c.percent).sum();
        assert!((pct - 100.0).abs() < 0.2, "and the percentages do too: {pct}");
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
        // Equal durations, so the tie-break must be doing the work.
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
