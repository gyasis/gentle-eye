_**Agent (claude-opus-4-5-20251101)**_
**Agent task list:**
 Phase 0: Research existing rust-record module structure
 Phase 0: Research MCP/rmcp best practices
 Phase 0: Research vision provider abstraction patterns
 Phase 1: Write research.md with consolidated findings
 Phase 1: Generate data-model.md
 Phase 1: Generate MCP tool contracts
 Phase 1: Generate quickstart.md
 Write plan.md with all findings
_**Agent (claude-opus-4-5-20251101)**_
Now let me create the contracts directory and MCP tool contracts:
_**Agent (claude-opus-4-5-20251101)**_
Tool use: **Bash** Create contracts directory
`mkdir -p /home/gyasis/Documents/code/gentle-eye/specs/001-mcp-screen-tools/contracts`
_**Agent (claude-opus-4-5-20251101)**_
Tool use: **Write**
  "$schema": "https://modelcontextprotocol.io/schemas/tools.json",
  "version": "1.0.0",
  "name": "gentle-eye",
  "description": "Screen recording and AI video analysis tools for Claude Code and other MCP hosts",
  "tools": [
      "name": "start_recording",
      "description": "Start a new screen recording session. Returns a unique recording ID that can be used to stop the recording or check its status.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "fps": {
            "type": "integer",
            "description": "Frames per second for the recording. Lower values (1-5) are recommended for typical debugging sessions to reduce file size. Default: 1",
            "minimum": 1,
            "maximum": 30,
            "default": 1
          },
          "output_dir": {
            "type": "string",
            "description": "Optional directory to save the recording. Must be an absolute path. If not provided, uses the default temp directory."
          },
          "max_duration_seconds": {
            "type": "integer",
            "description": "Maximum recording duration in seconds. Recording will auto-stop if this limit is reached. Default: 1800 (30 minutes)",
            "minimum": 1,
            "maximum": 7200,
            "default": 1800
          }
        },
        "additionalProperties": false
      "outputSchema": {
        "type": "object",
        "properties": {
          "recording_id": {
            "type": "string",
            "description": "Unique identifier (UUID) for this recording session"
          },
          "status": {
            "type": "string",
            "enum": ["recording"],
            "description": "Current status of the recording"
          },
          "message": {
            "type": "string",
            "description": "Human-readable confirmation message"
          }
        },
        "required": ["recording_id", "status", "message"]
      "name": "stop_recording",
      "description": "Stop an active recording and finalize the video file. Returns the path to the saved video file.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "recording_id": {
            "type": "string",
            "description": "The recording ID returned from start_recording"
          }
        },
        "required": ["recording_id"],
        "additionalProperties": false
      "outputSchema": {
        "type": "object",
        "properties": {
          "recording_id": {
            "type": "string",
            "description": "The recording ID that was stopped"
          },
          "status": {
            "type": "string",
            "enum": ["completed", "error"],
            "description": "Final status of the recording"
          },
          "file_path": {
            "type": "string",
            "description": "Absolute path to the saved video file"
          },
          "duration_ms": {
            "type": "integer",
            "description": "Total recording duration in milliseconds"
          },
          "file_size_bytes": {
            "type": "integer",
            "description": "Size of the output video file in bytes"
          },
          "error_message": {
            "type": "string",
            "description": "Error details if status is 'error'"
          }
        },
        "required": ["recording_id", "status"]
      "name": "get_recording_status",
      "description": "Get the current status of a recording session.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "recording_id": {
            "type": "string",
            "description": "The recording ID to check"
          }
        },
        "required": ["recording_id"],
        "additionalProperties": false
      "outputSchema": {
        "type": "object",
        "properties": {
          "recording_id": {
            "type": "string",
            "description": "The recording ID"
          },
          "status": {
            "type": "string",
            "enum": ["recording", "finalizing", "completed", "error", "cancelled"],
            "description": "Current status of the recording"
          },
          "start_time": {
            "type": "string",
            "format": "date-time",
            "description": "When the recording started (ISO 8601)"
          },
          "elapsed_ms": {
            "type": "integer",
            "description": "Elapsed time in milliseconds (if still recording)"
          },
          "file_path": {
            "type": "string",
            "description": "Path to video file (if completed)"
          },
          "error_message": {
            "type": "string",
            "description": "Error details (if status is error)"
          }
        },
        "required": ["recording_id", "status"]
      "name": "analyze_video",
      "description": "Analyze a video recording using a vision AI provider. Send a contextual prompt to get insights about what happened in the video.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "video_path": {
            "type": "string",
            "description": "Absolute path to the video file to analyze"
          },
          "prompt": {
            "type": "string",
            "description": "Analysis prompt describing what you want to know about the video. Examples: 'What terminal commands were executed?', 'Describe the error that appeared', 'What was the user trying to accomplish?'",
            "maxLength": 10000
          },
          "timeframe": {
            "type": "object",
            "description": "Optional time range to focus the analysis on",
            "properties": {
              "start_seconds": {
                "type": "number",
                "description": "Start time in seconds from video beginning",
                "minimum": 0
              },
              "end_seconds": {
                "type": "number",
                "description": "End time in seconds from video beginning"
              }
            },
            "required": ["start_seconds", "end_seconds"]
          }
        },
        "required": ["video_path", "prompt"],
        "additionalProperties": false
      "outputSchema": {
        "type": "object",
        "properties": {
          "analysis_text": {
            "type": "string",
            "description": "The vision AI's analysis of the video content"
          },
          "model_used": {
            "type": "string",
            "description": "The AI model that performed the analysis"
          },
          "token_count": {
            "type": "integer",
            "description": "Number of tokens used (if available from provider)"
          },
          "processing_time_ms": {
            "type": "integer",
            "description": "Time taken to generate the analysis in milliseconds"
          }
        },
        "required": ["analysis_text", "model_used", "processing_time_ms"]
      "name": "list_recordings",
      "description": "List recent screen recordings with metadata. Returns recordings sorted by start time, most recent first.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "limit": {
            "type": "integer",
            "description": "Maximum number of recordings to return. Default: 10",
            "minimum": 1,
            "maximum": 100,
            "default": 10
          },
          "status_filter": {
            "type": "string",
            "enum": ["recording", "completed", "error", "all"],
            "description": "Filter by recording status. Default: 'all'",
            "default": "all"
          }
        },
        "additionalProperties": false
      "outputSchema": {
        "type": "object",
        "properties": {
          "recordings": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "id": {
                  "type": "string",
                  "description": "Recording UUID"
                },
                "status": {
                  "type": "string",
                  "description": "Recording status"
                },
                "start_time": {
                  "type": "string",
                  "format": "date-time",
                  "description": "When recording started"
                },
                "duration_ms": {
                  "type": "integer",
                  "description": "Duration in milliseconds"
                },
                "file_path": {
                  "type": "string",
                  "description": "Path to video file"
                },
                "file_size_bytes": {
                  "type": "integer",
                  "description": "File size in bytes"
                }
              },
              "required": ["id", "status", "start_time"]
            }
          },
          "total_count": {
            "type": "integer",
            "description": "Total number of recordings matching the filter"
          }
        },
        "required": ["recordings", "total_count"]
      "name": "cancel_recording",
      "description": "Cancel an active recording without saving. Use this to abort a recording that is no longer needed.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "recording_id": {
            "type": "string",
            "description": "The recording ID to cancel"
          }
        },
        "required": ["recording_id"],
        "additionalProperties": false
      "outputSchema": {
        "type": "object",
        "properties": {
          "recording_id": {
            "type": "string",
            "description": "The recording ID that was cancelled"
          },
          "status": {
            "type": "string",
            "enum": ["cancelled"],
            "description": "Status after cancellation"
          },
          "message": {
            "type": "string",
            "description": "Confirmation message"
          }
        },
        "required": ["recording_id", "status", "message"]
      "name": "get_vision_provider_info",
      "description": "Get information about the configured vision AI provider and its capabilities.",
      "inputSchema": {
        "type": "object",
        "properties": {},
        "additionalProperties": false
      "outputSchema": {
        "type": "object",
        "properties": {
          "provider": {
            "type": "string",
            "description": "Name of the configured provider (gemini, ollama)"
          },
          "model": {
            "type": "string",
            "description": "Model being used for analysis"
          },
          "max_video_size_bytes": {
            "type": "integer",
            "description": "Maximum supported video file size"
          },
          "supports_native_video": {
            "type": "boolean",
            "description": "Whether provider supports direct video input (vs frame extraction)"
          },
          "available": {
            "type": "boolean",
            "description": "Whether the provider is currently available"
          },
          "error_message": {
            "type": "string",
            "description": "Error details if provider is not available"
          }
        },
        "required": ["provider", "model", "available"]
File created successfully at: 