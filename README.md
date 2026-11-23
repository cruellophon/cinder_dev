# Cinder (v0.6.0)

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
| [`bins/bumbo-app`](bins/bumbo-app) | Native desktop entry point (`cinder`) and Windows PE icon/resource embedding |
| [`crates/cinder-gui`](crates/cinder-gui) | Native GPU `eframe` / `egui` frontend (`chat`, `markdown`, `inspector`, `editor`, `explorer`, `forge`, `settings`, `sidebar`, `modals`, `plugins`, `customizations`, `theme`) |
| [`crates/harness-core`](crates/harness-core) | Multi-step agent loop, Orchestrator Planner/Executor routing, Vision & Research sub-agents, checkpoints, slash skills |
| [`crates/harness-tools`](crates/harness-tools) | Workspace-sandboxed file/dir tools, diff generator, `lopdf` PDF extractor, offline RAG index, MCP stdio client, `web_search`, safe OS URL opener |
| [`crates/harness-providers`](crates/harness-providers) | Provider adapters (Google Gemini, Anthropic, OpenAI-compat, local Ollama/`llama-server`), `<think>` tag extraction, fallback auto-switching |
| [`crates/harness-models`](crates/harness-models) | Hardware/GPU probe, VRAM fit estimator, Hugging Face GGUF inspector, resumable downloads, `llama-server` process manager |
| [`crates/harness-store`](crates/harness-store) | Atomic persistent state (`AppSettings`, `Session`, `ToolCallRecord`, quota tracking, drive inspection) |
| [`crates/harness-net`](crates/harness-net) | `NetworkGate` policy enforcement (`StrictLocal` vs. `Hybrid`) and real-time redacted network audit log |

---

## Downloads & Platform Support

Prebuilt native releases are published automatically via GitHub Actions on the [Releases page](https://github.com/cruellophon/cinder_dev/releases):

| Platform | Architecture | Package | Notes |
| :--- | :--- | :--- | :--- |
| **macOS (Intel)** | `x86_64` | `cinder-macos-intel.zip` | Native `Cinder.app` bundle for Intel Macs (macOS 12+) |
| **macOS (Apple Silicon)** | `aarch64` (`M1`/`M2`/`M3`/`M4`) | `cinder-macos-apple-silicon.zip` | Native `Cinder.app` bundle (macOS 12+) |
| **Linux** | `x86_64` | `cinder-linux-x86_64.tar.gz` | Standalone `cinder` binary + `.desktop` + `install.sh` |
| **Windows** | `x86_64` | `cinder.exe` | Single self-contained executable |

> **macOS First Launch (Gatekeeper Notice):** Community builds are ad-hoc signed rather than Apple-notarized. After moving `Cinder.app` into `/Applications`, clear the quarantine attribute once before opening:
> ```bash
> xattr -cr /Applications/Cinder.app
> ```

---

## Quick Start & Build from Source

### Prerequisites
- [Rust & Cargo](https://rustup.rs/) (stable toolchain, 2021 edition)
- **Linux (`apt`) system dependencies:**
  ```bash
  sudo apt-get update && sudo apt-get install -y \
    pkg-config libssl-dev libxcb-render0-dev libxcb-shape0-dev \
    libxcb-xfixes0-dev libxkbcommon-dev libgtk-3-dev libxdo-dev
  ```
- **macOS:** Xcode Command Line Tools (`xcode-select --install`). To cross-compile for an Intel Mac on Apple Silicon: `rustup target add x86_64-apple-darwin`

### Build & Test

```bash
# Run the full workspace unit test suite
cargo test --workspace

# Build the standalone single-file release executable
cargo build --release --bin cinder
```

- **Windows:** Copy standalone executable (`target\release\cinder.exe`) to `release\cinder.exe`.
- **macOS `.app` bundle:** `bash scripts/bundle-macos-app.sh target/release/cinder dist native 0.5.7-beta`
- **Linux archive:** `bash scripts/package-linux.sh target/release/cinder dist 0.5.7-beta`

### Runtime Flags & Environment Variables
- `--open-stage` / `--open-settings` / `--open-forge` / `--open-customizations` — open directly into a specific panel
- `CINDER_DATA_DIR=<PATH>` — override the runtime data directory where Cinder stores `state.json`, plugins, skills, and portable toolchains

---

## Privacy & API Key Storage

Cinder never stores API keys or runtime chat state inside source repositories. All provider keys (`gemini_api_key`, `anthropic_api_key`, `openrouter_api_key`, `openai_compat_api_key`, `huggingface_token`, `brave_search_api_key`), sessions, and preferences are stored strictly inside your local OS application data directory (`%LOCALAPPDATA%\Project_Bumbo\state.json` on Windows, `~/Library/Application Support/Project_Bumbo/state.json` on macOS, `~/.local/share/Project_Bumbo/state.json` on Linux, or `$CINDER_DATA_DIR`).

See [`SECURITY.md`](SECURITY.md) for details on sandboxing, network gating, and vulnerability reporting, and [`CHANGELOG.md`](CHANGELOG.md) for full release notes.

---

## Keyboard Shortcuts

| Windows / Linux | macOS | Action |
| :--- | :--- | :--- |
| `Ctrl+B` | `Cmd+B` | Toggle Left Sidebar |
| `Ctrl+E` | `Cmd+E` | Toggle Workbench Stage |
| `Ctrl+Shift+E` | `Cmd+Shift+E` | Toggle Workspace File Explorer |
| `Ctrl+K` | `Cmd+K` | Command Palette |
| `Ctrl+,` | `Cmd+,` | Settings Control Center |
| `Ctrl+S` | `Cmd+S` | Save Active Editor File |
| `Ctrl+V` | `Cmd+V` | Paste Screenshot / Clipboard Image or Files into Chat |
| `` Ctrl+` `` | `` Cmd+` `` | Toggle Integrated Workspace Terminal |

---

## License

Licensed under the [MIT License](LICENSE).
