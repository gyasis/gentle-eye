//! Live ATEM-stream crop validation (ignored — needs a stream at
//! `rtmp://localhost:7001/live/atem`). Run with:
//!   cargo test --test target_live_stream -- --ignored --nocapture

use gentle_eye::capture::stream::{capture_stream_frame, capture_stream_frame_cropped};
use gentle_eye::target::geometry::norm_to_pixel;
use gentle_eye::target::measure::{load_image_as_bgra, measure};
use gentle_eye::target::model::NormRect;

const URL: &str = "rtmp://localhost:7001/live/atem";

#[test]
#[ignore = "requires a live ATEM stream at rtmp://localhost:7001/live/atem"]
fn live_stream_crop_dims_match_region() {
    let dir = std::env::temp_dir().join("ge-atem-test");

    // Full frame → source resolution.
    let full = capture_stream_frame(URL, &dir).expect("capture full frame");
    assert!(full.width > 0 && full.height > 0, "stream produced a sized frame");

    // Crop to the center-right half, full height (normalized → pixels).
    let region = NormRect::new(0.5, 0.0, 0.5, 1.0);
    let rect = norm_to_pixel(region, (full.width, full.height), (0, 0));
    let cropped = capture_stream_frame_cropped(URL, &dir, Some(rect)).expect("capture cropped");

    // The ffmpeg `crop=` filter must yield exactly the requested pixel rect.
    assert_eq!(cropped.width, rect.w, "cropped width == region pixel width");
    assert_eq!(cropped.height, rect.h, "cropped height == region pixel height");
    eprintln!(
        "LIVE OK: full={}x{}  region(norm)=0.5,0,0.5,1  rect={:?}  cropped={}x{}  -> {}",
        full.width, full.height, rect, cropped.width, cropped.height,
        cropped.file_path.display()
    );

    // The full frame is also measurable (Zoom-then-Snap over real bytes).
    let (bgra, w, h) = load_image_as_bgra(&full.file_path).expect("load full frame as bgra");
    let m = measure(&bgra, w, h, w * 4, region).expect("measure full frame");
    eprintln!(
        "MEASURE: snapped={:?}  grid={:?}  edge_alignment={:.2}",
        m.snapped_rect, m.detected_grid, m.edge_alignment
    );
}
