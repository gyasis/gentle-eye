# gentle-eye — Architecture (mined)

## Module structure

```
gentle-eye/
├── Cargo.toml
├── benches/
│   ├── capture_performance.rs
│   └── mcp_response_time.rs
├── docs/
│   ├── API.md
│   └── INSTALLATION_SYSTEM.md
├── memory-bank/
├── modules/
│   └── rust-record/
│       ├── video-capture/
│       └── region-selector-ui/
├── prd/
├── specs/
│   └── 001-mcp-screen-tools/
├── target/
├── .specify/
└── .claude/
```

## Architecture-style snippets (438)

- "relativeWorkspacePath": "docs/ExecutionHistoryManager.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "docs/utils/state_agent.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "specs/001-cli-agent-interface/spec.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "promptchain/cli/README.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": ".gitignore",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "docs/T040-completion-summary.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "memory-bank/progress.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "docs/milestones/HISTORY_MODES_IMPLEMENTATION.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "specs/002-cli-orchestration/plan.md",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "promptchain/utils/agentic_step_processor.py",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "agentic_chat/agentic_team_chat.py",  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "**Module:** &#96;promptchain.utils.execution_history_manager&#96;"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "The &#96;ExecutionHistoryManager&#96; is a sophisticated history management system designed for complex agentic workflows. It provides structured, token-aware conversation history tracking with automatic truncation, filtering capabilities, and a public API for monitoring and statistics."  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "docs/ExecutionHistoryManager.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "The State Agent is a specialized agent within the PromptChain framework that manages conversation session state and history. It provides a powerful interface for users to search, navigate, summarize, and manipulate conversation histories across multiple sessions."  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "docs/utils/state_agent.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "specs/001-cli-agent-interface/spec.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **Event-Driven Architecture**: Async message passing between widgets"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "### Storage Architecture"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "promptchain/cli/README.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "Research_agent/document_search_workspace/"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": ".gitignore"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "docs/T040-completion-summary.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "# Feature Specification: PromptChain CLI Agent Interface"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "**Feature Branch**: &#96;001-cli-agent-interface&#96;"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "**Input**: User description: \"Build a CLI agent interface for PromptChain similar to Claude Code, Aider, Goose CLI, and Gemini CLI with interactive sessions, agent creation, and session management\""  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "memory-bank/progress.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "docs/milestones/HISTORY_MODES_IMPLEMENTATION.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "specs/002-cli-orchestration/plan.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- Expanded plugin architecture for custom functionality"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "promptchain/utils/agentic_step_processor.py"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "relativeWorkspacePath": "agentic_chat/agentic_team_chat.py"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "path": "/home/gyasis/Documents/code/PromptChain/.specstory/history/2025-04-16_01-25Z-designing-a-decision-chain-for-prompt-systems.md"  *(from _seg4_s242063_ff8d93fd0868.md)*
- + 4. **Decision Tracking**: "Find conversations about architecture decisions"  *(from _seg4_s242063_ff8d93fd0868.md)*
- + 1. Upload this skill to your Claude workspace  *(from _seg4_s242063_ff8d93fd0868.md)*
- +    | We should check the configuration  *(from _seg4_s242063_ff8d93fd0868.md)*
- Architecture lock-in assessment  *(from _seg7_s41087_0d69e0f54748.md)*
- Let me grab the architecture details:  *(from _seg11_s555_6dd933b370f4.md)*
- /home/gyasis/Documents/code/athena_connector/raw-sql/BCBS_SQL_ARCHITECTURE_ANALYSIS.md  *(from _seg5_s135187_16ffca153819.md)*
- Architecture Decision: Separate Processing Systems","","### **CRITICAL DESIGN PRINCIPLE**","**Claims processing failures DO NOT interfere with member worksheet processing** - BCBS member data continues to process successfully even if claims processing fails.","","### **Why Separate Systems?**","- **  *(from _seg9_s42318_6b5e7255b6ff.md)*
- Claims Processing Pipeline","","### Data Flow Architecture","","```","Excel File Upload  *(from _seg9_s42318_6b5e7255b6ff.md)*
- Technical Architecture","","### Core Components","","#### 1. **Payor Configuration System**","```python","# payor_claims_config.py","PAYOR_CLAIMS_CONFIGURATION = {","    'BCBS': PayorClaimsConfig(","        supports_claims=True,","        required=True,","        worksheet_names={'members': 'Members', 'claims': 'Claims  *(from _seg9_s42318_6b5e7255b6ff.md)*
- Completed Components","","#### Phase 1: Excel Detection Logic","- **Status**:  *(from _seg9_s42318_6b5e7255b6ff.md)*
- In Progress Components","","#### Phase 2: Claims Ledger System Design","- **Status**:  *(from _seg9_s42318_6b5e7255b6ff.md)*
- **DESIGNING**","- **File**: `claims_ledger.py` (NEW)","- **Features**:","  - Separate claims processing ledger","  - Independent tracking from member processing","  - Claims-specific status management","","#### Phase 3: Pipeline Integration","- **Status**:  *(from _seg9_s42318_6b5e7255b6ff.md)*
- **PLANNING**","- **File**: `run_payor_pipeline_snowflake_v2.py`","- **Progress**: Architecture redesign for separate claims processing","","#### Phase 4: Claims Aggregation Engine","- **Status**:  *(from _seg9_s42318_6b5e7255b6ff.md)*
- **PLANNING**","- **File**: `claims_aggregation.py` (NEW)","- **Progress**: Dedicated claims processing engine design","","###  *(from _seg9_s42318_6b5e7255b6ff.md)*
- Pending Components","","#### Phase 5: Testing & Validation","- **Status**:  *(from _seg9_s42318_6b5e7255b6ff.md)*
- **DESIGNING** - Separate claims ledger architecture  ","**Last Updated**: August 21, 2025","","---","","##  *(from _seg9_s42318_6b5e7255b6ff.md)*
- 10 architecture decisions with rationale and impact  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "agentic_chat/MCP_TOOL_OBSERVABILITY_ANALYSIS.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "docs/mcp_tool_hijacker.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "BRAND_GUIDELINE_RAG_IMPLEMENTATION_PLAN.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "docs/milestones/MCP_OBSERVABILITY_FIX_PROPOSAL.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "agentic_chat/LOGGING_ARCHITECTURE_OVERHAUL.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "GAP_ANALYSIS_HYBRIDRAG_VS_LIGHTRAG_RAGANYTHING.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "docs/milestones/MCP_OBSERVABILITY_LIBRARY_FIX_COMPLETE.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "docs/observability/mcp-events.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "prd/mcp_tool_hijacker_prd.md",  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "**Why**: MCPHelper was designed to be a low-level utility, and event emission for tool calls was expected to happen at a higher level (PromptChain or AgenticStepProcessor)."  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "agentic_chat/MCP_TOOL_OBSERVABILITY_ANALYSIS.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "- **Modular Design**: Non-breaking integration with existing PromptChain functionality"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "The MCP Tool Hijacker consists of four main components:"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "docs/mcp_tool_hijacker.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "## ARCHITECTURE CONFIRMATION"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "BRAND_GUIDELINE_RAG_IMPLEMENTATION_PLAN.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "docs/milestones/MCP_OBSERVABILITY_FIX_PROPOSAL.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "agentic_chat/LOGGING_ARCHITECTURE_OVERHAUL.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "### Gap 1: Visual Design Element Retrieval  *(from _seg4_s242052_32a4cd6346c3.md)*
- Supported | None |\n\n---\n\n## Specific Gaps for Brand Guideline Use Case\n\n### Gap 1: Visual Design Element Retrieval  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "GAP_ANALYSIS_HYBRIDRAG_VS_LIGHTRAG_RAGANYTHING.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "docs/observability/mcp-events.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "**Key Finding**: Your HybridRAG is excellent for **text-only documents**, but RAG-Anything fills critical gaps for **multimodal documents** (images, tables, formulas, charts) that are essential for brand guidelines and design documents."  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "The MCP Tool Hijacker is a specialized component that enables direct Model Context Protocol (MCP) tool execution without requiring LLM agent processing. This feature addresses the need for efficient, parameterized tool calls in the PromptChain library, allowing developers to bypass the full agent workflow for simple tool operations."  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "- **API Wrappers**: Creating simple interfaces to MCP tools"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "### MCP Tool Hijacker Architecture"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "The MCP Tool Hijacker provides a direct interface to MCP tools that:"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "relativeWorkspacePath": "prd/mcp_tool_hijacker_prd.md"  *(from _seg4_s242052_32a4cd6346c3.md)*
- :meth:`_expression.TextClause.columns` - primary creation interface.  *(from _seg12_s872370_4d087f32f09f.md)*
- /home/airflow/.local/lib/python3.10/site-packages/libcst/helpers/__pycache__/module.cpython-310.pyc  *(from _seg12_s872370_4d087f32f09f.md)*
- A GlobalScope is the scope of module. All module level assignments are recorded in GlobalScope.  *(from _seg12_s872370_4d087f32f09f.md)*
- amdgpu-enable-lower-module-lds  *(from _seg12_s872370_4d087f32f09f.md)*
- Optional name to give to the database in OpenMetadata. If left blank, we will use default as the database name.  *(from _seg12_s872370_4d087f32f09f.md)*
- go.shape.interface { Error() string }  *(from _seg12_s872370_4d087f32f09f.md)*
- 1*concurrent.node[*internal/abi.Type,interface {}]  *(from _seg12_s872370_4d087f32f09f.md)*
- 2*concurrent.entry[*internal/abi.Type,interface {}]  *(from _seg12_s872370_4d087f32f09f.md)*
- 4*[]*concurrent.node[*internal/abi.Type,interface {}]  *(from _seg12_s872370_4d087f32f09f.md)*
- 5*concurrent.indirect[*internal/abi.Type,interface {}]  *(from _seg12_s872370_4d087f32f09f.md)*
- 5*[]*concurrent.entry[*internal/abi.Type,interface {}]  *(from _seg12_s872370_4d087f32f09f.md)*
- 5*[0]*concurrent.node[*internal/abi.Type,interface {}]  *(from _seg12_s872370_4d087f32f09f.md)*
- Real numbers have no imaginary component.  *(from _seg12_s872370_4d087f32f09f.md)*
- is not a module, class, or callable.  *(from _seg12_s872370_4d087f32f09f.md)*
- /home/gyasis/.npm/_npx/952459504b2da320/node_modules/  *(from _seg12_s872370_4d087f32f09f.md)*
- - Three-layer architecture (source  *(from _seg4_s186425_afc57c1e7025.md)*
- User stated: "our measures and contact forms should be views so when we convert and pull latest when we need to buy filtering. is this not happening? we should never have to rebuild views...the purpose of a view is to be refreshed, so what is the problem with the pipeline?"  *(from _seg4_s186425_afc57c1e7025.md)*
- - Three-layer architecture: QUALITY_GAPS  *(from _seg4_s186425_afc57c1e7025.md)*
- - **View vs Table Architecture**: Views auto-refresh from source data; tables are static snapshots requiring manual refresh  *(from _seg4_s186425_afc57c1e7025.md)*
- - **User Feedback**: "our measures and contact forms should be views...we should never have to rebuild views...the purpose of a view is to be refreshed, so what is the problem with the pipeline?"  *(from _seg4_s186425_afc57c1e7025.md)*
- 11. "ok this is the issue our measures and contact forms should be views so when we convert and pull latest when we need to buy filtering. is this not happening? we should never have to rebuild views.......the purpose of a view is to be refreshed, so what is the problem with the pipeline?"  *(from _seg4_s186425_afc57c1e7025.md)*
- - COLUMN_NAME: INTERFACEVENDORID  *(from _seg4_s186425_afc57c1e7025.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector/src/main.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/build.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/src/main.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/glutin_egl_sys-e9b0611d580acc5f/out/egl_bindings.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/i-slint-compiler-f9b0f9b8887c1d51/out/included_library.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/khronos_api-a6450dee76900363/out/webgl_exts.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/rav1e-299436b2b40f4047/out/built.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/region-selector-ui-0bb9b2e44530d09a/out/main.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/serde-9a51dddb8dff7271/out/private.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/thiserror-19697615f084b9f3/out/private.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/tiny-xlib-0b8fca3ac9f008f8/out/libdir.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/build/x11-dl-d45222b9851b1867/out/config.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/examples/basic_capture.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/encoder.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/config.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/capture.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/display_manager.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/error.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/lib.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/frame_rate.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/metadata.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/memory.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/service.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/prerequisites.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/storage.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/tests/end_to_end.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/tests/service_integration.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/specs/001-mcp-screen-tools/contracts/traits.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/src/contracts/traits.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/src/analysis/traits.rs  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/.rustc_info.json  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/target/debug/.fingerprint/accesskit-50b0aa9b53c3b69e/lib-accesskit.json  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/docs/CHEATSHEET.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/docs/QUICKSTART.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/README.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/CHANGES_SUMMARY.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/TEST_PLAN.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/DEPLOYMENT_SUMMARY.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/AGENT_COORDINATION_REPORT.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/SLINT-UI-NOTES.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/LAYOUT-FIXES.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/README_ENHANCEMENTS.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/docs/API.md  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector/Cargo.toml  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/Cargo.toml  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/Cargo.toml  *(from _seg7_s230440_ea5c9103072f.md)*
- /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/Cargo.toml  *(from _seg7_s230440_ea5c9103072f.md)*
- Design a detailed implementation plan for refactoring gentle-eye's video analysis to:  *(from _seg6_s115470_5ed8aaaad94c.md)*
- - `src/contracts/traits.rs` - May need unified analyze_media method  *(from _seg6_s115470_5ed8aaaad94c.md)*
