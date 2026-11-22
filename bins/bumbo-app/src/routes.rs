use crate::state::{api_err, clean_windows_verbatim, encode_uri_component, AppState, SafeKeyFlags};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path as AxumPath, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use harness_core::{
    accept_checkpoint_file, answer_pending_question, compact_session_context,
    delete_session_artifact, handle_plan_review_action, list_skills_with_global,
    load_workspace_rules, resolve_pending_approval, revert_checkpoint_file,
    rewind_session_to_message, rollback_entire_checkpoint, run_turn, save_custom_skill,
    save_workspace_rules, upsert_session_artifact, HarnessContext,
};
use harness_models::{inspect_hf_repo, search_hf_gguf};
use harness_net::PrivacyMode;
use harness_providers::{
    probe_gemini_model_live, sync_google_models, verify_and_discover_provider_models,
};
use harness_store::{
    provider_key_name, AgentMode, McpServerEntry, MediaAttachment, ModelCatalogEntry,
    ModelQuotaLimit, OrchestratorConfig, PlanStatus, ProviderKind, QuotaTier, Session,
    SkillDefinition, UiCustomization, WorkspaceEntry,
};
use harness_tools::{
    build_workspace_index, decode_base64, export_antigravity_mcp_json, extract_pdf_file_text,
    inspect_project_dependencies, install_curated_plugin, install_project_dependency,
    is_dependency_allowlisted, list_plugin_catalog_with_status, list_workspace_tree,
    probe_mcp_server, resolve_workspace_path, search_workspace_knowledge, sync_from_antigravity,
    uninstall_curated_plugin,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

const INDEX_HTML: &str = include_str!("../ui/index.html");
const STYLES_CSS: &str = include_str!("../ui/styles.css");
const APP_JS: &str = include_str!("../ui/app.js");
const CINDER_ICO: &[u8] = include_bytes!("../ui/cinder.ico");
const CINDER_ICON_SVG: &str = include_str!("../ui/cinder-icon.svg");
const MANIFEST_JSON: &str = include_str!("../ui/manifest.json");

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/styles.css", get(serve_css))
        .route("/app.js", get(serve_js))
        .route("/favicon.ico", get(serve_favicon))
        .route("/cinder.ico", get(serve_favicon))
        .route("/cinder-icon.svg", get(serve_icon_svg))
        .route("/manifest.json", get(serve_manifest))
        .route("/api/state", get(api_state))
        .route("/api/activity", get(api_activity))
        .route("/api/privacy", post(api_set_privacy))
        .route("/api/network/audit", get(api_network_audit))
        .route("/api/network/clear", post(api_network_clear))
        .route("/api/workspace/select", post(api_workspace_select))
        .route("/api/workspace/remove", post(api_workspace_remove))
        .route("/api/workspace/file", get(api_workspace_file))
        .route("/api/workspace/write_file", post(api_workspace_write_file))
        .route("/api/workspace/upload", post(api_workspace_upload))
        .route("/api/workspace/raw", get(api_workspace_raw))
        .route("/api/workspace/rules", post(api_workspace_rules))
        .route("/api/workspace/watch", get(api_workspace_watch))
        .route("/api/workspace/terminal", post(api_workspace_terminal))
        .route("/api/system/pick_folder", post(api_system_pick_folder))
        .route("/api/chat/send", post(api_chat_send))
        .route("/api/chat/abort", post(api_chat_abort))
        .route("/api/chat/rewind", post(api_chat_rewind))
        .route("/api/chat/compact", post(api_chat_compact))
        .route("/api/chat/compare", post(api_chat_compare))
        .route("/api/chat/approve_tool", post(api_chat_approve_tool))
        .route("/api/chat/answer_question", post(api_chat_answer_question))
        .route("/api/chat/plan_action", post(api_chat_plan_action))
        .route("/api/checkpoints/file_action", post(api_checkpoint_file_action))
        .route("/api/checkpoints/rollback", post(api_checkpoint_rollback))
        .route("/api/skills", get(api_skills_list))
        .route("/api/skills/save", post(api_skills_save))
        .route("/api/knowledge/index", post(api_knowledge_index))
        .route("/api/knowledge/search", post(api_knowledge_search))
        .route("/api/sessions/new", post(api_sessions_new))
        .route("/api/sessions/select", post(api_sessions_select))
        .route("/api/sessions/mode", post(api_sessions_mode))
        .route("/api/sessions/artifact", post(api_sessions_artifact))
        .route("/api/sessions/:id", delete(api_sessions_delete))
        .route("/api/settings/harness", post(api_settings_harness))
        .route("/api/settings/customization", post(api_settings_customization))
        .route("/api/models/presets", get(api_models_presets))
        .route("/api/models/hf/search", get(api_hf_search))
        .route("/api/models/hf/repo", get(api_hf_repo))
        .route("/api/models/hf/download", post(api_hf_download))
        .route("/api/models/hf/cancel", post(api_hf_cancel))
        .route("/api/models/downloads", get(api_downloads))
        .route("/api/models/vault/set_dir", post(api_set_vault_dir))
        .route("/api/models/engine/install", post(api_install_engine))
        .route("/api/models/server/start", post(api_start_server))
        .route("/api/models/server/stop", post(api_stop_server))
        .route("/api/providers/keys", post(api_save_keys))
        .route("/api/providers/verify", post(api_providers_verify))
        .route("/api/providers/sync_google", post(api_sync_google))
        .route("/api/providers/custom_model", post(api_providers_custom_model))
        .route("/api/providers/tier", post(api_set_tier))
        .route("/api/mcp/save", post(api_mcp_save))
        .route("/api/mcp/probe", post(api_mcp_probe))
        .route("/api/plugins/catalog", get(api_plugins_catalog))
        .route("/api/plugins/install", post(api_plugins_install))
        .route("/api/plugins/uninstall", post(api_plugins_uninstall))
        .route("/api/dependencies/install", post(api_dependencies_install))
        .route("/api/install/jobs", get(api_install_jobs))
        .route("/api/antigravity/sync", post(api_antigravity_sync))
        .route("/api/antigravity/export_mcp", get(api_antigravity_export_mcp))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state)
}

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn serve_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLES_CSS,
    )
}

async fn serve_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn serve_favicon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/x-icon")], CINDER_ICO)
}

async fn serve_icon_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        CINDER_ICON_SVG,
    )
}

async fn serve_manifest() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/manifest+json; charset=utf-8",
        )],
        MANIFEST_JSON,
    )
}

async fn api_activity(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!(st.store.get_activity().await))
}

async fn api_state(
    State(st): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let force = params.get("refresh").map(|v| v == "1").unwrap_or(false);
    let snap = st.store.snapshot().await;
    let hw = st.get_hardware(force).await;
    let local_models = st
        .get_local_models(
            &snap.settings.model_vault_dir,
            hw.primary_vram_mb,
            snap.settings.local_ctx_size,
            force,
        )
        .await;
    let (cloud_catalog, cloud_quotas) = st.store.build_cloud_catalog_and_quotas().await;
    let engine_status = st.get_engine_status(force).await;
    let server_status = st.server_mgr.server_status_with_probe(&st.gate).await;
    let downloads = st.download_mgr.list_jobs().await;
    let privacy_audit = st.gate.audit_summary();
    let live_activity = st.store.get_activity().await;

    let workspace_tree = st
        .get_workspace_tree(&snap.settings.active_workspace_path, force)
        .await;
    let ws_root = Path::new(&snap.settings.active_workspace_path);
    let global_skills_dir = st.store.paths().skills_dir();
    let workspace_rules = load_workspace_rules(ws_root);
    let skills = list_skills_with_global(ws_root, Some(&global_skills_dir));
    let toolchains = st.get_toolchains(force).await;
    let plugins_catalog = list_plugin_catalog_with_status(st.store.paths(), &snap.mcp_servers);
    let project_deps = inspect_project_dependencies(ws_root);
    let install_jobs = st.install_jobs.list().await;
    let active_session = snap
        .sessions
        .iter()
        .find(|s| s.id == snap.settings.active_session_id)
        .cloned()
        .or_else(|| snap.sessions.first().cloned());

    let key_flags = SafeKeyFlags {
        has_gemini_api_key: !snap.settings.api_keys.gemini_api_key.trim().is_empty(),
        has_anthropic_api_key: !snap.settings.api_keys.anthropic_api_key.trim().is_empty(),
        has_openrouter_api_key: !snap.settings.api_keys.openrouter_api_key.trim().is_empty(),
        openai_compat_base_url: snap.settings.api_keys.openai_compat_base_url.clone(),
        has_openai_compat_api_key: !snap
            .settings
            .api_keys
            .openai_compat_api_key
            .trim()
            .is_empty(),
        has_huggingface_token: !snap.settings.api_keys.huggingface_token.trim().is_empty(),
        has_brave_search_api_key: !snap.settings.api_keys.brave_search_api_key.trim().is_empty(),
    };

    Ok(Json(serde_json::json!({
        "hardware": hw,
        "privacy_audit": privacy_audit,
        "live_activity": live_activity,
        "settings": {
            "privacy_mode": st.gate.mode(),
            "model_vault_dir": snap.settings.model_vault_dir,
            "quota_tier": snap.settings.quota_tier,
            "active_model_id": snap.settings.active_model_id,
            "compare_model_b_id": snap.settings.compare_model_b_id,
            "active_workspace_path": snap.settings.active_workspace_path,
            "active_session_id": snap.settings.active_session_id,
            "auto_approve_edits": snap.settings.auto_approve_edits,
            "auto_approve_commands": snap.settings.auto_approve_commands,
            "idle_unload_minutes": snap.settings.idle_unload_minutes,
            "local_ctx_size": snap.settings.local_ctx_size,
            "local_gpu_layers": snap.settings.local_gpu_layers,
            "expose_local_api": snap.settings.expose_local_api,
            "local_server_port": snap.settings.local_server_port,
            "ui": snap.settings.ui,
            "orchestrator": snap.settings.orchestrator,
            "auto_switch_models": snap.settings.auto_switch_models,
            "fallback_to_local": snap.settings.fallback_to_local,
        },
        "key_flags": key_flags,
        "workspaces": snap.workspaces,
        "workspace_tree": workspace_tree,
        "workspace_rules": workspace_rules,
        "skills": skills,
        "toolchains": toolchains,
        "plugins_catalog": plugins_catalog,
        "project_deps": project_deps,
        "install_jobs": install_jobs,
        "local_models": local_models,
        "cloud_catalog": cloud_catalog,
        "cloud_quotas": cloud_quotas,
        "custom_models": snap.custom_models,
        "mcp_servers": snap.mcp_servers,
        "engine_status": engine_status,
        "server_status": server_status,
        "downloads": downloads,
        "sessions": snap.sessions.iter().map(|s| serde_json::json!({
            "id": s.id,
            "title": s.title,
            "workspace_path": s.workspace_path,
            "mode": s.mode,
            "message_count": s.messages.len(),
            "updated_at": s.updated_at,
        })).collect::<Vec<_>>(),
        "active_session": active_session,
    })))
}

#[derive(Deserialize)]
struct PrivacyReq {
    privacy_mode: PrivacyMode,
}

async fn api_set_privacy(
    State(st): State<AppState>,
    Json(req): Json<PrivacyReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    st.gate.set_mode(req.privacy_mode);
    st.store
        .update(|s| {
            s.settings.privacy_mode = req.privacy_mode;
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "privacy_audit": st.gate.audit_summary()
    })))
}

async fn api_network_audit(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!(st.gate.audit_summary()))
}

async fn api_network_clear(State(st): State<AppState>) -> Json<serde_json::Value> {
    st.gate.clear_audit();
    Json(serde_json::json!(st.gate.audit_summary()))
}

#[derive(Deserialize)]
struct WorkspaceSelectReq {
    path: String,
    name: Option<String>,
}

async fn api_workspace_select(
    State(st): State<AppState>,
    Json(req): Json<WorkspaceSelectReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let p = PathBuf::from(req.path.trim());
    if !p.exists() || !p.is_dir() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("Workspace directory does not exist: {}", req.path),
        ));
    }
    let canon = std::fs::canonicalize(&p).unwrap_or(p);
    let canon_str = clean_windows_verbatim(canon.to_string_lossy().as_ref());
    let name = req.name.unwrap_or_else(|| {
        Path::new(&canon_str)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Workspace")
            .to_string()
    });

    st.store
        .update(|s| {
            s.settings.active_workspace_path = canon_str.clone();
            if !s
                .workspaces
                .iter()
                .any(|w| w.path.eq_ignore_ascii_case(&canon_str))
            {
                s.workspaces.push(WorkspaceEntry {
                    id: Uuid::new_v4().to_string(),
                    name,
                    path: canon_str.clone(),
                    is_uni_project: false,
                });
            }
            if let Some(sess) = s
                .sessions
                .iter_mut()
                .find(|x| x.id == s.settings.active_session_id)
            {
                sess.workspace_path = canon_str.clone();
            }
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    st.invalidate_cache().await;
    let tree = st.get_workspace_tree(&canon_str, true).await;
    let rules = load_workspace_rules(Path::new(&canon_str));
    let skills = list_skills_with_global(Path::new(&canon_str), Some(&st.store.paths().skills_dir()));
    let project_deps = inspect_project_dependencies(Path::new(&canon_str));
    Ok(Json(serde_json::json!({
        "active_workspace_path": canon_str,
        "workspace_tree": tree,
        "workspace_rules": rules,
        "skills": skills,
        "project_deps": project_deps,
    })))
}

#[derive(Deserialize)]
struct WorkspaceRemoveReq {
    path: String,
}

async fn api_workspace_remove(
    State(st): State<AppState>,
    Json(req): Json<WorkspaceRemoveReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let target = req.path.trim().to_string();
    let active_ws = st
        .store
        .update(|s| {
            if s.workspaces.len() > 1 {
                s.workspaces
                    .retain(|w| !w.path.eq_ignore_ascii_case(&target));
            }
            if !s
                .workspaces
                .iter()
                .any(|w| w.path.eq_ignore_ascii_case(&s.settings.active_workspace_path))
            {
                if let Some(first) = s.workspaces.first() {
                    s.settings.active_workspace_path = first.path.clone();
                }
            }
            s.settings.active_workspace_path.clone()
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    st.invalidate_cache().await;
    Ok(Json(serde_json::json!({
        "active_workspace_path": active_ws,
    })))
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

async fn api_workspace_file(
    State(st): State<AppState>,
    Query(q): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws_root = st.store.snapshot().await.settings.active_workspace_path;
    let abs = resolve_workspace_path(Path::new(&ws_root), &q.path)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    if !abs.exists() || !abs.is_file() {
        return Err(api_err(StatusCode::NOT_FOUND, "File not found"));
    }

    let ext = abs
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let size_bytes = abs.metadata().map(|m| m.len()).unwrap_or(0);

    if ext == "pdf" {
        let extracted = extract_pdf_file_text(&abs, 20)
            .unwrap_or_else(|e| format!("PDF text extraction failed: {e}"));
        return Ok(Json(serde_json::json!({
            "rel_path": q.path,
            "kind": "pdf",
            "extension": ext,
            "size_bytes": size_bytes,
            "content": extracted,
            "raw_url": format!("/api/workspace/raw?path={}", encode_uri_component(&q.path)),
        })));
    }

    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg") {
        return Ok(Json(serde_json::json!({
            "rel_path": q.path,
            "kind": "image",
            "extension": ext,
            "size_bytes": size_bytes,
            "raw_url": format!("/api/workspace/raw?path={}", encode_uri_component(&q.path)),
        })));
    }

    let content = std::fs::read_to_string(&abs)
        .unwrap_or_else(|_| format!("[Binary file: {size_bytes} bytes]"));
    let kind = if ext == "html" || ext == "htm" {
        "html"
    } else if ext == "md" || ext == "markdown" {
        "markdown"
    } else {
        "text"
    };

    Ok(Json(serde_json::json!({
        "rel_path": q.path,
        "kind": kind,
        "extension": ext,
        "size_bytes": size_bytes,
        "content": content,
        "raw_url": format!("/api/workspace/raw?path={}", encode_uri_component(&q.path)),
    })))
}

async fn api_workspace_raw(
    State(st): State<AppState>,
    Query(q): Query<FileQuery>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let ws_root = st.store.snapshot().await.settings.active_workspace_path;
    let abs = resolve_workspace_path(Path::new(&ws_root), &q.path)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let bytes = std::fs::read(&abs).map_err(|_| api_err(StatusCode::NOT_FOUND, "File not found"))?;
    let ext = abs
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        _ => "text/plain; charset=utf-8",
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(bytes))
        .unwrap())
}

#[derive(Deserialize)]
struct WriteFileReq {
    path: String,
    content: String,
}

async fn api_workspace_write_file(
    State(st): State<AppState>,
    Json(req): Json<WriteFileReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws_root = st.store.snapshot().await.settings.active_workspace_path;
    let abs = resolve_workspace_path(Path::new(&ws_root), &req.path)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    std::fs::write(&abs, &req.content)
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    st.invalidate_workspace_cache().await;
    let tree = st.get_workspace_tree(&ws_root, true).await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "rel_path": req.path,
        "workspace_tree": tree,
    })))
}

#[derive(Deserialize)]
struct UploadReq {
    filename: String,
    mime_type: Option<String>,
    data_base64: String,
    save_to_workspace: Option<bool>,
}

async fn api_workspace_upload(
    State(st): State<AppState>,
    Json(req): Json<UploadReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let raw_name = Path::new(req.filename.trim())
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment.bin")
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let clean_name = if raw_name.is_empty() {
        "attachment.bin".to_string()
    } else {
        raw_name
    };

    let bytes = decode_base64(&req.data_base64)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, format!("Base64 decode failed: {e}")))?;
    let size_bytes = bytes.len() as u64;
    let ext = Path::new(&clean_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mime_type = req
        .mime_type
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                "svg" => "image/svg+xml",
                "pdf" => "application/pdf",
                "md" | "markdown" => "text/markdown",
                "json" => "application/json",
                "html" | "htm" => "text/html",
                _ => "text/plain",
            }
            .to_string()
        });
    let is_image = mime_type.starts_with("image/");

    let ws_root = st.store.snapshot().await.settings.active_workspace_path;
    let mut rel_path: Option<String> = None;
    let save_ws = req.save_to_workspace.unwrap_or(false);
    if save_ws {
        let rel = format!(".bumbo/uploads/{clean_name}");
        let abs = resolve_workspace_path(Path::new(&ws_root), &rel)
            .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
        if let Some(p) = abs.parent() {
            std::fs::create_dir_all(p)
                .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        std::fs::write(&abs, &bytes)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        rel_path = Some(rel);
        st.invalidate_workspace_cache().await;
    }

    let text_content = if is_image {
        None
    } else if ext == "pdf" {
        let tmp = std::env::temp_dir().join(format!("bumbo_upload_{}.pdf", Uuid::new_v4()));
        std::fs::write(&tmp, &bytes)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let extracted = extract_pdf_file_text(&tmp, 15)
            .unwrap_or_else(|e| format!("PDF text extraction failed: {e}"));
        let _ = std::fs::remove_file(&tmp);
        Some(extracted)
    } else {
        Some(
            String::from_utf8_lossy(&bytes)
                .chars()
                .take(60_000)
                .collect(),
        )
    };

    let clean_b64 = if is_image {
        let b64 = match req.data_base64.split_once(";base64,") {
            Some((_, part)) => part.to_string(),
            None => req.data_base64,
        };
        Some(b64)
    } else {
        None
    };

    Ok(Json(serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "name": clean_name,
        "mime_type": mime_type,
        "is_image": is_image,
        "data_base64": clean_b64,
        "text_content": text_content,
        "rel_path": rel_path,
        "size_bytes": size_bytes,
    })))
}

#[derive(Deserialize)]
struct RulesReq {
    content: String,
}

async fn api_workspace_rules(
    State(st): State<AppState>,
    Json(req): Json<RulesReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws_root = st.store.snapshot().await.settings.active_workspace_path;
    save_workspace_rules(Path::new(&ws_root), &req.content)
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ChatSendReq {
    session_id: String,
    message: String,
    mode: AgentMode,
    model_id: String,
    compare_model_b_id: Option<String>,
    attached_files: Option<Vec<String>>,
    attachments: Option<Vec<MediaAttachment>>,
    thinking_effort: Option<String>,
}

async fn build_chat_turn_response(st: &AppState, session: Session) -> Json<serde_json::Value> {
    let snap = st.store.snapshot().await;
    let plan_waiting = session
        .active_plan
        .as_ref()
        .map(|p| p.status == PlanStatus::AwaitingReview)
        .unwrap_or(false);
    if snap.settings.ui.unload_after_turn
        && session.pending_approval.is_none()
        && session.pending_question.is_none()
        && !plan_waiting
    {
        let _ = st.server_mgr.stop_all_local_models(&st.gate).await;
        st.invalidate_cache().await;
    }
    let (_, cloud_quotas) = st.store.build_cloud_catalog_and_quotas().await;
    let server_status = st.server_mgr.server_status_with_probe(&st.gate).await;
    let privacy_audit = st.gate.audit_summary();
    let workspace_tree = st.get_workspace_tree(&session.workspace_path, true).await;
    Json(serde_json::json!({
        "session": session,
        "cloud_quotas": cloud_quotas,
        "server_status": server_status,
        "privacy_audit": privacy_audit,
        "workspace_tree": workspace_tree,
    }))
}

async fn api_chat_send(
    State(st): State<AppState>,
    Json(req): Json<ChatSendReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let hw = st.get_hardware(false).await;
    let local_models = st
        .get_local_models(
            &snap.settings.model_vault_dir,
            hw.primary_vram_mb,
            snap.settings.local_ctx_size,
            false,
        )
        .await;

    let active_model = req.model_id.clone();
    let compare_b = req
        .compare_model_b_id
        .clone()
        .unwrap_or_else(|| snap.settings.compare_model_b_id.clone());

    st.store
        .update(|s| {
            s.settings.active_model_id = active_model.clone();
            s.settings.compare_model_b_id = compare_b.clone();
            if let Some(ref eff) = req.thinking_effort {
                if !eff.trim().is_empty() {
                    s.settings.ui.thinking_effort = eff.clone();
                }
            }
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let ctx = HarnessContext {
        gate: &st.gate,
        store: &st.store,
        server_mgr: &st.server_mgr,
        local_models: &local_models,
    };

    let session = run_turn(
        &ctx,
        &req.session_id,
        &req.message,
        req.mode,
        &active_model,
        Some(&compare_b),
        req.attached_files.unwrap_or_default(),
        req.attachments.unwrap_or_default(),
    )
    .await
    .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(build_chat_turn_response(&st, session).await)
}

async fn api_chat_compare(
    State(st): State<AppState>,
    Json(mut req): Json<ChatSendReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    req.mode = AgentMode::Compare;
    api_chat_send(State(st), Json(req)).await
}

#[derive(Deserialize)]
struct ApproveToolReq {
    session_id: String,
    approve: bool,
}

async fn api_chat_approve_tool(
    State(st): State<AppState>,
    Json(req): Json<ApproveToolReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let hw = st.get_hardware(false).await;
    let local_models = st
        .get_local_models(
            &snap.settings.model_vault_dir,
            hw.primary_vram_mb,
            snap.settings.local_ctx_size,
            false,
        )
        .await;
    let ctx = HarnessContext {
        gate: &st.gate,
        store: &st.store,
        server_mgr: &st.server_mgr,
        local_models: &local_models,
    };

    let session = resolve_pending_approval(&ctx, &req.session_id, req.approve)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(build_chat_turn_response(&st, session).await)
}

#[derive(Deserialize)]
struct AnswerQuestionReq {
    session_id: String,
    selected_options: Vec<String>,
    custom_text: Option<String>,
}

async fn api_chat_answer_question(
    State(st): State<AppState>,
    Json(req): Json<AnswerQuestionReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let hw = st.get_hardware(false).await;
    let local_models = st
        .get_local_models(
            &snap.settings.model_vault_dir,
            hw.primary_vram_mb,
            snap.settings.local_ctx_size,
            false,
        )
        .await;
    let ctx = HarnessContext {
        gate: &st.gate,
        store: &st.store,
        server_mgr: &st.server_mgr,
        local_models: &local_models,
    };

    let session = answer_pending_question(
        &ctx,
        &req.session_id,
        req.selected_options,
        req.custom_text.unwrap_or_default(),
    )
    .await
    .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(build_chat_turn_response(&st, session).await)
}

#[derive(Deserialize)]
struct PlanActionReq {
    session_id: String,
    action: String,
    edited_markdown: Option<String>,
    feedback: Option<String>,
}

async fn api_chat_plan_action(
    State(st): State<AppState>,
    Json(req): Json<PlanActionReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let hw = st.get_hardware(false).await;
    let local_models = st
        .get_local_models(
            &snap.settings.model_vault_dir,
            hw.primary_vram_mb,
            snap.settings.local_ctx_size,
            false,
        )
        .await;
    let ctx = HarnessContext {
        gate: &st.gate,
        store: &st.store,
        server_mgr: &st.server_mgr,
        local_models: &local_models,
    };

    let session = handle_plan_review_action(
        &ctx,
        &req.session_id,
        &req.action,
        req.edited_markdown,
        req.feedback,
    )
    .await
    .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(build_chat_turn_response(&st, session).await)
}

#[derive(Deserialize)]
struct CheckpointFileReq {
    session_id: String,
    checkpoint_id: String,
    rel_path: String,
    action: String,
}

async fn api_checkpoint_file_action(
    State(st): State<AppState>,
    Json(req): Json<CheckpointFileReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let session = if req.action == "revert" {
        revert_checkpoint_file(&st.store, &req.session_id, &req.checkpoint_id, &req.rel_path).await
    } else {
        accept_checkpoint_file(&st.store, &req.session_id, &req.checkpoint_id, &req.rel_path).await
    }
    .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    st.invalidate_workspace_cache().await;
    let workspace_tree = st.get_workspace_tree(&session.workspace_path, true).await;
    Ok(Json(serde_json::json!({
        "session": session,
        "workspace_tree": workspace_tree,
    })))
}

#[derive(Deserialize)]
struct CheckpointRollbackReq {
    session_id: String,
    checkpoint_id: String,
}

async fn api_checkpoint_rollback(
    State(st): State<AppState>,
    Json(req): Json<CheckpointRollbackReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let session = rollback_entire_checkpoint(&st.store, &req.session_id, &req.checkpoint_id)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    st.invalidate_workspace_cache().await;
    let workspace_tree = st.get_workspace_tree(&session.workspace_path, true).await;
    Ok(Json(serde_json::json!({
        "session": session,
        "workspace_tree": workspace_tree,
    })))
}

async fn api_skills_list(State(st): State<AppState>) -> Json<serde_json::Value> {
    let ws = st.store.snapshot().await.settings.active_workspace_path;
    let skills = list_skills_with_global(Path::new(&ws), Some(&st.store.paths().skills_dir()));
    Json(serde_json::json!({ "skills": skills }))
}

async fn api_skills_save(
    State(st): State<AppState>,
    Json(skill): Json<SkillDefinition>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws = st.store.snapshot().await.settings.active_workspace_path;
    save_custom_skill(Path::new(&ws), &skill)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let skills = list_skills_with_global(Path::new(&ws), Some(&st.store.paths().skills_dir()));
    Ok(Json(serde_json::json!({ "skills": skills })))
}

async fn api_knowledge_index(
    State(st): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws = st.store.snapshot().await.settings.active_workspace_path;
    let stats = build_workspace_index(Path::new(&ws), &st.gate)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::json!({ "stats": stats })))
}

#[derive(Deserialize)]
struct KnowledgeSearchReq {
    query: String,
    top_k: Option<usize>,
}

async fn api_knowledge_search(
    State(st): State<AppState>,
    Json(req): Json<KnowledgeSearchReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws = st.store.snapshot().await.settings.active_workspace_path;
    let hits = search_workspace_knowledge(Path::new(&ws), &st.gate, &req.query, req.top_k.unwrap_or(6))
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::json!({ "hits": hits })))
}

#[derive(Deserialize)]
struct NewSessionReq {
    title: Option<String>,
}

async fn api_sessions_new(
    State(st): State<AppState>,
    Json(req): Json<NewSessionReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let now = Utc::now();
    let session = st
        .store
        .update(|s| {
            let sess = Session {
                id: Uuid::new_v4().to_string(),
                title: req.title.unwrap_or_else(|| "New Session".into()),
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
            s.settings.active_session_id = sess.id.clone();
            s.sessions.insert(0, sess.clone());
            sess
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "session": session })))
}

#[derive(Deserialize)]
struct SelectSessionReq {
    session_id: String,
}

async fn api_sessions_select(
    State(st): State<AppState>,
    Json(req): Json<SelectSessionReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let session = st
        .store
        .update(|s| {
            if let Some(found) = s.sessions.iter().find(|x| x.id == req.session_id).cloned() {
                s.settings.active_session_id = found.id.clone();
                Some(found)
            } else {
                None
            }
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Session not found"))?;

    Ok(Json(serde_json::json!({ "session": session })))
}

#[derive(Deserialize)]
struct SessionModeReq {
    session_id: String,
    mode: AgentMode,
}

async fn api_sessions_mode(
    State(st): State<AppState>,
    Json(req): Json<SessionModeReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let session = st
        .store
        .update(|s| {
            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == req.session_id) {
                sess.mode = req.mode;
                sess.updated_at = Utc::now();
                return Some(sess.clone());
            }
            None
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Session not found"))?;

    Ok(Json(serde_json::json!({ "ok": true, "session": session })))
}

async fn api_sessions_delete(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    st.store
        .update(|s| {
            s.sessions.retain(|x| x.id != id);
            if s.sessions.is_empty() {
                let now = Utc::now();
                let fresh = Session {
                    id: Uuid::new_v4().to_string(),
                    title: "Workbench Session".into(),
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
            } else if s.settings.active_session_id == id {
                s.settings.active_session_id = s.sessions[0].id.clone();
            }
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct HarnessSettingsReq {
    auto_approve_edits: Option<bool>,
    auto_approve_commands: Option<bool>,
    idle_unload_minutes: Option<u64>,
    local_ctx_size: Option<u32>,
    local_gpu_layers: Option<i32>,
    expose_local_api: Option<bool>,
    active_model_id: Option<String>,
    compare_model_b_id: Option<String>,
    auto_switch_models: Option<bool>,
    fallback_to_local: Option<bool>,
}

async fn api_settings_harness(
    State(st): State<AppState>,
    Json(req): Json<HarnessSettingsReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let prev_model = st.store.snapshot().await.settings.active_model_id;
    let settings = st
        .store
        .update(|s| {
            if let Some(v) = req.auto_approve_edits {
                s.settings.auto_approve_edits = v;
            }
            if let Some(v) = req.auto_approve_commands {
                s.settings.auto_approve_commands = v;
            }
            if let Some(v) = req.idle_unload_minutes {
                s.settings.idle_unload_minutes = v;
            }
            if let Some(v) = req.local_ctx_size {
                s.settings.local_ctx_size = v.clamp(2048, 131072);
            }
            if let Some(v) = req.local_gpu_layers {
                s.settings.local_gpu_layers = v.clamp(0, 999);
            }
            if let Some(v) = req.expose_local_api {
                s.settings.expose_local_api = v;
            }
            if let Some(v) = req.active_model_id {
                s.settings.active_model_id = v;
            }
            if let Some(v) = req.compare_model_b_id {
                s.settings.compare_model_b_id = v;
            }
            if let Some(v) = req.auto_switch_models {
                s.settings.auto_switch_models = v;
            }
            if let Some(v) = req.fallback_to_local {
                s.settings.fallback_to_local = v;
            }
            s.settings.clone()
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if prev_model.starts_with("local::") && !settings.active_model_id.starts_with("local::") {
        let _ = st.server_mgr.stop_all_local_models(&st.gate).await;
        st.invalidate_cache().await;
    }

    Ok(Json(serde_json::json!({
        "auto_approve_edits": settings.auto_approve_edits,
        "auto_approve_commands": settings.auto_approve_commands,
        "idle_unload_minutes": settings.idle_unload_minutes,
        "local_ctx_size": settings.local_ctx_size,
        "local_gpu_layers": settings.local_gpu_layers,
        "expose_local_api": settings.expose_local_api,
        "active_model_id": settings.active_model_id,
        "compare_model_b_id": settings.compare_model_b_id,
        "auto_switch_models": settings.auto_switch_models,
        "fallback_to_local": settings.fallback_to_local,
    })))
}

#[derive(Deserialize)]
struct CustomizationReq {
    ui: Option<UiCustomization>,
    orchestrator: Option<OrchestratorConfig>,
}

async fn api_settings_customization(
    State(st): State<AppState>,
    Json(req): Json<CustomizationReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let settings = st
        .store
        .update(|s| {
            if let Some(ui) = req.ui {
                s.settings.ui = ui;
            }
            if let Some(orch) = req.orchestrator {
                s.settings.orchestrator = orch;
            }
            s.settings.clone()
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "ui": settings.ui,
        "orchestrator": settings.orchestrator,
    })))
}

#[derive(Deserialize)]
struct HfSearchQuery {
    q: String,
}

async fn api_hf_search(
    State(st): State<AppState>,
    Query(q): Query<HfSearchQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let token = st.store.snapshot().await.settings.api_keys.huggingface_token;
    let results = search_hf_gguf(&st.gate, &q.q, &token)
        .await
        .map_err(|e| api_err(StatusCode::FORBIDDEN, e.to_string()))?;
    Ok(Json(serde_json::json!({ "models": results })))
}

#[derive(Deserialize)]
struct HfRepoQuery {
    repo_id: String,
}

async fn api_hf_repo(
    State(st): State<AppState>,
    Query(q): Query<HfRepoQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let hw = st.get_hardware(false).await;
    let details = inspect_hf_repo(
        &st.gate,
        &q.repo_id,
        &snap.settings.api_keys.huggingface_token,
        hw.primary_vram_mb,
        snap.settings.local_ctx_size,
    )
    .await
    .map_err(|e| api_err(StatusCode::FORBIDDEN, e.to_string()))?;
    Ok(Json(serde_json::json!(details)))
}

#[derive(Deserialize)]
struct HfDownloadReq {
    repo_id: String,
    filename: String,
    sha256: Option<String>,
    size_bytes: Option<u64>,
}

async fn api_hf_download(
    State(st): State<AppState>,
    Json(req): Json<HfDownloadReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let job = st
        .download_mgr
        .start_hf_download(
            Arc::clone(&st.gate),
            PathBuf::from(&snap.settings.model_vault_dir),
            req.repo_id,
            req.filename,
            req.sha256,
            req.size_bytes,
            snap.settings.api_keys.huggingface_token,
        )
        .await
        .map_err(|e| api_err(StatusCode::FORBIDDEN, e.to_string()))?;
    Ok(Json(serde_json::json!({ "job": job })))
}

#[derive(Deserialize)]
struct CancelJobReq {
    job_id: String,
}

async fn api_hf_cancel(
    State(st): State<AppState>,
    Json(req): Json<CancelJobReq>,
) -> Json<serde_json::Value> {
    let ok = st.download_mgr.cancel_job(&req.job_id).await;
    Json(serde_json::json!({ "cancelled": ok }))
}

async fn api_downloads(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "downloads": st.download_mgr.list_jobs().await }))
}

#[derive(Deserialize)]
struct VaultDirReq {
    dir: String,
}

async fn api_set_vault_dir(
    State(st): State<AppState>,
    Json(req): Json<VaultDirReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let p = PathBuf::from(req.dir.trim());
    std::fs::create_dir_all(&p)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, format!("Cannot create directory: {e}")))?;
    let clean = clean_windows_verbatim(
        std::fs::canonicalize(&p)
            .unwrap_or(p)
            .to_string_lossy()
            .as_ref(),
    );
    st.store
        .update(|s| {
            s.settings.model_vault_dir = clean.clone();
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    st.invalidate_cache().await;
    let hw = st.get_hardware(false).await;
    let snap = st.store.snapshot().await;
    let local_models = st
        .get_local_models(
            &snap.settings.model_vault_dir,
            hw.primary_vram_mb,
            snap.settings.local_ctx_size,
            true,
        )
        .await;
    Ok(Json(serde_json::json!({
        "model_vault_dir": clean,
        "local_models": local_models,
    })))
}

#[derive(Deserialize)]
struct InstallEngineReq {
    backend: Option<String>,
}

async fn api_install_engine(
    State(st): State<AppState>,
    Json(req): Json<InstallEngineReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let backend = req.backend.unwrap_or_else(|| "cuda".into());
    let msg = st
        .server_mgr
        .install_llama_server(Arc::clone(&st.gate), &backend)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    st.invalidate_cache().await;
    let engine_status = st.get_engine_status(true).await;
    Ok(Json(serde_json::json!({
        "message": msg,
        "engine_status": engine_status,
    })))
}

#[derive(Deserialize)]
struct StartServerReq {
    model_id: String,
}

async fn api_start_server(
    State(st): State<AppState>,
    Json(req): Json<StartServerReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let hw = st.get_hardware(false).await;
    let local_models = st
        .get_local_models(
            &snap.settings.model_vault_dir,
            hw.primary_vram_mb,
            snap.settings.local_ctx_size,
            false,
        )
        .await;
    let model = local_models
        .iter()
        .find(|m| m.id == req.model_id)
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Local model not found"))?;

    let status = st
        .server_mgr
        .start_local_model(
            &st.gate,
            model,
            snap.settings.local_server_port,
            snap.settings.local_ctx_size,
            snap.settings.local_gpu_layers,
            snap.settings.expose_local_api,
        )
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    st.store
        .update(|s| {
            s.settings.active_model_id = req.model_id.clone();
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    st.invalidate_cache().await;
    Ok(Json(serde_json::json!({
        "server_status": status,
        "privacy_audit": st.gate.audit_summary(),
    })))
}

async fn api_stop_server(
    State(st): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let unloaded = st
        .server_mgr
        .stop_all_local_models(&st.gate)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    st.invalidate_cache().await;
    let server_status = st.server_mgr.server_status_with_probe(&st.gate).await;
    let hardware = st.get_hardware(true).await;
    Ok(Json(serde_json::json!({
        "unloaded": unloaded,
        "server_status": server_status,
        "hardware": hardware,
    })))
}

#[derive(Deserialize)]
struct SaveKeysReq {
    gemini_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    openrouter_api_key: Option<String>,
    openai_compat_base_url: Option<String>,
    openai_compat_api_key: Option<String>,
    huggingface_token: Option<String>,
    brave_search_api_key: Option<String>,
    clear_keys: Option<Vec<String>>,
}

async fn apply_keys_update(st: &AppState, req: SaveKeysReq) -> anyhow::Result<()> {
    st.store
        .update(|s| {
            if let Some(clears) = req.clear_keys {
                for k in clears {
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
            }
            if let Some(v) = req.gemini_api_key {
                if !v.is_empty() {
                    s.settings.api_keys.gemini_api_key = v;
                }
            }
            if let Some(v) = req.anthropic_api_key {
                if !v.is_empty() {
                    s.settings.api_keys.anthropic_api_key = v;
                }
            }
            if let Some(v) = req.openrouter_api_key {
                if !v.is_empty() {
                    s.settings.api_keys.openrouter_api_key = v;
                }
            }
            if let Some(v) = req.openai_compat_base_url {
                s.settings.api_keys.openai_compat_base_url = v;
            }
            if let Some(v) = req.openai_compat_api_key {
                if !v.is_empty() {
                    s.settings.api_keys.openai_compat_api_key = v;
                }
            }
            if let Some(v) = req.huggingface_token {
                if !v.is_empty() {
                    s.settings.api_keys.huggingface_token = v;
                }
            }
            if let Some(v) = req.brave_search_api_key {
                if !v.is_empty() {
                    s.settings.api_keys.brave_search_api_key = v;
                }
            }
        })
        .await
}

async fn api_save_keys(
    State(st): State<AppState>,
    Json(req): Json<SaveKeysReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    apply_keys_update(&st, req)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let (catalog, quotas) = st.store.build_cloud_catalog_and_quotas().await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "cloud_catalog": catalog,
        "cloud_quotas": quotas,
    })))
}

async fn api_providers_verify(
    State(st): State<AppState>,
    Json(req): Json<SaveKeysReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    apply_keys_update(&st, req)
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let statuses = if st.gate.mode() == PrivacyMode::StrictLocal {
        let snap = st.store.snapshot().await;
        let k = &snap.settings.api_keys;
        vec![
            serde_json::json!({ "provider": "google_gemini", "configured": !k.gemini_api_key.trim().is_empty(), "verified": false, "model_count": 0, "message": if k.gemini_api_key.trim().is_empty() { "No API key set" } else { "Key saved (switch to Hybrid Mode to live-verify)" } }),
            serde_json::json!({ "provider": "anthropic", "configured": !k.anthropic_api_key.trim().is_empty(), "verified": false, "model_count": 0, "message": if k.anthropic_api_key.trim().is_empty() { "No API key set" } else { "Key saved (switch to Hybrid Mode to live-verify)" } }),
            serde_json::json!({ "provider": "open_router", "configured": !k.openrouter_api_key.trim().is_empty(), "verified": false, "model_count": 0, "message": if k.openrouter_api_key.trim().is_empty() { "No API key set" } else { "Key saved (switch to Hybrid Mode to live-verify)" } }),
            serde_json::json!({ "provider": "open_ai_compat", "configured": !k.openai_compat_base_url.trim().is_empty(), "verified": false, "model_count": 0, "message": if k.openai_compat_base_url.trim().is_empty() { "No Base URL set" } else { "Base URL saved (switch to Hybrid Mode to live-verify)" } }),
        ]
    } else {
        let res = verify_and_discover_provider_models(&st.gate, &st.store)
            .await
            .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
        res.into_iter()
            .map(|s| serde_json::to_value(s).unwrap_or_default())
            .collect()
    };

    let (catalog, quotas) = st.store.build_cloud_catalog_and_quotas().await;
    Ok(Json(serde_json::json!({
        "statuses": statuses,
        "cloud_catalog": catalog,
        "cloud_quotas": quotas,
    })))
}

async fn api_sync_google(
    State(st): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let key = st.store.snapshot().await.settings.api_keys.gemini_api_key;
    let discovered = sync_google_models(&st.gate, &key)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let count = discovered.len();
    st.store
        .update(|s| {
            s.blocked_models
                .retain(|id, _| !id.starts_with("gemini-") && !id.starts_with("gemma-"));
            s.synced_google_models = discovered.clone();
            s.verified_provider_models
                .insert("google_gemini".to_string(), discovered);
            if !s.settings.active_model_id.starts_with("local::") {
                let (cat, _) = harness_store::compute_catalog_and_quotas(s);
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
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let (catalog, quotas) = st.store.build_cloud_catalog_and_quotas().await;
    Ok(Json(serde_json::json!({
        "synced_count": count,
        "cloud_catalog": catalog,
        "cloud_quotas": quotas,
    })))
}

#[derive(Deserialize)]
struct SetTierReq {
    quota_tier: QuotaTier,
    custom_quotas: Option<HashMap<String, ModelQuotaLimit>>,
}

async fn api_set_tier(
    State(st): State<AppState>,
    Json(req): Json<SetTierReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    st.store
        .update(|s| {
            s.settings.quota_tier = req.quota_tier;
            if let Some(custom) = req.custom_quotas {
                s.custom_quotas = custom;
            }
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let (catalog, quotas) = st.store.build_cloud_catalog_and_quotas().await;
    Ok(Json(serde_json::json!({
        "cloud_catalog": catalog,
        "cloud_quotas": quotas,
    })))
}

async fn api_chat_abort(State(st): State<AppState>) -> Json<serde_json::Value> {
    st.store.request_abort();
    st.store.clear_activity().await;
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Deserialize)]
struct RewindReq {
    session_id: String,
    message_index: usize,
    revert_files: Option<bool>,
}

#[derive(Deserialize)]
struct SessionArtifactReq {
    session_id: String,
    artifact_id: Option<String>,
    title: Option<String>,
    kind: Option<String>,
    content: Option<String>,
    delete: Option<bool>,
}

async fn api_sessions_artifact(
    State(st): State<AppState>,
    Json(req): Json<SessionArtifactReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if req.delete.unwrap_or(false) {
        let aid = req
            .artifact_id
            .as_deref()
            .ok_or_else(|| api_err(StatusCode::BAD_REQUEST, "Missing artifact_id for delete"))?;
        let sess = delete_session_artifact(&st.store, &req.session_id, aid)
            .await
            .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
        return Ok(Json(serde_json::json!({ "session": sess })));
    }
    let title = req.title.unwrap_or_else(|| "Notes".into());
    let kind = req.kind.unwrap_or_else(|| "report".into());
    let content = req.content.unwrap_or_default();
    let artifact = upsert_session_artifact(&st.store, &req.session_id, &title, &kind, &content)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let snap = st.store.snapshot().await;
    let sess = snap
        .sessions
        .into_iter()
        .find(|s| s.id == req.session_id)
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Session not found"))?;
    Ok(Json(serde_json::json!({ "artifact": artifact, "session": sess })))
}

#[derive(Deserialize)]
struct CompactReq {
    session_id: String,
}

async fn api_chat_compact(
    State(st): State<AppState>,
    Json(req): Json<CompactReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let sess = compact_session_context(&st.store, &req.session_id)
        .await
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(build_chat_turn_response(&st, sess).await)
}

async fn api_chat_rewind(
    State(st): State<AppState>,
    Json(req): Json<RewindReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let (session, prompt_text, attached_files, attachments) = rewind_session_to_message(
        &st.store,
        &req.session_id,
        req.message_index,
        req.revert_files.unwrap_or(true),
    )
    .await
    .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;
    st.invalidate_workspace_cache().await;
    Ok(Json(serde_json::json!({
        "session": session,
        "prompt_text": prompt_text,
        "attached_files": attached_files,
        "attachments": attachments,
        "workspace_tree": st.get_workspace_tree(&session.workspace_path, true).await,
    })))
}

#[derive(Deserialize)]
struct TerminalReq {
    command: String,
}

async fn api_workspace_terminal(
    State(st): State<AppState>,
    Json(req): Json<TerminalReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws_root = st.store.snapshot().await.settings.active_workspace_path;
    let command = req.command.trim().to_string();
    if command.is_empty() {
        return Err(api_err(StatusCode::BAD_REQUEST, "Command cannot be empty."));
    }
    let started = std::time::Instant::now();
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoProfile", "-Command", &command]);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", &command]);
        c
    };
    cmd.current_dir(&ws_root);

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| api_err(StatusCode::REQUEST_TIMEOUT, "Command timed out after 60s"))?
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok(Json(serde_json::json!({
        "exit_code": output.status.code().unwrap_or(-1),
        "stdout": stdout,
        "stderr": stderr,
        "duration_ms": started.elapsed().as_millis() as u64,
    })))
}

async fn api_workspace_watch(State(st): State<AppState>) -> Json<serde_json::Value> {
    let ws_root = st.store.snapshot().await.settings.active_workspace_path;
    let (file_count, latest_modified_ms, hash) = tokio::task::spawn_blocking(move || {
        let root_path = Path::new(&ws_root);
        let tree = list_workspace_tree(root_path, 400);
        let mut file_count = 0usize;
        let mut latest_modified_ms = 0u64;
        let mut hash: u64 = 14695981039346656037u64;

        for node in &tree {
            for b in node.rel_path.as_bytes() {
                hash ^= *b as u64;
                hash = hash.wrapping_mul(1099511628211);
            }
            hash ^= node.size_bytes;
            hash = hash.wrapping_mul(1099511628211);
            if !node.is_dir {
                file_count += 1;
                if let Ok(meta) = std::fs::metadata(root_path.join(&node.rel_path)) {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH) {
                            let ms = dur.as_millis() as u64;
                            latest_modified_ms = latest_modified_ms.max(ms);
                            hash ^= ms;
                            hash = hash.wrapping_mul(1099511628211);
                        }
                    }
                }
            }
        }
        (ws_root, file_count, latest_modified_ms, hash)
    })
    .await
    .map(|(ws, fc, lm, h)| {
        let _ = ws;
        (fc, lm, h)
    })
    .unwrap_or((0, 0, 0));

    let ws_now = st.store.snapshot().await.settings.active_workspace_path;
    Json(serde_json::json!({
        "workspace_path": ws_now,
        "file_count": file_count,
        "latest_modified_ms": latest_modified_ms,
        "fingerprint": hash.to_string(),
    }))
}

async fn api_system_pick_folder() -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let picked = tokio::task::spawn_blocking(|| {
        harness_tools::pick_folder("Select Project or Vault Folder for Cinder")
    })
    .await
    .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "picked": picked.is_some(),
        "path": picked.unwrap_or_default(),
    })))
}

async fn api_models_presets(State(_st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "presets": [] }))
}

#[derive(Deserialize)]
struct CustomModelReq {
    id: String,
    display_name: Option<String>,
    provider: Option<ProviderKind>,
    context_window: Option<u64>,
    supports_thinking: Option<bool>,
    supports_tools: Option<bool>,
}

async fn api_providers_custom_model(
    State(st): State<AppState>,
    Json(req): Json<CustomModelReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let id = req.id.trim().to_string();
    if id.is_empty() {
        return Err(api_err(StatusCode::BAD_REQUEST, "Model ID cannot be empty."));
    }
    let lower = id.to_ascii_lowercase();
    let provider = req.provider.unwrap_or_else(|| {
        if lower.starts_with("gemini-") || lower.starts_with("gemma-") {
            ProviderKind::GoogleGemini
        } else if lower.starts_with("claude-") {
            ProviderKind::Anthropic
        } else if lower.contains('/') {
            ProviderKind::OpenRouter
        } else {
            ProviderKind::OpenAiCompat
        }
    });

    if provider == ProviderKind::GoogleGemini && st.gate.mode() == PrivacyMode::Hybrid {
        let key = st.store.snapshot().await.settings.api_keys.gemini_api_key;
        if !key.trim().is_empty() {
            let ok = probe_gemini_model_live(&st.gate, &key, &id).await;
            if !ok {
                return Err(api_err(
                    StatusCode::BAD_REQUEST,
                    format!("Model '{id}' failed live generation probe (404 / 400 / limit: 0 on current API tier)."),
                ));
            }
        }
    }

    let display = req
        .display_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| id.clone());
    let entry = ModelCatalogEntry {
        id: id.clone(),
        display_name: display.clone(),
        provider,
        context_window: req.context_window.unwrap_or(1_000_000),
        supports_thinking: req.supports_thinking.unwrap_or(true),
        supports_tools: req.supports_tools.unwrap_or(!lower.starts_with("gemma")),
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
        antigravity_alias: Some(display),
    };

    st.store
        .update(|s| {
            s.blocked_models.remove(&id);
            s.custom_models.retain(|m| m.id != id);
            s.custom_models.push(entry.clone());
            let p_key = provider_key_name(provider).to_string();
            let list = s.verified_provider_models.entry(p_key).or_default();
            if !list.iter().any(|m| m.id == id) {
                list.push(entry.clone());
            }
            if provider == ProviderKind::GoogleGemini
                && !s.synced_google_models.iter().any(|m| m.id == id)
            {
                s.synced_google_models.push(entry);
            }
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (catalog, quotas) = st.store.build_cloud_catalog_and_quotas().await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "cloud_catalog": catalog,
        "cloud_quotas": quotas,
    })))
}

#[derive(Deserialize)]
struct McpSaveReq {
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    target: String,
    enabled: Option<bool>,
    delete: Option<bool>,
    toggle_only: Option<bool>,
    command: Option<String>,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
    url: Option<String>,
    needs_network: Option<bool>,
    read_only: Option<bool>,
}

async fn api_mcp_save(
    State(st): State<AppState>,
    Json(req): Json<McpSaveReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mcp_servers = st
        .store
        .update(|s| {
            if req.delete.unwrap_or(false) {
                if let Some(ref del_id) = req.id {
                    s.mcp_servers.retain(|m| m.id != *del_id);
                }
            } else if req.toggle_only.unwrap_or(false) {
                if let Some(ref tid) = req.id {
                    if let Some(existing) = s.mcp_servers.iter_mut().find(|m| m.id == *tid) {
                        if let Some(en) = req.enabled {
                            existing.enabled = en;
                        } else {
                            existing.enabled = !existing.enabled;
                        }
                    }
                }
            } else {
                let id = req.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
                let transport = if req.transport.trim().is_empty() {
                    "stdio".to_string()
                } else {
                    req.transport.trim().to_string()
                };
                let target = req.target.trim().to_string();
                let is_http = transport.eq_ignore_ascii_case("http");
                let url = req
                    .url
                    .clone()
                    .unwrap_or_else(|| if is_http { target.clone() } else { String::new() });
                let needs_network = req.needs_network.unwrap_or_else(|| {
                    is_http && !url.contains("127.0.0.1") && !url.contains("localhost")
                });
                let entry = McpServerEntry {
                    id: id.clone(),
                    name: req.name.trim().to_string(),
                    transport,
                    target,
                    enabled: req.enabled.unwrap_or(true),
                    command: req.command.clone().unwrap_or_default(),
                    args: req.args.clone().unwrap_or_default(),
                    env: req.env.clone().unwrap_or_default(),
                    url,
                    source: "manual".into(),
                    needs_network,
                    read_only: req.read_only.unwrap_or(false),
                };
                if let Some(existing) = s.mcp_servers.iter_mut().find(|m| m.id == id) {
                    *existing = entry;
                } else {
                    s.mcp_servers.push(entry);
                }
            }
            s.mcp_servers.clone()
        })
        .await
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "ok": true, "mcp_servers": mcp_servers })))
}

#[derive(Deserialize)]
struct McpProbeReq {
    id: String,
}

async fn api_mcp_probe(
    State(st): State<AppState>,
    Json(req): Json<McpProbeReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let snap = st.store.snapshot().await;
    let entry = snap
        .mcp_servers
        .iter()
        .find(|m| m.id == req.id)
        .cloned()
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, format!("MCP server '{}' not found.", req.id)))?;

    let probe = probe_mcp_server(&st.gate, &entry).await;
    Ok(Json(serde_json::json!({ "ok": probe.ok, "probe": probe })))
}

async fn api_plugins_catalog(
    State(st): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let force = params.get("refresh").map(|v| v == "1").unwrap_or(false);
    let snap = st.store.snapshot().await;
    let toolchains = st.get_toolchains(force).await;
    let plugins = list_plugin_catalog_with_status(st.store.paths(), &snap.mcp_servers);
    let project_deps = inspect_project_dependencies(Path::new(&snap.settings.active_workspace_path));
    let install_jobs = st.install_jobs.list().await;

    Json(serde_json::json!({
        "toolchains": toolchains,
        "plugins": plugins,
        "project_deps": project_deps,
        "install_jobs": install_jobs,
    }))
}

#[derive(Deserialize)]
struct PluginInstallReq {
    plugin_id: String,
    #[serde(default)]
    wait: bool,
}

async fn api_plugins_install(
    State(st): State<AppState>,
    Json(req): Json<PluginInstallReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let catalog = list_plugin_catalog_with_status(st.store.paths(), &[]);
    let Some(item) = catalog.iter().find(|c| c.entry.id == req.plugin_id) else {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!("Plugin '{}' is not on Cinder's curated allow-list.", req.plugin_id),
        ));
    };

    if item.entry.kind != "skill_pack" && st.gate.mode() == PrivacyMode::StrictLocal {
        return Err(api_err(
            StatusCode::FORBIDDEN,
            format!(
                "Strict Local Mode blocked outbound install of '{}'. Switch to Hybrid Mode to install plugins.",
                item.entry.name
            ),
        ));
    }

    let job = st.install_jobs.start_job("plugin", &req.plugin_id).await;
    let job_id = job.id.clone();
    let plugin_id = req.plugin_id.clone();
    let st_bg = st.clone();

    let handle = tokio::spawn(async move {
        let rt_handle = tokio::runtime::Handle::current();
        let jobs_cb = st_bg.install_jobs.clone();
        let jid_cb = job_id.clone();

        let res = install_curated_plugin(&st_bg.gate, st_bg.store.paths(), &plugin_id, |pct, msg| {
            let jobs_inner = jobs_cb.clone();
            let jid_inner = jid_cb.clone();
            rt_handle.spawn(async move {
                jobs_inner.push_progress(&jid_inner, pct, msg).await;
            });
        })
        .await;

        match res {
            Ok(outcome) => {
                if let Some(mcp_entry) = outcome.mcp_entry {
                    let _ = st_bg
                        .store
                        .update(|s| {
                            if let Some(existing) =
                                s.mcp_servers.iter_mut().find(|m| m.id == mcp_entry.id)
                            {
                                *existing = mcp_entry;
                            } else {
                                s.mcp_servers.push(mcp_entry);
                            }
                        })
                        .await;
                }
                st_bg.invalidate_toolchains_cache().await;
                st_bg.install_jobs.finish_ok(&job_id, outcome.summary).await;
            }
            Err(e) => {
                st_bg.install_jobs.finish_err(&job_id, e.to_string()).await;
            }
        }
    });

    if req.wait {
        let _ = handle.await;
        let final_job = st
            .install_jobs
            .list()
            .await
            .into_iter()
            .find(|j| j.id == job.id)
            .unwrap_or(job);
        let ok = final_job.status == "completed";
        if !ok {
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                final_job
                    .error
                    .clone()
                    .unwrap_or_else(|| "Plugin installation failed.".into()),
            ));
        }
        return Ok(Json(serde_json::json!({ "ok": true, "job": final_job })));
    }

    Ok(Json(serde_json::json!({ "ok": true, "job": job })))
}

#[derive(Deserialize)]
struct PluginUninstallReq {
    plugin_id: String,
}

async fn api_plugins_uninstall(
    State(st): State<AppState>,
    Json(req): Json<PluginUninstallReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let maybe_mcp_id = uninstall_curated_plugin(st.store.paths(), &req.plugin_id)
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e.to_string()))?;

    if let Some(mcp_id) = maybe_mcp_id {
        let _ = st
            .store
            .update(|s| {
                s.mcp_servers.retain(|m| m.id != mcp_id);
            })
            .await;
    }
    st.invalidate_toolchains_cache().await;

    let snap = st.store.snapshot().await;
    let plugins = list_plugin_catalog_with_status(st.store.paths(), &snap.mcp_servers);
    Ok(Json(serde_json::json!({
        "ok": true,
        "plugins": plugins,
        "mcp_servers": snap.mcp_servers,
    })))
}

#[derive(Deserialize)]
struct DepInstallReq {
    manager: String,
    package: String,
    #[serde(default)]
    wait: bool,
}

async fn api_dependencies_install(
    State(st): State<AppState>,
    Json(req): Json<DepInstallReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if is_dependency_allowlisted(&req.manager, &req.package).is_none() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            format!(
                "Package '{}' ({}) is not on Cinder's curated allow-list.",
                req.package, req.manager
            ),
        ));
    }

    if st.gate.mode() == PrivacyMode::StrictLocal {
        return Err(api_err(
            StatusCode::FORBIDDEN,
            format!(
                "Strict Local Mode blocked installing '{}'. Switch to Hybrid Mode first.",
                req.package
            ),
        ));
    }

    let ws_path = st.store.snapshot().await.settings.active_workspace_path;
    let target_label = format!("{}:{}", req.manager, req.package);
    let job = st.install_jobs.start_job("dependency", &target_label).await;
    let job_id = job.id.clone();
    let manager = req.manager.clone();
    let package = req.package.clone();
    let st_bg = st.clone();

    let handle = tokio::spawn(async move {
        let rt_handle = tokio::runtime::Handle::current();
        let jobs_cb = st_bg.install_jobs.clone();
        let jid_cb = job_id.clone();

        let res = install_project_dependency(
            &st_bg.gate,
            st_bg.store.paths(),
            Path::new(&ws_path),
            &manager,
            &package,
            |pct, msg| {
                let jobs_inner = jobs_cb.clone();
                let jid_inner = jid_cb.clone();
                rt_handle.spawn(async move {
                    jobs_inner.push_progress(&jid_inner, pct, msg).await;
                });
            },
        )
        .await;

        match res {
            Ok(summary) => {
                st_bg.invalidate_workspace_cache().await;
                st_bg.install_jobs.finish_ok(&job_id, summary).await;
            }
            Err(e) => {
                st_bg.install_jobs.finish_err(&job_id, e.to_string()).await;
            }
        }
    });

    if req.wait {
        let _ = handle.await;
        let final_job = st
            .install_jobs
            .list()
            .await
            .into_iter()
            .find(|j| j.id == job.id)
            .unwrap_or(job);
        if final_job.status != "completed" {
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                final_job
                    .error
                    .clone()
                    .unwrap_or_else(|| "Dependency installation failed.".into()),
            ));
        }
        return Ok(Json(serde_json::json!({ "ok": true, "job": final_job })));
    }

    Ok(Json(serde_json::json!({ "ok": true, "job": job })))
}

async fn api_install_jobs(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "jobs": st.install_jobs.list().await }))
}

#[derive(Deserialize, Default)]
struct AgySyncReq {
    #[serde(default)]
    include_builtin_skills: bool,
}

async fn api_antigravity_sync(
    State(st): State<AppState>,
    Json(req): Json<AgySyncReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ws = st.store.snapshot().await.settings.active_workspace_path;
    let report = sync_from_antigravity(st.store.paths(), Path::new(&ws), req.include_builtin_skills)
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !report.imported_mcp_servers.is_empty() {
        let imported = report.imported_mcp_servers.clone();
        let _ = st
            .store
            .update(|s| {
                for entry in imported {
                    if let Some(existing) = s.mcp_servers.iter_mut().find(|m| m.id == entry.id) {
                        *existing = entry;
                    } else {
                        s.mcp_servers.push(entry);
                    }
                }
            })
            .await;
    }

    let skills = list_skills_with_global(Path::new(&ws), Some(&st.store.paths().skills_dir()));
    let mcp_servers = st.store.snapshot().await.mcp_servers;

    Ok(Json(serde_json::json!({
        "ok": true,
        "report": report,
        "skills": skills,
        "mcp_servers": mcp_servers,
    })))
}

async fn api_antigravity_export_mcp(State(st): State<AppState>) -> Json<serde_json::Value> {
    let snap = st.store.snapshot().await;
    Json(export_antigravity_mcp_json(&snap.mcp_servers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppCache;
    use harness_models::{DownloadManager, ServerManager};
    use harness_net::NetworkGate;
    use harness_store::{BumboPaths, BumboStore};
    use harness_tools::InstallJobManager;

    fn make_test_state() -> (AppState, PathBuf) {
        let dir = std::env::temp_dir().join(format!("cinder_route_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let store = Arc::new(BumboStore::init(paths.clone()).unwrap());
        let gate = Arc::new(NetworkGate::new(PrivacyMode::StrictLocal).unwrap());
        let st = AppState {
            gate,
            store,
            server_mgr: Arc::new(ServerManager::new(paths.engines_dir)),
            download_mgr: Arc::new(DownloadManager::new()),
            install_jobs: InstallJobManager::new(),
            cache: Arc::new(AppCache::default()),
        };
        (st, dir)
    }

    #[cfg(windows)]
    #[test]
    fn win32_folder_dialog_initializes_on_sta_thread() {
        let ok = std::thread::spawn(|| {
            harness_tools::native_dialogs::win32::create_folder_dialog("Cinder Folder Test").is_ok()
        })
        .join()
        .unwrap();
        assert!(ok, "COM IFileOpenDialog should initialize cleanly on Windows STA thread");
    }

    #[test]
    fn plugin_install_rejects_non_allowlisted_and_installs_skill_pack() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (st, dir) = make_test_state();
        rt.block_on(async {
            // Non-allowlisted plugin must be rejected
            let bad = api_plugins_install(
                State(st.clone()),
                Json(PluginInstallReq {
                    plugin_id: "untrusted-random-plugin".into(),
                    wait: true,
                }),
            )
            .await;
            assert!(bad.is_err());

            // Non-allowlisted dependency must be rejected
            let bad_dep = api_dependencies_install(
                State(st.clone()),
                Json(DepInstallReq {
                    manager: "cargo".into(),
                    package: "arbitrary-crate".into(),
                    wait: true,
                }),
            )
            .await;
            assert!(bad_dep.is_err());

            // Curated offline skill pack installs via temp folder even in Strict Local mode
            let ok = api_plugins_install(
                State(st.clone()),
                Json(PluginInstallReq {
                    plugin_id: "skill-rust-audit".into(),
                    wait: true,
                }),
            )
            .await
            .unwrap();
            assert_eq!(ok.0["ok"], true);
            assert!(st.store.paths().skills_dir().join("rust-audit").join("SKILL.md").exists());

            // Auto-switch settings persist cleanly via /api/settings/harness
            let sw = api_settings_harness(
                State(st.clone()),
                Json(HarnessSettingsReq {
                    auto_approve_edits: None,
                    auto_approve_commands: None,
                    idle_unload_minutes: None,
                    local_ctx_size: None,
                    local_gpu_layers: None,
                    expose_local_api: None,
                    active_model_id: None,
                    compare_model_b_id: None,
                    auto_switch_models: Some(true),
                    fallback_to_local: Some(true),
                }),
            )
            .await
            .unwrap();
            assert_eq!(sw.0["auto_switch_models"], true);
            assert_eq!(sw.0["fallback_to_local"], true);

            // Uninstall cleans up cleanly
            let un = api_plugins_uninstall(
                State(st.clone()),
                Json(PluginUninstallReq {
                    plugin_id: "skill-rust-audit".into(),
                }),
            )
            .await
            .unwrap();
            assert_eq!(un.0["ok"], true);
            assert!(!st.store.paths().skills_dir().join("rust-audit").exists());
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
