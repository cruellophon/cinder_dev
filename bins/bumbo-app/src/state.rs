use axum::{http::StatusCode, Json};
use harness_models::{
    probe_hardware, scan_local_models, DownloadManager, EngineStatus, HardwareReport,
    LocalModelEntry, ServerManager,
};
use harness_net::NetworkGate;
use harness_store::BumboStore;
use harness_tools::{
    list_workspace_tree, probe_toolchains, InstallJobManager, ToolchainStatus, WorkspaceFileNode,
};
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub const STRICT_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; frame-src 'self' blob:;";

#[derive(Default)]
pub struct AppCache {
    hardware: RwLock<Option<(Instant, HardwareReport)>>,
    engine_status: RwLock<Option<(Instant, EngineStatus)>>,
    local_models: RwLock<Option<(Instant, String, u64, u32, Vec<LocalModelEntry>)>>,
    workspace_tree: RwLock<Option<(Instant, String, Vec<WorkspaceFileNode>)>>,
    toolchains: RwLock<Option<(Instant, Vec<ToolchainStatus>)>>,
}

#[derive(Clone)]
pub struct AppState {
    pub gate: Arc<NetworkGate>,
    pub store: Arc<BumboStore>,
    pub server_mgr: Arc<ServerManager>,
    pub download_mgr: Arc<DownloadManager>,
    pub install_jobs: InstallJobManager,
    pub cache: Arc<AppCache>,
}

impl AppState {
    pub async fn get_hardware(&self, force: bool) -> HardwareReport {
        if !force {
            let r = self.cache.hardware.read().await;
            if let Some((at, ref hw)) = *r {
                if at.elapsed() < Duration::from_secs(30) {
                    return hw.clone();
                }
            }
        }
        let hw = tokio::task::spawn_blocking(probe_hardware)
            .await
            .unwrap_or_else(|_| probe_hardware());
        let mut w = self.cache.hardware.write().await;
        *w = Some((Instant::now(), hw.clone()));
        hw
    }

    pub async fn get_engine_status(&self, force: bool) -> EngineStatus {
        if !force {
            let r = self.cache.engine_status.read().await;
            if let Some((at, ref es)) = *r {
                if at.elapsed() < Duration::from_secs(15) {
                    return es.clone();
                }
            }
        }
        let es = self.server_mgr.engine_status(&self.gate).await;
        let mut w = self.cache.engine_status.write().await;
        *w = Some((Instant::now(), es.clone()));
        es
    }

    pub async fn get_local_models(
        &self,
        vault_dir: &str,
        vram_mb: u64,
        ctx_size: u32,
        force: bool,
    ) -> Vec<LocalModelEntry> {
        if !force {
            let r = self.cache.local_models.read().await;
            if let Some((at, ref cached_vault, cached_vram, cached_ctx, ref list)) = *r {
                if at.elapsed() < Duration::from_secs(10)
                    && cached_vault == vault_dir
                    && cached_vram == vram_mb
                    && cached_ctx == ctx_size
                {
                    return list.clone();
                }
            }
        }
        let vd = vault_dir.to_string();
        let list = tokio::task::spawn_blocking(move || scan_local_models(Path::new(&vd), vram_mb, ctx_size))
            .await
            .unwrap_or_default();
        let mut w = self.cache.local_models.write().await;
        *w = Some((
            Instant::now(),
            vault_dir.to_string(),
            vram_mb,
            ctx_size,
            list.clone(),
        ));
        list
    }

    pub async fn get_workspace_tree(&self, ws_root: &str, force: bool) -> Vec<WorkspaceFileNode> {
        if !force {
            let r = self.cache.workspace_tree.read().await;
            if let Some((at, ref cached_ws, ref tree)) = *r {
                if at.elapsed() < Duration::from_secs(4) && cached_ws == ws_root {
                    return tree.clone();
                }
            }
        }
        let ws = ws_root.to_string();
        let tree = tokio::task::spawn_blocking(move || {
            let p = Path::new(&ws);
            if p.exists() {
                list_workspace_tree(p, 400)
            } else {
                Vec::new()
            }
        })
        .await
        .unwrap_or_default();
        let mut w = self.cache.workspace_tree.write().await;
        *w = Some((Instant::now(), ws_root.to_string(), tree.clone()));
        tree
    }

    pub async fn get_toolchains(&self, force: bool) -> Vec<ToolchainStatus> {
        if !force {
            let r = self.cache.toolchains.read().await;
            if let Some((at, ref list)) = *r {
                if at.elapsed() < Duration::from_secs(30) {
                    return list.clone();
                }
            }
        }
        let paths = self.store.paths().clone();
        let list = tokio::task::spawn_blocking(move || probe_toolchains(&paths))
            .await
            .unwrap_or_default();
        let mut w = self.cache.toolchains.write().await;
        *w = Some((Instant::now(), list.clone()));
        list
    }

    pub async fn invalidate_cache(&self) {
        *self.cache.hardware.write().await = None;
        *self.cache.engine_status.write().await = None;
        *self.cache.local_models.write().await = None;
        *self.cache.workspace_tree.write().await = None;
        *self.cache.toolchains.write().await = None;
    }

    pub async fn invalidate_toolchains_cache(&self) {
        *self.cache.toolchains.write().await = None;
    }

    pub async fn invalidate_workspace_cache(&self) {
        *self.cache.workspace_tree.write().await = None;
    }
}

#[derive(Serialize)]
pub struct SafeKeyFlags {
    pub has_gemini_api_key: bool,
    pub has_anthropic_api_key: bool,
    pub has_openrouter_api_key: bool,
    pub openai_compat_base_url: String,
    pub has_openai_compat_api_key: bool,
    pub has_huggingface_token: bool,
    pub has_brave_search_api_key: bool,
}

pub fn api_err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "error": msg.into() })))
}

pub fn clean_windows_verbatim(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

pub fn encode_uri_component(input: &str) -> String {
    let mut out = String::new();
    for b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}
