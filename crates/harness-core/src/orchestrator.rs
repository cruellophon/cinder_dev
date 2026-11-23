use crate::{
    active_tools_for_context, apply_thinking_effort, build_fallback_plan_from_notes,
    build_pending_tool_approval, build_pending_user_question, build_plan_artifact,
    build_step_contingent_question, build_system_prompt, complete_cancellable,
    execute_tool_with_checkpoint, format_step_thought_trace, handle_delegate_research_tool,
    handle_delegate_vision_tool, snapshot_history_for_paused_tool, try_parse_inline_submit_plan,
    upsert_artifact_from_tool, upsert_session_artifact, HarnessContext,
};
use chrono::Utc;
use harness_providers::{CompletionRequest, ProviderMessage};
use harness_store::{
    AgentMode, AppSettings, ChatMessage, OrchestratorPhaseRecord, PendingToolApproval, PlanArtifact,
    PlanStatus, RiskTier, Session, ToolCallRecord,
};
use harness_tools::{
    classify_tool_risk_with_context, execute_tool_with_policy, extract_tool_secret_path,
    ToolExecutionResult,
};
use std::path::Path;
use uuid::Uuid;

fn friendly_model_label(settings: &AppSettings, model_id: &str) -> String {
    let fallback = model_id.strip_prefix("local::").unwrap_or(model_id);
    settings.resolve_model_alias(model_id, fallback).to_string()
}

pub async fn resolve_reachable_model(
    ctx: &HarnessContext<'_>,
    preferred_id: &str,
    fallback_id: &str,
) -> anyhow::Result<String> {
    let settings = ctx.store.snapshot().await.settings;
    let allow_cloud = ctx.gate.mode() == harness_net::PrivacyMode::Hybrid;
    let (catalog, _) = ctx.store.build_cloud_catalog_and_quotas().await;
    let is_reachable = |id: &str| -> bool {
        if id.starts_with("local::") {
            ctx.local_models
                .iter()
                .any(|m| m.id == id && !m.is_embedding)
        } else {
            allow_cloud
                && catalog
                    .iter()
                    .any(|c| c.id == id && c.reachable && !c.requires_payment)
        }
    };

    if is_reachable(preferred_id) {
        return Ok(preferred_id.to_string());
    }
    if is_reachable(fallback_id) {
        return Ok(fallback_id.to_string());
    }
    for chain_id in &settings.fallback_chain {
        let cid = chain_id.trim();
        if !cid.is_empty() && is_reachable(cid) {
            return Ok(cid.to_string());
        }
    }
    if !preferred_id.starts_with("local::") && allow_cloud {
        if let Some(cloud) = catalog
            .iter()
            .find(|c| c.reachable && !c.requires_payment)
        {
            return Ok(cloud.id.clone());
        }
    }
    if let Some(local) = ctx.local_models.iter().find(|m| !m.is_embedding) {
        return Ok(local.id.clone());
    }
    if allow_cloud {
        if let Some(cloud) = catalog
            .iter()
            .find(|c| c.reachable && !c.requires_payment)
        {
            return Ok(cloud.id.clone());
        }
    }
    anyhow::bail!("No reachable local or cloud models found.")
}

/// Automatically select the strongest reachable Planner model for Orchestrator Auto mode,
/// anchoring to the user's active cloud Chat model or ordered fallback chain first.
pub async fn resolve_auto_planner_model(
    ctx: &HarnessContext<'_>,
    active_fallback: &str,
) -> anyhow::Result<String> {
    if ctx.gate.mode() == harness_net::PrivacyMode::Hybrid {
        let settings = ctx.store.snapshot().await.settings;
        let (catalog, _) = ctx.store.build_cloud_catalog_and_quotas().await;
        let mut cloud: Vec<_> = catalog
            .iter()
            .filter(|c| c.reachable && !c.requires_payment && !c.id.starts_with("local::"))
            .collect();

        if !active_fallback.starts_with("local::")
            && cloud.iter().any(|c| c.id == active_fallback)
        {
            return Ok(active_fallback.to_string());
        }
        for chain_id in &settings.fallback_chain {
            let cid = chain_id.trim();
            if !cid.starts_with("local::") && cloud.iter().any(|c| c.id == cid) {
                return Ok(cid.to_string());
            }
        }

        cloud.sort_by_key(|c| {
            let l = c.id.to_ascii_lowercase();
            if (l.contains("gemini") && l.contains("pro"))
                || l.contains("sonnet")
                || l.contains("opus")
            {
                0
            } else if l.contains("gemini") && l.contains("flash") {
                1
            } else if l.contains("gemini") || l.contains("gpt") {
                2
            } else {
                3
            }
        });
        if let Some(best) = cloud.first() {
            return Ok(best.id.clone());
        }
    }
    resolve_reachable_model(ctx, active_fallback, active_fallback).await
}

/// Automatically select the fastest/strongest reachable Execution model:
/// anchors to the active cloud planner/Chat model or ordered fallback chain in Hybrid Mode,
/// or reuses the loaded VRAM-fitting local model / best coding local model in Strict Local Mode.
pub async fn resolve_auto_executor_model(
    ctx: &HarnessContext<'_>,
    planner_model: &str,
) -> anyhow::Result<String> {
    if ctx.gate.mode() == harness_net::PrivacyMode::Hybrid {
        let settings = ctx.store.snapshot().await.settings;
        let (catalog, _) = ctx.store.build_cloud_catalog_and_quotas().await;
        let mut cloud: Vec<_> = catalog
            .iter()
            .filter(|c| c.reachable && !c.requires_payment && !c.id.starts_with("local::"))
            .collect();

        if !planner_model.starts_with("local::")
            && cloud.iter().any(|c| c.id == planner_model)
        {
            return Ok(planner_model.to_string());
        }
        for chain_id in &settings.fallback_chain {
            let cid = chain_id.trim();
            if !cid.starts_with("local::") && cloud.iter().any(|c| c.id == cid) {
                return Ok(cid.to_string());
            }
        }

        cloud.sort_by_key(|c| {
            let l = c.id.to_ascii_lowercase();
            if (l.contains("gemini") && l.contains("flash") && !l.contains("lite"))
                || l.contains("sonnet")
                || l.contains("gpt-4o")
            {
                0
            } else if l.contains("gemini") {
                1
            } else if l.contains("claude")
                || l.contains("gpt")
                || l.contains("deepseek")
                || l.contains("qwen")
            {
                2
            } else {
                3
            }
        });
        if let Some(best) = cloud.first() {
            return Ok(best.id.clone());
        }
    }

    let srv = ctx.server_mgr.server_status().await;
    if srv.running {
        if let Some(ref loaded_id) = srv.model_id {
            if ctx
                .local_models
                .iter()
                .any(|m| m.id == *loaded_id && !m.is_embedding)
            {
                return Ok(loaded_id.clone());
            }
        }
    }

    let mut locals: Vec<_> = ctx
        .local_models
        .iter()
        .filter(|m| !m.is_embedding)
        .collect();
    locals.sort_by_key(|m| {
        let fit_rank = match m.vram_fit.tier {
            harness_models::FitTier::FitsInVram => 0u8,
            harness_models::FitTier::PartialOffload => 1,
            harness_models::FitTier::CpuHeavy => 2,
        };
        let l = m.id.to_ascii_lowercase();
        let fam_rank = if l.contains("coder")
            || l.contains("qwen3")
            || l.contains("gemma4")
            || l.contains("gemma-4")
            || l.contains("llama3")
        {
            0u8
        } else {
            1
        };
        (fit_rank, fam_rank, m.size_bytes)
    });
    if let Some(best_local) = locals.first() {
        return Ok(best_local.id.clone());
    }

    resolve_reachable_model(ctx, planner_model, planner_model).await
}

pub async fn run_orchestrator_turn(
    ctx: &HarnessContext<'_>,
    session_id: &str,
    workspace_root: &Path,
    prompt_with_context: &str,
    history_base: Vec<ProviderMessage>,
    attached_files: Vec<String>,
) -> anyhow::Result<Session> {
    let snapshot = ctx.store.snapshot().await;
    let orch_cfg = snapshot.settings.orchestrator.clone();
    let active_fallback = snapshot.settings.active_model_id.clone();

    let planner_model = if orch_cfg.planner_model_id.trim().is_empty()
        || orch_cfg.planner_model_id.eq_ignore_ascii_case("auto")
    {
        resolve_auto_planner_model(ctx, &active_fallback).await?
    } else {
        resolve_reachable_model(ctx, &orch_cfg.planner_model_id, &active_fallback).await?
    };
    let executor_model = if orch_cfg.executor_model_id.trim().is_empty()
        || orch_cfg.executor_model_id.eq_ignore_ascii_case("auto")
    {
        resolve_auto_executor_model(ctx, &planner_model).await?
    } else {
        resolve_reachable_model(ctx, &orch_cfg.executor_model_id, &planner_model).await?
    };

    let planner_label = friendly_model_label(&snapshot.settings, &planner_model);
    let executor_label = friendly_model_label(&snapshot.settings, &executor_model);

    // --- PHASE 1: PLANNER ---
    let enabled_modules = &snapshot.settings.ui.enabled_modules;
    let thinking_effort = &snapshot.settings.ui.thinking_effort;
    let base_plan_sys = format!(
        "{}\n\nYou are the PLANNER phase of an Orchestrator workflow. Inspect read-only and submit a concise Plan Artifact via `submit_plan` (do not assume files are edited yet).",
        build_system_prompt(
            workspace_root,
            AgentMode::Plan,
            planner_model.starts_with("local::"),
            None,
            &snapshot.settings.ui.custom_system_prompt,
        )
    );
    let (plan_thinking, plan_sys) = apply_thinking_effort(&base_plan_sys, thinking_effort);
    let has_workspace = !workspace_root.as_os_str().is_empty() && workspace_root.is_dir();
    let allow_web = ctx.gate.mode() == harness_net::PrivacyMode::Hybrid;
    let plan_specs =
        active_tools_for_context(AgentMode::Plan, allow_web, enabled_modules, has_workspace);
    let mut plan_history = history_base.clone();
    let mut all_tools: Vec<ToolCallRecord> = Vec::new();
    let mut extra_phases: Vec<OrchestratorPhaseRecord> = Vec::new();
    let mut accumulated_thoughts: Vec<String> = Vec::new();
    let mut planner_tokens = 0u64;
    let mut planner_latency = 0u64;
    let mut produced_plan: Option<PlanArtifact> = None;
    let mut plan_artifact_id: Option<String> = None;
    let mut planner_notes = String::new();

    for step in 0..4usize {
        if ctx.store.is_aborted() {
            planner_notes = "[Generation stopped by user]".into();
            break;
        }
        let step_verb = if step == 3 {
            "drafting plan..."
        } else if step == 0 {
            "investigating workspace..."
        } else {
            "analyzing findings..."
        };
        ctx.store
            .set_activity_stream(
                Some(session_id),
                "orchestrator_planner",
                &format!(
                    "Orchestrator Phase 1/2 · Planner ({planner_label}) · Turn {} — {step_verb}",
                    step + 1
                ),
                step + 1,
                4,
                &planner_model,
                &accumulated_thoughts.join("\n\n"),
                "",
                &all_tools,
            )
            .await;

        let req = CompletionRequest {
            model_id: planner_model.clone(),
            system_prompt: plan_sys.clone(),
            messages: plan_history.clone(),
            tools: if step == 3 {
                Vec::new()
            } else {
                plan_specs.clone()
            },
            enable_thinking: plan_thinking,
            max_tokens: 3072,
            temperature: 0.2,
        };
        let Some(resp) = complete_cancellable(ctx, req).await? else {
            planner_notes = "[Generation stopped by user]".into();
            break;
        };
        planner_tokens += resp.prompt_tokens + resp.completion_tokens + resp.thinking_tokens;
        planner_latency += resp.latency_ms;
        if let Some(th) = format_step_thought_trace(
            step + 1,
            resp.thinking.as_deref(),
            if resp.tool_calls.is_empty() {
                ""
            } else {
                &resp.content
            },
            &resp.tool_calls,
        ) {
            accumulated_thoughts.push(th);
        }

        if resp.tool_calls.is_empty() {
            planner_notes = resp.content;
            break;
        }

        plan_history.push(ProviderMessage {
            role: "assistant".into(),
            content: resp.content.clone(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: resp.tool_calls.clone(),
            images: Vec::new(),
        });

        for tc in &resp.tool_calls {
            ctx.store
                .set_activity_stream(
                    Some(session_id),
                    "orchestrator_planner",
                    &format!(
                        "Orchestrator Phase 1/2 · Planner ({planner_label}) · Turn {} · Running `{}`...",
                        step + 1,
                        tc.name
                    ),
                    step + 1,
                    4,
                    &planner_model,
                    &accumulated_thoughts.join("\n\n"),
                    "",
                    &all_tools,
                )
                .await;

            if tc.name == "write_artifact" {
                let (rec, msg_txt) = upsert_artifact_from_tool(ctx.store, session_id, tc).await;
                all_tools.push(rec);
                plan_history.push(ProviderMessage {
                    role: "tool".into(),
                    content: msg_txt,
                    tool_call_id: Some(tc.id.clone()),
                    tool_name: Some(tc.name.clone()),
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
                continue;
            }
            if tc.name == "submit_plan" {
                let mut plan = build_plan_artifact(&planner_model, &tc.arguments);
                if !orch_cfg.review_before_execute {
                    plan.status = PlanStatus::Approved;
                }
                if let Ok(art) = upsert_session_artifact(
                    ctx.store,
                    session_id,
                    &plan.title,
                    "plan",
                    &plan.markdown,
                )
                .await
                {
                    plan_artifact_id = Some(art.id);
                }
                produced_plan = Some(plan.clone());
                all_tools.push(ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments_json: tc.arguments.to_string(),
                    status: "completed".into(),
                    output: format!("Planned '{}'", plan.title),
                    diff: None,
                    file_path: None,
                    risk_tier: Some(RiskTier::SafeReadOnly),
                    subagent_summary: None,
                    subagent_model: None,
                });
                break;
            }

            if tc.name == "ask_user" {
                let snap_hist = snapshot_history_for_paused_tool(&plan_history, &tc.id);
                let pq = build_pending_user_question(
                    &tc.id,
                    &tc.arguments,
                    &planner_model,
                    serde_json::to_string(&snap_hist)?,
                    step,
                );
                let question = pq.question.clone();
                let msg = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "assistant".into(),
                    content: format!("Orchestrator Planner paused for clarification: {question}"),
                    thinking: if accumulated_thoughts.is_empty() {
                        None
                    } else {
                        Some(accumulated_thoughts.join("\n\n"))
                    },
                    model_id: Some(planner_model.clone()),
                    latency_ms: Some(planner_latency),
                    prompt_tokens: Some(planner_tokens),
                    completion_tokens: Some(0),
                    tool_calls: all_tools,
                    compare_partner: None,
                    checkpoint_id: None,
                    plan_artifact: None,
                    orchestrator_phases: vec![OrchestratorPhaseRecord {
                        phase: "planner".into(),
                        model_id: planner_model,
                        summary: format!("Asked user: {question}"),
                        latency_ms: planner_latency,
                        tokens: planner_tokens,
                    }],
                    attached_files,
                    attachments: Vec::new(),
                    artifact_id: None,
                    fallback_from_model_id: None,
                    fallback_reason: None,
                    created_at: Utc::now(),
                };
                let updated = ctx
                    .store
                    .update(|s| {
                        if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                            sx.pending_question = Some(pq);
                            sx.messages.push(msg);
                            sx.updated_at = Utc::now();
                            return Some(sx.clone());
                        }
                        None
                    })
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
                return Ok(updated);
            }

            if tc.name == "delegate_research" {
                let (rec, tool_msg, phase) = handle_delegate_research_tool(
                    ctx,
                    session_id,
                    workspace_root,
                    &planner_model,
                    tc,
                )
                .await;
                all_tools.push(rec);
                extra_phases.push(phase);
                plan_history.push(tool_msg);
                continue;
            }

            if tc.name == "delegate_vision" {
                let (rec, tool_msg, phase) =
                    handle_delegate_vision_tool(ctx, session_id, workspace_root, tc).await;
                all_tools.push(rec);
                extra_phases.push(phase);
                plan_history.push(tool_msg);
                continue;
            }

            let planner_is_cloud = !planner_model.starts_with("local::");
            let res = if planner_is_cloud && tc.name != "search_files" {
                if let Some(secret_path) = extract_tool_secret_path(&tc.name, &tc.arguments) {
                    ToolExecutionResult {
                        output: format!(
                            "Cloud Secret Guard: blocked automatic read of sensitive file '{secret_path}' during cloud planning. Ask the user directly or execute a read explicitly in Agent Mode where it can be approved."
                        ),
                        diff: None,
                        file_path: Some(secret_path),
                        is_error: true,
                    }
                } else {
                    execute_tool_with_policy(
                        workspace_root,
                        ctx.gate,
                        &snapshot.settings.api_keys.brave_search_api_key,
                        &tc.name,
                        &tc.arguments,
                        true,
                    )
                    .await
                }
            } else {
                execute_tool_with_policy(
                    workspace_root,
                    ctx.gate,
                    &snapshot.settings.api_keys.brave_search_api_key,
                    &tc.name,
                    &tc.arguments,
                    planner_is_cloud,
                )
                .await
            };
            all_tools.push(ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments_json: tc.arguments.to_string(),
                status: if res.is_error {
                    "error".into()
                } else {
                    "completed".into()
                },
                output: res.output.clone(),
                diff: res.diff,
                file_path: res.file_path,
                risk_tier: Some(RiskTier::SafeReadOnly),
                subagent_summary: None,
                subagent_model: None,
            });
            plan_history.push(ProviderMessage {
                role: "tool".into(),
                content: res.output,
                tool_call_id: Some(tc.id.clone()),
                tool_name: Some(tc.name.clone()),
                tool_calls: Vec::new(),
                images: Vec::new(),
            });
            ctx.store
                .set_activity_stream(
                    Some(session_id),
                    "orchestrator_planner",
                    &format!(
                        "Orchestrator Phase 1/2 · Planner ({planner_label}) · Turn {} · Completed `{}`",
                        step + 1,
                        tc.name
                    ),
                    step + 1,
                    4,
                    &planner_model,
                    &accumulated_thoughts.join("\n\n"),
                    "",
                    &all_tools,
                )
                .await;
        }

        if produced_plan.is_some() {
            break;
        }
    }

    let plan_status = if orch_cfg.review_before_execute {
        PlanStatus::AwaitingReview
    } else {
        PlanStatus::Approved
    };
    let mut plan = produced_plan
        .or_else(|| try_parse_inline_submit_plan(&planner_model, &planner_notes))
        .unwrap_or_else(|| {
            let source_notes = if planner_notes.trim().is_empty() {
                prompt_with_context
            } else {
                &planner_notes
            };
            build_fallback_plan_from_notes(
                &planner_model,
                "Orchestrator Plan",
                source_notes,
                plan_status,
            )
        });
    plan.status = plan_status;
    plan.executor_model_id = Some(executor_model.clone());

    if plan_artifact_id.is_none() && !ctx.store.is_aborted() && !plan.markdown.trim().is_empty() {
        if let Ok(art) = upsert_session_artifact(
            ctx.store,
            session_id,
            &plan.title,
            "plan",
            &plan.markdown,
        )
        .await
        {
            plan_artifact_id = Some(art.id);
        }
    }

    let planner_phase = OrchestratorPhaseRecord {
        phase: "planner".into(),
        model_id: planner_model.clone(),
        summary: plan.title.clone(),
        latency_ms: planner_latency,
        tokens: planner_tokens,
    };

    if orch_cfg.review_before_execute {
        let msg = ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: format!(
                "Orchestrator Planner (**{planner_label}**) prepared **{}**. Review and click **Approve & Execute Plan** to hand off to **{executor_label}**.",
                plan.title
            ),
            thinking: if accumulated_thoughts.is_empty() {
                None
            } else {
                Some(accumulated_thoughts.join("\n\n"))
            },
            model_id: Some(planner_model),
            latency_ms: Some(planner_latency),
            prompt_tokens: Some(planner_tokens),
            completion_tokens: Some(0),
            tool_calls: all_tools,
            compare_partner: None,
            checkpoint_id: None,
            plan_artifact: Some(plan.clone()),
            orchestrator_phases: vec![planner_phase],
            attached_files,
            attachments: Vec::new(),
            artifact_id: plan_artifact_id,
            fallback_from_model_id: None,
            fallback_reason: None,
            created_at: Utc::now(),
        };
        let updated = ctx
            .store
            .update(|s| {
                if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                    sx.active_plan = Some(plan);
                    sx.messages.push(msg);
                    sx.updated_at = Utc::now();
                    return Some(sx.clone());
                }
                None
            })
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        return Ok(updated);
    }

    // --- PHASE 2: EXECUTOR ---
    ctx.store
        .set_activity(
            Some(session_id),
            "orchestrator_executor",
            &format!("Orchestrator Phase 2/2 · Executor ({executor_label}) executing plan..."),
            2,
            2,
        )
        .await;

    let base_exec_sys = build_system_prompt(
        workspace_root,
        AgentMode::Agent,
        executor_model.starts_with("local::"),
        Some(&plan),
        &snapshot.settings.ui.custom_system_prompt,
    );
    let (exec_thinking, exec_sys) = apply_thinking_effort(&base_exec_sys, thinking_effort);
    let exec_specs = active_tools_for_context(
        AgentMode::Agent,
        ctx.gate.mode() == harness_net::PrivacyMode::Hybrid,
        enabled_modules,
        has_workspace,
    );
    let mut exec_history = history_base;
    exec_history.push(ProviderMessage {
        role: "user".into(),
        content: format!(
            "Execute this approved plan inside the workspace and verify if edits are made:\n\n{}",
            plan.markdown
        ),
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
        images: Vec::new(),
    });

    let checkpoint_id = Uuid::new_v4().to_string();
    let max_exec_steps = orch_cfg.max_autonomous_steps.clamp(2, 64);
    let mut produced_checkpoint = false;
    let mut exec_tokens = 0u64;
    let mut exec_latency = 0u64;
    let mut exec_content = String::new();
    let mut last_sig: Option<String> = None;
    let mut completed_cleanly = false;

    for step in 0..max_exec_steps {
        if ctx.store.is_aborted() {
            exec_content = if exec_content.trim().is_empty() {
                "[Generation stopped by user]".into()
            } else {
                format!("{exec_content}\n\n[Generation stopped by user]")
            };
            completed_cleanly = true;
            break;
        }
        ctx.store
            .set_activity_stream(
                Some(session_id),
                "orchestrator_executor",
                &format!(
                    "Orchestrator Phase 2/2 · Executor ({executor_label}) · Turn {}...",
                    step + 1
                ),
                step + 1,
                max_exec_steps,
                &executor_model,
                &accumulated_thoughts.join("\n\n"),
                &exec_content,
                &all_tools,
            )
            .await;

        let req = CompletionRequest {
            model_id: executor_model.clone(),
            system_prompt: exec_sys.clone(),
            messages: exec_history.clone(),
            tools: exec_specs.clone(),
            enable_thinking: exec_thinking,
            max_tokens: 4096,
            temperature: 0.2,
        };
        let Some(resp) = complete_cancellable(ctx, req).await? else {
            exec_content = "[Generation stopped by user]".into();
            completed_cleanly = true;
            break;
        };
        exec_tokens += resp.prompt_tokens + resp.completion_tokens + resp.thinking_tokens;
        exec_latency += resp.latency_ms;
        if let Some(th) = format_step_thought_trace(
            step + 1,
            resp.thinking.as_deref(),
            if resp.tool_calls.is_empty() {
                ""
            } else {
                &resp.content
            },
            &resp.tool_calls,
        ) {
            accumulated_thoughts.push(th);
        }

        if resp.tool_calls.is_empty() {
            exec_content = resp.content;
            completed_cleanly = true;
            break;
        }

        let sig = resp
            .tool_calls
            .iter()
            .map(|t| format!("{}:{}", t.name, t.arguments))
            .collect::<Vec<_>>()
            .join("|");
        if last_sig.as_deref() == Some(&sig) {
            exec_content = resp.content;
            completed_cleanly = true;
            break;
        }
        last_sig = Some(sig);

        exec_history.push(ProviderMessage {
            role: "assistant".into(),
            content: resp.content.clone(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: resp.tool_calls.clone(),
            images: Vec::new(),
        });

        let mut paused: Option<PendingToolApproval> = None;
        for tc in &resp.tool_calls {
            if tc.name == "write_artifact" {
                let (rec, msg_txt) = upsert_artifact_from_tool(ctx.store, session_id, tc).await;
                all_tools.push(rec);
                exec_history.push(ProviderMessage {
                    role: "tool".into(),
                    content: msg_txt,
                    tool_call_id: Some(tc.id.clone()),
                    tool_name: Some(tc.name.clone()),
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
                continue;
            }
            if tc.name == "delegate_vision" {
                let (rec, tool_msg, phase) =
                    handle_delegate_vision_tool(ctx, session_id, workspace_root, tc).await;
                all_tools.push(rec);
                extra_phases.push(phase);
                exec_history.push(tool_msg);
                continue;
            }
            if tc.name == "switch_mode" {
                let msg = "Orchestrator Executor is already running in execution mode with the approved plan active; continue executing steps directly.";
                all_tools.push(ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments_json: tc.arguments.to_string(),
                    status: "completed".into(),
                    output: msg.into(),
                    diff: None,
                    file_path: None,
                    risk_tier: Some(RiskTier::SafeReadOnly),
                    subagent_summary: None,
                    subagent_model: None,
                });
                exec_history.push(ProviderMessage {
                    role: "tool".into(),
                    content: msg.into(),
                    tool_call_id: Some(tc.id.clone()),
                    tool_name: Some(tc.name.clone()),
                    tool_calls: Vec::new(),
                    images: Vec::new(),
                });
                continue;
            }
            if tc.name == "delegate_research" {
                let (rec, tool_msg, phase) = handle_delegate_research_tool(
                    ctx,
                    session_id,
                    workspace_root,
                    &executor_model,
                    tc,
                )
                .await;
                all_tools.push(rec);
                extra_phases.push(phase);
                exec_history.push(tool_msg);
                continue;
            }
            if tc.name == "ask_user" {
                let snap_hist = snapshot_history_for_paused_tool(&exec_history, &tc.id);
                let pq = build_pending_user_question(
                    &tc.id,
                    &tc.arguments,
                    &executor_model,
                    serde_json::to_string(&snap_hist)?,
                    step,
                );
                let question = pq.question.clone();
                let exec_phase = OrchestratorPhaseRecord {
                    phase: "executor".into(),
                    model_id: executor_model.clone(),
                    summary: format!("Asked user: {question}"),
                    latency_ms: exec_latency,
                    tokens: exec_tokens,
                };
                let msg = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "assistant".into(),
                    content: if resp.content.trim().is_empty() {
                        format!("Orchestrator Executor paused for clarification: {question}")
                    } else {
                        resp.content.clone()
                    },
                    thinking: if accumulated_thoughts.is_empty() {
                        None
                    } else {
                        Some(accumulated_thoughts.join("\n\n"))
                    },
                    model_id: Some(format!("{planner_model} → {executor_model}")),
                    latency_ms: Some(planner_latency + exec_latency),
                    prompt_tokens: Some(planner_tokens + exec_tokens),
                    completion_tokens: Some(0),
                    tool_calls: all_tools,
                    compare_partner: None,
                    checkpoint_id: if produced_checkpoint {
                        Some(checkpoint_id)
                    } else {
                        None
                    },
                    plan_artifact: Some(plan.clone()),
                    orchestrator_phases: vec![planner_phase, exec_phase],
                    attached_files,
                    attachments: Vec::new(),
                    artifact_id: None,
                    fallback_from_model_id: None,
                    fallback_reason: None,
                    created_at: Utc::now(),
                };
                let updated = ctx
                    .store
                    .update(|s| {
                        if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                            sx.active_plan = Some(plan);
                            sx.pending_question = Some(pq);
                            sx.messages.push(msg);
                            sx.updated_at = Utc::now();
                            return Some(sx.clone());
                        }
                        None
                    })
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
                return Ok(updated);
            }
            let executor_is_cloud = !executor_model.starts_with("local::");
            let (risk_tier, risk_reason) =
                classify_tool_risk_with_context(&tc.name, &tc.arguments, executor_is_cloud);
            let needs_approval = match risk_tier {
                RiskTier::SafeReadOnly => false,
                RiskTier::HighRiskDestructive => true,
                RiskTier::WorkspaceMutation => match tc.name.as_str() {
                    "write_file" | "edit_file" => false, // Plan already approved by Orchestrator
                    "run_command" | "verify_workspace" => !snapshot.settings.auto_approve_commands,
                    _ => false,
                },
            };

            if needs_approval {
                let snap_hist = snapshot_history_for_paused_tool(&exec_history, &tc.id);
                paused = Some(build_pending_tool_approval(
                    workspace_root,
                    &tc.id,
                    &tc.name,
                    &tc.arguments,
                    risk_tier,
                    &risk_reason,
                    &executor_model,
                    serde_json::to_string(&snap_hist)?,
                    step,
                ));
                break;
            }

            ctx.store
                .set_activity_stream(
                    Some(session_id),
                    "orchestrator_executor",
                    &format!(
                        "Orchestrator Phase 2/2 · Executor ({executor_label}) · Turn {} · Running `{}`...",
                        step + 1,
                        tc.name
                    ),
                    step + 1,
                    max_exec_steps,
                    &executor_model,
                    &accumulated_thoughts.join("\n\n"),
                    &exec_content,
                    &all_tools,
                )
                .await;

            let exec_res = execute_tool_with_checkpoint(
                ctx,
                session_id,
                &checkpoint_id,
                &plan.title,
                &tc.arguments,
                workspace_root,
                &snapshot.settings.api_keys.brave_search_api_key,
                &tc.name,
                executor_is_cloud,
            )
            .await;

            if (tc.name == "write_file" || tc.name == "edit_file") && !exec_res.is_error {
                produced_checkpoint = true;
            }

            all_tools.push(ToolCallRecord {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments_json: tc.arguments.to_string(),
                status: if exec_res.is_error {
                    "error".into()
                } else {
                    "completed".into()
                },
                output: exec_res.output.clone(),
                diff: exec_res.diff,
                file_path: exec_res.file_path,
                risk_tier: Some(risk_tier),
                subagent_summary: None,
                subagent_model: None,
            });
            exec_history.push(ProviderMessage {
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
                    "orchestrator_executor",
                    &format!(
                        "Orchestrator Phase 2/2 · Executor ({executor_label}) · Turn {} · Completed `{}`",
                        step + 1,
                        tc.name
                    ),
                    step + 1,
                    max_exec_steps,
                    &executor_model,
                    &accumulated_thoughts.join("\n\n"),
                    &exec_content,
                    &all_tools,
                )
                .await;
        }

        if let Some(pending) = paused {
            let exec_phase = OrchestratorPhaseRecord {
                phase: "executor".into(),
                model_id: executor_model.clone(),
                summary: format!("Paused for review on `{}`", pending.tool_name),
                latency_ms: exec_latency,
                tokens: exec_tokens,
            };
            let msg = ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: "assistant".into(),
                content: format!(
                    "Orchestrator Executor (**{executor_label}**) requires review before running `{}`.",
                    pending.tool_name
                ),
                thinking: if accumulated_thoughts.is_empty() {
                    None
                } else {
                    Some(accumulated_thoughts.join("\n\n"))
                },
                model_id: Some(format!("{planner_model} → {executor_model}")),
                latency_ms: Some(planner_latency + exec_latency),
                prompt_tokens: Some(planner_tokens + exec_tokens),
                completion_tokens: Some(0),
                tool_calls: all_tools,
                compare_partner: None,
                checkpoint_id: if produced_checkpoint {
                    Some(checkpoint_id)
                } else {
                    None
                },
                plan_artifact: Some(plan.clone()),
                orchestrator_phases: vec![planner_phase, exec_phase],
                attached_files,
                attachments: Vec::new(),
                artifact_id: plan_artifact_id.clone(),
                fallback_from_model_id: None,
                fallback_reason: None,
                created_at: Utc::now(),
            };
            let updated = ctx
                .store
                .update(|s| {
                    if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                        sx.active_plan = Some(plan);
                        sx.pending_approval = Some(pending);
                        sx.messages.push(msg);
                        sx.updated_at = Utc::now();
                        return Some(sx.clone());
                    }
                    None
                })
                .await?
                .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
            return Ok(updated);
        }
    }

    if !completed_cleanly && max_exec_steps > 0 {
        let pq = build_step_contingent_question(
            max_exec_steps,
            &executor_model,
            serde_json::to_string(&exec_history)?,
            max_exec_steps - 1,
        );
        let exec_phase = OrchestratorPhaseRecord {
            phase: "executor".into(),
            model_id: executor_model.clone(),
            summary: format!("Paused at {max_exec_steps}-step contingent"),
            latency_ms: exec_latency,
            tokens: exec_tokens,
        };
        let msg = ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: if exec_content.trim().is_empty() {
                format!(
                    "Orchestrator Executor (**{executor_label}**) paused after {max_exec_steps} autonomous steps — select additional step contingent below to continue."
                )
            } else {
                format!(
                    "{exec_content}\n\nPaused after {max_exec_steps} autonomous steps — select additional step contingent below to continue."
                )
            },
            thinking: if accumulated_thoughts.is_empty() {
                None
            } else {
                Some(accumulated_thoughts.join("\n\n"))
            },
            model_id: Some(format!("{planner_model} → {executor_model}")),
            latency_ms: Some(planner_latency + exec_latency),
            prompt_tokens: Some(planner_tokens + exec_tokens),
            completion_tokens: Some(0),
            tool_calls: all_tools,
            compare_partner: None,
            checkpoint_id: if produced_checkpoint {
                Some(checkpoint_id)
            } else {
                None
            },
            plan_artifact: Some(plan.clone()),
            orchestrator_phases: {
                let mut phases = vec![planner_phase, exec_phase];
                phases.extend(extra_phases);
                phases
            },
            attached_files,
            attachments: Vec::new(),
            artifact_id: plan_artifact_id.clone(),
            fallback_from_model_id: None,
            fallback_reason: None,
            created_at: Utc::now(),
        };
        let updated = ctx
            .store
            .update(|s| {
                if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                    sx.active_plan = Some(plan);
                    sx.pending_question = Some(pq);
                    sx.messages.push(msg);
                    sx.updated_at = Utc::now();
                    return Some(sx.clone());
                }
                None
            })
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        return Ok(updated);
    }

    plan.status = PlanStatus::Executed;
    let exec_phase = OrchestratorPhaseRecord {
        phase: "executor".into(),
        model_id: executor_model.clone(),
        summary: "Execution completed".into(),
        latency_ms: exec_latency,
        tokens: exec_tokens,
    };

    let msg = ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: "assistant".into(),
        content: if exec_content.trim().is_empty() {
            format!("Orchestrator finished executing **{}**.", plan.title)
        } else {
            exec_content
        },
        thinking: if accumulated_thoughts.is_empty() {
            None
        } else {
            Some(accumulated_thoughts.join("\n\n"))
        },
        model_id: Some(format!("{planner_model} → {executor_model}")),
        latency_ms: Some(planner_latency + exec_latency),
        prompt_tokens: Some(planner_tokens + exec_tokens),
        completion_tokens: Some(0),
        tool_calls: all_tools,
        compare_partner: None,
        checkpoint_id: if produced_checkpoint {
            Some(checkpoint_id)
        } else {
            None
        },
        plan_artifact: Some(plan.clone()),
        orchestrator_phases: {
            let mut phases = vec![planner_phase, exec_phase];
            phases.extend(extra_phases);
            phases
        },
        attached_files,
        attachments: Vec::new(),
        artifact_id: plan_artifact_id,
        fallback_from_model_id: None,
        fallback_reason: None,
        created_at: Utc::now(),
    };

    let updated = ctx
        .store
        .update(|s| {
            if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                sx.active_plan = Some(plan);
                sx.messages.push(msg);
                sx.updated_at = Utc::now();
                return Some(sx.clone());
            }
            None
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_models::{
        FitTier, LocalModelEntry, LocalModelSource, ServerManager, VramFitAssessment,
    };
    use harness_net::{NetworkGate, PrivacyMode};
    use harness_store::{BumboPaths, BumboStore};

    #[test]
    fn resolve_reachable_model_falls_back_to_local_when_cloud_unconfigured() {
        let dir = std::env::temp_dir().join(format!("bumbo_orch_test_{}", Uuid::new_v4()));
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let gate = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let srv = ServerManager::new(dir.join("engines"));
        let locals = vec![LocalModelEntry {
            id: "local::qwen3.5:9b".into(),
            name: "qwen3.5:9b".into(),
            source: LocalModelSource::OllamaBlob,
            gguf_path: "dummy.gguf".into(),
            ollama_tag: Some("qwen3.5:9b".into()),
            mmproj_path: None,
            size_bytes: 6_000_000_000,
            quant: "Q4_K_M".into(),
            is_embedding: false,
            is_valid_gguf: true,
            vram_fit: VramFitAssessment {
                tier: FitTier::FitsInVram,
                label: "Fits".into(),
                weights_mb: 5700,
                kv_cache_mb: 1000,
                compute_buffer_mb: 600,
                estimated_total_mb: 7300,
                recommended_ngl: 99,
            },
        }];

        let ctx = HarnessContext {
            gate: &gate,
            store: &store,
            server_mgr: &srv,
            local_models: &locals,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let resolved =
                resolve_reachable_model(&ctx, "gemini-3-flash-preview", "local::qwen3.5:9b")
                    .await
                    .unwrap();
            assert_eq!(resolved, "local::qwen3.5:9b");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_auto_executor_prefers_cloud_in_hybrid_and_local_in_strict_local() {
        let dir = std::env::temp_dir().join(format!("bumbo_auto_exec_{}", Uuid::new_v4()));
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let srv = ServerManager::new(dir.join("engines"));
        let locals = vec![LocalModelEntry {
            id: "local::qwen3.5:9b".into(),
            name: "qwen3.5:9b".into(),
            source: LocalModelSource::OllamaBlob,
            gguf_path: "dummy.gguf".into(),
            ollama_tag: Some("qwen3.5:9b".into()),
            mmproj_path: None,
            size_bytes: 6_000_000_000,
            quant: "Q4_K_M".into(),
            is_embedding: false,
            is_valid_gguf: true,
            vram_fit: VramFitAssessment {
                tier: FitTier::FitsInVram,
                label: "Fits".into(),
                weights_mb: 5700,
                kv_cache_mb: 1000,
                compute_buffer_mb: 600,
                estimated_total_mb: 7300,
                recommended_ngl: 99,
            },
        }];

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            store
                .update(|s| {
                    s.settings.api_keys.gemini_api_key = "test-gem-key".into();
                })
                .await
                .unwrap();

            let gate_local = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
            let ctx_local = HarnessContext {
                gate: &gate_local,
                store: &store,
                server_mgr: &srv,
                local_models: &locals,
            };
            let picked_local =
                resolve_auto_executor_model(&ctx_local, "local::gemma4:12b")
                    .await
                    .unwrap();
            assert_eq!(picked_local, "local::qwen3.5:9b");

            let gate_hybrid = NetworkGate::new(PrivacyMode::Hybrid).unwrap();
            let ctx_hybrid = HarnessContext {
                gate: &gate_hybrid,
                store: &store,
                server_mgr: &srv,
                local_models: &locals,
            };
            let picked_hybrid =
                resolve_auto_executor_model(&ctx_hybrid, "local::gemma4:12b")
                    .await
                    .unwrap();
            assert!(
                !picked_hybrid.starts_with("local::") && picked_hybrid.contains("gemini"),
                "expected cloud gemini model in Hybrid mode, got: {picked_hybrid}"
            );
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_auto_orchestrator_anchors_to_active_cloud_model_and_fallback_chain() {
        let dir = std::env::temp_dir().join(format!("bumbo_orch_anchor_{}", Uuid::new_v4()));
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let srv = ServerManager::new(dir.join("engines"));
        let gate_hybrid = NetworkGate::new(PrivacyMode::Hybrid).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            store
                .update(|s| {
                    s.settings.api_keys.gemini_api_key = "gem-key".into();
                    s.settings.api_keys.anthropic_api_key = "ant-key".into();
                    s.settings.fallback_chain = vec!["claude-sonnet-4-5".into()];
                })
                .await
                .unwrap();

            let ctx = HarnessContext {
                gate: &gate_hybrid,
                store: &store,
                server_mgr: &srv,
                local_models: &[],
            };

            // Anchors to active cloud Chat model rather than jumping to Gemini 3.0
            let planner = resolve_auto_planner_model(&ctx, "claude-sonnet-4-5")
                .await
                .unwrap();
            assert_eq!(planner, "claude-sonnet-4-5");
            let executor = resolve_auto_executor_model(&ctx, &planner).await.unwrap();
            assert_eq!(executor, "claude-sonnet-4-5");

            // When active Chat model is local, fallback_chain comes before default ranking
            let planner_chain = resolve_auto_planner_model(&ctx, "local::gemma4:12b")
                .await
                .unwrap();
            assert_eq!(planner_chain, "claude-sonnet-4-5");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
