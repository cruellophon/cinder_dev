# Changelog

All notable changes to **Cinder** are documented in this file.

---

## [0.6.0] — Streaming Everywhere, Git & Terminal Workbench, Model Consistency & Turn-Scoped Fallback — 2026-11-23

### Added
- **Model Consistency, `FallbackPolicy` & Ordered Fallback Chain (`harness-store`, `harness-providers`, `harness-core`, `cinder-gui`):**
  - Added 3-way `FallbackPolicy` (`Ask` [**Ask First**, default], `AutoChain` [**Auto-Chain**], and `Strict` [**Never**]) plus a user-configurable **Ordered Fallback Chain** (`AppSettings::fallback_chain`) in **Settings → Models & Keys → Model Routing & Failover**.
  - Added in-chat **Model Fallback** action card (`Session::pending_fallback` / `PendingModelFallback`) offering `Retry <Model>`, `Continue with <Candidate>`, and `Cancel` without dropping accumulated multi-step tool progress.
  - Added persistent `Fallback: Orig → Actual` footer badge and one-click `Retry with Orig` action on fallback-completed messages (`ChatMessage::fallback_from_model_id`, `fallback_reason`).
  - Pinned `PlanArtifact::executor_model_id` across Orchestrator plan review/execution so approved plans execute with the planned model rather than drifting silently.
- **Git Status & Diff Integration (`harness-tools/src/git.rs`, `cinder-gui/src/{explorer.rs,editor.rs}`):**
  - Added Git repository status parsing (`branch`, ahead/behind counts, per-file status badges) in the **Explorer** pane and side-by-side/unified Git diff previews in the **Editor**.
- **Cross-Session & Message Search (`cinder-gui/src/sidebar.rs`):**
  - Added instant UTF-8-safe sidebar search across conversation titles, workspace paths, and message contents.
- **Persistent Terminal Working Directory (`cinder-gui/src/{backend.rs,inspector.rs}`):**
  - Tracked `cwd` across consecutive commands executed inside the Workbench integrated terminal drawer.
- **Cloud Secret-File Guard (`harness-tools/src/risk.rs`):**
  - Automatically escalates read/search operations on sensitive credential or key paths (`.env`, `.pem`, `.key`, secrets) for approval when executing under cloud models.
- **Local `llama-server` Crash Diagnostics & GPU OOM Risk Estimator (`harness-models`, `cinder-gui/src/forge.rs`):**
  - Added `assess_oom_risk` pre-flight warnings and `llama-server` stderr ring-buffer diagnostics so failed local launches report exact driver/VRAM causes instead of generic connection errors.

### Changed
- **Turn-Scoped Model Failover & Transient Primary Retry (`harness-providers`, `harness-core`, `orchestrator.rs`, `subagent.rs`):**
  - Stopped silently mutating `AppSettings::active_model_id` during provider failover or sub-agent routing; fallback switches now apply strictly to the active turn.
  - Added an automatic single 800ms transient retry on the user's selected cloud model before triggering `NeedsFallbackChoice` or `AutoChain` switching, and enforced strict chain adherence (no unrequested jumps outside a non-empty `fallback_chain`, always filtering `!requires_payment` and required tool/vision capabilities).
  - Anchored Orchestrator `Auto` planner/executor resolution and `Vision` / `Research` sub-agents to the user's active free cloud model and `fallback_chain` first.
- **Streaming Everywhere (`harness-tools`, `harness-providers`, `cinder-gui`):**
  - Streamed live output chunks from `run_command` shell executions, OpenAI-compatible SSE streams, and Anthropic delta accumulators directly into Chat traces.
- **Single-Source Workspace Versioning & Automated Release Notes (`Cargo.toml`, `.github/workflows/release.yml`, `scripts/`):**
  - Synchronized workspace crate version (`0.6.0`) via `env!("CARGO_PKG_VERSION")` across UI labels and build scripts, and configured `.github/workflows/release.yml` to extract release notes directly from `CHANGELOG.md`.

---

## [0.5.8-beta] — UI Overhaul, Model Aliases, Provider Probing & Security Hardening — 2026-11-23

### Added
- **Working Model Aliases & Assigned Aliases Ledger (`harness-store`, `cinder-gui/src/settings.rs`):**
  - Added persisted `AppSettings::model_aliases` (`BTreeMap<String, String>`) and `resolve_model_display_name` so assigned aliases appear consistently across Chat pickers, message footers, compare cards, live turn banners, Orchestrator dropdowns, and Forge tiles.
  - Rebuilt the bottom section of **Settings → Models & Keys** into **Model Aliases** featuring an active-model dropdown (`ComboBox`), alias text box, `Assign Alias` action, and an **Assigned Aliases Ledger** below with per-row `Remove` buttons (while tucking advanced custom cloud ID registration into a clean collapsible header).
- **OpenRouter & Multi-Provider Availability / Credit Probing (`harness-providers`, `harness-store/src/usage.rs`):**
  - Added `requires_payment: bool` to `ModelCatalogEntry` and `ModelQuotaSnapshot`.
  - Updated `sync_openrouter_models` to check `/api/v1/auth/key`, `/api/v1/credits`, and `/api/v1/models` pricing (`prompt` / `completion` and `:free`), separating free/reachable models from payment-gated models (discovering up to 40 free + 40 paid models cleanly) and classifying HTTP `402` / insufficient-credit errors in `classify_provider_error`.
  - Added `probe_gemini_model_status` and filtered `SelectableModel` / `build_fallback_candidates` so paid and unreachable models stay hidden from Chat pickers while appearing under dedicated **`Available`**, **`Paid / Credits`**, and **`Unreachable`** tabs in **Forge → Cloud & Quotas** (`Probe All Providers`).
- **Inline Quota & VRAM Fit Indicators (`cinder-gui/src/chat.rs`, `app_state.rs`):**
  - Added color-coded daily quota remaining badges (`100%`, `82%`) and local VRAM fit pills (`100% GPU`, `Hybrid GPU`, `CPU Heavy`) directly inside the Chat model trigger pill and right-aligned on every model picker row, with detailed quota/VRAM hover tooltips.
- **Unified Reusable Model Tile Grid (`cinder-gui/src/forge.rs`):**
  - Built a shared responsive `render_model_tile` + `model_tile_grid_layout` card component (with concise non-clipping `concise_vram_badge`, progress bar, metadata lines, and pinned action bar) reused across **Local Vault**, **Hugging Face Search Results**, and **Cloud Model Quotas**.
- **Trace Windowing & Contextual Popover Link (`cinder-gui/src/chat.rs`, `app_state.rs`, `sidebar.rs`, `lib.rs`):** Windowed long thought/tool chains cleanly (`window_trace_slice`) behind expandable toggles while keeping earlier errors visible and preserving full `ToolCallRecord` data; consolidated `GuiState::is_busy()`, `close_transient_popovers()`, and mutually exclusive popovers/command palette; removed unused `orch_steps_draft` state and added an `Aliases →` jump link in the Mode & Model popover.
- **Symlink Sandbox Regression Test (`harness-tools/src/sandbox.rs`):**
  - Added `resolve_workspace_path_blocks_symlinks_pointing_outside_workspace` verifying workspace escape blocking across symlinks.

### Changed
- **Smooth Window Dragging, Scrolling & `Shift+Enter` Composer Newlines (`cinder-gui`):**
  - Eliminated idle 30 Hz GPU/swapbuffer polling on the static topbar brand emblem and upgraded active focus animations to native display VSync cadence (`request_smooth_anim_repaint`) while backing off when unfocused; initiated `ViewportCommand::StartDrag` immediately on pointer movement in the full topbar drag strip.
  - Tuned global `ScrollStyle` and `line_scroll_speed` (`56.0`) with `reduce_texture_memory = true`, switched `GuiState::snap` to zero-copy `Arc<UiSnapshot>`, added thread-local Markdown AST & syntax-highlighted `LayoutJob` caches, replaced per-frame `TextEdit` code blocks with lightweight selectable labels (only capturing vertical wheel events when overflowed), and added 450px-overscan offscreen culling to the chat stream.
  - Wired `Shift+Enter` in the chat composer to insert newlines via `.return_key(...)` while bare `Enter` sends, wrapped inside a scrollable auto-growing composer input (up to `180 px`).
- **Settings Alignment & Save Button Swap (`cinder-gui/src/settings.rs`):**
  - Locked label and status columns (`fixed_label_col` / `fixed_status_col`) so all 6 provider credential rows (including split `OpenAI-Compat` Base URL + API Key inputs) and Orchestrator/Engine controls align evenly.
  - Swapped save button order and colors: **`Save Keys Offline`** (primary orange, left) and **`Save & Live-Probe Models`** (secondary grey, right).
- **Chat Mode/Model Popover & Turn-Only Working Counter (`cinder-gui/src/chat.rs`, `harness-core/src/lib.rs`):**
  - Replaced the downward-clipping `menu_button` with a custom `egui::Area` popover opening above the composer and to the left with streamlined rows and single-line model entries.
  - Replaced `step X/Y` step ceilings in live working indicators and trace headers with turn-only counters (`Turn N`).
- **Markdown Table Layout Overhaul (`cinder-gui/src/markdown.rs`):**
  - Replaced unconstrained `egui::Grid` tables with `compute_table_column_widths` and bordered, striped card tables enforcing readable minimum column widths (`130 px` / `110 px`) and proportional full-width wrapping so words no longer break mid-word.
- **Dead Code & WebView Cleanup (`ecbacde`):**
  - Removed the legacy Axum/HTML/WebView stack and unused Rust helpers (`-9,316` lines), reducing `cinder.exe` size from ~39.2 MB to ~33.5 MB.

### Fixed
- **Live Orchestrator Working UX & Eliminated Duplicate Status Pills (`harness-core/src/orchestrator.rs`, `cinder-gui/src/chat.rs`, `app_state.rs`):** Streamed real-time Phase 1 Planner and Phase 2 Executor progress, resolved model alias names, completed tool calls, and thought traces via `set_activity_stream`; replaced the duplicate waiting box in the message stream with a live turn panel featuring a `1. Planner → 2. Executor` stepper, animated ember shimmer bar, and ticking turn elapsed timer.
- **Viewport-Bounded & Scrollable Plan Modals (`cinder-gui/src/modals.rs`, `lib.rs`):** Constrained `EditPlan`, `RequestPlanChanges`, `NewArtifact`, and `LightboxImage` modals to viewport dimensions with internal vertical `ScrollArea`s so long plans never push close/action buttons off-screen, and wired `Escape` to dismiss any open prompt modal.
- **Plan → Artifacts Sync & Gemma Inline Tool Sanitization (`harness-core/src/lib.rs`, `orchestrator.rs`, `cinder-gui/src/chat.rs`, `inspector.rs`):** Parsed Gemma-style inline `<|tool_call|>call:submit_plan{...}<|tool_call|>` blocks and stripped leaked tool/think tags from fallback plans; guaranteed every generated or edited plan is upserted into **Workbench → Artifacts**; added **Open in Artifacts** on chat Plan cards and **Approve & Execute** on pending plan artifacts.
- **Nested Backticks Inside Bold Markdown (`cinder-gui/src/markdown.rs`):** Fixed `**bold (`code`)**` inline spans rendering literal backticks instead of styled monospace pills.
- **Shell Command Risk Classifier Hardening (`harness-tools/src/risk.rs`):** Replaced prefix-based safe command matching (which previously allowed prefix collisions such as `dir`/`dirty-script` and `ls`/`lsof`) with exact parsed-token allowlisting (`tokenize_shell_command`), closed metacharacter/chaining gaps (`|`, `&`, `\n`, `\r`, `<`, `${`), and expanded `HighRiskDestructive` coverage for PowerShell/.NET calls (`iex`, `Invoke-Expression`, `[IO.Directory]::Delete`, etc.).
- **Shell Execution & Process Hygiene (`harness-tools/src/lib.rs`, `risk.rs`):** Set `Stdio::null()` stdin and `kill_on_drop(true)` on `run_command` child processes with explicit 60s timeout termination messages; extended `risk.rs` to block `git diff --output=...` / `--git-dir` / `--work-tree` overrides from `SafeReadOnly` and flag encoded/nested shell launchers (`-EncodedCommand`, `cmd /c`, `pwsh -c`, `powershell -c`, `bash -c`).
- **Provider Failover, Local `llama-server` Recovery & Gemini 3 `thoughtSignature` Fallback (`harness-providers/src/lib.rs`):**
  - Fixed HTTP 400 `Function call is missing a thought_signature in functionCall parts` during cross-provider auto-switch into Gemini 3.x models by injecting `"skip_thought_signature_validator"` when replaying non-Gemini/local tool calls without native thought signatures (while preserving authentic `thoughtSignature` values verbatim).
  - Forced `Connection: close` on local inference requests to prevent stale keep-alive socket reuse against `llama-server`, and added automatic single-retry local server restart/recovery on dropped loopback connections before triggering cloud auto-switch.
  - Distinguished context-length errors (delegated cleanly to mid-task compaction without false switching/blocking), recognized OpenRouter `"No endpoints found"` and `"Provider returned error"` responses, and added an 800ms transient single-retry fallback when auto-switch is unavailable.
- **Immediate Dismissal of Step Contingent & Tool Approval Boxes (`cinder-gui/src/chat.rs`, `harness-core/src/lib.rs`):** Cleared `pending_question` and `pending_approval` immediately upon option/approval selection in both UI snapshot state and persistent store before resuming execution so prompts disappear instantaneously instead of lingering during the continued turn.
- **Security & Documentation Consistency (`SECURITY.md`, `README.md`):** Aligned OS state paths (`%LOCALAPPDATA%\Project_Bumbo\state.json`), version headings, and shell execution scope notes.

---

## [0.5.7-beta] — Exclusive Privacy Toggle, `fetch_url` & Research Sub-Agent — 2026-11-22

### Added
- **`fetch_url` Tool & Web Search Enrichment (`harness-tools/src/lib.rs`):** Added SSRF-safe `fetch_url` with GitHub API release/repo fast-path mapping, HTML-to-readable-text extraction, and automatic top-result fetching in `web_search`.

### Fixed
- **Exclusive Local ↔ Hybrid Privacy Toggle (`cinder-gui/src/sidebar.rs`, `harness-models`):** Removed silent `StrictLocal` auto-switching when starting local models and duplicate GUI toggles, making the bottom-left **System & Forge Telemetry** popover the sole user control over Local ↔ Hybrid mode.
- **Research Sub-Agent Guardrails (`harness-core/src/subagent.rs`):** Hardened research delegation output rules so delegated web lookups never fabricate or alter URLs.

---

## [0.5.6-beta] — Cross-Platform macOS & Linux Support — 2026-11-22

### Added
- **First-Class macOS Support (Intel `x86_64-apple-darwin` & Apple Silicon `aarch64-apple-darwin`):**
  - Native macOS full-size content view titlebar with traffic-light button clearance and `Cmd` keyboard shortcuts (`Cmd+B`, `Cmd+E`, `Cmd+Shift+E`, `Cmd+K`, `Cmd+,`, `Cmd+S`, `Cmd+F`, `Cmd+W`, ``Cmd+` ``, `Cmd+V`).
  - macOS Finder GUI `PATH` augmentation (`/opt/homebrew/bin`, `/usr/local/bin`, `~/.cargo/bin`, `~/.local/bin`) and `/Applications/Ollama.app/Contents/Resources/ollama` discovery.
  - Intel Mac `system_profiler SPDisplaysDataType` GPU & VRAM probing (fixing false "Apple Silicon Unified Memory" reporting on `x86_64` Macs while preserving Unified Memory on `aarch64`).
  - Native macOS system fonts (`SFNS`, `Helvetica Neue`, `Menlo`, `Apple Symbols`), escaped `osascript` multi-file/folder pickers, and `DYLD_LIBRARY_PATH` runtime loading for local `llama-server`.
- **First-Class Linux Support (`x86_64-unknown-linux-gnu`):**
  - Native Linux font loading (`DejaVu Sans`, `Ubuntu`, `Noto Sans`, `Liberation`), `/sys/class/drm` GPU VRAM fallback alongside `nvidia-smi`, `kdialog` fallback alongside `zenity`, and `LD_LIBRARY_PATH` setup for `llama-server`.
- **Pure-Rust `.tar.gz` / `.tgz` Engine & Node.js Installers (`flate2` + `tar`):**
  - Updated `llama.cpp` release asset matching & extraction (`select_llama_assets_for`, `extract_archive_flattened`) and Portable Node.js LTS toolchain installer (`nodejs_dist_spec_for`, `extract_targz_preserving_tree`) to support modern `.tar.gz` and `.zip` builds across Windows, macOS Intel/ARM64, and Linux, preserving symlinks and Unix `0o755` executable permissions.
- **Automated Multi-Platform Packaging & GitHub Actions Workflows:**
  - Added [`scripts/bundle-macos-app.sh`](scripts/bundle-macos-app.sh) (`Cinder.app` bundle + generated `Cinder.icns` + ad-hoc `codesign`), [`scripts/package-linux.sh`](scripts/package-linux.sh) (`.tar.gz` + `.desktop` + `install.sh`), [`.github/workflows/ci.yml`](.github/workflows/ci.yml), and [`.github/workflows/release.yml`](.github/workflows/release.yml).

### Fixed
- Replaced hardcoded `\` path joins in File Explorer, Drag-and-Drop, and Model Vault drive quick-select with portable `PathBuf::join` composition.
- Updated `sandbox::tests::relative_to_workspace_handles_verbatim_prefix_mismatch` and `native_dialogs::tests` to verify both POSIX and Windows paths cross-platform.

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
