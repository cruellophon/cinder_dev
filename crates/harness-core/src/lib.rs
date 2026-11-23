pub mod checkpoints;
pub mod orchestrator;
pub mod skills;
pub mod subagent;

pub use checkpoints::{
    accept_checkpoint_file, record_file_backup_in_checkpoint, revert_checkpoint_file,
    rollback_entire_checkpoint,
};
pub use orchestrator::{
    resolve_auto_executor_model, resolve_auto_planner_model, resolve_reachable_model,
    run_orchestrator_turn,
};
pub use skills::{
    expand_slash_skill, expand_slash_skill_with_global, list_skills, list_skills_with_global,
    save_custom_skill,
};
pub use subagent::{
    resolve_research_model, resolve_vision_model, run_research_subagent, run_vision_subagent,
};

use chrono::Utc;
use harness_models::{LocalModelEntry, ServerManager};
use harness_net::{NetworkGate, PrivacyMode};
use harness_providers::{
    complete, CompletionRequest, CompletionResponse, ProviderMessage, ToolCallRequest, ToolSpec,
};
use harness_store::{
    AgentMode, BumboStore, ChatMessage, CompareResult, MediaAttachment, OrchestratorPhaseRecord,
    PendingToolApproval, PendingUserQuestion, PlanArtifact, PlanStatus, PlanStep, RiskTier,
    Session, SessionArtifact, ToolCallRecord,
};
use harness_tools::{
    apply_file_change, call_mcp_tool, classify_tool_risk, discover_enabled_mcp_tools,
    execute_tool, extract_pdf_file_text, filter_tools_by_modules, mcp_tools_to_specs,
    prepare_file_change, resolve_workspace_path, sanitize_mcp_slug, tool_specs,
    ToolExecutionResult,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct HarnessContext<'a> {
    pub gate: &'a NetworkGate,
    pub store: &'a BumboStore,
    pub server_mgr: &'a ServerManager,
    pub local_models: &'a [LocalModelEntry],
}

pub(crate) async fn complete_cancellable(
    ctx: &HarnessContext<'_>,
    req: CompletionRequest,
) -> anyhow::Result<Option<CompletionResponse>> {
    let abort_wait = async {
        while !ctx.store.is_aborted() {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }
    };
    tokio::select! {
        _ = abort_wait => Ok(None),
        res = complete(ctx.gate, ctx.store, ctx.server_mgr, ctx.local_models, req) => {
            res.map(Some)
        }
    }
}

pub(crate) fn active_tools_for_mode(
    mode: AgentMode,
    allow_web: bool,
    enabled_modules: &[String],
) -> Vec<ToolSpec> {
    filter_tools_by_modules(tool_specs(mode, allow_web), enabled_modules)
}

pub(crate) fn active_tools_for_context(
    mode: AgentMode,
    allow_web: bool,
    enabled_modules: &[String],
    has_workspace: bool,
) -> Vec<ToolSpec> {
    let mut specs = active_tools_for_mode(mode, allow_web, enabled_modules);
    if !has_workspace {
        specs.retain(|t| {
            matches!(
                t.name.as_str(),
                "delegate_vision" | "write_artifact" | "ask_user" | "web_search" | "fetch_url"
            ) || (allow_web && t.name == "delegate_research")
        });
    }
    specs
}

pub(crate) fn apply_thinking_effort(
    base_sys_prompt: &str,
    effort: &str,
) -> (bool, String) {
    match effort {
        "off" => (false, base_sys_prompt.to_string()),
        "deep" => (
            true,
            format!(
                "{base_sys_prompt}\n\nReasoning Depth: DEEP. Carefully analyze dependencies, edge cases, and verification steps before taking action."
            ),
        ),
        _ => (true, base_sys_prompt.to_string()),
    }
}

pub async fn upsert_session_artifact(
    store: &BumboStore,
    session_id: &str,
    title: &str,
    kind: &str,
    content: &str,
) -> anyhow::Result<SessionArtifact> {
    let clean_title = if title.trim().is_empty() {
        "Session Artifact".to_string()
    } else {
        title.trim().to_string()
    };
    let clean_kind = match kind.trim() {
        "task_list" | "plan" | "walkthrough" | "report" | "html_widget" => kind.trim().to_string(),
        _ => "report".to_string(),
    };
    let now = Utc::now();
    store
        .update(|s| {
            let sess = s.sessions.iter_mut().find(|x| x.id == session_id)?;
            if let Some(existing) = sess.artifacts.iter_mut().find(|a| {
                a.title.eq_ignore_ascii_case(&clean_title)
                    || (clean_kind == "task_list" && a.kind == "task_list")
            }) {
                existing.title = clean_title.clone();
                existing.kind = clean_kind.clone();
                existing.content = content.to_string();
                existing.updated_at = now;
                sess.updated_at = now;
                return Some(existing.clone());
            }
            let art = SessionArtifact {
                id: Uuid::new_v4().to_string(),
                title: clean_title,
                kind: clean_kind,
                content: content.to_string(),
                updated_at: now,
            };
            sess.artifacts.push(art.clone());
            sess.updated_at = now;
            Some(art)
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))
}

pub async fn delete_session_artifact(
    store: &BumboStore,
    session_id: &str,
    artifact_id: &str,
) -> anyhow::Result<Session> {
    store
        .update(|s| {
            let sess = s.sessions.iter_mut().find(|x| x.id == session_id)?;
            sess.artifacts.retain(|a| a.id != artifact_id);
            sess.updated_at = Utc::now();
            Some(sess.clone())
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))
}

pub(crate) async fn upsert_artifact_from_tool(
    store: &BumboStore,
    session_id: &str,
    tc: &ToolCallRequest,
) -> (ToolCallRecord, String) {
    let title = tc.arguments["title"].as_str().unwrap_or("Session Artifact");
    let kind = tc.arguments["kind"].as_str().unwrap_or("report");
    let content = tc.arguments["content"].as_str().unwrap_or("");
    match upsert_session_artifact(store, session_id, title, kind, content).await {
        Ok(art) => {
            let out = format!("Artifact '{}' ({}) saved (id={}).", art.title, art.kind, art.id);
            let rec = ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments_json: tc.arguments.to_string(),
                status: "completed".into(),
                output: out.clone(),
                diff: None,
                file_path: None,
                risk_tier: Some(RiskTier::SafeReadOnly),
                subagent_summary: None,
                subagent_model: None,
            };
            (rec, out)
        }
        Err(e) => {
            let out = format!("Artifact error: {e}");
            let rec = ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments_json: tc.arguments.to_string(),
                status: "error".into(),
                output: out.clone(),
                diff: None,
                file_path: None,
                risk_tier: Some(RiskTier::SafeReadOnly),
                subagent_summary: None,
                subagent_model: None,
            };
            (rec, out)
        }
    }
}

pub async fn compact_session_context(store: &BumboStore, session_id: &str) -> anyhow::Result<Session> {
    store
        .update(|s| {
            let sess = s.sessions.iter_mut().find(|x| x.id == session_id)?;
            let keep_tail = 4usize;
            if sess.messages.len() <= keep_tail + 1 {
                return Some(sess.clone());
            }
            let split_idx = sess.messages.len().saturating_sub(keep_tail);
            let older = &sess.messages[..split_idx];

            let mut bullets = Vec::new();
            let mut touched_files: Vec<String> = Vec::new();
            for m in older {
                let first_line = m.content.lines().next().unwrap_or("").trim();
                let preview: String = first_line.chars().take(120).collect();
                if m.role == "user" {
                    bullets.push(format!("- **User**: {preview}"));
                } else {
                    let tools_cnt = m.tool_calls.len();
                    for tc in &m.tool_calls {
                        if let Some(fp) = &tc.file_path {
                            if !touched_files.contains(fp) {
                                touched_files.push(fp.clone());
                            }
                        }
                    }
                    bullets.push(format!(
                        "- **{}**{}: {preview}",
                        m.model_id.as_deref().unwrap_or("Assistant"),
                        if tools_cnt > 0 {
                            format!(" ({tools_cnt} tool call(s))")
                        } else {
                            String::new()
                        }
                    ));
                }
            }

            let files_line = if touched_files.is_empty() {
                String::new()
            } else {
                format!("\n\n**Files referenced**: `{}`", touched_files.join("`, `"))
            };
            let summary_msg = ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: "assistant".into(),
                content: format!(
                    "## Compacted Context Summary ({} earlier messages)\n{}{files_line}",
                    older.len(),
                    bullets.join("\n")
                ),
                thinking: None,
                model_id: Some("harness::compact".into()),
                latency_ms: Some(0),
                prompt_tokens: Some(0),
                completion_tokens: Some(0),
                tool_calls: Vec::new(),
                compare_partner: None,
                checkpoint_id: None,
                plan_artifact: None,
                orchestrator_phases: Vec::new(),
                attached_files: Vec::new(),
                attachments: Vec::new(),
                artifact_id: None,
                created_at: Utc::now(),
            };

            let mut compacted = vec![summary_msg];
            compacted.extend(sess.messages.drain(split_idx..));
            sess.messages = compacted;
            sess.updated_at = Utc::now();
            Some(sess.clone())
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))
}

fn truncate_middle_utf8(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars || max_chars < 64 {
        return s.to_string();
    }
    let head_len = (max_chars * 65) / 100;
    let tail_len = max_chars.saturating_sub(head_len + 48).max(32);
    let head: String = s.chars().take(head_len).collect();
    let tail: String = s.chars().skip(total.saturating_sub(tail_len)).collect();
    let dropped = total.saturating_sub(head_len + tail_len);
    format!("{head}\n\n... [truncated {dropped} chars to fit context window] ...\n\n{tail}")
}

fn estimate_provider_history_chars(history: &[ProviderMessage]) -> usize {
    history
        .iter()
        .map(|m| {
            let tc_len: usize = m
                .tool_calls
                .iter()
                .map(|t| t.name.len() + t.arguments.to_string().len() + 24)
                .sum();
            m.content.len() + tc_len + 32
        })
        .sum()
}

pub(crate) fn is_context_overflow_error(err_str: &str) -> bool {
    let lower = err_str.to_ascii_lowercase();
    lower.contains("exceeds the available context size")
        || lower.contains("context_length_exceeded")
        || lower.contains("maximum context length")
        || lower.contains("too many tokens")
        || lower.contains("prompt is too long")
        || (lower.contains("400") && lower.contains("context"))
}

/// Condense oversized or overflowing mid-task provider history in place while keeping tool-call pairs intact.
pub(crate) fn compact_provider_history_in_place(
    history: &mut Vec<ProviderMessage>,
    target_char_budget: usize,
    collected_tools: &[ToolCallRecord],
) -> bool {
    if history.is_empty() {
        return false;
    }
    let mut changed = false;
    let budget = target_char_budget.max(1800);
    let per_msg_cap = (budget / 2).max(1200);

    let total_len = history.len();

    for (idx, m) in history.iter_mut().enumerate() {
        if idx + 1 < total_len && !m.images.is_empty() {
            m.images.clear();
            m.content.push_str("\n[Earlier attached image omitted after context check]");
            changed = true;
        }
        if m.content.chars().count() > per_msg_cap {
            m.content = truncate_middle_utf8(&m.content, per_msg_cap);
            changed = true;
        }
    }

    if estimate_provider_history_chars(history) <= budget {
        return changed;
    }

    if history.len() > 3 {
        let mut split_idx = history.len().saturating_sub(3).max(1);
        while split_idx > 1 && history[split_idx].role == "tool" {
            split_idx -= 1;
        }
        while split_idx < history.len() && history[split_idx].role == "tool" {
            split_idx += 1;
        }

        if split_idx > 0 && split_idx < history.len() {
            let mut summary_lines = Vec::new();
            for m in &history[..split_idx] {
                let first_line = m.content.lines().next().unwrap_or("").trim();
                let preview: String = first_line.chars().take(140).collect();
                if m.role == "user" {
                    summary_lines.push(format!("- User: {preview}"));
                } else if m.role == "assistant" {
                    if !m.tool_calls.is_empty() {
                        let names: Vec<&str> = m.tool_calls.iter().map(|t| t.name.as_str()).collect();
                        summary_lines.push(format!("- Assistant called [{}]: {preview}", names.join(", ")));
                    } else if !preview.is_empty() {
                        summary_lines.push(format!("- Assistant: {preview}"));
                    }
                } else if m.role == "tool" {
                    let tname = m.tool_name.as_deref().unwrap_or("tool");
                    summary_lines.push(format!("- Tool `{tname}`: {preview}"));
                }
            }

            let mut touched_files: Vec<String> = Vec::new();
            for tc in collected_tools {
                if let Some(ref fp) = tc.file_path {
                    if !touched_files.contains(fp) {
                        touched_files.push(fp.clone());
                    }
                }
            }
            let files_note = if touched_files.is_empty() {
                String::new()
            } else {
                format!("\nFiles touched/read so far: `{}`", touched_files.join("`, `"))
            };

            let summary_text = format!(
                "[Mid-Task Auto-Compacted Context Checkpoint ({split_idx} earlier steps condensed)]\n{}\n{files_note}\nContinue seamlessly from the remaining recent steps below without repeating completed work.",
                summary_lines.join("\n")
            );
            let checkpoint_msg = ProviderMessage {
                role: "user".into(),
                content: truncate_middle_utf8(&summary_text, (budget / 3).max(900)),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
                images: Vec::new(),
            };

            let mut next = vec![checkpoint_msg];
            next.extend(history.drain(split_idx..));
            *history = next;
            changed = true;
        }
    }

    if estimate_provider_history_chars(history) > budget && !history.is_empty() {
        let slice_cap = (budget / history.len().max(1)).max(500);
        for m in history.iter_mut() {
            if !m.images.is_empty() {
                m.images.clear();
                changed = true;
            }
            if m.content.chars().count() > slice_cap {
                m.content = truncate_middle_utf8(&m.content, slice_cap);
                changed = true;
            }
        }
    }

    changed
}

pub async fn rewind_session_to_message(
    store: &BumboStore,
    session_id: &str,
    message_index: usize,
    revert_files: bool,
) -> anyhow::Result<(Session, String, Vec<String>, Vec<MediaAttachment>)> {
    let snap = store.snapshot().await;
    let sess = snap
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
    if message_index >= sess.messages.len() {
        anyhow::bail!("Message index out of range.");
    }
    let prompt_text = sess.messages[message_index].content.clone();
    let attached_files = sess.messages[message_index].attached_files.clone();
    let attachments = sess.messages[message_index].attachments.clone();
    let cp_ids: Vec<String> = sess.messages[message_index..]
        .iter()
        .filter_map(|m| m.checkpoint_id.clone())
        .collect();

    if revert_files {
        for cp_id in cp_ids.iter().rev() {
            let _ = rollback_entire_checkpoint(store, session_id, cp_id).await;
        }
    }

    let updated = store
        .update(|s| {
            if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sx.messages.truncate(message_index);
                sx.pending_approval = None;
                sx.pending_question = None;
                if revert_files {
                    sx.checkpoints.retain(|c| !cp_ids.contains(&c.id));
                }
                sx.updated_at = Utc::now();
                return Some(sx.clone());
            }
            None
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

    Ok((updated, prompt_text, attached_files, attachments))
}

fn read_markdown_lossy(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let slice = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let text = match std::str::from_utf8(slice) {
        Ok(s) => s.to_string(),
        Err(_) => slice.iter().map(|&b| b as char).collect(),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn load_workspace_rules(workspace_root: &Path) -> String {
    let cinder_path = workspace_root.join(".cinder").join("rules.md");
    if cinder_path.exists() {
        return read_markdown_lossy(&cinder_path).unwrap_or_default();
    }
    let legacy_path = workspace_root.join(".bumbo").join("rules.md");
    if legacy_path.exists() {
        return read_markdown_lossy(&legacy_path).unwrap_or_default();
    }
    String::new()
}

fn load_workspace_instruction_docs(workspace_root: &Path) -> Vec<(String, String)> {
    const MAX_TOTAL_BYTES: usize = 12 * 1024;
    let mut docs = Vec::new();
    let mut used_bytes = 0usize;

    for name in ["GEMINI.md", "CLAUDE.md", "AGENTS.md"] {
        let p = workspace_root.join(name);
        if p.is_file() {
            if let Some(txt) = read_markdown_lossy(&p) {
                let take_len = (MAX_TOTAL_BYTES.saturating_sub(used_bytes)).min(txt.len());
                if take_len == 0 {
                    break;
                }
                let mut end = take_len;
                while end > 0 && !txt.is_char_boundary(end) {
                    end -= 1;
                }
                used_bytes += end;
                docs.push((name.to_string(), txt[..end].to_string()));
            }
        }
    }

    let rules_dir = workspace_root.join(".agents").join("rules");
    if rules_dir.is_dir() && used_bytes < MAX_TOTAL_BYTES {
        let mut rule_files: Vec<PathBuf> = std::fs::read_dir(&rules_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown"))
                        .unwrap_or(false)
            })
            .collect();
        rule_files.sort();
        for p in rule_files {
            if used_bytes >= MAX_TOTAL_BYTES {
                break;
            }
            if let Some(txt) = read_markdown_lossy(&p) {
                let fname = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("rule.md");
                let take_len = (MAX_TOTAL_BYTES.saturating_sub(used_bytes)).min(txt.len());
                if take_len == 0 {
                    break;
                }
                let mut end = take_len;
                while end > 0 && !txt.is_char_boundary(end) {
                    end -= 1;
                }
                used_bytes += end;
                docs.push((format!(".agents/rules/{fname}"), txt[..end].to_string()));
            }
        }
    }

    docs
}

fn collect_md_overview_rec(
    dir: &Path,
    rel_prefix: &str,
    depth: usize,
    top_folders: &mut Vec<String>,
    md_files: &mut Vec<String>,
    total_md_seen: &mut usize,
) {
    if depth > 4 || md_files.len() >= 120 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(String, PathBuf, bool)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "target" | "node_modules" | "__pycache__" | ".venv" | "dist" | "build"
                )
            {
                return None;
            }
            let p = e.path();
            let is_dir = p.is_dir();
            Some((name, p, is_dir))
        })
        .collect();
    entries.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));

    for (name, p, is_dir) in entries {
        let rel = if rel_prefix.is_empty() {
            name
        } else {
            format!("{rel_prefix}/{name}")
        };
        if is_dir {
            if depth == 0 && top_folders.len() < 20 {
                top_folders.push(format!("{rel}/"));
            }
            collect_md_overview_rec(&p, &rel, depth + 1, top_folders, md_files, total_md_seen);
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown") {
                *total_md_seen += 1;
                if md_files.len() < 35 {
                    md_files.push(rel);
                }
            }
        }
    }
}

fn collect_workspace_markdown_overview(workspace_root: &Path) -> Option<String> {
    if workspace_root.as_os_str().is_empty() || !workspace_root.is_dir() {
        return None;
    }
    let mut top_folders = Vec::new();
    let mut md_files = Vec::new();
    let mut total_md_seen = 0usize;
    collect_md_overview_rec(
        workspace_root,
        "",
        0,
        &mut top_folders,
        &mut md_files,
        &mut total_md_seen,
    );
    if top_folders.is_empty() && md_files.is_empty() {
        return None;
    }
    let mut out = String::from("Workspace Markdown & Folder Overview:");
    if !top_folders.is_empty() {
        out.push_str("\nTop-level folders: ");
        out.push_str(&top_folders.join(", "));
    }
    if !md_files.is_empty() {
        out.push_str("\nMarkdown files (.md):");
        for md in &md_files {
            out.push_str("\n- ");
            out.push_str(md);
        }
        if total_md_seen > md_files.len() {
            out.push_str(&format!(
                "\n... (+{} more .md files; use search_files with '*.md')",
                total_md_seen - md_files.len()
            ));
        }
    }
    Some(out)
}

pub fn save_workspace_rules(workspace_root: &Path, content: &str) -> anyhow::Result<()> {
    let dir = workspace_root.join(".cinder");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("rules.md"), content)?;
    let legacy_dir = workspace_root.join(".bumbo");
    if legacy_dir.exists() {
        let _ = std::fs::write(legacy_dir.join("rules.md"), content);
    }
    Ok(())
}

fn mode_label(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Auto => "auto",
        AgentMode::Chat => "chat",
        AgentMode::Plan => "plan",
        AgentMode::Agent => "agent",
        AgentMode::Orchestrator => "orchestrator",
        AgentMode::Compare => "compare",
    }
}

fn parse_target_mode(raw: &str) -> Option<AgentMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "chat" => Some(AgentMode::Chat),
        "agent" => Some(AgentMode::Agent),
        "orchestrator" => Some(AgentMode::Orchestrator),
        "plan" => Some(AgentMode::Plan),
        _ => None,
    }
}

fn is_privacy_mode_keyword(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "hybrid"
            | "local"
            | "strict_local"
            | "strictlocal"
            | "strict-local"
            | "online"
            | "offline"
            | "internet"
            | "network"
    )
}

pub fn classify_auto_posture(
    user_text: &str,
    attached_files: &[String],
    active_plan: Option<&PlanArtifact>,
) -> (AgentMode, &'static str) {
    if active_plan
        .map(|p| p.status == PlanStatus::Approved)
        .unwrap_or(false)
    {
        return (AgentMode::Agent, "Approved plan ready for execution");
    }

    let lower = user_text.to_ascii_lowercase();
    let plan_keywords = [
        "design a",
        "architect",
        "sketch out",
        "create a plan",
        "plan out",
        "propose a plan",
        "before coding",
    ];
    if plan_keywords.iter().any(|k| lower.contains(k)) {
        return (AgentMode::Plan, "Design & planning task detected");
    }

    let action_keywords = [
        "edit ",
        "fix ",
        "refactor",
        "implement",
        "create file",
        "write a ",
        "update ",
        "run ",
        "test",
        "verify",
        "compile",
        "build",
        "search",
        "read ",
        "inspect ",
        "in the workspace",
        "check the ",
        ".rs",
        ".py",
        ".js",
        ".md",
        ".pdf",
        ".toml",
    ];
    if !attached_files.is_empty() || action_keywords.iter().any(|k| lower.contains(k)) {
        return (
            AgentMode::Agent,
            "Workspace inspection or coding action detected",
        );
    }

    (
        AgentMode::Chat,
        "Direct conversational query (can escalate via switch_mode)",
    )
}

pub(crate) fn build_system_prompt(
    workspace_root: &Path,
    mode: AgentMode,
    is_local: bool,
    active_plan: Option<&PlanArtifact>,
    custom_system_prompt: &str,
) -> String {
    let is_standalone = workspace_root.as_os_str().is_empty();
    let rules = if is_standalone {
        String::new()
    } else {
        load_workspace_rules(workspace_root)
    };
    let instruction_docs = if is_standalone {
        Vec::new()
    } else {
        load_workspace_instruction_docs(workspace_root)
    };
    let md_overview = if is_standalone {
        None
    } else {
        collect_workspace_markdown_overview(workspace_root)
    };
    let mode_desc = if is_standalone {
        "You are Cinder in a standalone one-off conversation with NO project folder attached. \
         Answer directly and concisely. Do NOT attempt to inspect local directories or run shell commands \
         (you may use `delegate_vision` for screenshots/images, `write_artifact` for notes/diagrams, \
         or `web_search` in Hybrid mode). If the user asks about a local repository or files on disk, \
         tell them to open or select that project folder in the left sidebar."
    } else {
        match mode {
            AgentMode::Auto => {
                "You are Cinder in ADAPTIVE AUTO mode. Match your workflow to the request: answer direct questions cleanly, use read-only tools + `submit_plan` or `ask_user` when planning, or use workspace file/shell tools + `verify_workspace` when editing/running code. You can also call `switch_mode` (`chat`, `plan`, `agent`, `orchestrator`) anytime to transition modes."
            }
            AgentMode::Chat => {
                "You are Cinder, a helpful local-first engineering & academic assistant. Answer conversational questions clearly and concisely. \
                If the task requires inspecting workspace files, designing an implementation plan, or editing/running code, call `switch_mode` (`target_mode`: 'plan' | 'agent' | 'orchestrator') to activate workspace tools."
            }
            AgentMode::Plan => {
                "You are Cinder in PLAN mode. Inspect the workspace read-only (`list_dir`, `read_file`, `search_files`, `extract_pdf_text`, `search_knowledge`, `delegate_research`). \
                If requirements have ambiguity, call `ask_user` with concise clickable options. \
                When ready, call `submit_plan` with a structured Plan Artifact (title, summary, steps, files_touched, verification) so the user can review and approve execution. \
                If the user asks for immediate implementation instead of a plan, call `switch_mode` (`target_mode`: 'agent')."
            }
            AgentMode::Agent | AgentMode::Orchestrator => {
                "You are Cinder in AGENT mode. Use tools carefully inside the active workspace, inspect before editing, run `verify_workspace` after edits to verify syntax/tests and self-correct any failures, and report only observed results. You may call `switch_mode` if planning review (`plan`) or pure conversation (`chat`) is more appropriate."
            }
            AgentMode::Compare => "You are Cinder. Answer accurately and concisely.",
        }
    };

    let compact_hint = if is_local {
        "\nKeep tool calls minimal and never repeat identical failing tool calls."
    } else {
        ""
    };

    let workspace_line = if is_standalone {
        "Standalone one-off conversation (no project workspace attached)".to_string()
    } else {
        format!("Active workspace: {}", workspace_root.display())
    };
    let mut prompt = format!("{mode_desc}\n{workspace_line}{compact_hint}");

    if !custom_system_prompt.trim().is_empty() {
        prompt.push_str("\n\nUser Custom Instructions:\n");
        prompt.push_str(custom_system_prompt.trim());
    }

    if let Some(plan) = active_plan {
        if plan.status == PlanStatus::Approved {
            prompt.push_str("\n\nApproved Plan to Execute:\n");
            prompt.push_str(&plan.markdown);
        }
    }

    if !rules.trim().is_empty() {
        prompt.push_str("\n\nProject Rules (.cinder/rules.md):\n");
        prompt.push_str(rules.trim());
    }

    for (doc_name, doc_text) in instruction_docs {
        prompt.push_str(&format!("\n\nProject Instructions ({doc_name}):\n{doc_text}"));
    }

    if let Some(overview) = md_overview {
        prompt.push_str("\n\n");
        prompt.push_str(&overview);
        prompt.push_str("\n(Pass relative paths like those above directly to `read_file` or use `search_files` with '*.md'.)");
    }

    prompt.push_str("\n\nVision Sub-Agent: if screenshots or images are attached (or if you need to inspect an image file inside the workspace), call `delegate_vision` to analyze/OCR them via a dedicated vision-capable sub-agent.");

    if !is_standalone && mode != AgentMode::Compare {
        prompt.push_str(
            "\nThought Trace & Completion Summary Protocol:\n\
             1. During tool steps, keep internal planning inside `<think>...</think>` and invoke your planned tool call immediately in the same turn (never dump raw stream-of-consciousness or `[Step ...]` markers into the user-facing response).\n\
             2. Once you finish executing tool steps, ALWAYS provide a concise, structured Markdown summary so the user can clearly see the task is finished and trace your decisions:\n\
                - **What Was Done**: numbered or bulleted summary of files and symbols inspected, created, or modified (wrap file paths and identifiers in backticks like `path/to/file.rs`).\n\
                - **Reasoning & Key Decisions**: explain why you made these changes or what root cause you found so any issues are easy to trace.\n\
                - **Verification & Status**: report command/test outcomes or any remaining caveats.\n\
             Workspace Pathing & Editing:\n\
             - Always pass workspace-relative forward-slash paths (e.g. `website/index.html`, never repeating the workspace folder name).\n\
             - For `edit_file`, NEVER include line-number prefixes (like `14: `) from `read_file`, and keep `target`/`replacement` concise (only the lines being changed; use `write_file` only for full-file rewrites).",
        );
    }

    prompt
}

fn append_hybrid_guidance(
    prompt: &mut String,
    allow_web: bool,
    active_model_id: &str,
    delegate_cloud_model: Option<&str>,
) {
    if !allow_web {
        return;
    }
    prompt.push_str(
        "\n\nLive Web Search Policy: When the user asks to research something online, look up Wikipedia or current facts, or provide source links, you MUST call `web_search` (directly or via `delegate_research`) and cite ONLY verified URLs returned by tool output. NEVER fabricate URLs or claim a web search succeeded if none ran or if it failed.",
    );
    if active_model_id.starts_with("local::") {
        if let Some(cloud_id) = delegate_cloud_model {
            prompt.push_str(&format!(
                "\nHybrid Cloud Delegation: You are in Hybrid Mode with cloud model '{cloud_id}' reachable. Do NOT be shy about delegating web research, complex math/science deep-dives, or hard analysis to `delegate_research` (it automatically delegates to '{cloud_id}' with live `web_search` enabled) or screenshots to `delegate_vision`."
            ));
        }
    }
}

pub(crate) fn build_structured_work_summary(tools: &[ToolCallRecord]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut edited: Vec<String> = Vec::new();
    let mut inspected: Vec<String> = Vec::new();
    let mut commands: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for tc in tools {
        let args: Value = serde_json::from_str(&tc.arguments_json).unwrap_or(Value::Null);
        let rel = tc
            .file_path
            .as_deref()
            .or_else(|| tool_arg_str(&args, &["path", "file_path", "filepath", "file"]))
            .map(str::to_string);
        match tc.name.as_str() {
            "edit_file" | "write_file" | "propose_edit" => {
                if let Some(f) = rel {
                    let item = format!("`{f}` ({})", tc.name);
                    if !edited.contains(&item) {
                        edited.push(item);
                    }
                }
            }
            "read_file" | "list_dir" | "search_files" | "extract_pdf_text"
            | "search_knowledge" => {
                let target = rel
                    .or_else(|| {
                        tool_arg_str(&args, &["pattern", "query", "dir", "directory"])
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "workspace".into());
                let item = format!("`{target}` via `{}`", tc.name);
                if !inspected.contains(&item) {
                    inspected.push(item);
                }
            }
            "run_command" | "verify_workspace" => {
                let cmd = tool_arg_str(&args, &["command", "cmd"]).unwrap_or("command");
                commands.push(format!("`{cmd}` ({})", tc.status));
            }
            _ => {
                let desc = describe_tool_intent_short(&tc.name, &args);
                inspected.push(desc);
            }
        }
        if tc.status != "completed" {
            let first_err = tc.output.lines().next().unwrap_or("error").trim();
            let cut: String = first_err.chars().take(120).collect();
            errors.push(format!("`{}`: {cut}", tc.name));
        }
    }

    let mut out = String::from("### Summary of Work & Reasoning\n");
    out.push_str("- **What Was Done**:\n");
    if !edited.is_empty() {
        out.push_str(&format!("  - Modified/created: {}\n", edited.join(", ")));
    }
    if !inspected.is_empty() {
        let preview: Vec<String> = inspected.into_iter().take(6).collect();
        out.push_str(&format!("  - Inspected: {}\n", preview.join(", ")));
    }
    if !commands.is_empty() {
        out.push_str(&format!("  - Ran commands: {}\n", commands.join(", ")));
    }
    out.push_str(&format!(
        "- **Status**: Completed {} tool step(s){}.",
        tools.len(),
        if errors.is_empty() {
            " with no tool errors"
        } else {
            " with errors"
        }
    ));
    if !errors.is_empty() {
        out.push_str(&format!("\n- **Issues Encountered**: {}", errors.join("; ")));
    }
    out
}

fn tool_arg_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for &k in keys {
        if let Some(s) = args.get(k).and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

fn short_file_basename(raw: &str) -> String {
    let norm = raw.trim().replace('\\', "/");
    norm.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(norm.as_str())
        .to_string()
}

pub(crate) fn describe_tool_intent_short(name: &str, args: &Value) -> String {
    let trunc = |s: &str, max: usize| -> String {
        let single = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if single.chars().count() <= max {
            single
        } else {
            let cut: String = single.chars().take(max.saturating_sub(1)).collect();
            format!("{cut}…")
        }
    };

    match name {
        "read_file" => {
            let rel = tool_arg_str(args, &["path", "file_path", "filepath", "file"])
                .unwrap_or("file");
            let base = short_file_basename(rel);
            let start = args["start_line"].as_u64();
            let end = args["end_line"].as_u64();
            match (start, end) {
                (Some(s), Some(e)) => format!("analyze {base} #L{s}-{e}"),
                (Some(s), None) => format!("analyze {base} from L{s}"),
                _ => format!("analyze {base}"),
            }
        }
        "search_files" => {
            let pat = tool_arg_str(args, &["pattern", "query", "regex", "text"]).unwrap_or("*");
            let pat_s = trunc(pat, 36);
            if let Some(sub) = tool_arg_str(args, &["sub_path", "path", "dir", "directory"])
                .filter(|s| *s != ".")
            {
                format!("search for '{pat_s}' in {}", trunc(sub, 28))
            } else {
                format!("search for '{pat_s}'")
            }
        }
        "list_dir" => {
            let rel = tool_arg_str(args, &["path", "dir", "directory", "folder"]).unwrap_or(".");
            if rel == "." {
                "explore workspace root".into()
            } else {
                format!("explore {}", trunc(rel, 40))
            }
        }
        "edit_file" | "propose_edit" => {
            let rel = tool_arg_str(args, &["path", "file_path", "filepath"]).unwrap_or("file");
            format!("edit {}", short_file_basename(rel))
        }
        "write_file" => {
            let rel = tool_arg_str(args, &["path", "file_path", "filepath"]).unwrap_or("file");
            format!("write {}", short_file_basename(rel))
        }
        "run_command" | "verify_workspace" => {
            let cmd = tool_arg_str(args, &["command", "cmd"]).unwrap_or("command");
            format!("run `{}`", trunc(cmd, 56))
        }
        "switch_mode" => {
            let target = tool_arg_str(args, &["target_mode"]).unwrap_or("next");
            format!("switch mode to {target}")
        }
        "submit_plan" => {
            let title = tool_arg_str(args, &["title"]).unwrap_or("plan");
            format!("submit plan '{}'", trunc(title, 40))
        }
        "write_artifact" => {
            let title = tool_arg_str(args, &["title"]).unwrap_or("artifact");
            format!("write artifact '{}'", trunc(title, 40))
        }
        "delegate_vision" => "analyze image via Vision Sub-Agent".into(),
        "delegate_research" => {
            let task = tool_arg_str(args, &["task"]).unwrap_or("workspace");
            format!("research '{}'", trunc(task, 48))
        }
        "search_knowledge" => {
            let q = tool_arg_str(args, &["query"]).unwrap_or("knowledge");
            format!("search knowledge for '{}'", trunc(q, 40))
        }
        "extract_pdf_text" => {
            let rel = tool_arg_str(args, &["path", "file_path", "filepath"]).unwrap_or("PDF");
            format!("extract PDF {}", short_file_basename(rel))
        }
        "ask_user" => "ask user for clarification".into(),
        other => format!("run {other}"),
    }
}

pub(crate) fn format_step_thought_trace(
    step_num: usize,
    thinking: Option<&str>,
    intermediate_content: &str,
    tool_calls: &[harness_providers::ToolCallRequest],
) -> Option<String> {
    let th = thinking.unwrap_or("").trim();
    let ic = intermediate_content.trim();
    if tool_calls.is_empty() {
        if th.is_empty() {
            return None;
        }
        return if step_num > 1 {
            Some(format!("[Step {step_num} · Final response]\n{th}"))
        } else {
            Some(th.to_string())
        };
    }

    let actions: Vec<String> = tool_calls
        .iter()
        .map(|tc| describe_tool_intent_short(&tc.name, &tc.arguments))
        .collect();
    let headline = actions.join(", ");

    let body = match (th.is_empty(), ic.is_empty()) {
        (false, false) if !th.contains(ic) => format!("{th}\n{ic}"),
        (false, _) => th.to_string(),
        (true, false) => ic.to_string(),
        (true, true) => format!("Planning to {headline}."),
    };
    Some(format!("[Step {step_num} · {headline}]\n{body}"))
}

pub(crate) fn looks_like_unexecuted_tool_intent(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 1400 || trimmed.contains("```") {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    const PATTERNS: &[&str] = &[
        "i'll start by searching",
        "i'll start by inspecting",
        "i'll start by checking",
        "i'll start by listing",
        "i'll start by reading",
        "i will start by",
        "let me search",
        "let me inspect",
        "let me check",
        "let me list",
        "i'll search for",
        "i will search for",
        "i will use ",
    ];
    PATTERNS.iter().any(|p| lower.contains(p))
}

pub(crate) fn encode_base64(input: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(TBL[(b0 >> 2) as usize] as char);
        out.push(TBL[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            TBL[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TBL[(b2 & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub(crate) fn image_mime_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

pub(crate) fn strip_data_url_prefix(raw: &str) -> &str {
    match raw.split_once(";base64,") {
        Some((_, b64)) => b64,
        None => raw,
    }
}

pub(crate) fn collect_session_images(
    sess: &Session,
    workspace_root: &Path,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for m in sess.messages.iter().rev().take(6) {
        for att in &m.attachments {
            if att.mime_type.starts_with("image/") {
                if let Some(ref b64) = att.data_base64 {
                    let clean = strip_data_url_prefix(b64).trim();
                    if !clean.is_empty() {
                        out.push((att.name.clone(), att.mime_type.clone(), clean.to_string()));
                        if out.len() >= 4 {
                            return out;
                        }
                    }
                }
            }
        }
        for rel in &m.attached_files {
            if let Ok(abs) = resolve_workspace_path(workspace_root, rel) {
                if abs.is_file() {
                    let ext = abs
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if let Some(mime) = image_mime_for_ext(&ext) {
                        if let Ok(bytes) = std::fs::read(&abs) {
                            if bytes.len() <= 10 * 1024 * 1024 {
                                out.push((
                                    rel.clone(),
                                    mime.to_string(),
                                    encode_base64(&bytes),
                                ));
                                if out.len() >= 4 {
                                    return out;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

pub(crate) async fn handle_delegate_vision_tool(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    workspace_root: &Path,
    tc: &ToolCallRequest,
) -> (ToolCallRecord, ProviderMessage, OrchestratorPhaseRecord) {
    let prompt = tc.arguments["prompt"]
        .as_str()
        .unwrap_or("Inspect the attached image(s) and summarize all relevant text and visual details.");
    let image_paths: Vec<String> = tc.arguments["image_paths"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let session_images = {
        let snap = ctx.store.snapshot().await;
        snap.sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| collect_session_images(s, workspace_root))
            .unwrap_or_default()
    };

    let t0 = std::time::Instant::now();
    let (status, summary, used_model) = match run_vision_subagent(
        ctx,
        session_id,
        workspace_root,
        prompt,
        &image_paths,
        &session_images,
    )
    .await
    {
        Ok((s, model)) => (
            "completed".to_string(),
            s,
            (!model.trim().is_empty()).then_some(model),
        ),
        Err(e) => ("error".to_string(), format!("Vision sub-agent error: {e}"), None),
    };
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    let phase = OrchestratorPhaseRecord {
        phase: "vision-subagent".into(),
        model_id: used_model
            .clone()
            .unwrap_or_else(|| "vision-subagent".into()),
        summary: format!(
            "Inspected {} image(s)",
            if image_paths.is_empty() {
                session_images.len()
            } else {
                image_paths.len()
            }
        ),
        latency_ms: elapsed_ms,
        tokens: 0,
    };

    let rec = ToolCallRecord {
        id: tc.id.clone(),
        name: tc.name.clone(),
        arguments_json: tc.arguments.to_string(),
        status,
        output: summary.clone(),
        diff: None,
        file_path: image_paths.first().cloned(),
        risk_tier: Some(RiskTier::SafeReadOnly),
        subagent_summary: Some(summary.clone()),
        subagent_model: used_model,
    };

    let msg = ProviderMessage {
        role: "tool".into(),
        content: summary,
        tool_call_id: Some(tc.id.clone()),
        tool_name: Some(tc.name.clone()),
        tool_calls: Vec::new(),
        images: Vec::new(),
    };

    (rec, msg, phase)
}

pub(crate) async fn handle_delegate_research_tool(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    workspace_root: &Path,
    caller_model_id: &str,
    tc: &ToolCallRequest,
) -> (ToolCallRecord, ProviderMessage, OrchestratorPhaseRecord) {
    let task = tc.arguments["task"]
        .as_str()
        .unwrap_or("Inspect workspace and research task.");
    let focus: Vec<String> = tc.arguments["focus_paths"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let prefer_cloud = tc.arguments["prefer_cloud"]
        .as_bool()
        .unwrap_or_else(|| caller_model_id.starts_with("local::"));

    let t0 = std::time::Instant::now();
    let (status, summary, used_model) = match run_research_subagent(
        ctx,
        session_id,
        workspace_root,
        caller_model_id,
        task,
        &focus,
        prefer_cloud,
    )
    .await
    {
        Ok((s, model)) => (
            "completed".to_string(),
            s,
            (!model.trim().is_empty()).then_some(model),
        ),
        Err(e) => (
            "error".to_string(),
            format!("Research sub-agent error: {e}"),
            None,
        ),
    };
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    let phase = OrchestratorPhaseRecord {
        phase: "research-subagent".into(),
        model_id: used_model
            .clone()
            .unwrap_or_else(|| caller_model_id.to_string()),
        summary: task.chars().take(80).collect(),
        latency_ms: elapsed_ms,
        tokens: 0,
    };

    let rec = ToolCallRecord {
        id: tc.id.clone(),
        name: tc.name.clone(),
        arguments_json: tc.arguments.to_string(),
        status,
        output: summary.clone(),
        diff: None,
        file_path: None,
        risk_tier: Some(RiskTier::SafeReadOnly),
        subagent_summary: Some(summary.clone()),
        subagent_model: used_model,
    };

    let msg = ProviderMessage {
        role: "tool".into(),
        content: summary,
        tool_call_id: Some(tc.id.clone()),
        tool_name: Some(tc.name.clone()),
        tool_calls: Vec::new(),
        images: Vec::new(),
    };

    (rec, msg, phase)
}

fn extract_attachment_context_and_images(
    workspace_root: &Path,
    attached_files: &[String],
    attachments: &[MediaAttachment],
) -> (String, Vec<(String, String)>) {
    let mut blocks = Vec::new();
    let mut images = Vec::new();

    for rel in attached_files.iter().take(6) {
        let Ok(abs) = resolve_workspace_path(workspace_root, rel) else {
            continue;
        };
        if !abs.is_file() {
            continue;
        }
        let ext = abs
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(mime) = image_mime_for_ext(&ext) {
            if let Ok(bytes) = std::fs::read(&abs) {
                if bytes.len() <= 8 * 1024 * 1024 {
                    images.push((mime.to_string(), encode_base64(&bytes)));
                    blocks.push(format!("### Attached Workspace Image: {rel} ({mime})"));
                }
            }
            continue;
        }
        let body = if ext == "pdf" {
            extract_pdf_file_text(&abs, 6).unwrap_or_default()
        } else {
            std::fs::read_to_string(&abs)
                .map(|raw| raw.lines().take(200).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default()
        };
        if !body.trim().is_empty() {
            blocks.push(format!("### File: {rel}\n```\n{body}\n```"));
        }
    }

    for att in attachments.iter().take(6) {
        if att.mime_type.starts_with("image/") {
            if let Some(ref b64) = att.data_base64 {
                let clean = strip_data_url_prefix(b64).trim();
                if !clean.is_empty() {
                    images.push((att.mime_type.clone(), clean.to_string()));
                    blocks.push(format!(
                        "### Dropped/Pasted Image: {} ({})",
                        att.name, att.mime_type
                    ));
                }
            }
        } else if let Some(ref txt) = att.text_content {
            if !txt.trim().is_empty() {
                let snippet: String = txt.lines().take(250).collect::<Vec<_>>().join("\n");
                blocks.push(format!("### Dropped File: {}\n```\n{snippet}\n```", att.name));
            }
        }
    }

    let ctx_str = if blocks.is_empty() {
        String::new()
    } else {
        format!(
            "<attached_workspace_context>\n{}\n</attached_workspace_context>\n\n",
            blocks.join("\n\n")
        )
    };
    (ctx_str, images)
}

pub async fn run_turn(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    user_text: &str,
    mode: AgentMode,
    model_id: &str,
    compare_model_b: Option<&str>,
    attached_files: Vec<String>,
    attachments: Vec<MediaAttachment>,
) -> anyhow::Result<Session> {
    ctx.store.clear_abort();
    let trimmed_cmd = user_text.trim();
    if trimmed_cmd.eq_ignore_ascii_case("/compact") {
        return compact_session_context(ctx.store, session_id).await;
    }

    let snapshot = ctx.store.snapshot().await;
    let sess_ref = snapshot.sessions.iter().find(|s| s.id == session_id);
    let workspace_path = sess_ref
        .map(|s| s.workspace_path.clone())
        .unwrap_or_else(|| snapshot.settings.active_workspace_path.clone());
    let active_plan = sess_ref.and_then(|s| s.active_plan.clone());

    // Persist `/learn <rule>` into `.bumbo/rules.md` automatically when attached to a project
    if !workspace_path.trim().is_empty() {
        if let Some(rule_rest) = trimmed_cmd
            .strip_prefix("/learn ")
            .or_else(|| trimmed_cmd.strip_prefix("/LEARN "))
        {
            let rule_clean = rule_rest.trim();
            if !rule_clean.is_empty() {
                let mut rules = load_workspace_rules(Path::new(&workspace_path));
                if !rules.ends_with('\n') && !rules.is_empty() {
                    rules.push('\n');
                }
                rules.push_str(&format!("- {rule_clean}\n"));
                let _ = save_workspace_rules(Path::new(&workspace_path), &rules);
            }
        }
    }

    let configured_steps = snapshot
        .settings
        .orchestrator
        .max_autonomous_steps
        .clamp(2, 64);
    let max_steps = if trimmed_cmd.starts_with("/goal ") || trimmed_cmd.eq_ignore_ascii_case("/goal")
    {
        configured_steps.max(16)
    } else {
        configured_steps
    };
    let effort_override = if trimmed_cmd.starts_with("/boost ")
        || trimmed_cmd.eq_ignore_ascii_case("/boost")
    {
        Some("deep".to_string())
    } else {
        None
    };

    let global_skills_dir = ctx.store.paths().skills_dir();
    let (expanded_text, skill_mode) = expand_slash_skill_with_global(
        Path::new(&workspace_path),
        Some(&global_skills_dir),
        user_text,
    );
    let effective_mode = skill_mode.unwrap_or(mode);
    let (attached_ctx, prompt_images) = extract_attachment_context_and_images(
        Path::new(&workspace_path),
        &attached_files,
        &attachments,
    );
    let prompt_with_context = if attached_ctx.is_empty() {
        expanded_text.clone()
    } else {
        format!("{attached_ctx}{expanded_text}")
    };

    let user_msg = ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: "user".into(),
        content: user_text.to_string(),
        thinking: None,
        model_id: None,
        latency_ms: None,
        prompt_tokens: None,
        completion_tokens: None,
        tool_calls: Vec::new(),
        compare_partner: None,
        checkpoint_id: None,
        plan_artifact: None,
        orchestrator_phases: Vec::new(),
        attached_files: attached_files.clone(),
        attachments: attachments.clone(),
        artifact_id: None,
        created_at: Utc::now(),
    };

    let history_msgs: Vec<ProviderMessage> = ctx
        .store
        .update(|s| {
            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sess.mode = effective_mode;
                sess.pending_approval = None;
                sess.pending_question = None;
                if sess.messages.is_empty() && !user_text.trim().is_empty() {
                    let trimmed: String = user_text.trim().chars().take(42).collect();
                    sess.title = trimmed;
                }
                sess.messages.push(user_msg);
                sess.updated_at = Utc::now();

                let len = sess.messages.len();
                let start = len.saturating_sub(16);
                let tail_slice = &sess.messages[start..];
                let tail_len = tail_slice.len();
                tail_slice
                    .iter()
                    .enumerate()
                    .map(|(idx, m)| {
                        let is_last = idx + 1 == tail_len;
                        let keep_images = idx + 2 >= tail_len;
                        let content = if is_last {
                            prompt_with_context.clone()
                        } else {
                            m.content.clone()
                        };
                        let imgs = if !keep_images {
                            Vec::new()
                        } else if is_last {
                            prompt_images.clone()
                        } else {
                            m.attachments
                                .iter()
                                .filter(|a| a.mime_type.starts_with("image/"))
                                .filter_map(|a| {
                                    a.data_base64
                                        .as_deref()
                                        .map(|b| (a.mime_type.clone(), strip_data_url_prefix(b).to_string()))
                                })
                                .collect()
                        };
                        ProviderMessage {
                            role: if m.role == "assistant" {
                                "assistant".into()
                            } else {
                                "user".into()
                            },
                            content,
                            tool_call_id: None,
                            tool_name: None,
                            tool_calls: Vec::new(),
                            images: imgs,
                        }
                    })
                    .collect()
            } else {
                vec![ProviderMessage {
                    role: "user".into(),
                    content: prompt_with_context.clone(),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                    images: prompt_images.clone(),
                }]
            }
        })
        .await?;

    ctx.store
        .set_activity_stream(
            Some(session_id),
            "thinking",
            &format!("Connecting to {model_id}..."),
            1,
            max_steps,
            model_id,
            "",
            "",
            &[],
        )
        .await;

    if effective_mode == AgentMode::Orchestrator {
        let res = run_orchestrator_turn(
            ctx,
            session_id,
            Path::new(&workspace_path),
            &prompt_with_context,
            history_msgs,
            attached_files,
        )
        .await;
        ctx.store.clear_activity().await;
        return res;
    }

    let resolved_model =
        resolve_reachable_model(ctx, model_id, &snapshot.settings.active_model_id)
            .await
            .unwrap_or_else(|_| model_id.to_string());
    let mut mode_phases: Vec<OrchestratorPhaseRecord> = Vec::new();
    if resolved_model != model_id {
        mode_phases.push(OrchestratorPhaseRecord {
            phase: "auto-fallback".into(),
            model_id: resolved_model.clone(),
            summary: format!("Redirected unreachable model '{model_id}' to '{resolved_model}'"),
            latency_ms: 0,
            tokens: 0,
        });
        let rm = resolved_model.clone();
        let _ = ctx
            .store
            .update(|s| {
                s.settings.active_model_id = rm;
            })
            .await;
    }
    let model_id = resolved_model.as_str();

    if effective_mode == AgentMode::Compare {
        let model_b = compare_model_b.unwrap_or(&snapshot.settings.compare_model_b_id);
        let resolved_b = resolve_reachable_model(ctx, model_b, model_id)
            .await
            .unwrap_or_else(|_| model_b.to_string());
        return run_compare_turn(
            ctx,
            session_id,
            &workspace_path,
            history_msgs,
            model_id,
            &resolved_b,
            &snapshot.settings.ui.custom_system_prompt,
        )
        .await;
    }

    let initial_loop_mode = if effective_mode == AgentMode::Auto {
        if workspace_path.trim().is_empty() {
            mode_phases.push(OrchestratorPhaseRecord {
                phase: "auto → Chat (standalone)".into(),
                model_id: model_id.to_string(),
                summary: "Standalone one-off conversation (no workspace attached).".into(),
                latency_ms: 0,
                tokens: 0,
            });
            AgentMode::Chat
        } else {
            let (posture, reason) = classify_auto_posture(
                &expanded_text,
                &attached_files,
                active_plan.as_ref(),
            );
            mode_phases.push(OrchestratorPhaseRecord {
                phase: format!("auto → {}", mode_label(posture)),
                model_id: model_id.to_string(),
                summary: reason.into(),
                latency_ms: 0,
                tokens: 0,
            });
            posture
        }
    } else {
        effective_mode
    };

    let allow_web = ctx.gate.mode() == PrivacyMode::Hybrid;
    let res = run_agent_loop(
        ctx,
        session_id,
        Path::new(&workspace_path),
        model_id,
        initial_loop_mode,
        allow_web,
        true,
        snapshot.settings.ui.custom_system_prompt.clone(),
        history_msgs,
        0,
        max_steps,
        effort_override,
        Vec::new(),
        mode_phases,
        Uuid::new_v4().to_string(),
        user_text.chars().take(48).collect(),
    )
    .await;

    ctx.store.clear_activity().await;
    res
}

async fn run_compare_turn(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    workspace_path: &str,
    history_msgs: Vec<ProviderMessage>,
    model_a: &str,
    model_b: &str,
    custom_sys: &str,
) -> anyhow::Result<Session> {
    ctx.store
        .set_activity(
            Some(session_id),
            "compare",
            &format!("Comparing {model_a} vs {model_b}..."),
            1,
            2,
        )
        .await;

    let sys_a = build_system_prompt(
        Path::new(workspace_path),
        AgentMode::Compare,
        model_a.starts_with("local::"),
        None,
        custom_sys,
    );
    let sys_b = build_system_prompt(
        Path::new(workspace_path),
        AgentMode::Compare,
        model_b.starts_with("local::"),
        None,
        custom_sys,
    );

    let req_a = CompletionRequest {
        model_id: model_a.to_string(),
        system_prompt: sys_a,
        messages: history_msgs.clone(),
        tools: Vec::new(),
        enable_thinking: true,
        max_tokens: 2048,
        temperature: 0.3,
    };
    let req_b = CompletionRequest {
        model_id: model_b.to_string(),
        system_prompt: sys_b,
        messages: history_msgs,
        tools: Vec::new(),
        enable_thinking: true,
        max_tokens: 2048,
        temperature: 0.3,
    };

    let res_a = complete(ctx.gate, ctx.store, ctx.server_mgr, ctx.local_models, req_a).await;
    let res_b = complete(ctx.gate, ctx.store, ctx.server_mgr, ctx.local_models, req_b).await;
    ctx.store.clear_activity().await;

    let assistant_msg = match (res_a, res_b) {
        (Ok(a), Ok(b)) => ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: a.content,
            thinking: a.thinking,
            model_id: Some(model_a.to_string()),
            latency_ms: Some(a.latency_ms),
            prompt_tokens: Some(a.prompt_tokens),
            completion_tokens: Some(a.completion_tokens),
            tool_calls: Vec::new(),
            compare_partner: Some(CompareResult {
                model_id: model_b.to_string(),
                content: b.content,
                thinking: b.thinking,
                latency_ms: b.latency_ms,
                total_tokens: b.prompt_tokens + b.completion_tokens + b.thinking_tokens,
            }),
            checkpoint_id: None,
            plan_artifact: None,
            orchestrator_phases: Vec::new(),
            attached_files: Vec::new(),
            attachments: Vec::new(),
            artifact_id: None,
            created_at: Utc::now(),
        },
        (Ok(a), Err(eb)) => ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: a.content,
            thinking: a.thinking,
            model_id: Some(model_a.to_string()),
            latency_ms: Some(a.latency_ms),
            prompt_tokens: Some(a.prompt_tokens),
            completion_tokens: Some(a.completion_tokens),
            tool_calls: Vec::new(),
            compare_partner: Some(CompareResult {
                model_id: model_b.to_string(),
                content: format!("Error from {model_b}: {eb}"),
                thinking: None,
                latency_ms: 0,
                total_tokens: 0,
            }),
            checkpoint_id: None,
            plan_artifact: None,
            orchestrator_phases: Vec::new(),
            attached_files: Vec::new(),
            attachments: Vec::new(),
            artifact_id: None,
            created_at: Utc::now(),
        },
        (Err(ea), Ok(b)) => ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: format!("Error from {model_a}: {ea}"),
            thinking: None,
            model_id: Some(model_a.to_string()),
            latency_ms: Some(0),
            prompt_tokens: None,
            completion_tokens: None,
            tool_calls: Vec::new(),
            compare_partner: Some(CompareResult {
                model_id: model_b.to_string(),
                content: b.content,
                thinking: b.thinking,
                latency_ms: b.latency_ms,
                total_tokens: b.prompt_tokens + b.completion_tokens + b.thinking_tokens,
            }),
            checkpoint_id: None,
            plan_artifact: None,
            orchestrator_phases: Vec::new(),
            attached_files: Vec::new(),
            attachments: Vec::new(),
            artifact_id: None,
            created_at: Utc::now(),
        },
        (Err(ea), Err(eb)) => {
            anyhow::bail!("Both models failed: {model_a}: {ea} | {model_b}: {eb}")
        }
    };

    let updated = ctx
        .store
        .update(|s| {
            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sess.messages.push(assistant_msg);
                sess.updated_at = Utc::now();
                return Some(sess.clone());
            }
            None
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

    Ok(updated)
}

pub async fn resolve_pending_approval(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    approve: bool,
) -> anyhow::Result<Session> {
    ctx.store.clear_abort();
    let snapshot = ctx.store.snapshot().await;
    let sess = snapshot
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

    let pending = sess
        .pending_approval
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No pending tool approval for this session."))?;

    let workspace_path = sess.workspace_path.clone();
    let mode = sess.mode;
    let mut history: Vec<ProviderMessage> =
        serde_json::from_str(&pending.agent_history_json).unwrap_or_default();

    ctx.store
        .update(|s| {
            if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sx.pending_approval = None;
                sx.updated_at = Utc::now();
            }
        })
        .await?;

    let args: Value = serde_json::from_str(&pending.arguments_json).unwrap_or_else(|_| Value::Null);
    let checkpoint_id = Uuid::new_v4().to_string();

    let exec_res = if approve {
        execute_tool_with_checkpoint(
            ctx,
            session_id,
            &checkpoint_id,
            &pending.tool_name,
            &args,
            Path::new(&workspace_path),
            &snapshot.settings.api_keys.brave_search_api_key,
            &pending.tool_name,
        )
        .await
    } else {
        ToolExecutionResult {
            output: "User rejected this tool action. Explain or propose an alternative.".into(),
            diff: pending.diff_preview.clone(),
            file_path: pending.target_path.clone(),
            is_error: true,
        }
    };

    let record = ToolCallRecord {
        id: pending.id.clone(),
        name: pending.tool_name.clone(),
        arguments_json: pending.arguments_json.clone(),
        status: if !approve {
            "rejected".into()
        } else if exec_res.is_error {
            "error".into()
        } else {
            "completed".into()
        },
        output: exec_res.output.clone(),
        diff: exec_res.diff.clone(),
        file_path: exec_res.file_path.clone(),
        risk_tier: Some(pending.risk_tier),
        subagent_summary: None,
        subagent_model: None,
    };

    history.push(ProviderMessage {
        role: "tool".into(),
        content: exec_res.output,
        tool_call_id: Some(pending.id.clone()),
        tool_name: Some(pending.tool_name.clone()),
        tool_calls: Vec::new(),
        images: Vec::new(),
    });

    let allow_web = ctx.gate.mode() == PrivacyMode::Hybrid;
    let configured_steps = snapshot
        .settings
        .orchestrator
        .max_autonomous_steps
        .clamp(2, 64);
    let next_max_steps = configured_steps.max(pending.step_index + 2);
    let res = run_agent_loop(
        ctx,
        session_id,
        Path::new(&workspace_path),
        &pending.model_id,
        mode,
        allow_web,
        true,
        snapshot.settings.ui.custom_system_prompt.clone(),
        history,
        pending.step_index + 1,
        next_max_steps,
        None,
        vec![record],
        Vec::new(),
        checkpoint_id,
        format!("Approved {}", pending.tool_name),
    )
    .await;

    ctx.store.clear_activity().await;
    res
}

pub(crate) fn parse_granted_contingent_steps(
    selected_options: &[String],
    custom_text: &str,
    default_steps: usize,
) -> Option<usize> {
    if let Some(opt) = selected_options.first() {
        let lower = opt.trim().to_ascii_lowercase();
        if lower.contains("stop") {
            return None;
        }
        let digits: String = lower
            .trim_start_matches('+')
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<usize>() {
            if n > 0 {
                return Some(n.clamp(1, 64));
            }
        }
    }
    let trimmed = custom_text.trim();
    if trimmed.eq_ignore_ascii_case("stop") {
        return None;
    }
    let custom_digits: String = trimmed
        .trim_start_matches('+')
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if let Ok(n) = custom_digits.parse::<usize>() {
        if n > 0 {
            return Some(n.clamp(1, 64));
        }
    }
    Some(default_steps.clamp(2, 64))
}

pub(crate) fn build_step_contingent_question(
    steps_used: usize,
    model_id: &str,
    agent_history_json: String,
    step_index: usize,
) -> PendingUserQuestion {
    PendingUserQuestion {
        id: format!("step_contingent::{steps_used}"),
        question: format!(
            "Reached the {steps_used}-step autonomous contingent while working on this task. Grant more auto steps to continue from where it left off?"
        ),
        options: vec![
            "+4 Steps".into(),
            "+8 Steps".into(),
            "+16 Steps".into(),
            "Stop & Summarize".into(),
        ],
        allow_multiple: false,
        model_id: model_id.to_string(),
        agent_history_json,
        step_index,
    }
}

pub async fn answer_pending_question(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    selected_options: Vec<String>,
    custom_text: String,
) -> anyhow::Result<Session> {
    ctx.store.clear_abort();
    let snapshot = ctx.store.snapshot().await;
    let sess = snapshot
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

    let pq = sess
        .pending_question
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No pending question for this session."))?;

    let workspace_path = sess.workspace_path.clone();
    let mode = sess.mode;
    let mut history: Vec<ProviderMessage> =
        serde_json::from_str(&pq.agent_history_json).unwrap_or_default();
    let configured_steps = snapshot
        .settings
        .orchestrator
        .max_autonomous_steps
        .clamp(2, 64);

    if pq.id.starts_with("step_contingent") {
        let granted = parse_granted_contingent_steps(
            &selected_options,
            &custom_text,
            configured_steps,
        );
        let allow_tools = granted.is_some();
        let (next_max_steps, status_note, user_msg_content, record_out) = match granted {
            None => (
                pq.step_index + 2,
                "Summarizing workload after autonomous contingent...".to_string(),
                "Autonomous step contingent ended by user. Do not call any tools; summarize the work completed so far and any remaining steps.".to_string(),
                "Stop & Summarize".to_string(),
            ),
            Some(extra) => {
                let note = custom_text.trim();
                let steering = if note.is_empty()
                    || note
                        .trim_start_matches('+')
                        .chars()
                        .all(|c| c.is_ascii_digit())
                {
                    format!(
                        "User granted +{extra} additional autonomous steps. Continue the workload from where you left off."
                    )
                } else {
                    format!("Continue with +{extra} autonomous steps. User note: {note}")
                };
                (
                    pq.step_index + 1 + extra,
                    format!("Continuing with +{extra} additional steps..."),
                    steering,
                    format!("+{extra} Steps"),
                )
            }
        };

        history.push(ProviderMessage {
            role: "user".into(),
            content: user_msg_content,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
            images: Vec::new(),
        });

        let record = ToolCallRecord {
            id: pq.id.clone(),
            name: "step_contingent".into(),
            arguments_json: serde_json::json!({ "question": pq.question }).to_string(),
            status: "completed".into(),
            output: record_out,
            diff: None,
            file_path: None,
            risk_tier: Some(RiskTier::SafeReadOnly),
            subagent_summary: None,
            subagent_model: None,
        };

        ctx.store
            .update(|s| {
                if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                    sx.pending_question = None;
                    sx.updated_at = Utc::now();
                }
            })
            .await?;

        let allow_web = ctx.gate.mode() == PrivacyMode::Hybrid;
        ctx.store
            .set_activity_stream(
                Some(session_id),
                "thinking",
                &status_note,
                pq.step_index + 1,
                next_max_steps,
                &pq.model_id,
                "",
                "",
                &[record.clone()],
            )
            .await;

        let res = run_agent_loop(
            ctx,
            session_id,
            Path::new(&workspace_path),
            &pq.model_id,
            mode,
            allow_web,
            allow_tools,
            snapshot.settings.ui.custom_system_prompt.clone(),
            history,
            pq.step_index + 1,
            next_max_steps,
            None,
            vec![record],
            Vec::new(),
            Uuid::new_v4().to_string(),
            "Continue workload".into(),
        )
        .await;

        ctx.store.clear_activity().await;
        return res;
    }

    let mut answer_parts = Vec::new();
    if !selected_options.is_empty() {
        answer_parts.push(format!("Selected: {}", selected_options.join(", ")));
    }
    if !custom_text.trim().is_empty() {
        answer_parts.push(format!("Note: {}", custom_text.trim()));
    }
    let answer_str = if answer_parts.is_empty() {
        "User acknowledged without adding details.".into()
    } else {
        answer_parts.join("\n")
    };

    history.push(ProviderMessage {
        role: "tool".into(),
        content: answer_str.clone(),
        tool_call_id: Some(pq.id.clone()),
        tool_name: Some("ask_user".into()),
        tool_calls: Vec::new(),
        images: Vec::new(),
    });

    let record = ToolCallRecord {
        id: pq.id.clone(),
        name: "ask_user".into(),
        arguments_json: serde_json::json!({ "question": pq.question }).to_string(),
        status: "completed".into(),
        output: answer_str,
        diff: None,
        file_path: None,
        risk_tier: Some(RiskTier::SafeReadOnly),
        subagent_summary: None,
        subagent_model: None,
    };

    ctx.store
        .update(|s| {
            if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sx.pending_question = None;
                sx.updated_at = Utc::now();
            }
        })
        .await?;

    let allow_web = ctx.gate.mode() == PrivacyMode::Hybrid;
    let next_max_steps = configured_steps.max(pq.step_index + 2);
    let res = run_agent_loop(
        ctx,
        session_id,
        Path::new(&workspace_path),
        &pq.model_id,
        mode,
        allow_web,
        true,
        snapshot.settings.ui.custom_system_prompt.clone(),
        history,
        pq.step_index + 1,
        next_max_steps,
        None,
        vec![record],
        Vec::new(),
        Uuid::new_v4().to_string(),
        "Continue after clarification".into(),
    )
    .await;

    ctx.store.clear_activity().await;
    res
}

pub(crate) fn sync_session_plan_status(
    sess: &mut Session,
    status: PlanStatus,
    edited_markdown: Option<&str>,
) -> Option<String> {
    let mut updated_md = None;
    let mut target_plan_id = None;
    if let Some(plan) = sess.active_plan.as_mut() {
        plan.status = status;
        if let Some(md) = edited_markdown {
            if !md.trim().is_empty() {
                plan.markdown = md.to_string();
            }
        }
        target_plan_id = Some(plan.id.clone());
        updated_md = Some(plan.markdown.clone());
    }
    if let Some(pid) = target_plan_id.as_deref() {
        for msg in &mut sess.messages {
            if let Some(ref mut pa) = msg.plan_artifact {
                if pa.id == pid {
                    pa.status = status;
                    if let Some(md) = edited_markdown {
                        if !md.trim().is_empty() {
                            pa.markdown = md.to_string();
                        }
                    }
                }
            }
        }
    } else if let Some(msg) = sess
        .messages
        .iter_mut()
        .rev()
        .find(|m| m.plan_artifact.is_some())
    {
        if let Some(ref mut pa) = msg.plan_artifact {
            pa.status = status;
            if let Some(md) = edited_markdown {
                if !md.trim().is_empty() {
                    pa.markdown = md.to_string();
                }
            }
            updated_md = Some(pa.markdown.clone());
        }
    }
    updated_md
}

pub(crate) fn snapshot_history_for_paused_tool(
    history: &[ProviderMessage],
    paused_tool_id: &str,
) -> Vec<ProviderMessage> {
    let mut snap = history.to_vec();
    if let Some(last_assistant) = snap
        .iter_mut()
        .rev()
        .find(|m| m.role == "assistant" && !m.tool_calls.is_empty())
    {
        if let Some(pos) = last_assistant
            .tool_calls
            .iter()
            .position(|tc| tc.id == paused_tool_id)
        {
            last_assistant.tool_calls.truncate(pos + 1);
        }
    }
    snap
}

pub async fn handle_plan_review_action(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    action: &str,
    edited_markdown: Option<String>,
    feedback: Option<String>,
) -> anyhow::Result<Session> {
    ctx.store.clear_abort();
    let snapshot = ctx.store.snapshot().await;
    let active_model = snapshot.settings.active_model_id.clone();

    if matches!(action, "execute" | "approve") {
        let edited_ref = edited_markdown.as_deref();
        let plan_md = ctx
            .store
            .update(|s| {
                if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                    sess.mode = AgentMode::Agent;
                    return sync_session_plan_status(sess, PlanStatus::Approved, edited_ref);
                }
                None
            })
            .await?
            .ok_or_else(|| anyhow::anyhow!("No active plan found to execute."))?;

        let prompt = format!(
            "The implementation plan has been reviewed and approved. Execute it step by step inside the active workspace and run `verify_workspace` at the end.\n\nApproved Plan:\n{plan_md}"
        );
        let updated = run_turn(
            ctx,
            session_id,
            &prompt,
            AgentMode::Agent,
            &active_model,
            None,
            Vec::new(),
            Vec::new(),
        )
        .await?;

        // Mark plan as executed if no approval pause happened
        let final_sess = ctx
            .store
            .update(|s| {
                if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                    if sess.pending_approval.is_none() && sess.pending_question.is_none() {
                        sync_session_plan_status(sess, PlanStatus::Executed, None);
                    }
                    return Some(sess.clone());
                }
                None
            })
            .await?
            .unwrap_or(updated);
        return Ok(final_sess);
    }

    // Request changes
    let note = feedback
        .filter(|f| !f.trim().is_empty())
        .unwrap_or_else(|| "Please revise the plan.".into());
    let edited_ref = edited_markdown.as_deref();
    ctx.store
        .update(|s| {
            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sync_session_plan_status(sess, PlanStatus::ChangesRequested, edited_ref);
            }
        })
        .await?;

    let prompt = format!(
        "Please revise the plan based on this review feedback and resubmit via `submit_plan`:\n{note}"
    );
    run_turn(
        ctx,
        session_id,
        &prompt,
        AgentMode::Plan,
        &active_model,
        None,
        Vec::new(),
        Vec::new(),
    )
    .await
}

pub(crate) fn build_pending_tool_approval(
    workspace_root: &Path,
    tc_id: &str,
    tool_name: &str,
    args: &Value,
    risk_tier: RiskTier,
    risk_reason: &str,
    model_id: &str,
    agent_history_json: String,
    step_index: usize,
) -> PendingToolApproval {
    let (diff_preview, target_path) = if tool_name == "write_file" || tool_name == "edit_file" {
        match prepare_file_change(workspace_root, tool_name, args) {
            Ok(ch) => (Some(ch.diff), Some(ch.rel_path)),
            Err(_) => (None, args["path"].as_str().map(|s| s.to_string())),
        }
    } else {
        (None, None)
    };
    PendingToolApproval {
        id: tc_id.to_string(),
        tool_name: tool_name.to_string(),
        arguments_json: args.to_string(),
        reason: format!("{risk_reason}: approve `{tool_name}`?"),
        diff_preview,
        target_path,
        model_id: model_id.to_string(),
        agent_history_json,
        step_index,
        risk_tier,
    }
}

pub(crate) fn build_pending_user_question(
    tc_id: &str,
    args: &Value,
    model_id: &str,
    agent_history_json: String,
    step_index: usize,
) -> PendingUserQuestion {
    let question = args["question"]
        .as_str()
        .unwrap_or("Please clarify how you want to proceed:")
        .to_string();
    let options: Vec<String> = args["options"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let allow_multiple = args["allow_multiple"].as_bool().unwrap_or(false);

    PendingUserQuestion {
        id: tc_id.to_string(),
        question,
        options,
        allow_multiple,
        model_id: model_id.to_string(),
        agent_history_json,
        step_index,
    }
}

pub(crate) async fn execute_tool_with_checkpoint(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    checkpoint_id: &str,
    checkpoint_title: &str,
    args: &Value,
    workspace_root: &Path,
    brave_key: &str,
    tool_name: &str,
) -> ToolExecutionResult {
    if tool_name == "write_file" || tool_name == "edit_file" {
        match prepare_file_change(workspace_root, tool_name, args) {
            Ok(change) => {
                let _ = record_file_backup_in_checkpoint(
                    ctx.store,
                    session_id,
                    checkpoint_id,
                    checkpoint_title,
                    &change.rel_path,
                    change.existed_before,
                    change.previous_content.clone(),
                    Some(change.new_content.clone()),
                    change.diff.clone(),
                )
                .await;
                return match apply_file_change(&change) {
                    Ok(res) => res,
                    Err(e) => ToolExecutionResult {
                        output: format!("Tool error ({tool_name}): {e}"),
                        diff: None,
                        file_path: Some(change.rel_path),
                        is_error: true,
                    },
                };
            }
            Err(e) => {
                return ToolExecutionResult {
                    output: format!("Tool error ({tool_name}): {e}"),
                    diff: None,
                    file_path: None,
                    is_error: true,
                };
            }
        }
    }

    if let Some(rest) = tool_name.strip_prefix("mcp__") {
        if let Some((srv_slug, raw_tool)) = rest.split_once("__") {
            let snap = ctx.store.snapshot().await;
            if let Some(entry) = snap
                .mcp_servers
                .iter()
                .find(|s| s.enabled && sanitize_mcp_slug(&s.name) == srv_slug)
            {
                return call_mcp_tool(ctx.gate, entry, raw_tool, args.clone()).await;
            }
            return ToolExecutionResult {
                output: format!(
                    "MCP server matching '{srv_slug}' is not enabled or no longer registered."
                ),
                diff: None,
                file_path: None,
                is_error: true,
            };
        }
    }

    execute_tool(workspace_root, ctx.gate, brave_key, tool_name, args).await
}

#[allow(clippy::too_many_arguments)]
async fn run_agent_loop(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    workspace_root: &Path,
    model_id: &str,
    initial_mode: AgentMode,
    allow_web: bool,
    allow_tools: bool,
    custom_sys_prompt: String,
    mut history: Vec<ProviderMessage>,
    start_step: usize,
    max_steps: usize,
    effort_override: Option<String>,
    mut collected_tools: Vec<ToolCallRecord>,
    mut mode_phases: Vec<OrchestratorPhaseRecord>,
    checkpoint_id: String,
    checkpoint_title: String,
) -> anyhow::Result<Session> {
    let snapshot = ctx.store.snapshot().await;
    let settings = snapshot.settings.clone();
    let active_plan = snapshot
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .and_then(|s| s.active_plan.clone());
    let has_approved_plan = active_plan
        .as_ref()
        .map(|p| p.status == PlanStatus::Approved)
        .unwrap_or(false);

    let mut current_mode = initial_mode;
    let mut active_model_id = model_id.to_string();
    let enabled_modules = settings.ui.enabled_modules.clone();
    let thinking_effort = effort_override.unwrap_or_else(|| settings.ui.thinking_effort.clone());
    let delegate_cloud_hint: Option<String> = if allow_web {
        let resolved = resolve_research_model(ctx, &active_model_id, true).await;
        (!resolved.starts_with("local::")).then_some(resolved)
    } else {
        None
    };
    let mut base_prompt = build_system_prompt(
        workspace_root,
        current_mode,
        active_model_id.starts_with("local::"),
        active_plan.as_ref(),
        &custom_sys_prompt,
    );
    append_hybrid_guidance(
        &mut base_prompt,
        allow_web,
        &active_model_id,
        delegate_cloud_hint.as_deref(),
    );
    let (mut enable_thinking, mut current_sys_prompt) =
        apply_thinking_effort(&base_prompt, &thinking_effort);

    // Discover live tools from enabled MCP servers once per turn
    let has_workspace = !workspace_root.as_os_str().is_empty() && workspace_root.is_dir();
    let mcp_tools = if !has_workspace || !snapshot.mcp_servers.iter().any(|s| s.enabled) {
        Vec::new()
    } else {
        discover_enabled_mcp_tools(ctx.gate, &snapshot.mcp_servers).await
    };
    let mcp_specs = mcp_tools_to_specs(&mcp_tools);

    let mut current_specs = if allow_tools {
        let mut s = active_tools_for_context(current_mode, allow_web, &enabled_modules, has_workspace);
        if has_workspace && !matches!(current_mode, AgentMode::Chat | AgentMode::Compare) {
            s.extend(mcp_specs.clone());
        }
        s
    } else {
        Vec::new()
    };

    let mut last_signature: Option<String> = None;
    let mut repeat_count: usize = 0;
    let mut self_correction_retries: usize = 0;
    let mut consecutive_error_steps: usize = 0;

    let mut total_prompt = 0u64;
    let mut total_completion = 0u64;
    let mut total_latency = 0u64;
    let mut accumulated_thoughts = Vec::new();
    let mut final_content = String::new();
    let mut last_artifact_id: Option<String> = None;
    let mut produced_checkpoint = !collected_tools.is_empty()
        && snapshot
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.checkpoints.iter().any(|c| c.id == checkpoint_id))
            .unwrap_or(false);

    for step in start_step..max_steps {
        if ctx.store.is_aborted() {
            final_content = if final_content.trim().is_empty() {
                "[Generation stopped by user]".into()
            } else {
                format!("{final_content}\n\n[Generation stopped by user]")
            };
            break;
        }

        let display_model = snapshot
            .settings
            .resolve_model_alias(&active_model_id, &active_model_id)
            .to_string();
        ctx.store
            .set_activity_stream(
                Some(session_id),
                "thinking",
                &format!(
                    "[{}] Waiting for {} (Turn {})...",
                    mode_label(current_mode).to_ascii_uppercase(),
                    display_model,
                    step + 1
                ),
                step + 1,
                max_steps,
                &active_model_id,
                &accumulated_thoughts.join("\n\n"),
                &final_content,
                &collected_tools,
            )
            .await;

        let ctx_token_budget = if active_model_id.starts_with("local::") {
            (snapshot.settings.local_ctx_size as usize)
                .saturating_sub(1536)
                .max(1536)
        } else {
            48_000
        };
        let char_budget = ctx_token_budget * 3;
        let total_chars = current_sys_prompt.len() + estimate_provider_history_chars(&history);
        if total_chars > char_budget {
            let target_hist = char_budget
                .saturating_sub(current_sys_prompt.len())
                .max(1800);
            if compact_provider_history_in_place(
                &mut history,
                (target_hist * 7) / 10,
                &collected_tools,
            ) {
                let _ = compact_session_context(ctx.store, session_id).await;
                mode_phases.push(OrchestratorPhaseRecord {
                    phase: "auto-compact".into(),
                    model_id: active_model_id.clone(),
                    summary: format!(
                        "Proactively compacted context (~{}k chars -> budget {}k)",
                        total_chars / 1000,
                        char_budget / 1000
                    ),
                    latency_ms: 0,
                    tokens: 0,
                });
                ctx.store
                    .set_activity_stream(
                        Some(session_id),
                        "thinking",
                        "Auto-compacted context mid-task to fit context window...",
                        step + 1,
                        max_steps,
                        &active_model_id,
                        &accumulated_thoughts.join("\n\n"),
                        &final_content,
                        &collected_tools,
                    )
                    .await;
            }
        }

        let req = CompletionRequest {
            model_id: active_model_id.clone(),
            system_prompt: current_sys_prompt.clone(),
            messages: history.clone(),
            tools: current_specs.clone(),
            enable_thinking,
            max_tokens: 4096,
            temperature: 0.25,
        };

        let completion_res = match complete_cancellable(ctx, req).await {
            Ok(v) => Ok(v),
            Err(e) if is_context_overflow_error(&e.to_string()) => {
                for m in &mut history {
                    if !m.images.is_empty() {
                        m.images.clear();
                        m.content
                            .push_str("\n[Image omitted after mid-task context compaction]");
                    }
                }
                let emergency_budget = (char_budget / 2)
                    .saturating_sub(current_sys_prompt.len() / 2)
                    .max(1600);
                compact_provider_history_in_place(&mut history, emergency_budget, &collected_tools);
                let _ = compact_session_context(ctx.store, session_id).await;
                mode_phases.push(OrchestratorPhaseRecord {
                    phase: "auto-compact".into(),
                    model_id: active_model_id.clone(),
                    summary: "Context limit reached — compacted history mid-task & continued".into(),
                    latency_ms: 0,
                    tokens: 0,
                });
                ctx.store
                    .set_activity_stream(
                        Some(session_id),
                        "thinking",
                        "Context window full — auto-compacted mid-task and continuing...",
                        step + 1,
                        max_steps,
                        &active_model_id,
                        &accumulated_thoughts.join("\n\n"),
                        &final_content,
                        &collected_tools,
                    )
                    .await;
                let retry_req = CompletionRequest {
                    model_id: active_model_id.clone(),
                    system_prompt: current_sys_prompt.clone(),
                    messages: history.clone(),
                    tools: current_specs.clone(),
                    enable_thinking,
                    max_tokens: 4096,
                    temperature: 0.25,
                };
                complete_cancellable(ctx, retry_req).await
            }
            Err(e) => Err(e),
        };

        let Some(resp) = completion_res? else {
            final_content = if final_content.trim().is_empty() {
                "[Generation stopped by user]".into()
            } else {
                format!("{final_content}\n\n[Generation stopped by user]")
            };
            break;
        };
        if let Some(prev_model) = resp.switched_from.as_ref() {
            if !resp.actual_model_id.is_empty() {
                active_model_id = resp.actual_model_id.clone();
            }
            let reason = resp
                .switch_reason
                .as_deref()
                .unwrap_or("unavailable / quota exhausted");
            mode_phases.push(OrchestratorPhaseRecord {
                phase: "auto-switch".into(),
                model_id: active_model_id.clone(),
                summary: format!("Switched {prev_model} → {active_model_id} ({reason})"),
                latency_ms: resp.latency_ms,
                tokens: resp.prompt_tokens + resp.completion_tokens,
            });
        }
        total_prompt += resp.prompt_tokens;
        total_completion += resp.completion_tokens;
        total_latency += resp.latency_ms;

        if resp.tool_calls.is_empty() {
            let (clean_resp_content, leaked_th) =
                harness_providers::extract_think_tags(&resp.content);
            let combined_th = match (resp.thinking.as_deref(), leaked_th.as_deref()) {
                (Some(a), Some(b)) if !a.trim().is_empty() && !b.trim().is_empty() => {
                    Some(format!("{}\n\n{}", a.trim(), b.trim()))
                }
                (Some(a), _) if !a.trim().is_empty() => Some(a.trim().to_string()),
                (_, Some(b)) if !b.trim().is_empty() => Some(b.trim().to_string()),
                _ => None,
            };

            if step == start_step
                && allow_tools
                && has_workspace
                && !matches!(current_mode, AgentMode::Chat | AgentMode::Compare)
                && collected_tools.is_empty()
                && looks_like_unexecuted_tool_intent(&clean_resp_content)
            {
                let th = combined_th.as_deref().unwrap_or("").trim();
                let ic = clean_resp_content.trim();
                let combined_plan = match (th.is_empty(), ic.is_empty()) {
                    (false, false) if !th.contains(ic) => format!("{th}\n{ic}"),
                    (false, _) => th.to_string(),
                    (true, _) => ic.to_string(),
                };
                if !combined_plan.is_empty() {
                    accumulated_thoughts.push(format!(
                        "[Step {} · Initial plan]\n{}",
                        step + 1,
                        combined_plan
                    ));
                }
                history.push(ProviderMessage {
                    role: "assistant".into(),
                    content: resp.content,
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
                history.push(ProviderMessage {
                    role: "user".into(),
                    content: "[System: You stated your plan above without invoking a tool. Invoke the planned tool call now.]".into(),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
                final_content.clear();
                ctx.store
                    .set_activity_stream(
                        Some(session_id),
                        "thinking",
                        "Planning complete — executing tool step...",
                        step + 1,
                        max_steps,
                        &active_model_id,
                        &accumulated_thoughts.join("\n\n"),
                        &final_content,
                        &collected_tools,
                    )
                    .await;
                continue;
            }

            if let Some(th) =
                format_step_thought_trace(step + 1, combined_th.as_deref(), "", &[])
            {
                accumulated_thoughts.push(th);
            }
            final_content = clean_resp_content;
            let display_model = snapshot
                .settings
                .resolve_model_alias(&active_model_id, &active_model_id)
                .to_string();
            ctx.store
                .set_activity_stream(
                    Some(session_id),
                    "streaming",
                    &format!(
                        "[{}] {} responded (Turn {})",
                        mode_label(current_mode).to_ascii_uppercase(),
                        display_model,
                        step + 1
                    ),
                    step + 1,
                    max_steps,
                    &active_model_id,
                    &accumulated_thoughts.join("\n\n"),
                    &final_content,
                    &collected_tools,
                )
                .await;
            break;
        }

        let (clean_inter_content, leaked_inter_th) =
            harness_providers::extract_think_tags(&resp.content);
        let merged_inter_th = match (resp.thinking.as_deref(), leaked_inter_th.as_deref()) {
            (Some(a), Some(b)) if !a.trim().is_empty() && !b.trim().is_empty() => {
                Some(format!("{}\n\n{}", a.trim(), b.trim()))
            }
            (Some(a), _) if !a.trim().is_empty() => Some(a.trim().to_string()),
            (_, Some(b)) if !b.trim().is_empty() => Some(b.trim().to_string()),
            _ => None,
        };
        if let Some(th) = format_step_thought_trace(
            step + 1,
            merged_inter_th.as_deref(),
            &clean_inter_content,
            &resp.tool_calls,
        ) {
            accumulated_thoughts.push(th);
        }
        final_content.clear();

        let display_model = snapshot
            .settings
            .resolve_model_alias(&active_model_id, &active_model_id)
            .to_string();
        ctx.store
            .set_activity_stream(
                Some(session_id),
                "streaming",
                &format!(
                    "[{}] {} planned tool action (Turn {})",
                    mode_label(current_mode).to_ascii_uppercase(),
                    display_model,
                    step + 1
                ),
                step + 1,
                max_steps,
                &active_model_id,
                &accumulated_thoughts.join("\n\n"),
                &final_content,
                &collected_tools,
            )
            .await;

        // Small-model loop detection
        let step_sig = resp
            .tool_calls
            .iter()
            .map(|t| format!("{}:{}", t.name, t.arguments))
            .collect::<Vec<_>>()
            .join("|");
        if last_signature.as_deref() == Some(&step_sig) {
            repeat_count += 1;
        } else {
            last_signature = Some(step_sig);
            repeat_count = 0;
        }

        if repeat_count == 1 {
            // Give the model one self-correction turn instead of aborting on the first repeat.
            history.push(ProviderMessage {
                role: "assistant".into(),
                content: resp.content.clone(),
                tool_call_id: None,
                tool_name: None,
                tool_calls: resp.tool_calls.clone(),
                images: Vec::new(),
            });
            for tc in &resp.tool_calls {
                history.push(ProviderMessage {
                    role: "tool".into(),
                    content: "Duplicate tool call skipped: you already called this tool with identical arguments. Use the previous result above and proceed to your next action or final response.".into(),
                    tool_call_id: Some(tc.id.clone()),
                    tool_name: Some(tc.name.clone()),
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
            }
            continue;
        }

        if repeat_count >= 2 {
            let summary_req = CompletionRequest {
                model_id: active_model_id.clone(),
                system_prompt: current_sys_prompt.clone(),
                messages: history.clone(),
                tools: Vec::new(),
                enable_thinking,
                max_tokens: 2048,
                temperature: 0.2,
            };
            if let Ok(Some(fin)) = complete_cancellable(ctx, summary_req).await {
                if !fin.content.trim().is_empty() {
                    final_content = fin.content;
                }
            }
            if final_content.trim().is_empty() {
                let (clean_repeat, _) = harness_providers::extract_think_tags(&resp.content);
                final_content = if !clean_repeat.trim().is_empty() {
                    clean_repeat
                } else if !collected_tools.is_empty() {
                    build_structured_work_summary(&collected_tools)
                } else {
                    "Paused agent loop: detected repeated identical tool calls. Please refine the prompt or switch to a larger model.".into()
                };
            }
            break;
        }

        history.push(ProviderMessage {
            role: "assistant".into(),
            content: resp.content.clone(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: resp.tool_calls.clone(),
            images: Vec::new(),
        });

        let mut paused_for_approval: Option<PendingToolApproval> = None;
        let mut step_executed_any = false;
        let mut step_all_errored = true;

        for tc in &resp.tool_calls {
            // 0. Intercept `switch_mode`
            if tc.name == "switch_mode" {
                let target_str = tc.arguments["target_mode"].as_str().unwrap_or("").trim();
                let Some(new_mode) = parse_target_mode(target_str) else {
                    step_executed_any = true;
                    let msg = if is_privacy_mode_keyword(target_str) {
                        "Permission denied: models cannot switch network privacy mode (Strict Local vs Hybrid). Only the user can change network privacy mode in the sidebar or Settings. `switch_mode` only changes work mode ('chat', 'plan', 'agent', 'orchestrator').".to_string()
                    } else {
                        format!(
                            "Invalid target_mode '{target_str}'. `switch_mode` only accepts work modes: 'chat', 'plan', 'agent', or 'orchestrator'."
                        )
                    };
                    collected_tools.push(ToolCallRecord {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments_json: tc.arguments.to_string(),
                        status: "error".into(),
                        output: msg.clone(),
                        diff: None,
                        file_path: None,
                        risk_tier: Some(RiskTier::SafeReadOnly),
                        subagent_summary: None,
                        subagent_model: None,
                    });
                    history.push(ProviderMessage {
                        role: "tool".into(),
                        content: msg,
                        tool_call_id: Some(tc.id.clone()),
                        tool_name: Some(tc.name.clone()),
                        tool_calls: Vec::new(),
                        images: Vec::new(),
                    });
                    continue;
                };
                let reason = tc.arguments["reason"]
                    .as_str()
                    .unwrap_or("Switching mode to fulfill task")
                    .to_string();

                if !has_workspace && new_mode != AgentMode::Chat {
                    step_executed_any = true;
                    let msg = "Cannot switch into a workspace mode because this is a standalone one-off conversation with no project folder attached. Do not run workspace or shell tools; answer directly or advise the user to open a project folder in the left sidebar.";
                    collected_tools.push(ToolCallRecord {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments_json: tc.arguments.to_string(),
                        status: "error".into(),
                        output: msg.into(),
                        diff: None,
                        file_path: None,
                        risk_tier: Some(RiskTier::SafeReadOnly),
                        subagent_summary: None,
                        subagent_model: None,
                    });
                    history.push(ProviderMessage {
                        role: "tool".into(),
                        content: msg.into(),
                        tool_call_id: Some(tc.id.clone()),
                        tool_name: Some(tc.name.clone()),
                        tool_calls: Vec::new(),
                        images: Vec::new(),
                    });
                    continue;
                }

                step_executed_any = true;
                step_all_errored = false;
                current_mode = new_mode;
                let mut base_next = build_system_prompt(
                    workspace_root,
                    current_mode,
                    active_model_id.starts_with("local::"),
                    active_plan.as_ref(),
                    &custom_sys_prompt,
                );
                append_hybrid_guidance(
                    &mut base_next,
                    allow_web,
                    &active_model_id,
                    delegate_cloud_hint.as_deref(),
                );
                let (et, sp) = apply_thinking_effort(&base_next, &thinking_effort);
                enable_thinking = et;
                current_sys_prompt = sp;
                current_specs = active_tools_for_context(
                    current_mode,
                    allow_web,
                    &enabled_modules,
                    has_workspace,
                );
                if has_workspace && !matches!(current_mode, AgentMode::Chat | AgentMode::Compare) {
                    current_specs.extend(mcp_specs.clone());
                }

                let _ = ctx
                    .store
                    .update(|s| {
                        if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                            sess.mode = current_mode;
                            sess.updated_at = Utc::now();
                        }
                    })
                    .await;

                let phase_label = format!("mode → {}", mode_label(current_mode));
                let out_msg = format!("Switched mode to {:?}: {reason}", current_mode);
                mode_phases.push(OrchestratorPhaseRecord {
                    phase: phase_label,
                    model_id: model_id.to_string(),
                    summary: reason,
                    latency_ms: resp.latency_ms,
                    tokens: resp.prompt_tokens + resp.completion_tokens,
                });
                collected_tools.push(ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments_json: tc.arguments.to_string(),
                    status: "completed".into(),
                    output: out_msg,
                    diff: None,
                    file_path: None,
                    risk_tier: Some(RiskTier::SafeReadOnly),
                    subagent_summary: None,
                    subagent_model: None,
                });
                let canon_label = mode_label(current_mode);
                history.push(ProviderMessage {
                    role: "tool".into(),
                    content: format!(
                        "Work mode switched to '{canon_label}'. Tools for {canon_label} mode are now active; proceed with the task."
                    ),
                    tool_call_id: Some(tc.id.clone()),
                    tool_name: Some(tc.name.clone()),
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
                continue;
            }

            // 0b. Intercept `write_artifact`
            if tc.name == "write_artifact" {
                let (rec, msg_txt) = upsert_artifact_from_tool(ctx.store, session_id, tc).await;
                if let Some(art_id) = msg_txt.split("id=").nth(1).and_then(|s| s.strip_suffix(").")) {
                    last_artifact_id = Some(art_id.to_string());
                }
                step_executed_any = true;
                if rec.status != "error" {
                    step_all_errored = false;
                }
                collected_tools.push(rec);
                history.push(ProviderMessage {
                    role: "tool".into(),
                    content: msg_txt,
                    tool_call_id: Some(tc.id.clone()),
                    tool_name: Some(tc.name.clone()),
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
                continue;
            }

            // 1. Intercept `submit_plan`
            if tc.name == "submit_plan" {
                let plan = build_plan_artifact(model_id, &tc.arguments);
                let plan_art_id = upsert_session_artifact(
                    ctx.store,
                    session_id,
                    &plan.title,
                    "plan",
                    &plan.markdown,
                )
                .await
                .ok()
                .map(|a| a.id);
                let plan_summary = if resp.content.trim().is_empty() {
                    format!("Submitted implementation plan **{}** for review.", plan.title)
                } else {
                    resp.content.clone()
                };
                collected_tools.push(ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments_json: tc.arguments.to_string(),
                    status: "completed".into(),
                    output: format!("Plan '{}' awaits user review.", plan.title),
                    diff: None,
                    file_path: None,
                    risk_tier: Some(RiskTier::SafeReadOnly),
                    subagent_summary: None,
                    subagent_model: None,
                });

                let msg = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "assistant".into(),
                    content: plan_summary,
                    thinking: if accumulated_thoughts.is_empty() {
                        None
                    } else {
                        Some(accumulated_thoughts.join("\n\n"))
                    },
                    model_id: Some(model_id.to_string()),
                    latency_ms: Some(total_latency),
                    prompt_tokens: Some(total_prompt),
                    completion_tokens: Some(total_completion),
                    tool_calls: collected_tools,
                    compare_partner: None,
                    checkpoint_id: None,
                    plan_artifact: Some(plan.clone()),
                    orchestrator_phases: mode_phases,
                    attached_files: Vec::new(),
                    attachments: Vec::new(),
                    artifact_id: plan_art_id.or(last_artifact_id),
                    created_at: Utc::now(),
                };

                let updated = ctx
                    .store
                    .update(|s| {
                        if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                            sess.active_plan = Some(plan);
                            sess.messages.push(msg);
                            sess.updated_at = Utc::now();
                            return Some(sess.clone());
                        }
                        None
                    })
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
                return Ok(updated);
            }

            // 2. Intercept `ask_user`
            if tc.name == "ask_user" {
                let snap_hist = snapshot_history_for_paused_tool(&history, &tc.id);
                let history_json = serde_json::to_string(&snap_hist)?;
                let pq = build_pending_user_question(
                    &tc.id,
                    &tc.arguments,
                    model_id,
                    history_json,
                    step,
                );
                let question = pq.question.clone();

                let msg = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "assistant".into(),
                    content: if resp.content.trim().is_empty() {
                        format!("Clarification needed: {question}")
                    } else {
                        resp.content
                    },
                    thinking: if accumulated_thoughts.is_empty() {
                        None
                    } else {
                        Some(accumulated_thoughts.join("\n\n"))
                    },
                    model_id: Some(model_id.to_string()),
                    latency_ms: Some(total_latency),
                    prompt_tokens: Some(total_prompt),
                    completion_tokens: Some(total_completion),
                    tool_calls: collected_tools,
                    compare_partner: None,
                    checkpoint_id: None,
                    plan_artifact: None,
                    orchestrator_phases: mode_phases,
                    attached_files: Vec::new(),
                    attachments: Vec::new(),
                    artifact_id: last_artifact_id,
                    created_at: Utc::now(),
                };

                let updated = ctx
                    .store
                    .update(|s| {
                        if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                            sess.pending_question = Some(pq);
                            sess.messages.push(msg);
                            sess.updated_at = Utc::now();
                            return Some(sess.clone());
                        }
                        None
                    })
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
                return Ok(updated);
            }

            // 3. Intercept `delegate_research`
            if tc.name == "delegate_research" {
                let (rec, tool_msg, phase) = handle_delegate_research_tool(
                    ctx,
                    session_id,
                    workspace_root,
                    &active_model_id,
                    tc,
                )
                .await;
                step_executed_any = true;
                if rec.status != "error" {
                    step_all_errored = false;
                }
                collected_tools.push(rec);
                mode_phases.push(phase);
                history.push(tool_msg);
                continue;
            }

            // 3b. Intercept `delegate_vision`
            if tc.name == "delegate_vision" {
                let (rec, tool_msg, phase) =
                    handle_delegate_vision_tool(ctx, session_id, workspace_root, tc).await;
                step_executed_any = true;
                if rec.status != "error" {
                    step_all_errored = false;
                }
                collected_tools.push(rec);
                mode_phases.push(phase);
                history.push(tool_msg);
                continue;
            }

            // 4. Smart Risk-Tier Classification (including live MCP tools)
            let (risk_tier, risk_reason) = if tc.name.starts_with("mcp__") {
                let is_ro = mcp_tools
                    .iter()
                    .find(|m| m.namespaced_name == tc.name)
                    .map(|m| m.read_only)
                    .unwrap_or(false);
                if is_ro {
                    (RiskTier::SafeReadOnly, "Read-only MCP tool".into())
                } else {
                    (
                        RiskTier::WorkspaceMutation,
                        "External MCP tool with potential side effects".into(),
                    )
                }
            } else {
                classify_tool_risk(&tc.name, &tc.arguments)
            };
            let needs_approval = match risk_tier {
                RiskTier::SafeReadOnly => false,
                RiskTier::HighRiskDestructive => true,
                RiskTier::WorkspaceMutation => match tc.name.as_str() {
                    "write_file" | "edit_file" => !(settings.auto_approve_edits || has_approved_plan),
                    "run_command" | "verify_workspace" => !settings.auto_approve_commands,
                    name if name.starts_with("mcp__") => !settings.auto_approve_commands,
                    _ => false,
                },
            };

            if needs_approval {
                let snap_hist = snapshot_history_for_paused_tool(&history, &tc.id);
                let history_json = serde_json::to_string(&snap_hist)?;
                paused_for_approval = Some(build_pending_tool_approval(
                    workspace_root,
                    &tc.id,
                    &tc.name,
                    &tc.arguments,
                    risk_tier,
                    &risk_reason,
                    &active_model_id,
                    history_json,
                    step,
                ));
                break;
            }

            let intent_detail = describe_tool_intent_short(&tc.name, &tc.arguments);
            ctx.store
                .set_activity_stream(
                    Some(session_id),
                    "tool",
                    &format!("Running: {intent_detail}..."),
                    step + 1,
                    max_steps,
                    &active_model_id,
                    &accumulated_thoughts.join("\n\n"),
                    &final_content,
                    &collected_tools,
                )
                .await;

            let mut exec_res = execute_tool_with_checkpoint(
                ctx,
                session_id,
                &checkpoint_id,
                &checkpoint_title,
                &tc.arguments,
                workspace_root,
                &settings.api_keys.brave_search_api_key,
                &tc.name,
            )
            .await;

            if (tc.name == "write_file" || tc.name == "edit_file") && !exec_res.is_error {
                produced_checkpoint = true;
            }

            // Self-correction hint when verification/command fails (skip when blocked by missing workspace / sandbox)
            let is_env_block = exec_res.output.contains("No project workspace")
                || exec_res.output.contains("No valid project workspace")
                || exec_res.output.contains("Sandbox policy blocked")
                || exec_res.output.contains("os error 123");
            if (tc.name == "verify_workspace" || tc.name == "run_command")
                && exec_res.is_error
                && !is_env_block
            {
                if self_correction_retries < 2 {
                    self_correction_retries += 1;
                    exec_res.output.push_str(&format!(
                        "\n\n[Auto-Verify Notice (retry {self_correction_retries}/2): inspect the error above, fix the issue in the workspace, and re-run verification.]"
                    ));
                }
            }

            step_executed_any = true;
            if !exec_res.is_error {
                step_all_errored = false;
            }

            collected_tools.push(ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments_json: tc.arguments.to_string(),
                status: if exec_res.is_error {
                    "error".into()
                } else {
                    "completed".into()
                },
                output: exec_res.output.clone(),
                diff: exec_res.diff.clone(),
                file_path: exec_res.file_path.clone(),
                risk_tier: Some(risk_tier),
                subagent_summary: None,
                subagent_model: None,
            });

            history.push(ProviderMessage {
                role: "tool".into(),
                content: exec_res.output,
                tool_call_id: Some(tc.id.clone()),
                tool_name: Some(tc.name.clone()),
                tool_calls: Vec::new(),
                images: Vec::new(),
            });
            ctx.store
                .set_activity_stream(
                    Some(session_id),
                    "tool",
                    &format!("Finished tool `{}`", tc.name),
                    step + 1,
                    max_steps,
                    model_id,
                    &accumulated_thoughts.join("\n\n"),
                    &final_content,
                    &collected_tools,
                )
                .await;
        }

        if let Some(pending) = paused_for_approval {
            let partial_msg = ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: "assistant".into(),
                content: if resp.content.trim().is_empty() {
                    format!("Waiting for your review to execute `{}`.", pending.tool_name)
                } else {
                    resp.content
                },
                thinking: if accumulated_thoughts.is_empty() {
                    None
                } else {
                    Some(accumulated_thoughts.join("\n\n"))
                },
                model_id: Some(active_model_id.clone()),
                latency_ms: Some(total_latency),
                prompt_tokens: Some(total_prompt),
                completion_tokens: Some(total_completion),
                tool_calls: collected_tools,
                compare_partner: None,
                checkpoint_id: if produced_checkpoint {
                    Some(checkpoint_id)
                } else {
                    None
                },
                plan_artifact: None,
                orchestrator_phases: mode_phases.clone(),
                attached_files: Vec::new(),
                attachments: Vec::new(),
                artifact_id: last_artifact_id,
                created_at: Utc::now(),
            };

            let updated = ctx
                .store
                .update(|s| {
                    if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                        sess.messages.push(partial_msg);
                        sess.pending_approval = Some(pending);
                        sess.updated_at = Utc::now();
                        return Some(sess.clone());
                    }
                    None
                })
                .await?
                .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
            return Ok(updated);
        }

        if step_executed_any && step_all_errored {
            consecutive_error_steps += 1;
        } else if step_executed_any {
            consecutive_error_steps = 0;
        }
        if consecutive_error_steps >= 3 {
            let summary_req = CompletionRequest {
                model_id: active_model_id.clone(),
                system_prompt: current_sys_prompt.clone(),
                messages: history.clone(),
                tools: Vec::new(),
                enable_thinking,
                max_tokens: 2048,
                temperature: 0.2,
            };
            if let Ok(Some(fin)) = complete_cancellable(ctx, summary_req).await {
                if !fin.content.trim().is_empty() {
                    final_content = fin.content;
                }
            }
            if final_content.trim().is_empty() {
                final_content = "Stopped retrying after 3 consecutive failing tool steps. Inspect the tool errors above or attach a valid project workspace.".into();
            }
            break;
        }

        if step == max_steps - 1 {
            let pq = build_step_contingent_question(
                max_steps,
                &active_model_id,
                serde_json::to_string(&history)?,
                step,
            );
            let pause_msg = ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: "assistant".into(),
                content: if final_content.trim().is_empty() {
                    format!(
                        "Paused after {max_steps} autonomous steps — select additional step contingent below to continue the workload."
                    )
                } else {
                    format!(
                        "{final_content}\n\nPaused after {max_steps} autonomous steps — select additional step contingent below to continue the workload."
                    )
                },
                thinking: if accumulated_thoughts.is_empty() {
                    None
                } else {
                    Some(accumulated_thoughts.join("\n\n"))
                },
                model_id: Some(active_model_id.clone()),
                latency_ms: Some(total_latency),
                prompt_tokens: Some(total_prompt),
                completion_tokens: Some(total_completion),
                tool_calls: collected_tools,
                compare_partner: None,
                checkpoint_id: if produced_checkpoint {
                    Some(checkpoint_id)
                } else {
                    None
                },
                plan_artifact: None,
                orchestrator_phases: mode_phases,
                attached_files: Vec::new(),
                attachments: Vec::new(),
                artifact_id: last_artifact_id,
                created_at: Utc::now(),
            };
            let updated = ctx
                .store
                .update(|s| {
                    if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                        sess.pending_question = Some(pq);
                        sess.messages.push(pause_msg);
                        sess.updated_at = Utc::now();
                        return Some(sess.clone());
                    }
                    None
                })
                .await?
                .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
            return Ok(updated);
        }
    }

    if !collected_tools.is_empty() && final_content.trim().is_empty() && !ctx.store.is_aborted() {
        ctx.store
            .set_activity_stream(
                Some(session_id),
                "summarizing",
                "Writing concise summary of work & reasoning...",
                max_steps,
                max_steps,
                &active_model_id,
                &accumulated_thoughts.join("\n\n"),
                "",
                &collected_tools,
            )
            .await;
        history.push(ProviderMessage {
            role: "user".into(),
            content: "All tool steps above are complete. Write a concise, structured Markdown summary for the user covering:\n1. **What Was Done** (referencing inspected/modified file paths and symbols in backticks)\n2. **Reasoning & Key Decisions** (why changes were made or what root cause was found)\n3. **Verification & Status**.\nDo not output `<think>` tags or raw internal monologue.".into(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
            images: Vec::new(),
        });
        let summary_req = CompletionRequest {
            model_id: active_model_id.clone(),
            system_prompt: current_sys_prompt.clone(),
            messages: history.clone(),
            tools: Vec::new(),
            enable_thinking: false,
            max_tokens: 1536,
            temperature: 0.2,
        };
        if let Ok(Some(fin)) = complete_cancellable(ctx, summary_req).await {
            total_prompt += fin.prompt_tokens;
            total_completion += fin.completion_tokens;
            total_latency += fin.latency_ms;
            let (clean_fin, leaked) = harness_providers::extract_think_tags(&fin.content);
            if let Some(th) = fin.thinking.or(leaked) {
                let trimmed_th = th.trim();
                if !trimmed_th.is_empty() {
                    accumulated_thoughts.push(format!("[Step Summary · Thought]\n{trimmed_th}"));
                }
            }
            if !clean_fin.trim().is_empty() {
                final_content = clean_fin;
            }
        }
        if final_content.trim().is_empty() {
            final_content = build_structured_work_summary(&collected_tools);
        }
    }

    let assistant_msg = ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content: final_content,
        thinking: if accumulated_thoughts.is_empty() {
            None
        } else {
            Some(accumulated_thoughts.join("\n\n"))
        },
        model_id: Some(active_model_id),
        latency_ms: Some(total_latency),
        prompt_tokens: Some(total_prompt),
        completion_tokens: Some(total_completion),
        tool_calls: collected_tools,
        compare_partner: None,
        checkpoint_id: if produced_checkpoint {
            Some(checkpoint_id)
        } else {
            None
        },
        plan_artifact: None,
        orchestrator_phases: mode_phases,
        attached_files: Vec::new(),
        attachments: Vec::new(),
        artifact_id: last_artifact_id,
        created_at: Utc::now(),
    };

    let updated = ctx
        .store
        .update(|s| {
            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sess.messages.push(assistant_msg);
                sess.updated_at = Utc::now();
                return Some(sess.clone());
            }
            None
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

    Ok(updated)
}

pub(crate) fn build_plan_artifact(model_id: &str, args: &Value) -> PlanArtifact {
    let title = args["title"]
        .as_str()
        .unwrap_or("Implementation Plan")
        .to_string();
    let summary = args["summary"].as_str().unwrap_or("").to_string();
    let mut steps = Vec::new();

    if let Some(arr) = args["steps"].as_array() {
        for (i, st) in arr.iter().enumerate() {
            let step_number = st["step_number"].as_u64().unwrap_or((i + 1) as u64) as usize;
            let description = st["description"].as_str().unwrap_or("").to_string();
            let files_touched: Vec<String> = st["files_touched"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let verification = st["verification"].as_str().map(|s| s.to_string());
            steps.push(PlanStep {
                step_number,
                description,
                files_touched,
                verification,
            });
        }
    }

    let mut md = format!("# {title}\n\n{summary}\n\n## Steps\n");
    for s in &steps {
        md.push_str(&format!("{}. {}\n", s.step_number, s.description));
        if !s.files_touched.is_empty() {
            md.push_str(&format!("   - Files: `{}`\n", s.files_touched.join("`, `")));
        }
        if let Some(ref v) = s.verification {
            md.push_str(&format!("   - Verify: `{v}`\n"));
        }
    }

    PlanArtifact {
        id: Uuid::new_v4().to_string(),
        title,
        summary,
        steps,
        markdown: md,
        status: PlanStatus::AwaitingReview,
        model_id: model_id.to_string(),
        created_at: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_posture_routes_chat_plan_and_agent() {
        let (m1, _) = classify_auto_posture("Explain what AES-GCM is.", &[], None);
        assert_eq!(m1, AgentMode::Chat);

        let (m2, _) = classify_auto_posture(
            "Architect and create a plan for refactoring auth before coding.",
            &[],
            None,
        );
        assert_eq!(m2, AgentMode::Plan);

        let (m3, _) = classify_auto_posture("Fix the bug in src/main.rs and run tests.", &[], None);
        assert_eq!(m3, AgentMode::Agent);
    }

    #[test]
    fn module_filtering_respects_enabled_flags() {
        let filtered = active_tools_for_mode(
            AgentMode::Agent,
            true,
            &["fs_read".into()],
        );
        assert!(filtered.iter().any(|t| t.name == "read_file"));
        assert!(!filtered.iter().any(|t| t.name == "write_file"));
        assert!(!filtered.iter().any(|t| t.name == "run_command"));
        assert!(!filtered.iter().any(|t| t.name == "web_search"));
        assert!(filtered.iter().any(|t| t.name == "switch_mode"));
        assert!(filtered.iter().any(|t| t.name == "write_artifact"));
    }

    #[test]
    fn compact_context_condenses_old_messages() {
        let dir = std::env::temp_dir().join(format!("bumbo_compact_test_{}", Uuid::new_v4()));
        let paths = harness_store::BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sid = store
                .update(|s| {
                    let sid = s.settings.active_session_id.clone();
                    let sess = s.sessions.iter_mut().find(|x| x.id == sid).unwrap();
                    for i in 0..8 {
                        sess.messages.push(ChatMessage {
                            id: Uuid::new_v4().to_string(),
                            role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                            content: format!("Message number {i}"),
                            thinking: None,
                            model_id: Some("gemini-3-flash-preview".into()),
                            latency_ms: Some(10),
                            prompt_tokens: Some(5),
                            completion_tokens: Some(5),
                            tool_calls: Vec::new(),
                            compare_partner: None,
                            checkpoint_id: None,
                            plan_artifact: None,
                            orchestrator_phases: Vec::new(),
                            attached_files: Vec::new(),
                            attachments: Vec::new(),
                            artifact_id: None,
                            created_at: Utc::now(),
                        });
                    }
                    sid
                })
                .await
                .unwrap();

            let compacted = compact_session_context(&store, &sid).await.unwrap();
            assert_eq!(compacted.messages.len(), 5);
            assert!(compacted.messages[0].content.contains("Compacted Context Summary"));
            assert_eq!(compacted.messages[4].content, "Message number 7");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mid_task_auto_compaction_shrinks_overflowing_history_and_detects_400_overflow() {
        assert!(is_context_overflow_error(
            "Endpoint http://127.0.0.1:18080/v1/chat/completions returned HTTP 400 Bad Request: request (570158 tokens) exceeds the available context size (16384 tokens)"
        ));
        assert!(!is_context_overflow_error("HTTP 500 internal error"));

        let huge_blob = "X".repeat(25_000);
        let mut history = vec![
            ProviderMessage {
                role: "user".into(),
                content: "Analyze this large lecture".into(),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
                images: vec![("image/png".into(), "AAAA".into())],
            },
            ProviderMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_call_id: None,
                tool_name: None,
                tool_calls: vec![ToolCallRequest {
                    id: "call_1".into(),
                    name: "read_file".into(),
                    arguments: serde_json::json!({ "path": "notes.md" }),
                    thought_signature: None,
                }],
                images: Vec::new(),
            },
            ProviderMessage {
                role: "tool".into(),
                content: huge_blob.clone(),
                tool_call_id: Some("call_1".into()),
                tool_name: Some("read_file".into()),
                tool_calls: Vec::new(),
                images: Vec::new(),
            },
            ProviderMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_call_id: None,
                tool_name: None,
                tool_calls: vec![ToolCallRequest {
                    id: "call_2".into(),
                    name: "search_files".into(),
                    arguments: serde_json::json!({ "pattern": "*.md" }),
                    thought_signature: None,
                }],
                images: Vec::new(),
            },
            ProviderMessage {
                role: "tool".into(),
                content: "Recent tool result".into(),
                tool_call_id: Some("call_2".into()),
                tool_name: Some("search_files".into()),
                tool_calls: Vec::new(),
                images: Vec::new(),
            },
        ];

        let changed = compact_provider_history_in_place(&mut history, 6_000, &[]);
        assert!(changed);
        assert!(estimate_provider_history_chars(&history) <= 6_500);
        assert!(history[0].images.is_empty());
        // Must never leave an orphaned `tool` message as the first non-user entry after checkpoint
        assert_ne!(history[1].role, "tool");
    }

    #[test]
    fn standalone_prompt_omits_workspace_rules() {
        let prompt = build_system_prompt(
            Path::new(""),
            AgentMode::Chat,
            false,
            None,
            "Be concise.",
        );
        assert!(prompt.contains("Standalone one-off conversation (no project workspace attached)"));
        assert!(!prompt.contains("Active workspace:"));
        assert!(!prompt.contains("Project Rules (.cinder/rules.md):"));
        assert!(prompt.contains("Be concise."));
    }

    #[test]
    fn parses_granted_contingent_steps_accurately() {
        assert_eq!(
            parse_granted_contingent_steps(&["+4 Steps".into()], "", 8),
            Some(4)
        );
        assert_eq!(
            parse_granted_contingent_steps(&["+16 Steps".into()], "", 8),
            Some(16)
        );
        assert_eq!(
            parse_granted_contingent_steps(&["Stop & Summarize".into()], "", 8),
            None
        );
        assert_eq!(parse_granted_contingent_steps(&[], "12", 8), Some(12));
        assert_eq!(parse_granted_contingent_steps(&[], "+20", 8), Some(20));
        assert_eq!(parse_granted_contingent_steps(&[], "stop", 8), None);
    }

    #[test]
    #[ignore = "Requires local user state.json with configured Gemini API key and network access"]
    fn live_gemini_sse_complete_and_sanitize_state() {
        let live_state_path =
            harness_store::BumboPaths::discover().state_file;
        assert!(
            live_state_path.exists(),
            "Expected state.json at {}",
            live_state_path.display()
        );
        let dir = std::env::temp_dir().join(format!("bumbo_live_sse_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let temp_state = dir.join("state.json");
        std::fs::copy(&live_state_path, &temp_state).unwrap();

        let paths = harness_store::BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: temp_state,
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let gate = NetworkGate::new(PrivacyMode::Hybrid).unwrap();
        let server_mgr = ServerManager::new(dir.join("engines"));
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let snap = store.snapshot().await;
            assert_eq!(snap.settings.active_model_id, "gemini-3-flash-preview");
            assert!(
                !snap
                    .verified_provider_models
                    .contains_key("open_ai_compat"),
                "stale open_ai_compat entry should be sanitized"
            );

            let sid = store
                .update(|s| {
                    let id = Uuid::new_v4().to_string();
                    let now = Utc::now();
                    s.sessions.insert(
                        0,
                        Session {
                            id: id.clone(),
                            title: "Live SSE Check".into(),
                            workspace_path: String::new(),
                            mode: AgentMode::Auto,
                            messages: Vec::new(),
                            pending_approval: None,
                            active_plan: None,
                            pending_question: None,
                            checkpoints: Vec::new(),
                            artifacts: Vec::new(),
                            created_at: now,
                            updated_at: now,
                        },
                    );
                    id
                })
                .await
                .unwrap();

            let ctx = HarnessContext {
                gate: &gate,
                store: &store,
                server_mgr: &server_mgr,
                local_models: &[],
            };
            let t0 = std::time::Instant::now();
            let sess = run_turn(
                &ctx,
                &sid,
                "Explain Rust ownership step by step in 3 concise bullet points.",
                AgentMode::Auto,
                "gemini-3-flash-preview",
                None,
                Vec::new(),
                Vec::new(),
            )
            .await
            .expect("live run_turn failed");
            let elapsed = t0.elapsed();
            let last = sess.messages.last().expect("missing assistant message");
            println!(
                "LIVE SSE elapsed={:?}, model={:?}, len={}, content={:?}",
                elapsed,
                last.model_id,
                last.content.len(),
                last.content
            );
            assert!(!last.content.trim().is_empty());
            assert!(
                elapsed.as_secs_f32() < 12.0,
                "Expected SSE response quickly, took {:?}",
                elapsed
            );

            // Also verify sync_google_models returns prioritized viable models
            let key = snap.settings.api_keys.gemini_api_key.clone();
            let synced = harness_providers::sync_google_models(&gate, &key)
                .await
                .expect("sync_google_models failed");
            println!(
                "SYNCED MODELS ({}): {:?}",
                synced.len(),
                synced.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
            );
            assert!(!synced.is_empty());
            assert_eq!(synced[0].id, "gemini-3-flash-preview");
            assert!(!synced.iter().any(|m| m.id == "gemini-3.7-flash"));
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delegate_vision_is_exposed_in_chat_plan_agent_and_gated_by_subagents_module() {
        for mode in [
            AgentMode::Chat,
            AgentMode::Plan,
            AgentMode::Agent,
            AgentMode::Orchestrator,
            AgentMode::Auto,
        ] {
            let specs = tool_specs(mode, false);
            assert!(
                specs.iter().any(|t| t.name == "delegate_vision"),
                "Expected delegate_vision in {mode:?}"
            );
            let with_subagents = filter_tools_by_modules(specs.clone(), &["subagents".into()]);
            assert!(with_subagents.iter().any(|t| t.name == "delegate_vision"));
            let without_subagents = filter_tools_by_modules(specs, &["fs_read".into()]);
            assert!(!without_subagents.iter().any(|t| t.name == "delegate_vision"));
        }
        assert!(tool_specs(AgentMode::Compare, false).is_empty());
    }

    #[test]
    fn resolve_vision_model_prefers_local_mmproj_in_strict_local_and_explicit_override() {
        use harness_models::{FitTier, LocalModelSource, VramFitAssessment};
        let dir = std::env::temp_dir().join(format!("bumbo_vision_test_{}", Uuid::new_v4()));
        let paths = harness_store::BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let gate = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let srv = ServerManager::new(dir.join("engines"));
        let make_fit = || VramFitAssessment {
            tier: FitTier::FitsInVram,
            label: "Fits".into(),
            weights_mb: 4000,
            kv_cache_mb: 512,
            compute_buffer_mb: 256,
            estimated_total_mb: 4768,
            recommended_ngl: 99,
        };
        let locals = vec![
            LocalModelEntry {
                id: "local::coder:7b".into(),
                name: "coder:7b".into(),
                source: LocalModelSource::VaultGguf,
                gguf_path: "coder.gguf".into(),
                ollama_tag: None,
                mmproj_path: None,
                size_bytes: 4_000_000_000,
                quant: "Q4_K_M".into(),
                is_embedding: false,
                is_valid_gguf: true,
                vram_fit: make_fit(),
            },
            LocalModelEntry {
                id: "local::gemma4:12b".into(),
                name: "gemma4:12b".into(),
                source: LocalModelSource::OllamaBlob,
                gguf_path: "gemma4.blob".into(),
                ollama_tag: Some("gemma4:12b".into()),
                mmproj_path: Some("mmproj.blob".into()),
                size_bytes: 7_000_000_000,
                quant: "GGUF Blob".into(),
                is_embedding: false,
                is_valid_gguf: true,
                vram_fit: make_fit(),
            },
        ];

        let ctx = HarnessContext {
            gate: &gate,
            store: &store,
            server_mgr: &srv,
            local_models: &locals,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let auto_pick = resolve_vision_model(&ctx).await.unwrap();
            assert_eq!(auto_pick, "local::gemma4:12b");

            store
                .update(|s| {
                    s.settings.orchestrator.vision_model_id = "local::coder:7b".into();
                })
                .await
                .unwrap();
            let override_pick = resolve_vision_model(&ctx).await.unwrap();
            assert_eq!(override_pick, "local::coder:7b");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn standalone_context_omits_workspace_tools_and_switch_mode() {
        let enabled = vec![
            "fs_read".into(),
            "fs_write".into(),
            "shell".into(),
            "rag".into(),
            "subagents".into(),
            "web".into(),
        ];
        for mode in [AgentMode::Chat, AgentMode::Auto, AgentMode::Plan, AgentMode::Agent] {
            let standalone = active_tools_for_context(mode, true, &enabled, false);
            assert!(
                !standalone.iter().any(|t| matches!(
                    t.name.as_str(),
                    "switch_mode" | "list_dir" | "read_file" | "run_command" | "verify_workspace"
                )),
                "Standalone {mode:?} leaked workspace/switch_mode tool"
            );
            assert!(
                standalone.iter().any(|t| t.name == "delegate_vision"),
                "Standalone {mode:?} should still keep delegate_vision"
            );
        }
        let with_ws = active_tools_for_context(AgentMode::Auto, false, &enabled, true);
        assert!(with_ws.iter().any(|t| t.name == "switch_mode"));
        assert!(with_ws.iter().any(|t| t.name == "list_dir"));
        assert!(with_ws.iter().any(|t| t.name == "run_command"));
    }

    #[test]
    fn switch_mode_parser_allows_only_work_modes_and_rejects_privacy_modes() {
        assert_eq!(parse_target_mode("chat"), Some(AgentMode::Chat));
        assert_eq!(parse_target_mode("PLAN"), Some(AgentMode::Plan));
        assert_eq!(parse_target_mode("agent"), Some(AgentMode::Agent));
        assert_eq!(
            parse_target_mode("orchestrator"),
            Some(AgentMode::Orchestrator)
        );
        for bad in ["hybrid", "local", "strict_local", "strict-local", "online", "offline", "bogus", ""] {
            assert_eq!(parse_target_mode(bad), None, "should reject '{bad}'");
        }
        assert!(is_privacy_mode_keyword("hybrid"));
        assert!(is_privacy_mode_keyword("Strict_Local"));
        assert!(is_privacy_mode_keyword("local"));
        assert!(!is_privacy_mode_keyword("agent"));
    }

    #[test]
    fn snapshot_history_for_paused_tool_truncates_unexecuted_sibling_calls() {
        let hist = vec![
            ProviderMessage {
                role: "user".into(),
                content: "Do things".into(),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
                images: Vec::new(),
            },
            ProviderMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_call_id: None,
                tool_name: None,
                tool_calls: vec![
                    harness_providers::ToolCallRequest {
                        id: "tc_1".into(),
                        name: "read_file".into(),
                        arguments: serde_json::json!({ "path": "a.txt" }),
                        thought_signature: None,
                    },
                    harness_providers::ToolCallRequest {
                        id: "tc_2".into(),
                        name: "ask_user".into(),
                        arguments: serde_json::json!({ "question": "Proceed?" }),
                        thought_signature: None,
                    },
                    harness_providers::ToolCallRequest {
                        id: "tc_3".into(),
                        name: "write_file".into(),
                        arguments: serde_json::json!({ "path": "b.txt", "content": "hi" }),
                        thought_signature: None,
                    },
                ],
                images: Vec::new(),
            },
            ProviderMessage {
                role: "tool".into(),
                content: "file contents".into(),
                tool_call_id: Some("tc_1".into()),
                tool_name: Some("read_file".into()),
                tool_calls: Vec::new(),
                images: Vec::new(),
            },
        ];

        let snapped = snapshot_history_for_paused_tool(&hist, "tc_2");
        assert_eq!(snapped[1].tool_calls.len(), 2);
        assert_eq!(snapped[1].tool_calls[0].id, "tc_1");
        assert_eq!(snapped[1].tool_calls[1].id, "tc_2");
    }

    #[test]
    fn sync_session_plan_status_updates_active_plan_and_chat_messages() {
        use harness_store::{PlanArtifact, PlanStatus};
        let now = Utc::now();
        let plan = PlanArtifact {
            id: "plan_42".into(),
            title: "Test Plan".into(),
            summary: "Summary".into(),
            steps: Vec::new(),
            markdown: "## Original".into(),
            status: PlanStatus::AwaitingReview,
            model_id: "gemini".into(),
            created_at: now,
        };
        let mut sess = Session {
            id: "sess_1".into(),
            title: "Plan Test".into(),
            workspace_path: String::new(),
            mode: AgentMode::Plan,
            messages: vec![ChatMessage {
                id: "m1".into(),
                role: "assistant".into(),
                content: "Submitted plan".into(),
                thinking: None,
                model_id: None,
                latency_ms: None,
                prompt_tokens: None,
                completion_tokens: None,
                tool_calls: Vec::new(),
                compare_partner: None,
                checkpoint_id: None,
                plan_artifact: Some(plan.clone()),
                orchestrator_phases: Vec::new(),
                attached_files: Vec::new(),
                attachments: Vec::new(),
                artifact_id: None,
                created_at: now,
            }],
            pending_approval: None,
            active_plan: Some(plan),
            pending_question: None,
            checkpoints: Vec::new(),
            artifacts: Vec::new(),
            created_at: now,
            updated_at: now,
        };

        let md = sync_session_plan_status(&mut sess, PlanStatus::Approved, Some("## Edited"))
            .unwrap();
        assert_eq!(md, "## Edited");
        assert_eq!(sess.active_plan.as_ref().unwrap().status, PlanStatus::Approved);
        assert_eq!(sess.active_plan.as_ref().unwrap().markdown, "## Edited");
        let msg_plan = sess.messages[0].plan_artifact.as_ref().unwrap();
        assert_eq!(msg_plan.status, PlanStatus::Approved);
        assert_eq!(msg_plan.markdown, "## Edited");

        sync_session_plan_status(&mut sess, PlanStatus::Executed, None);
        assert_eq!(sess.active_plan.as_ref().unwrap().status, PlanStatus::Executed);
        assert_eq!(
            sess.messages[0].plan_artifact.as_ref().unwrap().status,
            PlanStatus::Executed
        );
    }

    #[test]
    fn thought_trace_helpers_capture_intent_and_detect_unexecuted_plan() {
        let tc = harness_providers::ToolCallRequest {
            id: "t1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({
                "path": "crates/cinder-gui/src/chat.rs",
                "start_line": 640,
                "end_line": 780
            }),
            thought_signature: None,
        };
        assert_eq!(
            describe_tool_intent_short(&tc.name, &tc.arguments),
            "analyze chat.rs #L640-780"
        );

        let trace = format_step_thought_trace(
            1,
            Some("Need to inspect chat message trace rendering."),
            "Let me read the message card layout.",
            &[tc],
        )
        .unwrap();
        assert!(trace.starts_with("[Step 1 · analyze chat.rs #L640-780]\n"));
        assert!(trace.contains("Need to inspect chat message trace rendering."));
        assert!(trace.contains("Let me read the message card layout."));

        let synth = format_step_thought_trace(
            2,
            None,
            "",
            &[harness_providers::ToolCallRequest {
                id: "t2".into(),
                name: "search_files".into(),
                arguments: serde_json::json!({ "pattern": "*.ico" }),
                thought_signature: None,
            }],
        )
        .unwrap();
        assert_eq!(
            synth,
            "[Step 2 · search for '*.ico']\nPlanning to search for '*.ico'."
        );

        assert!(looks_like_unexecuted_tool_intent(
            "I'll start by searching for all .ico files in the project."
        ));
        assert!(!looks_like_unexecuted_tool_intent(
            "Here is the answer with code:\n```rust\nfn main() {}\n```"
        ));

        let summary = build_structured_work_summary(&[
            ToolCallRecord {
                id: "1".into(),
                name: "read_file".into(),
                arguments_json: serde_json::json!({ "path": "src/main.rs" }).to_string(),
                status: "completed".into(),
                output: "1: fn main() {}".into(),
                diff: None,
                file_path: Some("src/main.rs".into()),
                risk_tier: None,
                subagent_summary: None,
                subagent_model: None,
            },
            ToolCallRecord {
                id: "2".into(),
                name: "edit_file".into(),
                arguments_json: serde_json::json!({ "path": "src/main.rs" }).to_string(),
                status: "completed".into(),
                output: "ok".into(),
                diff: None,
                file_path: Some("src/main.rs".into()),
                risk_tier: None,
                subagent_summary: None,
                subagent_model: None,
            },
        ]);
        assert!(summary.contains("**What Was Done**"));
        assert!(summary.contains("`src/main.rs`"));
        assert!(summary.contains("Completed 2 tool step(s) with no tool errors."));
    }

    #[test]
    fn resolve_research_model_delegates_to_gemini_in_hybrid_and_stays_local_in_strict_local() {
        let dir = std::env::temp_dir().join(format!("cinder_res_model_{}", Uuid::new_v4()));
        let paths = harness_store::BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            store
                .update(|s| {
                    s.settings.api_keys.gemini_api_key = "test-gem-key".into();
                    s.settings.orchestrator.planner_model_id = "local::gemma4:12b".into();
                })
                .await
                .unwrap();

            let srv = ServerManager::new(dir.join("engines"));
            let locals = vec![];

            // Strict Local must stay local
            let gate_local = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
            let ctx_local = HarnessContext {
                gate: &gate_local,
                store: &store,
                server_mgr: &srv,
                local_models: &locals,
            };
            let picked_local = resolve_research_model(&ctx_local, "local::gemma4:12b", true).await;
            assert_eq!(picked_local, "local::gemma4:12b");

            // Hybrid delegates to reachable Gemini
            let gate_hybrid = NetworkGate::new(PrivacyMode::Hybrid).unwrap();
            let ctx_hybrid = HarnessContext {
                gate: &gate_hybrid,
                store: &store,
                server_mgr: &srv,
                local_models: &locals,
            };
            let picked_hybrid = resolve_research_model(&ctx_hybrid, "local::gemma4:12b", true).await;
            assert!(
                picked_hybrid.contains("gemini"),
                "expected gemini cloud model in Hybrid mode, got: {picked_hybrid}"
            );
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
