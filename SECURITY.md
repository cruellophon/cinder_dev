# Security Policy

## Credential & Secret Handling

- Cinder **never** stores API keys, tokens, chat histories, or local model weights inside project workspaces or Git repositories.
- Provider credentials and persistent application state live strictly in the OS application data directory (`%APPDATA%\Cinder\state.json` on Windows, or `$CINDER_DATA_DIR`).
- Runtime artifacts (`target/`, `release/`, `.bumbo/`, `.cinder/`, `state.json`, `.env*`, key files, and `*.gguf` binaries) are excluded via [`.gitignore`](.gitignore).

## Runtime Isolation & Defense-in-Depth

- **Network Gate (`harness-net`):** When set to `Strict Local Mode`, all outbound non-loopback HTTP requests are rejected before socket creation, and all allowed/blocked requests are logged to the Network Audit trail. Models cannot change network privacy mode via tool calls (`switch_mode` accepts work modes only).
- **Workspace Path Sandbox (`harness-tools/src/sandbox.rs`):** All workspace file/directory tools resolve paths through `resolve_workspace_path`, canonicalizing targets and blocking `..` parent traversal or escapes outside the active workspace root.
- **External URL Handling (`harness-tools/src/native_dialogs.rs`):** `open_url_safely` strictly allows `http`, `https`, and `mailto` schemes and invokes OS handlers (`ShellExecuteW` on Windows, `open` on macOS, `xdg-open` on Linux) without shell interpolation.
- **Tool Approval Gates:** Destructive actions, shell execution (`run_command`, `verify_workspace`), and workspace writes are risk-classified (`harness-tools/src/risk.rs`) and reversible via per-turn file checkpoints (`harness-core/src/checkpoints.rs`).

## Reporting a Vulnerability

If you discover a security issue, privacy leak, or sandbox bypass in Cinder, please report it responsibly by opening a private security advisory or contacting the repository owner at <https://github.com/cruellophon/cinder_dev/security>.
