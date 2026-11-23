use chrono::Utc;
use harness_core::{
    accept_checkpoint_file, answer_pending_question, compact_session_context,
    delete_session_artifact, handle_plan_review_action, list_skills_with_global,
    load_workspace_rules, resolve_pending_approval, revert_checkpoint_file,
    rewind_session_to_message, rollback_entire_checkpoint, run_turn, save_custom_skill,
    save_workspace_rules, upsert_session_artifact, HarnessContext,
};
use harness_models::{
    inspect_hf_repo, probe_hardware, scan_local_models, search_hf_gguf, DownloadJob,
    DownloadManager, EngineStatus, HardwareReport, HfModelSummary, HfRepoDetails, LocalModelEntry,
    ServerManager, ServerStatus,
};
use harness_net::{AuditSummary, NetworkGate, PrivacyMode};
use harness_providers::{
    probe_gemini_model_live, sync_google_models, verify_and_discover_provider_models,
    ProviderVerifyStatus,
};
use harness_store::{
    compute_catalog_and_quotas, provider_key_name, AgentMode, AppSettings, BumboPaths, BumboStore,
    ChatMessage, McpServerEntry, MediaAttachment, ModelCatalogEntry, ModelQuotaLimit,
    ModelQuotaSnapshot, OrchestratorConfig, ProviderKind, QuotaTier, Session, SkillDefinition,
    UiCustomization, WorkspaceEntry,
};
use harness_tools::{
    build_workspace_index, create_workspace_dir, create_workspace_file, delete_workspace_entry,
    evict_mcp_server_session, export_antigravity_mcp_json, inspect_project_dependencies,
    install_curated_plugin, install_custom_project_dependency, install_project_dependency,
    is_dependency_allowlisted, list_plugin_catalog_with_status, list_workspace_tree, pick_files,
    pick_folder, probe_mcp_server, probe_toolchains, rename_workspace_entry,
    search_workspace_knowledge, sync_from_antigravity, uninstall_curated_plugin,
    uninstall_project_dependency, validate_custom_package_name, AntigravitySyncReport, IndexStats,
    InstallJobManager, InstallJobRecord, KnowledgeSearchHit, McpProbeResult, PluginCatalogView,
    ProjectDependenciesOverview, ToolchainStatus, WorkspaceFileNode,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use tokio::runtime::Runtime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderPickTarget {
    Workspace,
    Vault,
}

#[derive(Debug, Clone)]
pub struct OpenedFilePayload {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub kind: String, // "text" | "image" | "pdf" | "binary"
    pub content: String,
    pub bytes: Vec<u8>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub workspace_path: String,
    pub mode: AgentMode,
    pub message_count: usize,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UiSnapshot {
    pub settings: AppSettings,
    pub workspaces: Vec<WorkspaceEntry>,
    pub sessions: Vec<SessionSummary>,
    pub active_session: Option<Session>,
    pub workspace_tree: Vec<WorkspaceFileNode>,
    pub workspace_rules: String,
    pub skills: Vec<SkillDefinition>,
    pub hardware: HardwareReport,
    pub local_models: Vec<LocalModelEntry>,
    pub cloud_catalog: Vec<ModelCatalogEntry>,
    pub cloud_quotas: Vec<ModelQuotaSnapshot>,
    pub custom_models: Vec<ModelCatalogEntry>,
    pub mcp_servers: Vec<McpServerEntry>,
    pub engine_status: EngineStatus,
    pub server_status: ServerStatus,
    pub downloads: Vec<DownloadJob>,
    pub privacy_audit: AuditSummary,
    pub live_activity: harness_store::LiveActivityStatus,
    pub toolchains: Vec<ToolchainStatus>,
    pub plugins_catalog: Vec<PluginCatalogView>,
    pub project_deps: ProjectDependenciesOverview,
    pub install_jobs: Vec<InstallJobRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct KeysPatch {
    pub gemini_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub openai_compat_base_url: Option<String>,
    pub openai_compat_api_key: Option<String>,
    pub huggingface_token: Option<String>,
    pub brave_search_api_key: Option<String>,
    pub clear_keys: Vec<String>,
}

pub enum BackendEvent {
    FullSnapshot(Box<UiSnapshot>),
    LiveTick {
        live_activity: harness_store::LiveActivityStatus,
        downloads: Vec<DownloadJob>,
        install_jobs: Vec<InstallJobRecord>,
        server_status: ServerStatus,
        privacy_audit: AuditSummary,
        was_downloading_or_installing_finished: bool,
    },
    TurnFinished(Result<Session, String>),
    ChatRewound {
        session: Session,
        prompt_text: String,
        attached_files: Vec<String>,
        attachments: Vec<MediaAttachment>,
    },
    Toast {
        msg: String,
        is_error: bool,
    },
    FolderPicked {
        target: FolderPickTarget,
        path: String,
    },
    FilesPicked(Vec<String>),
    FileLoaded(OpenedFilePayload),
    HfSearchResults(Vec<HfModelSummary>),
    HfRepoInspected(HfRepoDetails),
    ProviderVerifyResults(Vec<ProviderVerifyStatus>),
    KnowledgeIndexed(IndexStats),
    KnowledgeHits(Vec<KnowledgeSearchHit>),
    TerminalOutput {
        cmd: String,
        exit_code: i32,
        output: String,
        duration_ms: u64,
    },
    McpProbed(McpProbeResult),
    AntigravityExported(String),
    EntryRenamed {
        old_rel: String,
        new_rel: String,
    },
    EntryDeleted {
        rel_path: String,
    },
}

pub struct BackendHandle {
    rt: Runtime,
    gate: Arc<NetworkGate>,
    store: Arc<BumboStore>,
    server_mgr: Arc<ServerManager>,
    download_mgr: Arc<DownloadManager>,
    install_jobs: InstallJobManager,
    cached_hardware: tokio::sync::RwLock<Option<HardwareReport>>,
    cached_toolchains: tokio::sync::RwLock<Option<Vec<ToolchainStatus>>>,
    tx: Sender<BackendEvent>,
    ctx: egui::Context,
}

impl BackendHandle {
    pub fn start(ctx: egui::Context) -> anyhow::Result<(Arc<Self>, Receiver<BackendEvent>)> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let paths = BumboPaths::discover();
        let store = Arc::new(BumboStore::init(paths.clone())?);
        let initial_privacy = rt.block_on(async { store.snapshot().await.settings.privacy_mode });
        let gate = Arc::new(NetworkGate::new(initial_privacy)?);
        let server_mgr = Arc::new(ServerManager::new(paths.engines_dir.clone()));
        let download_mgr = Arc::new(DownloadManager::new());
        let install_jobs = InstallJobManager::new();

        let (tx, rx) = mpsc::channel();
        let handle = Arc::new(Self {
            rt,
            gate,
            store,
            server_mgr,
            download_mgr,
            install_jobs,
            cached_hardware: tokio::sync::RwLock::new(None),
            cached_toolchains: tokio::sync::RwLock::new(None),
            tx,
            ctx,
        });

        // Background 1: idle VRAM auto-quench loop
        {
            let h = Arc::clone(&handle);
            handle.rt.spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    let mins = h.store.snapshot().await.settings.idle_unload_minutes;
                    if h.server_mgr.check_idle_unload(&h.gate, mins).await {
                        tracing::info!(
                            "Auto-unloaded idle local model after {mins}m to free VRAM"
                        );
                        h.emit_snapshot(false).await;
                    }
                }
            });
        }

        // Background 2: lightweight live activity/download/install poll loop
        {
            let h = Arc::clone(&handle);
            handle.rt.spawn(async move {
                let mut had_bg_work = false;
                loop {
                    let sleep_ms = if had_bg_work { 50 } else { 320 };
                    tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                    let act = h.store.get_activity().await;
                    let downloads = h.download_mgr.list_jobs().await;
                    let jobs = h.install_jobs.list().await;

                    let bg_active = act.active
                        || downloads.iter().any(|d| {
                            matches!(
                                d.status,
                                harness_models::DownloadStatus::Downloading
                                    | harness_models::DownloadStatus::Verifying
                                    | harness_models::DownloadStatus::Queued
                            )
                        })
                        || jobs.iter().any(|j| j.status == "running");

                    if bg_active || had_bg_work {
                        let srv = h.server_mgr.server_status().await;
                        let finished_now = had_bg_work && !bg_active;
                        had_bg_work = bg_active;
                        h.send(BackendEvent::LiveTick {
                            live_activity: act,
                            downloads,
                            install_jobs: jobs,
                            server_status: srv,
                            privacy_audit: h.gate.audit_summary(),
                            was_downloading_or_installing_finished: finished_now,
                        });
                        if finished_now {
                            h.emit_snapshot(false).await;
                        }
                    }
                }
            });
        }

        // Emit initial full snapshot immediately
        handle.refresh_state(false);
        Ok((handle, rx))
    }

    fn send(&self, ev: BackendEvent) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }

    fn toast(&self, msg: impl Into<String>, is_error: bool) {
        self.send(BackendEvent::Toast {
            msg: msg.into(),
            is_error,
        });
    }

    pub fn shutdown_local_models(&self) {
        let srv = Arc::clone(&self.server_mgr);
        let gate = Arc::clone(&self.gate);
        let _ = self.rt.block_on(async move {
            let _ = srv.stop_all_local_models(&gate).await;
        });
    }

    async fn hardware_snapshot(&self, force_refresh: bool) -> HardwareReport {
        if !force_refresh {
            if let Some(hw) = self.cached_hardware.read().await.clone() {
                return hw;
            }
        }
        let hw = tokio::task::spawn_blocking(probe_hardware)
            .await
            .unwrap_or_else(|_| probe_hardware());
        *self.cached_hardware.write().await = Some(hw.clone());
        hw
    }

    async fn toolchains_snapshot(&self, force_refresh: bool) -> Vec<ToolchainStatus> {
        if !force_refresh {
            if let Some(tc) = self.cached_toolchains.read().await.clone() {
                return tc;
            }
        }
        let paths_tc = self.store.paths().clone();
        let tc = tokio::task::spawn_blocking(move || probe_toolchains(&paths_tc))
            .await
            .unwrap_or_default();
        *self.cached_toolchains.write().await = Some(tc.clone());
        tc
    }

    async fn build_snapshot(&self, probe_live_server: bool) -> UiSnapshot {
        let snap = self.store.snapshot().await;
        let hw = self.hardware_snapshot(probe_live_server).await;

        let vault = snap.settings.model_vault_dir.clone();
        let vram = hw.primary_vram_mb;
        let ctx_sz = snap.settings.local_ctx_size;
        let local_models = tokio::task::spawn_blocking(move || {
            scan_local_models(Path::new(&vault), vram, ctx_sz)
        })
        .await
        .unwrap_or_default();

        let (cloud_catalog, cloud_quotas) = self.store.build_cloud_catalog_and_quotas().await;
        let engine_status = self.server_mgr.engine_status(&self.gate).await;
        let server_status = if probe_live_server {
            self.server_mgr.server_status_with_probe(&self.gate).await
        } else {
            self.server_mgr.server_status().await
        };
        let downloads = self.download_mgr.list_jobs().await;
        let privacy_audit = self.gate.audit_summary();
        let live_activity = self.store.get_activity().await;

        let ws_str = snap.settings.active_workspace_path.clone();
        let ws_tree_str = ws_str.clone();
        let workspace_tree = tokio::task::spawn_blocking(move || {
            let p = Path::new(&ws_tree_str);
            if p.exists() {
                list_workspace_tree(p, 600)
            } else {
                Vec::new()
            }
        })
        .await
        .unwrap_or_default();

        let ws_root = Path::new(&ws_str);
        let global_skills = self.store.paths().skills_dir();
        let workspace_rules = load_workspace_rules(ws_root);
        let skills = list_skills_with_global(ws_root, Some(&global_skills));

        let toolchains = self.toolchains_snapshot(probe_live_server).await;
        let plugins_catalog =
            list_plugin_catalog_with_status(self.store.paths(), &snap.mcp_servers);
        let ws_deps = ws_str.clone();
        let project_deps =
            tokio::task::spawn_blocking(move || inspect_project_dependencies(Path::new(&ws_deps)))
                .await
                .unwrap_or_else(|_| inspect_project_dependencies(Path::new(&ws_str)));
        let install_jobs = self.install_jobs.list().await;

        let active_session = snap
            .sessions
            .iter()
            .find(|s| s.id == snap.settings.active_session_id)
            .cloned()
            .or_else(|| snap.sessions.first().cloned());

        let sessions = snap
            .sessions
            .iter()
            .map(|s| SessionSummary {
                id: s.id.clone(),
                title: s.title.clone(),
                workspace_path: s.workspace_path.clone(),
                mode: s.mode,
                message_count: s.messages.len(),
                updated_at: s.updated_at,
            })
            .collect();

        let mut settings = snap.settings;
        settings.privacy_mode = privacy_audit.privacy_mode;

        UiSnapshot {
            settings,
            workspaces: snap.workspaces,
            sessions,
            active_session,
            workspace_tree,
            workspace_rules,
            skills,
            hardware: hw,
            local_models,
            cloud_catalog,
            cloud_quotas,
            custom_models: snap.custom_models,
            mcp_servers: snap.mcp_servers,
            engine_status,
            server_status,
            downloads,
            privacy_audit,
            live_activity,
            toolchains,
            plugins_catalog,
            project_deps,
            install_jobs,
        }
    }

    async fn emit_snapshot(&self, probe_live_server: bool) {
        let snap = self.build_snapshot(probe_live_server).await;
        self.send(BackendEvent::FullSnapshot(Box::new(snap)));
    }

    pub fn refresh_state(self: &Arc<Self>, probe_live_server: bool) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            h.emit_snapshot(probe_live_server).await;
        });
    }

    // ---- Chat / Agent Actions ----

    #[allow(clippy::too_many_arguments)]
    pub fn send_prompt(
        self: &Arc<Self>,
        session_id: String,
        message: String,
        mode: AgentMode,
        model_id: String,
        compare_model_b: Option<String>,
        attached_files: Vec<String>,
        attachments: Vec<MediaAttachment>,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let snap = h.store.snapshot().await;
            let hw = h.hardware_snapshot(false).await;
            let local_models = scan_local_models(
                Path::new(&snap.settings.model_vault_dir),
                hw.primary_vram_mb,
                snap.settings.local_ctx_size,
            );
            let active_model = if model_id.is_empty() {
                snap.settings.active_model_id.clone()
            } else {
                model_id
            };
            let ctx = HarnessContext {
                gate: &h.gate,
                store: &h.store,
                server_mgr: &h.server_mgr,
                local_models: &local_models,
            };
            let res = run_turn(
                &ctx,
                &session_id,
                &message,
                mode,
                &active_model,
                compare_model_b.as_deref(),
                attached_files,
                attachments,
            )
            .await;

            if h.store.snapshot().await.settings.ui.unload_after_turn
                && active_model.starts_with("local::")
            {
                let _ = h.server_mgr.stop_all_local_models(&h.gate).await;
            }

            match res {
                Ok(sess) => {
                    h.send(BackendEvent::TurnFinished(Ok(sess)));
                    h.emit_snapshot(false).await;
                }
                Err(e) => {
                    let err_text = e.to_string();
                    h.store.clear_activity().await;
                    let err_msg = ChatMessage {
                        id: Uuid::new_v4().to_string(),
                        role: "assistant".into(),
                        content: format!(
                            "> [!WARNING]\n> **Request failed ({active_model})**\n>\n> {err_text}"
                        ),
                        thinking: None,
                        model_id: Some(active_model.clone()),
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
                    let updated_sess = h
                        .store
                        .update(|s| {
                            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                                sess.messages.push(err_msg);
                                sess.updated_at = Utc::now();
                                return Some(sess.clone());
                            }
                            None
                        })
                        .await
                        .ok()
                        .flatten();
                    h.toast(err_text.clone(), true);
                    if let Some(sess) = updated_sess {
                        h.send(BackendEvent::TurnFinished(Ok(sess)));
                    } else {
                        h.send(BackendEvent::TurnFinished(Err(err_text)));
                    }
                    h.emit_snapshot(false).await;
                }
            }
        });
    }

    pub fn abort_turn(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            h.store.request_abort();
            h.store.clear_activity().await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn approve_tool(self: &Arc<Self>, session_id: String, approve: bool) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let snap = h.store.snapshot().await;
            let hw = h.hardware_snapshot(false).await;
            let local_models = scan_local_models(
                Path::new(&snap.settings.model_vault_dir),
                hw.primary_vram_mb,
                snap.settings.local_ctx_size,
            );
            let ctx = HarnessContext {
                gate: &h.gate,
                store: &h.store,
                server_mgr: &h.server_mgr,
                local_models: &local_models,
            };
            match resolve_pending_approval(&ctx, &session_id, approve).await {
                Ok(sess) => {
                    h.send(BackendEvent::TurnFinished(Ok(sess)));
                    h.emit_snapshot(false).await;
                }
                Err(e) => {
                    let msg = e.to_string();
                    h.store.clear_activity().await;
                    h.toast(msg.clone(), true);
                    h.send(BackendEvent::TurnFinished(Err(msg)));
                    h.emit_snapshot(false).await;
                }
            }
        });
    }

    pub fn answer_question(
        self: &Arc<Self>,
        session_id: String,
        selected_options: Vec<String>,
        custom_text: String,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let snap = h.store.snapshot().await;
            let hw = h.hardware_snapshot(false).await;
            let local_models = scan_local_models(
                Path::new(&snap.settings.model_vault_dir),
                hw.primary_vram_mb,
                snap.settings.local_ctx_size,
            );
            let ctx = HarnessContext {
                gate: &h.gate,
                store: &h.store,
                server_mgr: &h.server_mgr,
                local_models: &local_models,
            };
            match answer_pending_question(&ctx, &session_id, selected_options, custom_text).await {
                Ok(sess) => {
                    h.send(BackendEvent::TurnFinished(Ok(sess)));
                    h.emit_snapshot(false).await;
                }
                Err(e) => {
                    let msg = e.to_string();
                    h.store.clear_activity().await;
                    h.toast(msg.clone(), true);
                    h.send(BackendEvent::TurnFinished(Err(msg)));
                    h.emit_snapshot(false).await;
                }
            }
        });
    }

    pub fn plan_action(
        self: &Arc<Self>,
        session_id: String,
        action: String,
        edited_markdown: Option<String>,
        feedback: Option<String>,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let snap = h.store.snapshot().await;
            let hw = h.hardware_snapshot(false).await;
            let local_models = scan_local_models(
                Path::new(&snap.settings.model_vault_dir),
                hw.primary_vram_mb,
                snap.settings.local_ctx_size,
            );
            let ctx = HarnessContext {
                gate: &h.gate,
                store: &h.store,
                server_mgr: &h.server_mgr,
                local_models: &local_models,
            };
            match handle_plan_review_action(&ctx, &session_id, &action, edited_markdown, feedback)
                .await
            {
                Ok(sess) => {
                    h.send(BackendEvent::TurnFinished(Ok(sess)));
                    h.emit_snapshot(false).await;
                }
                Err(e) => {
                    let msg = e.to_string();
                    h.store.clear_activity().await;
                    h.toast(msg.clone(), true);
                    h.send(BackendEvent::TurnFinished(Err(msg)));
                    h.emit_snapshot(false).await;
                }
            }
        });
    }

    pub fn compact_chat(self: &Arc<Self>, session_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            match compact_session_context(&h.store, &session_id).await {
                Ok(_) => {
                    h.toast("Compacted older conversation turns.", false);
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn rewind_chat(
        self: &Arc<Self>,
        session_id: String,
        message_index: usize,
        revert_files: bool,
        auto_retry: bool,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            match rewind_session_to_message(&h.store, &session_id, message_index, revert_files)
                .await
            {
                Ok((updated_sess, prompt_text, attached_files, attachments)) => {
                    if auto_retry {
                        let mode = updated_sess.mode;
                        h.send(BackendEvent::ChatRewound {
                            session: updated_sess,
                            prompt_text: String::new(),
                            attached_files: Vec::new(),
                            attachments: Vec::new(),
                        });
                        h.send_prompt(
                            session_id,
                            prompt_text,
                            mode,
                            String::new(),
                            None,
                            attached_files,
                            attachments,
                        );
                    } else {
                        h.send(BackendEvent::ChatRewound {
                            session: updated_sess,
                            prompt_text,
                            attached_files,
                            attachments,
                        });
                        h.toast(
                            if revert_files {
                                "Rewound conversation, restored files, and loaded prompt."
                            } else {
                                "Rewound conversation — prompt restored to composer."
                            },
                            false,
                        );
                        h.emit_snapshot(false).await;
                    }
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn checkpoint_file_action(
        self: &Arc<Self>,
        session_id: String,
        checkpoint_id: String,
        rel_path: String,
        revert: bool,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let res = if revert {
                revert_checkpoint_file(&h.store, &session_id, &checkpoint_id, &rel_path).await
            } else {
                accept_checkpoint_file(&h.store, &session_id, &checkpoint_id, &rel_path).await
            };
            match res {
                Ok(_) => h.emit_snapshot(false).await,
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn rollback_checkpoint(self: &Arc<Self>, session_id: String, checkpoint_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            match rollback_entire_checkpoint(&h.store, &session_id, &checkpoint_id).await {
                Ok(_) => {
                    h.toast("Rolled back checkpoint.", false);
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn upsert_artifact(
        self: &Arc<Self>,
        session_id: String,
        title: String,
        kind: String,
        content: String,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            match upsert_session_artifact(&h.store, &session_id, &title, &kind, &content).await {
                Ok(_) => h.emit_snapshot(false).await,
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn delete_artifact(self: &Arc<Self>, session_id: String, artifact_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            match delete_session_artifact(&h.store, &session_id, &artifact_id).await {
                Ok(_) => h.emit_snapshot(false).await,
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    // ---- Sessions & Workspaces ----

    pub fn create_session(
        self: &Arc<Self>,
        title: Option<String>,
        workspace_path: Option<String>,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    let ws = workspace_path
                        .filter(|p| !p.trim().is_empty())
                        .unwrap_or_else(|| s.settings.active_workspace_path.clone());
                    let now = Utc::now();
                    let sess = Session {
                        id: Uuid::new_v4().to_string(),
                        title: title.unwrap_or_else(|| "New Conversation".into()),
                        workspace_path: ws.clone(),
                        mode: AgentMode::Auto,
                        messages: Vec::new(),
                        pending_approval: None,
                        active_plan: None,
                        pending_question: None,
                        checkpoints: Vec::new(),
                        artifacts: Vec::new(),
                        created_at: now,
                        updated_at: now,
                    };
                    s.settings.active_session_id = sess.id.clone();
                    s.settings.active_workspace_path = ws;
                    s.sessions.insert(0, sess);
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn create_standalone_session(self: &Arc<Self>, title: Option<String>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    let now = Utc::now();
                    let sess = Session {
                        id: Uuid::new_v4().to_string(),
                        title: title.unwrap_or_else(|| "New Conversation".into()),
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
                    };
                    s.settings.active_session_id = sess.id.clone();
                    s.sessions.insert(0, sess);
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn move_session_to_workspace(
        self: &Arc<Self>,
        session_id: String,
        workspace_path: Option<String>,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let next_ws = workspace_path
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .unwrap_or_default();
            let _ = h
                .store
                .update(|s| {
                    if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                        sess.workspace_path = next_ws.clone();
                        sess.updated_at = Utc::now();
                        if s.settings.active_session_id == session_id && !next_ws.is_empty() {
                            s.settings.active_workspace_path = next_ws;
                        }
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn select_session(self: &Arc<Self>, session_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    if let Some(sess) = s.sessions.iter().find(|x| x.id == session_id) {
                        s.settings.active_session_id = session_id.clone();
                        if !sess.workspace_path.trim().is_empty()
                            && Path::new(&sess.workspace_path).exists()
                        {
                            s.settings.active_workspace_path = sess.workspace_path.clone();
                        }
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn rename_session(self: &Arc<Self>, session_id: String, new_title: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let clean = new_title.trim().to_string();
            if clean.is_empty() {
                return;
            }
            let _ = h
                .store
                .update(|s| {
                    if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                        sess.title = clean;
                        sess.updated_at = Utc::now();
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn delete_session(self: &Arc<Self>, session_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    s.sessions.retain(|x| x.id != session_id);
                    if s.sessions.is_empty() {
                        let now = Utc::now();
                        let fresh = Session {
                            id: Uuid::new_v4().to_string(),
                            title: "New Conversation".into(),
                            workspace_path: s.settings.active_workspace_path.clone(),
                            mode: AgentMode::Auto,
                            messages: Vec::new(),
                            pending_approval: None,
                            active_plan: None,
                            pending_question: None,
                            checkpoints: Vec::new(),
                            artifacts: Vec::new(),
                            created_at: now,
                            updated_at: now,
                        };
                        s.settings.active_session_id = fresh.id.clone();
                        s.sessions.push(fresh);
                    } else if s.settings.active_session_id == session_id {
                        s.settings.active_session_id = s.sessions[0].id.clone();
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn set_session_mode(self: &Arc<Self>, session_id: String, mode: AgentMode) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                        sess.mode = mode;
                        sess.updated_at = Utc::now();
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn select_workspace(self: &Arc<Self>, path: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let raw = path.trim();
            let pb = PathBuf::from(raw);
            if !pb.exists() || !pb.is_dir() {
                h.toast(format!("Workspace folder not found: {raw}"), true);
                return;
            }
            let canon = clean_verbatim(
                std::fs::canonicalize(&pb)
                    .unwrap_or(pb)
                    .to_string_lossy()
                    .to_string(),
            );
            let name = Path::new(&canon)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Workspace")
                .to_string();

            let _ = h
                .store
                .update(|s| {
                    s.settings.active_workspace_path = canon.clone();
                    if !s
                        .workspaces
                        .iter()
                        .any(|w| w.path.eq_ignore_ascii_case(&canon))
                    {
                        s.workspaces.push(WorkspaceEntry {
                            id: Uuid::new_v4().to_string(),
                            name,
                            path: canon.clone(),
                            is_uni_project: false,
                        });
                    }

                    let active_matches = s
                        .sessions
                        .iter()
                        .find(|x| x.id == s.settings.active_session_id)
                        .map(|x| x.workspace_path.eq_ignore_ascii_case(&canon))
                        .unwrap_or(false);

                    if !active_matches {
                        if let Some(existing) = s
                            .sessions
                            .iter()
                            .filter(|x| x.workspace_path.eq_ignore_ascii_case(&canon))
                            .max_by_key(|x| x.updated_at)
                        {
                            s.settings.active_session_id = existing.id.clone();
                        } else {
                            let now = Utc::now();
                            let fresh = Session {
                                id: Uuid::new_v4().to_string(),
                                title: "New Conversation".into(),
                                workspace_path: canon.clone(),
                                mode: AgentMode::Auto,
                                messages: Vec::new(),
                                pending_approval: None,
                                active_plan: None,
                                pending_question: None,
                                checkpoints: Vec::new(),
                                artifacts: Vec::new(),
                                created_at: now,
                                updated_at: now,
                            };
                            s.settings.active_session_id = fresh.id.clone();
                            s.sessions.push(fresh);
                        }
                    }
                })
                .await;
            h.toast(format!("Workspace set to {canon}"), false);
            h.emit_snapshot(false).await;
        });
    }

    pub fn remove_workspace(self: &Arc<Self>, path: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let target = path.trim().to_string();
            let _ = h
                .store
                .update(|s| {
                    if s.workspaces.len() > 1 {
                        s.workspaces.retain(|w| !w.path.eq_ignore_ascii_case(&target));
                        if s.settings.active_workspace_path.eq_ignore_ascii_case(&target) {
                            if let Some(first) = s.workspaces.first() {
                                s.settings.active_workspace_path = first.path.clone();
                            }
                        }
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    // ---- Native OS File / Folder Dialogs & Workspace CRUD ----

    pub fn open_folder_picker(self: &Arc<Self>, target: FolderPickTarget, title: &'static str) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let res = tokio::task::spawn_blocking(move || pick_folder(title)).await;
            match res {
                Ok(Ok(Some(p))) => {
                    h.send(BackendEvent::FolderPicked {
                        target,
                        path: p.clone(),
                    });
                    if target == FolderPickTarget::Workspace {
                        h.select_workspace(p);
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => h.toast(e.to_string(), true),
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn open_file_picker(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let res =
                tokio::task::spawn_blocking(|| pick_files("Attach Files or Pictures to Cinder"))
                    .await;
            match res {
                Ok(Ok(paths)) if !paths.is_empty() => {
                    h.send(BackendEvent::FilesPicked(paths));
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => h.toast(e.to_string(), true),
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn load_workspace_file(self: &Arc<Self>, rel_path: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            let abs = match harness_tools::resolve_workspace_path(Path::new(&ws), &rel_path) {
                Ok(p) => p,
                Err(e) => {
                    h.toast(e.to_string(), true);
                    return;
                }
            };
            let ext = abs
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let size_bytes = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);

            let payload = if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "ico")
            {
                let bytes = std::fs::read(&abs).unwrap_or_default();
                OpenedFilePayload {
                    rel_path,
                    abs_path: abs,
                    kind: "image".into(),
                    content: String::new(),
                    bytes,
                    size_bytes,
                }
            } else if ext == "pdf" {
                let text = harness_tools::extract_pdf_file_text(&abs, 30)
                    .unwrap_or_else(|e| format!("Failed to extract PDF text: {e}"));
                OpenedFilePayload {
                    rel_path,
                    abs_path: abs,
                    kind: "pdf".into(),
                    content: text,
                    bytes: Vec::new(),
                    size_bytes,
                }
            } else {
                match std::fs::read_to_string(&abs) {
                    Ok(content) => OpenedFilePayload {
                        rel_path,
                        abs_path: abs,
                        kind: "text".into(),
                        content,
                        bytes: Vec::new(),
                        size_bytes,
                    },
                    Err(_) => OpenedFilePayload {
                        rel_path,
                        abs_path: abs,
                        kind: "binary".into(),
                        content: format!("Binary file ({size_bytes} bytes)."),
                        bytes: Vec::new(),
                        size_bytes,
                    },
                }
            };
            h.send(BackendEvent::FileLoaded(payload));
        });
    }

    pub fn save_workspace_file(self: &Arc<Self>, rel_path: String, content: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match harness_tools::resolve_workspace_path(Path::new(&ws), &rel_path) {
                Ok(abs) => {
                    if let Err(e) = std::fs::write(&abs, &content) {
                        h.toast(format!("Failed to save {rel_path}: {e}"), true);
                    } else {
                        h.toast(format!("Saved {rel_path}"), false);
                        h.emit_snapshot(false).await;
                    }
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn create_file(self: &Arc<Self>, rel_path: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match create_workspace_file(Path::new(&ws), &rel_path) {
                Ok(_) => {
                    h.toast(format!("Created file {rel_path}"), false);
                    h.emit_snapshot(false).await;
                    h.load_workspace_file(rel_path);
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn create_dir(self: &Arc<Self>, rel_path: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match create_workspace_dir(Path::new(&ws), &rel_path) {
                Ok(_) => {
                    h.toast(format!("Created folder {rel_path}"), false);
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn rename_entry(self: &Arc<Self>, old_rel: String, new_name: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match rename_workspace_entry(Path::new(&ws), &old_rel, &new_name) {
                Ok(new_rel) => {
                    h.toast(format!("Renamed to {new_rel}"), false);
                    h.send(BackendEvent::EntryRenamed { old_rel, new_rel });
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn delete_entry(self: &Arc<Self>, rel_path: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match delete_workspace_entry(Path::new(&ws), &rel_path) {
                Ok(()) => {
                    h.toast(format!("Deleted {rel_path}"), false);
                    h.send(BackendEvent::EntryDeleted { rel_path });
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn reveal_entry(self: &Arc<Self>, path: PathBuf) {
        let h = Arc::clone(self);
        std::thread::spawn(move || {
            if let Err(e) = harness_tools::reveal_in_file_manager(&path) {
                h.toast(format!("Could not open file manager: {e}"), true);
            }
        });
    }

    // ---- Rules, Skills, Knowledge & Terminal ----

    pub fn save_rules(self: &Arc<Self>, content: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match save_workspace_rules(Path::new(&ws), &content) {
                Ok(_) => {
                    h.toast("Saved .cinder/rules.md", false);
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn save_skill(self: &Arc<Self>, skill: SkillDefinition) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match save_custom_skill(Path::new(&ws), &skill) {
                Ok(_) => {
                    h.toast(format!("Saved skill /{}", skill.slug), false);
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn index_kb(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match build_workspace_index(Path::new(&ws), &h.gate).await {
                Ok(stats) => {
                    h.toast(format!("Indexed {} chunks offline.", stats.total_chunks), false);
                    h.send(BackendEvent::KnowledgeIndexed(stats));
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn search_kb(self: &Arc<Self>, query: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match search_workspace_knowledge(Path::new(&ws), &h.gate, &query, 6).await {
                Ok(hits) => h.send(BackendEvent::KnowledgeHits(hits)),
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn run_terminal(self: &Arc<Self>, command: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws_root = h.store.snapshot().await.settings.active_workspace_path;
            let trimmed = command.trim().to_string();
            if trimmed.is_empty() {
                h.toast("Command cannot be empty.", true);
                return;
            }
            let started = std::time::Instant::now();
            let mut cmd = if cfg!(windows) {
                let mut c = tokio::process::Command::new("powershell");
                c.args(["-NoProfile", "-Command", &trimmed]);
                #[cfg(windows)]
                {
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    c.creation_flags(CREATE_NO_WINDOW);
                }
                c
            } else {
                let mut c = tokio::process::Command::new("sh");
                c.args(["-c", &trimmed]);
                c
            };
            if !ws_root.trim().is_empty() && Path::new(&ws_root).is_dir() {
                cmd.current_dir(&ws_root);
            }
            let out = match tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
                .await
            {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => {
                    h.toast(e.to_string(), true);
                    return;
                }
                Err(_) => {
                    h.toast("Terminal command timed out after 60s.", true);
                    return;
                }
            };
            let exit_code = out.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{stdout}{stderr}").trim().to_string();
            h.send(BackendEvent::TerminalOutput {
                cmd: trimmed,
                exit_code,
                output: if combined.is_empty() {
                    "(no output)".into()
                } else {
                    combined
                },
                duration_ms: started.elapsed().as_millis() as u64,
            });
        });
    }

    // ---- Plugins, Dependencies & Antigravity Sync ----

    pub fn install_plugin(self: &Arc<Self>, plugin_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let catalog = list_plugin_catalog_with_status(h.store.paths(), &[]);
            let Some(item) = catalog.iter().find(|c| c.entry.id == plugin_id) else {
                h.toast(format!("Plugin '{plugin_id}' is not on Cinder's allow-list."), true);
                return;
            };
            if item.entry.kind != "skill_pack" && h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    format!(
                        "Strict Local Mode blocked outbound install of '{}'. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                        item.entry.name
                    ),
                    true,
                );
                return;
            }
            let job = h.install_jobs.start_job("plugin", &plugin_id).await;
            let job_id = job.id.clone();
            h.emit_snapshot(false).await;
            let rt_handle = tokio::runtime::Handle::current();
            let jobs_cb = h.install_jobs.clone();
            let jid_cb = job_id.clone();

            let res = install_curated_plugin(&h.gate, h.store.paths(), &plugin_id, |pct, msg| {
                let ji = jobs_cb.clone();
                let id = jid_cb.clone();
                rt_handle.spawn(async move {
                    ji.push_progress(&id, pct, msg).await;
                });
            })
            .await;

            match res {
                Ok(outcome) => {
                    if let Some(mcp_entry) = outcome.mcp_entry {
                        let _ = h
                            .store
                            .update(|s| {
                                if let Some(ex) =
                                    s.mcp_servers.iter_mut().find(|m| m.id == mcp_entry.id)
                                {
                                    *ex = mcp_entry;
                                } else {
                                    s.mcp_servers.push(mcp_entry);
                                }
                            })
                            .await;
                    }
                    if let Some(probe) = outcome.probe_result {
                        h.send(BackendEvent::McpProbed(probe));
                    }
                    h.install_jobs.finish_ok(&job_id, outcome.summary.clone()).await;
                    h.toast(outcome.summary, false);
                }
                Err(e) => {
                    let msg = e.to_string();
                    h.install_jobs.finish_err(&job_id, msg.clone()).await;
                    h.toast(msg, true);
                }
            }
            h.emit_snapshot(true).await;
        });
    }

    pub fn uninstall_plugin(self: &Arc<Self>, plugin_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            match uninstall_curated_plugin(h.store.paths(), &plugin_id) {
                Ok(maybe_mcp) => {
                    if let Some(mcp_id) = maybe_mcp {
                        evict_mcp_server_session(&mcp_id).await;
                        let _ = h
                            .store
                            .update(|s| {
                                s.mcp_servers.retain(|m| m.id != mcp_id);
                            })
                            .await;
                    }
                    h.toast(format!("Uninstalled {plugin_id}"), false);
                    h.emit_snapshot(true).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn install_dependency(self: &Arc<Self>, manager: String, package: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if is_dependency_allowlisted(&manager, &package).is_none() {
                h.toast(
                    format!("Package '{package}' ({manager}) is not on Cinder's allow-list."),
                    true,
                );
                return;
            }
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    format!(
                        "Strict Local Mode blocked installing '{package}'. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first."
                    ),
                    true,
                );
                return;
            }
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            let job = h
                .install_jobs
                .start_job("dependency", &format!("{manager}:{package}"))
                .await;
            let job_id = job.id.clone();
            h.emit_snapshot(false).await;
            let rt_handle = tokio::runtime::Handle::current();
            let jobs_cb = h.install_jobs.clone();
            let jid_cb = job_id.clone();

            let res = install_project_dependency(
                &h.gate,
                h.store.paths(),
                Path::new(&ws),
                &manager,
                &package,
                |pct, msg| {
                    let ji = jobs_cb.clone();
                    let id = jid_cb.clone();
                    rt_handle.spawn(async move {
                        ji.push_progress(&id, pct, msg).await;
                    });
                },
            )
            .await;

            match res {
                Ok(summary) => {
                    h.install_jobs.finish_ok(&job_id, summary.clone()).await;
                    h.toast(summary, false);
                }
                Err(e) => {
                    let msg = e.to_string();
                    h.install_jobs.finish_err(&job_id, msg.clone()).await;
                    h.toast(msg, true);
                }
            }
            h.emit_snapshot(true).await;
        });
    }

    pub fn install_custom_dependency(self: &Arc<Self>, manager: String, package: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let entry = match validate_custom_package_name(&manager, &package) {
                Ok(e) => e,
                Err(e) => {
                    h.toast(e.to_string(), true);
                    return;
                }
            };
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    format!(
                        "Strict Local Mode blocked installing '{}'. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                        entry.name
                    ),
                    true,
                );
                return;
            }
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            let job = h
                .install_jobs
                .start_job("dependency", &format!("{}:{}", entry.manager, entry.name))
                .await;
            let job_id = job.id.clone();
            h.emit_snapshot(false).await;
            let rt_handle = tokio::runtime::Handle::current();
            let jobs_cb = h.install_jobs.clone();
            let jid_cb = job_id.clone();

            let res = install_custom_project_dependency(
                &h.gate,
                h.store.paths(),
                Path::new(&ws),
                &entry.manager,
                &entry.name,
                |pct, msg| {
                    let ji = jobs_cb.clone();
                    let id = jid_cb.clone();
                    rt_handle.spawn(async move {
                        ji.push_progress(&id, pct, msg).await;
                    });
                },
            )
            .await;

            match res {
                Ok(summary) => {
                    h.install_jobs.finish_ok(&job_id, summary.clone()).await;
                    h.toast(summary, false);
                }
                Err(e) => {
                    let msg = e.to_string();
                    h.install_jobs.finish_err(&job_id, msg.clone()).await;
                    h.toast(msg, true);
                }
            }
            h.emit_snapshot(true).await;
        });
    }

    pub fn uninstall_dependency(self: &Arc<Self>, manager: String, package: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            let ws_pb = PathBuf::from(&ws);
            let res = tokio::task::spawn_blocking(move || {
                uninstall_project_dependency(&ws_pb, &manager, &package)
            })
            .await;
            match res {
                Ok(Ok(summary)) => {
                    h.toast(summary, false);
                    h.emit_snapshot(false).await;
                }
                Ok(Err(e)) => h.toast(e.to_string(), true),
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn clear_install_jobs(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            h.install_jobs.clear_finished().await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn probe_mcp(self: &Arc<Self>, server_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let snap = h.store.snapshot().await;
            let Some(entry) = snap.mcp_servers.iter().find(|m| m.id == server_id).cloned() else {
                h.toast("MCP server not found.", true);
                return;
            };
            let probe = probe_mcp_server(&h.gate, &entry).await;
            if probe.ok {
                h.toast(
                    format!("MCP ready: {} tool(s) ({}ms)", probe.tools.len(), probe.latency_ms),
                    false,
                );
            } else {
                h.toast(
                    probe
                        .error
                        .clone()
                        .unwrap_or_else(|| "MCP probe failed".into()),
                    true,
                );
            }
            h.send(BackendEvent::McpProbed(probe));
        });
    }

    pub fn save_mcp(self: &Arc<Self>, name: String, transport: String, target: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let is_http = transport.eq_ignore_ascii_case("http");
            let entry = McpServerEntry {
                id: Uuid::new_v4().to_string(),
                name: name.trim().to_string(),
                transport,
                target: target.trim().to_string(),
                enabled: true,
                command: String::new(),
                args: Vec::new(),
                env: HashMap::new(),
                url: if is_http { target.trim().into() } else { String::new() },
                source: "manual".into(),
                needs_network: is_http
                    && !target.contains("127.0.0.1")
                    && !target.contains("localhost"),
                read_only: false,
            };
            let _ = h
                .store
                .update(|s| {
                    s.mcp_servers.push(entry);
                })
                .await;
            h.toast(format!("Added MCP server {}", name.trim()), false);
            h.emit_snapshot(false).await;
        });
    }

    pub fn toggle_mcp(self: &Arc<Self>, id: String, enabled: bool) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if !enabled {
                evict_mcp_server_session(&id).await;
            }
            let _ = h
                .store
                .update(|s| {
                    if let Some(m) = s.mcp_servers.iter_mut().find(|x| x.id == id) {
                        m.enabled = enabled;
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn remove_mcp(self: &Arc<Self>, id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            evict_mcp_server_session(&id).await;
            let _ = h
                .store
                .update(|s| {
                    s.mcp_servers.retain(|x| x.id != id);
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn sync_antigravity(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let ws = h.store.snapshot().await.settings.active_workspace_path;
            match sync_from_antigravity(h.store.paths(), Path::new(&ws), false) {
                Ok(AntigravitySyncReport {
                    imported_mcp_servers,
                    imported_skills,
                    ..
                }) => {
                    let n_mcp = imported_mcp_servers.len();
                    let n_sk = imported_skills.len();
                    if !imported_mcp_servers.is_empty() {
                        let _ = h
                            .store
                            .update(|s| {
                                for entry in imported_mcp_servers {
                                    if let Some(ex) =
                                        s.mcp_servers.iter_mut().find(|m| m.id == entry.id)
                                    {
                                        *ex = entry;
                                    } else {
                                        s.mcp_servers.push(entry);
                                    }
                                }
                            })
                            .await;
                    }
                    h.toast(
                        format!("Antigravity sync: {n_mcp} MCP server(s), {n_sk} skill(s) imported."),
                        false,
                    );
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn export_antigravity_mcp(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let snap = h.store.snapshot().await;
            let val = export_antigravity_mcp_json(&snap.mcp_servers);
            let txt = serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
            h.send(BackendEvent::AntigravityExported(txt));
        });
    }

    // ---- Local Models, HF Hub, Cloud Keys & Settings ----

    pub fn start_local_model(self: &Arc<Self>, model_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let snap = h.store.snapshot().await;
            let hw = h.hardware_snapshot(false).await;
            let locals = scan_local_models(
                Path::new(&snap.settings.model_vault_dir),
                hw.primary_vram_mb,
                snap.settings.local_ctx_size,
            );
            let Some(model) = locals.iter().find(|m| m.id == model_id) else {
                h.toast("Local model not found.", true);
                return;
            };
            let oom = harness_models::assess_oom_risk(
                &model.vram_fit,
                snap.settings.local_ctx_size,
                snap.settings.local_gpu_layers,
                hw.primary_vram_mb,
                hw.system_ram_mb,
            );
            if oom.level == harness_models::OomRiskLevel::HighOomRisk {
                h.toast(format!("⚠ {}: {}", oom.summary, oom.detail), true);
            }
            match h
                .server_mgr
                .start_local_model(
                    &h.gate,
                    model,
                    snap.settings.local_server_port,
                    snap.settings.local_ctx_size,
                    snap.settings.local_gpu_layers,
                    snap.settings.expose_local_api,
                )
                .await
            {
                Ok(_) => {
                    let _ = h
                        .store
                        .update(|s| {
                            s.settings.active_model_id = model_id.clone();
                        })
                        .await;
                    h.toast(format!("Serving {} on loopback", model.name), false);
                    h.emit_snapshot(true).await;
                }
                Err(e) => {
                    h.toast(e.to_string(), true);
                    h.emit_snapshot(false).await;
                }
            }
        });
    }

    pub fn stop_all_local_models(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            match h.server_mgr.stop_all_local_models(&h.gate).await {
                Ok(unloaded) => {
                    let msg = if unloaded.is_empty() {
                        "Forge quenched — VRAM free.".into()
                    } else {
                        format!("Quenched {} — VRAM freed.", unloaded.join(", "))
                    };
                    h.toast(msg, false);
                    h.emit_snapshot(true).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn install_engine(self: &Arc<Self>, backend: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    "Strict Local Mode blocked engine download. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                    true,
                );
                return;
            }
            h.toast(format!("Installing llama-server ({backend})..."), false);
            match h
                .server_mgr
                .install_llama_server(Arc::clone(&h.gate), &backend)
                .await
            {
                Ok(msg) => {
                    h.toast(msg, false);
                    h.emit_snapshot(true).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn set_vault_dir(self: &Arc<Self>, dir: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let p = PathBuf::from(dir.trim());
            if let Err(e) = std::fs::create_dir_all(&p) {
                h.toast(format!("Cannot create directory: {e}"), true);
                return;
            }
            let canon = clean_verbatim(
                std::fs::canonicalize(&p)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string(),
            );
            let _ = h
                .store
                .update(|s| {
                    s.settings.model_vault_dir = canon.clone();
                })
                .await;
            h.toast(format!("Model vault updated: {canon}"), false);
            h.emit_snapshot(false).await;
        });
    }

    pub fn search_hf(self: &Arc<Self>, query: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    "Strict Local Mode blocked Hugging Face network access. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                    true,
                );
                return;
            }
            let tok = h.store.snapshot().await.settings.api_keys.huggingface_token;
            match search_hf_gguf(&h.gate, &query, &tok).await {
                Ok(res) => h.send(BackendEvent::HfSearchResults(res)),
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn inspect_hf_repo(self: &Arc<Self>, repo_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    "Strict Local Mode blocked Hugging Face network access. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                    true,
                );
                return;
            }
            let snap = h.store.snapshot().await;
            let hw = h.hardware_snapshot(false).await;
            match inspect_hf_repo(
                &h.gate,
                &repo_id,
                &snap.settings.api_keys.huggingface_token,
                hw.primary_vram_mb,
                snap.settings.local_ctx_size,
            )
            .await
            {
                Ok(det) => h.send(BackendEvent::HfRepoInspected(det)),
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn download_hf(
        self: &Arc<Self>,
        repo_id: String,
        filename: String,
        sha256: Option<String>,
        size_bytes: Option<u64>,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    "Strict Local Mode blocked Hugging Face network access. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                    true,
                );
                return;
            }
            let snap = h.store.snapshot().await;
            match h
                .download_mgr
                .start_hf_download(
                    Arc::clone(&h.gate),
                    PathBuf::from(&snap.settings.model_vault_dir),
                    repo_id,
                    filename.clone(),
                    sha256,
                    size_bytes,
                    snap.settings.api_keys.huggingface_token,
                )
                .await
            {
                Ok(_) => {
                    h.toast(format!("Downloading {filename}..."), false);
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn pause_hf_download(self: &Arc<Self>, job_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h.download_mgr.pause_job(&job_id).await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn resume_hf_download(self: &Arc<Self>, job_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    "Strict Local Mode blocked Hugging Face network access. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                    true,
                );
                return;
            }
            let tok = h.store.snapshot().await.settings.api_keys.huggingface_token;
            if let Err(e) = h
                .download_mgr
                .resume_job(&job_id, Arc::clone(&h.gate), tok)
                .await
            {
                h.toast(e.to_string(), true);
            }
            h.emit_snapshot(false).await;
        });
    }

    pub fn cancel_hf_download(self: &Arc<Self>, job_id: String) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h.download_mgr.cancel_job(&job_id).await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn save_keys(self: &Arc<Self>, patch: KeysPatch, verify_live: bool) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    for k in &patch.clear_keys {
                        match k.as_str() {
                            "gemini" => {
                                s.settings.api_keys.gemini_api_key.clear();
                                s.verified_provider_models.remove("google_gemini");
                            }
                            "anthropic" => {
                                s.settings.api_keys.anthropic_api_key.clear();
                                s.verified_provider_models.remove("anthropic");
                            }
                            "openrouter" => {
                                s.settings.api_keys.openrouter_api_key.clear();
                                s.verified_provider_models.remove("open_router");
                            }
                            "openai_compat" => {
                                s.settings.api_keys.openai_compat_base_url.clear();
                                s.settings.api_keys.openai_compat_api_key.clear();
                                s.verified_provider_models.remove("open_ai_compat");
                            }
                            "huggingface" => s.settings.api_keys.huggingface_token.clear(),
                            "brave" => s.settings.api_keys.brave_search_api_key.clear(),
                            _ => {}
                        }
                    }
                    if let Some(v) = &patch.gemini_api_key {
                        if !v.is_empty() {
                            s.settings.api_keys.gemini_api_key = v.clone();
                        }
                    }
                    if let Some(v) = &patch.anthropic_api_key {
                        if !v.is_empty() {
                            s.settings.api_keys.anthropic_api_key = v.clone();
                        }
                    }
                    if let Some(v) = &patch.openrouter_api_key {
                        if !v.is_empty() {
                            s.settings.api_keys.openrouter_api_key = v.clone();
                        }
                    }
                    if let Some(v) = &patch.openai_compat_base_url {
                        s.settings.api_keys.openai_compat_base_url = v.clone();
                    }
                    if let Some(v) = &patch.openai_compat_api_key {
                        if !v.is_empty() {
                            s.settings.api_keys.openai_compat_api_key = v.clone();
                        }
                    }
                    if let Some(v) = &patch.huggingface_token {
                        if !v.is_empty() {
                            s.settings.api_keys.huggingface_token = v.clone();
                        }
                    }
                    if let Some(v) = &patch.brave_search_api_key {
                        if !v.is_empty() {
                            s.settings.api_keys.brave_search_api_key = v.clone();
                        }
                    }
                })
                .await;

            if verify_live {
                if h.gate.mode() == PrivacyMode::StrictLocal {
                    h.toast(
                        "API keys saved locally. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover to verify online.",
                        true,
                    );
                } else {
                    match verify_and_discover_provider_models(&h.gate, &h.store).await {
                        Ok(statuses) => {
                            h.send(BackendEvent::ProviderVerifyResults(statuses));
                            h.toast("Live-probed and updated reachable models.", false);
                        }
                        Err(e) => h.toast(e.to_string(), true),
                    }
                }
            } else {
                h.toast("API keys updated locally.", false);
            }
            h.emit_snapshot(false).await;
        });
    }

    pub fn sync_google(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            if h.gate.mode() == PrivacyMode::StrictLocal {
                h.toast(
                    "Strict Local Mode blocked Google model sync. Enable Hybrid Mode in the bottom-left System & Forge Telemetry popover first.",
                    true,
                );
                return;
            }
            let key = h.store.snapshot().await.settings.api_keys.gemini_api_key;
            match sync_google_models(&h.gate, &key).await {
                Ok(discovered) => {
                    let count = discovered.len();
                    let _ = h
                        .store
                        .update(|s| {
                            s.blocked_models
                                .retain(|id, _| !id.starts_with("gemini-") && !id.starts_with("gemma-"));
                            s.synced_google_models = discovered.clone();
                            s.verified_provider_models
                                .insert("google_gemini".to_string(), discovered);
                            if !s.settings.active_model_id.starts_with("local::") {
                                let (cat, _) = compute_catalog_and_quotas(s);
                                if !cat
                                    .iter()
                                    .any(|c| c.id == s.settings.active_model_id && c.reachable)
                                {
                                    if let Some(first) = cat.iter().find(|c| c.reachable) {
                                        s.settings.active_model_id = first.id.clone();
                                    }
                                }
                            }
                        })
                        .await;
                    h.toast(format!("Live-probed {count} reachable Google models."), false);
                    h.emit_snapshot(false).await;
                }
                Err(e) => h.toast(e.to_string(), true),
            }
        });
    }

    pub fn add_custom_model(self: &Arc<Self>, id: String, display_name: Option<String>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let clean = id.trim().to_string();
            if clean.is_empty() {
                h.toast("Model ID cannot be empty.", true);
                return;
            }
            let lower = clean.to_ascii_lowercase();
            let provider = if lower.starts_with("gemini-") || lower.starts_with("gemma-") {
                ProviderKind::GoogleGemini
            } else if lower.starts_with("claude-") {
                ProviderKind::Anthropic
            } else if lower.contains('/') {
                ProviderKind::OpenRouter
            } else {
                ProviderKind::OpenAiCompat
            };

            if provider == ProviderKind::GoogleGemini && h.gate.mode() == PrivacyMode::Hybrid {
                let key = h.store.snapshot().await.settings.api_keys.gemini_api_key;
                if !key.trim().is_empty() && !probe_gemini_model_live(&h.gate, &key, &clean).await
                {
                    h.toast(
                        format!("Model '{clean}' failed live probe (404 / limit: 0)."),
                        true,
                    );
                    return;
                }
            }

            let disp = display_name
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| clean.clone());
            let entry = ModelCatalogEntry {
                id: clean.clone(),
                display_name: disp.clone(),
                provider,
                context_window: 1_000_000,
                supports_thinking: true,
                supports_tools: !lower.starts_with("gemma"),
                supports_vision: true,
                tier_note: "Custom registered model".into(),
                quota_limit: Some(ModelQuotaLimit {
                    rpm_limit: 500,
                    tpm_limit: 2_000_000,
                    rpd_limit: 5_000,
                    daily_token_limit: 15_000_000,
                }),
                reachable: true,
                reachability_reason: "Custom registered model".into(),
                antigravity_alias: Some(disp),
                requires_payment: false,
            };
            let _ = h
                .store
                .update(|s| {
                    s.blocked_models.remove(&clean);
                    s.custom_models.retain(|m| m.id != clean);
                    s.custom_models.push(entry.clone());
                    let pk = provider_key_name(provider).to_string();
                    let list = s.verified_provider_models.entry(pk).or_default();
                    if !list.iter().any(|m| m.id == clean) {
                        list.push(entry.clone());
                    }
                    if provider == ProviderKind::GoogleGemini
                        && !s.synced_google_models.iter().any(|m| m.id == clean)
                    {
                        s.synced_google_models.push(entry);
                    }
                })
                .await;
            h.toast(format!("Registered custom model {clean}"), false);
            h.emit_snapshot(false).await;
        });
    }

    pub fn set_quota_tier(self: &Arc<Self>, tier: QuotaTier) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    s.settings.quota_tier = tier;
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn save_ui_and_orch(
        self: &Arc<Self>,
        ui: Option<UiCustomization>,
        orchestrator: Option<OrchestratorConfig>,
    ) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let _ = h
                .store
                .update(|s| {
                    if let Some(u) = ui {
                        s.settings.ui = u;
                    }
                    if let Some(o) = orchestrator {
                        s.settings.orchestrator = o;
                    }
                })
                .await;
            h.emit_snapshot(false).await;
        });
    }

    pub fn update_settings(self: &Arc<Self>, f: impl FnOnce(&mut AppSettings) + Send + 'static) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            let prev = h.store.snapshot().await.settings.active_model_id;
            let now_model = h
                .store
                .update(|s| {
                    f(&mut s.settings);
                    s.settings.active_model_id.clone()
                })
                .await
                .unwrap_or_default();
            if prev.starts_with("local::") && !now_model.starts_with("local::") {
                let _ = h.server_mgr.stop_all_local_models(&h.gate).await;
            }
            h.emit_snapshot(false).await;
        });
    }

    pub fn set_privacy(self: &Arc<Self>, mode: PrivacyMode) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            h.gate.set_mode(mode);
            let _ = h
                .store
                .update(|s| {
                    s.settings.privacy_mode = mode;
                })
                .await;
            h.toast(
                if mode == PrivacyMode::StrictLocal {
                    "Strict Local Mode active — external sockets blocked."
                } else {
                    "Hybrid Mode active — cloud APIs & curated installers allowed."
                },
                false,
            );
            h.emit_snapshot(false).await;
        });
    }

    pub fn clear_audit(self: &Arc<Self>) {
        let h = Arc::clone(self);
        self.rt.spawn(async move {
            h.gate.clear_audit();
            h.emit_snapshot(false).await;
        });
    }
}

fn clean_verbatim(p: String) -> String {
    p.strip_prefix(r"\\?\").unwrap_or(&p).to_string()
}
