use crate::backend::{BackendHandle, OpenedFilePayload, UiSnapshot};
use harness_models::{HfModelSummary, HfRepoDetails};
use harness_providers::ProviderVerifyStatus;
use harness_store::{AgentMode, PlanStatus, ProviderKind};
use harness_tools::{GitDiffEntry, IndexStats, KnowledgeSearchHit, McpProbeResult, WorkspaceFileNode};
use image::GenericImageView;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeTab {
    Vault,
    Hub,
    Quotas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotasTabFilter {
    Available,
    Paid,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomTab {
    Plugins,
    Skills,
    Mcp,
    Rules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainView {
    Chat,
    Automations,
    Forge(ForgeTab),
    Customizations(CustomTab),
}

#[derive(Debug, Clone)]
pub struct AutomationPlaybook {
    pub id: String,
    pub title: String,
    pub category: String,
    pub description: String,
    pub prompt: String,
    pub mode: AgentMode,
    pub is_builtin: bool,
}

fn default_automation_playbooks() -> Vec<AutomationPlaybook> {
    vec![
        AutomationPlaybook {
            id: "uni_lecture_digest".into(),
            title: "Uni Lecture & PDF Study Digest".into(),
            category: "Uni Helper".into(),
            description: "Scans markdown notes and PDFs in the active project and creates a structured study summary.".into(),
            prompt: "Inspect the markdown files and PDFs in this workspace, then generate a structured study digest highlighting definitions, formulas, and key exam takeaways.".into(),
            mode: AgentMode::Agent,
            is_builtin: true,
        },
        AutomationPlaybook {
            id: "uni_exam_quiz".into(),
            title: "Exam Quiz & Flashcard Generator".into(),
            category: "Uni Helper".into(),
            description: "Builds spaced-repetition Q&A flashcards and sample exam problems from course materials.".into(),
            prompt: "Review the active project's notes and lecture PDFs, then generate 10 exam-style practice questions with concise worked solutions and Anki-ready flashcards.".into(),
            mode: AgentMode::Agent,
            is_builtin: true,
        },
        AutomationPlaybook {
            id: "workspace_md_arch".into(),
            title: "Workspace Markdown & Architecture Map".into(),
            category: "Documentation".into(),
            description: "Maps modules, key folders, and existing documentation into a crisp overview.".into(),
            prompt: "Explore the active project's directory structure, markdown docs, and core files, and produce an accurate module architecture map citing only verified paths and symbols.".into(),
            mode: AgentMode::Agent,
            is_builtin: true,
        },
        AutomationPlaybook {
            id: "codebase_audit".into(),
            title: "Codebase Warning, Test & Bug Audit".into(),
            category: "Engineering".into(),
            description: "Runs project verification, inspects warnings or failing tests, and prioritizes concrete fixes.".into(),
            prompt: "Run workspace verification/tests, inspect any warnings, errors, or edge-case risks in changed files, and summarize actionable fixes with exact file references.".into(),
            mode: AgentMode::Agent,
            is_builtin: true,
        },
        AutomationPlaybook {
            id: "daily_study_plan".into(),
            title: "Daily Study & Sprint Plan".into(),
            category: "Planning".into(),
            description: "Creates an actionable milestone plan tailored to deadlines and files in the active workspace.".into(),
            prompt: "Review the current workspace files and draft a prioritized, step-by-step Plan Artifact for today's study or coding sprint.".into(),
            mode: AgentMode::Plan,
            is_builtin: true,
        },
        AutomationPlaybook {
            id: "deep_concept_breakdown".into(),
            title: "Deep Concept & Formula Breakdown".into(),
            category: "Uni Helper".into(),
            description: "Explains complex theorems, proofs, and worked examples step by step.".into(),
            prompt: "Identify the core theoretical concepts and formulas in the current project materials and explain each intuitively with worked examples and common pitfalls.".into(),
            mode: AgentMode::Auto,
            is_builtin: true,
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageTab {
    Editor,
    Artifacts,
    Diffs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffsViewMode {
    Checkpoints,
    GitHead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Appearance,
    Models,
    Engine,
    Agent,
    Privacy,
}

impl SettingsTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Models => "Models & Keys",
            Self::Engine => "Local Forge & GPU",
            Self::Agent => "Agent & Autonomy",
            Self::Privacy => "Privacy & Audit",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Appearance => "Customize surface contrast, ember glow accent, readability, and message metrics.",
            Self::Models => "Manage reachable model filtering, automatic quota fallback, and API credentials.",
            Self::Engine => "Configure GGUF storage, VRAM residency limits, and runtime inference backends.",
            Self::Agent => "Control network boundary, tool approvals, dual-model orchestration, and instructions.",
            Self::Privacy => "Inspect outbound network boundaries and live privacy audit logs.",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EditorTab {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub kind: String, // "text" | "image" | "pdf" | "binary"
    pub content: String,
    pub saved_content: String,
    pub preview_mode: bool,
    pub bytes: Vec<u8>,
    pub size_bytes: u64,
}

impl EditorTab {
    pub fn from_payload(p: OpenedFilePayload) -> Self {
        let preview = p.kind != "text";
        Self {
            rel_path: p.rel_path,
            abs_path: p.abs_path,
            kind: p.kind,
            content: p.content.clone(),
            saved_content: p.content,
            preview_mode: preview,
            bytes: p.bytes,
            size_bytes: p.size_bytes,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.kind == "text" && self.content != self.saved_content
    }
}

#[derive(Debug, Clone)]
pub enum PromptModal {
    None,
    NewFile {
        parent_rel: String,
        name: String,
    },
    NewFolder {
        parent_rel: String,
        name: String,
    },
    RenameEntry {
        old_rel: String,
        new_name: String,
    },
    DeleteEntry {
        rel_path: String,
        is_dir: bool,
    },
    RenameSession {
        session_id: String,
        title: String,
    },
    NewArtifact {
        title: String,
        kind: String,
        content: String,
    },
    EditPlan {
        markdown: String,
    },
    RequestPlanChanges {
        feedback: String,
    },
    LightboxImage {
        title: String,
        cache_key: String,
    },
}

#[derive(Debug, Clone)]
pub struct SelectableModel {
    pub id: String,
    pub display_name: String,
    pub canonical_name: String,
    pub badge: String,
    pub provider_label: String,
    pub category: String, // "local" | "google" | "cloud"
    pub reachable: bool,
    pub requires_payment: bool,
    pub is_local: bool,
    pub quota_pct: Option<f64>,
    pub quota_badge: String,
    pub quota_tooltip: String,
}

pub struct GuiState {
    pub snap: Option<Arc<UiSnapshot>>,

    // Layout toggles & resizable widths
    pub show_sidebar: bool,
    pub show_workbench: bool,
    pub show_explorer: bool,
    pub sidebar_width: f32,
    pub explorer_width: f32,
    pub show_terminal: bool,
    pub terminal_height: f32,

    // Active views
    pub main_view: MainView,
    pub stage_tab: StageTab,
    pub settings_tab: SettingsTab,
    pub show_settings_modal: bool,
    pub settings_search: String,
    pub show_cmd_palette: bool,
    pub show_status_popover: bool,
    pub show_downloads_popover: bool,
    pub composer_focused: bool,
    pub last_ctrl_down_at: Option<Instant>,
    pub ctrl_v_latch: bool,
    pub dragging_explorer_item: Option<(String, bool)>,

    // Navigation history
    pub nav_history: Vec<(MainView, String)>,
    pub nav_idx: isize,

    // Sidebar / Projects / Explorer state
    pub expanded_projects: HashSet<String>,
    pub expanded_project_all: HashSet<String>,
    pub did_init_projects: bool,
    pub show_session_filter: bool,
    pub session_filter: String,
    pub explorer_filter: String,
    pub explorer_expanded: HashSet<String>,

    // Chat composer state
    pub prompt_text: String,
    pub attached_files: Vec<String>,
    pub attached_media: Vec<harness_store::MediaAttachment>,
    pub selected_question_opts: HashSet<String>,
    pub question_custom_text: String,
    pub model_search: String,
    pub model_cat: String, // "all" | "local" | "google" | "cloud"
    pub show_model_popover: bool,
    pub attach_ws_filter: String,
    pub is_generating: bool,
    pub turn_started_at: Option<Instant>,

    // Editor & Terminal
    pub editor_tabs: Vec<EditorTab>,
    pub active_tab_path: Option<String>,
    pub editor_find_open: bool,
    pub editor_find_query: String,
    pub editor_replace_query: String,
    pub terminal_cmd: String,
    pub terminal_output: String,
    pub terminal_cwd: String,
    pub terminal_workspace_root: String,

    // Inspector & Diffs
    pub editing_artifact_id: Option<String>,
    pub artifact_edit_buf: String,
    pub diffs_view_mode: DiffsViewMode,
    pub git_diff_entries: Vec<GitDiffEntry>,
    pub git_diff_filter: Option<String>,
    pub git_diff_loaded_for_ws: String,
    pub git_diff_loading: bool,

    // Forge drafts
    pub vault_dir_draft: String,
    pub ctx_size_draft: String,
    pub ngl_draft: String,
    pub idle_mins_draft: String,
    pub hf_query: String,
    pub hf_results: Vec<HfModelSummary>,
    pub hf_repo_details: Option<HfRepoDetails>,
    pub key_gemini: String,
    pub key_anthropic: String,
    pub key_openrouter: String,
    pub key_oai_url: String,
    pub key_oai_key: String,
    pub key_hf: String,
    pub key_brave: String,
    pub verify_statuses: Vec<ProviderVerifyStatus>,
    pub quotas_filter: QuotasTabFilter,
    pub alias_target_model_id: String,
    pub alias_name_draft: String,
    pub custom_model_id: String,
    pub custom_model_name: String,

    // Plugins & Deps
    pub plugin_cat_filter: String, // "all" | "mcp" | "skill_pack" | "toolchain"
    pub dep_mgr_filter: Option<String>,
    pub last_dep_workspace: String,
    pub dep_search: String,
    pub custom_dep_input: String,
    pub agy_export_json: Option<String>,
    pub mcp_probes: HashMap<String, McpProbeResult>,

    // Customizations & Rules
    pub rules_draft: String,
    pub rules_dirty: bool,
    pub custom_sys_draft: String,
    pub kb_stats: Option<IndexStats>,
    pub kb_query: String,
    pub kb_hits: Vec<KnowledgeSearchHit>,
    pub show_new_skill_form: bool,
    pub skill_slug: String,
    pub skill_name: String,
    pub skill_desc: String,
    pub skill_prompt: String,
    pub skill_mode: AgentMode,
    pub mcp_name: String,
    pub mcp_transport: String,
    pub mcp_target: String,

    // Automations
    pub automations: Vec<AutomationPlaybook>,
    pub auto_index_on_workspace_switch: bool,
    pub show_new_automation_form: bool,
    pub auto_title_draft: String,
    pub auto_category_draft: String,
    pub auto_desc_draft: String,
    pub auto_prompt_draft: String,
    pub auto_mode_draft: AgentMode,
    pub last_automation_run: Option<(String, String)>,

    // Command palette & Modals
    pub cmd_query: String,
    pub cmd_selected_idx: usize,
    pub autocomplete_idx: usize,
    pub prompt_modal: PromptModal,

    // Toasts & Image Texture Cache
    pub toast: Option<(String, bool, Instant)>,
    pub textures: HashMap<String, egui::TextureHandle>,
}

impl GuiState {
    pub fn new() -> Self {
        Self {
            snap: None,
            show_sidebar: true,
            show_workbench: false,
            show_explorer: false,
            sidebar_width: 242.0,
            explorer_width: 256.0,
            show_terminal: false,
            terminal_height: 190.0,
            main_view: MainView::Chat,
            stage_tab: StageTab::Artifacts,
            settings_tab: SettingsTab::Appearance,
            show_settings_modal: false,
            settings_search: String::new(),
            show_cmd_palette: false,
            show_status_popover: false,
            show_downloads_popover: false,
            composer_focused: false,
            last_ctrl_down_at: None,
            ctrl_v_latch: false,
            dragging_explorer_item: None,
            nav_history: vec![(MainView::Chat, String::new())],
            nav_idx: 0,
            expanded_projects: HashSet::new(),
            expanded_project_all: HashSet::new(),
            did_init_projects: false,
            show_session_filter: false,
            session_filter: String::new(),
            explorer_filter: String::new(),
            explorer_expanded: HashSet::new(),
            prompt_text: String::new(),
            attached_files: Vec::new(),
            attached_media: Vec::new(),
            selected_question_opts: HashSet::new(),
            question_custom_text: String::new(),
            model_search: String::new(),
            model_cat: "all".into(),
            show_model_popover: false,
            attach_ws_filter: String::new(),
            is_generating: false,
            turn_started_at: None,
            editor_tabs: Vec::new(),
            active_tab_path: None,
            editor_find_open: false,
            editor_find_query: String::new(),
            editor_replace_query: String::new(),
            terminal_cmd: String::new(),
            terminal_output: "Ready. Run any command inside your active workspace.".into(),
            terminal_cwd: String::new(),
            terminal_workspace_root: String::new(),
            editing_artifact_id: None,
            artifact_edit_buf: String::new(),
            diffs_view_mode: DiffsViewMode::Checkpoints,
            git_diff_entries: Vec::new(),
            git_diff_filter: None,
            git_diff_loaded_for_ws: String::new(),
            git_diff_loading: false,
            vault_dir_draft: String::new(),
            ctx_size_draft: "16384".into(),
            ngl_draft: "99".into(),
            idle_mins_draft: "15".into(),
            hf_query: String::new(),
            hf_results: Vec::new(),
            hf_repo_details: None,
            key_gemini: String::new(),
            key_anthropic: String::new(),
            key_openrouter: String::new(),
            key_oai_url: String::new(),
            key_oai_key: String::new(),
            key_hf: String::new(),
            key_brave: String::new(),
            verify_statuses: Vec::new(),
            quotas_filter: QuotasTabFilter::Available,
            alias_target_model_id: String::new(),
            alias_name_draft: String::new(),
            custom_model_id: String::new(),
            custom_model_name: String::new(),
            plugin_cat_filter: "all".into(),
            dep_mgr_filter: None,
            last_dep_workspace: String::new(),
            dep_search: String::new(),
            custom_dep_input: String::new(),
            agy_export_json: None,
            mcp_probes: HashMap::new(),
            rules_draft: String::new(),
            rules_dirty: false,
            custom_sys_draft: String::new(),
            kb_stats: None,
            kb_query: String::new(),
            kb_hits: Vec::new(),
            show_new_skill_form: false,
            skill_slug: String::new(),
            skill_name: String::new(),
            skill_desc: String::new(),
            skill_prompt: String::new(),
            skill_mode: AgentMode::Auto,
            mcp_name: String::new(),
            mcp_transport: "stdio".into(),
            mcp_target: String::new(),
            automations: default_automation_playbooks(),
            auto_index_on_workspace_switch: true,
            show_new_automation_form: false,
            auto_title_draft: String::new(),
            auto_category_draft: "Uni Helper".into(),
            auto_desc_draft: String::new(),
            auto_prompt_draft: String::new(),
            auto_mode_draft: AgentMode::Agent,
            last_automation_run: None,
            cmd_query: String::new(),
            cmd_selected_idx: 0,
            autocomplete_idx: 0,
            prompt_modal: PromptModal::None,
            toast: None,
            textures: HashMap::new(),
        }
    }

    pub fn set_toast(&mut self, msg: impl Into<String>, is_error: bool) {
        self.toast = Some((msg.into(), is_error, Instant::now()));
    }

    pub fn close_transient_popovers(&mut self) {
        self.show_model_popover = false;
        self.show_status_popover = false;
        self.show_downloads_popover = false;
    }

    pub fn is_busy(&self) -> bool {
        self.is_generating
            || self
                .snap
                .as_ref()
                .map(|s| s.live_activity.active)
                .unwrap_or(false)
    }

    pub fn sync_turn_timer(&mut self) {
        if self.is_busy() {
            if self.turn_started_at.is_none() {
                self.turn_started_at = Some(Instant::now());
            }
        } else {
            self.turn_started_at = None;
        }
    }

    pub fn format_elapsed_since_turn_start(&self) -> Option<String> {
        let secs = self.turn_started_at?.elapsed().as_secs();
        if secs < 60 {
            Some(format!("{secs}s"))
        } else {
            Some(format!("{}m {:02}s", secs / 60, secs % 60))
        }
    }

    pub fn approve_active_plan_optimistically(&mut self, session_id: &str) {
        if let Some(ref mut snap_arc) = self.snap {
            let snap_mut = Arc::make_mut(snap_arc);
            if let Some(ref mut sess_mut) = snap_mut.active_session {
                if sess_mut.id == session_id {
                    if let Some(ref mut ap) = sess_mut.active_plan {
                        ap.status = PlanStatus::Approved;
                    }
                    for msg in &mut sess_mut.messages {
                        if let Some(ref mut pa) = msg.plan_artifact {
                            if pa.status == PlanStatus::AwaitingReview {
                                pa.status = PlanStatus::Approved;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn switch_main_view(&mut self, view: MainView, record_history: bool) {
        self.main_view = view;
        self.show_settings_modal = false;
        self.close_transient_popovers();
        if record_history {
            let sid = self
                .snap
                .as_ref()
                .map(|s| s.settings.active_session_id.clone())
                .unwrap_or_default();
            let should_push = self
                .nav_history
                .get(self.nav_idx.max(0) as usize)
                .map(|prev| prev.0 != view || prev.1 != sid)
                .unwrap_or(true);
            if should_push {
                let cut = (self.nav_idx + 1).max(0) as usize;
                self.nav_history.truncate(cut);
                self.nav_history.push((view, sid));
                if self.nav_history.len() > 30 {
                    self.nav_history.remove(0);
                }
                self.nav_idx = self.nav_history.len() as isize - 1;
            }
        }
    }

    pub fn open_stage_tab(&mut self, tab: StageTab) {
        self.main_view = MainView::Chat;
        self.show_settings_modal = false;
        self.close_transient_popovers();
        self.stage_tab = tab;
        self.show_workbench = true;
    }

    pub fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_search.clear();
        self.settings_tab = tab;
        self.show_settings_modal = true;
        self.close_transient_popovers();
    }

    pub fn navigate_history(&mut self, delta: isize, backend: &Arc<BackendHandle>) {
        let target = self.nav_idx + delta;
        if target < 0 || (target as usize) >= self.nav_history.len() {
            return;
        }
        self.nav_idx = target;
        let (view, sid) = self.nav_history[target as usize].clone();
        self.switch_main_view(view, false);
        if !sid.is_empty()
            && self
                .snap
                .as_ref()
                .map(|s| s.settings.active_session_id != sid)
                .unwrap_or(false)
        {
            backend.select_session(sid);
        }
    }

    pub fn sync_from_snapshot(&mut self, snap: &UiSnapshot) {
        self.vault_dir_draft = snap.settings.model_vault_dir.clone();
        self.ctx_size_draft = snap.settings.local_ctx_size.to_string();
        self.ngl_draft = snap.settings.local_gpu_layers.to_string();
        self.idle_mins_draft = snap.settings.idle_unload_minutes.to_string();
        if self.key_oai_url.is_empty() {
            self.key_oai_url = snap.settings.api_keys.openai_compat_base_url.clone();
        }
        if !self.rules_dirty {
            self.rules_draft = snap.workspace_rules.clone();
        }
        if self.custom_sys_draft.is_empty() {
            self.custom_sys_draft = snap.settings.ui.custom_system_prompt.clone();
        }
        if self.dep_mgr_filter.is_none()
            || self.last_dep_workspace != snap.settings.active_workspace_path
        {
            self.last_dep_workspace = snap.settings.active_workspace_path.clone();
            self.dep_mgr_filter = Some(
                snap.project_deps
                    .detected_ecosystems
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "cargo".into()),
            );
        }
        if self.terminal_workspace_root != snap.settings.active_workspace_path {
            self.terminal_workspace_root = snap.settings.active_workspace_path.clone();
            self.terminal_cwd = snap.settings.active_workspace_path.clone();
            self.git_diff_entries.clear();
            self.git_diff_loaded_for_ws.clear();
            self.git_diff_filter = None;
            self.git_diff_loading = false;
        }
        if !self.did_init_projects {
            self.expanded_projects
                .insert(norm_path(&snap.settings.active_workspace_path));
            self.did_init_projects = true;
        }
    }

    pub fn selectable_models(&self, respect_only_reachable: bool) -> Vec<SelectableModel> {
        let Some(snap) = self.snap.as_ref() else {
            return Vec::new();
        };
        let only_reach =
            respect_only_reachable && snap.settings.orchestrator.only_show_reachable_models;
        let mut out = Vec::new();

        for m in &snap.local_models {
            if m.is_embedding {
                continue;
            }
            let resolved_name = snap
                .settings
                .resolve_model_alias(&m.id, &m.name)
                .to_string();
            let vram_short = match m.vram_fit.tier {
                harness_models::FitTier::FitsInVram => "100% GPU".to_string(),
                harness_models::FitTier::PartialOffload => "Hybrid GPU".to_string(),
                harness_models::FitTier::CpuHeavy => "CPU Heavy".to_string(),
            };
            out.push(SelectableModel {
                id: m.id.clone(),
                display_name: resolved_name,
                canonical_name: m.name.clone(),
                badge: m.vram_fit.label.clone(),
                provider_label: "Local".into(),
                category: "local".into(),
                reachable: true,
                requires_payment: false,
                is_local: true,
                quota_pct: None,
                quota_badge: vram_short,
                quota_tooltip: format!(
                    "{} · {} (est. {} MB)",
                    m.vram_fit.label,
                    fmt_bytes(m.size_bytes),
                    m.vram_fit.estimated_total_mb
                ),
            });
        }

        for m in &snap.cloud_catalog {
            if respect_only_reachable && m.requires_payment {
                continue;
            }
            if only_reach && !m.reachable {
                continue;
            }
            let (prov_label, cat) = match m.provider {
                ProviderKind::GoogleGemini => ("Gemini", "google"),
                ProviderKind::Anthropic => ("Claude", "cloud"),
                ProviderKind::OpenRouter => ("OpenRouter", "cloud"),
                ProviderKind::OpenAiCompat => ("Custom", "cloud"),
                _ => ("Cloud", "cloud"),
            };
            let resolved_name = snap
                .settings
                .resolve_model_alias(&m.id, &m.display_name)
                .to_string();
            let quota_snap = snap.cloud_quotas.iter().find(|q| q.model_id == m.id);
            let quota_pct = quota_snap.map(|q| q.remaining_daily_pct);
            let quota_badge = if m.requires_payment {
                "Paid".to_string()
            } else if !m.reachable {
                "Offline".to_string()
            } else if let Some(pct) = quota_pct {
                format!("{pct:.0}%")
            } else {
                "Ready".to_string()
            };
            let quota_tooltip = if let Some(q) = quota_snap {
                format!(
                    "ID: {}\nDaily quota: {:.0}% remaining\nToday: {} reqs · {} tok · RPM {}/{}",
                    m.id,
                    q.remaining_daily_pct,
                    q.rpd_used,
                    q.tokens_today,
                    q.rpm_used,
                    q.rpm_limit
                )
            } else {
                format!("ID: {}\n{}", m.id, m.reachability_reason)
            };
            out.push(SelectableModel {
                id: m.id.clone(),
                display_name: resolved_name,
                canonical_name: m.display_name.clone(),
                badge: if m.requires_payment {
                    "Paid".into()
                } else if m.reachable {
                    "Ready".into()
                } else {
                    "Unavailable".into()
                },
                provider_label: prov_label.into(),
                category: cat.into(),
                reachable: m.reachable,
                requires_payment: m.requires_payment,
                is_local: false,
                quota_pct,
                quota_badge,
                quota_tooltip,
            });
        }
        out
    }

    pub fn get_or_load_texture(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        bytes: &[u8],
    ) -> Option<egui::TextureHandle> {
        if let Some(h) = self.textures.get(key) {
            return Some(h.clone());
        }
        let img = image::load_from_memory(bytes).ok()?;
        let (w, h) = img.dimensions();
        let rgba = img.to_rgba8();
        let color_img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
        let handle = ctx.load_texture(key, color_img, egui::TextureOptions::LINEAR);
        self.textures.insert(key.to_string(), handle.clone());
        Some(handle)
    }

    fn push_rgba_image_attachment(&mut self, w: u32, h: u32, rgba: Vec<u8>, name_override: Option<String>) -> bool {
        use image::ImageEncoder;
        if w == 0 || h == 0 {
            return false;
        }
        let Some(rgba_buf) = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w, h, rgba) else {
            return false;
        };
        let mut png_bytes = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        if encoder
            .write_image(
                rgba_buf.as_raw(),
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .is_err()
            || png_bytes.is_empty()
        {
            return false;
        }
        let size_bytes = png_bytes.len() as u64;
        let name = name_override.unwrap_or_else(|| {
            format!("screenshot-{}.png", chrono::Local::now().format("%H%M%S"))
        });
        let b64 = encode_base64(&png_bytes);
        self.attached_media.push(harness_store::MediaAttachment {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            mime_type: "image/png".into(),
            data_base64: Some(b64),
            text_content: None,
            size_bytes,
        });
        self.set_toast(
            format!("Pasted screenshot {}×{} ({}) into chat.", w, h, fmt_bytes(size_bytes)),
            false,
        );
        true
    }

    pub fn attach_clipboard_image(&mut self) -> bool {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(img) = cb.get_image() {
                if self.push_rgba_image_attachment(
                    img.width as u32,
                    img.height as u32,
                    img.bytes.into_owned(),
                    None,
                ) {
                    return true;
                }
            }
            #[cfg(windows)]
            if let Some((w, h, rgba)) = try_read_win32_cf_dib() {
                if self.push_rgba_image_attachment(w, h, rgba, None) {
                    return true;
                }
            }
            if let Ok(files) = cb.get().file_list() {
                if !files.is_empty() {
                    self.attach_external_paths(&files);
                    return true;
                }
            }
        } else {
            #[cfg(windows)]
            if let Some((w, h, rgba)) = try_read_win32_cf_dib() {
                if self.push_rgba_image_attachment(w, h, rgba, None) {
                    return true;
                }
            }
        }
        false
    }

    pub fn attach_dropped_bytes(&mut self, name: String, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let size_bytes = bytes.len() as u64;
        let lower_name = name.to_ascii_lowercase();

        if let Ok(fmt) = image::guess_format(bytes) {
            let direct_mime = match fmt {
                image::ImageFormat::Png => Some("image/png"),
                image::ImageFormat::Jpeg => Some("image/jpeg"),
                image::ImageFormat::WebP => Some("image/webp"),
                image::ImageFormat::Gif => Some("image/gif"),
                _ => None,
            };
            if let Some(mime) = direct_mime {
                if !self
                    .attached_media
                    .iter()
                    .any(|m| m.name == name && m.size_bytes == size_bytes)
                {
                    self.attached_media.push(harness_store::MediaAttachment {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: name.clone(),
                        mime_type: mime.into(),
                        data_base64: Some(encode_base64(bytes)),
                        text_content: None,
                        size_bytes,
                    });
                    self.set_toast(format!("Attached image {name} for Vision."), false);
                }
                return;
            }
            if let Ok(dyn_img) = image::load_from_memory(bytes) {
                let rgba = dyn_img.to_rgba8();
                let (w, h) = rgba.dimensions();
                if self.push_rgba_image_attachment(w, h, rgba.into_raw(), Some(format!("{name}.png"))) {
                    return;
                }
            }
        }

        if lower_name.ends_with(".pdf") {
            let tmp = std::env::temp_dir()
                .join(format!("cinder_drop_{}.pdf", uuid::Uuid::new_v4().simple()));
            let extracted = if std::fs::write(&tmp, bytes).is_ok() {
                let res = harness_tools::extract_pdf_file_text(&tmp, 20).ok();
                let _ = std::fs::remove_file(&tmp);
                res
            } else {
                None
            };
            let text_content = extracted.unwrap_or_else(|| {
                format!("[PDF file attached: {name} ({}) — text extraction yielded no selectable text]", fmt_bytes(size_bytes))
            });
            self.attached_media.push(harness_store::MediaAttachment {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                mime_type: "application/pdf".into(),
                data_base64: None,
                text_content: Some(text_content),
                size_bytes,
            });
            self.set_toast(format!("Attached PDF {name}."), false);
            return;
        }

        if let Some(txt) = decode_dropped_text(bytes) {
            let capped: String = txt.chars().take(24_000).collect();
            self.attached_media.push(harness_store::MediaAttachment {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                mime_type: "text/plain".into(),
                data_base64: None,
                text_content: Some(capped),
                size_bytes,
            });
            self.set_toast(format!("Attached file {name}."), false);
        } else {
            self.attached_media.push(harness_store::MediaAttachment {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                mime_type: "application/octet-stream".into(),
                data_base64: None,
                text_content: Some(format!("[Binary file attached: {name} ({})]", fmt_bytes(size_bytes))),
                size_bytes,
            });
            self.set_toast(format!("Attached binary file {name}."), false);
        }
    }

    pub fn attach_external_paths(&mut self, paths: &[PathBuf]) -> Option<String> {
        let ws_root: Option<PathBuf> = self
            .snap
            .as_ref()
            .map(|s| s.settings.active_workspace_path.trim())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);
        let mut external_dir_to_open: Option<String> = None;

        for path in paths {
            if path.is_dir() {
                if let Some(ref root) = ws_root {
                    if let Some(rel) = harness_tools::relative_to_workspace(root, path) {
                        if !rel.is_empty() && !self.attached_files.iter().any(|f| f == &rel) {
                            self.attached_files.push(rel.clone());
                            self.set_toast(format!("Attached folder {rel} to chat."), false);
                        }
                        continue;
                    }
                }
                let dir_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("folder");
                external_dir_to_open = Some(path.to_string_lossy().to_string());
                self.set_toast(format!("Opening folder {dir_name} as active project..."), false);
                continue;
            }
            if !path.is_file() {
                continue;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("attachment")
                .to_string();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let is_img_ext = matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "jfif"
            );

            if let Some(ref root) = ws_root {
                if let Some(rel) = harness_tools::relative_to_workspace(root, path) {
                    if !rel.is_empty() {
                        if !self.attached_files.iter().any(|f| f == &rel) {
                            self.attached_files.push(rel.clone());
                        }
                        if !is_img_ext {
                            self.set_toast(format!("Attached workspace file {rel}."), false);
                            continue;
                        }
                    }
                }
            }

            let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
                if let Ok(bytes) = std::fs::read(path) {
                    if !self
                        .attached_media
                        .iter()
                        .any(|m| m.name == name && m.size_bytes == size_bytes)
                    {
                        let b64 = encode_base64(&bytes);
                        let mime = match ext.as_str() {
                            "png" => "image/png",
                            "webp" => "image/webp",
                            "gif" => "image/gif",
                            _ => "image/jpeg",
                        };
                        self.attached_media.push(harness_store::MediaAttachment {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: name.clone(),
                            mime_type: mime.into(),
                            data_base64: Some(b64),
                            text_content: None,
                            size_bytes,
                        });
                        self.set_toast(format!("Attached image {name} for Vision."), false);
                    }
                }
            } else if ext == "pdf" {
                let text_content = match harness_tools::extract_pdf_file_text(path, 20) {
                    Ok(txt) if !txt.trim().is_empty() => txt,
                    _ => format!(
                        "[PDF file attached: {name} ({}) — no selectable text extracted]",
                        fmt_bytes(size_bytes)
                    ),
                };
                self.attached_media.push(harness_store::MediaAttachment {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.clone(),
                    mime_type: "application/pdf".into(),
                    data_base64: None,
                    text_content: Some(text_content),
                    size_bytes,
                });
                self.set_toast(format!("Attached PDF {name}."), false);
            } else if let Ok(bytes) = std::fs::read(path) {
                if is_img_ext || image::guess_format(&bytes).is_ok() {
                    if let Ok(dyn_img) = image::load_from_memory(&bytes) {
                        let rgba = dyn_img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        let out_name = if ext == "png" {
                            name.clone()
                        } else {
                            format!("{name}.png")
                        };
                        if self.push_rgba_image_attachment(w, h, rgba.into_raw(), Some(out_name)) {
                            continue;
                        }
                    }
                }
                if let Some(txt) = decode_dropped_text(&bytes) {
                    let capped: String = txt.chars().take(24_000).collect();
                    self.attached_media.push(harness_store::MediaAttachment {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: name.clone(),
                        mime_type: "text/plain".into(),
                        data_base64: None,
                        text_content: Some(capped),
                        size_bytes,
                    });
                    self.set_toast(format!("Attached file {name}."), false);
                } else {
                    self.attached_media.push(harness_store::MediaAttachment {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: name.clone(),
                        mime_type: "application/octet-stream".into(),
                        data_base64: None,
                        text_content: Some(format!(
                            "[Binary file attached: {name} ({})]",
                            fmt_bytes(size_bytes)
                        )),
                        size_bytes,
                    });
                    self.set_toast(format!("Attached binary file {name}."), false);
                }
            }
        }
        external_dir_to_open
    }
}

pub fn resolve_model_display_name(snap: &UiSnapshot, model_id: &str) -> String {
    let trimmed = model_id.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        return "Auto".to_string();
    }
    if let Some(alias) = snap
        .settings
        .model_aliases
        .get(trimmed)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return alias.to_string();
    }
    if let Some(loc) = snap.local_models.iter().find(|m| m.id == trimmed) {
        return loc.name.clone();
    }
    if let Some(cloud) = snap.cloud_catalog.iter().find(|m| m.id == trimmed) {
        return cloud.display_name.clone();
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
}

fn decode_dropped_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Some(
            String::from_utf16_lossy(&units)
                .trim_start_matches('\u{FEFF}')
                .to_string(),
        );
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return Some(
            String::from_utf16_lossy(&units)
                .trim_start_matches('\u{FEFF}')
                .to_string(),
        );
    }
    let slice = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(slice) {
        return Some(s.trim_start_matches('\u{FEFF}').to_string());
    }
    if slice.iter().take(512).filter(|&&b| b == 0).count() > 2 {
        return None;
    }
    Some(slice.iter().map(|&b| b as char).collect())
}

#[cfg(windows)]
fn try_read_win32_cf_dib() -> Option<(u32, u32, Vec<u8>)> {
    use image::ImageDecoder;
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(hwnd: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn IsClipboardFormatAvailable(format: u32) -> i32;
        fn GetClipboardData(u_format: u32) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalLock(hmem: *mut c_void) -> *const u8;
        fn GlobalUnlock(hmem: *mut c_void) -> i32;
        fn GlobalSize(hmem: *mut c_void) -> usize;
    }

    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;

    let mut opened = false;
    for _ in 0..4 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    if !opened {
        return None;
    }

    let mut raw: Option<Vec<u8>> = None;
    for fmt in [CF_DIB, CF_DIBV5] {
        if unsafe { IsClipboardFormatAvailable(fmt) } == 0 {
            continue;
        }
        let handle = unsafe { GetClipboardData(fmt) };
        if handle.is_null() {
            continue;
        }
        let size = unsafe { GlobalSize(handle) };
        if size > 0 {
            let ptr = unsafe { GlobalLock(handle) };
            if !ptr.is_null() {
                let slice = unsafe { std::slice::from_raw_parts(ptr, size) };
                raw = Some(slice.to_vec());
                unsafe {
                    GlobalUnlock(handle);
                }
                break;
            }
        }
    }
    unsafe {
        CloseClipboard();
    }

    let bytes = raw?;
    let decoder =
        image::codecs::bmp::BmpDecoder::new_without_file_header(std::io::Cursor::new(&bytes)).ok()?;
    let (w, h) = decoder.dimensions();
    let rgba = image::DynamicImage::from_decoder(decoder).ok()?.into_rgba8();
    Some((w, h, rgba.into_raw()))
}

pub fn norm_path(p: &str) -> String {
    p.replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

pub fn fmt_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn fmt_rel_age(dt: chrono::DateTime<chrono::Utc>) -> String {
    let mins = (chrono::Utc::now() - dt).num_minutes();
    if mins < 1 {
        "now".into()
    } else if mins < 60 {
        format!("{mins}m")
    } else if mins < 60 * 24 {
        format!("{}h", mins / 60)
    } else {
        format!("{}d", mins / (60 * 24))
    }
}

pub fn mode_label(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Auto => "Auto",
        AgentMode::Plan => "Plan",
        AgentMode::Agent => "Agent",
        AgentMode::Orchestrator => "Orchestrator",
        AgentMode::Chat => "Chat",
        AgentMode::Compare => "Compare",
    }
}

pub fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn decode_base64_simple(s: &str) -> Option<Vec<u8>> {
    harness_tools::decode_base64(s).ok()
}

/// Hierarchical folder tree node built from flat `WorkspaceFileNode`s.
#[derive(Debug, Clone)]
pub struct FsTreeNode {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub extension: String,
    pub children: BTreeMap<String, FsTreeNode>,
}

impl FsTreeNode {
    pub fn build_tree(nodes: &[WorkspaceFileNode]) -> Vec<FsTreeNode> {
        let mut root_map: BTreeMap<String, FsTreeNode> = BTreeMap::new();
        for n in nodes {
            let parts: Vec<&str> = n.rel_path.split('/').filter(|s| !s.is_empty()).collect();
            if parts.is_empty() {
                continue;
            }
            let mut cur = &mut root_map;
            for (idx, seg) in parts.iter().enumerate() {
                let is_last = idx + 1 == parts.len();
                let rel_here = parts[..=idx].join("/");
                let entry = cur.entry((*seg).to_string()).or_insert_with(|| FsTreeNode {
                    name: (*seg).to_string(),
                    rel_path: rel_here,
                    is_dir: if is_last { n.is_dir } else { true },
                    size_bytes: if is_last { n.size_bytes } else { 0 },
                    extension: if is_last {
                        n.extension.clone()
                    } else {
                        String::new()
                    },
                    children: BTreeMap::new(),
                });
                if is_last {
                    entry.is_dir = n.is_dir;
                    entry.size_bytes = n.size_bytes;
                    entry.extension = n.extension.clone();
                }
                cur = &mut entry.children;
            }
        }
        Self::sorted_vec(root_map)
    }

    pub fn sorted_children(&self) -> Vec<&FsTreeNode> {
        let mut items: Vec<&FsTreeNode> = self.children.values().collect();
        items.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
        });
        items
    }

    fn sorted_vec(map: BTreeMap<String, FsTreeNode>) -> Vec<FsTreeNode> {
        let mut items: Vec<FsTreeNode> = map.into_values().collect();
        items.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
        });
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_sorts_filesystem_tree_dirs_first() {
        let flat = vec![
            WorkspaceFileNode {
                name: "z.rs".into(),
                rel_path: "z.rs".into(),
                is_dir: false,
                size_bytes: 10,
                extension: "rs".into(),
            },
            WorkspaceFileNode {
                name: "a.rs".into(),
                rel_path: "src/a.rs".into(),
                is_dir: false,
                size_bytes: 20,
                extension: "rs".into(),
            },
            WorkspaceFileNode {
                name: "src".into(),
                rel_path: "src".into(),
                is_dir: true,
                size_bytes: 0,
                extension: String::new(),
            },
        ];
        let tree = FsTreeNode::build_tree(&flat);
        assert_eq!(tree.len(), 2);
        assert!(tree[0].is_dir);
        assert_eq!(tree[0].name, "src");
        let children = tree[0].sorted_children();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "a.rs");
        assert_eq!(tree[1].name, "z.rs");
    }

    #[test]
    fn default_playbooks_are_unique_and_nonempty() {
        let state = GuiState::new();
        assert_eq!(state.automations.len(), 6);
        assert!(state.auto_index_on_workspace_switch);
        let mut ids = HashSet::new();
        for pb in &state.automations {
            assert!(!pb.title.trim().is_empty());
            assert!(!pb.prompt.trim().is_empty());
            assert!(ids.insert(pb.id.clone()), "duplicate id {}", pb.id);
        }
    }

    #[test]
    fn attach_external_paths_handles_images_utf16_and_folders() {
        let dir = std::env::temp_dir().join(format!("cinder_dnd_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        // 1x1 PNG bytes via image encoder
        let mut img_bytes = Vec::new();
        {
            use image::ImageEncoder;
            let enc = image::codecs::png::PngEncoder::new(&mut img_bytes);
            enc.write_image(&[255, 110, 48, 255], 1, 1, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        let img_path = dir.join("shot.png");
        std::fs::write(&img_path, &img_bytes).unwrap();

        // UTF-16LE BOM text file
        let mut utf16_bytes = vec![0xFFu8, 0xFEu8];
        for cu in "Hello Cinder".encode_utf16() {
            utf16_bytes.extend_from_slice(&cu.to_le_bytes());
        }
        let note_path = dir.join("note.txt");
        std::fs::write(&note_path, &utf16_bytes).unwrap();

        // Subfolder outside workspace -> returns external folder path
        let sub_dir = dir.join("dropped_repo");
        std::fs::create_dir_all(&sub_dir).unwrap();

        let mut st = GuiState::new();
        let open_dir = st.attach_external_paths(&[img_path, note_path, sub_dir.clone()]);
        assert_eq!(
            open_dir.as_deref(),
            Some(sub_dir.to_string_lossy().as_ref())
        );
        assert_eq!(st.attached_media.len(), 2);
        assert_eq!(st.attached_media[0].mime_type, "image/png");
        assert_eq!(
            st.attached_media[1].text_content.as_deref(),
            Some("Hello Cinder")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn close_transient_popovers_and_is_busy_consolidate_ui_state() {
        let mut st = GuiState::new();
        st.show_model_popover = true;
        st.show_status_popover = true;
        st.show_downloads_popover = true;
        st.close_transient_popovers();
        assert!(!st.show_model_popover);
        assert!(!st.show_status_popover);
        assert!(!st.show_downloads_popover);

        assert!(!st.is_busy());
        st.is_generating = true;
        assert!(st.is_busy());
    }
}
