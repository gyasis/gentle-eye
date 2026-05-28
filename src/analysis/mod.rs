//! Vision analysis: the `VisionProvider` trait + pluggable Gemini/Ollama backends.
//!
//! Skeleton (Wave 0, 2026-05-28): module declarations only. The submodules
//! (`traits`, `config`, `gemini`, `ollama`) are authored in later waves;
//! re-exports are added there to avoid unused-import errors under `-D warnings`.

pub mod config;
pub mod gemini;
pub mod ocr;
pub mod ollama;
pub mod traits;

pub use gemini::GeminiProvider;
pub use ollama::OllamaProvider;
