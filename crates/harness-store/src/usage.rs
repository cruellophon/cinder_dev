use crate::types::{
    ApiKeys, ModelCatalogEntry, ModelQuotaLimit, ModelQuotaSnapshot, ModelUsageState,
    PersistedState, ProviderKind, QuotaTier,
};
use chrono::{Duration as ChronoDuration, Utc};

pub fn is_viable_gemini_chat_model(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if !(lower.starts_with("gemini-") || lower.starts_with("gemma-")) {
        return false;
    }
    if lower == "gemini-3.5-flash" || lower.ends_with("/gemini-3.5-flash") {
        return false;
    }
    // Exclude non-chat, media, specialized, retired 404, or unstable hanging /v1beta series
    let excluded_substrings = [
        "embedding",
        "aqa",
        "tts",
        "image",
        "banana",
        "transcribe",
        "robotics",
        "computer-use",
        "veo",
        "imagen",
        "audio",
        "lyria",
        "deep-research",
        "antigravity-preview",
        "customtools",
        "gemini-2.5-",
        "gemini-2.0-",
        "gemini-3.7-",
        "gemini-3.8-",
        "gemini-4-",
        "gemini-omni",
        "gemini-flash-latest",
        "gemini-pro-latest",
        "gemma-3-",
        "gemma-4-31b",
    ];
    !excluded_substrings.iter().any(|bad| lower.contains(bad))
}

pub fn gemini_model_priority_rank(id: &str) -> usize {
    const PRIORITY: &[&str] = &[
        "gemini-3-flash-preview",
        "gemini-3.1-flash-lite",
        "gemini-3.6-flash",
        "gemini-3.5-flash-lite",
        "gemini-3.1-flash-lite-preview",
        "gemini-flash-lite-latest",
        "gemma-4-26b-a4b-it",
    ];
    PRIORITY
        .iter()
        .position(|p| p.eq_ignore_ascii_case(id))
        .unwrap_or(PRIORITY.len())
}

pub fn sort_gemini_models_by_priority(models: &mut [ModelCatalogEntry]) {
    models.sort_by(|a, b| {
        gemini_model_priority_rank(&a.id)
            .cmp(&gemini_model_priority_rank(&b.id))
            .then_with(|| a.id.cmp(&b.id))
    });
}

pub fn default_google_pro_catalog(tier: QuotaTier) -> Vec<ModelCatalogEntry> {
    let scale = match tier {
        QuotaTier::FreeTier => 0.1,
        QuotaTier::GoogleAiPro | QuotaTier::Custom => 1.0,
    };
    let s = |v: u64| ((v as f64) * scale).max(1.0) as u64;

    vec![
        ModelCatalogEntry {
            id: "gemini-3-flash-preview".into(),
            display_name: "Gemini 3 Flash".into(),
            provider: ProviderKind::GoogleGemini,
            context_window: 1_000_000,
            supports_thinking: true,
            supports_tools: true,
            supports_vision: true,
            tier_note: "Fast reasoning, agentic tools & coding".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: s(1_000),
                tpm_limit: s(4_000_000),
                rpd_limit: s(10_000),
                daily_token_limit: s(25_000_000),
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Gemini 3 Flash (Reasoning & Agent)".into()),
        },
        ModelCatalogEntry {
            id: "gemini-3.5-flash-lite".into(),
            display_name: "Gemini 3.5 Flash-Lite".into(),
            provider: ProviderKind::GoogleGemini,
            context_window: 1_000_000,
            supports_thinking: false,
            supports_tools: true,
            supports_vision: true,
            tier_note: "High-speed coding & lightweight agent tasks".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: s(4_000),
                tpm_limit: s(4_000_000),
                rpd_limit: s(20_000),
                daily_token_limit: s(50_000_000),
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Gemini 3.5 Flash-Lite (Fast Execution)".into()),
        },
        ModelCatalogEntry {
            id: "gemini-3.1-flash-lite".into(),
            display_name: "Gemini 3.1 Flash-Lite".into(),
            provider: ProviderKind::GoogleGemini,
            context_window: 1_000_000,
            supports_thinking: false,
            supports_tools: true,
            supports_vision: true,
            tier_note: "Fast general chat & tool execution".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: s(4_000),
                tpm_limit: s(4_000_000),
                rpd_limit: s(20_000),
                daily_token_limit: s(50_000_000),
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Gemini 3.1 Flash-Lite".into()),
        },
        ModelCatalogEntry {
            id: "gemini-flash-lite-latest".into(),
            display_name: "Gemini Flash-Lite Latest".into(),
            provider: ProviderKind::GoogleGemini,
            context_window: 1_000_000,
            supports_thinking: false,
            supports_tools: true,
            supports_vision: true,
            tier_note: "Rolling Flash-Lite alias on Gemini API".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: s(4_000),
                tpm_limit: s(4_000_000),
                rpd_limit: s(20_000),
                daily_token_limit: s(50_000_000),
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Gemini Flash-Lite Latest".into()),
        },
        ModelCatalogEntry {
            id: "gemini-3.1-flash-lite-preview".into(),
            display_name: "Gemini 3.1 Flash-Lite Preview".into(),
            provider: ProviderKind::GoogleGemini,
            context_window: 1_000_000,
            supports_thinking: false,
            supports_tools: true,
            supports_vision: true,
            tier_note: "Preview fast Flash-Lite release".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: s(4_000),
                tpm_limit: s(4_000_000),
                rpd_limit: s(20_000),
                daily_token_limit: s(50_000_000),
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Gemini 3.1 Flash-Lite Preview".into()),
        },
        ModelCatalogEntry {
            id: "gemma-4-26b-a4b-it".into(),
            display_name: "Gemma 4 26B A4B MoE (Google API)".into(),
            provider: ProviderKind::GoogleGemini,
            context_window: 128_000,
            supports_thinking: false,
            supports_tools: false,
            supports_vision: false,
            tier_note: "Gemma 4 26B A4B sparse MoE on Gemini API".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: s(30),
                tpm_limit: s(15_000),
                rpd_limit: s(14_400),
                daily_token_limit: s(5_000_000),
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Gemma 4 26B A4B MoE".into()),
        },
        ModelCatalogEntry {
            id: "claude-sonnet-4-5".into(),
            display_name: "Claude Sonnet 4.5 (Anthropic)".into(),
            provider: ProviderKind::Anthropic,
            context_window: 200_000,
            supports_thinking: true,
            supports_tools: true,
            supports_vision: true,
            tier_note: "Antigravity-viable coding model via Anthropic key".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: 50,
                tpm_limit: 400_000,
                rpd_limit: 2_000,
                daily_token_limit: 10_000_000,
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Claude Sonnet 4.5".into()),
        },
        ModelCatalogEntry {
            id: "claude-opus-4-5".into(),
            display_name: "Claude Opus 4.5 (Anthropic)".into(),
            provider: ProviderKind::Anthropic,
            context_window: 200_000,
            supports_thinking: true,
            supports_tools: true,
            supports_vision: true,
            tier_note: "Antigravity-viable deep reasoning via Anthropic key".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: 50,
                tpm_limit: 400_000,
                rpd_limit: 1_000,
                daily_token_limit: 5_000_000,
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("Claude Opus 4.5".into()),
        },
        ModelCatalogEntry {
            id: "openrouter/auto".into(),
            display_name: "OpenRouter Auto".into(),
            provider: ProviderKind::OpenRouter,
            context_window: 200_000,
            supports_thinking: false,
            supports_tools: true,
            supports_vision: false,
            tier_note: "Access Gemini, Claude, DeepSeek, Qwen via OpenRouter".into(),
            quota_limit: Some(ModelQuotaLimit {
                rpm_limit: 200,
                tpm_limit: 1_000_000,
                rpd_limit: 5_000,
                daily_token_limit: 20_000_000,
            }),
            reachable: false,
            reachability_reason: String::new(),
            antigravity_alias: Some("OpenRouter Auto".into()),
        },
    ]
}

pub fn provider_key_name(p: ProviderKind) -> &'static str {
    match p {
        ProviderKind::LocalLlama => "local_llama",
        ProviderKind::LocalOllama => "local_ollama",
        ProviderKind::GoogleGemini => "google_gemini",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenRouter => "open_router",
        ProviderKind::OpenAiCompat => "open_ai_compat",
    }
}

pub fn has_provider_key(keys: &ApiKeys, p: ProviderKind) -> bool {
    match p {
        ProviderKind::LocalLlama | ProviderKind::LocalOllama => true,
        ProviderKind::GoogleGemini => !keys.gemini_api_key.trim().is_empty(),
        ProviderKind::Anthropic => !keys.anthropic_api_key.trim().is_empty(),
        ProviderKind::OpenRouter => !keys.openrouter_api_key.trim().is_empty(),
        ProviderKind::OpenAiCompat => !keys.openai_compat_base_url.trim().is_empty(),
    }
}

pub fn missing_key_reason(p: ProviderKind) -> String {
    match p {
        ProviderKind::GoogleGemini => "Configure Gemini API Key in Settings to unlock".into(),
        ProviderKind::Anthropic => "Configure Anthropic API Key in Settings to unlock".into(),
        ProviderKind::OpenRouter => "Configure OpenRouter API Key in Settings to unlock".into(),
        ProviderKind::OpenAiCompat => "Configure Custom OpenAI Base URL in Settings to unlock".into(),
        ProviderKind::LocalLlama | ProviderKind::LocalOllama => "Local GGUF on disk".into(),
    }
}

pub fn compute_catalog_and_quotas(
    state: &PersistedState,
) -> (Vec<ModelCatalogEntry>, Vec<ModelQuotaSnapshot>) {
    let now = Utc::now();
    let today = now.date_naive();

    let mut catalog = default_google_pro_catalog(state.settings.quota_tier);

    // Append legacy synced google models if viable and not present
    for synced in &state.synced_google_models {
        if synced.provider == ProviderKind::GoogleGemini && !is_viable_gemini_chat_model(&synced.id) {
            continue;
        }
        if !catalog.iter().any(|c| c.id == synced.id) {
            catalog.push(synced.clone());
        }
    }

    // Merge user-registered custom models
    for cm in &state.custom_models {
        if let Some(existing) = catalog.iter_mut().find(|c| c.id == cm.id) {
            *existing = cm.clone();
        } else {
            catalog.push(cm.clone());
        }
    }

    // Merge verified models discovered from live provider APIs
    for (_prov_key, verified_list) in &state.verified_provider_models {
        for vm in verified_list {
            if vm.provider == ProviderKind::GoogleGemini && !is_viable_gemini_chat_model(&vm.id) {
                continue;
            }
            if let Some(existing) = catalog.iter_mut().find(|c| c.id == vm.id) {
                existing.display_name = vm.display_name.clone();
                existing.context_window = vm.context_window;
                existing.supports_thinking = vm.supports_thinking;
                existing.supports_tools = vm.supports_tools;
                existing.supports_vision = vm.supports_vision;
                if existing.antigravity_alias.is_none() {
                    existing.antigravity_alias = vm.antigravity_alias.clone();
                }
            } else {
                catalog.push(vm.clone());
            }
        }
    }

    // Compute exact reachability on every catalog entry
    for item in &mut catalog {
        if let Some(block_reason) = state.blocked_models.get(&item.id) {
            item.reachable = false;
            item.reachability_reason = block_reason.clone();
            continue;
        }
        let has_key = has_provider_key(&state.settings.api_keys, item.provider);
        let p_key = provider_key_name(item.provider);
        if !has_key {
            item.reachable = false;
            item.reachability_reason = missing_key_reason(item.provider);
        } else if let Some(verified_list) = state.verified_provider_models.get(p_key) {
            if verified_list.iter().any(|v| v.id == item.id) {
                item.reachable = true;
                item.reachability_reason = "Verified reachable via live API".into();
            } else {
                item.reachable = false;
                item.reachability_reason =
                    "Not returned by your API key during live verification".into();
            }
        } else {
            item.reachable = true;
            item.reachability_reason =
                "API key configured (click Verify Reachable Models to probe live)".into();
        }
    }

    let tomorrow = (today + ChronoDuration::days(1))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let reset_in_seconds = (tomorrow - now).num_seconds().max(0);

    let mut snapshots = Vec::new();
    for item in &catalog {
        let limit = state
            .custom_quotas
            .get(&item.id)
            .cloned()
            .or_else(|| item.quota_limit.clone())
            .unwrap_or(ModelQuotaLimit {
                rpm_limit: 60,
                tpm_limit: 500_000,
                rpd_limit: 1_500,
                daily_token_limit: 10_000_000,
            });

        let mut usage_copy = state
            .usage
            .get(&item.id)
            .cloned()
            .unwrap_or_else(|| ModelUsageState::new(&item.id, today));
        usage_copy.roll_day_if_needed(today, now);

        let rpm_used = usage_copy.recent_events.len() as u64;
        let tpm_used: u64 = usage_copy.recent_events.iter().map(|e| e.total_tokens).sum();

        let rpd_pct = if limit.rpd_limit > 0 {
            100.0 * (1.0 - (usage_copy.rpd_used as f64 / limit.rpd_limit as f64))
        } else {
            100.0
        };
        let tok_pct = if limit.daily_token_limit > 0 {
            100.0 * (1.0 - (usage_copy.tokens_today as f64 / limit.daily_token_limit as f64))
        } else {
            100.0
        };

        let mut header_source = false;
        let mut tightest = rpd_pct.min(tok_pct);

        if let (Some(rem), Some(lim)) = (
            usage_copy.provider_remaining_requests,
            usage_copy.provider_limit_requests,
        ) {
            if lim > 0 {
                header_source = true;
                tightest = tightest.min(100.0 * (rem as f64 / lim as f64));
            }
        }
        if let (Some(rem), Some(lim)) = (
            usage_copy.provider_remaining_tokens,
            usage_copy.provider_limit_tokens,
        ) {
            if lim > 0 {
                header_source = true;
                tightest = tightest.min(100.0 * (rem as f64 / lim as f64));
            }
        }

        let remaining_daily_pct = if !item.reachable && state.blocked_models.contains_key(&item.id) {
            0.0
        } else {
            tightest.clamp(0.0, 100.0)
        };

        snapshots.push(ModelQuotaSnapshot {
            model_id: item.id.clone(),
            display_name: item.display_name.clone(),
            provider: item.provider,
            context_window: item.context_window,
            supports_thinking: item.supports_thinking,
            supports_tools: item.supports_tools,
            remaining_daily_pct,
            rpm_used,
            rpm_limit: limit.rpm_limit,
            tpm_used,
            tpm_limit: limit.tpm_limit,
            rpd_used: usage_copy.rpd_used,
            rpd_limit: limit.rpd_limit,
            tokens_today: usage_copy.tokens_today,
            daily_token_limit: limit.daily_token_limit,
            prompt_tokens_today: usage_copy.prompt_tokens_today,
            completion_tokens_today: usage_copy.completion_tokens_today,
            thinking_tokens_today: usage_copy.thinking_tokens_today,
            reset_in_seconds,
            header_source,
            reachable: item.reachable,
            reachability_reason: item.reachability_reason.clone(),
            antigravity_alias: item.antigravity_alias.clone(),
        });
    }

    (catalog, snapshots)
}
