//! US4 acceptance coverage — entries that remember the layout.
//!
//! The claim under test: an entry carries WHERE its text came from, and the
//! ordering is geometric and deterministic. These run across the storage and
//! regions boundaries, where the unit tests each see only one side.

use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use gentle_eye::dayflow::models::{ActivityCategory, TimelineEntry};
use gentle_eye::dayflow::timeline::{SqliteTimelineStore, TimelineStore};
use gentle_eye::dayflow::models::provenance_in_reading_order;
use gentle_eye::regions::{reading_order, Granularity, Region, Source};
use gentle_eye::storage::database::init_in_memory;
use gentle_eye::target::model::PixelRect;
use uuid::Uuid;

fn store() -> SqliteTimelineStore {
    SqliteTimelineStore::new(Arc::new(Mutex::new(init_in_memory().unwrap())))
}

fn pane(x: u32, y: u32, w: u32, h: u32, display: u32) -> Region {
    let mut r = Region::new(
        PixelRect { x, y, w, h },
        Source::Wm,
        Granularity::Pane,
        0.8,
    );
    r.display_id = display;
    r
}

fn entry_at(min: i64, activity: &str) -> TimelineEntry {
    let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    TimelineEntry {
        id: Uuid::new_v4(),
        recording_id: Uuid::new_v4(),
        start_time: base + chrono::Duration::minutes(min),
        end_time: base + chrono::Duration::minutes(min + 5),
        category: ActivityCategory::Coding,
        app: "editor".into(),
        activity: activity.into(),
        summary: format!("did {activity}"),
        provenance: None,
    }
}

#[test]
fn a_two_pane_capture_reconstructs_its_on_screen_arrangement() {
    // US4's independent test: entries whose region and parent references
    // rebuild the layout, identically on every run.
    let mut regions = vec![
        pane(0, 0, 1920, 1080, 0),  // the window
        pane(960, 40, 940, 1000, 0), // right pane
        pane(20, 40, 920, 1000, 0),  // left pane
    ];
    gentle_eye::regions::assign_parents(&mut regions);

    let prov = provenance_in_reading_order(&regions);
    assert_eq!(prov.len(), 3);

    let s = store();
    let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    for (n, p) in prov.iter().enumerate() {
        let mut e = entry_at(n as i64 * 10, &format!("pane {n}"));
        e.provenance = Some(*p);
        s.insert_entry(&e).unwrap();
    }

    let read = s
        .query_range(base, base + chrono::Duration::hours(1))
        .unwrap();
    assert_eq!(read.len(), 3);

    // Every entry came back with its geometry intact...
    let recovered: Vec<_> = read.iter().map(|e| e.provenance.expect("provenance survives")).collect();
    assert_eq!(recovered, prov, "the whole arrangement round-trips through SQLite");

    // ...the containment edge rebuilds the tree...
    let window_id = regions[0].identity();
    let children: Vec<_> = recovered
        .iter()
        .filter(|p| p.parent_region_id == Some(window_id))
        .collect();
    assert_eq!(children.len(), 2, "both panes nest under the window");

    // ...and the order is left pane before right pane, not detection order.
    let left = recovered.iter().find(|p| p.bbox_x == 20).unwrap();
    let right = recovered.iter().find(|p| p.bbox_x == 960).unwrap();
    assert!(
        left.reading_order < right.reading_order,
        "left pane reads first, though it was detected last"
    );
}

#[test]
fn the_arrangement_is_identical_on_every_run() {
    let mut regions = vec![
        pane(640, 12, 300, 500, 0),
        pane(10, 10, 300, 500, 0),
        pane(325, 14, 300, 500, 0),
        pane(10, 600, 930, 200, 0),
    ];
    gentle_eye::regions::assign_parents(&mut regions);
    let first = provenance_in_reading_order(&regions);
    for _ in 0..25 {
        assert_eq!(
            provenance_in_reading_order(&regions),
            first,
            "geometry gives the same answer every time — a model would not"
        );
    }
}

#[test]
fn entries_written_before_the_migration_survive_with_empty_provenance() {
    // T033/T036: the columns are nullable and the old rows have no geometry.
    // The pixels they came from are gone, so provenance can NEVER be filled in
    // — NULL is the honest value, and an invented default would be
    // indistinguishable from a measured one.
    let s = store();
    let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();

    let old = entry_at(0, "before provenance existed"); // provenance: None
    s.insert_entry(&old).unwrap();

    let mut new = entry_at(10, "after");
    new.provenance = Some(provenance_in_reading_order(&[pane(5, 6, 7, 8, 1)])[0]);
    s.insert_entry(&new).unwrap();

    let read = s.query_range(base, base + chrono::Duration::hours(1)).unwrap();
    assert_eq!(read.len(), 2, "the old row is readable, not an error");
    assert!(read[0].provenance.is_none(), "and honestly reports no layout");
    assert_eq!(read[0].activity, "before provenance existed");

    let p = read[1].provenance.expect("the new row has geometry");
    assert_eq!((p.bbox_x, p.bbox_y, p.bbox_w, p.bbox_h), (5, 6, 7, 8));
    assert_eq!(p.display_id, 1);
}

#[test]
fn the_migration_is_re_runnable() {
    // Idempotent: `init_in_memory` runs the column-adds every time, and a
    // second run must not error on "duplicate column name".
    for _ in 0..3 {
        let s = store();
        let mut e = entry_at(0, "x");
        e.provenance = Some(provenance_in_reading_order(&[pane(1, 2, 3, 4, 0)])[0]);
        s.insert_entry(&e).unwrap();
        assert_eq!(s.count().unwrap(), 1);
    }
}

#[test]
fn a_region_id_identifies_the_same_pane_across_captures() {
    // Identity is derived from where the region IS, not from its index in
    // whatever vector a capture happened to build — so "this entry came from
    // the editor pane" is answerable across a day, not only within one frame.
    // The editor pane sits at a DIFFERENT vector index in each capture, and
    // deliberately so: with the same index in both, an implementation that used
    // the index as the identity would pass this test by coincidence. (It did —
    // the first version of this fixture put the editor at index 1 in both, and
    // the mutation survived.)
    let capture_one = vec![
        pane(0, 0, 100, 100, 0),
        pane(200, 0, 300, 400, 0), // editor at index 1
    ];
    let capture_two = vec![
        pane(200, 0, 300, 400, 0), // editor at index 0
        pane(50, 50, 10, 10, 0),
        pane(0, 0, 100, 100, 0),
    ];

    let a = provenance_in_reading_order(&capture_one);
    let b = provenance_in_reading_order(&capture_two);

    let editor_a = a.iter().find(|p| p.bbox_x == 200).unwrap();
    let editor_b = b.iter().find(|p| p.bbox_x == 200).unwrap();
    assert_eq!(
        editor_a.region_id, editor_b.region_id,
        "the same pane keeps its identity though its index changed"
    );
    assert_ne!(
        editor_a.reading_order, editor_b.reading_order,
        "while its READING position legitimately differs between captures"
    );
}

#[test]
fn displays_never_interleave_in_a_stored_arrangement() {
    let regions = vec![
        pane(10, 500, 300, 200, 1),
        pane(10, 10, 300, 200, 0),
        pane(10, 20, 300, 200, 1),
        pane(10, 700, 300, 200, 0),
    ];
    let prov = provenance_in_reading_order(&regions);
    let displays: Vec<u32> = prov.iter().map(|p| p.display_id).collect();
    assert_eq!(
        displays,
        vec![0, 0, 1, 1],
        "a top-left region on one panel is not comparable to a bottom-right on another"
    );
    // reading_order is a total rank across the capture, not per display
    let ranks: Vec<u32> = prov.iter().map(|p| p.reading_order).collect();
    assert_eq!(ranks, vec![0, 1, 2, 3]);
}

#[test]
fn reading_order_places_every_region_exactly_once() {
    let regions: Vec<Region> = (0..17)
        .map(|i| pane((i % 4) * 300, (i / 4) * 250, 280, 240, i % 2))
        .collect();
    let mut seen = reading_order(&regions);
    assert_eq!(seen.len(), regions.len());
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), regions.len(), "no region dropped or duplicated");
}

#[test]
fn a_half_written_provenance_row_reads_as_no_layout_not_as_a_box_of_zeros() {
    // The writer always writes provenance all-or-nothing, so this row cannot
    // arise through the API — which is exactly why the guard against it went
    // untested and a mutation that filled the gaps with zeros survived. A row
    // like this can still appear: a partial migration, a hand-edit, an older
    // writer. Filling the gaps would describe a region at (0,0) sized 0x0 that
    // was never on screen, and it would be indistinguishable from a measured
    // one.
    let conn = init_in_memory().unwrap();
    let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    conn.execute(
        "INSERT INTO timeline_entries \
         (id, recording_id, start_time, end_time, category, app, activity, summary, \
          region_id, display_id) \
         VALUES (?1, ?2, ?3, ?4, 'coding', 'editor', 'partial', 'half a row', 42, 1)",
        rusqlite::params![
            Uuid::new_v4().to_string(),
            Uuid::new_v4().to_string(),
            base.to_rfc3339(),
            (base + chrono::Duration::minutes(5)).to_rfc3339(),
        ],
    )
    .unwrap();

    let s = SqliteTimelineStore::new(Arc::new(Mutex::new(conn)));
    let read = s.query_range(base, base + chrono::Duration::hours(1)).unwrap();
    assert_eq!(read.len(), 1, "the row is still readable");
    assert!(
        read[0].provenance.is_none(),
        "a box missing its geometry is NOT a layout: {:?}",
        read[0].provenance
    );
}
