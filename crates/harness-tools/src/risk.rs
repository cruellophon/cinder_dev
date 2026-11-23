use harness_store::RiskTier;
use serde_json::Value;

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
        || cmd.contains('\n')
        || cmd.contains('\r')
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
        "dir" | "ls" | "pwd" | "get-childitem" | "gci" | "echo" | "write-output" => true,
        "git" => is_safe_readonly_git(&tokens),
        "cargo" => match tokens.get(1).map(String::as_str) {
            Some("check") | Some("metadata") => true,
            Some("test") => tokens.iter().any(|t| t == "--no-run"),
            _ => false,
        },
        "python" | "python3" => {
            tokens.get(1).map(String::as_str) == Some("-m")
                && tokens.get(2).map(String::as_str) == Some("py_compile")
        }
        "pytest" => tokens.iter().any(|t| t == "--collect-only"),
        _ => false,
    }
}

pub fn classify_tool_risk(tool_name: &str, args: &Value) -> (RiskTier, String) {
    match tool_name {
        "list_dir" | "read_file" | "search_files" | "extract_pdf_text" | "propose_edit"
        | "search_knowledge" | "delegate_research" | "delegate_vision" | "submit_plan"
        | "ask_user" | "web_search" | "fetch_url" | "switch_mode" | "write_artifact" => (
            RiskTier::SafeReadOnly,
            "Read-only / planning operation".into(),
        ),
        "write_file" | "edit_file" => (
            RiskTier::WorkspaceMutation,
            "Modifies workspace file".into(),
        ),
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
            "cargo check",
            "cargo test --workspace --no-run",
            "python -m py_compile main.py",
        ] {
            let (tier, _) = classify_tool_risk("run_command", &json!({ "command": cmd }));
            assert_eq!(tier, RiskTier::SafeReadOnly, "cmd={cmd}");
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
}
