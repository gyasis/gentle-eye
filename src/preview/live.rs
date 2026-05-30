//! PV3 — live preview (default OFF): show what's being captured, in real time.
//!
//! Default renderer is **ffplay** (zero new crates). Two sources:
//! - **Display**: pipe `ScreenCapturer` BGRA frames to ffplay as `rawvideo` over
//!   stdin, **cropped to the active target** first (so preview == program).
//! - **Stream**: point ffplay at the relay URL, adding an ffmpeg `crop=` filter
//!   when a target is active.
//!
//! The arg-builders below are pure + unit-tested; the frame-pump loop needs a
//! live display and is exercised manually / by an ignored integration test.

use crate::target::model::PixelRect;

/// ffplay args to display a live `w`×`h` BGRA `rawvideo` feed read from stdin
/// (the `-` positional). `w`/`h` are the CROPPED dimensions when a target is active.
pub fn live_display_args(w: u32, h: u32) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-window_title".into(),
        "gentle-eye live".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "bgra".into(),
        "-video_size".into(),
        format!("{w}x{h}"),
        "-".into(), // ffplay reads stdin from the positional "-"
    ]
}

/// ffplay args to display a live stream `url`, optionally cropped to `crop`
/// (an ffmpeg `crop=w:h:x:y` filter). The input is the trailing positional.
pub fn live_stream_args(url: &str, crop: Option<PixelRect>) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-window_title".into(),
        "gentle-eye live".into(),
    ];
    if let Some(r) = crop {
        a.push("-vf".into());
        a.push(format!("crop={}:{}:{}:{}", r.w, r.h, r.x, r.y));
    }
    a.push(url.to_string());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_pair(a: &[String], k: &str, v: &str) -> bool {
        a.windows(2).any(|w| w[0] == k && w[1] == v)
    }

    #[test]
    fn display_feed_is_rawvideo_bgra_from_stdin() {
        let a = live_display_args(960, 1080);
        assert!(has_pair(&a, "-f", "rawvideo"));
        assert!(has_pair(&a, "-pixel_format", "bgra"));
        assert!(has_pair(&a, "-video_size", "960x1080"));
        assert_eq!(a.last().unwrap(), "-"); // stdin
    }

    #[test]
    fn stream_without_target_has_no_crop() {
        let a = live_stream_args("rtmp://x/live/atem", None);
        assert!(!a.iter().any(|s| s == "-vf"));
        assert_eq!(a.last().unwrap(), "rtmp://x/live/atem");
    }

    #[test]
    fn stream_with_active_target_injects_crop() {
        let a = live_stream_args(
            "rtmp://x/live/atem",
            Some(PixelRect { x: 960, y: 0, w: 960, h: 1080 }),
        );
        assert!(has_pair(&a, "-vf", "crop=960:1080:960:0"));
        assert_eq!(a.last().unwrap(), "rtmp://x/live/atem"); // input stays last
    }
}
