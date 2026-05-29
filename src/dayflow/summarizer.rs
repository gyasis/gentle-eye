//! Rust-native Map-Reduce chunk summarizer (Wave 3).
//!
//! Ports videolocr's `process_video_chunks_with_gemini`: each 15-min chunk is
//! summarized by a `VisionProvider` (Gemini native-video by default, Ollama
//! frame+OCR fallback), threading a rolling `CONTEXT SUMMARY FOR NEXT CHUNK`
//! forward, then reducing per-chunk summaries into a session digest.
//!
//! TODO(Wave 3): `ChunkSummarizer` trait + provider-backed impl.
