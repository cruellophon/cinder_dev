pub mod mcp;
pub mod native_dialogs;
pub mod packages;
pub mod rag;
pub mod risk;
pub mod sandbox;

pub use mcp::{
    call_mcp_tool, discover_enabled_mcp_tools, evict_mcp_server_session, mcp_tools_to_specs,
    namespaced_mcp_tool, probe_mcp_server, sanitize_mcp_slug, McpDiscoveredTool, McpProbeResult,
};
pub use native_dialogs::{
    open_url_safely, open_with_system_default, pick_files, pick_folder, reveal_in_file_manager,
};
pub use packages::{
    curated_dependency_allowlist, curated_plugin_catalog, export_antigravity_mcp_json,
    inspect_project_dependencies, install_curated_plugin, install_custom_project_dependency,
    install_project_dependency, is_dependency_allowlisted, list_plugin_catalog_with_status,
    probe_toolchains, sync_from_antigravity, uninstall_curated_plugin,
    uninstall_project_dependency, validate_custom_package_name, AntigravitySyncReport,
    CuratedDependency, CuratedPluginEntry, InstallJobManager, InstallJobRecord, PluginCatalogView,
    PluginInstallOutcome, ProjectDependenciesOverview, TempStageDir, ToolchainStatus,
};
pub use rag::{
    build_workspace_index, search_workspace_knowledge, IndexStats, KnowledgeSearchHit,
};
pub use risk::classify_tool_risk;
pub use sandbox::{
    clean_latex_pdf_text, create_workspace_dir, create_workspace_file, delete_workspace_entry,
    extract_pdf_file_text, list_workspace_tree, relative_to_workspace, rename_workspace_entry,
    resolve_workspace_path, strip_verbatim_prefix, WorkspaceFileNode,
};

use harness_net::{NetworkGate, PrivacyMode};
use harness_providers::ToolSpec;
use harness_store::AgentMode;
use regex::{Regex, RegexBuilder};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use similar::TextDiff;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub output: String,
    pub diff: Option<String>,
    pub file_path: Option<String>,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct ProposedChange {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub existed_before: bool,
    pub previous_content: Option<String>,
    pub new_content: String,
    pub diff: String,
}

fn switch_mode_spec() -> ToolSpec {
    ToolSpec {
        name: "switch_mode".into(),
        description: "Dynamically switch your work mode ONLY: 'chat' (direct answer), 'plan' (read-only exploration & plan artifact), 'agent' (write/edit files, shell commands & verify_workspace), or 'orchestrator' (dual-model planner + executor). Models CANNOT switch network privacy mode (Strict Local vs Hybrid) — only the user controls network privacy.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "target_mode": {
                    "type": "string",
                    "enum": ["chat", "plan", "agent", "orchestrator"],
                    "description": "Work mode to switch to ('chat', 'plan', 'agent', 'orchestrator'). Cannot be 'hybrid' or 'local'."
                },
                "reason": {
                    "type": "string",
                    "description": "Concise reason why switching work mode helps complete the task."
                }
            },
            "required": ["target_mode", "reason"]
        }),
    }
}

fn vision_subagent_spec() -> ToolSpec {
    ToolSpec {
        name: "delegate_vision".into(),
        description: "Delegate image, screenshot, OCR, UI, or diagram inspection to an isolated Vision Sub-Agent running a vision-capable model. Use this whenever analyzing images/screenshots attached in chat or image files inside the workspace.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Specific visual question or OCR/extraction instruction (e.g. 'Transcribe all visible code/errors and describe the UI issue')."
                },
                "image_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional workspace-relative image paths (.png/.jpg/.webp/.gif). Omit or pass empty to inspect image(s) attached in the current conversation."
                }
            },
            "required": ["prompt"]
        }),
    }
}

fn delegate_research_spec() -> ToolSpec {
    ToolSpec {
        name: "delegate_research".into(),
        description: "Delegate research, web synthesis, deep-dive investigation, or complex analysis to an isolated Research Sub-Agent. In Hybrid Mode, automatically delegates to a reachable cloud model (such as Gemini) with live web_search enabled; in Strict Local Mode, inspects workspace files and PDFs offline.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "Specific research question to answer." },
                "focus_paths": { "type": "array", "items": { "type": "string" }, "description": "Optional workspace files or directories to focus on." },
                "prefer_cloud": { "type": "boolean", "description": "In Hybrid Mode, prefer delegating to a cloud model such as Gemini (defaults to true for local models)." }
            },
            "required": ["task"]
        }),
    }
}

fn web_search_spec() -> ToolSpec {
    ToolSpec {
        name: "web_search".into(),
        description: "Search the live web and Wikipedia for verifiable articles and URLs (only available in Hybrid Mode; for direct URLs, prefer `fetch_url`).".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }),
    }
}

pub fn fetch_url_spec() -> ToolSpec {
    ToolSpec {
        name: "fetch_url".into(),
        description: "Fetch and extract readable text/Markdown from a direct public http:// or https:// webpage or GitHub URL (such as a release page, repository, or documentation link). Only available in Hybrid Mode.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Direct public http:// or https:// URL to read." }
            },
            "required": ["url"]
        }),
    }
}

pub fn tool_specs(mode: AgentMode, allow_web: bool) -> Vec<ToolSpec> {
    if mode == AgentMode::Compare {
        return Vec::new();
    }
    if mode == AgentMode::Chat {
        let mut chat_specs = vec![switch_mode_spec(), vision_subagent_spec()];
        if allow_web {
            chat_specs.push(delegate_research_spec());
            chat_specs.push(web_search_spec());
            chat_specs.push(fetch_url_spec());
        }
        return chat_specs;
    }

    let mut specs = vec![
        switch_mode_spec(),
        vision_subagent_spec(),
        ToolSpec {
            name: "list_dir".into(),
            description: "List files and folders inside a workspace directory.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path inside workspace ('.' for root)." },
                    "max_depth": { "type": "integer", "description": "Max folder depth (default 2)." }
                }
            }),
        },
        ToolSpec {
            name: "read_file".into(),
            description: "Read numbered lines from a file inside the workspace.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path to the file." },
                    "start_line": { "type": "integer", "description": "1-based start line (optional)." },
                    "end_line": { "type": "integer", "description": "1-based end line inclusive (optional)." }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "search_files".into(),
            description: "Search text/regex across workspace files with file and line numbers.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex or text pattern to find." },
                    "sub_path": { "type": "string", "description": "Optional subfolder to search in." }
                },
                "required": ["pattern"]
            }),
        },
        ToolSpec {
            name: "extract_pdf_text".into(),
            description: "Extract readable text locally (100% offline) from a PDF inside the workspace.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path to the .pdf file." },
                    "max_pages": { "type": "integer", "description": "Max pages to extract (default 15)." }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "search_knowledge".into(),
            description: "100% offline semantic + BM25 search across indexed workspace docs, code, and PDFs.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language question or keyword query." },
                    "top_k": { "type": "integer", "description": "Number of chunks to return (default 5)." }
                },
                "required": ["query"]
            }),
        },
        delegate_research_spec(),
        ToolSpec {
            name: "ask_user".into(),
            description: "Pause and ask the user a clarifying question with clickable options before proceeding.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "Clear question for the user." },
                    "options": { "type": "array", "items": { "type": "string" }, "description": "2 to 5 concise choices." },
                    "allow_multiple": { "type": "boolean", "description": "Allow selecting multiple options." }
                },
                "required": ["question", "options"]
            }),
        },
        ToolSpec {
            name: "submit_plan".into(),
            description: "Submit a structured implementation plan artifact for user review and one-click execution.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "summary": { "type": "string" },
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step_number": { "type": "integer" },
                                "description": { "type": "string" },
                                "files_touched": { "type": "array", "items": { "type": "string" } },
                                "verification": { "type": "string" }
                            },
                            "required": ["step_number", "description"]
                        }
                    }
                },
                "required": ["title", "summary", "steps"]
            }),
        },
        ToolSpec {
            name: "write_artifact".into(),
            description: "Create or update a persistent session artifact displayed in the right-hand Artifacts panel (task checklist, plan, walkthrough, report, or self-contained interactive html_widget).".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short artifact title (e.g. 'Task Checklist', 'Walkthrough', 'Architecture Diagram')." },
                    "kind": {
                        "type": "string",
                        "enum": ["task_list", "plan", "walkthrough", "report", "html_widget"],
                        "description": "Artifact category."
                    },
                    "content": { "type": "string", "description": "Markdown or self-contained HTML content." }
                },
                "required": ["title", "kind", "content"]
            }),
        },
    ];

    if mode == AgentMode::Plan {
        specs.push(ToolSpec {
            name: "propose_edit".into(),
            description: "Compute a unified diff preview for a proposed edit without modifying disk. Pass workspace-relative path, keep target/replacement concise (only the changed lines), and NEVER include read_file line-number prefixes (like '14: ').".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "target": { "type": "string", "description": "Exact text block to replace without line-number prefixes (leave empty to create/overwrite)." },
                    "replacement": { "type": "string" }
                },
                "required": ["path", "replacement"]
            }),
        });
    } else if matches!(
        mode,
        AgentMode::Agent | AgentMode::Auto | AgentMode::Orchestrator
    ) {
        specs.push(ToolSpec {
            name: "write_file".into(),
            description: "Create or overwrite a file inside the workspace (creates checkpoint & unified diff).".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        });
        specs.push(ToolSpec {
            name: "edit_file".into(),
            description: "Replace a concise exact target text block inside a workspace file (creates checkpoint & unified diff). Pass workspace-relative path, keep target/replacement focused on only the lines being changed (use write_file for full-file rewrites), and NEVER include read_file line-number prefixes (like '14: ').".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "target": { "type": "string" },
                    "replacement": { "type": "string" }
                },
                "required": ["path", "target", "replacement"]
            }),
        });
        specs.push(ToolSpec {
            name: "run_command".into(),
            description: if cfg!(windows) {
                "Run a shell command inside the active workspace directory (Windows already runs inside PowerShell — pass commands directly without wrapping in 'powershell -Command').".into()
            } else {
                "Run a shell command inside the active workspace directory (runs via `sh -c`).".into()
            },
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
        });
        specs.push(ToolSpec {
            name: "verify_workspace".into(),
            description: if cfg!(windows) {
                "Run a verification command (e.g. cargo check, python -m py_compile, pytest) after edits inside the active workspace (Windows already runs inside PowerShell).".into()
            } else {
                "Run a verification command (e.g. cargo check, python3 -m py_compile, pytest) after edits inside the active workspace (runs via `sh -c`).".into()
            },
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
        });
    }

    if allow_web {
        specs.push(web_search_spec());
        specs.push(fetch_url_spec());
    }

    specs
}

pub fn filter_tools_by_modules(specs: Vec<ToolSpec>, enabled_modules: &[String]) -> Vec<ToolSpec> {
    let has = |m: &str| enabled_modules.iter().any(|x| x == m);
    specs
        .into_iter()
        .filter(|t| match t.name.as_str() {
            "list_dir" | "read_file" | "search_files" | "extract_pdf_text" | "propose_edit" => {
                has("fs_read")
            }
            "write_file" | "edit_file" => has("fs_write"),
            "run_command" | "verify_workspace" => has("shell"),
            "search_knowledge" => has("rag"),
            "delegate_research" | "delegate_vision" => has("subagents"),
            "web_search" | "fetch_url" => has("web"),
            _ => true,
        })
        .collect()
}

const PATH_ARG_KEYS: &[&str] = &[
    "path",
    "file_path",
    "filepath",
    "file",
    "filename",
    "filePath",
    "target_file",
    "target_path",
];

fn arg_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for &k in keys {
        if let Some(s) = args.get(k).and_then(|v| v.as_str()) {
            return Some(s);
        }
    }
    None
}

fn parse_numbered_code_line(line: &str) -> Option<&str> {
    let line_no_cr = line.strip_suffix('\r').unwrap_or(line);
    let trimmed_start = line_no_cr.trim_start_matches(' ');
    let digits_len = trimmed_start
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits_len == 0 || digits_len > 6 {
        return None;
    }
    let num: usize = trimmed_start[..digits_len].parse().ok()?;
    if num == 0 {
        return None;
    }
    let after_digits = &trimmed_start[digits_len..];
    let after_colon = after_digits.strip_prefix(':')?;
    if after_colon.is_empty() {
        Some("")
    } else if let Some(rest) = after_colon.strip_prefix(' ') {
        Some(rest)
    } else {
        None
    }
}

/// Strip `read_file`-style line-number prefixes (`"  14: code"`) when a model accidentally
/// copies numbered output into `edit_file` `target` or `replacement`.
fn strip_read_file_line_numbers(text: &str) -> Option<String> {
    let norm = text.replace("\r\n", "\n");
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut non_empty = 0usize;
    let mut matching = 0usize;
    let mut matching_with_space = 0usize;
    let mut out_lines: Vec<&str> = Vec::with_capacity(lines.len());

    for &line in &lines {
        if !line.trim().is_empty() {
            non_empty += 1;
            if let Some(stripped) = parse_numbered_code_line(line) {
                matching += 1;
                if line.contains(": ") {
                    matching_with_space += 1;
                }
                out_lines.push(stripped);
            } else {
                out_lines.push(line);
            }
        } else {
            out_lines.push(line);
        }
    }

    let triggered = (non_empty == 1 && matching == 1 && matching_with_space == 1)
        || (non_empty >= 2 && matching * 4 >= non_empty * 3);
    if triggered {
        Some(out_lines.join("\n"))
    } else {
        None
    }
}

fn replace_once_preserving_eol(old_text: &str, needle: &str, replacement: &str) -> Option<String> {
    if old_text.contains(needle) {
        return Some(old_text.replacen(needle, replacement, 1));
    }
    let norm_old = old_text.replace("\r\n", "\n");
    let norm_needle = needle.replace("\r\n", "\n");
    if norm_old.contains(&norm_needle) {
        let norm_repl = replacement.replace("\r\n", "\n");
        let replaced = norm_old.replacen(&norm_needle, &norm_repl, 1);
        return Some(if old_text.contains("\r\n") {
            replaced.replace('\n', "\r\n")
        } else {
            replaced
        });
    }
    None
}

pub fn prepare_file_change(
    workspace_root: &Path,
    tool_name: &str,
    args: &Value,
) -> anyhow::Result<ProposedChange> {
    let rel_path = match arg_str(args, PATH_ARG_KEYS) {
        Some(p) => p,
        None => {
            if args.get("raw").is_some() {
                anyhow::bail!(
                    "Tool call arguments JSON for '{tool_name}' was truncated or malformed before 'path' was completed. Keep target/replacement concise (only the lines being changed, without line-number prefixes) or use write_file for full-file rewrites."
                );
            }
            anyhow::bail!("Missing 'path' argument");
        }
    };
    let mut abs_path = resolve_workspace_path(workspace_root, rel_path)?;
    if tool_name != "write_file" && !abs_path.is_file() {
        if abs_path.is_dir() {
            anyhow::bail!("'{rel_path}' is a directory, not a file.");
        }
        if let Some((found_abs, _)) = find_workspace_file_fallback(workspace_root, rel_path)? {
            abs_path = found_abs;
        }
    }

    let canonical_rel = relative_to_workspace(workspace_root, &abs_path)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| rel_path.trim().replace('\\', "/"));

    let existed_before = abs_path.is_file();
    let previous_content = if existed_before {
        Some(
            std::fs::read(&abs_path)
                .map(|b| decode_text_resilient(&b).unwrap_or_default())
                .unwrap_or_default(),
        )
    } else {
        None
    };
    let old_text = previous_content.as_deref().unwrap_or("");

    let new_content = if tool_name == "write_file" {
        arg_str(args, &["content", "text"])
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?
            .to_string()
    } else {
        let target = arg_str(args, &["target", "old_text", "old_string"]).unwrap_or("");
        let replacement = arg_str(args, &["replacement", "new_text", "new_string"])
            .ok_or_else(|| anyhow::anyhow!("Missing 'replacement' argument"))?;
        if target.is_empty() {
            replacement.to_string()
        } else if !existed_before {
            anyhow::bail!(
                "File '{canonical_rel}' does not exist in the workspace (try list_dir or search_files to locate the exact relative path)."
            );
        } else if let Some(updated) = replace_once_preserving_eol(old_text, target, replacement) {
            updated
        } else if let Some(stripped_target) = strip_read_file_line_numbers(target) {
            let effective_repl = strip_read_file_line_numbers(replacement)
                .unwrap_or_else(|| replacement.to_string());
            if let Some(updated) =
                replace_once_preserving_eol(old_text, &stripped_target, &effective_repl)
            {
                updated
            } else {
                anyhow::bail!(
                    "Target text was not found in '{canonical_rel}'. Re-read the exact lines via read_file and omit line-number prefixes (like '14: ') in target/replacement."
                );
            }
        } else {
            anyhow::bail!(
                "Target text was not found in '{canonical_rel}'. Re-read the exact lines via read_file and omit line-number prefixes (like '14: ') in target/replacement."
            );
        }
    };

    let diff = TextDiff::from_lines(old_text, &new_content)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{canonical_rel}"), &format!("b/{canonical_rel}"))
        .to_string();

    Ok(ProposedChange {
        rel_path: canonical_rel,
        abs_path,
        existed_before,
        previous_content,
        new_content,
        diff,
    })
}

pub fn apply_file_change(change: &ProposedChange) -> anyhow::Result<ToolExecutionResult> {
    if let Some(parent) = change.abs_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&change.abs_path, &change.new_content)?;
    Ok(ToolExecutionResult {
        output: format!(
            "Updated '{}' ({} bytes).",
            change.rel_path,
            change.new_content.len()
        ),
        diff: Some(change.diff.clone()),
        file_path: Some(change.rel_path.clone()),
        is_error: false,
    })
}

pub async fn execute_tool(
    workspace_root: &Path,
    gate: &NetworkGate,
    brave_key: &str,
    name: &str,
    args: &Value,
) -> ToolExecutionResult {
    let res = match name {
        "list_dir" => run_list_dir(workspace_root, args),
        "read_file" => run_read_file(workspace_root, args),
        "search_files" => run_search_files(workspace_root, args),
        "extract_pdf_text" => run_extract_pdf_text(workspace_root, args),
        "search_knowledge" => run_search_knowledge(workspace_root, gate, args).await,
        "propose_edit" => match prepare_file_change(workspace_root, "edit_file", args) {
            Ok(c) => Ok(ToolExecutionResult {
                output: format!(
                    "Diff preview generated for '{}' (not written to disk).",
                    c.rel_path
                ),
                diff: Some(c.diff),
                file_path: Some(c.rel_path),
                is_error: false,
            }),
            Err(e) => Err(e),
        },
        "write_file" | "edit_file" => match prepare_file_change(workspace_root, name, args) {
            Ok(change) => apply_file_change(&change),
            Err(e) => Err(e),
        },
        "run_command" | "verify_workspace" => run_shell_command(workspace_root, args).await,
        "web_search" => run_web_search(gate, brave_key, args).await,
        "fetch_url" => run_fetch_url(gate, args).await,
        other => Err(anyhow::anyhow!("Unknown tool '{other}'")),
    };

    match res {
        Ok(ok) => ok,
        Err(e) => ToolExecutionResult {
            output: format!("Tool error ({name}): {e}"),
            diff: None,
            file_path: None,
            is_error: true,
        },
    }
}

async fn run_search_knowledge(
    workspace_root: &Path,
    gate: &NetworkGate,
    args: &Value,
) -> anyhow::Result<ToolExecutionResult> {
    let query = args["query"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'query'"))?;
    let top_k = args["top_k"].as_u64().unwrap_or(5).clamp(1, 12) as usize;
    let hits = search_workspace_knowledge(workspace_root, gate, query, top_k).await?;
    if hits.is_empty() {
        return Ok(ToolExecutionResult {
            output: "No relevant knowledge chunks found.".into(),
            diff: None,
            file_path: None,
            is_error: false,
        });
    }
    let formatted = hits
        .into_iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "[{}] {} ({}) [score: {}]\n{}",
                i + 1,
                h.rel_path,
                h.location,
                h.score,
                h.snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(ToolExecutionResult {
        output: formatted,
        diff: None,
        file_path: None,
        is_error: false,
    })
}

fn run_list_dir(workspace_root: &Path, args: &Value) -> anyhow::Result<ToolExecutionResult> {
    let rel = arg_str(args, &["path", "dir", "directory", "folder"]).unwrap_or(".");
    let max_depth = args["max_depth"].as_u64().unwrap_or(3).clamp(1, 5) as usize;
    let initial = resolve_workspace_path(workspace_root, rel)?;
    let (target, resolved_note) = if initial.is_dir() {
        (initial, None)
    } else if initial.is_file() {
        anyhow::bail!("'{rel}' is a file; call read_file with path '{rel}' instead.");
    } else if let Some((found_dir, mrel)) = find_workspace_dir_fallback(workspace_root, rel)? {
        (found_dir, Some(format!("[Resolved '{rel}' -> '{mrel}']")))
    } else {
        anyhow::bail!("Directory '{rel}' does not exist in the workspace.");
    };

    let mut lines = Vec::new();
    let walker = WalkDir::new(&target)
        .max_depth(max_depth)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !matches!(
                n.as_ref(),
                ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
            )
        });

    for entry in walker.flatten() {
        if entry.path() == target {
            continue;
        }
        let Some(sub) = relative_to_workspace(workspace_root, entry.path()) else {
            continue;
        };
        if sub.is_empty() {
            continue;
        }
        let suffix = if entry.file_type().is_dir() { "/" } else { "" };
        lines.push(format!("{sub}{suffix}"));
        if lines.len() >= 200 {
            lines.push("... (truncated at 200 entries)".into());
            break;
        }
    }

    let mut output = if lines.is_empty() {
        "(empty directory)".into()
    } else {
        lines.join("\n")
    };
    if let Some(note) = resolved_note {
        output = format!("{note}\n{output}");
    }

    Ok(ToolExecutionResult {
        output,
        diff: None,
        file_path: None,
        is_error: false,
    })
}

fn cp1252_byte_to_char(b: u8) -> char {
    match b {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        _ => b as char,
    }
}

/// Decode file bytes tolerating UTF-8/UTF-16 BOMs and Windows-1252 / Latin-1 fallback.
fn decode_text_resilient(bytes: &[u8]) -> anyhow::Result<String> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Ok(String::from_utf16_lossy(&units)
            .trim_start_matches('\u{FEFF}')
            .to_string());
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return Ok(String::from_utf16_lossy(&units)
            .trim_start_matches('\u{FEFF}')
            .to_string());
    }
    let slice = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(slice) {
        return Ok(s.trim_start_matches('\u{FEFF}').to_string());
    }
    if slice.iter().take(512).filter(|&&b| b == 0).count() > 2 {
        anyhow::bail!("File appears to be binary (for PDFs call extract_pdf_text).");
    }
    Ok(slice.iter().map(|&b| cp1252_byte_to_char(b)).collect())
}

fn normalize_fallback_query(clean_root: &Path, query: &str) -> String {
    let mut s = query.trim().to_string();
    for (open, close) in [('"', '"'), ('\'', '\''), ('`', '`')] {
        if s.len() >= 2 && s.starts_with(open) && s.ends_with(close) {
            s = s[1..s.len() - 1].trim().to_string();
        }
    }
    let norm = s
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string();
    if let Some(ws_name) = clean_root.file_name().and_then(|n| n.to_str()) {
        let first = norm.split('/').next().unwrap_or("");
        let matches_ws = if cfg!(windows) {
            first.eq_ignore_ascii_case(ws_name)
        } else {
            first == ws_name
        };
        if matches_ws && norm.len() > first.len() {
            return norm[first.len()..].trim_start_matches('/').to_string();
        }
    }
    norm
}

fn find_workspace_entry_fallback(
    workspace_root: &Path,
    query: &str,
    want_dir: bool,
) -> anyhow::Result<Option<(PathBuf, String)>> {
    let clean_root = strip_verbatim_prefix(
        std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf()),
    );
    let norm_q = normalize_fallback_query(&clean_root, query);
    if norm_q.is_empty() || norm_q.contains("..") {
        return Ok(None);
    }
    let q_lower = norm_q.to_ascii_lowercase();
    let q_name = Path::new(&norm_q)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let walker = WalkDir::new(&clean_root)
        .max_depth(6)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !matches!(
                n.as_ref(),
                ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
            )
        });

    let mut suffix_matches: Vec<(PathBuf, String)> = Vec::new();
    let mut name_matches: Vec<(PathBuf, String)> = Vec::new();
    let suffix_needle = format!("/{q_lower}");

    for entry in walker.flatten() {
        let is_match_kind = if want_dir {
            entry.file_type().is_dir() && entry.path() != clean_root
        } else {
            entry.file_type().is_file()
        };
        if !is_match_kind {
            continue;
        }
        let Some(rel) = relative_to_workspace(&clean_root, entry.path()) else {
            continue;
        };
        let rel_lower = rel.to_ascii_lowercase();
        if rel_lower == q_lower || rel_lower.ends_with(&suffix_needle) {
            suffix_matches.push((entry.path().to_path_buf(), rel));
        } else if !q_name.is_empty()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&q_name)
        {
            name_matches.push((entry.path().to_path_buf(), rel));
        }
    }

    let chosen = if !suffix_matches.is_empty() {
        suffix_matches
    } else {
        name_matches
    };
    let kind_noun = if want_dir { "directories" } else { "files" };
    match chosen.len() {
        0 => Ok(None),
        1 => Ok(chosen.into_iter().next()),
        _ => {
            let listing: Vec<String> = chosen
                .into_iter()
                .take(10)
                .map(|(_, r)| format!("- {r}"))
                .collect();
            anyhow::bail!(
                "Path '{query}' matched multiple {kind_noun} in the workspace:\n{}\nPass the exact relative path.",
                listing.join("\n")
            )
        }
    }
}

fn find_workspace_file_fallback(
    workspace_root: &Path,
    query: &str,
) -> anyhow::Result<Option<(PathBuf, String)>> {
    find_workspace_entry_fallback(workspace_root, query, false)
}

fn find_workspace_dir_fallback(
    workspace_root: &Path,
    query: &str,
) -> anyhow::Result<Option<(PathBuf, String)>> {
    find_workspace_entry_fallback(workspace_root, query, true)
}

fn compile_search_regex(pattern: &str) -> Regex {
    let trimmed = pattern.trim();
    let is_glob = (trimmed.contains('*') || trimmed.contains('?'))
        && !trimmed.contains(".*")
        && !trimmed.contains(".+")
        && !trimmed.contains('\\')
        && !trimmed.contains('(')
        && !trimmed.contains('[')
        && !trimmed.contains('{')
        && !trimmed.contains('|')
        && !trimmed.contains('^')
        && !trimmed.contains('$');

    if is_glob {
        let stripped = trimmed.strip_prefix("**/").unwrap_or(trimmed);
        let mut expr = String::new();
        for ch in stripped.chars() {
            match ch {
                '*' => expr.push_str(".*"),
                '?' => expr.push('.'),
                _ => expr.push_str(&regex::escape(&ch.to_string())),
            }
        }
        if !stripped.ends_with('*') && !stripped.ends_with('?') && stripped.contains('.') {
            expr.push('$');
        }
        if let Ok(re) = RegexBuilder::new(&expr).case_insensitive(true).build() {
            return re;
        }
    }

    RegexBuilder::new(trimmed)
        .case_insensitive(true)
        .build()
        .unwrap_or_else(|_| {
            RegexBuilder::new(&regex::escape(trimmed))
                .case_insensitive(true)
                .build()
                .unwrap()
        })
}

fn run_read_file(workspace_root: &Path, args: &Value) -> anyhow::Result<ToolExecutionResult> {
    let rel = arg_str(args, PATH_ARG_KEYS).ok_or_else(|| anyhow::anyhow!("Missing 'path'"))?;
    let abs = resolve_workspace_path(workspace_root, rel)?;

    let (target_path, resolved_rel, used_fallback) = if abs.is_file() {
        let r = relative_to_workspace(workspace_root, &abs).unwrap_or_else(|| rel.to_string());
        (abs, r, false)
    } else if let Some((found, mrel)) = find_workspace_file_fallback(workspace_root, rel)? {
        (found, mrel, true)
    } else if abs.is_dir() {
        anyhow::bail!("'{rel}' is a directory; call list_dir with path '{rel}' instead.");
    } else {
        anyhow::bail!(
            "Could not find '{rel}' in the workspace (try list_dir or search_files with '*.md')."
        );
    };

    let bytes = std::fs::read(&target_path)
        .map_err(|e| anyhow::anyhow!("Could not read '{resolved_rel}': {e}"))?;
    let content = decode_text_resilient(&bytes)
        .map_err(|e| anyhow::anyhow!("Could not decode '{resolved_rel}' as text: {e}"))?;
    let all_lines: Vec<&str> = content.lines().collect();
    let total = all_lines.len();
    let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize;
    let end = args["end_line"]
        .as_u64()
        .unwrap_or((start + 249) as u64)
        .max(start as u64) as usize;
    let clamped_end = end.min(total);

    let mut out = Vec::new();
    if used_fallback {
        out.push(format!("[Resolved '{rel}' -> '{resolved_rel}']"));
    }
    if start <= total && !all_lines.is_empty() {
        for idx in (start - 1)..clamped_end {
            out.push(format!("{:>4}: {}", idx + 1, all_lines[idx]));
        }
    } else {
        out.push("(empty file)".into());
    }
    if clamped_end < total {
        out.push(format!("... ({} more lines)", total - clamped_end));
    }

    Ok(ToolExecutionResult {
        output: out.join("\n"),
        diff: None,
        file_path: Some(resolved_rel),
        is_error: false,
    })
}

fn run_search_files(workspace_root: &Path, args: &Value) -> anyhow::Result<ToolExecutionResult> {
    let pattern = arg_str(args, &["pattern", "query", "regex", "text"])
        .ok_or_else(|| anyhow::anyhow!("Missing 'pattern'"))?;
    let sub = arg_str(args, &["sub_path", "path", "dir", "directory", "folder"]).unwrap_or(".");
    let mut root = resolve_workspace_path(workspace_root, sub)?;
    if !root.exists() {
        if let Some((found_dir, _)) = find_workspace_dir_fallback(workspace_root, sub)? {
            root = found_dir;
        } else if let Some((found_file, _)) = find_workspace_file_fallback(workspace_root, sub)? {
            root = found_file;
        } else {
            anyhow::bail!("Search path '{sub}' does not exist in the workspace.");
        }
    }
    let re = compile_search_regex(pattern);

    let mut hits = Vec::new();
    let walker = WalkDir::new(&root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !matches!(
                n.as_ref(),
                ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
            )
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = relative_to_workspace(workspace_root, entry.path()).unwrap_or_else(|| {
            strip_verbatim_prefix(entry.path().to_path_buf())
                .to_string_lossy()
                .replace('\\', "/")
        });

        if re.is_match(&rel) {
            hits.push(format!("[file] {rel}"));
            if hits.len() >= 100 {
                hits.push("... (truncated at 100 matches)".into());
                return Ok(ToolExecutionResult {
                    output: hits.join("\n"),
                    diff: None,
                    file_path: None,
                    is_error: false,
                });
            }
        }

        if entry.metadata().map(|m| m.len()).unwrap_or(0) > 2 * 1024 * 1024 {
            continue;
        }
        if entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(text) = decode_text_resilient(&bytes) else {
            continue;
        };

        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                hits.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                if hits.len() >= 100 {
                    hits.push("... (truncated at 100 matches)".into());
                    return Ok(ToolExecutionResult {
                        output: hits.join("\n"),
                        diff: None,
                        file_path: None,
                        is_error: false,
                    });
                }
            }
        }
    }

    Ok(ToolExecutionResult {
        output: if hits.is_empty() {
            "No matches found.".into()
        } else {
            hits.join("\n")
        },
        diff: None,
        file_path: None,
        is_error: false,
    })
}

fn run_extract_pdf_text(workspace_root: &Path, args: &Value) -> anyhow::Result<ToolExecutionResult> {
    let rel = arg_str(args, PATH_ARG_KEYS).ok_or_else(|| anyhow::anyhow!("Missing 'path'"))?;
    let max_pages = args["max_pages"].as_u64().unwrap_or(15).clamp(1, 50) as usize;
    let initial = resolve_workspace_path(workspace_root, rel)?;
    let (abs, resolved_rel) = if initial.is_file() {
        let r = relative_to_workspace(workspace_root, &initial).unwrap_or_else(|| rel.to_string());
        (initial, r)
    } else if let Some((found, mrel)) = find_workspace_file_fallback(workspace_root, rel)? {
        (found, mrel)
    } else if initial.is_dir() {
        anyhow::bail!("'{rel}' is a directory, not a PDF file.");
    } else {
        anyhow::bail!("Could not find PDF '{rel}' in the workspace.");
    };
    let text = extract_pdf_file_text(&abs, max_pages)?;
    Ok(ToolExecutionResult {
        output: text,
        diff: None,
        file_path: Some(resolved_rel),
        is_error: false,
    })
}

async fn run_shell_command(
    workspace_root: &Path,
    args: &Value,
) -> anyhow::Result<ToolExecutionResult> {
    if workspace_root.as_os_str().is_empty() || !workspace_root.is_dir() {
        anyhow::bail!(
            "No valid project workspace directory is attached; open or select a project folder in the sidebar before running shell commands."
        );
    }
    let cwd = strip_verbatim_prefix(
        std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf()),
    );
    let command = arg_str(args, &["command", "cmd"])
        .ok_or_else(|| anyhow::anyhow!("Missing 'command'"))?;

    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoProfile", "-Command", command]);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(&cwd);

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output()).await??;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap_or(-1);

    let mut combined = format!("Exit code: {code}\n");
    if !stdout.trim().is_empty() {
        combined.push_str("STDOUT:\n");
        combined.push_str(&stdout);
        combined.push('\n');
    }
    if !stderr.trim().is_empty() {
        combined.push_str("STDERR:\n");
        combined.push_str(&stderr);
        combined.push('\n');
    }
    if combined.len() > 16_000 {
        let mut end = 16_000;
        while !combined.is_char_boundary(end) {
            end -= 1;
        }
        combined.truncate(end);
        combined.push_str("\n... [output truncated]");
    }

    Ok(ToolExecutionResult {
        output: combined,
        diff: None,
        file_path: None,
        is_error: !output.status.success(),
    })
}

fn decode_basic_html_entities(s: &str) -> String {
    let tag_re = Regex::new(r"<[^>]+>").expect("valid tag regex");
    let no_tags = tag_re.replace_all(s, "");
    let replaced = no_tags
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unwrap_ddg_href(raw_href: &str) -> Option<String> {
    let clean = raw_href.replace("&amp;", "&").trim().to_string();
    let full = if clean.starts_with("//") {
        format!("https:{clean}")
    } else if clean.starts_with('/') {
        format!("https://duckduckgo.com{clean}")
    } else {
        clean
    };
    let parsed = url::Url::parse(&full).ok()?;
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if host.contains("duckduckgo.com") {
        for (k, v) in parsed.query_pairs() {
            if k == "uddg" && (v.starts_with("http://") || v.starts_with("https://")) {
                return Some(v.into_owned());
            }
        }
        return None;
    }
    if matches!(parsed.scheme(), "http" | "https") {
        Some(full)
    } else {
        None
    }
}

fn parse_ddg_html_results(html: &str) -> Vec<(String, String, String)> {
    let link_re = RegexBuilder::new(
        r#"<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#,
    )
    .case_insensitive(true)
    .dot_matches_new_line(true)
    .build()
    .expect("valid ddg link regex");

    let snip_re = RegexBuilder::new(
        r#"<(?:a|div)[^>]*class="[^"]*result__snippet[^"]*"[^>]*>(.*?)</(?:a|div)>"#,
    )
    .case_insensitive(true)
    .dot_matches_new_line(true)
    .build()
    .expect("valid ddg snippet regex");

    let snippets: Vec<String> = snip_re
        .captures_iter(html)
        .map(|c| decode_basic_html_entities(&c[1]))
        .collect();

    let mut results = Vec::new();
    for (idx, cap) in link_re.captures_iter(html).enumerate() {
        let Some(url) = unwrap_ddg_href(&cap[1]) else {
            continue;
        };
        let title = decode_basic_html_entities(&cap[2]);
        if title.is_empty() {
            continue;
        }
        let snippet = snippets.get(idx).cloned().unwrap_or_default();
        results.push((title, url, snippet));
        if results.len() >= 6 {
            break;
        }
    }
    results
}

async fn search_wikipedia_api(
    gate: &NetworkGate,
    query: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let filtered_words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| !w.eq_ignore_ascii_case("wikipedia"))
        .collect();
    let effective_q = if filtered_words.is_empty() {
        query.trim().to_string()
    } else {
        filtered_words.join(" ")
    };
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("action", "query")
        .append_pair("list", "search")
        .append_pair("srsearch", &effective_q)
        .append_pair("utf8", "1")
        .append_pair("format", "json")
        .append_pair("srlimit", "6")
        .finish();
    let api_url = format!("https://en.wikipedia.org/w/api.php?{encoded}");
    let resp = gate
        .request(Method::GET, &api_url, "web_search_wikipedia")?
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let val: Value = resp.json().await.unwrap_or(Value::Null);
    let mut out = Vec::new();
    if let Some(arr) = val["query"]["search"].as_array() {
        for item in arr.iter().take(6) {
            let title = item["title"].as_str().unwrap_or("").trim();
            if title.is_empty() {
                continue;
            }
            let snippet = decode_basic_html_entities(item["snippet"].as_str().unwrap_or(""));
            if let Ok(mut wiki_url) = url::Url::parse("https://en.wikipedia.org") {
                if let Ok(mut segs) = wiki_url.path_segments_mut() {
                    segs.push("wiki").push(&title.replace(' ', "_"));
                }
                out.push((
                    format!("{title} - Wikipedia"),
                    wiki_url.to_string(),
                    snippet,
                ));
            }
        }
    }
    Ok(out)
}

async fn search_duckduckgo_html(
    gate: &NetworkGate,
    query: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", query)
        .finish();
    let url = format!("https://html.duckduckgo.com/html/?{encoded}");
    let resp = gate
        .request(Method::GET, &url, "web_search_ddg_html")?
        .header("Accept", "text/html")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let html = resp.text().await.unwrap_or_default();
    Ok(parse_ddg_html_results(&html))
}

async fn search_duckduckgo_instant(
    gate: &NetworkGate,
    query: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", query)
        .append_pair("format", "json")
        .append_pair("no_html", "1")
        .finish();
    let url = format!("https://api.duckduckgo.com/?{encoded}");
    let resp = gate
        .request(Method::GET, &url, "web_search_ddg")?
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let text = resp.text().await.unwrap_or_default();
    let Ok(val) = serde_json::from_str::<Value>(&text) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let abstract_txt = val["AbstractText"].as_str().unwrap_or("");
    let abstract_url = val["AbstractURL"].as_str().unwrap_or("");
    if !abstract_txt.is_empty() && !abstract_url.is_empty() {
        let heading = val["Heading"].as_str().unwrap_or("Instant Answer");
        out.push((
            heading.to_string(),
            abstract_url.to_string(),
            abstract_txt.to_string(),
        ));
    }
    if let Some(topics) = val["RelatedTopics"].as_array() {
        for t in topics.iter().take(6) {
            if let (Some(txt), Some(u)) = (t["Text"].as_str(), t["FirstURL"].as_str()) {
                let short_t: String = txt.chars().take(72).collect();
                out.push((short_t, u.to_string(), txt.to_string()));
            }
        }
    }
    Ok(out)
}

fn format_web_search_hits(provider: &str, hits: &[(String, String, String)]) -> String {
    let formatted: Vec<String> = hits
        .iter()
        .map(|(title, url, snippet)| {
            if snippet.trim().is_empty() {
                format!("- **[{title}]({url})**")
            } else {
                format!("- **[{title}]({url})**\n  {snippet}")
            }
        })
        .collect();
    format!(
        "[Live Web Search via {provider} · {} verified result(s)]\n{}\n\n(Cite ONLY these verified URLs; do not invent links.)",
        hits.len(),
        formatted.join("\n\n")
    )
}

fn validate_public_fetch_url(raw_url: &str) -> anyhow::Result<url::Url> {
    let cleaned = raw_url
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '<' || c == '>');
    let parsed = url::Url::parse(cleaned)
        .map_err(|e| anyhow::anyhow!("Invalid URL '{raw_url}': {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!(
            "Unsupported URL scheme '{}' (only http:// and https:// are allowed).",
            parsed.scheme()
        );
    }

    match parsed.host() {
        None => anyhow::bail!("URL has no host: '{raw_url}'"),
        Some(url::Host::Ipv4(ipv4)) => {
            if ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_unspecified()
                || ipv4.is_multicast()
            {
                anyhow::bail!("Private, loopback, or link-local IPv4 targets are blocked: '{raw_url}'");
            }
        }
        Some(url::Host::Ipv6(ipv6)) => {
            let segs = ipv6.segments();
            let is_unique_local = (segs[0] & 0xfe00) == 0xfc00;
            let is_link_local = (segs[0] & 0xffc0) == 0xfe80;
            let mapped_blocked = ipv6.to_ipv4_mapped().is_some_and(|v4| {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
            });
            if ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6.is_multicast()
                || is_unique_local
                || is_link_local
                || mapped_blocked
            {
                anyhow::bail!("Private, loopback, or link-local IPv6 targets are blocked: '{raw_url}'");
            }
        }
        Some(url::Host::Domain(domain)) => {
            let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
            if d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
            {
                anyhow::bail!("Local or internal domain targets are blocked: '{raw_url}'");
            }
        }
    }

    Ok(parsed)
}

fn github_api_fast_path(url: &url::Url) -> Option<(&'static str, String)> {
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if host != "github.com" && host != "www.github.com" {
        return None;
    }
    let segs: Vec<&str> = url
        .path_segments()?
        .filter(|s| !s.trim().is_empty())
        .collect();
    match segs.as_slice() {
        [owner, repo, "releases", "tag", tag] => Some((
            "github_release",
            format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}"),
        )),
        [owner, repo, "releases"] | [owner, repo, "releases", "latest"] => Some((
            "github_releases_list",
            format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=5"),
        )),
        [owner, repo] => Some((
            "github_readme",
            format!("https://api.github.com/repos/{owner}/{repo}/readme"),
        )),
        _ => None,
    }
}

fn decode_entities_preserving_lines(raw: &str) -> String {
    raw.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

fn html_to_readable_text(html: &str) -> (Option<String>, String) {
    let title_re = RegexBuilder::new(r"(?is)<title[^>]*>(.*?)</title>")
        .build()
        .expect("valid title regex");
    let title = title_re
        .captures(html)
        .map(|c| decode_basic_html_entities(&c[1]))
        .filter(|s| !s.is_empty());

    let strip_blocks_re = RegexBuilder::new(
        r"(?is)<(script|style|noscript|svg)[^>]*>.*?</(script|style|noscript|svg)>",
    )
    .build()
    .expect("valid strip blocks regex");
    let no_blocks = strip_blocks_re.replace_all(html, " ");

    let line_break_re = RegexBuilder::new(
        r"(?is)</?(?:p|div|section|article|header|footer|aside|main|li|tr|h[1-6]|blockquote|pre)[^>]*>|<br\s*/?>|<hr\s*/?>",
    )
    .build()
    .expect("valid block tag regex");
    let with_breaks = line_break_re.replace_all(&no_blocks, "\n");

    let tag_re = Regex::new(r"<[^>]+>").expect("valid tag regex");
    let no_tags = tag_re.replace_all(&with_breaks, " ");
    let decoded = decode_entities_preserving_lines(&no_tags);

    let mut clean_lines: Vec<String> = Vec::new();
    let mut prev_empty = false;
    for line in decoded.lines() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if !prev_empty && !clean_lines.is_empty() {
                clean_lines.push(String::new());
            }
            prev_empty = true;
        } else {
            clean_lines.push(collapsed);
            prev_empty = false;
        }
    }
    while clean_lines.last().is_some_and(|l| l.is_empty()) {
        clean_lines.pop();
    }

    (title, clean_lines.join("\n"))
}

fn truncate_chars_boundary(s: &str, max_chars: usize) -> String {
    let total_chars = s.chars().count();
    if total_chars <= max_chars {
        return s.to_string();
    }
    let byte_end = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    format!("{}\n\n... [content truncated]", &s[..byte_end])
}

async fn read_response_bytes_capped(
    resp: reqwest::Response,
    max_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk_res) = stream.next().await {
        let bytes = chunk_res?;
        let remaining = max_bytes.saturating_sub(buf.len());
        if remaining == 0 {
            break;
        }
        if bytes.len() <= remaining {
            buf.extend_from_slice(&bytes);
        } else {
            buf.extend_from_slice(&bytes[..remaining]);
            break;
        }
    }
    Ok(buf)
}

fn format_github_release_json(rel: &Value) -> String {
    let name = rel["name"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| rel["tag_name"].as_str())
        .unwrap_or("Release");
    let tag = rel["tag_name"].as_str().unwrap_or("");
    let html_url = rel["html_url"].as_str().unwrap_or("");
    let published = rel["published_at"].as_str().unwrap_or("");
    let body = rel["body"].as_str().unwrap_or("").trim();

    let mut out = if tag.is_empty() || name == tag {
        format!("# {name}\n")
    } else {
        format!("# {name} ({tag})\n")
    };
    if !html_url.is_empty() {
        out.push_str(&format!("URL: {html_url}\n"));
    }
    if !published.is_empty() {
        out.push_str(&format!("Published: {published}\n"));
    }
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
        out.push('\n');
    }
    if let Some(assets) = rel["assets"].as_array() {
        if !assets.is_empty() {
            out.push_str("\nAssets:\n");
            for a in assets {
                let aname = a["name"].as_str().unwrap_or("asset");
                let size = a["size"].as_u64().unwrap_or(0);
                out.push_str(&format!("- {aname} ({size} bytes)\n"));
            }
        }
    }
    out
}

async fn try_fetch_github_api(
    gate: &NetworkGate,
    kind: &str,
    api_url: &str,
    source_url: &str,
) -> Option<String> {
    let accept = if kind == "github_readme" {
        "application/vnd.github.raw+json"
    } else {
        "application/vnd.github+json"
    };
    let req = gate
        .request(Method::GET, api_url, "fetch_url_github_api")
        .ok()?
        .header("User-Agent", "cinder-ide")
        .header("Accept", accept);
    let resp = tokio::time::timeout(std::time::Duration::from_secs(15), req.send())
        .await
        .ok()?
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = read_response_bytes_capped(resp, 512 * 1024).await.ok()?;
    let raw_text = String::from_utf8_lossy(&bytes).to_string();

    if kind == "github_readme" {
        return Some(format!(
            "[Fetched GitHub README via API · {source_url}]\n\n{}",
            truncate_chars_boundary(&raw_text, 12_000)
        ));
    }

    let val: Value = serde_json::from_str(&raw_text).ok()?;
    let formatted = if kind == "github_release" {
        format_github_release_json(&val)
    } else {
        let arr = val.as_array()?;
        if arr.is_empty() {
            "No published releases found.".to_string()
        } else {
            arr.iter()
                .take(5)
                .map(format_github_release_json)
                .collect::<Vec<_>>()
                .join("\n---\n\n")
        }
    };

    Some(format!(
        "[Fetched GitHub Releases via API · {source_url}]\n\n{}",
        truncate_chars_boundary(&formatted, 12_000)
    ))
}

fn extract_direct_url_from_query(query: &str) -> Option<String> {
    let trimmed = query
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '<' || c == '>');
    if !trimmed.contains(char::is_whitespace)
        && (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        && validate_public_fetch_url(trimmed).is_ok()
    {
        return Some(trimmed.to_string());
    }

    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > 5 {
        return None;
    }
    let mut url_found: Option<String> = None;
    for tok in tokens {
        let clean_tok = tok.trim_matches(|c| {
            c == '"' || c == '\'' || c == '`' || c == '<' || c == '>' || c == ',' || c == '.'
        });
        if (clean_tok.starts_with("http://") || clean_tok.starts_with("https://"))
            && validate_public_fetch_url(clean_tok).is_ok()
        {
            if url_found.is_some() {
                return None;
            }
            url_found = Some(clean_tok.to_string());
        } else {
            let lower = clean_tok.to_ascii_lowercase();
            if !matches!(
                lower.as_str(),
                "go" | "to" | "open" | "read" | "fetch" | "check" | "visit" | "from" | "see"
            ) {
                return None;
            }
        }
    }
    url_found
}

async fn run_fetch_url(gate: &NetworkGate, args: &Value) -> anyhow::Result<ToolExecutionResult> {
    if gate.mode() == PrivacyMode::StrictLocal {
        anyhow::bail!(
            "fetch_url is disabled in Strict Local Mode. Inform the user that they can enable Hybrid Mode in the bottom-left System & Forge Telemetry popover if they want web access; models cannot switch privacy mode."
        );
    }

    let raw_url = arg_str(args, &["url", "query", "href"])
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' argument"))?;
    let parsed = validate_public_fetch_url(raw_url)?;
    let target_url = parsed.to_string();

    if let Some((kind, api_url)) = github_api_fast_path(&parsed) {
        if let Some(formatted) = try_fetch_github_api(gate, kind, &api_url, &target_url).await {
            return Ok(ToolExecutionResult {
                output: formatted,
                diff: None,
                file_path: None,
                is_error: false,
            });
        }
    }

    let req = gate
        .request(Method::GET, &target_url, "fetch_url")?
        .header("User-Agent", "cinder-ide")
        .header(
            "Accept",
            "text/html,text/plain,text/markdown,application/json;q=0.9",
        );
    let resp = tokio::time::timeout(std::time::Duration::from_secs(15), req.send())
        .await
        .map_err(|_| anyhow::anyhow!("Fetching '{target_url}' timed out after 15s"))??;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("Fetching '{target_url}' returned HTTP {status}");
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = read_response_bytes_capped(resp, 512 * 1024).await?;
    let raw_text = String::from_utf8_lossy(&bytes).to_string();

    let (title_opt, body) = if content_type.contains("html")
        || raw_text.trim_start().starts_with('<')
    {
        html_to_readable_text(&raw_text)
    } else {
        (None, raw_text)
    };

    let body_trimmed = truncate_chars_boundary(body.trim(), 12_000);
    let mut out = format!("[Fetched URL: {target_url}]\n");
    if let Some(title) = title_opt {
        out.push_str(&format!("Title: {title}\n"));
    }
    out.push('\n');
    out.push_str(&body_trimmed);

    Ok(ToolExecutionResult {
        output: out,
        diff: None,
        file_path: None,
        is_error: false,
    })
}

async fn run_web_search(
    gate: &NetworkGate,
    brave_key: &str,
    args: &Value,
) -> anyhow::Result<ToolExecutionResult> {
    if gate.mode() == PrivacyMode::StrictLocal {
        anyhow::bail!(
            "web_search is disabled in Strict Local Mode. Inform the user that they can enable Hybrid Mode in the bottom-left System & Forge Telemetry popover if they want web search; models cannot switch privacy mode."
        );
    }
    let query = args["query"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'query'"))?;

    if let Some(direct_url) = extract_direct_url_from_query(query) {
        return run_fetch_url(gate, &json!({ "url": direct_url })).await;
    }

    if !brave_key.trim().is_empty() {
        let encoded: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("q", query)
            .append_pair("count", "6")
            .finish();
        let url = format!("https://api.search.brave.com/res/v1/web/search?{encoded}");
        let resp = gate
            .request(Method::GET, &url, "web_search_brave")?
            .header("X-Subscription-Token", brave_key.trim())
            .header("Accept", "application/json")
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Brave search HTTP {}", resp.status());
        }
        let val: Value = resp.json().await?;
        let mut hits = Vec::new();
        if let Some(arr) = val["web"]["results"].as_array() {
            for r in arr.iter().take(6) {
                let title = r["title"].as_str().unwrap_or("Result").trim();
                let u = r["url"].as_str().unwrap_or("").trim();
                let desc = r["description"].as_str().unwrap_or("").trim();
                if !u.is_empty() {
                    hits.push((title.to_string(), u.to_string(), desc.to_string()));
                }
            }
        }
        if !hits.is_empty() {
            return Ok(ToolExecutionResult {
                output: format_web_search_hits("Brave Search", &hits),
                diff: None,
                file_path: None,
                is_error: false,
            });
        }
    }

    let prefer_wiki = query.to_ascii_lowercase().contains("wikipedia");
    if prefer_wiki {
        if let Ok(hits) = search_wikipedia_api(gate, query).await {
            if !hits.is_empty() {
                return Ok(ToolExecutionResult {
                    output: format_web_search_hits("Wikipedia API", &hits),
                    diff: None,
                    file_path: None,
                    is_error: false,
                });
            }
        }
        if let Ok(hits) = search_duckduckgo_html(gate, query).await {
            if !hits.is_empty() {
                return Ok(ToolExecutionResult {
                    output: format_web_search_hits("DuckDuckGo", &hits),
                    diff: None,
                    file_path: None,
                    is_error: false,
                });
            }
        }
    } else {
        if let Ok(hits) = search_duckduckgo_html(gate, query).await {
            if !hits.is_empty() {
                return Ok(ToolExecutionResult {
                    output: format_web_search_hits("DuckDuckGo", &hits),
                    diff: None,
                    file_path: None,
                    is_error: false,
                });
            }
        }
        if let Ok(hits) = search_wikipedia_api(gate, query).await {
            if !hits.is_empty() {
                return Ok(ToolExecutionResult {
                    output: format_web_search_hits("Wikipedia API", &hits),
                    diff: None,
                    file_path: None,
                    is_error: false,
                });
            }
        }
    }

    if let Ok(hits) = search_duckduckgo_instant(gate, query).await {
        if !hits.is_empty() {
            return Ok(ToolExecutionResult {
                output: format_web_search_hits("DuckDuckGo Instant", &hits),
                diff: None,
                file_path: None,
                is_error: false,
            });
        }
    }

    anyhow::bail!(
        "Live web search returned no results for '{query}'. Tell the user honestly that web search returned no results — NEVER fabricate URLs or claim a web search succeeded."
    )
}

/// Decode standard or URL-safe base64 (stripping optional data URL prefix & whitespace).
pub fn decode_base64(input: &str) -> anyhow::Result<Vec<u8>> {
    let raw = match input.split_once(";base64,") {
        Some((_, b64)) => b64,
        None => input,
    };
    let clean: Vec<u8> = raw
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if clean.len() % 4 == 1 {
        anyhow::bail!("Invalid base64 length");
    }

    let val_of = |c: u8| -> anyhow::Result<u8> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' | b'-' => Ok(62),
            b'/' | b'_' => Ok(63),
            _ => anyhow::bail!("Invalid base64 byte: {c}"),
        }
    };

    let mut out = Vec::with_capacity((clean.len() * 3) / 4);
    for chunk in clean.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let a = val_of(chunk[0])?;
        let b = val_of(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 && chunk[2] != b'=' {
            let c = val_of(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk.len() > 3 && chunk[3] != b'=' {
                let d = val_of(chunk[3])?;
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    #[test]
    fn base64_decoder_handles_padding_and_data_url() {
        assert_eq!(decode_base64("SGVsbG8gd29ybGQ=").unwrap(), b"Hello world");
        assert_eq!(
            decode_base64("data:text/plain;base64,SGVsbG8=").unwrap(),
            b"Hello"
        );
        assert!(decode_base64("invalid*char").is_err());
    }

    #[test]
    fn list_dir_finds_markdown_in_subfolders() {
        let root = std::env::temp_dir().join(format!(
            "cinder_tools_list_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("Vorlesung")).unwrap();
        std::fs::write(root.join("README.md"), "# Uni\n").unwrap();
        std::fs::write(root.join("Vorlesung").join("kapitel1.md"), "# Kapitel 1\n").unwrap();

        let res = run_list_dir(&root, &json!({ "path": "." })).unwrap();
        assert!(!res.output.contains("empty directory"), "got: {}", res.output);
        assert!(res.output.contains("README.md"), "got: {}", res.output);
        assert!(
            res.output.contains("Vorlesung/kapitel1.md"),
            "got: {}",
            res.output
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_files_handles_md_glob_and_latin1_content() {
        let root = std::env::temp_dir().join(format!(
            "cinder_tools_search_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("Seminar")).unwrap();
        std::fs::write(root.join("notes.md"), "Intro line\n").unwrap();
        // Latin-1 encoded "Lösung"
        std::fs::write(
            root.join("Seminar").join("blatt.md"),
            b"Aufgabe: L\xf6sung pr\xfcfen\n",
        )
        .unwrap();

        let md_hits = run_search_files(&root, &json!({ "pattern": "*.md" })).unwrap();
        assert!(md_hits.output.contains("[file] notes.md"), "got: {}", md_hits.output);
        assert!(
            md_hits.output.contains("[file] Seminar/blatt.md"),
            "got: {}",
            md_hits.output
        );

        let text_hits = run_search_files(&root, &json!({ "pattern": "Lösung" })).unwrap();
        assert!(
            text_hits.output.contains("Seminar/blatt.md:1:"),
            "got: {}",
            text_hits.output
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_file_resolves_nested_filename_and_latin1() {
        let root = std::env::temp_dir().join(format!(
            "cinder_tools_read_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("Vorlesung")).unwrap();
        std::fs::write(
            root.join("Vorlesung").join("kapitel1.md"),
            b"Gr\xfc\xdfe aus Mathe\n",
        )
        .unwrap();

        let res = run_read_file(&root, &json!({ "path": "kapitel1.md" })).unwrap();
        assert_eq!(res.file_path.as_deref(), Some("Vorlesung/kapitel1.md"));
        assert!(res.output.contains("Grüße aus Mathe"), "got: {}", res.output);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn edit_file_handles_windows_crlf_and_arg_aliases() {
        let root = std::env::temp_dir().join(format!(
            "cinder_tools_crlf_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("sample.txt"), "line1\r\nold_block\r\nline3\r\n").unwrap();

        let change = prepare_file_change(
            &root,
            "edit_file",
            &json!({
                "file_path": "sample.txt",
                "old_text": "line1\nold_block",
                "new_text": "line1\nnew_block"
            }),
        )
        .unwrap();
        apply_file_change(&change).unwrap();
        let written = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(written, "line1\r\nnew_block\r\nline3\r\n");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn edit_file_resolves_nested_path_fallback_and_strips_line_numbers() {
        let root = std::env::temp_dir().join(format!(
            "cinder_tools_edit_fallback_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("website")).unwrap();
        std::fs::write(
            root.join("website").join("index.html"),
            "<div class=\"nav-brand\">\n  <span>CINDER</span>\n</div>\n",
        )
        .unwrap();

        // Resolving bare filename `"index.html"` via `"file"` alias AND stripping read_file line numbers
        let change = prepare_file_change(
            &root,
            "edit_file",
            &json!({
                "file": "index.html",
                "target": "  14: <div class=\"nav-brand\">\n  15:   <span>CINDER</span>",
                "replacement": "  14: <div class=\"nav-brand centered\">\n  15:   <span>CINDER</span>"
            }),
        )
        .unwrap();
        assert_eq!(change.rel_path, "website/index.html");
        apply_file_change(&change).unwrap();
        let written = std::fs::read_to_string(root.join("website").join("index.html")).unwrap();
        assert!(written.contains("<div class=\"nav-brand centered\">"));
        assert!(!written.contains("14:"), "line number leaked: {written}");

        // Single-line numbered prefix should also be stripped when needed
        let change2 = prepare_file_change(
            &root,
            "edit_file",
            &json!({
                "path": "index.html",
                "target": "  15:   <span>CINDER</span>",
                "replacement": "  15:   <span>CINDER IDE</span>"
            }),
        )
        .unwrap();
        apply_file_change(&change2).unwrap();
        let written2 = std::fs::read_to_string(root.join("website").join("index.html")).unwrap();
        assert!(written2.contains("<span>CINDER IDE</span>"));

        // Truncated raw JSON error message
        let err = prepare_file_change(
            &root,
            "edit_file",
            &json!({ "raw": "{\"replacement\":\"...truncated" }),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("truncated or malformed"),
            "unexpected error: {err}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_dir_and_search_resolve_subdir_fallback() {
        let root = std::env::temp_dir().join(format!(
            "cinder_tools_dir_fallback_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("apps").join("website")).unwrap();
        std::fs::write(
            root.join("apps").join("website").join("style.css"),
            ".hero { color: #ff6b2b; }\n",
        )
        .unwrap();

        let listed = run_list_dir(&root, &json!({ "path": "website" })).unwrap();
        assert!(listed.output.contains("apps/website"), "got: {}", listed.output);
        assert!(
            listed.output.contains("apps/website/style.css"),
            "got: {}",
            listed.output
        );

        let searched =
            run_search_files(&root, &json!({ "pattern": "#ff6b2b", "sub_path": "website" }))
                .unwrap();
        assert!(
            searched.output.contains("apps/website/style.css:1:"),
            "got: {}",
            searched.output
        );

        let missing = run_search_files(
            &root,
            &json!({ "pattern": "anything", "sub_path": "does_not_exist" }),
        );
        assert!(missing.is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn empty_workspace_returns_clear_error_instead_of_os_123() {
        let gate = harness_net::NetworkGate::new(harness_net::PrivacyMode::StrictLocal).unwrap();
        let list_res = execute_tool(Path::new(""), &gate, "", "list_dir", &json!({})).await;
        assert!(list_res.is_error);
        assert!(
            list_res.output.contains("No project workspace is attached"),
            "got: {}",
            list_res.output
        );

        let cmd_res = execute_tool(
            Path::new(""),
            &gate,
            "",
            "run_command",
            &json!({ "command": "echo hi" }),
        )
        .await;
        assert!(cmd_res.is_error);
        assert!(
            cmd_res.output.contains("No valid project workspace"),
            "got: {}",
            cmd_res.output
        );
    }

    #[tokio::test]
    async fn web_search_is_blocked_in_strict_local_without_prompting_model_to_switch() {
        let gate = harness_net::NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let res = execute_tool(
            Path::new(""),
            &gate,
            "",
            "web_search",
            &json!({ "query": "Riemann hypothesis" }),
        )
        .await;
        assert!(res.is_error);
        assert!(
            res.output.contains("models cannot switch privacy mode"),
            "unexpected output: {}",
            res.output
        );
    }

    #[test]
    fn chat_mode_exposes_web_search_and_delegate_research_in_hybrid() {
        let off = tool_specs(AgentMode::Chat, false);
        assert!(!off.iter().any(|t| t.name == "web_search"));
        assert!(!off.iter().any(|t| t.name == "delegate_research"));

        let on = tool_specs(AgentMode::Chat, true);
        assert!(on.iter().any(|t| t.name == "web_search"));
        assert!(on.iter().any(|t| t.name == "fetch_url"));
        assert!(on.iter().any(|t| t.name == "delegate_research"));

        let web_off = filter_tools_by_modules(on.clone(), &["fs_read".into()]);
        assert!(!web_off.iter().any(|t| t.name == "fetch_url"));
        let web_on = filter_tools_by_modules(on, &["web".into()]);
        assert!(web_on.iter().any(|t| t.name == "fetch_url"));
    }

    #[tokio::test]
    async fn fetch_url_is_blocked_in_strict_local() {
        let gate = harness_net::NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let res = execute_tool(
            Path::new(""),
            &gate,
            "",
            "fetch_url",
            &json!({ "url": "https://github.com/cruellophon/cinder_dev/releases/tag/v0.5.6-beta" }),
        )
        .await;
        assert!(res.is_error);
        assert!(
            res.output.contains("models cannot switch privacy mode"),
            "unexpected output: {}",
            res.output
        );
    }

    #[test]
    fn validate_public_fetch_url_blocks_private_and_loopback_and_allows_public() {
        for blocked in [
            "http://127.0.0.1:8080/secret",
            "http://localhost:11434/api/tags",
            "http://192.168.1.1/admin",
            "http://10.0.0.1/status",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]:8080/",
            "http://[::ffff:127.0.0.1]/",
            "file:///etc/passwd",
        ] {
            assert!(
                validate_public_fetch_url(blocked).is_err(),
                "expected SSRF block for {blocked}"
            );
        }

        let ok = validate_public_fetch_url(
            "https://github.com/cruellophon/cinder_dev/releases/tag/v0.5.6-beta",
        )
        .unwrap();
        assert_eq!(ok.host_str(), Some("github.com"));
    }

    #[test]
    fn github_api_fast_path_maps_release_and_repo_urls() {
        let tag_u = url::Url::parse(
            "https://github.com/cruellophon/cinder_dev/releases/tag/v0.5.6-beta",
        )
        .unwrap();
        assert_eq!(
            github_api_fast_path(&tag_u),
            Some((
                "github_release",
                "https://api.github.com/repos/cruellophon/cinder_dev/releases/tags/v0.5.6-beta"
                    .to_string()
            ))
        );

        let list_u =
            url::Url::parse("https://github.com/cruellophon/cinder_dev/releases").unwrap();
        assert_eq!(
            github_api_fast_path(&list_u),
            Some((
                "github_releases_list",
                "https://api.github.com/repos/cruellophon/cinder_dev/releases?per_page=5"
                    .to_string()
            ))
        );

        let repo_u = url::Url::parse("https://github.com/cruellophon/cinder_dev").unwrap();
        assert_eq!(
            github_api_fast_path(&repo_u),
            Some((
                "github_readme",
                "https://api.github.com/repos/cruellophon/cinder_dev/readme".to_string()
            ))
        );
    }

    #[test]
    fn html_to_readable_text_strips_scripts_styles_and_extracts_title() {
        let html = r#"
        <html>
          <head>
            <title>Cinder &amp; Release Notes</title>
            <style>body { color: red; }</style>
          </head>
          <body>
            <script>console.log("ignore");</script>
            <h1>v0.5.6-beta</h1>
            <p>Added macOS Intel &amp; Apple Silicon support.</p>
          </body>
        </html>
        "#;
        let (title, body) = html_to_readable_text(html);
        assert_eq!(title.as_deref(), Some("Cinder & Release Notes"));
        assert!(!body.contains("color: red"));
        assert!(!body.contains("console.log"));
        assert!(body.contains("v0.5.6-beta"));
        assert!(body.contains("Added macOS Intel & Apple Silicon support."));
    }

    #[test]
    fn web_search_html_parser_extracts_ddg_and_wikipedia_entities() {
        let html = r#"
        <div class="result">
          <a class="result__a" href="https://duckduckgo.com/l/?uddg=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FLanglands_program">Langlands program &amp; math</a>
          <a class="result__snippet">Bridge between <b>number theory</b> and representation theory.</a>
        </div>
        "#;
        let hits = parse_ddg_html_results(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "Langlands program & math");
        assert_eq!(hits[0].1, "https://en.wikipedia.org/wiki/Langlands_program");
        assert_eq!(
            hits[0].2,
            "Bridge between number theory and representation theory."
        );
    }
}
