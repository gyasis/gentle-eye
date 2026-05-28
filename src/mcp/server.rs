//! MCP server: `GentleEyeServer` implementing rmcp's `ServerHandler`.
//!
//! Wires the real [`RecordingService`], [`VisionProvider`], and storage into the
//! seven MCP tools (`start_recording`, `stop_recording`, `get_recording_status`,
//! `analyze_video`, `list_recordings`, `cancel_recording`,
//! `get_vision_provider_info`). Tool input schemas are generated from the
//! `schemars`-derived input types. Domain failures are returned as tool results
//! with `is_error = true`; only malformed requests become protocol errors.
//!
//! Authored 2026-05-28 against rmcp 0.1.5 (`ServerHandler`, `Tool`,
//! `CallToolResult`) — the recovered source was a stub.

use crate::analysis::{GeminiProvider, OllamaProvider};
use crate::capture::CaptureService;
use crate::config::{AppConfig, VisionConfig as AppVisionConfig};
use crate::contracts::errors::GentleEyeError;
use crate::contracts::traits::{
    RecordingConfig, RecordingService, RecordingStatus, TimeRange, VisionConfig, VisionProvider,
};
use crate::mcp::tools::{
    AnalyzeVideoInput, AnalyzeVideoOutput, CancelRecordingInput, CancelRecordingOutput,
    GetRecordingStatusInput, GetRecordingStatusOutput, GetVisionProviderInfoOutput,
    ListRecordingsInput, ListRecordingsOutput, ReadScreenTextInput, ReadScreenTextOutput,
    RecordingSummary, StartRecordingInput, StartRecordingOutput, StopRecordingInput,
    StopRecordingOutput,
};
use crate::storage::StorageManager;
use chrono::Utc;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, ErrorData, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// The Gentle-Eye MCP server. Cheaply cloneable (all state behind `Arc`).
#[derive(Clone)]
pub struct GentleEyeServer {
    config: AppConfig,
    recording: Arc<dyn RecordingService>,
    vision: Arc<dyn VisionProvider>,
}

impl GentleEyeServer {
    /// Load configuration and construct the server with live storage + provider.
    pub async fn new() -> Result<Self, GentleEyeError> {
        // `config::ConfigError` is distinct from the `contracts` error taxonomy,
        // so map it onto the MCP error variant explicitly.
        let config =
            AppConfig::load().map_err(|e| GentleEyeError::Mcp(format!("configuration error: {e}")))?;
        let storage = Arc::new(StorageManager::new(config.storage.base_dir.clone())?);
        // Display to capture: GENTLE_EYE_DISPLAY (index), default 0 (primary).
        let display_index = std::env::var("GENTLE_EYE_DISPLAY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let recording: Arc<dyn RecordingService> =
            Arc::new(CaptureService::new(storage, display_index));
        let vision = build_vision_provider(&config.vision)?;
        Ok(Self {
            config,
            recording,
            vision,
        })
    }

    /// Borrow the loaded configuration.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// The configured vision provider (shared with the CLI front-end).
    pub fn vision(&self) -> &Arc<dyn VisionProvider> {
        &self.vision
    }

    /// The recording service (shared with the CLI front-end).
    pub fn recording(&self) -> &Arc<dyn RecordingService> {
        &self.recording
    }

    /// Serve the MCP protocol over stdio until the client disconnects.
    pub async fn serve_stdio(&self) -> Result<(), GentleEyeError> {
        let running = self
            .clone()
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| GentleEyeError::Mcp(format!("MCP serve init failed: {e}")))?;
        running
            .waiting()
            .await
            .map_err(|e| GentleEyeError::Mcp(format!("MCP serve loop failed: {e}")))?;
        Ok(())
    }

    async fn dispatch(&self, request: CallToolRequestParam) -> Result<CallToolResult, ErrorData> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        let result = match request.name.as_ref() {
            "start_recording" => self.tool_start_recording(parse_args(args)?).await,
            "stop_recording" => self.tool_stop_recording(parse_args(args)?).await,
            "get_recording_status" => self.tool_get_status(parse_args(args)?).await,
            "analyze_video" => self.tool_analyze_video(parse_args(args)?).await,
            "list_recordings" => self.tool_list_recordings(parse_args(args)?).await,
            "cancel_recording" => self.tool_cancel(parse_args(args)?).await,
            "get_vision_provider_info" => self.tool_provider_info().await,
            "read_screen_text" => self.tool_read_screen_text(parse_args(args)?).await,
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool: {other}"),
                    None,
                ))
            }
        };
        Ok(result)
    }

    async fn tool_start_recording(&self, input: StartRecordingInput) -> CallToolResult {
        let mut config = RecordingConfig::default();
        if let Some(fps) = input.fps {
            config.fps = fps.clamp(1, 30) as u8;
        }
        if let Some(dir) = input.output_dir {
            config.output_dir = PathBuf::from(dir);
        }
        if let Some(max) = input.max_duration_seconds {
            config.max_duration_seconds = Some(max);
        }
        match self.recording.start_recording(config).await {
            Ok(rec) => ok_json(StartRecordingOutput {
                recording_id: rec.id.to_string(),
                status: status_str(rec.status).to_string(),
                message: "Recording started".to_string(),
            }),
            Err(e) => err_text(e.to_string()),
        }
    }

    async fn tool_stop_recording(&self, input: StopRecordingInput) -> CallToolResult {
        let id = match Uuid::parse_str(&input.recording_id) {
            Ok(id) => id,
            Err(_) => return err_text(format!("invalid recording_id: {}", input.recording_id)),
        };
        match self.recording.stop_recording(id).await {
            Ok(rec) => ok_json(StopRecordingOutput {
                recording_id: rec.id.to_string(),
                status: status_str(rec.status).to_string(),
                file_path: rec.file_path.map(path_to_string),
                duration_ms: rec.duration_ms,
                file_size_bytes: rec.file_size_bytes,
                error_message: rec.error_message,
            }),
            Err(e) => err_text(e.to_string()),
        }
    }

    async fn tool_get_status(&self, input: GetRecordingStatusInput) -> CallToolResult {
        let id = match Uuid::parse_str(&input.recording_id) {
            Ok(id) => id,
            Err(_) => return err_text(format!("invalid recording_id: {}", input.recording_id)),
        };
        match self.recording.get_status(id).await {
            Ok(rec) => {
                let elapsed_ms = if rec.status == RecordingStatus::Recording {
                    Some((Utc::now() - rec.start_time).num_milliseconds().max(0) as u64)
                } else {
                    rec.duration_ms
                };
                ok_json(GetRecordingStatusOutput {
                    recording_id: rec.id.to_string(),
                    status: status_str(rec.status).to_string(),
                    start_time: Some(rec.start_time),
                    elapsed_ms,
                    file_path: rec.file_path.map(path_to_string),
                    error_message: rec.error_message,
                })
            }
            Err(e) => err_text(e.to_string()),
        }
    }

    async fn tool_analyze_video(&self, input: AnalyzeVideoInput) -> CallToolResult {
        let timeframe = input.timeframe.map(|t| TimeRange {
            start_seconds: t.start_seconds,
            end_seconds: t.end_seconds,
        });
        match self
            .vision
            .analyze_video(Path::new(&input.video_path), &input.prompt, timeframe)
            .await
        {
            Ok(result) => ok_json(AnalyzeVideoOutput {
                analysis_text: result.analysis_text,
                model_used: result.model_used,
                token_count: result.token_count.map(u64::from),
                processing_time_ms: result.processing_time_ms,
            }),
            Err(e) => err_text(e.to_string()),
        }
    }

    async fn tool_list_recordings(&self, input: ListRecordingsInput) -> CallToolResult {
        let limit = input.limit.unwrap_or(10) as usize;
        let status_filter = input
            .status_filter
            .as_deref()
            .and_then(parse_status_filter);
        match self.recording.list_recordings(limit, status_filter).await {
            Ok(recs) => {
                let recordings: Vec<RecordingSummary> = recs
                    .into_iter()
                    .map(|r| RecordingSummary {
                        id: r.id.to_string(),
                        status: status_str(r.status).to_string(),
                        start_time: r.start_time,
                        duration_ms: r.duration_ms,
                        file_path: r.file_path.map(path_to_string),
                        file_size_bytes: r.file_size_bytes,
                    })
                    .collect();
                let total_count = recordings.len() as u32;
                ok_json(ListRecordingsOutput {
                    recordings,
                    total_count,
                })
            }
            Err(e) => err_text(e.to_string()),
        }
    }

    async fn tool_cancel(&self, input: CancelRecordingInput) -> CallToolResult {
        let id = match Uuid::parse_str(&input.recording_id) {
            Ok(id) => id,
            Err(_) => return err_text(format!("invalid recording_id: {}", input.recording_id)),
        };
        match self.recording.cancel_recording(id).await {
            Ok(rec) => ok_json(CancelRecordingOutput {
                recording_id: rec.id.to_string(),
                status: status_str(rec.status).to_string(),
                message: "Recording cancelled".to_string(),
            }),
            Err(e) => err_text(e.to_string()),
        }
    }

    async fn tool_read_screen_text(&self, input: ReadScreenTextInput) -> CallToolResult {
        use crate::analysis::ocr;
        if !ocr::ocr_available() {
            return err_text("tesseract is not available for OCR on this host");
        }
        let (text, source) = if let Some(video) = input.video_path.as_deref() {
            match ocr::ocr_video(Path::new(video)) {
                Ok(t) => (t, "video"),
                Err(e) => return err_text(e.to_string()),
            }
        } else if let Some(image) = input.image_path.as_deref() {
            match ocr::ocr_image(Path::new(image)) {
                Ok(t) => (t, "image"),
                Err(e) => return err_text(e.to_string()),
            }
        } else {
            return err_text("provide image_path or video_path");
        };
        ok_json(ReadScreenTextOutput {
            text,
            source: source.to_string(),
        })
    }

    async fn tool_provider_info(&self) -> CallToolResult {
        let health = self.vision.health_check().await;
        ok_json(GetVisionProviderInfoOutput {
            provider: self.vision.name().to_string(),
            model: self.vision.model().to_string(),
            max_video_size_bytes: Some(self.vision.max_video_size()),
            supports_native_video: Some(self.vision.supports_native_video()),
            available: health.is_ok(),
            error_message: health.err().map(|e| e.to_string()),
        })
    }
}

impl ServerHandler for GentleEyeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "gentle-eye".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "Screen recording and AI video analysis tools for debugging sessions.".to_string(),
            ),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tool_catalog(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.dispatch(request).await
    }
}

// ---- helpers ---------------------------------------------------------------

fn build_vision_provider(
    cfg: &AppVisionConfig,
) -> Result<Arc<dyn VisionProvider>, GentleEyeError> {
    let model = if cfg.provider == "ollama" {
        cfg.ollama_model.clone()
    } else {
        cfg.gemini_model.clone()
    };
    let vision_config = VisionConfig {
        provider: cfg.provider.clone(),
        api_key: cfg.gemini_api_key.clone(),
        model,
        timeout_seconds: cfg.timeout_seconds,
        max_video_size_bytes: 0, // 0 => provider default
    };
    let provider: Arc<dyn VisionProvider> = if cfg.provider == "ollama" {
        let url = format!("http://{}:{}", cfg.ollama_host, cfg.ollama_port);
        Arc::new(OllamaProvider::with_url(&vision_config, url)?)
    } else {
        Arc::new(GeminiProvider::new(&vision_config)?)
    };
    Ok(provider)
}

fn tool_catalog() -> Vec<Tool> {
    vec![
        Tool::new(
            "start_recording",
            "Start a new screen recording session.",
            schema_for::<StartRecordingInput>(),
        ),
        Tool::new(
            "stop_recording",
            "Stop an active recording and finalize the video file.",
            schema_for::<StopRecordingInput>(),
        ),
        Tool::new(
            "get_recording_status",
            "Get the current status of a recording.",
            schema_for::<GetRecordingStatusInput>(),
        ),
        Tool::new(
            "analyze_video",
            "Analyze a recorded video with the configured vision AI provider.",
            schema_for::<AnalyzeVideoInput>(),
        ),
        Tool::new(
            "list_recordings",
            "List recent recordings with metadata.",
            schema_for::<ListRecordingsInput>(),
        ),
        Tool::new(
            "cancel_recording",
            "Cancel a recording without saving the video.",
            schema_for::<CancelRecordingInput>(),
        ),
        Tool::new(
            "get_vision_provider_info",
            "Get information about the configured vision AI provider.",
            empty_object_schema(),
        ),
        Tool::new(
            "read_screen_text",
            "Extract on-screen text (OCR) from an image or video.",
            schema_for::<ReadScreenTextInput>(),
        ),
    ]
}

fn schema_for<T: schemars::JsonSchema>() -> Arc<JsonObject> {
    let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<T>();
    let value = serde_json::to_value(root).unwrap_or_else(|_| Value::Object(JsonObject::new()));
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

fn empty_object_schema() -> Arc<JsonObject> {
    let mut map = JsonObject::new();
    map.insert("type".to_string(), Value::String("object".to_string()));
    map.insert("properties".to_string(), Value::Object(JsonObject::new()));
    Arc::new(map)
}

fn parse_args<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ErrorData> {
    serde_json::from_value(value)
        .map_err(|e| ErrorData::invalid_params(format!("invalid arguments: {e}"), None))
}

fn ok_json<T: serde::Serialize>(value: T) -> CallToolResult {
    match Content::json(value) {
        Ok(content) => CallToolResult::success(vec![content]),
        Err(e) => CallToolResult::error(vec![Content::text(format!("serialization error: {e}"))]),
    }
}

fn err_text(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.into())])
}

fn status_str(status: RecordingStatus) -> &'static str {
    match status {
        RecordingStatus::Recording => "recording",
        RecordingStatus::Completed => "completed",
        RecordingStatus::Cancelled => "cancelled",
        RecordingStatus::Failed => "failed",
    }
}

fn parse_status_filter(s: &str) -> Option<RecordingStatus> {
    match s.to_lowercase().as_str() {
        "recording" => Some(RecordingStatus::Recording),
        "completed" => Some(RecordingStatus::Completed),
        "cancelled" => Some(RecordingStatus::Cancelled),
        "error" | "failed" => Some(RecordingStatus::Failed),
        // "all" or anything else => no filter
        _ => None,
    }
}

fn path_to_string(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_exposes_all_tools() {
        let tools = tool_catalog();
        assert_eq!(tools.len(), 8);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"start_recording"));
        assert!(names.contains(&"analyze_video"));
        assert!(names.contains(&"get_vision_provider_info"));
        assert!(names.contains(&"read_screen_text"));
    }

    #[test]
    fn tool_schemas_are_objects() {
        for tool in tool_catalog() {
            assert_eq!(
                tool.input_schema.get("type").and_then(Value::as_str),
                Some("object"),
                "tool {} schema must be an object",
                tool.name
            );
        }
    }

    #[test]
    fn status_filter_parsing() {
        assert_eq!(parse_status_filter("completed"), Some(RecordingStatus::Completed));
        assert_eq!(parse_status_filter("error"), Some(RecordingStatus::Failed));
        assert_eq!(parse_status_filter("all"), None);
    }
}
