# Active Context

**Last Updated**: 2026-05-28 12:53:28

## Current Focus
Add capture_stream_frame: grab a frame from a live stream URL (ATEM/RTSP/HTTP)

Reconstructs the stream-capture tool lost in the disaster (only its output type
survived in server.rs.partial). FFmpeg grabs one frame from an rtsp/http/srt
stream into a PNG and reports its dimensions.

Wired into all three front-ends: MCP tool 'capture_stream_frame' (9 tools now),
CLI 'capture-stream --url URL [--out DIR]', and library 'gentle_eye::capture_stream_frame'.
clippy -D warnings clean; +3 unit tests (arg builder, dim parser, empty-url guard).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

## Recent Changes
```
 .claude/activity_stream.md                     |  9 +++++++++
 .claude/session_snapshots/snapshot_latest.json |  2 +-
 .claude/system_bus.json                        |  5 +++++
 memory-bank/private/gyasis/activeContext.md    | 26 +++++++++++++-------------
 4 files changed, 28 insertions(+), 14 deletions(-)
```

## Modified Files
.claude/activity_stream.md
.claude/session_snapshots/snapshot_latest.json
.claude/system_bus.json
memory-bank/private/gyasis/activeContext.md

## Next Actions
- Continue implementation
- Run tests
- Create checkpoint
