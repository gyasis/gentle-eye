//! Live vision smoke tests: pass ONE image to BOTH providers and print the
//! description the model returns. These are `#[ignore]`d (they need a network +
//! credentials / a reachable Ollama), so they never run in the normal suite.
//!
//! Run them explicitly:
//!   set -a; . ~/dev/.env; set +a
//!   OLLAMA_HOST=http://192.168.0.159:11434 \
//!     cargo test --test live_vision -- --ignored --nocapture

use gentle_eye::analysis::{GeminiProvider, OllamaProvider};
use gentle_eye::contracts::traits::{VisionConfig, VisionProvider};
use std::path::Path;

const PROMPT: &str =
    "Describe this image in one short sentence, and state exactly what text appears in it.";

fn test_image() -> String {
    std::env::var("GE_TEST_IMAGE").unwrap_or_else(|_| "/tmp/ge_test.png".to_string())
}

#[tokio::test]
#[ignore = "live: requires GEMINI_API_KEY + network"]
async fn gemini_describes_image() {
    let config = VisionConfig {
        provider: "gemini".to_string(),
        api_key: None, // falls back to GEMINI_API_KEY / GOOGLE_API_KEY env
        model: std::env::var("GE_GEMINI_MODEL").unwrap_or_default(), // empty => provider default
        timeout_seconds: 60,
        max_video_size_bytes: 0,
    };
    let provider = GeminiProvider::new(&config).expect("GEMINI_API_KEY must be set");
    let result = provider
        .analyze_image(Path::new(&test_image()), PROMPT)
        .await
        .expect("gemini analyze_image failed");
    println!(
        "\n[GEMINI model={} tokens={:?}]\n  {}\n",
        result.model_used,
        result.token_count,
        result.analysis_text.trim()
    );
    assert!(!result.analysis_text.trim().is_empty());
    assert_eq!(result.provider, "gemini");
}

#[tokio::test]
#[ignore = "live: requires a reachable Ollama (OLLAMA_HOST) with a vision model"]
async fn ollama_describes_image() {
    let config = VisionConfig {
        provider: "ollama".to_string(),
        api_key: None,
        model: std::env::var("GE_OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5vl:7b".to_string()),
        timeout_seconds: 120,
        max_video_size_bytes: 0,
    };
    // OllamaProvider::new reads OLLAMA_HOST (default http://localhost:11434).
    let provider = OllamaProvider::new(&config).expect("ollama provider build");
    let result = provider
        .analyze_image(Path::new(&test_image()), PROMPT)
        .await
        .expect("ollama analyze_image failed");
    println!(
        "\n[OLLAMA model={} tokens={:?}]\n  {}\n",
        result.model_used,
        result.token_count,
        result.analysis_text.trim()
    );
    assert!(!result.analysis_text.trim().is_empty());
    assert_eq!(result.provider, "ollama");
}
