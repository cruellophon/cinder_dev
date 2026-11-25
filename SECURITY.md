# Security Policy

## Credential & Secret Handling

- Cinder **never** stores API keys, tokens, chat histories, or local model weights inside project workspaces or Git repositories.
- Provider credentials and persistent application state live strictly in the OS application data directory (`%LOCALAPPDATA%\Project_Bumbo\state.json` on Windows, `~/Library/Application Support/Project_Bumbo/state.json` on macOS, `~/.local/share/Project_Bumbo/state.json` on Linux, or `$CINDER_DATA_DIR`), protected with strict user-only permissions (`0o700` directory / `0o600` file on Unix).
- Runtime artifacts (`target/`, `release/`, `.bumbo/`, `.cinder/`, `state.json`, `knowledge_index.json`, `.env*`, key files, and `*.gguf` binaries) are excluded via [`.gitignore`](.gitignore).

## Runtime Isolation & Defense-in-Depth

- **Network Gate & Redirect/DNS SSRF Guards (`harness-net`, `harness-tools`):** When set to `Strict Local Mode`, all outbound non-loopback HTTP requests and HTTP redirects (`Location` hops) are rejected before socket creation, and all allowed/blocked requests are logged to the Network Audit trail. Models cannot change network privacy mode via tool calls (`switch_mode` accepts work modes only). `fetch_url` resolves DNS ahead of connection, blocks loopback/private/link-local/CGNAT (`100.64.0.0/10`)/`0.0.0.0/8`/ULA ranges, and re-validates every manual redirect hop.
- **Workspace & Scratchpad Path Sandbox (`harness-tools/src/sandbox.rs`, `harness-tools/src/packages.rs`):** All workspace and standalone session scratchpad file tools resolve paths through `resolve_workspace_path`, validating every path component via `symlink_metadata` so parent traversal (`..`), outside symlinks, and **dangling symlinks** cannot escape the root. Tar/zip extraction validates archive entries and symlink targets.
- **File Launch, Config-Write & MCP Hygiene (`harness-tools/src/{risk.rs,git.rs,native_dialogs.rs,mcp.rs}`):** `open_url_safely` strictly allows `http`, `https`, and `mailto`; `open_with_system_default` blocks symlinks and dangerous executable/script extensions (`exe`, `bat`, `cmd`, `ps1`, `vbs`, `hta`, `lnk`, etc.) and uses Win32 `ShellExecuteW` without shell interpolation; writes to `.git/*`, `.cargo/config*`, `.vscode/tasks.json`, or `mcp_config.json` always escalate to manual approval; workspace-imported MCP servers default to disabled; and Git probes force `core.fsmonitor=false` and `protocol.file.allow=never`.
- **Tool Approval Gates, Environment Scrubbing & Opt-In Runtimes (`harness-tools/src/{risk.rs,software.rs}`):** `classify_tool_risk` restricts `SafeReadOnly` commands to side-effect-free allowlisted read-only invocations with confined relative path arguments (no subexpressions or `cargo` build scripts), scrubs sensitive cloud/token environment variables from child shell and MCP stdio processes, and defaults workspace shell execution inside standalone scratchpads to manual approval. Optional user-configured CLI runtimes (`AllowedRuntimeEntry` via `run_local_software`) are off by default, never leaked in cloud system prompts, require manual approval on every invocation, clear child environments (`env_clear`), block inline eval flags, and enforce OS process-tree termination (Win32 Job Objects / Unix process groups) on timeout or cancellation.

## Reporting a Vulnerability

If you discover a security issue, privacy leak, or sandbox bypass in Cinder, please report it responsibly by opening a private security advisory or contacting the repository owner at <https://github.com/cruellophon/cinder_dev/security>.
