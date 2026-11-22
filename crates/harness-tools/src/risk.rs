use harness_store::RiskTier;
use serde_json::Value;

const HIGH_RISK_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "remove-item",
    "del /s",
    "rmdir /s",
    "format-volume",
    "git reset --hard",
    "git clean -f",
    "git push --force",
    "invoke-webrequest",
    "curl ",
    "wget ",
    "pip install",
    "npm install",
    "cargo install",
    "winget ",
    "shutdown",
    "reg ",
    "netsh",
];

const SAFE_COMMAND_PREFIXES: &[&str] = &[
    "git status",
    "git diff",
    "git log",
    "git branch",
    "cargo check",
    "cargo test --no-run",
    "python -m py_compile",
    "pytest --collect-only",
    "dir",
    "ls",
    "pwd",
    "get-childitem",
    "echo ",
];

pub fn classify_tool_risk(tool_name: &str, args: &Value) -> (RiskTier, String) {
    match tool_name {
        "list_dir" | "read_file" | "search_files" | "extract_pdf_text" | "propose_edit"
        | "search_knowledge" | "delegate_research" | "delegate_vision" | "submit_plan"
        | "ask_user" | "web_search" | "switch_mode" | "write_artifact" => (
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

            if HIGH_RISK_PATTERNS.iter().any(|p| cmd.contains(p)) {
                return (
                    RiskTier::HighRiskDestructive,
                    "High-risk or destructive shell command — always requires review".into(),
                );
            }

            let has_chaining = cmd.contains("&&")
                || cmd.contains("||")
                || cmd.contains(';')
                || cmd.contains('>')
                || cmd.contains('`')
                || cmd.contains("$(");

            if !has_chaining
                && SAFE_COMMAND_PREFIXES
                    .iter()
                    .any(|p| cmd == p.trim() || cmd.starts_with(p))
            {
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
}
