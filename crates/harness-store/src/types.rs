use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use harness_net::PrivacyMode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaTier {
    GoogleAiPro,
    FreeTier,
    Custom,
}

impl Default for QuotaTier {
    fn default() -> Self {
        Self::GoogleAiPro
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    LocalLlama,
    LocalOllama,
    GoogleGemini,
    Anthropic,
    OpenRouter,
    OpenAiCompat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelQuotaLimit {
    pub rpm_limit: u64,
    pub tpm_limit: u64,
    pub rpd_limit: u64,
    pub daily_token_limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub provider: ProviderKind,
    pub context_window: u64,
    pub supports_thinking: bool,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub tier_note: String,
    pub quota_limit: Option<ModelQuotaLimit>,
    #[serde(default)]
    pub reachable: bool,
    #[serde(default)]
    pub reachability_reason: String,
    #[serde(default)]
    pub antigravity_alias: Option<String>,
    #[serde(default)]
    pub requires_payment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEvent {
    pub timestamp: DateTime<Utc>,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsageState {
    pub model_id: String,
    pub date: NaiveDate,
    pub rpd_used: u64,
    pub tokens_today: u64,
    pub prompt_tokens_today: u64,
    pub completion_tokens_today: u64,
    pub thinking_tokens_today: u64,
    pub recent_events: VecDeque<TokenEvent>,
    pub provider_remaining_requests: Option<u64>,
    pub provider_limit_requests: Option<u64>,
    pub provider_remaining_tokens: Option<u64>,
    pub provider_limit_tokens: Option<u64>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl ModelUsageState {
    pub fn new(model_id: &str, today: NaiveDate) -> Self {
        Self {
            model_id: model_id.to_string(),
            date: today,
            rpd_used: 0,
            tokens_today: 0,
            prompt_tokens_today: 0,
            completion_tokens_today: 0,
            thinking_tokens_today: 0,
            recent_events: VecDeque::new(),
            provider_remaining_requests: None,
            provider_limit_requests: None,
            provider_remaining_tokens: None,
            provider_limit_tokens: None,
            last_used_at: None,
        }
    }

    pub fn roll_day_if_needed(&mut self, today: NaiveDate, now: DateTime<Utc>) {
        if self.date != today {
            self.date = today;
            self.rpd_used = 0;
            self.tokens_today = 0;
            self.prompt_tokens_today = 0;
            self.completion_tokens_today = 0;
            self.thinking_tokens_today = 0;
            self.provider_remaining_requests = None;
            self.provider_remaining_tokens = None;
        }
        let cutoff = now - ChronoDuration::seconds(60);
        while let Some(front) = self.recent_events.front() {
            if front.timestamp < cutoff {
                self.recent_events.pop_front();
            } else {
                break;
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelQuotaSnapshot {
    pub model_id: String,
    pub display_name: String,
    pub provider: ProviderKind,
    pub context_window: u64,
    pub supports_thinking: bool,
    pub supports_tools: bool,
    pub remaining_daily_pct: f64,
    pub rpm_used: u64,
    pub rpm_limit: u64,
    pub tpm_used: u64,
    pub tpm_limit: u64,
    pub rpd_used: u64,
    pub rpd_limit: u64,
    pub tokens_today: u64,
    pub daily_token_limit: u64,
    pub prompt_tokens_today: u64,
    pub completion_tokens_today: u64,
    pub thinking_tokens_today: u64,
    pub reset_in_seconds: i64,
    pub header_source: bool,
    #[serde(default)]
    pub reachable: bool,
    #[serde(default)]
    pub reachability_reason: String,
    #[serde(default)]
    pub antigravity_alias: Option<String>,
    #[serde(default)]
    pub requires_payment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeys {
    pub gemini_api_key: String,
    pub anthropic_api_key: String,
    pub openrouter_api_key: String,
    pub openai_compat_base_url: String,
    pub openai_compat_api_key: String,
    pub huggingface_token: String,
    pub brave_search_api_key: String,
}

impl Default for ApiKeys {
    fn default() -> Self {
        Self {
            gemini_api_key: String::new(),
            anthropic_api_key: String::new(),
            openrouter_api_key: String::new(),
            openai_compat_base_url: String::new(),
            openai_compat_api_key: String::new(),
            huggingface_token: String::new(),
            brave_search_api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_uni_project: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    Auto,
    Chat,
    Plan,
    Agent,
    Orchestrator,
    Compare,
}

impl Default for AgentMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    SafeReadOnly,
    WorkspaceMutation,
    HighRiskDestructive,
}

impl Default for RiskTier {
    fn default() -> Self {
        Self::WorkspaceMutation
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub step_number: usize,
    pub description: String,
    pub files_touched: Vec<String>,
    pub verification: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    AwaitingReview,
    Approved,
    ChangesRequested,
    Executed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanArtifact {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub steps: Vec<PlanStep>,
    pub markdown: String,
    pub status: PlanStatus,
    pub model_id: String,
    #[serde(default)]
    pub executor_model_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    #[default]
    Ask,
    AutoChain,
    Strict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingModelFallback {
    pub id: String,
    pub failed_model_id: String,
    pub error_summary: String,
    pub reason: String,
    pub candidates: Vec<String>,
    pub mode: AgentMode,
    pub agent_history_json: String,
    pub step_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUserQuestion {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
    pub allow_multiple: bool,
    pub model_id: String,
    pub agent_history_json: String,
    pub step_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeStatus {
    PendingReview,
    Accepted,
    Reverted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBackup {
    pub rel_path: String,
    pub existed_before: bool,
    pub previous_content: Option<String>,
    pub new_content: Option<String>,
    pub diff: String,
    pub status: FileChangeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCheckpoint {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub files: Vec<FileBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt_template: String,
    pub recommended_mode: AgentMode,
    pub is_builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveActivityStatus {
    pub active: bool,
    pub session_id: Option<String>,
    pub phase: String,
    pub detail: String,
    pub step: usize,
    pub max_steps: usize,
    #[serde(default)]
    pub partial_content: String,
    #[serde(default)]
    pub partial_thinking: String,
    #[serde(default)]
    pub base_thinking: String,
    #[serde(default)]
    pub active_model: String,
    #[serde(default)]
    pub completed_tools: Vec<ToolCallRecord>,
    #[serde(default)]
    pub live_tool_name: String,
    #[serde(default)]
    pub live_tool_output: String,
    pub updated_at: DateTime<Utc>,
}

impl Default for LiveActivityStatus {
    fn default() -> Self {
        Self {
            active: false,
            session_id: None,
            phase: "idle".into(),
            detail: String::new(),
            step: 0,
            max_steps: 0,
            partial_content: String::new(),
            partial_thinking: String::new(),
            base_thinking: String::new(),
            active_model: String::new(),
            completed_tools: Vec::new(),
            live_tool_name: String::new(),
            live_tool_output: String::new(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorPhaseRecord {
    pub phase: String,
    pub model_id: String,
    pub summary: String,
    pub latency_ms: u64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
    pub status: String,
    pub output: String,
    pub diff: Option<String>,
    pub file_path: Option<String>,
    #[serde(default)]
    pub risk_tier: Option<RiskTier>,
    #[serde(default)]
    pub subagent_summary: Option<String>,
    #[serde(default)]
    pub subagent_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
    pub data_base64: Option<String>,
    #[serde(default)]
    pub text_content: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifact {
    pub id: String,
    pub title: String,
    pub kind: String, // "task_list" | "plan" | "walkthrough" | "report" | "html_widget"
    pub content: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub model_id: Option<String>,
    pub latency_ms: Option<u64>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub compare_partner: Option<CompareResult>,
    #[serde(default)]
    pub checkpoint_id: Option<String>,
    #[serde(default)]
    pub plan_artifact: Option<PlanArtifact>,
    #[serde(default)]
    pub orchestrator_phases: Vec<OrchestratorPhaseRecord>,
    #[serde(default)]
    pub attached_files: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<MediaAttachment>,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub fallback_from_model_id: Option<String>,
    #[serde(default)]
    pub fallback_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareResult {
    pub model_id: String,
    pub content: String,
    pub thinking: Option<String>,
    pub latency_ms: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolApproval {
    pub id: String,
    pub tool_name: String,
    pub arguments_json: String,
    pub reason: String,
    pub diff_preview: Option<String>,
    pub target_path: Option<String>,
    pub model_id: String,
    pub agent_history_json: String,
    pub step_index: usize,
    #[serde(default)]
    pub risk_tier: RiskTier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub workspace_path: String,
    pub mode: AgentMode,
    pub messages: Vec<ChatMessage>,
    pub pending_approval: Option<PendingToolApproval>,
    #[serde(default)]
    pub active_plan: Option<PlanArtifact>,
    #[serde(default)]
    pub pending_question: Option<PendingUserQuestion>,
    #[serde(default)]
    pub pending_fallback: Option<PendingModelFallback>,
    #[serde(default)]
    pub checkpoints: Vec<TurnCheckpoint>,
    #[serde(default)]
    pub artifacts: Vec<SessionArtifact>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_enabled_modules() -> Vec<String> {
    vec![
        "fs_read".into(),
        "fs_write".into(),
        "shell".into(),
        "rag".into(),
        "subagents".into(),
        "web".into(),
    ]
}

fn default_thinking_effort() -> String {
    "standard".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    pub id: String,
    pub name: String,
    pub transport: String,
    /// Display string: the command line (stdio) or URL (http). Kept for older state files.
    pub target: String,
    pub enabled: bool,
    /// Executable for stdio servers (resolved at install time).
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Remote Streamable-HTTP endpoint for http servers.
    #[serde(default)]
    pub url: String,
    /// "catalog" (curated one-click install), "antigravity" (imported), or "manual".
    #[serde(default = "default_mcp_source")]
    pub source: String,
    /// Server reaches the internet itself; blocked while Strict Local mode is on.
    #[serde(default)]
    pub needs_network: bool,
    /// Every tool on this server is read-only, so calls never need approval.
    #[serde(default)]
    pub read_only: bool,
}

fn default_mcp_source() -> String {
    "manual".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiCustomization {
    pub theme: String,
    pub accent_color: String,
    pub font_scale: String,
    pub show_right_workbench: bool,
    pub wrap_code_blocks: bool,
    pub show_token_metrics: bool,
    #[serde(default)]
    pub unload_after_turn: bool,
    pub custom_system_prompt: String,
    #[serde(default = "default_enabled_modules")]
    pub enabled_modules: Vec<String>,
    #[serde(default = "default_thinking_effort")]
    pub thinking_effort: String,
}

impl Default for UiCustomization {
    fn default() -> Self {
        Self {
            theme: "cinder_forge".into(),
            accent_color: "cinder".into(),
            font_scale: "default".into(),
            show_right_workbench: false,
            wrap_code_blocks: true,
            show_token_metrics: true,
            unload_after_turn: false,
            custom_system_prompt: String::new(),
            enabled_modules: default_enabled_modules(),
            thinking_effort: default_thinking_effort(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrchestratorConfig {
    pub planner_model_id: String,
    pub executor_model_id: String,
    #[serde(default)]
    pub vision_model_id: String,
    pub review_before_execute: bool,
    pub auto_verify_after_edits: bool,
    pub max_autonomous_steps: usize,
    pub only_show_reachable_models: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            planner_model_id: "local::gemma4:12b".into(),
            executor_model_id: "local::qwen3.5:9b".into(),
            vision_model_id: String::new(),
            review_before_execute: false,
            auto_verify_after_edits: true,
            max_autonomous_steps: 8,
            only_show_reachable_models: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub privacy_mode: PrivacyMode,
    pub model_vault_dir: String,
    pub quota_tier: QuotaTier,
    pub active_model_id: String,
    pub compare_model_b_id: String,
    pub active_workspace_path: String,
    pub active_session_id: String,
    pub auto_approve_edits: bool,
    pub auto_approve_commands: bool,
    pub idle_unload_minutes: u64,
    pub local_ctx_size: u32,
    pub local_gpu_layers: i32,
    pub expose_local_api: bool,
    pub local_server_port: u16,
    pub api_keys: ApiKeys,
    #[serde(default)]
    pub ui: UiCustomization,
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    /// Automatically retry on another reachable model when one is rate-limited,
    /// out of quota, overloaded, or unavailable.
    #[serde(default = "default_true")]
    pub auto_switch_models: bool,
    /// Allow automatic switching to fall back to local GGUF / Ollama models
    /// (off by default because it loads the GPU without asking).
    #[serde(default)]
    pub fallback_to_local: bool,
    /// Controls fallback behavior on model failure / rate limits:
    /// Ask before switching (default), Auto-switch along user's chain, or Strict (never switch).
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,
    /// User-ordered fallback model IDs tried in order when a primary model fails.
    #[serde(default)]
    pub fallback_chain: Vec<String>,
    /// User-assigned custom display aliases mapped by model ID.
    #[serde(default)]
    pub model_aliases: std::collections::BTreeMap<String, String>,
}

impl AppSettings {
    /// Returns the user-assigned alias for `model_id` if present and non-empty,
    /// otherwise returns `fallback`.
    pub fn resolve_model_alias<'a>(&'a self, model_id: &str, fallback: &'a str) -> &'a str {
        self.model_aliases
            .get(model_id)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback)
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    pub settings: AppSettings,
    pub workspaces: Vec<WorkspaceEntry>,
    pub sessions: Vec<Session>,
    pub usage: HashMap<String, ModelUsageState>,
    pub custom_quotas: HashMap<String, ModelQuotaLimit>,
    pub synced_google_models: Vec<ModelCatalogEntry>,
    #[serde(default)]
    pub verified_provider_models: HashMap<String, Vec<ModelCatalogEntry>>,
    #[serde(default)]
    pub blocked_models: HashMap<String, String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerEntry>,
    #[serde(default)]
    pub custom_models: Vec<ModelCatalogEntry>,
}
