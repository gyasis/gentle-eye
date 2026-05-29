//! Dayflow session engine + real-time scheduler (Wave 5).
//!
//! `DayflowEngine`: start/stop sessions (with a max-duration cap), driving the
//! record → chunk → summarize → timeline pipeline. A background task summarizes
//! each chunk in real time (every `chunk_minutes`).
//!
//! TODO(Wave 5): `DayflowEngine` trait + impl over the capture/summarizer/timeline layers.
