//! On-the-fly chunk planning for dayflow recordings (Wave 3 · PRD dayflow P1).
//!
//! A long dayflow recording is split into fixed-duration chunks (default 15 min,
//! matching Gemini's ~1fps native sampling / videolocr's chunking) so each chunk
//! can be summarized independently with rolling context. This module owns the
//! deterministic part — computing the chunk **manifest**: a list of [`ChunkRef`]
//! with monotonic, contiguous, non-overlapping wall-clock ranges and stable file
//! names. The live ffmpeg segment-muxer that actually writes the chunk files
//! (T220) consumes this plan; wiring it into the capture loop is a live-capture
//! follow-up (validated via an ignored integration test, like the vision paths).

use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

use crate::dayflow::models::ChunkRef;

/// File name for the chunk at `index` (zero-padded, stable for sorting).
pub fn chunk_filename(index: usize) -> String {
    format!("chunk_{index:03}.mp4")
}

/// Plan the chunk manifest for a recording of `total_secs` seconds starting at
/// `start`, split into `chunk_minutes`-minute chunks. The final chunk holds the
/// remainder. Each chunk's `[start_wall, end_wall)` is contiguous with the next
/// and non-overlapping; files land in `dir` as `chunk_<NNN>.mp4`.
///
/// A zero-length recording yields no chunks. `chunk_minutes` is clamped to ≥1.
pub fn plan_chunks(
    start: DateTime<Utc>,
    total_secs: u64,
    chunk_minutes: u32,
    dir: &Path,
) -> Vec<ChunkRef> {
    let chunk_secs = i64::from(chunk_minutes.max(1)) * 60;
    let total = total_secs as i64;
    let mut chunks = Vec::new();
    let mut offset: i64 = 0;
    let mut index: usize = 0;
    while offset < total {
        let dur = (total - offset).min(chunk_secs);
        chunks.push(ChunkRef {
            index,
            path: PathBuf::from(dir).join(chunk_filename(index)),
            start_wall: start + Duration::seconds(offset),
            end_wall: start + Duration::seconds(offset + dur),
        });
        offset += dur;
        index += 1;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn thirty_five_min_yields_three_chunks_monotonic_nonoverlapping() {
        let dir = Path::new("/tmp/rec");
        let chunks = plan_chunks(t0(), 35 * 60, 15, dir); // 15 + 15 + 5

        assert_eq!(chunks.len(), 3, "35min / 15min → 3 chunks");

        // Monotonic, contiguous (no gap), non-overlapping; index increments.
        for w in chunks.windows(2) {
            assert!(w[0].start_wall < w[0].end_wall, "positive duration");
            assert_eq!(w[0].end_wall, w[1].start_wall, "contiguous, no overlap/gap");
            assert_eq!(w[1].index, w[0].index + 1, "monotonic index");
        }

        // First two are full 15-min chunks; last is the 5-min remainder.
        assert_eq!((chunks[0].end_wall - chunks[0].start_wall).num_seconds(), 15 * 60);
        assert_eq!((chunks[2].end_wall - chunks[2].start_wall).num_seconds(), 5 * 60);

        // Total coverage equals the recording length exactly.
        let covered: i64 = chunks
            .iter()
            .map(|c| (c.end_wall - c.start_wall).num_seconds())
            .sum();
        assert_eq!(covered, 35 * 60);

        // Stable, zero-padded file names.
        assert_eq!(chunks[0].path.file_name().unwrap(), "chunk_000.mp4");
        assert_eq!(chunks[2].path.file_name().unwrap(), "chunk_002.mp4");
        assert_eq!(chunks[0].start_wall, t0());
    }

    #[test]
    fn exact_multiple_has_no_remainder_chunk() {
        let chunks = plan_chunks(t0(), 30 * 60, 15, Path::new("/r"));
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[1].end_wall - chunks[1].start_wall).num_seconds(), 15 * 60);
    }

    #[test]
    fn shorter_than_one_chunk_yields_single_chunk() {
        let chunks = plan_chunks(t0(), 90, 15, Path::new("/r"));
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].end_wall - chunks[0].start_wall).num_seconds(), 90);
    }

    #[test]
    fn zero_length_yields_no_chunks() {
        assert!(plan_chunks(t0(), 0, 15, Path::new("/r")).is_empty());
    }
}
