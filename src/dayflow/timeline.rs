//! SQLite-backed activity timeline store + `ask_day` (Wave 4).
//!
//! Persists `TimelineEntry` rows (`timeline_entries` table) and answers
//! range queries + grounded Q&A over the day.
//!
//! TODO(Wave 4): `TimelineStore` trait + `StorageManager`-backed impl.
