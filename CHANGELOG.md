# Changelog

All notable changes to **Cinder** are documented in this file.

---

## [0.5.5] / v5.5 — 2026-11-22

### Added
- **Clickable Links in Chat, Artifacts & Markdown Previews (`cinder-gui/src/markdown.rs`, `harness-tools/src/native_dialogs.rs`):**
  - Added scheme-validated `open_url_safely` (allowing strictly `http`, `https`, and `mailto` via Win32 `ShellExecuteW` without `cmd.exe` shell interpretation).
  - Added Unicode char-accurate `Galley` hit-testing for both `[label](url)` Markdown links and bare `https://` / `http://` URLs with pointer hover cursor and tooltips.
- **Zero-Key Live Web Search & Honest Reporting (`harness-tools/src/lib.rs`, `harness-core/src/lib.rs`):**
  - Added multi-tier `web_search` in Hybrid Mode combining Brave Search (if configured), the official Wikipedia MediaWiki API, DuckDuckGo HTML Lite (`uddg=` redirect unwrapping), and DuckDuckGo Instant Answer fallback.
  - Exposed `web_search` and `delegate_research` in `Chat` and standalone sessions whenever Hybrid Mode is active, failing explicitly if no live results return rather than letting models invent links.
- **Proactive Cloud Delegation & Exact Sub-Model Visibility (`harness-core`, `harness-store`, `cinder-gui`):**
  - Added `resolve_research_model` so local models in Hybrid Mode automatically delegate `delegate_research` calls to reachable cloud models (preferring Gemini) with live `web_search` enabled, while staying offline in `StrictLocal` Mode.
  - Added `subagent_model: Option<String>` to `ToolCallRecord`, rendering `Vision Sub-Agent → <model>` and `Research Sub-Agent → <model>` directly in inline traces, `Delegated model:` badges in drawers, and `[WEB] Searched web` rows.
- **`Auto` Planner & Executor Selection (`harness-core/src/orchestrator.rs`, `cinder-gui/src/settings.rs`):**
  - Added `resolve_auto_executor_model` and `resolve_auto_planner_model` along with **Auto (Best Planner Model)** and **Auto (Best Execution Model)** dropdown options in Orchestrator Settings (picking top reachable cloud models in Hybrid Mode or loaded/VRAM-fitting local models in Strict Local Mode).
- **Resizable Code, Diff, Artifact & Tool Drawers (`cinder-gui/src/markdown.rs`, `inspector.rs`, `chat.rs`):**
  - Added bottom drag handles, double-click fit/reset, and `Expand` / `Collapse` toggles for Markdown code snippets, unified diffs, Artifact raw editors, and expanded tool output drawers (with isolated per-artifact and per-message scroll/height IDs).
- **Inline Thought Traces & Structured Summaries (`cinder-gui/src/chat.rs`, `harness-core/src/lib.rs`):**
  - Made the activity and planning trace render inline without an outer card box, added animated `. . .` working dots behind active steps, auto-collapsed finished traces, and enforced structured post-work summaries.
- **Multimodal `Ctrl+V` & Drag-and-Drop (`cinder-gui/src/app_state.rs`, `explorer.rs`, `lib.rs`):**
  - Added Win32 `CF_DIB` / `CF_DIBV5` clipboard screenshot fallback, clipboard file-list attachment (`Ctrl+V`), Win32 `WM_DROPFILES` filters, external folder/file drop handling, and in-app File Explorer drag-and-drop.
- **Repository & Documentation Hygiene:**
  - Added `.gitignore`, `CHANGELOG.md`, `SECURITY.md`, MIT `LICENSE`, and scrubbed personal workspace paths.

### Changed
- Standardized standalone distribution into a single self-contained `cinder.exe` executable with embedded orb icons and assets.
- Updated `website/index.html` links to target <https://github.com/cruellophon/cinder_dev>.

### Fixed
- **Strict Local vs. Hybrid Privacy Synchronization (`cinder-gui/src/backend.rs`, `sidebar.rs`, `lib.rs`, `bumbo-app/src/routes.rs`):**
  - Made `NetworkGate::mode()` the single source of truth across snapshots, `LiveTick`, the lower-left footer indicator, and `/api/state`.
  - Stopped `start_local_model` from overwriting persisted `privacy_mode` without updating `NetworkGate`, and removed silent `StrictLocal → Hybrid` overrides on unrelated settings changes or chat turns.
- **Blocked Models From Changing Network Privacy Mode (`harness-core/src/lib.rs`, `harness-tools/src/lib.rs`, `harness-net/src/lib.rs`):**
  - Changed `parse_target_mode` to `Option<AgentMode>` so `switch_mode` accepts only `'chat'`, `'plan'`, `'agent'`, and `'orchestrator'`, rejecting privacy keywords (`hybrid`, `local`, `strict_local`, `online`, `offline`) with an explicit permission error.
- **Workspace Pathing & `edit_file` Reliability (`harness-tools/src/sandbox.rs`, `harness-tools/src/lib.rs`):**
  - Normalized quotes/backticks, `%20`, `file:///` URIs, `@` prefixes, and redundant `<workspace_folder>/` prefixes inside `resolve_workspace_path` while preserving strict sandbox traversal blocks.
  - Added nested path fallback parity across `edit_file`, `propose_edit`, `list_dir`, `search_files`, `read_file`, and `extract_pdf_text`.
  - Added `strip_read_file_line_numbers` fallback in `prepare_file_change` and actionable recovery errors when truncated `{"raw": ...}` tool arguments occur.

---

## [0.5.0] — Earlier Milestone

- Completed pure-Rust migration from WebView wrapper to native `eframe` / `egui` GPU frontend (`crates/cinder-gui`), local GGUF Forge with VRAM fit estimation, multi-provider adapter suite, checkpoint rollback, MCP server bridge, and 5 customizable UI themes.
