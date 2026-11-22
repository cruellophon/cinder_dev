use crate::{encode_base64, image_mime_for_ext, strip_data_url_prefix, HarnessContext};
use harness_models::LocalModelEntry;
use harness_net::PrivacyMode;
use harness_providers::{complete, CompletionRequest, ProviderMessage, ToolSpec};
use harness_tools::{execute_tool, resolve_workspace_path};
use serde_json::json;
use std::path::Path;

const MAX_SUBAGENT_STEPS: usize = 3;

fn is_known_vision_model_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    lower.contains("gemma4")
        || lower.contains("gemma-4")
        || lower.contains("gemma3")
        || lower.contains("gemma-3")
        || lower.contains("qwen3.5")
        || lower.contains("qwen2.5-vl")
        || lower.contains("qwen2-vl")
        || lower.contains("-vl")
        || lower.contains("vision")
        || lower.contains("llava")
        || lower.contains("minicpm-v")
        || lower.contains("pixtral")
        || lower.contains("moondream")
}

fn score_local_vision_model(m: &LocalModelEntry) -> u8 {
    let lower = m.id.to_ascii_lowercase();
    let has_proj = m.mmproj_path.is_some();
    match (has_proj, is_known_vision_model_id(&lower)) {
        (true, true) => {
            if lower.contains("gemma4") || lower.contains("vl") || lower.contains("vision") {
                4
            } else {
                3
            }
        }
        (true, false) => 2,
        (false, true) => 1,
        (false, false) => 0,
    }
}

pub async fn resolve_vision_model(ctx: &HarnessContext<'_>) -> anyhow::Result<String> {
    let settings = ctx.store.snapshot().await.settings;
    let (catalog, _) = ctx.store.build_cloud_catalog_and_quotas().await;
    let allow_cloud = ctx.gate.mode() == PrivacyMode::Hybrid;

    // 1. User-selected Vision Sub-Agent model override
    let configured = settings.orchestrator.vision_model_id.trim();
    if !configured.is_empty() {
        if configured.starts_with("local::") {
            if ctx
                .local_models
                .iter()
                .any(|m| m.id == configured && !m.is_embedding)
            {
                return Ok(configured.to_string());
            }
        } else if allow_cloud && catalog.iter().any(|c| c.id == configured && c.reachable) {
            return Ok(configured.to_string());
        }
    }

    // 2. Reuse already-loaded local model when it already has an active --mmproj projector
    let srv = ctx.server_mgr.server_status().await;
    if srv.running && srv.has_mmproj {
        if let Some(ref active_id) = srv.model_id {
            if ctx
                .local_models
                .iter()
                .any(|m| m.id == *active_id && !m.is_embedding)
            {
                return Ok(active_id.clone());
            }
        }
    }

    // 3. In Hybrid Mode, prefer fast reachable cloud vision models
    if allow_cloud {
        let mut cloud_candidates: Vec<_> = catalog
            .iter()
            .filter(|c| c.reachable && c.supports_vision)
            .collect();
        cloud_candidates.sort_by_key(|c| {
            let l = c.id.to_ascii_lowercase();
            if l.contains("flash") {
                0
            } else if l.contains("sonnet") || l.contains("gpt-4o") || l.contains("pro") {
                1
            } else {
                2
            }
        });
        if let Some(best_cloud) = cloud_candidates.first() {
            return Ok(best_cloud.id.clone());
        }
    }

    // 4. Pick best local non-embedding model with mmproj or known multimodal tag
    let mut local_candidates: Vec<&LocalModelEntry> = ctx
        .local_models
        .iter()
        .filter(|m| !m.is_embedding && score_local_vision_model(m) > 0)
        .collect();
    local_candidates.sort_by(|a, b| {
        score_local_vision_model(b)
            .cmp(&score_local_vision_model(a))
            .then_with(|| a.id.cmp(&b.id))
    });
    if let Some(best_local) = local_candidates.first() {
        return Ok(best_local.id.clone());
    }

    anyhow::bail!(
        "No vision-capable model is available. Configure 'Vision Sub-Agent' in Settings -> Agent, install a local model with an mmproj projector (e.g. Gemma 4 / Qwen 3.5), or add a cloud API key in Hybrid Mode."
    )
}

pub async fn run_vision_subagent(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    workspace_root: &Path,
    prompt: &str,
    image_paths: &[String],
    session_images: &[(String, String, String)],
) -> anyhow::Result<(String, String)> {
    let mut images: Vec<(String, String)> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    if !image_paths.is_empty() {
        for rel in image_paths.iter().take(4) {
            let Ok(abs) = resolve_workspace_path(workspace_root, rel) else {
                notes.push(format!("Rejected path outside workspace: {rel}"));
                continue;
            };
            if !abs.is_file() {
                notes.push(format!("Image file not found: {rel}"));
                continue;
            }
            let ext = abs
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let Some(mime) = image_mime_for_ext(&ext) else {
                notes.push(format!("Unsupported image format: {rel}"));
                continue;
            };
            match std::fs::read(&abs) {
                Ok(bytes) if bytes.len() <= 10 * 1024 * 1024 => {
                    images.push((mime.to_string(), encode_base64(&bytes)));
                    labels.push(rel.clone());
                }
                Ok(_) => notes.push(format!("Image exceeds 10 MB limit: {rel}")),
                Err(e) => notes.push(format!("Could not read {rel}: {e}")),
            }
        }
    } else {
        for (name, mime, b64) in session_images.iter().take(4) {
            let clean = strip_data_url_prefix(b64).trim();
            if !clean.is_empty() {
                images.push((mime.clone(), clean.to_string()));
                labels.push(name.clone());
            }
        }
    }

    if images.is_empty() {
        let extra = if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join("; "))
        };
        return Ok((
            format!(
                "No images available to inspect{extra}. Ask the user to paste (Ctrl+V) or attach a screenshot, or specify valid workspace image paths."
            ),
            String::new(),
        ));
    }

    let vision_model = resolve_vision_model(ctx).await?;
    ctx.store
        .set_activity(
            Some(session_id),
            "vision_subagent",
            &format!(
                "Vision Sub-Agent ({vision_model}) inspecting {} image(s)...",
                images.len()
            ),
            1,
            1,
        )
        .await;

    let sys_prompt = "You are Cinder's specialized Vision Sub-Agent. Examine the attached image(s) carefully and report ONLY what is clearly visible:\n\
        1. Transcribe exact visible text, UI labels, file paths, stack traces, numbers, or code verbatim.\n\
        2. Describe layout bugs, alignment glitches, visual styling, or diagram structure clearly and concisely.\n\
        3. Never guess or fabricate obscured text; state clearly if any region is too small or blurry.";

    let req = CompletionRequest {
        model_id: vision_model.clone(),
        system_prompt: sys_prompt.into(),
        messages: vec![ProviderMessage {
            role: "user".into(),
            content: if prompt.trim().is_empty() {
                "Inspect the attached image(s) in detail and transcribe all relevant text, UI details, or errors.".into()
            } else {
                prompt.to_string()
            },
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
            images,
        }],
        tools: Vec::new(),
        enable_thinking: false,
        max_tokens: 2048,
        temperature: 0.1,
    };

    let resp = complete(ctx.gate, ctx.store, ctx.server_mgr, ctx.local_models, req).await?;
    let body = if resp.content.trim().is_empty() {
        "Vision Sub-Agent returned an empty response.".to_string()
    } else {
        resp.content.trim().to_string()
    };
    let used_model = if resp.actual_model_id.is_empty() {
        vision_model
    } else {
        resp.actual_model_id
    };

    Ok((
        format!(
            "[Vision Sub-Agent · {used_model} · images: {}]\n{body}",
            labels.join(", ")
        ),
        used_model,
    ))
}

/// Choose the model for `delegate_research`: in Hybrid Mode, local models delegate to a fast
/// reachable cloud model (preferring Gemini) unless `StrictLocal` is active.
pub async fn resolve_research_model(
    ctx: &HarnessContext<'_>,
    caller_model_id: &str,
    prefer_cloud: bool,
) -> String {
    if ctx.gate.mode() == PrivacyMode::Hybrid
        && (caller_model_id.starts_with("local::") || prefer_cloud)
    {
        let settings = ctx.store.snapshot().await.settings;
        let (catalog, _) = ctx.store.build_cloud_catalog_and_quotas().await;

        let configured = settings.orchestrator.planner_model_id.trim();
        if !configured.is_empty()
            && !configured.starts_with("local::")
            && catalog.iter().any(|c| c.id == configured && c.reachable)
        {
            return configured.to_string();
        }

        let mut cloud_candidates: Vec<_> = catalog
            .iter()
            .filter(|c| c.reachable && !c.id.starts_with("local::"))
            .collect();
        cloud_candidates.sort_by_key(|c| {
            let l = c.id.to_ascii_lowercase();
            if l.contains("gemini") && l.contains("flash") {
                0
            } else if l.contains("gemini") {
                1
            } else if l.contains("sonnet") || l.contains("gpt-4o") || l.contains("pro") {
                2
            } else {
                3
            }
        });
        if let Some(best_cloud) = cloud_candidates.first() {
            return best_cloud.id.clone();
        }
    }

    caller_model_id.to_string()
}

pub async fn run_research_subagent(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    workspace_root: &Path,
    caller_model_id: &str,
    task: &str,
    focus_paths: &[String],
    prefer_cloud: bool,
) -> anyhow::Result<(String, String)> {
    let target_model = resolve_research_model(ctx, caller_model_id, prefer_cloud).await;
    let has_ws = !workspace_root.as_os_str().is_empty() && workspace_root.is_dir();
    let allow_web = ctx.gate.mode() == PrivacyMode::Hybrid;

    let mut subagent_specs = Vec::new();
    if has_ws {
        subagent_specs.extend([
            ToolSpec {
                name: "list_dir".into(),
                description: "List files inside a workspace directory.".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "max_depth": { "type": "integer" }
                    }
                }),
            },
            ToolSpec {
                name: "read_file".into(),
                description: "Read numbered lines from a file inside the workspace.".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "start_line": { "type": "integer" },
                        "end_line": { "type": "integer" }
                    },
                    "required": ["path"]
                }),
            },
            ToolSpec {
                name: "search_files".into(),
                description: "Search text/regex across workspace files.".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "sub_path": { "type": "string" }
                    },
                    "required": ["pattern"]
                }),
            },
            ToolSpec {
                name: "extract_pdf_text".into(),
                description: "Extract text from a workspace PDF offline.".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "max_pages": { "type": "integer" }
                    },
                    "required": ["path"]
                }),
            },
            ToolSpec {
                name: "search_knowledge".into(),
                description: "Search the offline workspace knowledge index.".into(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "top_k": { "type": "integer" }
                    },
                    "required": ["query"]
                }),
            },
        ]);
    }
    if allow_web {
        subagent_specs.push(ToolSpec {
            name: "web_search".into(),
            description: "Search the live web and Wikipedia for verified articles and URLs.".into(),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        });
    }

    let focus_str = if focus_paths.is_empty() {
        "none specified".to_string()
    } else {
        focus_paths.join(", ")
    };

    let ws_line = if has_ws {
        format!("Workspace: {}", workspace_root.display())
    } else {
        "Standalone session (no local workspace attached).".to_string()
    };
    let web_rule = if allow_web {
        "When asked about online facts, Wikipedia, or sources/URLs, ALWAYS call `web_search` and cite ONLY exact verified URLs returned by `web_search`. Never invent links or claim a web search ran if it failed or was skipped."
    } else {
        "You are in Strict Local Mode (offline only). Inspect local workspace files strictly."
    };

    let sys_prompt = format!(
        "You are Cinder's isolated Research Sub-Agent ({target_model}).\n\
        {ws_line}\n\
        Focus paths: {focus_str}\n\
        {web_rule}\n\
        You have at most {MAX_SUBAGENT_STEPS} tool rounds; return a factual, concise synthesis."
    );

    let mut history = vec![ProviderMessage {
        role: "user".into(),
        content: task.to_string(),
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
        images: Vec::new(),
    }];

    let brave_key = ctx
        .store
        .snapshot()
        .await
        .settings
        .api_keys
        .brave_search_api_key;

    let mut final_summary = String::new();
    let mut actual_model = target_model.clone();
    for step in 0..MAX_SUBAGENT_STEPS {
        ctx.store
            .set_activity(
                Some(session_id),
                "subagent",
                &format!(
                    "Research Sub-Agent ({target_model}) investigating ({}/{MAX_SUBAGENT_STEPS})...",
                    step + 1
                ),
                step + 1,
                MAX_SUBAGENT_STEPS,
            )
            .await;

        let req = CompletionRequest {
            model_id: target_model.clone(),
            system_prompt: sys_prompt.clone(),
            messages: history.clone(),
            tools: if step == MAX_SUBAGENT_STEPS - 1 || subagent_specs.is_empty() {
                Vec::new()
            } else {
                subagent_specs.clone()
            },
            enable_thinking: false,
            max_tokens: 2048,
            temperature: 0.2,
        };

        let resp = complete(ctx.gate, ctx.store, ctx.server_mgr, ctx.local_models, req).await?;
        if !resp.actual_model_id.trim().is_empty() {
            actual_model = resp.actual_model_id.clone();
        }
        if resp.tool_calls.is_empty() || step == MAX_SUBAGENT_STEPS - 1 {
            final_summary = resp.content;
            break;
        }

        history.push(ProviderMessage {
            role: "assistant".into(),
            content: resp.content,
            tool_call_id: None,
            tool_name: None,
            tool_calls: resp.tool_calls.clone(),
            images: Vec::new(),
        });

        for tc in resp.tool_calls {
            let res = execute_tool(workspace_root, ctx.gate, &brave_key, &tc.name, &tc.arguments).await;
            history.push(ProviderMessage {
                role: "tool".into(),
                content: res.output,
                tool_call_id: Some(tc.id),
                tool_name: Some(tc.name),
                tool_calls: Vec::new(),
                images: Vec::new(),
            });
        }
    }

    if final_summary.trim().is_empty() {
        final_summary = "Subagent completed inspection with no additional text.".into();
    }

    Ok((
        format!("[Research Sub-Agent · {actual_model}]\n{final_summary}"),
        actual_model,
    ))
}
