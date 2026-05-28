//! Example: use gentle-eye as a *library* — record a screen, then read its text.
//!
//! This is what an external crate would write after adding gentle-eye as a
//! dependency. Run it (needs a display + ffmpeg + tesseract):
//!
//!   cargo run --example record_and_read
//!
//! Optionally analyze with a provider by setting GEMINI_API_KEY or OLLAMA_HOST.

use gentle_eye::{analyze, read_text_video, record, VisionConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Record display 0 for 6 seconds at 2 fps.
    let rec = record(0, Duration::from_secs(6), 2, "/tmp/gentle-eye-example").await?;
    println!(
        "recorded {:?} ({} bytes, status {:?})",
        rec.file_path,
        rec.file_size_bytes.unwrap_or(0),
        rec.status
    );

    let Some(path) = rec.file_path.clone() else {
        return Ok(());
    };

    // 2. Read on-screen text with OCR (no model/key needed).
    println!("\n--- on-screen text (OCR) ---\n{}", read_text_video(&path)?);

    // 3. (Optional) describe it with a vision provider. Defaults to Ollama;
    //    set provider to "gemini" with GEMINI_API_KEY in the environment.
    let cfg = VisionConfig {
        provider: "ollama".to_string(),
        ..Default::default()
    };
    match analyze(&cfg, &path, "Briefly, what is shown on screen?", true).await {
        Ok(result) => println!("\n--- {} says ---\n{}", result.model_used, result.analysis_text),
        Err(e) => eprintln!("\n(analysis skipped: {e})"),
    }
    Ok(())
}
