use futures_util::StreamExt;
use harness_models::{ActiveServerBackend, LocalModelEntry, ServerManager};
use harness_net::NetworkGate;
use harness_store::{
    is_viable_gemini_chat_model, sort_gemini_models_by_priority, BumboStore, FallbackPolicy,
    ModelCatalogEntry, ModelQuotaLimit, ProviderKind,
};
use reqwest::header::{HeaderMap, CONNECTION};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    #[serde(default)]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMessage {
    pub role: String, // "user" | "assistant" | "tool"
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    #[serde(default)]
    pub images: Vec<(String, String)>, // (mime_type, data_base64)
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model_id: String,
    pub system_prompt: String,
    pub messages: Vec<ProviderMessage>,
    pub tools: Vec<ToolSpec>,
    pub enable_thinking: bool,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub thinking: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub thinking_tokens: u64,
    pub latency_ms: u64,
    /// The actual model ID that generated this response (may differ if auto-switched).
    #[serde(default)]
    pub actual_model_id: String,
    /// Original model ID if Cinder automatically switched away from a failing/rate-limited model.
    #[serde(default)]
    pub switched_from: Option<String>,
    /// Human-readable reason for automatic model switching.
    #[serde(default)]
    pub switch_reason: Option<String>,
}

async fn complete_single(
    gate: &NetworkGate,
    store: &BumboStore,
    settings: &harness_store::AppSettings,
    server_mgr: &ServerManager,
    local_models: &[LocalModelEntry],
    catalog: &[ModelCatalogEntry],
    req: &CompletionRequest,
) -> anyhow::Result<(CompletionResponse, Option<RateHeaders>)> {
    // 1. Local model routing
    if req.model_id.starts_with("local::") {
        let target_local = local_models
            .iter()
            .find(|m| m.id == req.model_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Local model '{}' was not found in the vault or Ollama blobs.",
                    req.model_id
                )
            })?;

        let status = server_mgr.server_status().await;
        if !status.running || status.model_id.as_deref() != Some(&target_local.id) {
            server_mgr
                .start_local_model(
                    gate,
                    target_local,
                    settings.local_server_port,
                    settings.local_ctx_size,
                    settings.local_gpu_layers,
                    settings.expose_local_api,
                )
                .await?;
        }

        let (backend, endpoint, token, model_name) = server_mgr
            .active_endpoint_and_token()
            .await
            .ok_or_else(|| anyhow::anyhow!("Local model server is not active."))?;

        let active_status = server_mgr.server_status().await;
        let has_images = req.messages.iter().any(|m| !m.images.is_empty());
        let adjusted_req;
        let effective_req = if backend == ActiveServerBackend::LlamaServer
            && has_images
            && !active_status.has_mmproj
        {
            adjusted_req = strip_images_with_vision_hint(
                req,
                "active local GGUF model has no companion --mmproj vision projector",
            );
            &adjusted_req
        } else {
            req
        };

        let mut resp = match backend {
            ActiveServerBackend::LlamaServer => {
                match complete_openai_compat(
                    gate,
                    store,
                    &format!("{endpoint}/v1/chat/completions"),
                    token.as_deref(),
                    &model_name,
                    effective_req,
                    "local_inference",
                )
                .await
                {
                    Ok(ok) => ok,
                    Err(first_err) if is_local_transport_error(&first_err.to_string()) => {
                        let st = server_mgr.server_status().await;
                        if !st.running {
                            server_mgr
                                .start_local_model(
                                    gate,
                                    target_local,
                                    settings.local_server_port,
                                    settings.local_ctx_size,
                                    settings.local_gpu_layers,
                                    settings.expose_local_api,
                                )
                                .await?;
                        } else {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                        let (next_backend, next_endpoint, next_token, next_model_name) =
                            server_mgr
                                .active_endpoint_and_token()
                                .await
                                .ok_or(first_err)?;
                        if next_backend != ActiveServerBackend::LlamaServer {
                            anyhow::bail!(
                                "Local model backend changed unexpectedly during recovery."
                            );
                        }
                        let retry_resp = complete_openai_compat(
                            gate,
                            store,
                            &format!("{next_endpoint}/v1/chat/completions"),
                            next_token.as_deref(),
                            &next_model_name,
                            effective_req,
                            "local_inference",
                        )
                        .await;
                        match retry_resp {
                            Ok(ok) => ok,
                            Err(retry_err) => {
                                let post_status = server_mgr.server_status().await;
                                if let Some(diag) = post_status.last_error {
                                    anyhow::bail!("{retry_err} ({diag})");
                                }
                                return Err(retry_err);
                            }
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            ActiveServerBackend::OllamaLoopback => {
                complete_ollama_loopback(gate, store, &endpoint, &model_name, effective_req)
                    .await?
            }
            ActiveServerBackend::None => anyhow::bail!("Local model backend not initialized."),
        };
        resp.actual_model_id = req.model_id.clone();
        return Ok((resp, None));
    }

    // 2. Cloud catalog routing
    let entry = catalog.iter().find(|c| c.id == req.model_id);
    let cloud_supports_vision = entry.map(|e| e.supports_vision).unwrap_or(true);
    let cloud_adjusted_req;
    let effective_cloud_req = if !cloud_supports_vision
        && req.messages.iter().any(|m| !m.images.is_empty())
    {
        cloud_adjusted_req = strip_images_with_vision_hint(req, "selected text-only cloud model");
        &cloud_adjusted_req
    } else {
        req
    };
    let provider = entry
        .map(|e| e.provider)
        .unwrap_or_else(|| infer_provider_from_id(&effective_cloud_req.model_id));

    let (mut resp, rate_headers) = match provider {
        ProviderKind::GoogleGemini => {
            if settings.api_keys.gemini_api_key.trim().is_empty() {
                anyhow::bail!(
                    "Gemini API key is not configured. Open Settings -> Cloud Keys & Models to add your Google AI / Gemini API key (or switch to a Local model)."
                );
            }
            let supports_thinking = entry.map(|e| e.supports_thinking).unwrap_or_else(|| {
                let lower = effective_cloud_req.model_id.to_ascii_lowercase();
                lower.contains("thinking") || lower.contains("gemini-3-flash")
            });
            let supports_tools = entry.map(|e| e.supports_tools).unwrap_or_else(|| {
                !effective_cloud_req
                    .model_id
                    .to_ascii_lowercase()
                    .starts_with("gemma")
            });
            complete_gemini(
                gate,
                store,
                &settings.api_keys.gemini_api_key,
                effective_cloud_req,
                supports_thinking,
                supports_tools,
            )
            .await?
        }
        ProviderKind::Anthropic => {
            if settings.api_keys.anthropic_api_key.trim().is_empty() {
                anyhow::bail!("Anthropic API key is not configured. Add it in Settings -> Cloud Keys & Models.");
            }
            complete_anthropic(
                gate,
                store,
                &settings.api_keys.anthropic_api_key,
                effective_cloud_req,
            )
            .await?
        }
        ProviderKind::OpenRouter => {
            if settings.api_keys.openrouter_api_key.trim().is_empty() {
                anyhow::bail!("OpenRouter API key is not configured. Add it in Settings -> Cloud Keys & Models.");
            }
            complete_openai_compat_with_headers(
                gate,
                store,
                "https://openrouter.ai/api/v1/chat/completions",
                Some(settings.api_keys.openrouter_api_key.trim()),
                &effective_cloud_req.model_id,
                effective_cloud_req,
                "openrouter_api",
            )
            .await?
        }
        ProviderKind::OpenAiCompat => {
            let base = settings.api_keys.openai_compat_base_url.trim().trim_end_matches('/');
            if base.is_empty() {
                anyhow::bail!("Custom OpenAI-compatible Base URL is not configured.");
            }
            let key = settings.api_keys.openai_compat_api_key.trim();
            complete_openai_compat_with_headers(
                gate,
                store,
                &format!("{base}/chat/completions"),
                if key.is_empty() { None } else { Some(key) },
                &effective_cloud_req.model_id,
                effective_cloud_req,
                "openai_compat_api",
            )
            .await?
        }
        ProviderKind::LocalLlama | ProviderKind::LocalOllama => {
            anyhow::bail!("Local provider must use local:: model prefix.");
        }
    };

    resp.actual_model_id = req.model_id.clone();
    Ok((resp, Some(rate_headers)))
}

fn strip_images_with_vision_hint(req: &CompletionRequest, reason: &str) -> CompletionRequest {
    let mut cloned = req.clone();
    for m in &mut cloned.messages {
        if !m.images.is_empty() {
            let count = m.images.len();
            m.images.clear();
            m.content.push_str(&format!(
                "\n[Attached image(s) ({count}) cannot be viewed directly by this model ({reason}). Call the `delegate_vision` tool with a clear `prompt` to inspect them via the Vision Sub-Agent.]"
            ));
        }
    }
    cloned
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackChoicePrompt {
    pub failed_model_id: String,
    pub error_summary: String,
    pub reason: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum CompletionOutcome {
    Completed(CompletionResponse),
    NeedsFallbackChoice(FallbackChoicePrompt),
}

pub async fn complete(
    gate: &NetworkGate,
    store: &BumboStore,
    server_mgr: &ServerManager,
    local_models: &[LocalModelEntry],
    req: CompletionRequest,
) -> anyhow::Result<CompletionResponse> {
    match complete_interactive(gate, store, server_mgr, local_models, req).await? {
        CompletionOutcome::Completed(resp) => Ok(resp),
        CompletionOutcome::NeedsFallbackChoice(prompt) => {
            anyhow::bail!(
                "Model '{}' failed ({}). Retry or select another model.",
                prompt.failed_model_id,
                prompt.error_summary
            )
        }
    }
}

pub async fn complete_interactive(
    gate: &NetworkGate,
    store: &BumboStore,
    server_mgr: &ServerManager,
    local_models: &[LocalModelEntry],
    req: CompletionRequest,
) -> anyhow::Result<CompletionOutcome> {
    let settings = store.snapshot().await.settings;
    let (catalog, _) = store.build_cloud_catalog_and_quotas().await;

    let first_attempt = complete_single(
        gate,
        store,
        &settings,
        server_mgr,
        local_models,
        &catalog,
        &req,
    )
    .await;

    let (resp, maybe_rate_headers, usage_model_id) = match first_attempt {
        Ok((resp, headers)) => (resp, headers, req.model_id.clone()),
        Err(first_err) => {
            let raw_err = first_err.to_string();
            let (should_switch, should_block, reason) = classify_provider_error(&raw_err);
            let is_local = req.model_id.starts_with("local::");

            if should_block && !is_local {
                let _ = store.mark_model_blocked(&req.model_id, reason).await;
            }

            // Always try a single transient retry on the user's chosen primary model BEFORE switching or prompting.
            if !is_local && should_switch && !should_block {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let (retry_catalog, _) = store.build_cloud_catalog_and_quotas().await;
                if let Ok((retry_resp, retry_headers)) = complete_single(
                    gate,
                    store,
                    &settings,
                    server_mgr,
                    local_models,
                    &retry_catalog,
                    &req,
                )
                .await
                {
                    if let Some(headers) = retry_headers {
                        let _ = store
                            .record_usage(
                                &req.model_id,
                                retry_resp.prompt_tokens,
                                retry_resp.completion_tokens,
                                retry_resp.thinking_tokens,
                                headers.rem_req,
                                headers.lim_req,
                                headers.rem_tok,
                                headers.lim_tok,
                            )
                            .await;
                    }
                    return Ok(CompletionOutcome::Completed(retry_resp));
                }
            }

            let effective_policy = if !settings.auto_switch_models {
                FallbackPolicy::Strict
            } else {
                settings.fallback_policy
            };

            if !should_switch || effective_policy == FallbackPolicy::Strict {
                anyhow::bail!("{}", format_clean_provider_error(&raw_err));
            }

            let failed_provider = if is_local {
                ProviderKind::LocalLlama
            } else {
                catalog
                    .iter()
                    .find(|c| c.id == req.model_id)
                    .map(|e| e.provider)
                    .unwrap_or_else(|| infer_provider_from_id(&req.model_id))
            };

            let (fresh_catalog, _) = store.build_cloud_catalog_and_quotas().await;
            let needs_tools = !req.tools.is_empty();
            let needs_vision = req.messages.iter().any(|m| !m.images.is_empty());
            let allow_cloud = gate.mode() == harness_net::PrivacyMode::Hybrid;

            if effective_policy == FallbackPolicy::Ask {
                let candidates = build_fallback_candidates_for_request(
                    &req.model_id,
                    failed_provider,
                    &fresh_catalog,
                    local_models,
                    &settings.fallback_chain,
                    true,
                    settings.fallback_to_local,
                    allow_cloud,
                    needs_tools,
                    needs_vision,
                );
                return Ok(CompletionOutcome::NeedsFallbackChoice(FallbackChoicePrompt {
                    failed_model_id: req.model_id.clone(),
                    error_summary: format_clean_provider_error(&raw_err),
                    reason: reason.to_string(),
                    candidates: candidates.into_iter().take(3).collect(),
                }));
            }

            // FallbackPolicy::AutoChain: obey user's explicit chain if configured; otherwise auto-discover.
            let allow_auto = settings.fallback_chain.is_empty();
            let candidates = build_fallback_candidates_for_request(
                &req.model_id,
                failed_provider,
                &fresh_catalog,
                local_models,
                &settings.fallback_chain,
                allow_auto,
                settings.fallback_to_local,
                allow_cloud,
                needs_tools,
                needs_vision,
            );

            if candidates.is_empty() {
                anyhow::bail!("{}", format_clean_provider_error(&raw_err));
            }

            let mut last_fb_err = String::new();
            let mut switched_ok: Option<(CompletionResponse, Option<RateHeaders>, String)> = None;

            for cand_id in candidates.into_iter().take(3) {
                let mut fb_req = req.clone();
                fb_req.model_id = cand_id.clone();
                match complete_single(
                    gate,
                    store,
                    &settings,
                    server_mgr,
                    local_models,
                    &fresh_catalog,
                    &fb_req,
                )
                .await
                {
                    Ok((mut fb_resp, fb_headers)) => {
                        let note = format!(
                            "[Fallback] Model '{}' hit {} — continued this turn on '{}'.",
                            req.model_id, reason, cand_id
                        );
                        fb_resp.thinking = Some(match fb_resp.thinking.take() {
                            Some(prev) if !prev.trim().is_empty() => format!("{note}\n\n{prev}"),
                            _ => note,
                        });
                        fb_resp.actual_model_id = cand_id.clone();
                        fb_resp.switched_from = Some(req.model_id.clone());
                        fb_resp.switch_reason = Some(reason.to_string());
                        switched_ok = Some((fb_resp, fb_headers, cand_id));
                        break;
                    }
                    Err(fb_err) => {
                        let fb_raw = fb_err.to_string();
                        let (_, fb_block, fb_reason) = classify_provider_error(&fb_raw);
                        if fb_block && !cand_id.starts_with("local::") {
                            let _ = store.mark_model_blocked(&cand_id, fb_reason).await;
                        }
                        last_fb_err = format!(
                            "{cand_id}: {}",
                            format_clean_provider_error(&fb_raw)
                        );
                    }
                }
            }

            match switched_ok {
                Some(ok) => ok,
                None => anyhow::bail!(
                    "Model '{}' failed ({}) and fallback chain failed ({})",
                    req.model_id,
                    format_clean_provider_error(&raw_err),
                    last_fb_err
                ),
            }
        }
    };

    if let Some(headers) = maybe_rate_headers {
        store
            .record_usage(
                &usage_model_id,
                resp.prompt_tokens,
                resp.completion_tokens,
                resp.thinking_tokens,
                headers.rem_req,
                headers.lim_req,
                headers.rem_tok,
                headers.lim_tok,
            )
            .await?;
    }

    Ok(CompletionOutcome::Completed(resp))
}

pub fn is_local_transport_error(err_str: &str) -> bool {
    let lower = err_str.to_ascii_lowercase();
    lower.contains("connection")
        || lower.contains("error sending request")
        || lower.contains("channel closed")
        || lower.contains("eof")
        || lower.contains("broken pipe")
        || lower.contains("reset")
}

pub fn classify_provider_error(err_str: &str) -> (bool, bool, &'static str) {
    let lower = err_str.to_ascii_lowercase();
    if lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("too many tokens")
        || lower.contains("prompt is too long")
        || lower.contains("context_length_exceeded")
    {
        return (false, false, "context length exceeded");
    }
    if err_str.contains("HTTP 402")
        || lower.contains("payment required")
        || lower.contains("insufficient credits")
        || lower.contains("credit balance is too low")
        || lower.contains("requires more credits")
    {
        return (true, true, "requires paid credits / payment required");
    }
    if lower.contains("limit: 0")
        || err_str.contains("HTTP 404")
        || err_str.contains("NOT_FOUND")
        || lower.contains("no endpoints found")
        || lower.contains("not a valid model")
        || lower.contains("model not found")
    {
        return (true, true, "0 quota / unavailable on current API tier");
    }
    if err_str.contains("HTTP 429")
        || err_str.contains("RESOURCE_EXHAUSTED")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("quota exceeded")
    {
        return (true, false, "rate limit / quota exhausted");
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return (true, false, "request timed out");
    }
    if is_local_transport_error(err_str) {
        return (true, false, "connection dropped / reset");
    }
    if err_str.contains("HTTP 500")
        || err_str.contains("HTTP 503")
        || err_str.contains("HTTP 529")
        || err_str.contains("HTTP 502")
        || err_str.contains("HTTP 504")
        || lower.contains("overloaded")
        || lower.contains("provider returned error")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
        || err_str.contains("UNAVAILABLE")
        || err_str.contains("INTERNAL")
    {
        return (true, false, "provider overloaded / temporarily unavailable");
    }
    if err_str.contains("HTTP 401") || err_str.contains("HTTP 403") {
        return (true, false, "authentication / access denied");
    }
    (false, false, "provider error")
}

fn is_vision_capable_local_entry(m: &LocalModelEntry) -> bool {
    if m.mmproj_path.is_some() {
        return true;
    }
    let l = m.id.to_ascii_lowercase();
    l.contains("gemma4")
        || l.contains("gemma-4")
        || l.contains("gemma3")
        || l.contains("gemma-3")
        || l.contains("qwen3.5")
        || l.contains("qwen2.5-vl")
        || l.contains("qwen2-vl")
        || l.contains("-vl")
        || l.contains("vision")
        || l.contains("llava")
        || l.contains("minicpm-v")
        || l.contains("pixtral")
}

#[allow(clippy::too_many_arguments)]
pub fn build_fallback_candidates_for_request(
    failed_id: &str,
    failed_provider: ProviderKind,
    catalog: &[ModelCatalogEntry],
    local_models: &[LocalModelEntry],
    fallback_chain: &[String],
    allow_auto_discovery: bool,
    fallback_to_local: bool,
    allow_cloud: bool,
    needs_tools: bool,
    needs_vision: bool,
) -> Vec<String> {
    const GEMINI_PRIORITY: &[&str] = &[
        "gemini-3-flash-preview",
        "gemini-3.5-flash-lite",
        "gemini-3.1-flash-lite",
        "gemini-3.6-flash",
        "gemini-flash-lite-latest",
        "gemini-3.1-flash-lite-preview",
        "gemma-4-26b-a4b-it",
    ];

    let is_cloud_eligible = |c: &ModelCatalogEntry| -> bool {
        allow_cloud
            && c.reachable
            && !c.requires_payment
            && (!needs_tools || c.supports_tools)
            && (!needs_vision || c.supports_vision)
    };

    let is_id_eligible = |id: &str| -> bool {
        let tid = id.trim();
        if tid.is_empty() || tid == failed_id {
            return false;
        }
        if tid.starts_with("local::") {
            local_models.iter().any(|m| {
                m.id == tid
                    && !m.is_embedding
                    && m.is_valid_gguf
                    && (!needs_vision || is_vision_capable_local_entry(m))
            })
        } else {
            catalog.iter().any(|c| c.id == tid && is_cloud_eligible(c))
        }
    };

    let mut out: Vec<String> = Vec::new();
    let mut push_unique = |id: &str| {
        let tid = id.trim();
        if !tid.is_empty() && tid != failed_id && !out.iter().any(|x| x == tid) {
            out.push(tid.to_string());
        }
    };

    // 1. User's explicit ordered fallback chain comes strictly first.
    for cand in fallback_chain {
        if is_id_eligible(cand) {
            push_unique(cand);
        }
    }

    if !allow_auto_discovery {
        return out;
    }

    // 2. Same-provider free reachable cloud fallbacks
    if allow_cloud {
        if failed_provider == ProviderKind::GoogleGemini {
            for pref in GEMINI_PRIORITY {
                if catalog
                    .iter()
                    .any(|c| c.id == *pref && c.provider == ProviderKind::GoogleGemini && is_cloud_eligible(c))
                {
                    push_unique(pref);
                }
            }
        }
        for c in catalog
            .iter()
            .filter(|c| c.provider == failed_provider && is_cloud_eligible(c))
        {
            push_unique(&c.id);
        }

        // 3. Cross-provider reachable free cloud models
        let provider_order = [
            ProviderKind::GoogleGemini,
            ProviderKind::Anthropic,
            ProviderKind::OpenRouter,
            ProviderKind::OpenAiCompat,
        ];
        for p in provider_order {
            if p == failed_provider {
                continue;
            }
            if p == ProviderKind::GoogleGemini {
                for pref in GEMINI_PRIORITY {
                    if catalog.iter().any(|c| {
                        c.id == *pref && c.provider == ProviderKind::GoogleGemini && is_cloud_eligible(c)
                    }) {
                        push_unique(pref);
                    }
                }
            }
            for c in catalog
                .iter()
                .filter(|c| c.provider == p && is_cloud_eligible(c))
            {
                push_unique(&c.id);
            }
        }
    }

    // 4. Optional fallback to local GGUF/Ollama models (only when opted in)
    if fallback_to_local {
        let mut locals: Vec<&LocalModelEntry> = local_models
            .iter()
            .filter(|m| {
                !m.is_embedding
                    && m.is_valid_gguf
                    && m.id != failed_id
                    && (!needs_vision || is_vision_capable_local_entry(m))
            })
            .collect();
        locals.sort_by_key(|m| m.vram_fit.estimated_total_mb);
        for m in locals {
            push_unique(&m.id);
        }
    }

    out
}

pub fn build_fallback_candidates(
    failed_id: &str,
    failed_provider: ProviderKind,
    catalog: &[ModelCatalogEntry],
    local_models: &[LocalModelEntry],
    fallback_to_local: bool,
) -> Vec<String> {
    build_fallback_candidates_for_request(
        failed_id,
        failed_provider,
        catalog,
        local_models,
        &[],
        true,
        fallback_to_local,
        true,
        false,
        false,
    )
}

pub fn format_clean_provider_error(raw: &str) -> String {
    if let Some(json_start) = raw.find('{') {
        let prefix = raw[..json_start].trim().trim_end_matches(':').trim();
        let json_part = &raw[json_start..];
        if let Ok(val) = serde_json::from_str::<Value>(json_part) {
            if let Some(msg) = val["error"]["message"].as_str() {
                let first_line = msg.lines().next().unwrap_or(msg).trim();
                let short_msg: String = first_line.chars().take(240).collect();
                return if prefix.is_empty() {
                    short_msg
                } else {
                    format!("{prefix}: {short_msg}")
                };
            }
        }
    }
    raw.chars().take(280).collect()
}

fn infer_provider_from_id(id: &str) -> ProviderKind {
    let lower = id.to_ascii_lowercase();
    if lower.starts_with("gemini-") || lower.starts_with("gemma-") {
        ProviderKind::GoogleGemini
    } else if lower.starts_with("claude-") {
        ProviderKind::Anthropic
    } else if lower.contains('/') {
        ProviderKind::OpenRouter
    } else {
        ProviderKind::OpenAiCompat
    }
}

#[derive(Default)]
struct RateHeaders {
    rem_req: Option<u64>,
    lim_req: Option<u64>,
    rem_tok: Option<u64>,
    lim_tok: Option<u64>,
}

fn extract_rate_headers(headers: &HeaderMap) -> RateHeaders {
    let parse_h = |names: &[&str]| -> Option<u64> {
        for n in names {
            if let Some(val) = headers.get(*n).and_then(|v| v.to_str().ok()) {
                if let Ok(parsed) = val.trim().parse::<u64>() {
                    return Some(parsed);
                }
            }
        }
        None
    };

    RateHeaders {
        rem_req: parse_h(&[
            "x-ratelimit-remaining-requests",
            "anthropic-ratelimit-requests-remaining",
        ]),
        lim_req: parse_h(&[
            "x-ratelimit-limit-requests",
            "anthropic-ratelimit-requests-limit",
        ]),
        rem_tok: parse_h(&[
            "x-ratelimit-remaining-tokens",
            "anthropic-ratelimit-tokens-remaining",
        ]),
        lim_tok: parse_h(&[
            "x-ratelimit-limit-tokens",
            "anthropic-ratelimit-tokens-limit",
        ]),
    }
}

async fn complete_ollama_loopback(
    gate: &NetworkGate,
    store: &BumboStore,
    endpoint: &str,
    model_name: &str,
    req: &CompletionRequest,
) -> anyhow::Result<CompletionResponse> {
    let start = Instant::now();
    let url = format!("{endpoint}/api/chat");

    let mut msgs = Vec::new();
    if !req.system_prompt.trim().is_empty() {
        msgs.push(json!({ "role": "system", "content": req.system_prompt }));
    }
    for m in &req.messages {
        if m.role == "assistant" && !m.tool_calls.is_empty() {
            let tcs: Vec<Value> = m
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }
                    })
                })
                .collect();
            msgs.push(json!({
                "role": "assistant",
                "content": m.content,
                "tool_calls": tcs,
            }));
        } else if m.role == "tool" {
            msgs.push(json!({
                "role": "tool",
                "content": m.content,
                "tool_name": m.tool_name.clone().unwrap_or_default(),
            }));
        } else if m.images.is_empty() {
            msgs.push(json!({
                "role": m.role,
                "content": m.content,
            }));
        } else {
            let imgs: Vec<&str> = m.images.iter().map(|(_, b64)| b64.as_str()).collect();
            msgs.push(json!({
                "role": m.role,
                "content": m.content,
                "images": imgs,
            }));
        }
    }

    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters_schema,
                }
            })
        })
        .collect();

    let mut body = json!({
        "model": model_name,
        "messages": msgs,
        "stream": true,
        "options": {
            "temperature": req.temperature,
            "num_predict": req.max_tokens,
        }
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }

    let resp = gate
        .request(Method::POST, &url, "local_ollama_inference")?
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Local Ollama returned {status}: {text}");
    }

    let mut raw_content = String::new();
    let mut raw_thinking = String::new();
    let mut tool_calls = Vec::new();
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;
    let mut buf: Vec<u8> = Vec::new();
    let mut last_flush = Instant::now();
    let mut stream = resp.bytes_stream();

    let process_line = |line: &str,
                        raw_content: &mut String,
                        raw_thinking: &mut String,
                        tool_calls: &mut Vec<ToolCallRequest>,
                        prompt_tokens: &mut u64,
                        completion_tokens: &mut u64|
     -> bool {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return false;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            return false;
        };
        let mut updated = false;
        if let Some(c) = val["message"]["content"].as_str() {
            if !c.is_empty() {
                raw_content.push_str(c);
                updated = true;
            }
        }
        if let Some(t) = val["message"]["thinking"].as_str() {
            if !t.is_empty() {
                raw_thinking.push_str(t);
                updated = true;
            }
        }
        if let Some(arr) = val["message"]["tool_calls"].as_array() {
            for tc in arr {
                if let Some(name) = tc["function"]["name"].as_str() {
                    let args = tc["function"]["arguments"].clone();
                    tool_calls.push(ToolCallRequest {
                        id: Uuid::new_v4().to_string(),
                        name: name.to_string(),
                        arguments: args,
                        thought_signature: None,
                    });
                }
            }
        }
        if let Some(pe) = val["prompt_eval_count"].as_u64() {
            *prompt_tokens = pe;
        }
        if let Some(ec) = val["eval_count"].as_u64() {
            *completion_tokens = ec;
        }
        updated
    };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
        let mut any_delta = false;
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            if process_line(
                &line,
                &mut raw_content,
                &mut raw_thinking,
                &mut tool_calls,
                &mut prompt_tokens,
                &mut completion_tokens,
            ) {
                any_delta = true;
            }
        }
        if any_delta && last_flush.elapsed() >= Duration::from_millis(75) {
            let (clean_so_far, tag_th) = extract_think_tags_streaming(&raw_content);
            let clean_preview = strip_live_inline_tool_markup(&clean_so_far);
            let combined_th = match (raw_thinking.trim().is_empty(), tag_th) {
                (true, Some(t)) => t,
                (false, Some(t)) => format!("{}\n\n{t}", raw_thinking.trim()),
                (false, None) => raw_thinking.clone(),
                (true, None) => String::new(),
            };
            store
                .update_activity_deltas(&combined_th, clean_preview)
                .await;
            last_flush = Instant::now();
        }
    }

    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf);
        let _ = process_line(
            &line,
            &mut raw_content,
            &mut raw_thinking,
            &mut tool_calls,
            &mut prompt_tokens,
            &mut completion_tokens,
        );
    }

    let (mut clean_content, tag_thinking) = extract_think_tags(&raw_content);
    let msg_thinking = if raw_thinking.trim().is_empty() {
        None
    } else {
        Some(raw_thinking.trim().to_string())
    };
    let thinking = match (msg_thinking, tag_thinking) {
        (Some(a), Some(b)) => Some(format!("{a}\n\n{b}")),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    if tool_calls.is_empty() && !req.tools.is_empty() {
        let allowed: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
        let (stripped, extracted) = extract_fallback_tool_calls_and_clean(&clean_content, &allowed);
        if !extracted.is_empty() {
            clean_content = stripped;
            tool_calls = extracted;
        }
    }

    if req.model_id.starts_with("local::")
        && prompt_tokens == 0
        && completion_tokens == 0
        && (!clean_content.is_empty() || !tool_calls.is_empty())
    {
        let th_len = thinking.as_deref().map(str::len).unwrap_or(0);
        let req_chars: usize = req.system_prompt.len()
            + req.messages.iter().map(|m| m.content.len()).sum::<usize>();
        prompt_tokens = ((req_chars / 4) as u64).max(1);
        completion_tokens = (((clean_content.len() + th_len) / 4) as u64).max(1);
    }

    Ok(CompletionResponse {
        content: clean_content,
        thinking,
        tool_calls,
        prompt_tokens,
        completion_tokens,
        thinking_tokens: 0,
        latency_ms: start.elapsed().as_millis() as u64,
        actual_model_id: req.model_id.clone(),
        switched_from: None,
        switch_reason: None,
    })
}

#[derive(Debug, Default)]
struct OpenAiStreamToolAcc {
    id: String,
    name: String,
    arguments_buf: String,
}

#[derive(Debug, Default)]
pub(crate) struct OpenAiStreamAccumulator {
    raw_content: String,
    reasoning_out: String,
    tools: BTreeMap<usize, OpenAiStreamToolAcc>,
    prompt_tokens: u64,
    completion_tokens: u64,
    thinking_tokens: u64,
}

impl OpenAiStreamAccumulator {
    pub(crate) fn apply_chunk(&mut self, val: &Value) -> bool {
        let mut updated = false;
        let delta = &val["choices"][0]["delta"];
        if let Some(c) = delta["content"].as_str() {
            if !c.is_empty() {
                self.raw_content.push_str(c);
                updated = true;
            }
        }
        if let Some(r) = delta["reasoning_content"]
            .as_str()
            .or_else(|| delta["reasoning"].as_str())
        {
            if !r.is_empty() {
                self.reasoning_out.push_str(r);
                updated = true;
            }
        }
        if let Some(arr) = delta["tool_calls"].as_array() {
            for (vec_idx, tc) in arr.iter().enumerate() {
                let idx = tc["index"]
                    .as_u64()
                    .map(|n| n as usize)
                    .unwrap_or(vec_idx);
                let entry = self.tools.entry(idx).or_default();
                if let Some(id) = tc["id"].as_str() {
                    if !id.is_empty() && entry.id.is_empty() {
                        entry.id = id.to_string();
                    }
                }
                if let Some(name) = tc["function"]["name"].as_str() {
                    entry.name.push_str(name);
                    updated = true;
                }
                if let Some(args) = tc["function"]["arguments"].as_str() {
                    entry.arguments_buf.push_str(args);
                    updated = true;
                }
            }
        }
        if let Some(pt) = val["usage"]["prompt_tokens"]
            .as_u64()
            .or_else(|| val["timings"]["prompt_n"].as_u64())
            .or_else(|| val["prompt_eval_count"].as_u64())
        {
            self.prompt_tokens = pt;
        }
        if let Some(ct) = val["usage"]["completion_tokens"]
            .as_u64()
            .or_else(|| val["timings"]["predicted_n"].as_u64())
            .or_else(|| val["eval_count"].as_u64())
        {
            self.completion_tokens = ct;
        }
        if let Some(rt) = val["usage"]["completion_tokens_details"]["reasoning_tokens"].as_u64() {
            self.thinking_tokens = rt;
        }
        updated
    }

    pub(crate) fn apply_non_streaming_json(&mut self, val: &Value) {
        let msg = &val["choices"][0]["message"];
        if let Some(c) = msg["content"].as_str() {
            self.raw_content = c.to_string();
        }
        if let Some(r) = msg["reasoning_content"]
            .as_str()
            .or_else(|| msg["reasoning"].as_str())
        {
            self.reasoning_out = r.to_string();
        }
        if let Some(arr) = msg["tool_calls"].as_array() {
            for (idx, tc) in arr.iter().enumerate() {
                if let Some(name) = tc["function"]["name"].as_str() {
                    let id = tc["id"]
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    let raw_args = &tc["function"]["arguments"];
                    let arguments_buf = if let Some(s) = raw_args.as_str() {
                        s.to_string()
                    } else if raw_args.is_null() {
                        "{}".to_string()
                    } else {
                        raw_args.to_string()
                    };
                    self.tools.insert(
                        idx,
                        OpenAiStreamToolAcc {
                            id,
                            name: name.to_string(),
                            arguments_buf,
                        },
                    );
                }
            }
        }
        if let Some(pt) = val["usage"]["prompt_tokens"]
            .as_u64()
            .or_else(|| val["timings"]["prompt_n"].as_u64())
            .or_else(|| val["prompt_eval_count"].as_u64())
        {
            self.prompt_tokens = pt;
        }
        if let Some(ct) = val["usage"]["completion_tokens"]
            .as_u64()
            .or_else(|| val["timings"]["predicted_n"].as_u64())
            .or_else(|| val["eval_count"].as_u64())
        {
            self.completion_tokens = ct;
        }
        if let Some(rt) = val["usage"]["completion_tokens_details"]["reasoning_tokens"].as_u64() {
            self.thinking_tokens = rt;
        }
    }

    pub(crate) fn live_deltas(&self) -> (String, String) {
        let (clean_content, leaked_th) = extract_think_tags_streaming(&self.raw_content);
        let thought = match (
            self.reasoning_out.trim().is_empty(),
            leaked_th.as_deref().map(str::trim),
        ) {
            (false, Some(t)) if !t.is_empty() => format!("{}\n\n{t}", self.reasoning_out.trim()),
            (false, _) => self.reasoning_out.trim().to_string(),
            (true, Some(t)) => t.to_string(),
            (true, None) => String::new(),
        };
        (thought, clean_content)
    }

    pub(crate) fn into_response(
        self,
        req: &CompletionRequest,
        latency_ms: u64,
    ) -> CompletionResponse {
        let (mut clean_content, tag_thinking) = extract_think_tags(&self.raw_content);
        let reasoning_clean = {
            let trimmed = self.reasoning_out.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        let thinking = match (reasoning_clean, tag_thinking) {
            (Some(r), Some(t)) => Some(format!("{r}\n\n{t}")),
            (Some(r), None) => Some(r),
            (None, Some(t)) => Some(t),
            (None, None) => None,
        };

        let mut tool_calls: Vec<ToolCallRequest> = self
            .tools
            .into_values()
            .filter(|t| !t.name.trim().is_empty())
            .map(|t| {
                let id = if t.id.trim().is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    t.id
                };
                let trimmed = t.arguments_buf.trim();
                let arguments = if trimmed.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(trimmed)
                        .unwrap_or_else(|_| json!({ "raw": t.arguments_buf }))
                };
                ToolCallRequest {
                    id,
                    name: t.name,
                    arguments,
                    thought_signature: None,
                }
            })
            .collect();

        if tool_calls.is_empty() && !req.tools.is_empty() {
            let allowed: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
            let (stripped, extracted) =
                extract_fallback_tool_calls_and_clean(&clean_content, &allowed);
            if !extracted.is_empty() {
                clean_content = stripped;
                tool_calls = extracted;
            }
        }

        let (prompt_tokens, completion_tokens) = if req.model_id.starts_with("local::")
            && self.prompt_tokens == 0
            && self.completion_tokens == 0
            && (!clean_content.is_empty() || !tool_calls.is_empty())
        {
            let th_len = thinking.as_deref().map(str::len).unwrap_or(0);
            let req_chars: usize = req.system_prompt.len()
                + req.messages.iter().map(|m| m.content.len()).sum::<usize>();
            (
                ((req_chars / 4) as u64).max(1),
                (((clean_content.len() + th_len) / 4) as u64).max(1),
            )
        } else {
            (self.prompt_tokens, self.completion_tokens)
        };

        CompletionResponse {
            content: clean_content,
            thinking,
            tool_calls,
            prompt_tokens,
            completion_tokens,
            thinking_tokens: self.thinking_tokens,
            latency_ms,
            actual_model_id: req.model_id.clone(),
            switched_from: None,
            switch_reason: None,
        }
    }
}

/// Shared SSE stream driver with per-chunk stall timeout, 45ms live-activity delta throttling,
/// and automatic non-SSE JSON body fallback.
async fn drive_sse_stream(
    resp: reqwest::Response,
    store: &BumboStore,
    stall_timeout: Duration,
    label: &str,
    model_id: &str,
    mut apply_event: impl FnMut(&Value, &mut String, &mut String) -> bool,
) -> anyhow::Result<Option<Value>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut raw_body: Vec<u8> = Vec::new();
    let mut saw_sse_data = false;
    let mut last_flush = Instant::now();
    let mut stream = resp.bytes_stream();
    let mut latest_thought = String::new();
    let mut latest_text = String::new();

    loop {
        let next_item = tokio::time::timeout(stall_timeout, stream.next())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "{label} timed out after {}s for model '{model_id}'",
                    stall_timeout.as_secs()
                )
            })?;
        let Some(chunk) = next_item else {
            break;
        };
        let chunk = chunk?;
        if !saw_sse_data {
            raw_body.extend_from_slice(&chunk);
        }
        buf.extend_from_slice(&chunk);

        let mut any_delta = false;
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let trimmed = line.trim();
            if let Some(data_payload) = trimmed.strip_prefix("data:") {
                if !saw_sse_data {
                    saw_sse_data = true;
                    raw_body.clear();
                }
                let json_str = data_payload.trim();
                if json_str.is_empty() || json_str == "[DONE]" {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                    if apply_event(&val, &mut latest_thought, &mut latest_text) {
                        any_delta = true;
                    }
                }
            }
        }

        if any_delta && last_flush.elapsed() >= Duration::from_millis(45) {
            let clean_preview = strip_live_inline_tool_markup(&latest_text);
            store
                .update_activity_deltas(&latest_thought, clean_preview)
                .await;
            last_flush = Instant::now();
        }
    }

    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf);
        if let Some(data_payload) = line.trim().strip_prefix("data:") {
            let json_str = data_payload.trim();
            if !json_str.is_empty() && json_str != "[DONE]" {
                saw_sse_data = true;
                raw_body.clear();
                if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                    let _ = apply_event(&val, &mut latest_thought, &mut latest_text);
                }
            }
        }
    }

    if !saw_sse_data && !raw_body.is_empty() {
        if let Ok(val) = serde_json::from_slice::<Value>(&raw_body) {
            return Ok(Some(val));
        }
    }

    if saw_sse_data {
        let clean_preview = strip_live_inline_tool_markup(&latest_text);
        store
            .update_activity_deltas(&latest_thought, clean_preview)
            .await;
    }

    Ok(None)
}

async fn complete_openai_compat(
    gate: &NetworkGate,
    store: &BumboStore,
    url: &str,
    bearer_token: Option<&str>,
    model_name: &str,
    req: &CompletionRequest,
    purpose: &str,
) -> anyhow::Result<CompletionResponse> {
    let (resp, _) = complete_openai_compat_with_headers(
        gate,
        store,
        url,
        bearer_token,
        model_name,
        req,
        purpose,
    )
    .await?;
    Ok(resp)
}

async fn complete_openai_compat_with_headers(
    gate: &NetworkGate,
    store: &BumboStore,
    url: &str,
    bearer_token: Option<&str>,
    model_name: &str,
    req: &CompletionRequest,
    purpose: &str,
) -> anyhow::Result<(CompletionResponse, RateHeaders)> {
    let start = Instant::now();

    let mut msgs = Vec::new();
    if !req.system_prompt.trim().is_empty() {
        msgs.push(json!({ "role": "system", "content": req.system_prompt }));
    }
    for m in &req.messages {
        if m.role == "assistant" && !m.tool_calls.is_empty() {
            let tcs: Vec<Value> = m
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments.to_string(),
                        }
                    })
                })
                .collect();
            msgs.push(json!({
                "role": "assistant",
                "content": m.content,
                "tool_calls": tcs,
            }));
        } else if m.role == "tool" {
            msgs.push(json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id.clone().unwrap_or_else(|| "call_0".into()),
                "content": m.content,
            }));
        } else if m.images.is_empty() {
            msgs.push(json!({
                "role": m.role,
                "content": m.content,
            }));
        } else {
            let mut parts = Vec::new();
            if !m.content.trim().is_empty() {
                parts.push(json!({ "type": "text", "text": m.content }));
            }
            for (mime, b64) in &m.images {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{mime};base64,{b64}") },
                }));
            }
            msgs.push(json!({
                "role": m.role,
                "content": parts,
            }));
        }
    }

    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters_schema,
                }
            })
        })
        .collect();

    let mut body = json!({
        "model": model_name,
        "messages": msgs,
        "temperature": req.temperature,
        "max_tokens": req.max_tokens,
        "stream": true,
    });
    if purpose == "openrouter_api" || purpose == "local_inference" {
        body["stream_options"] = json!({ "include_usage": true });
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }

    let mut http_req = gate.request(Method::POST, url, purpose)?.json(&body);
    if purpose == "local_inference" {
        http_req = http_req.header(CONNECTION, "close");
    }
    if let Some(tok) = bearer_token {
        http_req = http_req.bearer_auth(tok);
    }

    let stall_timeout = if purpose == "local_inference" {
        Duration::from_secs(120)
    } else {
        Duration::from_secs(45)
    };
    let resp = tokio::time::timeout(stall_timeout, http_req.send())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Endpoint {url} request timed out after {}s for model '{}'",
                stall_timeout.as_secs(),
                req.model_id
            )
        })??;
    let rate_headers = extract_rate_headers(resp.headers());
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Endpoint {url} returned HTTP {status}: {text}");
    }

    let mut acc = OpenAiStreamAccumulator::default();
    if let Some(non_sse_val) = drive_sse_stream(
        resp,
        store,
        stall_timeout,
        &format!("Endpoint {url} stream"),
        &req.model_id,
        |val, out_th, out_txt| {
            let changed = acc.apply_chunk(val);
            if changed {
                let (th, txt) = acc.live_deltas();
                *out_th = th;
                *out_txt = txt;
            }
            changed
        },
    )
    .await?
    {
        acc.apply_non_streaming_json(&non_sse_val);
        let (th, txt) = acc.live_deltas();
        store.update_activity_deltas(&th, &txt).await;
    }

    Ok((
        acc.into_response(req, start.elapsed().as_millis() as u64),
        rate_headers,
    ))
}

fn apply_gemini_chunk_value(
    val: &Value,
    thought_out: &mut String,
    text_out: &mut String,
    tool_calls: &mut Vec<ToolCallRequest>,
    prompt_tokens: &mut u64,
    completion_tokens: &mut u64,
    thinking_tokens: &mut u64,
) -> bool {
    let mut updated = false;
    if let Some(parts) = val["candidates"][0]["content"]["parts"].as_array() {
        for p in parts {
            let is_thought = p["thought"].as_bool().unwrap_or(false);
            if let Some(txt) = p["text"].as_str() {
                if !txt.is_empty() {
                    if is_thought {
                        thought_out.push_str(txt);
                    } else {
                        text_out.push_str(txt);
                    }
                    updated = true;
                }
            }
            if let Some(fc) = p.get("functionCall") {
                if let Some(name) = fc["name"].as_str() {
                    let thought_signature = p
                        .get("thoughtSignature")
                        .or_else(|| p.get("thought_signature"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let id = fc["id"]
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    let args = if fc["args"].is_object() {
                        fc["args"].clone()
                    } else {
                        json!({})
                    };
                    tool_calls.push(ToolCallRequest {
                        id,
                        name: name.to_string(),
                        arguments: args,
                        thought_signature,
                    });
                    updated = true;
                }
            }
        }
    }

    let usage = &val["usageMetadata"];
    if let Some(v) = usage["promptTokenCount"].as_u64() {
        *prompt_tokens = v;
    }
    if let Some(v) = usage["candidatesTokenCount"].as_u64() {
        *completion_tokens = v;
    }
    if let Some(v) = usage["thoughtsTokenCount"].as_u64() {
        *thinking_tokens = v;
    }
    updated
}

fn build_gemini_contents(messages: &[ProviderMessage]) -> Vec<Value> {
    let mut contents: Vec<Value> = Vec::new();
    for m in messages {
        if m.role == "assistant" {
            let mut parts = Vec::new();
            if !m.content.trim().is_empty() {
                parts.push(json!({ "text": m.content }));
            }
            let turn_has_real_sig = m
                .tool_calls
                .iter()
                .any(|tc| tc.thought_signature.as_deref().is_some_and(|s| !s.trim().is_empty()));
            for (idx, tc) in m.tool_calls.iter().enumerate() {
                let args = if tc.arguments.is_object() {
                    tc.arguments.clone()
                } else {
                    json!({})
                };
                let mut part = json!({
                    "functionCall": {
                        "name": tc.name,
                        "args": args,
                    }
                });
                if let Some(ref sig) = tc.thought_signature {
                    if !sig.trim().is_empty() {
                        part["thoughtSignature"] = Value::String(sig.clone());
                    }
                } else if !turn_has_real_sig && idx == 0 {
                    part["thoughtSignature"] =
                        Value::String("skip_thought_signature_validator".into());
                }
                parts.push(part);
            }
            if !parts.is_empty() {
                contents.push(json!({ "role": "model", "parts": parts }));
            }
        } else if m.role == "tool" {
            let fn_name = m.tool_name.clone().unwrap_or_else(|| "tool".into());
            let part = json!({
                "functionResponse": {
                    "name": fn_name,
                    "response": { "result": m.content }
                }
            });
            let merged_into_prev = if let Some(last) = contents.last_mut() {
                let is_user_tool_turn = last["role"] == "user"
                    && last["parts"]
                        .as_array()
                        .map(|arr| arr.iter().any(|p| p.get("functionResponse").is_some()))
                        .unwrap_or(false);
                if is_user_tool_turn {
                    if let Some(arr) = last["parts"].as_array_mut() {
                        arr.push(part.clone());
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !merged_into_prev {
                contents.push(json!({
                    "role": "user",
                    "parts": [part]
                }));
            }
        } else {
            let mut parts = Vec::new();
            if !m.content.trim().is_empty() || m.images.is_empty() {
                parts.push(json!({ "text": m.content }));
            }
            for (mime, b64) in &m.images {
                parts.push(json!({
                    "inlineData": {
                        "mimeType": mime,
                        "data": b64,
                    }
                }));
            }
            contents.push(json!({
                "role": "user",
                "parts": parts,
            }));
        }
    }
    contents
}

async fn complete_gemini(
    gate: &NetworkGate,
    store: &BumboStore,
    api_key: &str,
    req: &CompletionRequest,
    supports_thinking: bool,
    supports_tools: bool,
) -> anyhow::Result<(CompletionResponse, RateHeaders)> {
    let start = Instant::now();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
        req.model_id
    );

    let contents = build_gemini_contents(&req.messages);

    let mut generation_config = json!({
        "temperature": req.temperature,
        "maxOutputTokens": req.max_tokens,
    });
    if supports_thinking && req.enable_thinking {
        generation_config["thinkingConfig"] = json!({ "includeThoughts": true });
    }

    let mut body = json!({
        "contents": contents,
        "generationConfig": generation_config,
    });

    if !req.system_prompt.trim().is_empty() {
        body["systemInstruction"] = json!({
            "parts": [{ "text": req.system_prompt }]
        });
    }

    if supports_tools && !req.tools.is_empty() {
        let decls: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let params = clean_schema_for_gemini(&t.parameters_schema);
                let has_props = params
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .map(|m| !m.is_empty())
                    .unwrap_or(false);
                let mut decl = json!({
                    "name": t.name,
                    "description": t.description,
                });
                if has_props {
                    decl["parameters"] = params;
                }
                decl
            })
            .collect();
        body["tools"] = json!([{ "functionDeclarations": decls }]);
    }

    let http_req = gate
        .request(Method::POST, &url, "gemini_api")?
        .header("x-goog-api-key", api_key.trim())
        .json(&body);

    let resp = tokio::time::timeout(Duration::from_secs(45), http_req.send())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Gemini API request timed out after 45s for model '{}'",
                req.model_id
            )
        })??;

    let rate_headers = extract_rate_headers(resp.headers());
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Gemini API HTTP {status}: {text}");
    }

    let mut text_out = String::new();
    let mut thought_out = String::new();
    let mut tool_calls = Vec::new();
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;
    let mut thinking_tokens = 0u64;
    let mut buf: Vec<u8> = Vec::new();
    let mut raw_body: Vec<u8> = Vec::new();
    let mut saw_sse_data = false;
    let mut last_flush = Instant::now();
    let mut stream = resp.bytes_stream();

    loop {
        let next_item = tokio::time::timeout(Duration::from_secs(45), stream.next())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Gemini API stream stalled (timed out after 45s) for model '{}'",
                    req.model_id
                )
            })?;
        let Some(chunk) = next_item else {
            break;
        };
        let chunk = chunk?;
        if !saw_sse_data {
            raw_body.extend_from_slice(&chunk);
        }
        buf.extend_from_slice(&chunk);

        let mut any_delta = false;
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let trimmed = line.trim();
            if let Some(data_payload) = trimmed.strip_prefix("data:") {
                if !saw_sse_data {
                    saw_sse_data = true;
                    raw_body.clear();
                }
                let json_str = data_payload.trim();
                if json_str.is_empty() || json_str == "[DONE]" {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                    if apply_gemini_chunk_value(
                        &val,
                        &mut thought_out,
                        &mut text_out,
                        &mut tool_calls,
                        &mut prompt_tokens,
                        &mut completion_tokens,
                        &mut thinking_tokens,
                    ) {
                        any_delta = true;
                    }
                }
            }
        }

        if any_delta && last_flush.elapsed() >= Duration::from_millis(45) {
            store.update_activity_deltas(&thought_out, &text_out).await;
            last_flush = Instant::now();
        }
    }

    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf);
        let trimmed = line.trim();
        if let Some(data_payload) = trimmed.strip_prefix("data:") {
            saw_sse_data = true;
            raw_body.clear();
            let json_str = data_payload.trim();
            if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                let _ = apply_gemini_chunk_value(
                    &val,
                    &mut thought_out,
                    &mut text_out,
                    &mut tool_calls,
                    &mut prompt_tokens,
                    &mut completion_tokens,
                    &mut thinking_tokens,
                );
            }
        }
    }

    if !saw_sse_data && !raw_body.is_empty() {
        if let Ok(val) = serde_json::from_slice::<Value>(&raw_body) {
            match val {
                Value::Array(items) => {
                    for item in &items {
                        let _ = apply_gemini_chunk_value(
                            item,
                            &mut thought_out,
                            &mut text_out,
                            &mut tool_calls,
                            &mut prompt_tokens,
                            &mut completion_tokens,
                            &mut thinking_tokens,
                        );
                    }
                }
                other => {
                    let _ = apply_gemini_chunk_value(
                        &other,
                        &mut thought_out,
                        &mut text_out,
                        &mut tool_calls,
                        &mut prompt_tokens,
                        &mut completion_tokens,
                        &mut thinking_tokens,
                    );
                }
            }
        }
    }

    store.update_activity_deltas(&thought_out, &text_out).await;

    Ok((
        CompletionResponse {
            content: text_out,
            thinking: if thought_out.trim().is_empty() {
                None
            } else {
                Some(thought_out)
            },
            tool_calls,
            prompt_tokens,
            completion_tokens,
            thinking_tokens,
            latency_ms: start.elapsed().as_millis() as u64,
            actual_model_id: req.model_id.clone(),
            switched_from: None,
            switch_reason: None,
        },
        rate_headers,
    ))
}

fn build_anthropic_messages(messages_in: &[ProviderMessage]) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for m in messages_in {
        if m.role == "assistant" {
            let mut blocks = Vec::new();
            if !m.content.trim().is_empty() {
                blocks.push(json!({ "type": "text", "text": m.content }));
            }
            for tc in &m.tool_calls {
                let input = if tc.arguments.is_object() {
                    tc.arguments.clone()
                } else {
                    json!({})
                };
                blocks.push(json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.name,
                    "input": input,
                }));
            }
            if blocks.is_empty() {
                messages.push(json!({ "role": "assistant", "content": "" }));
            } else {
                messages.push(json!({ "role": "assistant", "content": blocks }));
            }
        } else if m.role == "tool" {
            let block = json!({
                "type": "tool_result",
                "tool_use_id": m.tool_call_id.clone().unwrap_or_else(|| "tool_0".into()),
                "content": m.content,
            });
            let merged_into_prev = if let Some(last) = messages.last_mut() {
                let is_user_tool_turn = last["role"] == "user"
                    && last["content"]
                        .as_array()
                        .map(|arr| arr.iter().any(|b| b["type"] == "tool_result"))
                        .unwrap_or(false);
                if is_user_tool_turn {
                    if let Some(arr) = last["content"].as_array_mut() {
                        arr.push(block.clone());
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !merged_into_prev {
                messages.push(json!({
                    "role": "user",
                    "content": [block]
                }));
            }
        } else if m.images.is_empty() {
            messages.push(json!({ "role": "user", "content": m.content }));
        } else {
            let mut blocks = Vec::new();
            for (mime, b64) in &m.images {
                blocks.push(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": mime,
                        "data": b64,
                    }
                }));
            }
            blocks.push(json!({ "type": "text", "text": m.content }));
            messages.push(json!({ "role": "user", "content": blocks }));
        }
    }
    messages
}

#[derive(Debug, Default)]
struct AnthropicStreamToolAcc {
    id: String,
    name: String,
    input_json_buf: String,
    initial_input: Value,
}

#[derive(Debug, Default)]
pub(crate) struct AnthropicStreamAccumulator {
    text_out: String,
    thought_out: String,
    tools: BTreeMap<usize, AnthropicStreamToolAcc>,
    prompt_tokens: u64,
    completion_tokens: u64,
}

impl AnthropicStreamAccumulator {
    pub(crate) fn apply_chunk(&mut self, val: &Value) -> bool {
        let mut updated = false;
        match val["type"].as_str().unwrap_or("") {
            "message_start" => {
                if let Some(in_tok) = val["message"]["usage"]["input_tokens"].as_u64() {
                    self.prompt_tokens = in_tok;
                }
            }
            "content_block_start" => {
                let idx = val["index"].as_u64().unwrap_or(0) as usize;
                let cb = &val["content_block"];
                match cb["type"].as_str().unwrap_or("") {
                    "text" => {
                        if !self.text_out.is_empty() && !self.text_out.ends_with('\n') {
                            self.text_out.push('\n');
                            updated = true;
                        }
                        if let Some(t) = cb["text"].as_str() {
                            if !t.is_empty() {
                                self.text_out.push_str(t);
                                updated = true;
                            }
                        }
                    }
                    "thinking" => {
                        if let Some(t) = cb["thinking"].as_str() {
                            if !t.is_empty() {
                                self.thought_out.push_str(t);
                                updated = true;
                            }
                        }
                    }
                    "tool_use" => {
                        let name = cb["name"].as_str().unwrap_or("").to_string();
                        let id = cb["id"]
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| Uuid::new_v4().to_string());
                        let initial_input = if cb["input"].is_object() {
                            cb["input"].clone()
                        } else {
                            json!({})
                        };
                        self.tools.insert(
                            idx,
                            AnthropicStreamToolAcc {
                                id,
                                name,
                                input_json_buf: String::new(),
                                initial_input,
                            },
                        );
                        updated = true;
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let idx = val["index"].as_u64().unwrap_or(0) as usize;
                let delta = &val["delta"];
                match delta["type"].as_str().unwrap_or("") {
                    "thinking_delta" => {
                        if let Some(t) = delta["thinking"].as_str() {
                            if !t.is_empty() {
                                self.thought_out.push_str(t);
                                updated = true;
                            }
                        }
                    }
                    "text_delta" => {
                        if let Some(t) = delta["text"].as_str() {
                            if !t.is_empty() {
                                self.text_out.push_str(t);
                                updated = true;
                            }
                        }
                    }
                    "input_json_delta" => {
                        if let Some(pj) = delta["partial_json"].as_str() {
                            self.tools
                                .entry(idx)
                                .or_default()
                                .input_json_buf
                                .push_str(pj);
                            updated = true;
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(out_tok) = val["usage"]["output_tokens"].as_u64() {
                    self.completion_tokens = out_tok;
                }
            }
            _ => {}
        }
        updated
    }

    pub(crate) fn apply_non_streaming_json(&mut self, val: &Value) {
        if let Some(blocks) = val["content"].as_array() {
            for (idx, b) in blocks.iter().enumerate() {
                match b["type"].as_str().unwrap_or("") {
                    "thinking" => {
                        if let Some(t) = b["thinking"].as_str() {
                            self.thought_out.push_str(t);
                        }
                    }
                    "text" => {
                        if let Some(t) = b["text"].as_str() {
                            if !self.text_out.is_empty() {
                                self.text_out.push('\n');
                            }
                            self.text_out.push_str(t);
                        }
                    }
                    "tool_use" => {
                        if let Some(name) = b["name"].as_str() {
                            self.tools.insert(
                                idx,
                                AnthropicStreamToolAcc {
                                    id: b["id"]
                                        .as_str()
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| Uuid::new_v4().to_string()),
                                    name: name.to_string(),
                                    input_json_buf: String::new(),
                                    initial_input: b["input"].clone(),
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(pt) = val["usage"]["input_tokens"].as_u64() {
            self.prompt_tokens = pt;
        }
        if let Some(ct) = val["usage"]["output_tokens"].as_u64() {
            self.completion_tokens = ct;
        }
    }

    pub(crate) fn into_response(self, model_id: &str, latency_ms: u64) -> CompletionResponse {
        let tool_calls = self
            .tools
            .into_values()
            .filter(|t| !t.name.trim().is_empty())
            .map(|t| {
                let id = if t.id.trim().is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    t.id
                };
                let trimmed = t.input_json_buf.trim();
                let arguments = if trimmed.is_empty() {
                    t.initial_input
                } else {
                    serde_json::from_str(trimmed)
                        .unwrap_or_else(|_| json!({ "raw": t.input_json_buf }))
                };
                ToolCallRequest {
                    id,
                    name: t.name,
                    arguments,
                    thought_signature: None,
                }
            })
            .collect();

        CompletionResponse {
            content: self.text_out,
            thinking: if self.thought_out.trim().is_empty() {
                None
            } else {
                Some(self.thought_out)
            },
            tool_calls,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            thinking_tokens: 0,
            latency_ms,
            actual_model_id: model_id.to_string(),
            switched_from: None,
            switch_reason: None,
        }
    }
}

async fn complete_anthropic(
    gate: &NetworkGate,
    store: &BumboStore,
    api_key: &str,
    req: &CompletionRequest,
) -> anyhow::Result<(CompletionResponse, RateHeaders)> {
    let start = Instant::now();
    let url = "https://api.anthropic.com/v1/messages";

    let messages = build_anthropic_messages(&req.messages);

    let mut body = json!({
        "model": req.model_id,
        "max_tokens": req.max_tokens.max(1024),
        "messages": messages,
        "stream": true,
    });
    if !req.system_prompt.trim().is_empty() {
        body["system"] = Value::String(req.system_prompt.clone());
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters_schema,
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }

    let http_req = gate
        .request(Method::POST, url, "anthropic_api")?
        .header("x-api-key", api_key.trim())
        .header("anthropic-version", "2023-06-01")
        .json(&body);

    let resp = tokio::time::timeout(Duration::from_secs(45), http_req.send())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Anthropic API request timed out after 45s for model '{}'",
                req.model_id
            )
        })??;

    let rate_headers = extract_rate_headers(resp.headers());
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API HTTP {status}: {text}");
    }

    let mut acc = AnthropicStreamAccumulator::default();
    if let Some(non_sse_val) = drive_sse_stream(
        resp,
        store,
        Duration::from_secs(45),
        "Anthropic API stream",
        &req.model_id,
        |val, out_th, out_txt| {
            let changed = acc.apply_chunk(val);
            if changed {
                *out_th = acc.thought_out.clone();
                *out_txt = acc.text_out.clone();
            }
            changed
        },
    )
    .await?
    {
        acc.apply_non_streaming_json(&non_sse_val);
        store
            .update_activity_deltas(&acc.thought_out, &acc.text_out)
            .await;
    }

    Ok((
        acc.into_response(&req.model_id, start.elapsed().as_millis() as u64),
        rate_headers,
    ))
}

#[derive(Debug, Deserialize)]
struct GeminiModelsListResp {
    models: Option<Vec<GeminiModelRaw>>,
}

#[derive(Debug, Deserialize)]
struct GeminiModelRaw {
    name: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    description: Option<String>,
    #[serde(rename = "inputTokenLimit")]
    input_token_limit: Option<u64>,
    #[serde(rename = "supportedGenerationMethods")]
    supported_generation_methods: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
enum GeminiProbeOutcome {
    Available,
    PaymentRequired(String),
    Unreachable(String),
}

async fn probe_gemini_model_status(
    gate: &NetworkGate,
    api_key: &str,
    model_id: &str,
) -> GeminiProbeOutcome {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        model_id
    );
    let Ok(builder) = gate.request(Method::POST, &url, "gemini_model_live_probe") else {
        return GeminiProbeOutcome::Unreachable("Blocked by Strict Local privacy mode".into());
    };
    let req = builder
        .header("x-goog-api-key", api_key.trim())
        .json(&json!({
            "contents": [{ "role": "user", "parts": [{ "text": "ok" }] }],
            "generationConfig": { "maxOutputTokens": 1 }
        }));

    let Ok(Ok(resp)) = tokio::time::timeout(Duration::from_secs(4), req.send()).await else {
        return GeminiProbeOutcome::Unreachable("Probe timed out / unreachable".into());
    };
    let status = resp.status();
    if status.is_success() {
        return GeminiProbeOutcome::Available;
    }
    let body = resp.text().await.unwrap_or_default();
    let lower = body.to_ascii_lowercase();
    if status.as_u16() == 402
        || lower.contains("billing")
        || lower.contains("paid tier")
        || lower.contains("enable billing")
        || (lower.contains("limit: 0") && lower.contains("free_tier"))
    {
        GeminiProbeOutcome::PaymentRequired(format!(
            "HTTP {} · Requires paid billing tier",
            status.as_u16()
        ))
    } else if lower.contains("limit: 0") {
        GeminiProbeOutcome::PaymentRequired("Quota limit is 0 on free tier (paid plan required)".into())
    } else if status.as_u16() == 404 {
        GeminiProbeOutcome::Unreachable("HTTP 404 · Not available on this endpoint".into())
    } else {
        GeminiProbeOutcome::Unreachable(format!("HTTP {} · Unavailable", status.as_u16()))
    }
}

pub async fn probe_gemini_model_live(gate: &NetworkGate, api_key: &str, model_id: &str) -> bool {
    matches!(
        probe_gemini_model_status(gate, api_key, model_id).await,
        GeminiProbeOutcome::Available
    )
}

pub async fn sync_google_models(
    gate: &NetworkGate,
    api_key: &str,
) -> anyhow::Result<Vec<ModelCatalogEntry>> {
    if api_key.trim().is_empty() {
        anyhow::bail!("Enter a Gemini API key first.");
    }
    let url = "https://generativelanguage.googleapis.com/v1beta/models?pageSize=100";
    let resp = gate
        .request(Method::GET, url, "gemini_model_catalog_sync")?
        .header("x-goog-api-key", api_key.trim())
        .send()
        .await?;
    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Google models sync HTTP error: {}",
            format_clean_provider_error(&text)
        );
    }

    let parsed: GeminiModelsListResp = resp.json().await?;
    let mut candidates = Vec::new();
    for m in parsed.models.unwrap_or_default() {
        let methods = m.supported_generation_methods.unwrap_or_default();
        if !methods.iter().any(|x| x == "generateContent") {
            continue;
        }
        let id = m.name.strip_prefix("models/").unwrap_or(&m.name).to_string();
        if !is_viable_gemini_chat_model(&id) {
            continue;
        }
        let lower = id.to_ascii_lowercase();
        let supports_thinking = lower.contains("thinking")
            || lower.contains("gemini-3-flash")
            || lower.contains("pro");
        let supports_tools = !lower.starts_with("gemma");

        let display_name = m.display_name.unwrap_or_else(|| id.clone());
        candidates.push(ModelCatalogEntry {
            id: id.clone(),
            display_name: display_name.clone(),
            provider: ProviderKind::GoogleGemini,
            context_window: m.input_token_limit.unwrap_or(1_000_000),
            supports_thinking,
            supports_tools,
            supports_vision: true,
            tier_note: m
                .description
                .unwrap_or_else(|| "Verified via Google Gemini API".into()),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: 500,
                tpm_limit: 2_000_000,
                rpd_limit: 5_000,
                daily_token_limit: 15_000_000,
            }),
            reachable: false,
            reachability_reason: "Pending verification".into(),
            antigravity_alias: Some(display_name),
            requires_payment: false,
        });
    }

    let mut probed = Vec::new();
    for chunk in candidates.chunks(6) {
        let checks = futures_util::future::join_all(
            chunk
                .iter()
                .map(|cand| probe_gemini_model_status(gate, api_key, &cand.id)),
        )
        .await;
        for (cand, outcome) in chunk.iter().zip(checks.into_iter()) {
            let mut entry = cand.clone();
            match outcome {
                GeminiProbeOutcome::Available => {
                    entry.reachable = true;
                    entry.requires_payment = false;
                    entry.reachability_reason = "Verified reachable via live probe".into();
                }
                GeminiProbeOutcome::PaymentRequired(reason) => {
                    entry.reachable = false;
                    entry.requires_payment = true;
                    entry.reachability_reason = reason;
                }
                GeminiProbeOutcome::Unreachable(reason) => {
                    entry.reachable = false;
                    entry.requires_payment = false;
                    entry.reachability_reason = reason;
                }
            }
            probed.push(entry);
        }
    }

    sort_gemini_models_by_priority(&mut probed);
    probed.sort_by(|a, b| b.reachable.cmp(&a.reachable));
    Ok(probed)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderVerifyStatus {
    pub provider: String,
    pub configured: bool,
    pub verified: bool,
    pub model_count: usize,
    pub message: String,
}

pub async fn verify_and_discover_provider_models(
    gate: &NetworkGate,
    store: &BumboStore,
) -> anyhow::Result<Vec<ProviderVerifyStatus>> {
    let snap = store.snapshot().await;
    let keys = snap.settings.api_keys;
    let mut statuses = Vec::new();
    let mut verified_map = std::collections::HashMap::new();

    // 1. Google Gemini
    if keys.gemini_api_key.trim().is_empty() {
        statuses.push(ProviderVerifyStatus {
            provider: "google_gemini".into(),
            configured: false,
            verified: false,
            model_count: 0,
            message: "No API key set".into(),
        });
    } else {
        match sync_google_models(gate, &keys.gemini_api_key).await {
            Ok(list) => {
                let reachable_count = list
                    .iter()
                    .filter(|m| m.reachable && !m.requires_payment)
                    .count();
                let paid_count = list.iter().filter(|m| m.requires_payment).count();
                let message = if paid_count > 0 {
                    format!("Verified · {reachable_count} reachable · {paid_count} paid")
                } else {
                    format!("Verified · {reachable_count} models reachable")
                };
                verified_map.insert("google_gemini".to_string(), list);
                statuses.push(ProviderVerifyStatus {
                    provider: "google_gemini".into(),
                    configured: true,
                    verified: true,
                    model_count: reachable_count,
                    message,
                });
            }
            Err(e) => {
                statuses.push(ProviderVerifyStatus {
                    provider: "google_gemini".into(),
                    configured: true,
                    verified: false,
                    model_count: 0,
                    message: format!("Check failed: {e}"),
                });
            }
        }
    }

    // 2. Anthropic
    if keys.anthropic_api_key.trim().is_empty() {
        statuses.push(ProviderVerifyStatus {
            provider: "anthropic".into(),
            configured: false,
            verified: false,
            model_count: 0,
            message: "No API key set".into(),
        });
    } else {
        match sync_anthropic_models(gate, &keys.anthropic_api_key).await {
            Ok(list) => {
                let reachable_count = list
                    .iter()
                    .filter(|m| m.reachable && !m.requires_payment)
                    .count();
                verified_map.insert("anthropic".to_string(), list);
                statuses.push(ProviderVerifyStatus {
                    provider: "anthropic".into(),
                    configured: true,
                    verified: true,
                    model_count: reachable_count,
                    message: format!("Verified · {reachable_count} models reachable"),
                });
            }
            Err(e) => {
                statuses.push(ProviderVerifyStatus {
                    provider: "anthropic".into(),
                    configured: true,
                    verified: false,
                    model_count: 0,
                    message: format!("Check failed: {e}"),
                });
            }
        }
    }

    // 3. OpenRouter
    if keys.openrouter_api_key.trim().is_empty() {
        statuses.push(ProviderVerifyStatus {
            provider: "open_router".into(),
            configured: false,
            verified: false,
            model_count: 0,
            message: "No API key set".into(),
        });
    } else {
        match sync_openrouter_models(gate, &keys.openrouter_api_key).await {
            Ok(list) => {
                let reachable_count = list
                    .iter()
                    .filter(|m| m.reachable && !m.requires_payment)
                    .count();
                let paid_count = list.iter().filter(|m| m.requires_payment).count();
                let message = if paid_count > 0 {
                    format!("Verified · {reachable_count} reachable · {paid_count} paid")
                } else {
                    format!("Verified · {reachable_count} models reachable")
                };
                verified_map.insert("open_router".to_string(), list);
                statuses.push(ProviderVerifyStatus {
                    provider: "open_router".into(),
                    configured: true,
                    verified: true,
                    model_count: reachable_count,
                    message,
                });
            }
            Err(e) => {
                statuses.push(ProviderVerifyStatus {
                    provider: "open_router".into(),
                    configured: true,
                    verified: false,
                    model_count: 0,
                    message: format!("Check failed: {e}"),
                });
            }
        }
    }

    // 4. Custom OpenAI-Compatible
    if keys.openai_compat_base_url.trim().is_empty() {
        statuses.push(ProviderVerifyStatus {
            provider: "open_ai_compat".into(),
            configured: false,
            verified: false,
            model_count: 0,
            message: "No Base URL set".into(),
        });
    } else {
        match sync_openai_compat_models(
            gate,
            &keys.openai_compat_base_url,
            &keys.openai_compat_api_key,
        )
        .await
        {
            Ok(list) => {
                let reachable_count = list
                    .iter()
                    .filter(|m| m.reachable && !m.requires_payment)
                    .count();
                verified_map.insert("open_ai_compat".to_string(), list);
                statuses.push(ProviderVerifyStatus {
                    provider: "open_ai_compat".into(),
                    configured: true,
                    verified: true,
                    model_count: reachable_count,
                    message: format!("Verified · {reachable_count} models reachable"),
                });
            }
            Err(e) => {
                statuses.push(ProviderVerifyStatus {
                    provider: "open_ai_compat".into(),
                    configured: true,
                    verified: false,
                    model_count: 0,
                    message: format!("Check failed: {e}"),
                });
            }
        }
    }

    store
        .update(|s| {
            for (k, v) in verified_map {
                for m in &v {
                    if m.reachable && !m.requires_payment {
                        s.blocked_models.remove(&m.id);
                    }
                }
                if k == "google_gemini" {
                    s.blocked_models
                        .retain(|id, _| !id.starts_with("gemini-") && !id.starts_with("gemma-"));
                    s.synced_google_models = v.clone();
                }
                s.verified_provider_models.insert(k, v);
            }
            if !s.settings.active_model_id.starts_with("local::") {
                let (cat, _) = harness_store::compute_catalog_and_quotas(s);
                let still_ok = cat
                    .iter()
                    .any(|c| c.id == s.settings.active_model_id && c.reachable && !c.requires_payment);
                if !still_ok {
                    if let Some(first_reachable) = cat
                        .iter()
                        .find(|c| c.reachable && !c.requires_payment)
                    {
                        s.settings.active_model_id = first_reachable.id.clone();
                    }
                }
            }
        })
        .await?;

    Ok(statuses)
}

async fn sync_anthropic_models(
    gate: &NetworkGate,
    api_key: &str,
) -> anyhow::Result<Vec<ModelCatalogEntry>> {
    let resp = gate
        .request(
            Method::GET,
            "https://api.anthropic.com/v1/models",
            "anthropic_model_verify",
        )?
        .header("x-api-key", api_key.trim())
        .header("anthropic-version", "2023-06-01")
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic HTTP {status}: {text}");
    }
    let val: Value = resp.json().await?;
    let mut out = Vec::new();
    for item in val["data"].as_array().cloned().unwrap_or_default() {
        if let Some(id) = item["id"].as_str() {
            let display = item["display_name"]
                .as_str()
                .unwrap_or(id)
                .to_string();
            out.push(ModelCatalogEntry {
                id: id.to_string(),
                display_name: display.clone(),
                provider: ProviderKind::Anthropic,
                context_window: 200_000,
                supports_thinking: true,
                supports_tools: true,
                supports_vision: true,
                tier_note: "Verified via Anthropic API".into(),
                quota_limit: Some(ModelQuotaLimit {
                    rpm_limit: 50,
                    tpm_limit: 400_000,
                    rpd_limit: 2_000,
                    daily_token_limit: 10_000_000,
                }),
                reachable: true,
                reachability_reason: "Verified reachable via live API".into(),
                antigravity_alias: Some(display),
                requires_payment: false,
            });
        }
    }
    Ok(out)
}

fn parse_openrouter_price(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite()),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|x| x.is_finite()),
        _ => None,
    }
}

pub fn evaluate_openrouter_account_has_credits(
    auth_json: &Value,
    credits_json: Option<&Value>,
) -> bool {
    let data = &auth_json["data"];
    let mut has_paid_balance = if let Some(cj) = credits_json {
        let total = cj["data"]["total_credits"].as_f64();
        let used = cj["data"]["total_usage"].as_f64().unwrap_or(0.0);
        match total {
            Some(t) if t.is_finite() && used.is_finite() => (t - used) > 0.0001,
            _ => !data["is_free_tier"].as_bool().unwrap_or(true),
        }
    } else {
        !data["is_free_tier"].as_bool().unwrap_or(true)
    };

    if let Some(rem) = data["limit_remaining"].as_f64() {
        if rem.is_finite() && rem <= 0.0001 {
            has_paid_balance = false;
        }
    }
    has_paid_balance
}

pub fn is_openrouter_model_free(id: &str, item: &Value) -> bool {
    if id.ends_with(":free") {
        return true;
    }
    let prompt_price = parse_openrouter_price(item["pricing"].get("prompt"));
    let comp_price = parse_openrouter_price(item["pricing"].get("completion"));
    matches!((prompt_price, comp_price), (Some(p), Some(c)) if p <= 0.0 && c <= 0.0)
}

async fn sync_openrouter_models(
    gate: &NetworkGate,
    api_key: &str,
) -> anyhow::Result<Vec<ModelCatalogEntry>> {
    let auth_resp = gate
        .request(
            Method::GET,
            "https://openrouter.ai/api/v1/auth/key",
            "openrouter_key_verify",
        )?
        .bearer_auth(api_key.trim())
        .send()
        .await?;
    if !auth_resp.status().is_success() {
        let code = auth_resp.status();
        if code.as_u16() == 402 {
            anyhow::bail!("OpenRouter HTTP 402: Payment required");
        }
        anyhow::bail!("OpenRouter key rejected (HTTP {code})");
    }
    let auth_json: Value = auth_resp.json().await.unwrap_or(Value::Null);

    let credits_json: Option<Value> = match gate.request(
        Method::GET,
        "https://openrouter.ai/api/v1/credits",
        "openrouter_credits_check",
    ) {
        Ok(builder) => match tokio::time::timeout(
            Duration::from_secs(3),
            builder.bearer_auth(api_key.trim()).send(),
        )
        .await
        {
            Ok(Ok(r)) if r.status().is_success() => r.json::<Value>().await.ok(),
            _ => None,
        },
        Err(_) => None,
    };

    let has_credits = evaluate_openrouter_account_has_credits(&auth_json, credits_json.as_ref());

    let list_resp = gate
        .request(
            Method::GET,
            "https://openrouter.ai/api/v1/models",
            "openrouter_models_list",
        )?
        .bearer_auth(api_key.trim())
        .send()
        .await?;
    if !list_resp.status().is_success() {
        anyhow::bail!("OpenRouter models list failed (HTTP {})", list_resp.status());
    }
    let val: Value = list_resp.json().await?;

    let auto_entry = ModelCatalogEntry {
        id: "openrouter/auto".into(),
        display_name: "OpenRouter Auto".into(),
        provider: ProviderKind::OpenRouter,
        context_window: 200_000,
        supports_thinking: false,
        supports_tools: true,
        supports_vision: false,
        tier_note: if has_credits {
            "Verified via OpenRouter (credits active)".into()
        } else {
            "OpenRouter Auto (requires paid credits)".into()
        },
        quota_limit: Some(ModelQuotaLimit {
            rpm_limit: 200,
            tpm_limit: 1_000_000,
            rpd_limit: 5_000,
            daily_token_limit: 20_000_000,
        }),
        reachable: has_credits,
        reachability_reason: if has_credits {
            "Verified reachable via live API".into()
        } else {
            "Requires paid OpenRouter credits".into()
        },
        antigravity_alias: Some("OpenRouter Auto".into()),
        requires_payment: !has_credits,
    };

    let mut free_entries: Vec<ModelCatalogEntry> = Vec::new();
    let mut paid_entries: Vec<ModelCatalogEntry> = Vec::new();

    for item in val["data"].as_array().cloned().unwrap_or_default() {
        let Some(id) = item["id"].as_str() else {
            continue;
        };
        if id == "openrouter/auto" {
            continue;
        }
        let is_free = is_openrouter_model_free(id, &item);
        if is_free && free_entries.len() >= 40 {
            continue;
        }
        if !is_free && paid_entries.len() >= 40 {
            continue;
        }
        if free_entries.len() >= 40 && paid_entries.len() >= 40 {
            break;
        }

        let name = item["name"].as_str().unwrap_or(id).to_string();
        let ctx = item["context_length"].as_u64().unwrap_or(128_000);
        let reachable = is_free || has_credits;
        let requires_payment = !is_free && !has_credits;
        let reason = if is_free {
            "Free tier · Verified reachable".to_string()
        } else if has_credits {
            "Verified reachable (paid credits active)".to_string()
        } else {
            "Requires paid OpenRouter credits".to_string()
        };
        let entry = ModelCatalogEntry {
            id: id.to_string(),
            display_name: name.clone(),
            provider: ProviderKind::OpenRouter,
            context_window: ctx,
            supports_thinking: id.contains("thinking") || id.contains("r1"),
            supports_tools: true,
            supports_vision: false,
            tier_note: if is_free {
                "OpenRouter Free Tier".into()
            } else if has_credits {
                "Verified via OpenRouter".into()
            } else {
                "OpenRouter Paid Model".into()
            },
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: if is_free { 20 } else { 200 },
                tpm_limit: if is_free { 200_000 } else { 1_000_000 },
                rpd_limit: if is_free { 200 } else { 5_000 },
                daily_token_limit: if is_free { 2_000_000 } else { 20_000_000 },
            }),
            reachable,
            reachability_reason: reason,
            antigravity_alias: Some(name),
            requires_payment,
        };
        if is_free {
            free_entries.push(entry);
        } else {
            paid_entries.push(entry);
        }
    }

    let mut out = Vec::new();
    if has_credits {
        out.push(auto_entry);
        out.extend(free_entries);
        out.extend(paid_entries);
    } else {
        out.extend(free_entries);
        out.push(auto_entry);
        out.extend(paid_entries);
    }
    Ok(out)
}

async fn sync_openai_compat_models(
    gate: &NetworkGate,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<Vec<ModelCatalogEntry>> {
    let url = format!("{}/models", base_url.trim().trim_end_matches('/'));
    let mut req = gate.request(Method::GET, &url, "openai_compat_verify")?;
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Endpoint HTTP {}", resp.status());
    }
    let val: Value = resp.json().await?;
    let mut out = Vec::new();
    for item in val["data"].as_array().cloned().unwrap_or_default().into_iter().take(30) {
        if let Some(id) = item["id"].as_str() {
            out.push(ModelCatalogEntry {
                id: id.to_string(),
                display_name: id.to_string(),
                provider: ProviderKind::OpenAiCompat,
                context_window: 128_000,
                supports_thinking: false,
                supports_tools: true,
                supports_vision: false,
                tier_note: "Verified via custom OpenAI endpoint".into(),
                quota_limit: None,
                reachable: true,
                reachability_reason: "Verified reachable via live API".into(),
                antigravity_alias: Some(id.to_string()),
                requires_payment: false,
            });
        }
    }
    Ok(out)
}

fn collect_root_defs(schema: &Value) -> serde_json::Map<String, Value> {
    let mut defs = serde_json::Map::new();
    for key in ["$defs", "definitions"] {
        if let Some(obj) = schema.get(key).and_then(|v| v.as_object()) {
            for (k, v) in obj {
                defs.insert(k.clone(), v.clone());
            }
        }
    }
    defs
}

fn map_json_type_to_gemini(t: &str) -> Option<&'static str> {
    match t.trim().to_ascii_lowercase().as_str() {
        "string" => Some("STRING"),
        "integer" => Some("INTEGER"),
        "number" => Some("NUMBER"),
        "boolean" => Some("BOOLEAN"),
        "array" => Some("ARRAY"),
        "object" => Some("OBJECT"),
        _ => None,
    }
}

fn is_null_schema_variant(v: &Value) -> bool {
    if let Some(t) = v.get("type") {
        if t.as_str()
            .map(|s| s.eq_ignore_ascii_case("null"))
            .unwrap_or(false)
        {
            return true;
        }
        if let Some(arr) = t.as_array() {
            if !arr.is_empty()
                && arr
                    .iter()
                    .all(|x| x.as_str().map(|s| s.eq_ignore_ascii_case("null")).unwrap_or(false))
            {
                return true;
            }
        }
    }
    v.get("const").map(|c| c.is_null()).unwrap_or(false)
}

fn clean_schema_for_gemini(schema: &Value) -> Value {
    let root_defs = collect_root_defs(schema);
    clean_schema_for_gemini_inner(schema, &root_defs, 0)
}

fn clean_schema_for_gemini_inner(
    schema: &Value,
    root_defs: &serde_json::Map<String, Value>,
    depth: usize,
) -> Value {
    if depth > 8 {
        return json!({ "type": "STRING" });
    }
    let Some(map) = schema.as_object() else {
        return json!({ "type": "STRING" });
    };

    let outer_desc = map
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut nullable = map
        .get("nullable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Inline `#/$defs/...` or `#/definitions/...` JSON Schema refs
    if let Some(ref_str) = map.get("$ref").and_then(|v| v.as_str()) {
        let def_name = ref_str
            .strip_prefix("#/$defs/")
            .or_else(|| ref_str.strip_prefix("#/definitions/"))
            .or_else(|| ref_str.rsplit('/').next())
            .unwrap_or("");
        let mut resolved = root_defs
            .get(def_name)
            .map(|target| clean_schema_for_gemini_inner(target, root_defs, depth + 1))
            .unwrap_or_else(|| json!({ "type": "STRING" }));
        if let Value::Object(ref mut out) = resolved {
            if !out.contains_key("description") {
                if let Some(desc) = outer_desc {
                    out.insert("description".into(), Value::String(desc));
                }
            }
            if nullable {
                out.insert("nullable".into(), Value::Bool(true));
            }
        }
        return resolved;
    }

    // Flatten `anyOf` / `oneOf` (e.g. Pydantic Optional[T] / union schemas) into one Gemini schema + `nullable`
    if let Some(Value::Array(variants)) = map.get("anyOf").or_else(|| map.get("oneOf")) {
        if variants.iter().any(is_null_schema_variant) {
            nullable = true;
        }
        let chosen = variants
            .iter()
            .find(|v| !is_null_schema_variant(v))
            .cloned()
            .unwrap_or_else(|| json!({ "type": "STRING" }));
        let mut resolved = clean_schema_for_gemini_inner(&chosen, root_defs, depth + 1);
        if let Value::Object(ref mut out) = resolved {
            if !out.contains_key("description") {
                if let Some(desc) = outer_desc {
                    out.insert("description".into(), Value::String(desc));
                }
            }
            if nullable {
                out.insert("nullable".into(), Value::Bool(true));
            }
        }
        return resolved;
    }

    let parsed_type = match map.get("type") {
        Some(Value::String(s)) => {
            if s.eq_ignore_ascii_case("null") {
                nullable = true;
                None
            } else {
                map_json_type_to_gemini(s)
            }
        }
        Some(Value::Array(arr)) => {
            let mut found = None;
            for item in arr {
                if let Some(s) = item.as_str() {
                    if s.eq_ignore_ascii_case("null") {
                        nullable = true;
                    } else if found.is_none() {
                        found = map_json_type_to_gemini(s);
                    }
                }
            }
            found
        }
        _ => {
            if map.get("properties").and_then(|v| v.as_object()).is_some() {
                Some("OBJECT")
            } else if map.get("items").is_some() {
                Some("ARRAY")
            } else if map.get("enum").is_some() {
                Some("STRING")
            } else {
                None
            }
        }
    };

    let mut clean = serde_json::Map::new();
    let mut gemini_type = parsed_type.unwrap_or("STRING");

    if let Some(desc) = outer_desc {
        clean.insert("description".into(), Value::String(desc));
    }

    if let Some(Value::Array(vals)) = map.get("enum") {
        let mut enum_strs = Vec::new();
        for v in vals {
            if v.is_null() {
                nullable = true;
            } else if let Some(s) = v.as_str() {
                enum_strs.push(Value::String(s.to_string()));
            } else {
                enum_strs.push(Value::String(v.to_string()));
            }
        }
        if !enum_strs.is_empty() {
            gemini_type = "STRING";
            clean.insert("enum".into(), Value::Array(enum_strs));
        }
    }

    clean.insert("type".into(), Value::String(gemini_type.to_string()));
    if nullable {
        clean.insert("nullable".into(), Value::Bool(true));
    }

    if gemini_type == "ARRAY" {
        let items_schema = match map.get("items") {
            Some(Value::Array(tuple_items)) => tuple_items
                .first()
                .map(|first| clean_schema_for_gemini_inner(first, root_defs, depth + 1))
                .unwrap_or_else(|| json!({ "type": "STRING" })),
            Some(item_obj @ Value::Object(_)) => {
                clean_schema_for_gemini_inner(item_obj, root_defs, depth + 1)
            }
            _ => json!({ "type": "STRING" }),
        };
        clean.insert("items".into(), items_schema);
    } else if gemini_type == "OBJECT" {
        let mut clean_props = serde_json::Map::new();
        if let Some(props) = map.get("properties").and_then(|v| v.as_object()) {
            for (k, v) in props {
                clean_props.insert(k.clone(), clean_schema_for_gemini_inner(v, root_defs, depth + 1));
            }
        }
        if !clean_props.is_empty() {
            if let Some(Value::Array(req_arr)) = map.get("required") {
                let clean_req: Vec<Value> = req_arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .filter(|k| clean_props.contains_key(*k))
                    .map(|k| Value::String(k.to_string()))
                    .collect();
                if !clean_req.is_empty() {
                    clean.insert("required".into(), Value::Array(clean_req));
                }
            }
            clean.insert("properties".into(), Value::Object(clean_props));
        } else if depth > 0 {
            // Gemini v1beta rejects nested OBJECT properties with empty `properties`.
            clean.insert("type".into(), Value::String("STRING".into()));
        }
    }

    Value::Object(clean)
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    h.windows(n.len()).position(|w| {
        w.iter()
            .zip(n.iter())
            .all(|(&a, &b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
    })
}

pub fn extract_think_tags(raw: &str) -> (String, Option<String>) {
    const TAG_PAIRS: &[(&str, &str)] = &[
        ("<think>", "</think>"),
        ("<thinking>", "</thinking>"),
        ("<thought>", "</thought>"),
        ("<|thought|>", "</|thought|>"),
        ("<|channel|>thought", "<|message|>"),
    ];

    let mut working = raw.to_string();
    let mut thoughts: Vec<String> = Vec::new();

    // 1. Extract all closed or unclosed opening-tag thought blocks.
    loop {
        let mut earliest: Option<(usize, usize, &str)> = None;
        for &(open, close) in TAG_PAIRS {
            if let Some(pos) = find_ascii_ci(&working, open) {
                if earliest.map(|(e, _, _)| pos < e).unwrap_or(true) {
                    earliest = Some((pos, open.len(), close));
                }
            }
        }
        let Some((start, open_len, close_tag)) = earliest else {
            break;
        };
        let after_open = start + open_len;
        if let Some(rel_end) = find_ascii_ci(&working[after_open..], close_tag) {
            let end = after_open + rel_end;
            let th = working[after_open..end].trim().to_string();
            if !th.is_empty() {
                thoughts.push(th);
            }
            let before = working[..start].trim_end();
            let after = working[end + close_tag.len()..].trim_start();
            working = match (before.is_empty(), after.is_empty()) {
                (true, true) => String::new(),
                (true, false) => after.to_string(),
                (false, true) => before.to_string(),
                (false, false) => format!("{before}\n\n{after}"),
            };
        } else {
            let unclosed = working[after_open..].trim().to_string();
            if !unclosed.is_empty() {
                thoughts.push(unclosed);
            }
            working = working[..start].trim().to_string();
            break;
        }
    }

    // 2. Handle orphan closing tags (e.g. llama.cpp / Ollama templates that prefill <think>).
    loop {
        let mut earliest_close: Option<(usize, usize)> = None;
        for &(_, close) in TAG_PAIRS {
            if let Some(pos) = find_ascii_ci(&working, close) {
                if earliest_close.map(|(e, _)| pos < e).unwrap_or(true) {
                    earliest_close = Some((pos, close.len()));
                }
            }
        }
        let Some((pos, close_len)) = earliest_close else {
            break;
        };
        let leaked = working[..pos].trim().to_string();
        if !leaked.is_empty() {
            thoughts.push(leaked);
        }
        working = working[pos + close_len..].trim_start().to_string();
    }

    // 3. Strip leading echoed `[Step N · ...]` lines if present.
    let mut kept_lines: Vec<&str> = Vec::new();
    let mut at_head = true;
    for line in working.lines() {
        let lt = line.trim();
        if at_head && lt.starts_with("[Step ") && lt.ends_with(']') {
            thoughts.push(lt[1..lt.len() - 1].to_string());
            continue;
        }
        if !lt.is_empty() {
            at_head = false;
        }
        kept_lines.push(line);
    }
    let clean = kept_lines.join("\n").trim().to_string();

    (
        clean,
        if thoughts.is_empty() {
            None
        } else {
            Some(thoughts.join("\n\n"))
        },
    )
}

pub fn extract_think_tags_streaming(raw: &str) -> (String, Option<String>) {
    extract_think_tags(raw)
}

pub fn parse_gemma_like_args_to_json(body: &str) -> Option<Value> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Some(json!({}));
    }
    if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
        return Some(val);
    }

    let bytes = trimmed.as_bytes();
    let mut out = String::with_capacity(trimmed.len() + 32);
    let mut i = 0usize;

    while i < bytes.len() {
        let rem = &trimmed[i..];
        if rem.starts_with("<|\"|>") {
            i += "<|\"|>".len();
            let after = &trimmed[i..];
            let end_rel = after.find("<|\"|>")?;
            let literal = &after[..end_rel];
            let quoted = serde_json::to_string(literal).ok()?;
            out.push_str(&quoted);
            i += end_rel + "<|\"|>".len();
            continue;
        }

        let ch = rem.chars().next()?;
        if ch == '"' {
            out.push('"');
            i += 1;
            while i < bytes.len() {
                let c = trimmed[i..].chars().next()?;
                out.push(c);
                i += c.len_utf8();
                if c == '\\' && i < bytes.len() {
                    let esc = trimmed[i..].chars().next()?;
                    out.push(esc);
                    i += esc.len_utf8();
                } else if c == '"' {
                    break;
                }
            }
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let prev_non_ws = out.chars().rev().find(|c| !c.is_whitespace());
            let mut end = i + ch.len_utf8();
            while end < bytes.len() {
                let nc = trimmed[end..].chars().next()?;
                if nc.is_ascii_alphanumeric() || nc == '_' {
                    end += nc.len_utf8();
                } else {
                    break;
                }
            }
            let ident = &trimmed[i..end];
            let next_non_ws = trimmed[end..].chars().find(|c| !c.is_whitespace());
            if matches!(prev_non_ws, Some('{') | Some(',')) && next_non_ws == Some(':') {
                out.push('"');
                out.push_str(ident);
                out.push('"');
                i = end;
                continue;
            }
            out.push_str(ident);
            i = end;
            continue;
        }

        out.push(ch);
        i += ch.len_utf8();
    }

    serde_json::from_str::<Value>(&out).ok()
}

fn extract_balanced_json_object(src: &str, open_brace_idx: usize) -> Option<usize> {
    if !src.get(open_brace_idx..)?.starts_with('{') {
        return None;
    }
    let bytes = src.as_bytes();
    let mut i = open_brace_idx;
    let mut depth = 0usize;
    while i < bytes.len() {
        let rem = &src[i..];
        if rem.starts_with("<|\"|>") {
            i += "<|\"|>".len();
            let rel_end = src[i..].find("<|\"|>")?;
            i += rel_end + "<|\"|>".len();
            continue;
        }
        let ch = rem.chars().next()?;
        if ch == '"' {
            i += 1;
            while i < bytes.len() {
                let c = src[i..].chars().next()?;
                i += c.len_utf8();
                if c == '\\' && i < bytes.len() {
                    let esc = src[i..].chars().next()?;
                    i += esc.len_utf8();
                } else if c == '"' {
                    break;
                }
            }
            continue;
        }
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += ch.len_utf8();
    }
    None
}

fn is_valid_inline_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn normalize_inline_tool_arguments(val: Option<&Value>) -> Value {
    match val {
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                json!({})
            } else {
                serde_json::from_str::<Value>(t)
                    .ok()
                    .or_else(|| parse_gemma_like_args_to_json(t))
                    .unwrap_or_else(|| json!({ "raw": s }))
            }
        }
        Some(Value::Null) | None => json!({}),
        Some(other) => other.clone(),
    }
}

fn parse_json_tool_object(val: &Value) -> Option<(String, Value)> {
    if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
        let args = normalize_inline_tool_arguments(
            val.get("arguments").or_else(|| val.get("parameters")),
        );
        return Some((name.trim().to_string(), args));
    }
    if let Some(func) = val.get("function").and_then(|v| v.as_object()) {
        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
            let args = normalize_inline_tool_arguments(
                func.get("arguments").or_else(|| func.get("parameters")),
            );
            return Some((name.trim().to_string(), args));
        }
    }
    None
}

fn consume_optional_close_tag(src: &str, mut idx: usize, close_tags: &[&str]) -> usize {
    let ws = src[idx..]
        .find(|c: char| !c.is_whitespace())
        .unwrap_or(src.len() - idx);
    let after_ws = idx + ws;
    for tag in close_tags {
        if src[after_ws..].starts_with(tag) {
            idx = after_ws + tag.len();
            return idx;
        }
    }
    idx
}

fn strip_markdown_code_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(after_ticks) = t.strip_prefix("```") {
        let body = if let Some(nl) = after_ticks.find('\n') {
            &after_ticks[nl + 1..]
        } else {
            after_ticks
        };
        let body = body.trim();
        return body.strip_suffix("```").unwrap_or(body).trim();
    }
    t
}

pub fn strip_live_inline_tool_markup(s: &str) -> &str {
    const OPEN_MARKERS: [&str; 7] = [
        "<|tool_call|>",
        "<tool_call>",
        "<function_call>",
        "<｜tool▁calls▁begin｜>",
        "<｜tool▁call▁begin｜>",
        "<|tool_calls_begin|>",
        "<|tool_call_begin|>",
    ];
    let mut earliest: Option<usize> = None;
    for marker in OPEN_MARKERS {
        if let Some(pos) = s.find(marker) {
            earliest = Some(earliest.map_or(pos, |e| e.min(pos)));
        }
    }
    match earliest {
        Some(pos) => s[..pos].trim_end(),
        None => s,
    }
}

/// Extract inline fallback tool calls across Gemma (`<|tool_call|>call:...`),
/// Qwen/Hermes (`<tool_call>{...}</tool_call>`), and DeepSeek
/// (`<｜tool▁call▁begin｜>...<｜tool▁call▁end｜>`) formats, stripping matched
/// raw markup from the returned assistant content.
pub fn extract_fallback_tool_calls_and_clean(
    text: &str,
    allowed_tool_names: &[&str],
) -> (String, Vec<ToolCallRequest>) {
    let is_allowed = |name: &str| -> bool {
        is_valid_inline_tool_name(name)
            && (allowed_tool_names.is_empty() || allowed_tool_names.iter().any(|&a| a == name))
    };

    let mut spans: Vec<(usize, usize, ToolCallRequest)> = Vec::new();

    // 1. Gemma 3/4 inline format: `<|tool_call|>call:<name>{...}<|tool_call|>` (or JSON inside tag)
    const GEMMA_OPEN: &str = "<|tool_call|>";
    let mut pos = 0usize;
    while let Some(rel) = text[pos..].find(GEMMA_OPEN) {
        let start = pos + rel;
        let after_open = start + GEMMA_OPEN.len();
        let ws = text[after_open..]
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(text.len() - after_open);
        let body_start = after_open + ws;
        let rem = &text[body_start..];
        let after_call = rem.strip_prefix("call:").unwrap_or(rem);
        let after_call_abs = body_start + (rem.len() - after_call.len());

        let mut matched_span_end: Option<usize> = None;
        if after_call.starts_with('{') {
            if let Some(obj_end_rel) = extract_balanced_json_object(after_call, 0) {
                if let Some(val) = parse_gemma_like_args_to_json(&after_call[..obj_end_rel]) {
                    if let Some((name, args)) = parse_json_tool_object(&val) {
                        let span_end = consume_optional_close_tag(
                            text,
                            after_call_abs + obj_end_rel,
                            &["<|/tool_call|>", "<|tool_call|>"],
                        );
                        matched_span_end = Some(span_end);
                        if is_allowed(&name) {
                            spans.push((
                                start,
                                span_end,
                                ToolCallRequest {
                                    id: Uuid::new_v4().to_string(),
                                    name,
                                    arguments: args,
                                    thought_signature: None,
                                },
                            ));
                        }
                    }
                }
            }
        } else if let Some(brace_rel) = after_call.find('{') {
            let name = after_call[..brace_rel].trim();
            if is_valid_inline_tool_name(name) {
                if let Some(obj_end_rel) = extract_balanced_json_object(after_call, brace_rel) {
                    if let Some(args) =
                        parse_gemma_like_args_to_json(&after_call[brace_rel..obj_end_rel])
                    {
                        let span_end = consume_optional_close_tag(
                            text,
                            after_call_abs + obj_end_rel,
                            &["<|/tool_call|>", "<|tool_call|>"],
                        );
                        matched_span_end = Some(span_end);
                        if is_allowed(name) {
                            spans.push((
                                start,
                                span_end,
                                ToolCallRequest {
                                    id: Uuid::new_v4().to_string(),
                                    name: name.to_string(),
                                    arguments: args,
                                    thought_signature: None,
                                },
                            ));
                        }
                    }
                }
            }
        }

        pos = matched_span_end.unwrap_or(after_open);
    }

    // 2. Qwen / Hermes / Llama / Mistral XML tags: `<tool_call>...</tool_call>` & `<function_call>...</function_call>`
    for (open_tag, close_tag) in [
        ("<tool_call>", "</tool_call>"),
        ("<function_call>", "</function_call>"),
    ] {
        let mut pos = 0usize;
        while let Some(rel) = text[pos..].find(open_tag) {
            let start = pos + rel;
            let after_open = start + open_tag.len();
            let next_open_rel = text[after_open..].find(open_tag);
            let close_rel = text[after_open..].find(close_tag);

            let (block_end, span_end) = match (close_rel, next_open_rel) {
                (Some(cr), Some(no)) if cr < no => {
                    (after_open + cr, after_open + cr + close_tag.len())
                }
                (Some(cr), None) => (after_open + cr, after_open + cr + close_tag.len()),
                (None, None) => (text.len(), text.len()),
                _ => {
                    pos = after_open;
                    continue;
                }
            };

            let inner = &text[after_open..block_end];
            let mut extracted_here: Vec<ToolCallRequest> = Vec::new();
            let mut idx = 0usize;
            while let Some(brace_rel) = inner[idx..].find('{') {
                let brace_abs = idx + brace_rel;
                let Some(obj_end) = extract_balanced_json_object(inner, brace_abs) else {
                    break;
                };
                let prefix_before_brace = inner[idx..brace_abs].trim();
                let gemma_name = prefix_before_brace
                    .strip_prefix("call:")
                    .unwrap_or(prefix_before_brace)
                    .trim();

                let json_slice = &inner[brace_abs..obj_end];
                if let Some(val) = parse_gemma_like_args_to_json(json_slice) {
                    if let Some((name, args)) = parse_json_tool_object(&val) {
                        if is_allowed(&name) {
                            extracted_here.push(ToolCallRequest {
                                id: Uuid::new_v4().to_string(),
                                name,
                                arguments: args,
                                thought_signature: None,
                            });
                        }
                    } else if is_allowed(gemma_name) {
                        extracted_here.push(ToolCallRequest {
                            id: Uuid::new_v4().to_string(),
                            name: gemma_name.to_string(),
                            arguments: val,
                            thought_signature: None,
                        });
                    }
                }
                idx = obj_end;
            }

            if !extracted_here.is_empty() {
                for req in extracted_here {
                    spans.push((start, span_end, req));
                }
            }
            pos = span_end;
        }
    }

    // 3. DeepSeek R1/V3 inline tokens: `<｜tool▁call▁begin｜>...<｜tool▁sep｜><name>...<｜tool▁call▁end｜>`
    for (open_tok, sep_tok, close_tok, calls_end_tok) in [
        (
            "<｜tool▁call▁begin｜>",
            "<｜tool▁sep｜>",
            "<｜tool▁call▁end｜>",
            "<｜tool▁calls▁end｜>",
        ),
        (
            "<|tool_call_begin|>",
            "<|tool_sep|>",
            "<|tool_call_end|>",
            "<|tool_calls_end|>",
        ),
    ] {
        let mut pos = 0usize;
        while let Some(rel) = text[pos..].find(open_tok) {
            let start = pos + rel;
            let after_open = start + open_tok.len();
            let (block_end, span_end) = if let Some(cr) = text[after_open..].find(close_tok) {
                (after_open + cr, after_open + cr + close_tok.len())
            } else if let Some(ce) = text[after_open..].find(calls_end_tok) {
                (after_open + ce, after_open + ce)
            } else {
                (text.len(), text.len())
            };

            let inner = &text[after_open..block_end];
            let payload_src = if let Some(sep_idx) = inner.find(sep_tok) {
                &inner[sep_idx + sep_tok.len()..]
            } else {
                inner
            };
            let trimmed_payload = payload_src.trim_start();
            let name_len = trimmed_payload
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
                .unwrap_or(trimmed_payload.len());
            let name = &trimmed_payload[..name_len];
            let arg_body = strip_markdown_code_fence(&trimmed_payload[name_len..]);
            let parsed_args = if arg_body.is_empty() {
                Some(json!({}))
            } else if let Some(brace_rel) = arg_body.find('{') {
                extract_balanced_json_object(arg_body, brace_rel)
                    .and_then(|end| parse_gemma_like_args_to_json(&arg_body[brace_rel..end]))
            } else {
                parse_gemma_like_args_to_json(arg_body)
            };

            if is_allowed(name) {
                if let Some(args) = parsed_args {
                    spans.push((
                        start,
                        span_end,
                        ToolCallRequest {
                            id: Uuid::new_v4().to_string(),
                            name: name.to_string(),
                            arguments: args,
                            thought_signature: None,
                        },
                    ));
                }
            }
            pos = span_end.max(after_open);
        }
    }

    if spans.is_empty() {
        return (text.to_string(), Vec::new());
    }

    spans.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    // Keep overlapping spans that share the exact same [start, end) (e.g. multi-call XML block)
    // and discard nested/partially overlapping spans with a strictly smaller/crossed start.
    let mut accepted_spans: Vec<(usize, usize)> = Vec::new();
    let mut tool_calls: Vec<ToolCallRequest> = Vec::with_capacity(spans.len());
    let mut last_end = 0usize;
    let mut last_exact_span: Option<(usize, usize)> = None;
    for (s, e, req) in spans {
        if last_exact_span == Some((s, e)) {
            tool_calls.push(req);
            continue;
        }
        if s < last_end {
            continue;
        }
        accepted_spans.push((s, e));
        last_exact_span = Some((s, e));
        last_end = e;
        tool_calls.push(req);
    }

    let mut stitched = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (s, e) in accepted_spans {
        if s > cursor {
            stitched.push_str(&text[cursor..s]);
        }
        cursor = e;
    }
    if cursor < text.len() {
        stitched.push_str(&text[cursor..]);
    }

    for wrapper in [
        "<｜tool▁calls▁begin｜>",
        "<｜tool▁calls▁end｜>",
        "<|tool_calls_begin|>",
        "<|tool_calls_end|>",
    ] {
        if stitched.contains(wrapper) {
            stitched = stitched.replace(wrapper, "");
        }
    }

    // Collapse runs of 3+ consecutive newlines caused by stripping tool blocks.
    let mut collapsed = String::with_capacity(stitched.len());
    let mut empty_run = 0usize;
    for line in stitched.lines() {
        if line.trim().is_empty() {
            empty_run += 1;
            if empty_run <= 1 && !collapsed.is_empty() {
                collapsed.push('\n');
            }
        } else {
            empty_run = 0;
            if !collapsed.is_empty() {
                collapsed.push('\n');
            }
            collapsed.push_str(line.trim_end());
        }
    }

    (collapsed.trim().to_string(), tool_calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_think_tags_cleanly() {
        let (clean, thought) =
            extract_think_tags("<think>step 1\nstep 2</think>\nFinal answer here.");
        assert_eq!(clean, "Final answer here.");
        assert_eq!(thought.as_deref(), Some("step 1\nstep 2"));

        let (partial_clean, partial_thought) =
            extract_think_tags_streaming("<think>reasoning in progress");
        assert_eq!(partial_clean, "");
        assert_eq!(partial_thought.as_deref(), Some("reasoning in progress"));

        let (orphan_clean, orphan_thought) =
            extract_think_tags("Checking workspace files now.\n</think>\n### Summary\nDone.");
        assert_eq!(orphan_clean, "### Summary\nDone.");
        assert_eq!(
            orphan_thought.as_deref(),
            Some("Checking workspace files now.")
        );

        let (multi_clean, multi_thought) = extract_think_tags(
            "<thinking>first check</thinking>\n\n<thought>second check</thought>\nFinal result.",
        );
        assert_eq!(multi_clean, "Final result.");
        assert_eq!(
            multi_thought.as_deref(),
            Some("first check\n\nsecond check")
        );
    }

    #[test]
    fn classifies_auto_switch_errors_across_providers() {
        let (sw, blk, _) = classify_provider_error("Gemini API HTTP 429: limit: 0 RESOURCE_EXHAUSTED");
        assert!(sw && blk);

        let (sw2, blk2, _) = classify_provider_error("Anthropic API HTTP 529: Overloaded");
        assert!(sw2 && !blk2);

        let (sw3, blk3, _) = classify_provider_error("Endpoint returned HTTP 429 Too Many Requests");
        assert!(sw3 && !blk3);

        let (sw4, blk4, _) = classify_provider_error("Gemini API HTTP 400: invalid argument");
        assert!(!sw4 && !blk4);

        let (sw5, blk5, _) =
            classify_provider_error("Gemini API request timed out after 12s for model 'gemini-3.7-flash'");
        assert!(sw5 && !blk5);

        let (sw6, blk6, _) =
            classify_provider_error("error sending request for url: connection closed before message completed");
        assert!(sw6 && !blk6);

        let (sw7, blk7, r7) =
            classify_provider_error("OpenRouter HTTP 402: Insufficient credits / payment required");
        assert!(sw7 && blk7 && r7.contains("paid credits"));

        let (sw8, blk8, _) =
            classify_provider_error("HTTP 400: maximum context length is 128000 tokens");
        assert!(!sw8 && !blk8);

        let (sw9, blk9, _) =
            classify_provider_error("OpenRouter HTTP 404: No endpoints found for this model");
        assert!(sw9 && blk9);

        let (sw10, blk10, _) =
            classify_provider_error("OpenRouter HTTP 502: Provider returned error");
        assert!(sw10 && !blk10);
    }

    #[test]
    fn evaluates_openrouter_credits_and_free_models() {
        let free_auth = json!({ "data": { "is_free_tier": true, "limit_remaining": null } });
        assert!(!evaluate_openrouter_account_has_credits(&free_auth, None));

        let paid_credits = json!({ "data": { "total_credits": 10.0, "total_usage": 2.5 } });
        assert!(evaluate_openrouter_account_has_credits(
            &free_auth,
            Some(&paid_credits)
        ));

        let exhausted_key = json!({ "data": { "is_free_tier": false, "limit_remaining": 0.0 } });
        assert!(!evaluate_openrouter_account_has_credits(
            &exhausted_key,
            Some(&paid_credits)
        ));

        assert!(is_openrouter_model_free(
            "meta-llama/llama-3.3-70b-instruct:free",
            &json!({ "pricing": { "prompt": "0", "completion": "0" } })
        ));
        assert!(is_openrouter_model_free(
            "custom/zero-cost",
            &json!({ "pricing": { "prompt": "0", "completion": "0.0" } })
        ));
        assert!(!is_openrouter_model_free(
            "anthropic/claude-sonnet-4",
            &json!({ "pricing": { "prompt": "0.000003", "completion": "0.000015" } })
        ));
    }

    #[test]
    fn builds_cross_provider_and_optional_local_fallbacks() {
        use harness_models::{FitTier, LocalModelSource, VramFitAssessment};
        let cat = vec![
            ModelCatalogEntry {
                id: "gemini-3-flash-preview".into(),
                display_name: "Gemini 3 Flash".into(),
                provider: ProviderKind::GoogleGemini,
                context_window: 1_000_000,
                supports_thinking: true,
                supports_tools: true,
                supports_vision: true,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: false,
            },
            ModelCatalogEntry {
                id: "claude-sonnet-4-5".into(),
                display_name: "Claude".into(),
                provider: ProviderKind::Anthropic,
                context_window: 200_000,
                supports_thinking: true,
                supports_tools: true,
                supports_vision: true,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: false,
            },
        ];
        let locals = vec![LocalModelEntry {
            id: "local::qwen3.5:9b".into(),
            name: "qwen3.5:9b".into(),
            source: LocalModelSource::VaultGguf,
            gguf_path: "qwen.gguf".into(),
            ollama_tag: None,
            mmproj_path: None,
            size_bytes: 5_000_000_000,
            quant: "Q4_K_M".into(),
            is_embedding: false,
            is_valid_gguf: true,
            vram_fit: VramFitAssessment {
                tier: FitTier::FitsInVram,
                label: "Full GPU".into(),
                weights_mb: 5000,
                kv_cache_mb: 512,
                compute_buffer_mb: 512,
                estimated_total_mb: 6024,
                recommended_ngl: 99,
            },
        }];

        let cloud_only = build_fallback_candidates(
            "gemini-3-flash-preview",
            ProviderKind::GoogleGemini,
            &cat,
            &locals,
            false,
        );
        assert_eq!(cloud_only, vec!["claude-sonnet-4-5"]);

        let with_local = build_fallback_candidates(
            "gemini-3-flash-preview",
            ProviderKind::GoogleGemini,
            &cat,
            &locals,
            true,
        );
        assert_eq!(with_local, vec!["claude-sonnet-4-5", "local::qwen3.5:9b"]);
    }

    #[test]
    fn fallback_candidates_for_request_honors_user_chain_capabilities_and_strict_discovery() {
        let cat = vec![
            ModelCatalogEntry {
                id: "gemini-3-flash-preview".into(),
                display_name: "Gemini 3 Flash".into(),
                provider: ProviderKind::GoogleGemini,
                context_window: 1_000_000,
                supports_thinking: true,
                supports_tools: true,
                supports_vision: true,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: false,
            },
            ModelCatalogEntry {
                id: "claude-sonnet-4-5".into(),
                display_name: "Claude Sonnet".into(),
                provider: ProviderKind::Anthropic,
                context_window: 200_000,
                supports_thinking: true,
                supports_tools: true,
                supports_vision: true,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: false,
            },
            ModelCatalogEntry {
                id: "openrouter/paid-pro".into(),
                display_name: "Paid Pro".into(),
                provider: ProviderKind::OpenRouter,
                context_window: 128_000,
                supports_thinking: false,
                supports_tools: true,
                supports_vision: true,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: true,
            },
            ModelCatalogEntry {
                id: "openrouter/free-chat-only".into(),
                display_name: "Chat Only".into(),
                provider: ProviderKind::OpenRouter,
                context_window: 64_000,
                supports_thinking: false,
                supports_tools: false,
                supports_vision: false,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: false,
            },
            ModelCatalogEntry {
                id: "openrouter/qwen-coder:free".into(),
                display_name: "Qwen Coder Free".into(),
                provider: ProviderKind::OpenRouter,
                context_window: 128_000,
                supports_thinking: true,
                supports_tools: true,
                supports_vision: false,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: false,
            },
        ];

        // With explicit fallback_chain & allow_auto_discovery = false:
        // - Must preserve chain order, filter out paid & non-tool & non-vision models
        // - Must NOT leak gemini-3-flash-preview since auto_discovery is false!
        let chain = vec![
            "openrouter/paid-pro".to_string(),
            "openrouter/free-chat-only".to_string(),
            "openrouter/qwen-coder:free".to_string(),
            "claude-sonnet-4-5".to_string(),
        ];
        let tools_only = build_fallback_candidates_for_request(
            "openai/primary",
            ProviderKind::OpenAiCompat,
            &cat,
            &[],
            &chain,
            false,
            false,
            true,
            true,
            false,
        );
        assert_eq!(
            tools_only,
            vec!["openrouter/qwen-coder:free", "claude-sonnet-4-5"]
        );

        let vision_required = build_fallback_candidates_for_request(
            "openai/primary",
            ProviderKind::OpenAiCompat,
            &cat,
            &[],
            &chain,
            false,
            false,
            true,
            true,
            true,
        );
        assert_eq!(vision_required, vec!["claude-sonnet-4-5"]);

        // Strict Local: allow_cloud = false drops all cloud models
        let strict_local = build_fallback_candidates_for_request(
            "openai/primary",
            ProviderKind::OpenAiCompat,
            &cat,
            &[],
            &chain,
            true,
            false,
            false,
            true,
            false,
        );
        assert!(strict_local.is_empty());
    }

    #[test]
    fn clean_schema_for_gemini_normalizes_mcp_union_types_anyof_defs_and_empty_objects() {
        let mcp_schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$defs": {
                "RefItem": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "label": { "type": "string", "minLength": 1 }
                    },
                    "required": ["label", "missing_field"]
                }
            },
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "repo_path": {
                    "type": "string",
                    "title": "Repo Path",
                    "description": "Path to repo"
                },
                "max_count": {
                    "anyOf": [{ "type": "integer", "exclusiveMinimum": 0 }, { "type": "null" }],
                    "default": null,
                    "description": "Optional limit"
                },
                "revision": {
                    "type": ["string", "null"],
                    "default": null,
                    "description": "Nullable union type list"
                },
                "tags": {
                    "type": "array"
                },
                "item": {
                    "$ref": "#/$defs/RefItem"
                }
            },
            "required": ["repo_path", "nonexistent"]
        });

        let cleaned = clean_schema_for_gemini(&mcp_schema);
        assert_eq!(cleaned["type"], "OBJECT");
        assert!(cleaned.get("$schema").is_none());
        assert!(cleaned.get("$defs").is_none());
        assert!(cleaned.get("additionalProperties").is_none());
        assert_eq!(cleaned["required"], json!(["repo_path"]));

        // Scalar union `["string", "null"]` must become scalar `"STRING"` + `nullable: true`
        let rev = &cleaned["properties"]["revision"];
        assert_eq!(rev["type"], "STRING");
        assert_eq!(rev["nullable"], true);
        assert!(rev.get("default").is_none());

        // Pydantic `anyOf: [{type: integer}, {type: null}]` -> `"INTEGER"` + `nullable: true`
        let mc = &cleaned["properties"]["max_count"];
        assert_eq!(mc["type"], "INTEGER");
        assert_eq!(mc["nullable"], true);
        assert_eq!(mc["description"], "Optional limit");
        assert!(mc.get("exclusiveMinimum").is_none());

        // Array missing `items` gets default `{"type": "STRING"}`
        assert_eq!(cleaned["properties"]["tags"]["type"], "ARRAY");
        assert_eq!(
            cleaned["properties"]["tags"]["items"],
            json!({ "type": "STRING" })
        );

        // `$ref` resolved & `required` filtered to existing properties
        let item = &cleaned["properties"]["item"];
        assert_eq!(item["type"], "OBJECT");
        assert_eq!(item["required"], json!(["label"]));
        assert!(item["properties"]["label"].get("minLength").is_none());

        // Empty-property object loses `"properties"` so declaration builder can omit `parameters`
        let empty_obj = clean_schema_for_gemini(&json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }));
        assert_eq!(empty_obj["type"], "OBJECT");
        assert!(empty_obj.get("properties").is_none());
    }

    #[test]
    fn gemini_and_anthropic_merge_parallel_tool_responses_and_coerce_null_args() {
        let msgs = vec![
            ProviderMessage {
                role: "user".into(),
                content: "Inspect repo".into(),
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
                    ToolCallRequest {
                        id: "c1".into(),
                        name: "git_status".into(),
                        arguments: Value::Null,
                        thought_signature: Some("sig1".into()),
                    },
                    ToolCallRequest {
                        id: "c2".into(),
                        name: "list_dir".into(),
                        arguments: json!({ "path": "src" }),
                        thought_signature: None,
                    },
                ],
                images: Vec::new(),
            },
            ProviderMessage {
                role: "tool".into(),
                content: "clean".into(),
                tool_call_id: Some("c1".into()),
                tool_name: Some("git_status".into()),
                tool_calls: Vec::new(),
                images: Vec::new(),
            },
            ProviderMessage {
                role: "tool".into(),
                content: "lib.rs".into(),
                tool_call_id: Some("c2".into()),
                tool_name: Some("list_dir".into()),
                tool_calls: Vec::new(),
                images: Vec::new(),
            },
        ];

        let gem = build_gemini_contents(&msgs);
        assert_eq!(gem.len(), 3, "parallel tool responses must merge into 1 user turn");
        assert_eq!(gem[1]["parts"][0]["functionCall"]["args"], json!({}));
        assert_eq!(gem[2]["role"], "user");
        let gem_parts = gem[2]["parts"].as_array().unwrap();
        assert_eq!(gem_parts.len(), 2);
        assert_eq!(gem_parts[0]["functionResponse"]["name"], "git_status");
        assert_eq!(gem_parts[1]["functionResponse"]["name"], "list_dir");

        let ant = build_anthropic_messages(&msgs);
        assert_eq!(ant.len(), 3, "parallel tool_results must merge into 1 user turn");
        assert_eq!(ant[1]["content"][0]["input"], json!({}));
        assert_eq!(ant[2]["role"], "user");
        let ant_blocks = ant[2]["content"].as_array().unwrap();
        assert_eq!(ant_blocks.len(), 2);
        assert_eq!(ant_blocks[0]["type"], "tool_result");
        assert_eq!(ant_blocks[0]["tool_use_id"], "c1");
        assert_eq!(ant_blocks[1]["tool_use_id"], "c2");
    }

    #[test]
    fn build_gemini_contents_injects_skip_thought_signature_validator_for_cross_provider_calls() {
        let no_sig_msgs = vec![ProviderMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: vec![
                ToolCallRequest {
                    id: "l1".into(),
                    name: "read_file".into(),
                    arguments: json!({ "path": "Cargo.toml" }),
                    thought_signature: None,
                },
                ToolCallRequest {
                    id: "l2".into(),
                    name: "list_dir".into(),
                    arguments: json!({ "path": "." }),
                    thought_signature: None,
                },
            ],
            images: Vec::new(),
        }];
        let gem_fallback = build_gemini_contents(&no_sig_msgs);
        assert_eq!(
            gem_fallback[0]["parts"][0]["thoughtSignature"],
            "skip_thought_signature_validator"
        );
        assert!(gem_fallback[0]["parts"][1].get("thoughtSignature").is_none());

        let with_sig_msgs = vec![ProviderMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: vec![
                ToolCallRequest {
                    id: "g1".into(),
                    name: "read_file".into(),
                    arguments: json!({ "path": "Cargo.toml" }),
                    thought_signature: Some("sig_real".into()),
                },
                ToolCallRequest {
                    id: "g2".into(),
                    name: "list_dir".into(),
                    arguments: json!({ "path": "." }),
                    thought_signature: None,
                },
            ],
            images: Vec::new(),
        }];
        let gem_real = build_gemini_contents(&with_sig_msgs);
        assert_eq!(gem_real[0]["parts"][0]["thoughtSignature"], "sig_real");
        assert!(gem_real[0]["parts"][1].get("thoughtSignature").is_none());
    }

    #[test]
    fn local_transport_error_detector_matches_connection_resets_only() {
        assert!(is_local_transport_error(
            "error sending request for url (http://127.0.0.1:55997/v1/chat/completions)"
        ));
        assert!(is_local_transport_error("connection reset by peer"));
        assert!(!is_local_transport_error("HTTP 400 Bad Request"));
        assert!(!is_local_transport_error("context length exceeded"));
    }

    #[test]
    fn openai_stream_accumulator_assembles_tokens_reasoning_and_chunked_tool_calls() {
        let mut acc = OpenAiStreamAccumulator::default();
        assert!(acc.apply_chunk(&json!({
            "choices": [{ "delta": { "reasoning_content": "Check files first." } }]
        })));
        assert!(acc.apply_chunk(&json!({
            "choices": [{ "delta": { "content": "<think>Hidden</think>Hello " } }]
        })));
        assert!(acc.apply_chunk(&json!({
            "choices": [{ "delta": { "content": "world" } }]
        })));
        assert!(acc.apply_chunk(&json!({
            "choices": [{ "delta": {
                "tool_calls": [{ "index": 0, "id": "call_1", "function": { "name": "read_", "arguments": "{\"path\":" } }]
            }}]
        })));
        assert!(acc.apply_chunk(&json!({
            "choices": [{ "delta": {
                "tool_calls": [{ "index": 0, "function": { "name": "file", "arguments": "\"Cargo.toml\"}" } }]
            }}]
        })));
        let _ = acc.apply_chunk(&json!({
            "usage": { "prompt_tokens": 12, "completion_tokens": 9, "completion_tokens_details": { "reasoning_tokens": 4 } }
        }));

        let (live_th, live_txt) = acc.live_deltas();
        assert!(live_th.contains("Check files first."));
        assert!(live_th.contains("Hidden"));
        assert_eq!(live_txt.trim(), "Hello world");

        let req = CompletionRequest {
            model_id: "openrouter::demo".into(),
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            enable_thinking: true,
            max_tokens: 1024,
            temperature: 0.2,
        };
        let resp = acc.into_response(&req, 42);
        assert_eq!(resp.content.trim(), "Hello world");
        let th = resp.thinking.expect("should preserve combined reasoning");
        assert!(th.contains("Check files first."));
        assert!(th.contains("Hidden"));
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_1");
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].arguments, json!({ "path": "Cargo.toml" }));
        assert_eq!(resp.prompt_tokens, 12);
        assert_eq!(resp.completion_tokens, 9);
        assert_eq!(resp.thinking_tokens, 4);
    }

    #[test]
    fn anthropic_stream_accumulator_assembles_thinking_text_and_tool_use_deltas() {
        let mut acc = AnthropicStreamAccumulator::default();
        acc.apply_chunk(&json!({
            "type": "message_start",
            "message": { "usage": { "input_tokens": 30 } }
        }));
        acc.apply_chunk(&json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": { "type": "thinking" }
        }));
        acc.apply_chunk(&json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "thinking_delta", "thinking": "Step A" }
        }));
        acc.apply_chunk(&json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": { "type": "text" }
        }));
        acc.apply_chunk(&json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": { "type": "text_delta", "text": "Reading Cargo.toml." }
        }));
        acc.apply_chunk(&json!({
            "type": "content_block_start",
            "index": 2,
            "content_block": { "type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {} }
        }));
        acc.apply_chunk(&json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": { "type": "input_json_delta", "partial_json": "{\"path\":" }
        }));
        acc.apply_chunk(&json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": { "type": "input_json_delta", "partial_json": "\"Cargo.toml\"}" }
        }));
        acc.apply_chunk(&json!({
            "type": "message_delta",
            "usage": { "output_tokens": 18 }
        }));

        let resp = acc.into_response("claude-sonnet-4-5", 88);
        assert_eq!(resp.content, "Reading Cargo.toml.");
        assert_eq!(resp.thinking.as_deref(), Some("Step A"));
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "toolu_1");
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].arguments, json!({ "path": "Cargo.toml" }));
        assert_eq!(resp.prompt_tokens, 30);
        assert_eq!(resp.completion_tokens, 18);
    }

    #[test]
    fn extracts_gemma_qwen_and_deepseek_inline_tool_calls_and_cleans_text() {
        let gemma = "Let me check that file.\n<|tool_call|>call:read_file{path:<|\"|>src/main.rs<|\"|>,start_line:1}<|tool_call|>";
        let (clean_g, calls_g) =
            extract_fallback_tool_calls_and_clean(gemma, &["read_file", "list_dir"]);
        assert_eq!(clean_g, "Let me check that file.");
        assert_eq!(calls_g.len(), 1);
        assert_eq!(calls_g[0].name, "read_file");
        assert_eq!(calls_g[0].arguments["path"], "src/main.rs");
        assert_eq!(calls_g[0].arguments["start_line"], 1);

        let gemma_nested = "<|tool_call|>call:submit_plan{plan_title:<|\"|>Site<|\"|>,plan_steps:[{description:<|\"|>Step 1<|\"|>,file_path:<|\"|>a.rs<|\"|>}]}<|tool_call|>";
        let (clean_gn, calls_gn) =
            extract_fallback_tool_calls_and_clean(gemma_nested, &["submit_plan"]);
        assert!(clean_gn.is_empty());
        assert_eq!(calls_gn.len(), 1);
        assert_eq!(calls_gn[0].name, "submit_plan");
        assert_eq!(calls_gn[0].arguments["plan_steps"][0]["file_path"], "a.rs");

        let qwen = "Scanning files...\n<tool_call>\n{\"name\":\"list_dir\",\"arguments\":{\"path\":\"src\"}}\n</tool_call>\n<tool_call>\n{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}\n</tool_call>";
        let (clean_q, calls_q) =
            extract_fallback_tool_calls_and_clean(qwen, &["list_dir", "read_file"]);
        assert_eq!(clean_q, "Scanning files...");
        assert_eq!(calls_q.len(), 2);
        assert_eq!(calls_q[0].name, "list_dir");
        assert_eq!(calls_q[0].arguments, json!({ "path": "src" }));
        assert_eq!(calls_q[1].name, "read_file");
        assert_eq!(calls_q[1].arguments, json!({ "path": "Cargo.toml" }));

        let deepseek = "Searching repository.\n<｜tool▁calls▁begin｜><｜tool▁call▁begin｜>function<｜tool▁sep｜>grep_search\n```json\n{\"query\":\"TODO\"}\n```<｜tool▁call▁end｜><｜tool▁calls▁end｜>";
        let (clean_d, calls_d) = extract_fallback_tool_calls_and_clean(deepseek, &["grep_search"]);
        assert_eq!(clean_d, "Searching repository.");
        assert_eq!(calls_d.len(), 1);
        assert_eq!(calls_d[0].name, "grep_search");
        assert_eq!(calls_d[0].arguments, json!({ "query": "TODO" }));
    }

    #[test]
    fn inline_tool_parser_respects_allowlist_and_live_stripper() {
        let disallowed =
            "Documentation example:\n<tool_call>{\"name\":\"unknown_tool\",\"arguments\":{}}</tool_call>";
        let (clean, calls) = extract_fallback_tool_calls_and_clean(disallowed, &["read_file"]);
        assert!(calls.is_empty());
        assert_eq!(clean, disallowed);

        let streaming = "Checking now.\n<|tool_call|>call:read_file{path:<|\"|>Cargo.toml";
        assert_eq!(strip_live_inline_tool_markup(streaming), "Checking now.");
    }

    #[test]
    fn openai_stream_accumulator_extracts_llama_cpp_timings_and_local_fallback() {
        let mut acc = OpenAiStreamAccumulator::default();
        acc.apply_chunk(&json!({
            "choices": [{ "delta": { "content": "Hello from local model!" } }],
            "timings": { "prompt_n": 42, "predicted_n": 7 }
        }));
        let req = CompletionRequest {
            model_id: "local::qwen3.5:9b".into(),
            system_prompt: "sys".into(),
            messages: vec![],
            tools: vec![],
            enable_thinking: false,
            max_tokens: 256,
            temperature: 0.2,
        };
        let resp = acc.into_response(&req, 100);
        assert_eq!(resp.prompt_tokens, 42);
        assert_eq!(resp.completion_tokens, 7);

        let mut acc_fallback = OpenAiStreamAccumulator::default();
        acc_fallback.apply_chunk(&json!({
            "choices": [{ "delta": { "content": "Generated forty-six characters of output text!" } }]
        }));
        let resp_fb = acc_fallback.into_response(&req, 50);
        assert!(resp_fb.prompt_tokens >= 1);
        assert!(resp_fb.completion_tokens >= 10);
    }
}
