# Cinder (v5.5)

> A local-first, 100% pure-Rust desktop agentic IDE and GGUF model forge powered by a native `eframe` / `egui` GPU frontend (`cinder-gui`) and modular backend crates — zero Electron, zero WebView bloat, and strict network privacy controls.

Repository: <https://github.com/cruellophon/cinder_dev>

---

## Core Capabilities

- **Local-First GGUF Model Forge (`harness-models`):**
  - Search and inspect Hugging Face GGUF repositories with per-quantization GPU VRAM fit estimation.
  - Managed local `llama-server` lifecycle with pause/resume chunked downloads, automatic idle VRAM unloading, and support for Ollama, LM Studio, or any OpenAI-compatible local endpoint.
- **Strict Local vs. Hybrid Network Gate (`harness-net`):**
  - **Strict Local Mode:** Physically blocks non-loopback sockets and logs every loopback/external decision in the live Network Audit trail.
  - **Hybrid Mode:** Enables external cloud APIs (Google Gemini, Anthropic, OpenAI/OpenRouter), live web search, and model/plugin installers when explicitly enabled by the user. Models are strictly prevented from switching network privacy mode.
- **Multi-Mode Agentic Engine (`harness-core`):**
  - **`Auto`**, **`Chat`**, **`Plan`**, **`Agent`**, **`Orchestrator`** (dual-model Planner + Executor with **`Auto` Best Planner / Execution Model** selection), and **`Compare`** (side-by-side model evaluation).
  - Live borderless inline thought traces with animated working indicators, per-step colored file badges, and automatic structured post-work summaries.
- **Multimodal Vision & Sub-Agent Delegation (`harness-core/src/subagent.rs`):**
  - Paste screenshots directly via `Ctrl+V` (Win32 `CF_DIB` / `CF_DIBV5` + Explorer file lists) or drag-and-drop images, code files, PDFs, and folders anywhere onto the window.
  - Dedicated **Vision Sub-Agent** (local `--mmproj` or cloud vision) and **Research Sub-Agent** (automatically delegates to reachable cloud models such as Gemini in Hybrid Mode, with exact delegated model visibility in the UI trace).
- **Live Zero-Key Web Search & Clickable Chat Links:**
  - Multi-tier `web_search` in Hybrid Mode combining Brave Search (if configured), the official Wikipedia MediaWiki API, DuckDuckGo HTML Lite, and Instant Answer fallback — failing honestly instead of fabricating URLs.
  - Interactive Markdown `[label](url)` and bare `https://` links inside Chat, Artifacts, and Markdown previews using scheme-restricted OS launching (`http`, `https`, `mailto`).
- **Offline Workspace RAG & IDE Workbench (`harness-tools`, `cinder-gui`):**
  - 100% offline PDF text extraction (`lopdf`) and semantic + BM25 workspace indexing.
  - Workspace File Explorer, syntax-highlighted Code Editor + live Markdown preview, resizable code/diff/artifact boxes, integrated PowerShell/shell terminal drawer, turn checkpoints with per-file accept/revert and rollback, MCP server integration, custom slash skills, and 5 UI themes.

---

## Workspace Architecture

| Crate / Binary | Responsibility |
| :--- | :--- |
| [`bins/bumbo-app`](bins/bumbo-app) | Desktop & optional `--headless` HTTP server entry point (`cinder` / `bumbo-app`), embedded assets, Axum routes |
| [`crates/cinder-gui`](crates/cinder-gui) | Native GPU `eframe` / `egui` frontend (`chat`, `markdown`, `inspector`, `editor`, `explorer`, `forge`, `settings`, `sidebar`, `modals`, `plugins`, `customizations`, `theme`) |
| [`crates/harness-core`](crates/harness-core) | Multi-step agent loop, Orchestrator Planner/Executor routing, Vision & Research sub-agents, checkpoints, slash skills |
| [`crates/harness-tools`](crates/harness-tools) | Workspace-sandboxed file/dir tools, diff generator, `lopdf` PDF extractor, offline RAG index, MCP stdio client, `web_search`, safe OS URL opener |
| [`crates/harness-providers`](crates/harness-providers) | Provider adapters (Google Gemini, Anthropic, OpenAI-compat, local Ollama/`llama-server`), `<think>` tag extraction, fallback auto-switching |
| [`crates/harness-models`](crates/harness-models) | Hardware/GPU probe, VRAM fit estimator, Hugging Face GGUF inspector, resumable downloads, `llama-server` process manager |
| [`crates/harness-store`](crates/harness-store) | Atomic persistent state (`AppSettings`, `Session`, `ToolCallRecord`, quota tracking, drive inspection) |
| [`crates/harness-net`](crates/harness-net) | `NetworkGate` policy enforcement (`StrictLocal` vs. `Hybrid`) and real-time redacted network audit log |

---

## Quick Start & Build from Source

### Prerequisites
- [Rust & Cargo](https://rustup.rs/) (stable toolchain, 2021 edition)

### Build & Test

```powershell
# Run the full workspace unit test suite
cargo test --workspace

# Build the standalone single-file release executable
cargo build --release --bin cinder

# Copy standalone executable to release/ (optional local release folder, gitignored)
Copy-Item -Force target\release\cinder.exe release\cinder.exe
```

### Runtime Flags & Environment Variables
- `--headless` — start the embedded Axum web UI & API server (`http://127.0.0.1:17770`) instead of opening the native GUI
- `--port <PORT>` — bind `--headless` to a custom loopback port
- `--open-stage` / `--open-settings` / `--open-forge` / `--open-customizations` — open directly into a specific panel
- `CINDER_DATA_DIR=<PATH>` — override the runtime data directory where Cinder stores `state.json`, plugins, skills, and portable toolchains

---

## Privacy & API Key Storage

Cinder never stores API keys or runtime chat state inside source repositories. All provider keys (`gemini_api_key`, `anthropic_api_key`, `openrouter_api_key`, `openai_compat_api_key`, `huggingface_token`, `brave_search_api_key`), sessions, and preferences are stored strictly inside your local OS application data directory (`%APPDATA%\Cinder\state.json` on Windows, or `$CINDER_DATA_DIR`).

See [`SECURITY.md`](SECURITY.md) for details on sandboxing, network gating, and vulnerability reporting, and [`CHANGELOG.md`](CHANGELOG.md) for full release notes.

---

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Ctrl+B` | Toggle Left Sidebar |
| `Ctrl+E` | Toggle Workbench Stage |
| `Ctrl+Shift+E` | Toggle Workspace File Explorer |
| `Ctrl+K` | Command Palette |
| `Ctrl+,` | Settings Control Center |
| `Ctrl+S` | Save Active Editor File |
| `Ctrl+V` | Paste Screenshot / Clipboard Image or Files into Chat |
| `` Ctrl+` `` | Toggle Integrated Workspace Terminal |

---

## License

Licensed under the [MIT License](LICENSE).
