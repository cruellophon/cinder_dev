pub mod drives;
pub mod types;
pub mod usage;

pub use drives::{probe_storage_drives, BumboPaths, DriveInfo};
pub use types::*;
pub use usage::{
    compute_catalog_and_quotas, default_google_pro_catalog, gemini_model_priority_rank,
    has_provider_key, is_viable_gemini_chat_model, provider_key_name,
    sort_gemini_models_by_priority,
};

use chrono::Utc;
use harness_net::PrivacyMode;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use uuid::Uuid;

fn default_workspaces() -> Vec<WorkspaceEntry> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Current Workspace")
        .to_string();
    vec![WorkspaceEntry {
        id: Uuid::new_v4().to_string(),
        name,
        path: cwd.to_string_lossy().to_string(),
        is_uni_project: false,
    }]
}

pub struct BumboStore {
    paths: BumboPaths,
    state: RwLock<PersistedState>,
    live_activity: RwLock<LiveActivityStatus>,
    abort_flag: AtomicBool,
}

impl BumboStore {
    pub fn init(paths: BumboPaths) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&paths.app_data_dir)?;
        std::fs::create_dir_all(&paths.engines_dir)?;

        let mut state = if paths.state_file.exists() {
            let raw = std::fs::read_to_string(&paths.state_file)?;
            serde_json::from_str::<PersistedState>(&raw)
                .unwrap_or_else(|_| Self::default_state(&paths))
        } else {
            Self::default_state(&paths)
        };

        Self::sanitize_loaded_state(&mut state);
        let _ = std::fs::create_dir_all(&state.settings.model_vault_dir);
        Self::write_atomic_to(&paths.state_file, &state)?;

        Ok(Self {
            paths,
            state: RwLock::new(state),
            live_activity: RwLock::new(LiveActivityStatus::default()),
            abort_flag: AtomicBool::new(false),
        })
    }

    pub fn request_abort(&self) {
        self.abort_flag.store(true, Ordering::SeqCst);
    }

    pub fn clear_abort(&self) {
        self.abort_flag.store(false, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.abort_flag.load(Ordering::SeqCst)
    }

    fn default_state(paths: &BumboPaths) -> PersistedState {
        let workspaces = default_workspaces();
        let active_workspace_path = workspaces
            .first()
            .map(|w| w.path.clone())
            .unwrap_or_else(|| ".".to_string());

        let now = Utc::now();
        let initial_session = Session {
            id: Uuid::new_v4().to_string(),
            title: "Workbench Session".into(),
            workspace_path: active_workspace_path.clone(),
            mode: AgentMode::Auto,
            messages: Vec::new(),
            pending_approval: None,
            active_plan: None,
            pending_question: None,
            pending_fallback: None,
            checkpoints: Vec::new(),
            artifacts: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        let active_session_id = initial_session.id.clone();

        PersistedState {
            settings: AppSettings {
                privacy_mode: PrivacyMode::StrictLocal,
                model_vault_dir: paths.default_vault_dir.to_string_lossy().to_string(),
                quota_tier: QuotaTier::GoogleAiPro,
                active_model_id: "local::qwen3.5:9b".into(),
                compare_model_b_id: "local::gemma4:12b".into(),
                active_workspace_path,
                active_session_id,
                auto_approve_edits: false,
                auto_approve_commands: false,
                idle_unload_minutes: 15,
                local_ctx_size: 16384,
                local_gpu_layers: 99,
                expose_local_api: false,
                local_server_port: 18080,
                api_keys: ApiKeys::default(),
                ui: UiCustomization::default(),
                orchestrator: OrchestratorConfig::default(),
                auto_switch_models: true,
                fallback_to_local: false,
                fallback_policy: FallbackPolicy::Ask,
                fallback_chain: Vec::new(),
                model_aliases: std::collections::BTreeMap::new(),
            },
            workspaces,
            sessions: vec![initial_session],
            usage: HashMap::new(),
            custom_quotas: HashMap::new(),
            synced_google_models: Vec::new(),
            verified_provider_models: HashMap::new(),
            blocked_models: HashMap::new(),
            mcp_servers: Vec::new(),
            custom_models: Vec::new(),
        }
    }

    fn sanitize_loaded_state(state: &mut PersistedState) {
        if state.settings.ui.theme == "antigravity_dark" && state.settings.ui.accent_color == "blue"
        {
            state.settings.ui.theme = "singularity_obsidian".into();
            state.settings.ui.accent_color = "amber".into();
            state.settings.ui.show_right_workbench = false;
        }
        if state.settings.ui.thinking_effort.trim().is_empty() {
            state.settings.ui.thinking_effort = "standard".into();
        }
        if state.synced_google_models.len() > 12 {
            // Legacy unprobed `/v1beta/models` catalog dump (e.g. contains `gemini-4-neon` with limit: 0).
            state.synced_google_models.clear();
            state.verified_provider_models.remove("google_gemini");
        }

        state.synced_google_models.retain(|m| {
            usage::is_viable_gemini_chat_model(&m.id)
                && !m.id.contains("nonexistent-")
                && !state.blocked_models.contains_key(&m.id)
        });
        usage::sort_gemini_models_by_priority(&mut state.synced_google_models);

        state.custom_models.retain(|m| {
            !m.id.contains("nonexistent-")
                && has_provider_key(&state.settings.api_keys, m.provider)
        });

        let api_keys = state.settings.api_keys.clone();
        state.verified_provider_models.retain(|prov_key, list| {
            let key_configured = match prov_key.as_str() {
                "google_gemini" => has_provider_key(&api_keys, ProviderKind::GoogleGemini),
                "anthropic" => has_provider_key(&api_keys, ProviderKind::Anthropic),
                "open_router" => has_provider_key(&api_keys, ProviderKind::OpenRouter),
                "open_ai_compat" => has_provider_key(&api_keys, ProviderKind::OpenAiCompat),
                _ => true,
            };
            if !key_configured {
                return false;
            }
            list.retain(|m| {
                !m.id.contains("nonexistent-")
                    && has_provider_key(&api_keys, m.provider)
                    && (m.provider != ProviderKind::GoogleGemini
                        || (usage::is_viable_gemini_chat_model(&m.id)
                            && !state.blocked_models.contains_key(&m.id)))
            });
            if prov_key == "google_gemini" {
                usage::sort_gemini_models_by_priority(list);
            }
            !list.is_empty()
        });

        state.workspaces.retain(|w| {
            !w.name.starts_with("bumbo_ws_verify_") && Path::new(&w.path).exists()
        });
        if state.workspaces.is_empty() {
            state.workspaces = default_workspaces();
        }
        if !state
            .workspaces
            .iter()
            .any(|w| w.path.eq_ignore_ascii_case(&state.settings.active_workspace_path))
        {
            state.settings.active_workspace_path = state
                .workspaces
                .first()
                .map(|w| w.path.clone())
                .unwrap_or_else(|| ".".into());
        }

        let (catalog, _) = compute_catalog_and_quotas(state);
        let active_id = state.settings.active_model_id.clone();
        let active_valid = active_id.starts_with("local::")
            || catalog.iter().any(|c| c.id == active_id && c.reachable);
        if !active_valid {
            state.settings.active_model_id = if !state.settings.api_keys.gemini_api_key.trim().is_empty() {
                catalog
                    .iter()
                    .find(|c| c.reachable && c.provider == ProviderKind::GoogleGemini)
                    .map(|c| c.id.clone())
                    .unwrap_or_else(|| "gemini-3-flash-preview".into())
            } else {
                "local::qwen3.5:9b".into()
            };
        }
    }

    pub fn paths(&self) -> &BumboPaths {
        &self.paths
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let guard = self.state.read().await;
        self.write_atomic(&*guard)
    }

    fn write_atomic(&self, state: &PersistedState) -> anyhow::Result<()> {
        Self::write_atomic_to(&self.paths.state_file, state)
    }

    fn write_atomic_to(state_file: &Path, state: &PersistedState) -> anyhow::Result<()> {
        if let Some(parent) = state_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = state_file.with_extension(format!("json.{}.tmp", Uuid::new_v4().simple()));
        let payload = serde_json::to_string_pretty(state)?;
        std::fs::write(&tmp, payload)?;
        let mut last_err = None;
        for attempt in 0..4 {
            match std::fs::rename(&tmp, state_file) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 3 {
                        std::thread::sleep(std::time::Duration::from_millis(15));
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&tmp);
        Err(last_err
            .map(anyhow::Error::from)
            .unwrap_or_else(|| anyhow::anyhow!("Failed to persist state file")))
    }

    pub async fn snapshot(&self) -> PersistedState {
        self.state.read().await.clone()
    }

    pub async fn update<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut PersistedState) -> R,
    {
        let result = {
            let mut guard = self.state.write().await;
            let res = f(&mut *guard);
            self.write_atomic(&*guard)?;
            res
        };
        Ok(result)
    }

    pub async fn set_activity(
        &self,
        session_id: Option<&str>,
        phase: &str,
        detail: &str,
        step: usize,
        max_steps: usize,
    ) {
        let mut guard = self.live_activity.write().await;
        guard.active = true;
        guard.session_id = session_id.map(|s| s.to_string());
        guard.phase = phase.to_string();
        guard.detail = detail.to_string();
        guard.step = step;
        guard.max_steps = max_steps;
        guard.updated_at = Utc::now();
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn set_activity_stream(
        &self,
        session_id: Option<&str>,
        phase: &str,
        detail: &str,
        step: usize,
        max_steps: usize,
        active_model: &str,
        partial_thinking: &str,
        partial_content: &str,
        completed_tools: &[ToolCallRecord],
    ) {
        let mut guard = self.live_activity.write().await;
        *guard = LiveActivityStatus {
            active: true,
            session_id: session_id.map(|s| s.to_string()),
            phase: phase.to_string(),
            detail: detail.to_string(),
            step,
            max_steps,
            partial_content: partial_content.to_string(),
            partial_thinking: partial_thinking.to_string(),
            base_thinking: partial_thinking.to_string(),
            active_model: active_model.to_string(),
            completed_tools: completed_tools.to_vec(),
            live_tool_name: String::new(),
            live_tool_output: String::new(),
            updated_at: Utc::now(),
        };
    }

    pub async fn set_live_tool_output(&self, tool_label: &str, output_tail: &str) {
        let mut guard = self.live_activity.write().await;
        if guard.active {
            guard.live_tool_name = tool_label.to_string();
            guard.live_tool_output = output_tail.to_string();
            guard.updated_at = Utc::now();
        }
    }

    pub async fn clear_live_tool_output(&self) {
        let mut guard = self.live_activity.write().await;
        guard.live_tool_name.clear();
        guard.live_tool_output.clear();
        guard.updated_at = Utc::now();
    }

    pub async fn update_activity_deltas(&self, partial_thinking: &str, partial_content: &str) {
        let mut guard = self.live_activity.write().await;
        if guard.active {
            guard.partial_thinking = match (
                guard.base_thinking.trim().is_empty(),
                partial_thinking.trim().is_empty(),
            ) {
                (true, _) => partial_thinking.to_string(),
                (false, true) => guard.base_thinking.clone(),
                (false, false) => {
                    format!("{}\n\n{}", guard.base_thinking.trim(), partial_thinking.trim())
                }
            };
            guard.partial_content = partial_content.to_string();
            guard.updated_at = Utc::now();
        }
    }

    pub async fn clear_activity(&self) {
        let mut guard = self.live_activity.write().await;
        *guard = LiveActivityStatus::default();
    }

    pub async fn get_activity(&self) -> LiveActivityStatus {
        self.live_activity.read().await.clone()
    }

    pub async fn record_usage(
        &self,
        model_id: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        thinking_tokens: u64,
        provider_remaining_requests: Option<u64>,
        provider_limit_requests: Option<u64>,
        provider_remaining_tokens: Option<u64>,
        provider_limit_tokens: Option<u64>,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        let today = now.date_naive();
        let total = prompt_tokens + completion_tokens + thinking_tokens;

        self.update(|s| {
            let entry = s
                .usage
                .entry(model_id.to_string())
                .or_insert_with(|| ModelUsageState::new(model_id, today));
            entry.roll_day_if_needed(today, now);
            entry.rpd_used += 1;
            entry.tokens_today += total;
            entry.prompt_tokens_today += prompt_tokens;
            entry.completion_tokens_today += completion_tokens;
            entry.thinking_tokens_today += thinking_tokens;
            entry.recent_events.push_back(TokenEvent {
                timestamp: now,
                total_tokens: total,
            });
            if provider_remaining_requests.is_some() {
                entry.provider_remaining_requests = provider_remaining_requests;
            }
            if provider_limit_requests.is_some() {
                entry.provider_limit_requests = provider_limit_requests;
            }
            if provider_remaining_tokens.is_some() {
                entry.provider_remaining_tokens = provider_remaining_tokens;
            }
            if provider_limit_tokens.is_some() {
                entry.provider_limit_tokens = provider_limit_tokens;
            }
            entry.last_used_at = Some(now);
        })
        .await
    }

    pub async fn mark_model_blocked(&self, model_id: &str, reason: &str) -> anyhow::Result<()> {
        self.update(|s| {
            s.blocked_models
                .insert(model_id.to_string(), reason.to_string());
            s.synced_google_models.retain(|m| m.id != model_id);
            for list in s.verified_provider_models.values_mut() {
                list.retain(|m| m.id != model_id);
            }
        })
        .await
    }

    pub async fn build_cloud_catalog_and_quotas(
        &self,
    ) -> (Vec<ModelCatalogEntry>, Vec<ModelQuotaSnapshot>) {
        let state = self.state.read().await;
        compute_catalog_and_quotas(&state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_pct_updates_after_usage() {
        let dir = std::env::temp_dir().join(format!("bumbo_store_test_{}", Uuid::new_v4()));
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            store
                .record_usage(
                    "gemini-3-flash-preview",
                    1_250_000,
                    1_250_000,
                    0,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
            let (_, snaps) = store.build_cloud_catalog_and_quotas().await;
            let pro = snaps
                .iter()
                .find(|s| s.model_id == "gemini-3-flash-preview")
                .unwrap();
            assert_eq!(pro.tokens_today, 2_500_000);
            assert!((pro.remaining_daily_pct - 90.0).abs() < 0.01);
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reachability_respects_configured_api_keys() {
        let dir = std::env::temp_dir().join(format!("bumbo_reach_test_{}", Uuid::new_v4()));
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (cat_empty, _) = store.build_cloud_catalog_and_quotas().await;
            assert!(cat_empty.iter().all(|m| !m.reachable));

            store
                .update(|s| {
                    s.settings.api_keys.gemini_api_key = "test-gemini-key".into();
                })
                .await
                .unwrap();

            let (cat_with_gem, _) = store.build_cloud_catalog_and_quotas().await;
            let gem = cat_with_gem
                .iter()
                .find(|m| m.id == "gemini-3-flash-preview")
                .unwrap();
            let claude = cat_with_gem
                .iter()
                .find(|m| m.id == "claude-sonnet-4-5")
                .unwrap();
            assert!(gem.reachable);
            assert!(!claude.reachable);

            store
                .mark_model_blocked("gemini-3-flash-preview", "0 quota limit")
                .await
                .unwrap();
            let (cat_blocked, snaps_blocked) = store.build_cloud_catalog_and_quotas().await;
            let gem_blocked = cat_blocked
                .iter()
                .find(|m| m.id == "gemini-3-flash-preview")
                .unwrap();
            assert!(!gem_blocked.reachable);
            let snap_blocked = snaps_blocked
                .iter()
                .find(|s| s.model_id == "gemini-3-flash-preview")
                .unwrap();
            assert_eq!(snap_blocked.remaining_daily_pct, 0.0);
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_filters_unstable_models_and_resets_active() {
        assert!(!is_viable_gemini_chat_model("gemini-3.7-flash"));
        assert!(!is_viable_gemini_chat_model("gemini-3.8-flash"));
        assert!(!is_viable_gemini_chat_model("gemini-3.5-flash"));
        assert!(!is_viable_gemini_chat_model("gemma-4-31b-it"));
        assert!(is_viable_gemini_chat_model("gemini-3-flash-preview"));
        assert!(is_viable_gemini_chat_model("gemini-3.5-flash-lite"));
        assert!(is_viable_gemini_chat_model("gemini-3.1-flash-lite"));
        assert!(is_viable_gemini_chat_model("gemini-3.6-flash"));

        let dir = std::env::temp_dir().join(format!("bumbo_sanitize_test_{}", Uuid::new_v4()));
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = BumboStore::init(paths.clone()).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mk = |id: &str, prov: ProviderKind| ModelCatalogEntry {
                id: id.into(),
                display_name: id.into(),
                provider: prov,
                context_window: 128_000,
                supports_thinking: false,
                supports_tools: true,
                supports_vision: true,
                tier_note: String::new(),
                quota_limit: None,
                reachable: true,
                reachability_reason: String::new(),
                antigravity_alias: None,
                requires_payment: false,
            };
            store
                .update(|s| {
                    s.settings.api_keys.gemini_api_key = "test-gem".into();
                    s.settings.active_model_id = "gemini-3.7-flash".into();
                    s.synced_google_models = vec![
                        mk("gemini-3.7-flash", ProviderKind::GoogleGemini),
                        mk("gemini-3.5-flash-lite", ProviderKind::GoogleGemini),
                        mk("gemini-3-flash-preview", ProviderKind::GoogleGemini),
                    ];
                    s.verified_provider_models.insert(
                        "google_gemini".into(),
                        s.synced_google_models.clone(),
                    );
                    s.verified_provider_models.insert(
                        "open_ai_compat".into(),
                        vec![mk("nonexistent-model-xyz", ProviderKind::OpenAiCompat)],
                    );
                })
                .await
                .unwrap();
        });
        drop(store);

        let reloaded = BumboStore::init(paths).unwrap();
        rt.block_on(async {
            let snap = reloaded.snapshot().await;
            assert_eq!(snap.settings.active_model_id, "gemini-3-flash-preview");
            assert!(!snap
                .verified_provider_models
                .contains_key("open_ai_compat"));
            let gem_list = &snap.verified_provider_models["google_gemini"];
            assert_eq!(gem_list[0].id, "gemini-3-flash-preview");
            assert!(!gem_list.iter().any(|m| m.id == "gemini-3.7-flash"));
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
