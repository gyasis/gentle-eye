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
    AnalyzeVideoInput, AnalyzeVideoOutput, AskDayInput, CancelRecordingInput, CancelRecordingOutput,
    DayflowStatusInput, GetTimelineInput, StartDayflowInput, StopDayflowInput,
    CaptureStreamFrameInput, CaptureStreamFrameOutput, DefineTargetInput, DefineTargetOutput,
    FocusTargetInput, FocusTargetOutput, MeasureTargetInput, MeasureTargetOutput,
    GetRecordingStatusInput, GetRecordingStatusOutput, GetVisionProviderInfoOutput,
    ListRecordingsInput, ListRecordingsOutput, ReadScreenTextInput, ReadScreenTextOutput,
    RecordingSummary, StartRecordingInput, StartRecordingOutput, StopRecordingInput,
    StopRecordingOutput,
};
use crate::storage::StorageManager;
use crate::target::geometry::norm_to_pixel;
use crate::target::model::{PixelRect, Target, TargetSource};
use crate::target::store::TargetStore;
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
    /// The ONE dayflow engine. MCP is an adapter over it, exactly as the CLI
    /// and HTTP surfaces are — so "started on one surface, visible from the
    /// others" is a property of there being one state, not an agreement three
    /// implementations have to keep.
    dayflow: Arc<crate::dayflow::service::DayflowService>,
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
        let timeline = Arc::new(crate::dayflow::timeline::SqliteTimelineStore::new(
            Arc::new(std::sync::Mutex::new(crate::storage::database::init_database(
                &config.storage.base_dir.join("gentle-eye.db"),
            )?)),
        ));
        let dayflow = Arc::new(crate::dayflow::service::DayflowService::new(
            timeline,
            config.dayflow.clone(),
        ));
        Ok(Self {
            config,
            recording,
            vision,
            dayflow,
        })
    }

    /// The dayflow engine (shared with the CLI and HTTP front-ends).
    ///
    /// Handed out rather than duplicated: every surface must drive THIS
    /// instance, or "started on one, visible from the others" stops being true.
    pub fn dayflow(&self) -> &Arc<crate::dayflow::service::DayflowService> {
        &self.dayflow
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
            "capture_stream_frame" => self.tool_capture_stream_frame(parse_args(args)?).await,
            "define_target" => self.tool_define_target(parse_args(args)?).await,
            "focus_target" => self.tool_focus_target(parse_args(args)?).await,
            "measure_target" => self.tool_measure_target(parse_args(args)?).await,
            "start_dayflow" => self.tool_start_dayflow(parse_args(args)?),
            "stop_dayflow" => self.tool_stop_dayflow(parse_args(args)?),
            "dayflow_status" => self.tool_dayflow_status(parse_args(args)?),
            "get_timeline" => self.tool_get_timeline(parse_args(args)?),
            "ask_day" => self.tool_ask_day(parse_args(args)?),
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool: {other}"),
                    None,
                ))
            }
        };
        Ok(result)
    }

    // ---- Dayflow (US6) -----------------------------------------------------
    //
    // Every one of these is a THIN adapter: parse the wire shape, call the one
    // service, serialise the answer. No decision lives here, because a decision
    // that lives in a surface is a decision the other two surfaces do not make.

    fn tool_start_dayflow(&self, input: StartDayflowInput) -> CallToolResult {
        let mode = match input.mode.as_deref() {
            None | Some("session") => crate::dayflow::models::DayflowMode::Session,
            Some("daemon") => crate::dayflow::models::DayflowMode::Daemon,
            Some(other) => return err_text(format!("unknown mode '{other}': use session or daemon")),
        };
        // The SAME parse the CLI and HTTP use: three surfaces cannot drift
        // when there is one type to drift from (FR-115).
        let spec = match crate::dayflow::source::SourceSpec::parse(
            input.displays,
            input.window,
            input.target,
            input.input,
        ) {
            Ok(s) => s,
            Err(e) => return err_text(e),
        };
        match self.dayflow.start_session(mode, spec, Utc::now()) {
            Ok(id) => ok_json(serde_json::json!({
                "session_id": id.to_string(),
                "message": "Dayflow started",
            })),
            Err(e) => err_text(e.to_string()),
        }
    }

    fn tool_stop_dayflow(&self, _input: StopDayflowInput) -> CallToolResult {
        match self.dayflow.stop(Utc::now()) {
            Ok(closed) => ok_json(serde_json::json!({
                "windows_closed": closed.len(),
                "message": "Dayflow stopped",
            })),
            Err(e) => err_text(e.to_string()),
        }
    }

    fn tool_dayflow_status(&self, input: DayflowStatusInput) -> CallToolResult {
        self.tool_dayflow_status_at(input, Utc::now())
    }

    /// [`Self::tool_dayflow_status`], with the clock supplied.
    fn tool_dayflow_status_at(
        &self,
        _input: DayflowStatusInput,
        now: chrono::DateTime<Utc>,
    ) -> CallToolResult {
        // A DEGRADED session is reported as a successful call with the
        // degradation in the payload. Returning an MCP error for "recording but
        // not producing" would make every caller treat a recoverable state as a
        // failed request.
        match self.dayflow.status(now) {
            Ok(status) => ok_json(status),
            Err(e) => err_text(e.to_string()),
        }
    }

    fn tool_get_timeline(&self, input: GetTimelineInput) -> CallToolResult {
        self.tool_get_timeline_at(input, Utc::now())
    }

    /// [`Self::tool_get_timeline`], with the clock supplied.
    ///
    /// The seam exists for the same reason `http::route_at` has one: a rule
    /// that reads the clock itself cannot be driven into the state where it
    /// matters, so it is undefended by construction. R36 learned that and then
    /// applied it to one surface out of three.
    fn tool_get_timeline_at(&self, input: GetTimelineInput, now: chrono::DateTime<Utc>) -> CallToolResult {
        let (from, to) = match crate::dayflow::service::resolve_range(input.from.as_deref(), input.to.as_deref(), now) {
            Ok(r) => r,
            Err(e) => return err_text(e),
        };
        // `standup: true` returns the categorized digest (FR-028) — through
        // the same DayflowService::standup the CLI and HTTP surfaces call, so
        // the digest cannot differ by surface. Wave 11's parity work shipped
        // it on two surfaces out of three; the MCP tool was the missing one.
        if input.standup.unwrap_or(false) {
            return match self.dayflow.standup(from, to) {
                Ok(s) => ok_json(serde_json::json!({
                    "digest": s,
                    "text": crate::dayflow::standup::render(&s),
                })),
                Err(e) => err_text(e.to_string()),
            };
        }
        match self.dayflow.timeline(from, to) {
            Ok(slice) => ok_json(serde_json::json!({
                "from": from.to_rfc3339(),
                "to": to.to_rfc3339(),
                "entries": slice.entries,
                "gaps": slice.gaps,
            })),
            Err(e) => err_text(e.to_string()),
        }
    }

    fn tool_ask_day(&self, input: AskDayInput) -> CallToolResult {
        self.tool_ask_day_at(input, Utc::now())
    }

    /// [`Self::tool_ask_day`], with the clock supplied.
    fn tool_ask_day_at(&self, input: AskDayInput, now: chrono::DateTime<Utc>) -> CallToolResult {
        let (from, to) = match crate::dayflow::service::resolve_range(input.from.as_deref(), input.to.as_deref(), now) {
            Ok(r) => r,
            Err(e) => return err_text(e),
        };
        // The answerer is the configured vision provider's text path in
        // production; here the grounding rules are what matter, and they refuse
        // to consult anything at all when the range is empty.
        let result = self.dayflow.ask(&input.question, from, to, |prompt| {
            format!("[no model configured for ask_day]\n{prompt}")
        });
        match result {
            Ok(answer) => ok_json(answer),
            Err(e) => err_text(e.to_string()),
        }
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

    async fn tool_capture_stream_frame(&self, input: CaptureStreamFrameInput) -> CallToolResult {
        let output_dir = input
            .output_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("gentle-eye/frames"));
        match crate::capture::stream::capture_stream_frame(&input.stream_url, &output_dir) {
            Ok(frame) => ok_json(CaptureStreamFrameOutput {
                file_path: frame.file_path.to_string_lossy().into_owned(),
                width: frame.width,
                height: frame.height,
                file_size_bytes: frame.file_size_bytes,
                stream_url: frame.stream_url,
                captured_at: frame.captured_at,
                message: format!(
                    "Captured a {}x{} frame from the stream",
                    frame.width, frame.height
                ),
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

    async fn tool_define_target(&self, input: DefineTargetInput) -> CallToolResult {
        if !input.region.is_valid() {
            return err_text(format!(
                "invalid region {:?}: x,y,w,h must lie within 0–1 with positive area",
                input.region
            ));
        }
        let make_active = input.set_active.unwrap_or(true);
        let mut target = Target::new(input.name.clone(), input.source.clone(), input.region);
        target.active = make_active;

        let mut store = match TargetStore::load() {
            Ok(s) => s,
            Err(e) => return err_text(format!("could not load targets: {e}")),
        };
        store.add(target.clone());
        if let Err(e) = store.save() {
            return err_text(format!("could not save target: {e}"));
        }

        // Best-effort confirmation image so the agent can SEE the crop and
        // re-call with an adjusted region. Absent when no source is reachable.
        let (pixel_rect, confirmation_image, note) = match self.confirm_target(&target) {
            Ok((rect, path)) => (Some(rect), Some(path), String::new()),
            Err(e) => (None, None, format!(" (no confirmation image: {e})")),
        };

        ok_json(DefineTargetOutput {
            name: input.name,
            active: make_active,
            pixel_rect,
            confirmation_image,
            message: format!(
                "Target defined{}.{}",
                if make_active { " and active" } else { "" },
                note
            ),
        })
    }

    async fn tool_focus_target(&self, input: FocusTargetInput) -> CallToolResult {
        let mut store = match TargetStore::load() {
            Ok(s) => s,
            Err(e) => return err_text(format!("could not load targets: {e}")),
        };
        if let Err(e) = store.set_active(&input.name) {
            return err_text(e.to_string());
        }
        if let Err(e) = store.save() {
            return err_text(format!("could not save target: {e}"));
        }
        let region = store
            .active()
            .map(|t| t.region)
            .unwrap_or_else(|| crate::target::model::NormRect::new(0.0, 0.0, 1.0, 1.0));
        ok_json(FocusTargetOutput {
            name: input.name.clone(),
            active: true,
            region,
            message: format!("Now focused on target '{}'", input.name),
        })
    }

    async fn tool_measure_target(&self, input: MeasureTargetInput) -> CallToolResult {
        if !input.region.is_valid() {
            return err_text(format!("invalid region {:?}", input.region));
        }
        let (bgra, w, h) = match self.capture_bgra(&input.source) {
            Ok(t) => t,
            Err(e) => return err_text(format!("could not capture frame for measurement: {e}")),
        };
        let stride = w * 4;
        let result = match crate::target::measure::measure(&bgra, w, h, stride, input.region) {
            Ok(r) => r,
            Err(e) => return err_text(e.to_string()),
        };
        let red_marker = if input.find_red_marker.unwrap_or(false) {
            crate::target::measure::find_red_marker(&bgra, w, h, stride)
        } else {
            None
        };
        // Best-effort Redline Overlay so the VLM can supervise the CV.
        let overlay_image = {
            let dir = std::env::temp_dir().join("gentle-eye/targets");
            let out = dir.join("measure_overlay.png");
            let snapped_px = norm_to_pixel(result.snapped_rect, (w as u32, h as u32), (0, 0));
            match crate::target::measure::bgra_to_gray(&bgra, w, h, stride) {
                Ok(gray)
                    if crate::target::measure::write_redline_overlay(&gray, snapped_px, &out)
                        .is_ok() =>
                {
                    Some(out.to_string_lossy().into_owned())
                }
                _ => None,
            }
        };
        ok_json(MeasureTargetOutput {
            result: Some(result),
            red_marker,
            overlay_image,
            message: "Measurement complete — inspect the overlay and snapped_rect.".to_string(),
        })
    }

    /// Capture one frame as a tightly-packed BGRA buffer for measurement.
    /// Best-effort — errors when no display/stream is reachable.
    fn capture_bgra(&self, source: &TargetSource) -> Result<(Vec<u8>, usize, usize), GentleEyeError> {
        match source {
            TargetSource::Display { index } => {
                let mut cap = crate::capture::screen::ScreenCapturer::new(*index)?;
                let (fw, fh) = (cap.width(), cap.height());
                let buf = cap.capture_frame(std::time::Duration::from_secs(2))?;
                let stride = buf.len().checked_div(fh).unwrap_or(fw * 4);
                if stride == fw * 4 {
                    Ok((buf, fw, fh))
                } else {
                    // Repack padded rows into a tight BGRA buffer.
                    let rect = PixelRect { x: 0, y: 0, w: fw as u32, h: fh as u32 };
                    let (tight, tw, th) = crate::target::crop::crop_bgra(&buf, fw, fh, stride, rect)?;
                    Ok((tight, tw as usize, th as usize))
                }
            }
            TargetSource::Stream { url } => {
                let dir = std::env::temp_dir().join("gentle-eye/targets");
                std::fs::create_dir_all(&dir)
                    .map_err(|e| GentleEyeError::Mcp(format!("temp dir: {e}")))?;
                let frame = crate::capture::stream::capture_stream_frame(url, &dir)?;
                let (bgra, w, h) = crate::target::measure::load_image_as_bgra(&frame.file_path)?;
                Ok((bgra, w, h))
            }
        }
    }

    /// Capture one frame for `target`, crop it, and write a confirmation PNG.
    /// Returns the resolved pixel rect + the PNG path. Best-effort — errors when
    /// no display/stream is reachable (e.g. headless / CI).
    fn confirm_target(&self, target: &Target) -> Result<(PixelRect, String), GentleEyeError> {
        let dir = std::env::temp_dir().join("gentle-eye/targets");
        std::fs::create_dir_all(&dir).map_err(|e| GentleEyeError::Mcp(format!("temp dir: {e}")))?;
        match &target.source {
            TargetSource::Display { index } => {
                let mut cap = crate::capture::screen::ScreenCapturer::new(*index)?;
                let (fw, fh) = (cap.width(), cap.height());
                let buf = cap.capture_frame(std::time::Duration::from_secs(2))?;
                let stride = buf.len().checked_div(fh).unwrap_or(fw * 4);
                let rect = norm_to_pixel(target.region, (fw as u32, fh as u32), (0, 0));
                let (cropped, cw, ch) = crate::target::crop::crop_bgra(&buf, fw, fh, stride, rect)?;
                let out = dir.join(format!("{}.png", sanitize(&target.name)));
                crate::capture::stream::write_bgra_png(&cropped, cw, ch, &out)?;
                Ok((rect, out.to_string_lossy().into_owned()))
            }
            TargetSource::Stream { url } => {
                // Probe a full frame for resolution → compute pixel rect →
                // capture the cropped frame.
                let full = crate::capture::stream::capture_stream_frame(url, &dir)?;
                let rect = norm_to_pixel(target.region, (full.width, full.height), (0, 0));
                let frame =
                    crate::capture::stream::capture_stream_frame_cropped(url, &dir, Some(rect))?;
                Ok((rect, frame.file_path.to_string_lossy().into_owned()))
            }
        }
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
        // A full URL in `ollama_host` (scheme, and possibly a path such as the
        // Atelier governor's /llm/ollama lane) is used verbatim; a bare host still
        // gets the host:port treatment.
        let url = if cfg.ollama_host.starts_with("http://")
            || cfg.ollama_host.starts_with("https://")
        {
            cfg.ollama_host.clone()
        } else {
            format!("http://{}:{}", cfg.ollama_host, cfg.ollama_port)
        };
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
            "start_dayflow",
            "Start continuous activity tracking. Dayflow SAMPLES the screen at an \
             interval rather than recording it, so an all-day session costs a few \
             frames per minute instead of a video stream.",
            schema_for::<StartDayflowInput>(),
        ),
        Tool::new(
            "stop_dayflow",
            "Stop the running dayflow session and close its open windows.",
            schema_for::<StopDayflowInput>(),
        ),
        Tool::new(
            "dayflow_status",
            "Whether dayflow is running, and whether it is actually PRODUCING. A \
             degraded session returns successfully with the degradation in the \
             payload — it is running, just not producing.",
            schema_for::<DayflowStatusInput>(),
        ),
        Tool::new(
            "get_timeline",
            "Activity timeline entries overlapping a time range. Defaults to today \
             so far.",
            schema_for::<GetTimelineInput>(),
        ),
        Tool::new(
            "ask_day",
            "Answer a question about a time range, grounded STRICTLY on recorded \
             entries. Says so plainly when the range holds no record rather than \
             inventing one. NOTE: no model is wired yet — a range WITH records \
             returns the grounding prompt rather than an answer, so only the \
             refusal path is fully functional today.",
            schema_for::<AskDayInput>(),
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
        Tool::new(
            "capture_stream_frame",
            "Grab a single frame from a live stream URL (RTSP/HTTP/SRT, e.g. an ATEM output) as a PNG.",
            schema_for::<CaptureStreamFrameInput>(),
        ),
        Tool::new(
            "define_target",
            "Define a region-of-interest ('target') to crop a display or stream to, using NORMALIZED 0-1 coordinates. Returns a confirmation image of the crop so you can self-correct the region.",
            schema_for::<DefineTargetInput>(),
        ),
        Tool::new(
            "focus_target",
            "Switch the active target by name (one target is active at a time). All subsequent capture/analysis crops to it.",
            schema_for::<FocusTargetInput>(),
        ),
        Tool::new(
            "measure_target",
            "Zoom-then-Snap measurement: snap a rough normalized region to real edges, detect a tiled-pane grid, optionally find a red marker. Returns a snapped_rect + a Redline overlay to supervise the CV.",
            schema_for::<MeasureTargetInput>(),
        ),
    ]
}

/// Filesystem-safe slug for a target name (used in the confirmation PNG path).
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
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

    /// A server whose dayflow engine is in-memory, so the MCP ADAPTERS can be
    /// driven directly. Without this, nothing tested them: replacing
    /// `tool_dayflow_status`'s body with a constant survived the entire suite,
    /// producing exactly the "the CLI says running and the dashboard says
    /// stopped" contradiction the design claims to prevent.
    fn test_server() -> GentleEyeServer {
        let store = Arc::new(crate::dayflow::timeline::SqliteTimelineStore::new(Arc::new(
            std::sync::Mutex::new(crate::storage::database::init_in_memory().unwrap()),
        )));
        let config = AppConfig::default();
        let dayflow = Arc::new(crate::dayflow::service::DayflowService::new(
            store,
            config.dayflow.clone(),
        ));
        GentleEyeServer {
            config,
            recording: Arc::new(crate::capture::service::CaptureService::new(
                Arc::new(StorageManager::new(std::env::temp_dir()).unwrap()),
                0,
            )),
            vision: Arc::new(crate::analysis::ollama::OllamaProvider::with_url(
                &VisionConfig {
                    provider: "ollama".into(),
                    api_key: None,
                    model: "stub".into(),
                    timeout_seconds: 1,
                    max_video_size_bytes: 0,
                },
                "http://127.0.0.1:1".to_string(),
            )
            .unwrap()),
            dayflow,
        }
    }

    fn body_of(result: &CallToolResult) -> String {
        format!("{:?}", result.content)
    }

    #[test]
    fn the_mcp_status_tool_reports_the_shared_engine_not_a_constant() {
        let s = test_server();
        let before = s.tool_dayflow_status(DayflowStatusInput {});
        assert!(body_of(&before).contains("false"), "nothing running yet");

        // Start through the SERVICE — the same instance HTTP and the CLI use.
        s.dayflow
            .start(crate::dayflow::models::DayflowMode::Session, vec![0], Utc::now())
            .unwrap();

        let after = s.tool_dayflow_status(DayflowStatusInput {});
        let text = body_of(&after);
        assert!(
            text.contains("\"running\": true") || text.contains("running: true") || text.contains("true"),
            "the MCP tool must report the shared engine's state: {text}"
        );
        assert!(text.contains("session_id"), "including which session: {text}");
    }

    #[test]
    fn the_mcp_start_and_stop_tools_drive_the_shared_engine() {
        let s = test_server();
        let started = s.tool_start_dayflow(StartDayflowInput::default());
        assert!(body_of(&started).contains("session_id"), "{:?}", started.content);
        assert!(
            s.dayflow.status(Utc::now()).unwrap().running,
            "the service sees what the MCP tool started"
        );

        // A second start is refused here too, not silently replacing the first.
        let again = s.tool_start_dayflow(StartDayflowInput::default());
        assert_eq!(again.is_error, Some(true), "{:?}", again.content);

        s.tool_stop_dayflow(StopDayflowInput {});
        assert!(!s.dayflow.status(Utc::now()).unwrap().running, "and stop reaches it too");
    }

    #[test]
    fn the_mcp_timeline_tool_defaults_to_today_so_far() {
        // The default lived in three copies; two of them (MCP, CLI) had no test
        // at all, and mutating either to produce an EMPTY range survived the
        // whole suite — every question would have been answered about nothing.
        let s = test_server();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-26T15:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let out = s.tool_get_timeline_at(GetTimelineInput { from: None, to: None, standup: None }, now);
        let text = body_of(&out);
        assert!(text.contains("2026-08-26T00:00:00"), "starts at midnight: {text}");
        assert!(text.contains("2026-08-26T15:30:00"), "ends at now: {text}");
    }

    #[test]
    fn the_mcp_timeline_tool_returns_gaps_alongside_entries() {
        // T023 on the MCP surface, with a REAL recorded pause: key-presence
        // alone would survive a surface that serialises an always-empty array.
        let s = test_server();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-26T15:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        s.dayflow
            .start(crate::dayflow::models::DayflowMode::Session, vec![0], now)
            .unwrap();
        s.dayflow
            .with_run(|r| r.turn_off(now + chrono::Duration::minutes(1)))
            .unwrap();
        let out = s.tool_get_timeline_at(
            GetTimelineInput { from: None, to: Some("2026-08-26T17:00:00Z".into()), standup: None },
            now,
        );
        // `body_of` is a Debug render of the content, not raw JSON — assert on
        // the serialized fact itself: the cause token can only appear if the
        // recorded gap actually reached the tool's output.
        let text = body_of(&out);
        assert!(text.contains("gaps"), "the MCP surface returns gaps: {text}");
        assert!(text.contains("user_off"), "with the recorded cause: {text}");
    }

    #[test]
    fn the_mcp_standup_flag_returns_the_same_digest_the_other_surfaces_compute() {
        // FR-028 through the MCP surface. Seeded with a real entry so the
        // digest has content — a key-presence check on an empty day would pass
        // against a surface that computes nothing.
        let s = test_server();
        let base = chrono::DateTime::parse_from_rfc3339("2026-08-26T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        s.dayflow
            .insert_entry(&crate::dayflow::models::TimelineEntry {
                id: uuid::Uuid::new_v4(),
                recording_id: uuid::Uuid::new_v4(),
                start_time: base,
                end_time: base + chrono::Duration::minutes(30),
                category: crate::dayflow::models::ActivityCategory::Meeting,
                app: "zoom".into(),
                activity: "planning".into(),
                summary: "sprint planning".into(),
                provenance: None,
            })
            .unwrap();
        let out = s.tool_get_timeline_at(
            GetTimelineInput {
                from: Some("2026-08-26T00:00:00Z".into()),
                to: Some("2026-08-26T17:00:00Z".into()),
                standup: Some(true),
            },
            base + chrono::Duration::hours(8),
        );
        let text = body_of(&out);
        assert!(text.contains("digest"), "returns the digest shape: {text}");
        assert!(text.contains("meeting"), "with the seeded category: {text}");
        assert!(text.contains("planning"), "and its activity: {text}");
    }

    #[test]
    fn catalog_exposes_all_tools() {
        let tools = tool_catalog();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        for expected in [
            "start_recording",
            "analyze_video",
            "get_vision_provider_info",
            "read_screen_text",
            "capture_stream_frame",
            "define_target",
            "focus_target",
            "measure_target",
            // US6: every dayflow tool must be LISTED, not merely dispatchable.
            // A tool the server answers but never advertises is one no client
            // can discover.
            "start_dayflow",
            "stop_dayflow",
            "dayflow_status",
            "get_timeline",
            "ask_day",
        ] {
            assert!(names.contains(&expected), "{expected} is missing from tools/list");
        }
        assert_eq!(tools.len(), 17, "and nothing was added without being named here");

        // Every advertised tool must also DISPATCH — a catalog entry with no
        // arm returns "unknown tool" to a client that did exactly what the
        // catalog told it to.
        let dispatchable = include_str!("server.rs");
        for name in &names {
            assert!(
                dispatchable.contains(&format!("\"{name}\" =>")),
                "{name} is advertised but has no dispatch arm"
            );
        }
    }

    #[test]
    fn target_name_sanitized() {
        assert_eq!(sanitize("left-pane 2!"), "left_pane_2_");
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
