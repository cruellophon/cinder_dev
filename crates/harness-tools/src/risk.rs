use harness_store::RiskTier;
use serde_json::Value;
use std::path::Path;

const HIGH_RISK_SUBSTRINGS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "rm -r ",
    "remove-item",
    "del /s",
    "rmdir /s",
    "format-volume",
    "clear-content",
    "[io.directory]::delete",
    "[system.io.directory]::delete",
    "[io.file]::delete",
    "[system.io.file]::delete",
    "invoke-expression",
    "invoke-webrequest",
    "invoke-restmethod",
    "set-executionpolicy",
    "stop-process",
    "start-process",
    "git reset --hard",
    "git clean -f",
    "git push --force",
    "git push -f",
    "pip install",
    "npm install",
    "cargo install",
    "chmod 777",
    "dd if=",
    "mkfs",
    "kill -9",
    "reg delete",
    "reg add",
    "net user",
    "net localgroup",
    "schtasks",
    "| sh",
    "| bash",
    "| iex",
    "-encodedcommand",
    "-enc ",
    "cmd /c",
    "powershell -c",
    "pwsh -c",
    "bash -c",
    "sh -c",
];

const HIGH_RISK_PROGRAMS: &[&str] = &[
    "iex",
    "irm",
    "iwr",
    "pwsh",
    "powershell",
    "cmd",
    "rm",
    "del",
    "erase",
    "rmdir",
    "rd",
    "ri",
    "curl",
    "wget",
    "winget",
    "shutdown",
    "reboot",
    "poweroff",
    "reg",
    "netsh",
    "taskkill",
    "sudo",
    "eval",
    "chmod",
    "chown",
    "pkill",
    "killall",
    "ssh",
    "scp",
];

/// Splits a shell command into lowercase argument tokens, respecting single/double quotes.
fn tokenize_shell_command(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in cmd.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(current.to_ascii_lowercase());
                    current.clear();
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current.to_ascii_lowercase());
    }
    tokens
}

fn normalize_program_name(token: &str) -> &str {
    let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".cmd"))
        .or_else(|| base.strip_suffix(".bat"))
        .or_else(|| base.strip_suffix(".ps1"))
        .unwrap_or(base)
}

fn has_shell_metacharacters(cmd: &str) -> bool {
    cmd.contains("&&")
        || cmd.contains("||")
        || cmd.contains(';')
        || cmd.contains('|')
        || cmd.contains('&')
        || cmd.contains('>')
        || cmd.contains('<')
        || cmd.contains('`')
        || cmd.contains("$(")
        || cmd.contains("${")
        || cmd.contains('$')
        || cmd.contains('(')
        || cmd.contains(')')
        || cmd.contains('[')
        || cmd.contains(']')
        || cmd.contains('{')
        || cmd.contains('}')
        || cmd.contains('%')
        || cmd.contains('@')
        || cmd.contains('~')
        || cmd.contains('\n')
        || cmd.contains('\r')
}

fn are_readonly_path_args_confined(tokens: &[String], start_idx: usize) -> bool {
    for tok in tokens.iter().skip(start_idx) {
        if tok.starts_with('-') {
            continue;
        }
        if tok.contains("..")
            || tok.starts_with('/')
            || tok.starts_with('\\')
            || tok.contains(':')
        {
            return false;
        }
    }
    true
}

fn segment_leading_programs(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    for seg in cmd.split([';', '|', '&', '\n', '\r']) {
        let seg_tokens = tokenize_shell_command(seg.trim());
        if let Some(first) = seg_tokens.first() {
            out.push(normalize_program_name(first).to_string());
        }
    }
    out
}

fn is_safe_readonly_git(tokens: &[String]) -> bool {
    let Some(sub) = tokens.get(1).map(String::as_str) else {
        return false;
    };
    if matches!(sub, "-c" | "--exec-path" | "--git-dir" | "--work-tree")
        || sub.starts_with("--git-dir=")
        || sub.starts_with("--work-tree=")
    {
        return false;
    }
    let has_dangerous_git_flag = tokens.iter().any(|t| {
        matches!(t.as_str(), "-c" | "--exec-path" | "--output" | "--git-dir" | "--work-tree")
            || t.starts_with("--output=")
            || t.starts_with("--git-dir=")
            || t.starts_with("--work-tree=")
    });
    if has_dangerous_git_flag {
        return false;
    }
    match sub {
        "status" | "diff" | "log" | "show" | "rev-parse" => true,
        "branch" => !tokens.iter().skip(2).any(|f| {
            matches!(
                f.as_str(),
                "-d" | "-m"
                    | "-c"
                    | "--delete"
                    | "--move"
                    | "--copy"
                    | "--set-upstream-to"
                    | "--unset-upstream"
                    | "--force"
                    | "-f"
            ) || f.starts_with("-d")
                || f.starts_with("-m")
                || f.starts_with("-c")
        }) && !tokens.iter().skip(2).any(|t| !t.starts_with('-')),
        _ => false,
    }
}

fn is_safe_readonly_command(cmd_raw: &str) -> bool {
    if has_shell_metacharacters(cmd_raw) {
        return false;
    }
    let tokens = tokenize_shell_command(cmd_raw.trim());
    let Some(first) = tokens.first() else {
        return false;
    };
    let prog = normalize_program_name(first);

    match prog {
        "dir" | "ls" | "get-childitem" | "gci" => are_readonly_path_args_confined(&tokens, 1),
        "pwd" => tokens.len() == 1,
        "git" => is_safe_readonly_git(&tokens),
        "python" | "python3" => {
            tokens.get(1).map(String::as_str) == Some("-m")
                && tokens.get(2).map(String::as_str) == Some("py_compile")
                && are_readonly_path_args_confined(&tokens, 3)
        }
        "pytest" => {
            tokens.iter().any(|t| t == "--collect-only")
                && are_readonly_path_args_confined(&tokens, 1)
        }
        _ => false,
    }
}

pub fn is_sensitive_secret_path(path: &str) -> bool {
    let trimmed = path
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .replace('\\', "/");
    if trimmed.is_empty() {
        return false;
    }
    let lower_path = trimmed.to_ascii_lowercase();
    let fname = Path::new(&lower_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(lower_path.as_str());

    if matches!(
        fname,
        ".env.example"
            | ".env.sample"
            | ".env.template"
            | ".env.dist"
            | ".env.schema"
            | "example.env"
            | "sample.env"
    ) || fname.contains(".example.")
        || fname.contains(".sample.")
        || fname.contains(".template.")
    {
        return false;
    }

    if fname == ".env" || fname.starts_with(".env.") {
        return true;
    }

    if matches!(
        fname,
        "id_rsa"
            | "id_ed25519"
            | "id_ecdsa"
            | "id_dsa"
            | ".npmrc"
            | ".netrc"
            | ".pypirc"
            | ".pgpass"
            | ".htpasswd"
            | ".dockercfg"
            | "master.key"
            | "credentials.json"
            | "credentials.yaml"
            | "credentials.yml"
            | "credentials.toml"
            | "credentials.ini"
            | "secrets.json"
            | "secrets.yaml"
            | "secrets.yml"
            | "secrets.toml"
            | "secrets.ini"
            | "state.json"
            | "mcp_config.json"
            | ".mcp.json"
            | ".git-credentials"
            | "auth.json"
            | "hosts.yml"
    ) {
        return true;
    }

    if (fname.starts_with("service-account") || fname.starts_with("service_account"))
        && fname.ends_with(".json")
    {
        return true;
    }

    matches!(
        Path::new(fname).extension().and_then(|e| e.to_str()),
        Some("key" | "pem" | "p12" | "pfx" | "jks" | "keystore" | "secret" | "kdbx")
    )
}

pub fn extract_tool_secret_path(tool_name: &str, args: &Value) -> Option<String> {
    if !matches!(
        tool_name,
        "read_file" | "edit_file" | "write_file" | "propose_edit" | "extract_pdf_text" | "search_files"
    ) {
        return None;
    }
    for key in [
        "path",
        "file_path",
        "filepath",
        "file",
        "filename",
        "target_file",
        "sub_path",
    ] {
        if let Some(raw) = args.get(key).and_then(|v| v.as_str()) {
            let t = raw.trim();
            if !t.is_empty() && is_sensitive_secret_path(t) {
                return Some(t.to_string());
            }
        }
    }
    None
}

pub fn classify_tool_risk_with_context(
    tool_name: &str,
    args: &Value,
    is_cloud_model: bool,
) -> (RiskTier, String) {
    if is_cloud_model {
        if let Some(secret_path) = extract_tool_secret_path(tool_name, args) {
            return (
                RiskTier::HighRiskDestructive,
                format!(
                    "Cloud Secret Guard: '{secret_path}' appears to contain secrets/credentials and would be sent to a cloud model"
                ),
            );
        }
    }
    classify_tool_risk(tool_name, args)
}

pub fn is_high_risk_config_write_path(path: &str) -> bool {
    let trimmed = path
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .replace('\\', "/")
        .to_ascii_lowercase();
    if trimmed.is_empty() {
        return false;
    }
    let norm = trimmed.trim_start_matches("./");
    let fname = Path::new(norm)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(norm);

    if norm.split('/').any(|seg| seg == ".git") || fname == ".gitmodules" {
        return true;
    }
    if norm.ends_with(".cargo/config")
        || norm.ends_with(".cargo/config.toml")
        || matches!(fname, "rust-toolchain" | "rust-toolchain.toml")
    {
        return true;
    }
    if norm.ends_with(".vscode/tasks.json")
        || norm.ends_with(".vscode/launch.json")
        || norm.ends_with(".vscode/settings.json")
    {
        return true;
    }
    if matches!(
        fname,
        "mcp_config.json"
            | ".mcp.json"
            | ".bashrc"
            | ".zshrc"
            | ".profile"
            | ".bash_profile"
            | "profile.ps1"
    ) {
        return true;
    }
    false
}

pub fn classify_tool_risk(tool_name: &str, args: &Value) -> (RiskTier, String) {
    match tool_name {
        "list_dir" | "read_file" | "search_files" | "extract_pdf_text" | "propose_edit"
        | "search_knowledge" | "delegate_research" | "delegate_vision" | "submit_plan"
        | "ask_user" | "web_search" | "fetch_url" | "switch_mode" | "write_artifact" => (
            RiskTier::SafeReadOnly,
            "Read-only / planning operation".into(),
        ),
        "write_file" | "edit_file" => {
            for key in [
                "path",
                "file_path",
                "filepath",
                "file",
                "filename",
                "target_file",
            ] {
                if let Some(p) = args.get(key).and_then(|v| v.as_str()) {
                    if is_high_risk_config_write_path(p) {
                        return (
                            RiskTier::HighRiskDestructive,
                            format!(
                                "Security Guard: modifying execution/hook config '{p}' always requires manual approval"
                            ),
                        );
                    }
                }
            }
            (
                RiskTier::WorkspaceMutation,
                "Modifies workspace file".into(),
            )
        }
        "run_local_software" => {
            let rt_id = args["runtime_id"].as_str().unwrap_or("runtime");
            (
                RiskTier::HighRiskDestructive,
                format!(
                    "Allowed Runtime '{rt_id}': mandatory user approval required before local execution"
                ),
            )
        }
        "run_command" | "verify_workspace" => {
            let cmd_raw = args["command"].as_str().unwrap_or("");
            let cmd = cmd_raw.trim().to_ascii_lowercase();

            if HIGH_RISK_SUBSTRINGS.iter().any(|p| cmd.contains(p))
                || segment_leading_programs(&cmd)
                    .iter()
                    .any(|prog| HIGH_RISK_PROGRAMS.contains(&prog.as_str()))
            {
                return (
                    RiskTier::HighRiskDestructive,
                    "High-risk or destructive shell command — always requires review".into(),
                );
            }

            if is_safe_readonly_command(cmd_raw) {
                return (
                    RiskTier::SafeReadOnly,
                    "Safe read-only or syntax check command".into(),
                );
            }

            (
                RiskTier::WorkspaceMutation,
                "Workspace shell execution".into(),
            )
        }
        _ => (
            RiskTier::WorkspaceMutation,
            "Workspace tool execution".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_risk_tiers_accurately() {
        let (safe_tier, _) = classify_tool_risk("run_command", &json!({ "command": "git status" }));
        assert_eq!(safe_tier, RiskTier::SafeReadOnly);

        let (mut_tier, _) = classify_tool_risk(
            "edit_file",
            &json!({ "path": "a.rs", "target": "x", "replacement": "y" }),
        );
        assert_eq!(mut_tier, RiskTier::WorkspaceMutation);

        let (high1, _) = classify_tool_risk("run_command", &json!({ "command": "rm -rf ./build" }));
        assert_eq!(high1, RiskTier::HighRiskDestructive);

        let (high2, _) = classify_tool_risk(
            "run_command",
            &json!({ "command": "Remove-Item -Recurse -Force ./temp" }),
        );
        assert_eq!(high2, RiskTier::HighRiskDestructive);
    }

    #[test]
    fn rejects_prefix_collisions_chaining_and_powershell_destructive_bypasses() {
        // Exact program allowlist must reject prefix collisions
        for cmd in ["dirty-script", "lsof -i :8080", "lsblk", "git branch -D feat", "git branch new-b"] {
            let (tier, _) = classify_tool_risk("run_command", &json!({ "command": cmd }));
            assert_eq!(tier, RiskTier::WorkspaceMutation, "cmd={cmd}");
        }

        // Metacharacters / chaining / pipelines / newlines must never be SafeReadOnly
        for cmd in [
            "git status | findstr Cargo",
            "ls & whoami",
            "ls\npwd",
            "echo hi > out.txt",
            "echo $(whoami)",
            "ls | Out-Null",
        ] {
            let (tier, _) = classify_tool_risk("run_command", &json!({ "command": cmd }));
            assert_ne!(tier, RiskTier::SafeReadOnly, "cmd={cmd}");
        }

        // High-risk bypass vectors must be HighRiskDestructive
        for cmd in [
            "[IO.Directory]::Delete('foo', $true)",
            "Invoke-Expression \"whoami\"",
            "iex $payload",
            "echo test | iex",
            "git push -f origin main",
            "del foo.txt",
        ] {
            let (tier, _) = classify_tool_risk("run_command", &json!({ "command": cmd }));
            assert_eq!(tier, RiskTier::HighRiskDestructive, "cmd={cmd}");
        }

        // Legitimate read-only commands still classify cleanly
        for cmd in [
            "git status",
            "git diff --stat",
            "git branch",
            "ls -la",
            "dir",
            "pwd",
            "python -m py_compile main.py",
        ] {
            let (tier, _) = classify_tool_risk("run_command", &json!({ "command": cmd }));
            assert_eq!(tier, RiskTier::SafeReadOnly, "cmd={cmd}");
        }

        // Build-script / proc-macro / PowerShell subexpression / off-workspace path escapes must never be SafeReadOnly
        for cmd in [
            "cargo check",
            "cargo test --workspace --no-run",
            "echo [System.Diagnostics.Process]::Start('calc')",
            "echo $env:PATH",
            "dir C:\\Windows",
            "ls ../secret",
        ] {
            let (tier, _) = classify_tool_risk("run_command", &json!({ "command": cmd }));
            assert_ne!(tier, RiskTier::SafeReadOnly, "cmd={cmd}");
        }

        // Extra bypass checks: git --output, encoded/nested shells
        let (git_out, _) = classify_tool_risk("run_command", &json!({ "command": "git diff --output=leak.txt" }));
        assert_eq!(git_out, RiskTier::WorkspaceMutation);

        for high_cmd in [
            "powershell -EncodedCommand SQBFAFgA",
            "cmd /c dir",
            "bash -c \"rm -rf /\"",
        ] {
            let (tier, _) = classify_tool_risk("run_command", &json!({ "command": high_cmd }));
            assert_eq!(tier, RiskTier::HighRiskDestructive, "high_cmd={high_cmd}");
        }
    }

    #[test]
    fn cloud_secret_guard_escalates_sensitive_files_only_for_cloud_models() {
        for secret in [
            ".env",
            ".env.local",
            ".env.production",
            "config/prod.pem",
            "certs/server.key",
            "secrets.json",
            "credentials.yaml",
            "keys/service-account-prod.json",
            ".ssh/id_ed25519",
        ] {
            assert!(is_sensitive_secret_path(secret), "expected sensitive: {secret}");
        }
        for safe in [
            ".env.example",
            ".env.sample",
            ".env.template",
            "src/main.rs",
            "README.md",
            "config/settings.example.json",
        ] {
            assert!(!is_sensitive_secret_path(safe), "expected safe: {safe}");
        }

        let read_env = json!({ "path": ".env" });
        let (local_tier, _) = classify_tool_risk_with_context("read_file", &read_env, false);
        assert_eq!(local_tier, RiskTier::SafeReadOnly);

        let (cloud_tier, reason) = classify_tool_risk_with_context("read_file", &read_env, true);
        assert_eq!(cloud_tier, RiskTier::HighRiskDestructive);
        assert!(reason.contains("Cloud Secret Guard"));

        let (cloud_example, _) =
            classify_tool_risk_with_context("read_file", &json!({ "path": ".env.example" }), true);
        assert_eq!(cloud_example, RiskTier::SafeReadOnly);
    }

    #[test]
    fn high_risk_config_write_guard_escalates_git_hooks_cargo_and_vscode_configs() {
        for sensitive_cfg in [
            ".git/hooks/pre-commit",
            ".git/config",
            ".cargo/config.toml",
            ".vscode/tasks.json",
            ".agents/mcp_config.json",
            ".mcp.json",
        ] {
            let (tier, _) = classify_tool_risk(
                "write_file",
                &json!({ "path": sensitive_cfg, "content": "x" }),
            );
            assert_eq!(
                tier,
                RiskTier::HighRiskDestructive,
                "expected HighRiskDestructive for {sensitive_cfg}"
            );
        }

        let (normal, _) = classify_tool_risk(
            "write_file",
            &json!({ "path": "src/main.rs", "content": "fn main() {}" }),
        );
        assert_eq!(normal, RiskTier::WorkspaceMutation);
    }
}
