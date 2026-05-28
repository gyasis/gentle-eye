# gentle-eye — Product Requirements (mined)

## Project purpose (from session titles)

- **llm-agent-screen-and-video-understanding** — give LLM agents access to screen capture & video analysis
- **research-and-analyze-gemini** — Gemini API as the video understanding backend
- **explore-the-screen-recorder** — built on top of existing screen recording approaches
- **design-a-detailed-implementation** — modular Rust + MCP server architecture

## Sub-modules discovered

- `modules/rust-record/video-capture/`
- `modules/rust-record/region-selector-ui/`

## Requirements-style snippets (106)

- /home/gyasis/Documents/code/athena_connector/raw-sql/data/measure_requirements_pivoted.csv  *(from _seg5_s135344_66a85c62e889.md)*
- /home/gyasis/Documents/code/athena_connector/raw-sql/data/measure_requirements_analysis.sql  *(from _seg5_s135344_66a85c62e889.md)*
- "text": "- **Flexible Retrieval**: Filter and format history based on use case requirements"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-012**: System MUST list all saved sessions with timestamps"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-013**: System MUST restore full conversation history and agent configurations when resuming a saved session"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-014**: System MUST support file reference syntax &#96;@file.txt&#96; to include file contents in prompts"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-015**: System MUST support directory reference syntax &#96;@directory/&#96; to discover relevant files"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-016**: System MUST prompt users for confirmation before applying file edits"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-017**: System MUST execute shell commands prefixed with &#96;!&#96; and display output"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-018**: System MUST provide a shell mode toggle with &#96;!!&#96; for consecutive command execution"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-019**: System MUST handle Ctrl+C without terminating session (cancel current operation only)"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-020**: System MUST handle Ctrl+D or &#96;/exit&#96; to gracefully terminate session"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-021**: System MUST preserve working directory context across conversation exchanges"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-022**: System MUST support command history navigation with up/down arrow keys"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-023**: System MUST provide help documentation via &#96;/help&#96; command"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-024**: System MUST display available slash commands with &#96;/help commands&#96;"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-025**: System MUST support multi-line input when users need to paste code or complex prompts"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-026**: System MUST preserve ANSI color codes and formatting in shell command output"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "- **FR-027**: System MUST auto-save session state periodically for crash recovery"  *(from _seg4_s242063_ff8d93fd0868.md)*
- "text": "        objective: str,"  *(from _seg4_s242063_ff8d93fd0868.md)*
- As you mentioned, contract format may filter these scores out if they don't meet BCBS evidence requirements, but **the quality table now captures AUDIT-C screening data** as requested.  *(from _seg5_s135187_16ffca153819.md)*
- -- who should be excluded from cervical cancer screening requirements  *(from _seg5_s135187_16ffca153819.md)*
- - Proper field mapping from Athena to BCBS requirements  *(from _seg5_s135187_16ffca153819.md)*
- ./data/measure_requirements_analysis.sql  *(from _seg5_s135187_16ffca153819.md)*
- Member Processing Impact**: **ZERO impact on member processing success rates**","","#### Alerting Requirements","- **Critical**: Claims processing failures","- **Warning**: Data quality scores below threshold","- **Info**: Processing completion notifications","- **  *(from _seg9_s42318_6b5e7255b6ff.md)*
- Batch Processing with Progress Tracking","","### **CRITICAL REQUIREMENT: Large Row Counts**","","Claims files can  *(from _seg9_s42318_6b5e7255b6ff.md)*
- **PENDING**","- **Requirements**:","  - End-to-end workflow testing","  - Performance validation","  - Error scenario testing","","---","","##  *(from _seg9_s42318_6b5e7255b6ff.md)*
- "text": "        system_prompt = f\"\"\"Your goal is to achieve the following objective: {self.objective}"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "FINAL ANSWER REQUIREMENTS:"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "- The user cannot see tool results directly - they only see YOUR final response"  *(from _seg4_s242052_32a4cd6346c3.md)*
- "text": "# MCP Tool Hijacker - Product Requirements Document"  *(from _seg4_s242052_32a4cd6346c3.md)*
- /home/gyasis/Documents/code/gentle-eye/specs/001-mcp-screen-tools/checklists/requirements.md  *(from _seg7_s230440_ea5c9103072f.md)*
- -- qr.ReservedField  -- Removed per QA requirements  *(from _seg5_s135770_64e73323b519.md)*
- 'BCBS REQUIREMENT: Retinal eye exams must be performed by optometrist/ophthalmologist. Other codes (retinopathy evidence, imaging) can be any provider. Send MEASURE_TYPE, DATE_OF_SERVICE, CODE_TYPE, CODE' as COMMENT,  *(from _seg5_s136832_aef4bd571200.md)*
- Direct LightRAG test failed: {e}\")\n+             \n+         return test_result\n+     \n+     async def test_promptchain_mcp(self, query: str, objective: str) -> Dict[str, Any]:\n+         \"\"\"Test LightRAG through PromptChain MCP integration\"\"\"\n+         print(\"  *(from _seg6_s72180_094ed23647ba.md)*
- Objective: Table structeu research fore SQL query Genration thorugh proper joins...\n\n  *(from _seg6_s72180_094ed23647ba.md)*
- /workbench.editor.languageDetectionOpenedLanguages.workspace[["markdown",true],["pip-requirements",true],["plaintext",true],["ignore",true],["json",true],["properties",true],["jsonc",true],["python",true]]  *(from _seg4_s260957_1dc9a6aa5438.md)*
- Patient `110676` has **multiple P4P measures** in the QMRESULT table for the same clinical requirement (diabetic eye exam):  *(from _seg5_s36691_eebe51e4b085.md)*
- 1. **7 QMRESULT records** (different P4P measure variations for the same clinical requirement)  *(from _seg5_s36691_eebe51e4b085.md)*
- - **Multiple P4P measure names** for the same clinical requirement (EED)  *(from _seg5_s36691_eebe51e4b085.md)*
- 1. 7 QMRESULT records (different P4P measure variations for the same clinical requirement) expaeoind how thes measrue aling with aoue bcbs concatrual oblicatiosn  *(from _seg5_s36691_eebe51e4b085.md)*
- "macOS 12+ System Requirements",  *(from _seg4_s114277_d68b9ff25b4c.md)*
- /home/gyasis/Documents/code/athena_connector/raw-sql/contract/bcbs_report_local/test_requirements.txt  *(from _seg5_s136565_3e945f4fec6a.md)*
- - Medicare annual wellness visits requirement  *(from _seg5_s136565_3e945f4fec6a.md)*
- /home/gyasis/Documents/code/athena_connector/raw-sql/memory-bank/progress.md:264:- Aligns with BCBS_Code_List.csv RetEye code requirements  *(from _seg5_s136565_3e945f4fec6a.md)*
- /home/gyasis/Documents/code/athena_connector/raw-sql/memory-bank/activeContext.md:95:- Aligns with BCBS_Code_List.csv RetEye code requirements  *(from _seg5_s136565_3e945f4fec6a.md)*
- - Contract requirements breakdown  *(from _seg5_s135893_43774c55873f.md)*
- - Meets BCBSMN 2025 Care for Older Adults (COA) contract requirements  *(from _seg5_s135893_43774c55873f.md)*
- Contract Requirements: Send MEASURE_TYPE, DATE_OF_SERVICE, RESULT, CODE_TYPE, and CODE  *(from _seg5_s135893_43774c55873f.md)*
- /home/gyasis/Documents/code/athena_connector/raw-sql/validation/analysis/zero_measure_reports/CounsPhysAct_investigation.md:32:  - `data/measure_requirements_analysis.sql` - Line 22: Lists CounsPhysAct as required measure  *(from _seg5_s135893_43774c55873f.md)*
