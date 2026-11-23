# Security Policy

## Credential & Secret Handling

- Cinder **never** stores API keys, tokens, chat histories, or local model weights inside project workspaces or Git repositories.
- Provider credentials and persistent application state live strictly in the OS application data directory (`%LOCALAPPDATA%\Project_Bumbo\state.json` on Windows, `~/Library/Application Support/Project_Bumbo/state.json` on macOS, `~/.local/share/Project_Bumbo/state.json` on Linux, or `$CINDER_DATA_DIR`).
- Runtime artifacts (`target/`, `release/`, `.bumbo/`, `.cinder/`, `state.json`, `knowledge_index.json`, `.env*`, key files, and `*.gguf` binaries) are excluded via [`.gitignore`](.gitignore).

## Runtime Isolation & Defense-in-Depth

- **Network Gate (`harness-net`):** When set to `Strict Local Mode`, all outbound non-loopback HTTP requests are rejected before socket creation, and all allowed/blocked requests are logged to the Network Audit trail. Models cannot change network privacy mode via tool calls (`switch_mode` accepts work modes only).
- **Workspace Path Sandbox (`harness-tools/src/sandbox.rs`):** All workspace file/directory tools resolve paths through `resolve_workspace_path`, canonicalizing targets (including symlinks) and blocking `..` parent traversal or escapes outside the active workspace root.
- **External URL Handling (`harness-tools/src/native_dialogs.rs`):** `open_url_safely` strictly allows `http`, `https`, and `mailto` schemes and invokes OS handlers (`ShellExecuteW` on Windows, `open` on macOS, `xdg-open` on Linux) without shell interpolation.
- **Tool Approval Gates & Shell Scope:** Workspace writes, destructive actions, and shell execution (`run_command`, `verify_workspace`) are classified by `classify_tool_risk` (`harness-tools/src/risk.rs`) using a parsed-token allowlist and high-risk heuristics to drive user approval prompts (`auto_approve_commands` defaults to `false`), alongside per-turn file checkpoints (`harness-core/src/checkpoints.rs`). Because `run_command` executes with the current user's OS privileges through PowerShell or `sh` rather than an OS-level container, shell commands are not restricted by `resolve_workspace_path` and should be reviewed before approval.

## Reporting a Vulnerability

If you discover a security issue, privacy leak, or sandbox bypass in Cinder, please report it responsibly by opening a private security advisory or contacting the repository owner at <https://github.com/cruellophon/cinder_dev/security>.
