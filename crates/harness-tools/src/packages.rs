use crate::mcp::{probe_mcp_server, McpProbeResult};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use harness_net::{NetworkGate, PrivacyMode};
use harness_store::{BumboPaths, McpServerEntry};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ============================================================================
// Temp-folder staging guard (guarantees zero leftover files on error)
// ============================================================================

pub struct TempStageDir {
    path: PathBuf,
    cleanup: bool,
}

impl TempStageDir {
    pub fn create(parent: &Path, prefix: &str) -> anyhow::Result<Self> {
        std::fs::create_dir_all(parent)?;
        let safe_prefix: String = prefix
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let path = parent.join(format!(
            "cinder-stage-{}-{}",
            safe_prefix,
            &Uuid::new_v4().to_string()[..8]
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            cleanup: true,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Move staged directory to `dest` atomically (same volume) or via recursive copy fallback.
    pub fn commit_to(mut self, dest: &Path) -> anyhow::Result<()> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if dest.exists() {
            std::fs::remove_dir_all(dest)?;
        }
        match std::fs::rename(&self.path, dest) {
            Ok(()) => {
                self.cleanup = false;
                Ok(())
            }
            Err(_) => {
                copy_dir_recursive(&self.path, dest)?;
                // Drop will clean up self.path
                Ok(())
            }
        }
    }
}

impl Drop for TempStageDir {
    fn drop(&mut self) {
        if self.cleanup && self.path.exists() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(p) = target.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn configure_no_window_std(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

fn configure_no_window_tokio(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

// ============================================================================
// Toolchain Detection (Python, Cargo, Git, Node/npm system or Cinder portable)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub version: String,
    pub path: String,
    pub source: String, // "portable" | "system" | "missing"
    pub installable_plugin_id: Option<String>,
}

pub fn portable_node_dir(paths: &BumboPaths) -> PathBuf {
    paths.toolchains_dir().join("node")
}

pub fn resolve_cargo_executable() -> Option<PathBuf> {
    if let Some(home) = dirs::home_dir() {
        #[cfg(windows)]
        let cand = home.join(".cargo").join("bin").join("cargo.exe");
        #[cfg(not(windows))]
        let cand = home.join(".cargo").join("bin").join("cargo");
        if cand.exists() && probe_cmd_version(&cand.to_string_lossy(), &["--version"]).is_some() {
            return Some(cand);
        }
    }
    if probe_cmd_version("cargo", &["--version"]).is_some() {
        return Some(PathBuf::from("cargo"));
    }
    None
}

fn prepend_tool_dir_to_path(tool_path: &Path) -> String {
    let mut path_env = std::env::var("PATH").unwrap_or_default();
    if let Some(parent) = tool_path.parent() {
        if !parent.as_os_str().is_empty() {
            let sep = if cfg!(windows) { ";" } else { ":" };
            path_env = format!("{}{sep}{path_env}", parent.display());
        }
    }
    path_env
}

pub fn resolve_node_and_npm(paths: &BumboPaths) -> Option<(PathBuf, PathBuf, &'static str)> {
    let pdir = portable_node_dir(paths);
    #[cfg(windows)]
    let (p_node, p_npm) = (pdir.join("node.exe"), pdir.join("npm.cmd"));
    #[cfg(not(windows))]
    let (p_node, p_npm) = (pdir.join("bin").join("node"), pdir.join("bin").join("npm"));

    if p_node.exists() && p_npm.exists() {
        return Some((p_node, p_npm, "portable"));
    }

    if let Some((sys_node, _)) = probe_cmd_version("node", &["--version"]) {
        #[cfg(windows)]
        let npm_bin = PathBuf::from("npm.cmd");
        #[cfg(not(windows))]
        let npm_bin = PathBuf::from("npm");
        if probe_cmd_version(&npm_bin.to_string_lossy(), &["--version"]).is_some() {
            return Some((PathBuf::from(sys_node), npm_bin, "system"));
        }
    }
    None
}

pub fn resolve_python_executable() -> Option<String> {
    let check_args = [
        "-c",
        "import sys; print(f'Python {sys.version.split()[0]}')",
    ];
    for cand in ["python", "python3", "py"] {
        if probe_cmd_version(cand, &check_args).is_some() {
            return Some(cand.to_string());
        }
    }
    #[cfg(windows)]
    {
        if let Some(local_app) = std::env::var_os("LOCALAPPDATA") {
            let py_root = PathBuf::from(local_app).join("Programs").join("Python");
            if let Ok(entries) = std::fs::read_dir(&py_root) {
                let mut candidates: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path().join("python.exe"))
                    .filter(|p| p.exists())
                    .collect();
                candidates.sort();
                candidates.reverse();
                for cand in candidates {
                    let s = cand.to_string_lossy().to_string();
                    if probe_cmd_version(&s, &check_args).is_some() {
                        return Some(s);
                    }
                }
            }
        }
    }
    None
}

fn build_npm_tokio_command(node_path: &Path, npm_path: &Path) -> tokio::process::Command {
    let path_env = prepend_tool_dir_to_path(node_path);
    #[cfg(windows)]
    let npm_cli = node_path
        .parent()
        .map(|d| d.join("node_modules").join("npm").join("bin").join("npm-cli.js"));
    #[cfg(not(windows))]
    let npm_cli = node_path.parent().and_then(|d| {
        d.parent().map(|root| {
            root.join("lib")
                .join("node_modules")
                .join("npm")
                .join("bin")
                .join("npm-cli.js")
        })
    });

    let mut cmd = if let Some(cli) = npm_cli.filter(|p| p.exists()) {
        let mut c = tokio::process::Command::new(node_path);
        c.arg(cli);
        c
    } else {
        tokio::process::Command::new(npm_path)
    };
    cmd.env("PATH", path_env);
    crate::scrub_sensitive_child_env(&mut cmd);
    configure_no_window_tokio(&mut cmd);
    cmd
}

fn nodejs_dist_spec_for(
    os: &str,
    arch: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match (os, arch) {
        ("windows", "x86_64") => Some(("win-x64-zip", "win-x64.zip", "zip")),
        ("macos", "aarch64") => Some(("osx-arm64-tar", "darwin-arm64.tar.gz", "targz")),
        ("macos", "x86_64") => Some(("osx-x64-tar", "darwin-x64.tar.gz", "targz")),
        ("linux", "x86_64") => Some(("linux-x64", "linux-x64.tar.gz", "targz")),
        ("linux", "aarch64") => Some(("linux-arm64", "linux-arm64.tar.gz", "targz")),
        _ => None,
    }
}

fn parse_shasums256_for_file(text: &str, filename: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_ascii_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(name_tok) = parts.next() else {
            continue;
        };
        let clean_name = name_tok.trim_start_matches('*');
        if clean_name == filename
            && hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

async fn ensure_node_and_npm<F>(
    gate: &NetworkGate,
    paths: &BumboPaths,
    stage_root: &Path,
    require_portable: bool,
    on_log: &mut F,
) -> anyhow::Result<(PathBuf, PathBuf, &'static str)>
where
    F: FnMut(u8, String) + Send,
{
    if let Some((node, npm, source)) = resolve_node_and_npm(paths) {
        if !require_portable || source == "portable" {
            return Ok((node, npm, source));
        }
    }

    if gate.mode() == PrivacyMode::StrictLocal {
        anyhow::bail!(
            "Strict Local Mode blocked downloading Portable Node.js LTS. Switch to Hybrid Mode first."
        );
    }

    let (index_file_tag, archive_suffix, archive_kind) =
        nodejs_dist_spec_for(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
            anyhow::anyhow!(
                "Automatic Portable Node.js LTS install is not configured for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;

    let stage = TempStageDir::create(stage_root, "nodejs-lts")?;
    on_log(
        10,
        "Querying nodejs.org/dist/index.json for latest LTS release...".into(),
    );

    let idx_resp = gate
        .request(
            Method::GET,
            "https://nodejs.org/dist/index.json",
            "toolchain_nodejs_index",
        )?
        .send()
        .await?;
    if !idx_resp.status().is_success() {
        anyhow::bail!("nodejs.org index returned HTTP {}", idx_resp.status());
    }
    let releases: Vec<Value> = idx_resp.json().await?;
    let lts = releases
        .iter()
        .find(|r| {
            let is_lts = r.get("lts").map(|v| v.is_string()).unwrap_or(false);
            let has_target = r["files"]
                .as_array()
                .map(|a| a.iter().any(|f| f.as_str() == Some(index_file_tag)))
                .unwrap_or(false);
            is_lts && has_target
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Could not locate a Node.js LTS release matching '{index_file_tag}'."
            )
        })?;

    let ver = lts["version"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing Node.js LTS version field"))?;
    let archive_name = format!("node-{ver}-{archive_suffix}");
    let shasums_url = format!("https://nodejs.org/dist/{ver}/SHASUMS256.txt");
    on_log(14, format!("Fetching SHA-256 checksums for {ver}..."));
    let shasums_resp = gate
        .request(Method::GET, &shasums_url, "toolchain_nodejs_checksums")?
        .send()
        .await?;
    if !shasums_resp.status().is_success() {
        anyhow::bail!(
            "nodejs.org SHASUMS256.txt returned HTTP {}",
            shasums_resp.status()
        );
    }
    let shasums_text = shasums_resp.text().await?;
    let expected_sha256 = parse_shasums256_for_file(&shasums_text, &archive_name).ok_or_else(|| {
        anyhow::anyhow!("SHA-256 checksum missing for '{archive_name}' in SHASUMS256.txt")
    })?;

    let url = format!("https://nodejs.org/dist/{ver}/{archive_name}");

    on_log(18, format!("Downloading {archive_name} into temp folder..."));
    let resp = gate
        .download_request(Method::GET, &url, "toolchain_nodejs_download")?
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Node.js download failed (HTTP {})", resp.status());
    }

    let total_len = resp.content_length().unwrap_or(0);
    let archive_path = stage.path().join(&archive_name);
    let mut file = std::fs::File::create(&archive_path)?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut last_pct = 18u8;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        hasher.update(&bytes);
        file.write_all(&bytes)?;
        downloaded += bytes.len() as u64;
        if total_len > 0 {
            let pct = 18 + (((downloaded as f64 / total_len as f64) * 37.0).round() as u8).min(37);
            if pct >= last_pct + 6 {
                last_pct = pct;
                let mb_done = downloaded as f64 / (1024.0 * 1024.0);
                let mb_tot = total_len as f64 / (1024.0 * 1024.0);
                on_log(
                    pct,
                    format!("Downloading {archive_name} ({mb_done:.1} / {mb_tot:.1} MB)..."),
                );
            }
        }
    }
    file.flush()?;
    drop(file);

    let computed_sha256 = hex::encode(hasher.finalize());
    if !computed_sha256.eq_ignore_ascii_case(&expected_sha256) {
        anyhow::bail!(
            "SHA-256 verification failed for {archive_name}: expected {expected_sha256}, got {computed_sha256}"
        );
    }

    on_log(60, "Extracting Node.js LTS archive inside temp folder...".into());
    let extract_dir = stage.path().join("extracted");
    let ap = archive_path.clone();
    let ed = extract_dir.clone();
    tokio::task::spawn_blocking(move || {
        if archive_kind == "zip" {
            extract_zip_preserving_tree(&ap, &ed)
        } else {
            extract_targz_preserving_tree(&ap, &ed)
        }
    })
    .await??;

    let inner_dir = std::fs::read_dir(&extract_dir)?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            if !p.is_dir() {
                return false;
            }
            #[cfg(windows)]
            {
                p.join("node.exe").exists()
            }
            #[cfg(not(windows))]
            {
                p.join("bin").join("node").exists()
            }
        })
        .ok_or_else(|| anyhow::anyhow!("Extracted Node.js archive did not contain node binary"))?;

    #[cfg(windows)]
    let node_bin = inner_dir.join("node.exe");
    #[cfg(not(windows))]
    let node_bin = inner_dir.join("bin").join("node");

    let _ver_check = probe_cmd_version(&node_bin.to_string_lossy(), &["--version"])
        .ok_or_else(|| anyhow::anyhow!("Downloaded node binary failed --version check"))?;

    let final_dir = portable_node_dir(paths);
    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir)?;
    }
    if let Some(p) = final_dir.parent() {
        std::fs::create_dir_all(p)?;
    }
    if std::fs::rename(&inner_dir, &final_dir).is_err() {
        copy_dir_recursive(&inner_dir, &final_dir)?;
    }

    resolve_node_and_npm(paths)
        .ok_or_else(|| anyhow::anyhow!("Portable Node.js installed but failed verification"))
}

fn probe_cmd_version(program: &str, args: &[&str]) -> Option<(String, String)> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    configure_no_window_std(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let ver = combined
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("installed")
        .to_string();
    Some((program.to_string(), ver))
}

pub fn probe_toolchains(paths: &BumboPaths) -> Vec<ToolchainStatus> {
    let mut list = Vec::new();

    // Cargo
    if let Some(cargo_path) = resolve_cargo_executable() {
        let p_str = cargo_path.to_string_lossy().to_string();
        let v = probe_cmd_version(&p_str, &["--version"])
            .map(|(_, v)| v)
            .unwrap_or_else(|| "cargo installed".into());
        list.push(ToolchainStatus {
            id: "cargo".into(),
            name: "Rust / Cargo".into(),
            installed: true,
            version: v,
            path: p_str,
            source: "system".into(),
            installable_plugin_id: None,
        });
    } else {
        list.push(ToolchainStatus {
            id: "cargo".into(),
            name: "Rust / Cargo".into(),
            installed: false,
            version: "Not found on PATH".into(),
            path: String::new(),
            source: "missing".into(),
            installable_plugin_id: None,
        });
    }

    // Python
    if let Some(py) = resolve_python_executable() {
        let ver = probe_cmd_version(&py, &["--version"])
            .map(|(_, v)| v)
            .unwrap_or_else(|| "Python".into());
        list.push(ToolchainStatus {
            id: "python".into(),
            name: "Python + pip".into(),
            installed: true,
            version: ver,
            path: py,
            source: "system".into(),
            installable_plugin_id: None,
        });
    } else {
        list.push(ToolchainStatus {
            id: "python".into(),
            name: "Python + pip".into(),
            installed: false,
            version: "Not found on PATH".into(),
            path: String::new(),
            source: "missing".into(),
            installable_plugin_id: None,
        });
    }

    // Node.js (Cinder portable or system)
    if let Some((node_path, _, source)) = resolve_node_and_npm(paths) {
        let ver = probe_cmd_version(&node_path.to_string_lossy(), &["--version"])
            .map(|(_, v)| v)
            .unwrap_or_else(|| "installed".into());
        list.push(ToolchainStatus {
            id: "node".into(),
            name: "Node.js + npm".into(),
            installed: true,
            version: ver,
            path: node_path.to_string_lossy().to_string(),
            source: source.into(),
            installable_plugin_id: Some("toolchain-nodejs".into()),
        });
    } else {
        list.push(ToolchainStatus {
            id: "node".into(),
            name: "Node.js + npm".into(),
            installed: false,
            version: "Click Install Portable Node.js LTS".into(),
            path: String::new(),
            source: "missing".into(),
            installable_plugin_id: Some("toolchain-nodejs".into()),
        });
    }

    // Git
    if let Some((p, v)) = probe_cmd_version("git", &["--version"]) {
        list.push(ToolchainStatus {
            id: "git".into(),
            name: "Git".into(),
            installed: true,
            version: v,
            path: p,
            source: "system".into(),
            installable_plugin_id: None,
        });
    } else {
        list.push(ToolchainStatus {
            id: "git".into(),
            name: "Git".into(),
            installed: false,
            version: "Not found on PATH".into(),
            path: String::new(),
            source: "missing".into(),
            installable_plugin_id: None,
        });
    }

    list
}

// ============================================================================
// Curated Allow-List: Project Dependencies (Cargo / Python / npm)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratedDependency {
    pub manager: String, // "cargo" | "python" | "npm"
    pub name: String,
    pub category: String,
    pub description: String,
    pub features_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDependenciesOverview {
    pub workspace_path: String,
    pub detected_ecosystems: Vec<String>,
    pub installed_by_manager: HashMap<String, Vec<String>>,
    pub allowlist: Vec<CuratedDependency>,
}

pub fn curated_dependency_allowlist() -> Vec<CuratedDependency> {
    vec![
        // Cargo (Rust)
        CuratedDependency {
            manager: "cargo".into(),
            name: "serde".into(),
            category: "Serialization".into(),
            description: "Fast serialization/deserialization framework (adds derive feature).".into(),
            features_hint: Some("derive".into()),
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "serde_json".into(),
            category: "Serialization".into(),
            description: "Strongly typed JSON parsing and serialization for Rust.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "tokio".into(),
            category: "Async Runtime".into(),
            description: "Multi-threaded async runtime (adds full feature).".into(),
            features_hint: Some("full".into()),
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "anyhow".into(),
            category: "Error Handling".into(),
            description: "Flexible error propagation for Rust applications.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "thiserror".into(),
            category: "Error Handling".into(),
            description: "Derive macro for structured library error enums.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "reqwest".into(),
            category: "HTTP".into(),
            description: "Ergonomic async HTTP client with JSON support.".into(),
            features_hint: Some("json".into()),
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "clap".into(),
            category: "CLI".into(),
            description: "Command-line argument parser with derive macros.".into(),
            features_hint: Some("derive".into()),
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "tracing".into(),
            category: "Observability".into(),
            description: "Structured async logging and diagnostics.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "chrono".into(),
            category: "Date & Time".into(),
            description: "Timezone-aware date and time handling.".into(),
            features_hint: Some("serde".into()),
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "uuid".into(),
            category: "Utilities".into(),
            description: "UUID generation and parsing (v4 + serde).".into(),
            features_hint: Some("v4,serde".into()),
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "regex".into(),
            category: "Text".into(),
            description: "Linear-time regular expressions for Rust.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "axum".into(),
            category: "Web".into(),
            description: "Ergonomic, modular Tokio-native web framework.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "rand".into(),
            category: "Utilities".into(),
            description: "Random number generation utilities.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "walkdir".into(),
            category: "Filesystem".into(),
            description: "Cross-platform recursive directory traversal.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "zip".into(),
            category: "Archives".into(),
            description: "Read and write ZIP archives in pure Rust.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "dirs".into(),
            category: "Platform".into(),
            description: "Standard user config, cache, and home directories.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "futures".into(),
            category: "Async Runtime".into(),
            description: "Abstractions and combinators for asynchronous streams and futures.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "cargo".into(),
            name: "rayon".into(),
            category: "Parallelism".into(),
            description: "Data-parallelism library with work-stealing parallel iterators.".into(),
            features_hint: None,
        },
        // Python (pip)
        CuratedDependency {
            manager: "python".into(),
            name: "requests".into(),
            category: "HTTP".into(),
            description: "Simple, battle-tested synchronous HTTP client.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "httpx".into(),
            category: "HTTP".into(),
            description: "Modern async & HTTP/2 client for Python.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "pydantic".into(),
            category: "Validation".into(),
            description: "Data validation and typed settings using Python type hints.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "fastapi".into(),
            category: "Web".into(),
            description: "High-performance typed API framework.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "pytest".into(),
            category: "Testing".into(),
            description: "Clean, extensible Python testing framework.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "numpy".into(),
            category: "Science".into(),
            description: "Fast N-dimensional numerical computing arrays.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "pandas".into(),
            category: "Data".into(),
            description: "Tabular data analysis and DataFrame toolkit.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "matplotlib".into(),
            category: "Visualization".into(),
            description: "Publication-quality plotting and charts.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "rich".into(),
            category: "CLI".into(),
            description: "Rich terminal formatting, tables, progress bars, and tracebacks.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "pyyaml".into(),
            category: "Config".into(),
            description: "YAML parser and emitter for Python.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "beautifulsoup4".into(),
            category: "Parsing".into(),
            description: "HTML/XML parsing and scraping helpers.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "scipy".into(),
            category: "Science".into(),
            description: "Scientific computing algorithms (optimization, stats, linear algebra).".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "pillow".into(),
            category: "Imaging".into(),
            description: "Python Imaging Library for opening, manipulating, and saving images.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "cryptography".into(),
            category: "Security".into(),
            description: "Cryptographic recipes and primitives for Python.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "sympy".into(),
            category: "Math".into(),
            description: "Symbolic mathematics library for algebra, calculus, and equations.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "tqdm".into(),
            category: "CLI".into(),
            description: "Fast, extensible progress bars for loops and CLI pipelines.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "python".into(),
            name: "ruff".into(),
            category: "Quality".into(),
            description: "Extremely fast Python linter and code formatter.".into(),
            features_hint: None,
        },
        // Node / npm
        CuratedDependency {
            manager: "npm".into(),
            name: "typescript".into(),
            category: "Language".into(),
            description: "TypeScript compiler and static type checker.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "zod".into(),
            category: "Validation".into(),
            description: "TypeScript-first schema validation with static inference.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "axios".into(),
            category: "HTTP".into(),
            description: "Promise-based HTTP client for browser and Node.js.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "express".into(),
            category: "Web".into(),
            description: "Minimalist web server framework for Node.js.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "vitest".into(),
            category: "Testing".into(),
            description: "Fast Vite-native unit testing framework.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "eslint".into(),
            category: "Quality".into(),
            description: "Pluggable JavaScript/TypeScript linter.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "prettier".into(),
            category: "Quality".into(),
            description: "Opinionated code formatter.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "dotenv".into(),
            category: "Config".into(),
            description: "Load environment variables from .env files.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "vite".into(),
            category: "Build".into(),
            description: "Fast frontend build tool and development server.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "tsx".into(),
            category: "Runtime".into(),
            description: "Run TypeScript files seamlessly in Node.js.".into(),
            features_hint: None,
        },
        CuratedDependency {
            manager: "npm".into(),
            name: "lucide-react".into(),
            category: "UI Icons".into(),
            description: "Clean, consistent SVG icon library for React.".into(),
            features_hint: None,
        },
    ]
}

pub fn is_dependency_allowlisted(manager: &str, package: &str) -> Option<CuratedDependency> {
    let m = manager.trim().to_ascii_lowercase();
    let p = package.trim().to_ascii_lowercase();
    curated_dependency_allowlist()
        .into_iter()
        .find(|d| d.manager == m && d.name.eq_ignore_ascii_case(&p))
}

pub fn inspect_project_dependencies(workspace_root: &Path) -> ProjectDependenciesOverview {
    let mut ecosystems = Vec::new();
    let mut installed: HashMap<String, Vec<String>> = HashMap::new();

    // Cargo.toml
    let cargo_toml = workspace_root.join("Cargo.toml");
    if cargo_toml.exists() {
        ecosystems.push("cargo".into());
        if let Ok(raw) = std::fs::read_to_string(&cargo_toml) {
            let mut found = HashSet::new();
            let mut in_deps = false;
            for line in raw.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    in_deps = l.contains("dependencies");
                    continue;
                }
                if in_deps && !l.is_empty() && !l.starts_with('#') {
                    if let Some((k, _)) = l.split_once('=') {
                        let dep = k.trim().trim_matches('"');
                        if !dep.is_empty() {
                            found.insert(dep.to_string());
                        }
                    }
                }
            }
            let mut v: Vec<String> = found.into_iter().collect();
            v.sort();
            installed.insert("cargo".into(), v);
        }
    }

    // Python
    let req_txt = workspace_root.join("requirements.txt");
    let pyproj = workspace_root.join("pyproject.toml");
    let has_py_files = std::fs::read_dir(workspace_root)
        .ok()
        .map(|rd| {
            rd.flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("py"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if req_txt.exists() || pyproj.exists() || has_py_files {
        ecosystems.push("python".into());
        let mut found = HashSet::new();
        if let Ok(raw) = std::fs::read_to_string(&req_txt) {
            for line in raw.lines() {
                let l = line.trim();
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                let name = l
                    .split(|c: char| matches!(c, '=' | '>' | '<' | '~' | '!' | '[' | ' '))
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() {
                    found.insert(name.to_ascii_lowercase());
                }
            }
        }
        let mut v: Vec<String> = found.into_iter().collect();
        v.sort();
        installed.insert("python".into(), v);
    }

    // npm (package.json)
    let pkg_json = workspace_root.join("package.json");
    if pkg_json.exists() {
        ecosystems.push("npm".into());
        let mut found = HashSet::new();
        if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                for sec in ["dependencies", "devDependencies"] {
                    if let Some(obj) = val.get(sec).and_then(|v| v.as_object()) {
                        for k in obj.keys() {
                            found.insert(k.clone());
                        }
                    }
                }
            }
        }
        let mut v: Vec<String> = found.into_iter().collect();
        v.sort();
        installed.insert("npm".into(), v);
    }

    ProjectDependenciesOverview {
        workspace_path: workspace_root.to_string_lossy().to_string(),
        detected_ecosystems: ecosystems,
        installed_by_manager: installed,
        allowlist: curated_dependency_allowlist(),
    }
}

fn sanitize_crate_name(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.starts_with('-') || s.starts_with('_') || s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        if s.starts_with('-') || s.starts_with('_') {
            s.remove(0);
        } else {
            s = format!("app-{s}");
            break;
        }
    }
    if s.is_empty() {
        "cinder-project".into()
    } else {
        s
    }
}

fn upsert_toml_dependency_line(raw: &str, section: &str, pkg: &str, resolved_line: &str) -> String {
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_target = false;
    let mut replaced = false;
    let mut saw_section = false;
    let mut insert_after_idx: Option<usize> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_target = trimmed == section;
            out_lines.push(line.to_string());
            if in_target {
                saw_section = true;
                insert_after_idx = Some(out_lines.len());
            }
            continue;
        }
        if in_target && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((k, _)) = trimmed.split_once('=') {
                let key = k.trim().trim_matches('"');
                if key.eq_ignore_ascii_case(pkg) {
                    out_lines.push(resolved_line.to_string());
                    replaced = true;
                    continue;
                }
            }
        }
        out_lines.push(line.to_string());
    }

    if !replaced {
        if saw_section {
            if let Some(idx) = insert_after_idx {
                out_lines.insert(idx, resolved_line.to_string());
            }
        } else {
            if !out_lines.is_empty() && !out_lines.last().map(|l| l.is_empty()).unwrap_or(true) {
                out_lines.push(String::new());
            }
            out_lines.push(section.to_string());
            out_lines.push(resolved_line.to_string());
        }
    }

    let mut joined = out_lines.join("\n");
    if !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

fn remove_toml_dependency_line(raw: &str, pkg: &str) -> (String, bool) {
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_deps = false;
    let mut removed = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed.contains("dependencies");
            out_lines.push(line.to_string());
            continue;
        }
        if in_deps && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((k, _)) = trimmed.split_once('=') {
                let key = k.trim().trim_matches('"');
                if key.eq_ignore_ascii_case(pkg) {
                    removed = true;
                    continue;
                }
            }
        }
        out_lines.push(line.to_string());
    }

    let mut joined = out_lines.join("\n");
    if !joined.is_empty() && !joined.ends_with('\n') {
        joined.push('\n');
    }
    (joined, removed)
}

/// Remove a previously added project dependency cleanly from `Cargo.toml`, `requirements.txt`/`.venv`, or `package.json`.
pub fn uninstall_project_dependency(
    workspace_root: &Path,
    manager: &str,
    package: &str,
) -> anyhow::Result<String> {
    let mgr = manager.trim().to_ascii_lowercase();
    let pkg = package.trim();
    match mgr.as_str() {
        "cargo" => {
            let ws_cargo = workspace_root.join("Cargo.toml");
            if !ws_cargo.exists() {
                anyhow::bail!("No Cargo.toml found in active workspace.");
            }
            let raw = std::fs::read_to_string(&ws_cargo)?;
            let (updated, removed) = remove_toml_dependency_line(&raw, pkg);
            if removed {
                let tmp = ws_cargo.with_extension("toml.cinder_tmp");
                std::fs::write(&tmp, updated)?;
                std::fs::rename(&tmp, &ws_cargo)?;
                Ok(format!("Removed `{pkg}` from Cargo.toml"))
            } else {
                Ok(format!("`{pkg}` was not listed in Cargo.toml"))
            }
        }
        "python" => {
            let req_path = workspace_root.join("requirements.txt");
            let mut removed_req = false;
            if req_path.exists() {
                let raw = std::fs::read_to_string(&req_path)?;
                let mut kept = Vec::new();
                for line in raw.lines() {
                    let name = line
                        .trim()
                        .split(|c: char| matches!(c, '=' | '>' | '<' | '~' | '!' | '[' | ' '))
                        .next()
                        .unwrap_or("")
                        .trim();
                    if name.eq_ignore_ascii_case(pkg) {
                        removed_req = true;
                    } else {
                        kept.push(line);
                    }
                }
                if removed_req {
                    let mut next = kept.join("\n");
                    if !next.is_empty() && !next.ends_with('\n') {
                        next.push('\n');
                    }
                    let tmp = req_path.with_extension("txt.cinder_tmp");
                    std::fs::write(&tmp, next)?;
                    std::fs::rename(&tmp, &req_path)?;
                }
            }

            #[cfg(windows)]
            let venv_py = workspace_root.join(".venv").join("Scripts").join("python.exe");
            #[cfg(not(windows))]
            let venv_py = workspace_root.join(".venv").join("bin").join("python");
            if venv_py.exists() {
                let mut cmd = std::process::Command::new(&venv_py);
                cmd.args(["-m", "pip", "uninstall", "-y", pkg]);
                crate::scrub_sensitive_std_env(&mut cmd);
                configure_no_window_std(&mut cmd);
                let _ = cmd.output();
            }

            Ok(format!("Removed Python package `{pkg}`"))
        }
        "npm" => {
            let pkg_json = workspace_root.join("package.json");
            if !pkg_json.exists() {
                anyhow::bail!("No package.json found in active workspace.");
            }
            let raw = std::fs::read_to_string(&pkg_json)?;
            let mut val: Value = serde_json::from_str(&raw)?;
            let mut removed = false;
            for sec in ["dependencies", "devDependencies"] {
                if let Some(obj) = val.get_mut(sec).and_then(|v| v.as_object_mut()) {
                    if obj.remove(pkg).is_some() {
                        removed = true;
                    }
                }
            }
            if removed {
                let pretty = format!("{}\n", serde_json::to_string_pretty(&val)?);
                let tmp = pkg_json.with_extension("json.cinder_tmp");
                std::fs::write(&tmp, pretty)?;
                std::fs::rename(&tmp, &pkg_json)?;
            }
            Ok(format!("Removed npm package `{pkg}` from package.json"))
        }
        other => anyhow::bail!("Unsupported package manager '{other}'"),
    }
}

/// Validate a user-entered custom dependency package name safely (blocking flags, traversal, and shell metacharacters).
pub fn validate_custom_package_name(manager: &str, package: &str) -> anyhow::Result<CuratedDependency> {
    let mgr = manager.trim().to_ascii_lowercase();
    if !matches!(mgr.as_str(), "cargo" | "python" | "npm") {
        anyhow::bail!("Unsupported package manager '{manager}'");
    }
    if let Some(curated) = is_dependency_allowlisted(&mgr, package) {
        return Ok(curated);
    }

    let trimmed = package.trim();
    if trimmed.is_empty() || trimmed.len() > 80 {
        anyhow::bail!("Package name must be between 1 and 80 characters.");
    }
    if trimmed.starts_with('-') || trimmed.starts_with('.') || trimmed.contains("..") {
        anyhow::bail!("Invalid package name '{trimmed}': leading dashes/dots and '..' are rejected.");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | '/'))
    {
        anyhow::bail!("Package name '{trimmed}' contains invalid characters.");
    }
    if trimmed.contains('@') {
        if mgr != "npm" || !trimmed.starts_with('@') || trimmed[1..].contains('@') {
            anyhow::bail!("'@' is only permitted at the start of scoped npm packages.");
        }
    }
    if trimmed.contains('/') {
        let slash_count = trimmed.matches('/').count();
        if mgr != "npm"
            || !trimmed.starts_with('@')
            || slash_count != 1
            || trimmed.ends_with('/')
            || trimmed.starts_with("@/")
        {
            anyhow::bail!("'/' is only permitted once in scoped npm packages like @scope/pkg.");
        }
    }

    Ok(CuratedDependency {
        manager: mgr.clone(),
        name: trimmed.to_string(),
        category: "Custom".into(),
        description: format!("Custom {mgr} package staged in isolated temp folder."),
        features_hint: None,
    })
}

/// Install a curated allow-listed dependency into `workspace_root`, ALWAYS staging & verifying
/// in an isolated temporary folder first so failures never corrupt the project.
pub async fn install_project_dependency<F>(
    gate: &NetworkGate,
    paths: &BumboPaths,
    workspace_root: &Path,
    manager: &str,
    package: &str,
    on_log: F,
) -> anyhow::Result<String>
where
    F: FnMut(u8, String) + Send,
{
    let entry = is_dependency_allowlisted(manager, package).ok_or_else(|| {
        anyhow::anyhow!(
            "Package '{package}' ({manager}) is not on Cinder's curated allow-list. Only curated packages can be one-click installed."
        )
    })?;
    install_resolved_dependency(gate, paths, workspace_root, entry, on_log).await
}

/// Install any validated custom dependency into `workspace_root` via isolated temp-folder staging.
pub async fn install_custom_project_dependency<F>(
    gate: &NetworkGate,
    paths: &BumboPaths,
    workspace_root: &Path,
    manager: &str,
    package: &str,
    on_log: F,
) -> anyhow::Result<String>
where
    F: FnMut(u8, String) + Send,
{
    let entry = validate_custom_package_name(manager, package)?;
    install_resolved_dependency(gate, paths, workspace_root, entry, on_log).await
}

async fn install_resolved_dependency<F>(
    gate: &NetworkGate,
    paths: &BumboPaths,
    workspace_root: &Path,
    entry: CuratedDependency,
    mut on_log: F,
) -> anyhow::Result<String>
where
    F: FnMut(u8, String) + Send,
{

    if gate.mode() == PrivacyMode::StrictLocal {
        anyhow::bail!(
            "Strict Local Mode blocked outbound package download for '{}'. Switch to Hybrid Mode to install dependencies.",
            entry.name
        );
    }

    if !workspace_root.exists() {
        anyhow::bail!(
            "Workspace folder '{}' does not exist.",
            workspace_root.display()
        );
    }

    let stage_root = std::env::temp_dir();
    let stage = TempStageDir::create(&stage_root, &format!("dep-{}-{}", entry.manager, entry.name))?;
    on_log(
        10,
        format!(
            "Created isolated temp staging folder: {}",
            stage.path().display()
        ),
    );

    match entry.manager.as_str() {
        "cargo" => {
            // Audit registry access via NetworkGate policy (already verified Hybrid)
            let _ = gate.request(Method::HEAD, "https://crates.io", "cargo_dependency_install");
            let cargo_bin = resolve_cargo_executable().ok_or_else(|| {
                anyhow::anyhow!("Rust / Cargo executable not found (~/.cargo/bin or PATH).")
            })?;
            let cargo_path_env = prepend_tool_dir_to_path(&cargo_bin);

            let ws_cargo = workspace_root.join("Cargo.toml");
            if !ws_cargo.exists() {
                let raw_name = workspace_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("cinder-project");
                let crate_name = sanitize_crate_name(raw_name);
                std::fs::write(
                    &ws_cargo,
                    format!(
                        "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
                    ),
                )?;
                let src_dir = workspace_root.join("src");
                if !src_dir.join("main.rs").exists() && !src_dir.join("lib.rs").exists() {
                    std::fs::create_dir_all(&src_dir)?;
                    std::fs::write(src_dir.join("lib.rs"), "")?;
                }
                on_log(
                    20,
                    format!("Initialized workspace Cargo.toml for crate `{crate_name}`."),
                );
            }

            // Stage a clean dummy crate in the temp folder so `cargo add` resolves & downloads
            // from crates.io completely isolated before touching the project's Cargo.toml.
            std::fs::create_dir_all(stage.path().join("src"))?;
            std::fs::write(
                stage.path().join("Cargo.toml"),
                "[package]\nname = \"cinder-stage-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )?;
            std::fs::write(stage.path().join("src").join("lib.rs"), "")?;

            let mut add_args = vec!["add".to_string(), entry.name.clone()];
            if let Some(ref feats) = entry.features_hint {
                add_args.push("--features".into());
                add_args.push(feats.clone());
            }

            on_log(
                35,
                format!("Staging `cargo {}` in temp folder...", add_args.join(" ")),
            );
            let mut cmd = tokio::process::Command::new(&cargo_bin);
            cmd.args(&add_args)
                .current_dir(stage.path())
                .env("PATH", &cargo_path_env);
            crate::scrub_sensitive_child_env(&mut cmd);
            configure_no_window_tokio(&mut cmd);
            let out = cmd.output().await?;
            let combined = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            if !out.status.success() {
                anyhow::bail!(
                    "Temp staging failed (project untouched):\n{}",
                    combined.trim()
                );
            }

            // Extract resolved spec line from staged Cargo.toml
            let staged_toml = std::fs::read_to_string(stage.path().join("Cargo.toml"))?;
            let resolved_line = staged_toml
                .lines()
                .find(|l| {
                    l.trim_start().starts_with(&format!("{} ", entry.name))
                        || l.trim_start().starts_with(&format!("{}=", entry.name))
                })
                .unwrap_or("")
                .trim()
                .to_string();
            on_log(
                70,
                format!(
                    "Verified in temp stage ({resolved_line}). Committing to workspace Cargo.toml..."
                ),
            );

            let ws_raw = std::fs::read_to_string(&ws_cargo)?;
            let is_virtual_workspace =
                ws_raw.contains("[workspace]") && !ws_raw.contains("[package]");

            if is_virtual_workspace {
                let updated = upsert_toml_dependency_line(
                    &ws_raw,
                    "[workspace.dependencies]",
                    &entry.name,
                    &resolved_line,
                );
                let tmp_toml = ws_cargo.with_extension("toml.cinder_tmp");
                std::fs::write(&tmp_toml, updated)?;
                std::fs::rename(&tmp_toml, &ws_cargo)?;
            } else {
                let mut commit_cmd = tokio::process::Command::new(&cargo_bin);
                commit_cmd
                    .arg("add")
                    .arg("--offline")
                    .arg(&entry.name)
                    .current_dir(workspace_root)
                    .env("PATH", &cargo_path_env);
                crate::scrub_sensitive_child_env(&mut commit_cmd);
                if let Some(ref feats) = entry.features_hint {
                    commit_cmd.arg("--features").arg(feats);
                }
                configure_no_window_tokio(&mut commit_cmd);
                let c_out = commit_cmd.output().await?;
                if !c_out.status.success() {
                    let mut retry = tokio::process::Command::new(&cargo_bin);
                    retry
                        .args(&add_args)
                        .current_dir(workspace_root)
                        .env("PATH", &cargo_path_env);
                    crate::scrub_sensitive_child_env(&mut retry);
                    configure_no_window_tokio(&mut retry);
                    let r_out = retry.output().await?;
                    if !r_out.status.success() && !resolved_line.is_empty() {
                        let updated = upsert_toml_dependency_line(
                            &ws_raw,
                            "[dependencies]",
                            &entry.name,
                            &resolved_line,
                        );
                        let tmp_toml = ws_cargo.with_extension("toml.cinder_tmp");
                        std::fs::write(&tmp_toml, updated)?;
                        std::fs::rename(&tmp_toml, &ws_cargo)?;
                    } else if !r_out.status.success() {
                        anyhow::bail!(
                            "Commit to workspace Cargo.toml failed: {}",
                            String::from_utf8_lossy(&r_out.stderr).trim()
                        );
                    }
                }
            }

            on_log(
                100,
                format!(
                    "Installed Rust crate `{}` cleanly (temp folder cleaned).",
                    entry.name
                ),
            );
            Ok(format!("Installed `{}` ({resolved_line})", entry.name))
        }
        "python" => {
            let _ = gate.request(Method::HEAD, "https://pypi.org", "pip_dependency_install");
            let py = resolve_python_executable()
                .ok_or_else(|| anyhow::anyhow!("Python was not found on PATH."))?;

            let wheels_dir = stage.path().join("wheels");
            std::fs::create_dir_all(&wheels_dir)?;

            on_log(
                30,
                format!(
                    "Downloading & verifying wheels for `{}` inside temp folder...",
                    entry.name
                ),
            );
            let mut dl = tokio::process::Command::new(&py);
            dl.args([
                "-m",
                "pip",
                "download",
                "--only-binary=:all:",
                "--no-input",
                "--disable-pip-version-check",
                "-d",
            ])
            .arg(&wheels_dir)
            .arg(&entry.name);
            crate::scrub_sensitive_child_env(&mut dl);
            configure_no_window_tokio(&mut dl);
            let dl_out = dl.output().await?;
            if !dl_out.status.success() {
                anyhow::bail!(
                    "Temp wheel download failed (project untouched):\n{}",
                    String::from_utf8_lossy(&dl_out.stderr).trim()
                );
            }

            // Ensure project-local `.venv` with working interpreter
            let venv_dir = workspace_root.join(".venv");
            #[cfg(windows)]
            let venv_py = venv_dir.join("Scripts").join("python.exe");
            #[cfg(not(windows))]
            let venv_py = venv_dir.join("bin").join("python");

            if !venv_py.exists() {
                on_log(55, "Creating isolated workspace `.venv`...".into());
                let mut mk = tokio::process::Command::new(&py);
                mk.args(["-m", "venv"]).arg(&venv_dir);
                crate::scrub_sensitive_child_env(&mut mk);
                configure_no_window_tokio(&mut mk);
                let mk_out = mk.output().await?;
                if !mk_out.status.success() {
                    anyhow::bail!(
                        "Failed to create workspace .venv: {}",
                        String::from_utf8_lossy(&mk_out.stderr).trim()
                    );
                }
            }

            on_log(
                75,
                format!(
                    "Installing verified `{}` from temp wheels into `.venv`...",
                    entry.name
                ),
            );
            let mut inst = tokio::process::Command::new(&venv_py);
            inst.args([
                "-m",
                "pip",
                "install",
                "--only-binary=:all:",
                "--no-input",
                "--disable-pip-version-check",
                "--no-index",
                "--find-links",
            ])
            .arg(&wheels_dir)
            .arg(&entry.name);
            crate::scrub_sensitive_child_env(&mut inst);
            configure_no_window_tokio(&mut inst);
            let inst_out = inst.output().await?;
            if !inst_out.status.success() {
                anyhow::bail!(
                    "Install into workspace .venv failed:\n{}",
                    String::from_utf8_lossy(&inst_out.stderr).trim()
                );
            }

            // Append cleanly to requirements.txt if not already present
            let req_path = workspace_root.join("requirements.txt");
            let existing = std::fs::read_to_string(&req_path).unwrap_or_default();
            let already = existing.lines().any(|l| {
                l.trim()
                    .split(|c: char| matches!(c, '=' | '>' | '<' | '~' | '!' | '[' | ' '))
                    .next()
                    .map(|n| n.eq_ignore_ascii_case(&entry.name))
                    .unwrap_or(false)
            });
            if !already {
                let mut next = existing;
                if !next.is_empty() && !next.ends_with('\n') {
                    next.push('\n');
                }
                next.push_str(&format!("{}\n", entry.name));
                let tmp_req = req_path.with_extension("txt.cinder_tmp");
                std::fs::write(&tmp_req, next)?;
                std::fs::rename(&tmp_req, &req_path)?;
            }

            on_log(
                100,
                format!(
                    "Installed Python package `{}` into `.venv` & requirements.txt (temp folder cleaned).",
                    entry.name
                ),
            );
            Ok(format!("Installed `{}` into workspace `.venv`", entry.name))
        }
        "npm" => {
            let _ = gate.request(
                Method::HEAD,
                "https://registry.npmjs.org",
                "npm_dependency_install",
            );
            let node_stage_root = paths.app_data_dir.join("tmp_stage");
            let (node_path, npm_path, _) =
                ensure_node_and_npm(gate, paths, &node_stage_root, false, &mut on_log).await?;

            let ws_pkg = workspace_root.join("package.json");
            if !ws_pkg.exists() {
                std::fs::write(
                    stage.path().join("package.json"),
                    "{\n  \"name\": \"cinder-workspace\",\n  \"version\": \"0.1.0\",\n  \"private\": true\n}\n",
                )?;
            } else {
                std::fs::copy(&ws_pkg, stage.path().join("package.json"))?;
                let ws_lock = workspace_root.join("package-lock.json");
                if ws_lock.exists() {
                    let _ = std::fs::copy(&ws_lock, stage.path().join("package-lock.json"));
                }
            }

            on_log(
                68,
                format!("Staging `npm install {}` inside temp folder...", entry.name),
            );
            let mut cmd = build_npm_tokio_command(&node_path, &npm_path);
            cmd.args(["install", "--ignore-scripts", "--no-audit", "--no-fund", &entry.name])
                .current_dir(stage.path());
            let out = cmd.output().await?;
            if !out.status.success() {
                anyhow::bail!(
                    "Temp npm staging failed (project untouched):\n{}{}",
                    String::from_utf8_lossy(&out.stdout).trim(),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }

            on_log(
                88,
                "Verified in temp stage. Committing package.json & node_modules to workspace..."
                    .into(),
            );
            std::fs::copy(stage.path().join("package.json"), &ws_pkg)?;
            if stage.path().join("package-lock.json").exists() {
                let _ = std::fs::copy(
                    stage.path().join("package-lock.json"),
                    workspace_root.join("package-lock.json"),
                );
            }
            if stage.path().join("node_modules").exists() {
                copy_dir_recursive(
                    &stage.path().join("node_modules"),
                    &workspace_root.join("node_modules"),
                )?;
            }

            on_log(
                100,
                format!(
                    "Installed npm package `{}` cleanly (temp folder cleaned).",
                    entry.name
                ),
            );
            Ok(format!("Installed npm package `{}`", entry.name))
        }
        other => anyhow::bail!("Unsupported package manager '{other}'"),
    }
}

// ============================================================================
// Curated Allow-List: Plugins (MCP Servers, Skill Packs, Portable Toolchains)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratedPluginEntry {
    pub id: String,
    pub name: String,
    pub kind: String, // "mcp_python" | "mcp_node" | "skill_pack" | "toolchain"
    pub category: String,
    pub version: String,
    pub package_spec: String,
    pub entrypoint: String,
    pub description: String,
    pub requires_toolchain: Option<String>,
    pub needs_network: bool,
    pub read_only: bool,
    pub antigravity_compatible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCatalogView {
    #[serde(flatten)]
    pub entry: CuratedPluginEntry,
    pub installed: bool,
    pub enabled: bool,
    pub install_path: Option<String>,
    pub mcp_server_id: Option<String>,
}

pub fn curated_plugin_catalog() -> Vec<CuratedPluginEntry> {
    vec![
        // Verified Python MCP servers (work immediately with system Python via temp-staged venv)
        CuratedPluginEntry {
            id: "mcp-time".into(),
            name: "Time & Timezone MCP".into(),
            kind: "mcp_python".into(),
            category: "MCP Tool Server".into(),
            version: "2026.11.18".into(),
            package_spec: "mcp-server-time==2026.11.18".into(),
            entrypoint: "mcp_server_time".into(),
            description: "Official MCP server adding get_current_time, convert_time, and get_time_ago tools. 100% offline.".into(),
            requires_toolchain: Some("python".into()),
            needs_network: false,
            read_only: true,
            antigravity_compatible: true,
        },
        CuratedPluginEntry {
            id: "mcp-git".into(),
            name: "Git Repository Inspector MCP".into(),
            kind: "mcp_python".into(),
            category: "MCP Tool Server".into(),
            version: "2026.11.18".into(),
            package_spec: "mcp-server-git==2026.11.18".into(),
            entrypoint: "mcp_server_git".into(),
            description: "Official MCP server for structured git status, diffs, commits, log, and branch inspection. 100% offline.".into(),
            requires_toolchain: Some("python".into()),
            needs_network: false,
            read_only: false,
            antigravity_compatible: true,
        },
        CuratedPluginEntry {
            id: "mcp-fetch".into(),
            name: "Web Fetch & Markdown MCP".into(),
            kind: "mcp_python".into(),
            category: "MCP Tool Server".into(),
            version: "2026.11.18".into(),
            package_spec: "mcp-server-fetch==2026.11.18".into(),
            entrypoint: "mcp_server_fetch".into(),
            description: "Official MCP server that fetches web pages and converts them into clean Markdown for the agent (requires Hybrid Mode).".into(),
            requires_toolchain: Some("python".into()),
            needs_network: true,
            read_only: false,
            antigravity_compatible: true,
        },
        // Verified Node.js MCP servers (work with system Node or Cinder Portable Node.js LTS)
        CuratedPluginEntry {
            id: "mcp-memory".into(),
            name: "Knowledge Graph Memory MCP".into(),
            kind: "mcp_node".into(),
            category: "MCP Tool Server".into(),
            version: "2.2.1".into(),
            package_spec: "@modelcontextprotocol/server-memory@2.2.1".into(),
            entrypoint: "@modelcontextprotocol/server-memory/dist/index.js".into(),
            description: "Persistent entity & relation knowledge graph memory across sessions. 100% offline.".into(),
            requires_toolchain: Some("node".into()),
            needs_network: false,
            read_only: false,
            antigravity_compatible: true,
        },
        CuratedPluginEntry {
            id: "mcp-sequential-thinking".into(),
            name: "Sequential Thinking MCP".into(),
            kind: "mcp_node".into(),
            category: "MCP Tool Server".into(),
            version: "2.2.0".into(),
            package_spec: "@modelcontextprotocol/server-sequential-thinking@2.2.0".into(),
            entrypoint: "@modelcontextprotocol/server-sequential-thinking/dist/index.js".into(),
            description: "Structured hypothesis, branching, and revision reasoning tool for complex debugging & architecture. 100% offline.".into(),
            requires_toolchain: Some("node".into()),
            needs_network: false,
            read_only: true,
            antigravity_compatible: true,
        },
        CuratedPluginEntry {
            id: "mcp-context7".into(),
            name: "Context7 Live Docs MCP".into(),
            kind: "mcp_node".into(),
            category: "MCP Tool Server".into(),
            version: "4.7.3".into(),
            package_spec: "@upstash/context7-mcp@4.7.3".into(),
            entrypoint: "@upstash/context7-mcp/dist/index.js".into(),
            description: "Pulls up-to-date, version-accurate library documentation directly into agent context (requires Hybrid Mode).".into(),
            requires_toolchain: Some("node".into()),
            needs_network: true,
            read_only: true,
            antigravity_compatible: true,
        },
        // Managed Portable Toolchain
        CuratedPluginEntry {
            id: "toolchain-nodejs".into(),
            name: "Portable Node.js LTS Runtime".into(),
            kind: "toolchain".into(),
            category: "Managed Runtime".into(),
            version: "LTS".into(),
            package_spec: "nodejs-lts".into(),
            entrypoint: if cfg!(windows) { "node.exe" } else { "bin/node" }.into(),
            description: "Downloads official Node.js LTS archive into Cinder's local toolchains folder — unlocks Node MCP servers & npm dependencies without system install.".into(),
            requires_toolchain: None,
            needs_network: true,
            read_only: true,
            antigravity_compatible: true,
        },
        // Curated Antigravity-Compatible SKILL.md Packs (instant offline 1-click)
        CuratedPluginEntry {
            id: "skill-rust-audit".into(),
            name: "Rust Safety & Perf Audit (/rust-audit)".into(),
            kind: "skill_pack".into(),
            category: "Antigravity Skill".into(),
            version: "1.0.0".into(),
            package_spec: "rust-audit".into(),
            entrypoint: "rust-audit".into(),
            description: "Installs `/rust-audit` SKILL.md for deep borrow-checker, unsafe, unwrap, concurrency, and allocation audits.".into(),
            requires_toolchain: None,
            needs_network: false,
            read_only: true,
            antigravity_compatible: true,
        },
        CuratedPluginEntry {
            id: "skill-test-gen".into(),
            name: "Automated Test Generator (/test-gen)".into(),
            kind: "skill_pack".into(),
            category: "Antigravity Skill".into(),
            version: "1.0.0".into(),
            package_spec: "test-gen".into(),
            entrypoint: "test-gen".into(),
            description: "Installs `/test-gen` SKILL.md that analyzes edge cases, generates unit/integration tests, and verifies them with verify_workspace.".into(),
            requires_toolchain: None,
            needs_network: false,
            read_only: true,
            antigravity_compatible: true,
        },
        CuratedPluginEntry {
            id: "skill-docs-writer".into(),
            name: "Architecture & API Docs (/docs-writer)".into(),
            kind: "skill_pack".into(),
            category: "Antigravity Skill".into(),
            version: "1.0.0".into(),
            package_spec: "docs-writer".into(),
            entrypoint: "docs-writer".into(),
            description: "Installs `/docs-writer` SKILL.md to produce crisp READMEs, Mermaid architecture diagrams, and verified API references.".into(),
            requires_toolchain: None,
            needs_network: false,
            read_only: true,
            antigravity_compatible: true,
        },
        CuratedPluginEntry {
            id: "skill-refactor-clean".into(),
            name: "Safe Modular Refactor (/refactor-clean)".into(),
            kind: "skill_pack".into(),
            category: "Antigravity Skill".into(),
            version: "1.0.0".into(),
            package_spec: "refactor-clean".into(),
            entrypoint: "refactor-clean".into(),
            description: "Installs `/refactor-clean` SKILL.md for checkpointed, behavior-preserving modular refactors with test verification.".into(),
            requires_toolchain: None,
            needs_network: false,
            read_only: true,
            antigravity_compatible: true,
        },
    ]
}

fn curated_skill_markdown(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "skill-rust-audit" => Some((
            "rust-audit",
            "---\nname: Rust Safety & Perf Audit\ndescription: Audit Rust crates for panics, unwraps, lock-across-await, allocations, and clippy issues.\nmode: plan\n---\nPerform a thorough Rust audit of the active workspace ({input}).\n1. Inspect `Cargo.toml` and key `.rs` files using read-only tools.\n2. Flag `.unwrap()`/`.expect()` on fallible paths, blocking calls inside async contexts, lock guards held across `.await`, and unnecessary clones.\n3. Check security boundaries (path traversal, command injection, network gate enforcement).\n4. Present findings ordered by severity with exact file paths, line numbers, and minimal fix patches.\n",
        )),
        "skill-test-gen" => Some((
            "test-gen",
            "---\nname: Automated Test Generator\ndescription: Design and implement targeted unit and integration tests for edge cases and regressions.\nmode: agent\n---\nGenerate high-signal automated tests for: {input}.\n1. Read the target implementation and existing tests first.\n2. Identify boundary conditions, error paths, and happy paths.\n3. Add concise, deterministic tests (no flaky network or wall-clock dependencies).\n4. Run `verify_workspace` to confirm all new and existing tests pass cleanly.\n",
        )),
        "skill-docs-writer" => Some((
            "docs-writer",
            "---\nname: Architecture & API Docs\ndescription: Generate clear, accurate READMEs, API docs, and Mermaid diagrams grounded in actual code.\nmode: agent\n---\nInspect the workspace codebase for: {input}.\n1. Read actual routes, CLI flags, config structs, and entrypoints (never invent undocumented flags).\n2. Produce concise, developer-friendly Markdown documentation with copy-pasteable quick-start commands and a Mermaid architecture diagram.\n3. Verify all referenced file paths and commands exist.\n",
        )),
        "skill-refactor-clean" => Some((
            "refactor-clean",
            "---\nname: Safe Modular Refactor\ndescription: Refactor code cleanly in small verified steps without breaking existing behavior.\nmode: agent\n---\nRefactor cleanly: {input}.\n1. Inspect callers and tests before editing.\n2. Extract cohesive modules/functions with clear interfaces and minimal churn.\n3. Run `verify_workspace` after edits and fix any compiler warnings or test regressions.\n",
        )),
        _ => None,
    }
}

pub fn list_plugin_catalog_with_status(
    paths: &BumboPaths,
    mcp_servers: &[McpServerEntry],
) -> Vec<PluginCatalogView> {
    let plugins_dir = paths.plugins_dir();
    let skills_dir = paths.skills_dir();

    curated_plugin_catalog()
        .into_iter()
        .map(|entry| {
            let mcp_id = format!("plugin::{}", entry.id);
            match entry.kind.as_str() {
                "mcp_python" | "mcp_node" => {
                    let dir = plugins_dir.join(&entry.id);
                    let registered = mcp_servers.iter().find(|s| s.id == mcp_id);
                    let installed = dir.exists() && dir.join("cinder-plugin.json").exists();
                    PluginCatalogView {
                        installed,
                        enabled: registered.map(|r| r.enabled).unwrap_or(false),
                        install_path: if installed {
                            Some(dir.to_string_lossy().to_string())
                        } else {
                            None
                        },
                        mcp_server_id: if installed { Some(mcp_id) } else { None },
                        entry,
                    }
                }
                "skill_pack" => {
                    let skill_file = skills_dir.join(&entry.entrypoint).join("SKILL.md");
                    let installed = skill_file.exists();
                    PluginCatalogView {
                        installed,
                        enabled: installed,
                        install_path: if installed {
                            Some(skill_file.to_string_lossy().to_string())
                        } else {
                            None
                        },
                        mcp_server_id: None,
                        entry,
                    }
                }
                "toolchain" => {
                    let node_ready = resolve_node_and_npm(paths);
                    let pdir = portable_node_dir(paths);
                    let portable_installed = pdir.exists() && node_ready.as_ref().map(|(_, _, s)| *s == "portable").unwrap_or(false);
                    PluginCatalogView {
                        installed: portable_installed,
                        enabled: node_ready.is_some(),
                        install_path: if portable_installed {
                            Some(pdir.to_string_lossy().to_string())
                        } else {
                            node_ready.map(|(p, _, _)| p.to_string_lossy().to_string())
                        },
                        mcp_server_id: None,
                        entry,
                    }
                }
                _ => PluginCatalogView {
                    installed: false,
                    enabled: false,
                    install_path: None,
                    mcp_server_id: None,
                    entry,
                },
            }
        })
        .collect()
}

pub struct PluginInstallOutcome {
    pub summary: String,
    pub mcp_entry: Option<McpServerEntry>,
    pub probe_result: Option<McpProbeResult>,
}

fn package_name_from_npm_spec(spec: &str) -> &str {
    if let Some(rest) = spec.strip_prefix('@') {
        if let Some(idx) = rest.rfind('@') {
            return &spec[..idx + 1];
        }
        return spec;
    }
    spec.split('@').next().unwrap_or(spec)
}

fn resolve_node_package_entrypoint(final_dir: &Path, entry: &CuratedPluginEntry) -> PathBuf {
    let pkg_name = package_name_from_npm_spec(&entry.package_spec);
    let pkg_dir = final_dir.join("node_modules").join(pkg_name);
    let pkg_json = pkg_dir.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
        if let Ok(val) = serde_json::from_str::<Value>(&raw) {
            if let Some(bin_str) = val.get("bin").and_then(|b| b.as_str()) {
                let cand = pkg_dir.join(bin_str);
                if cand.exists() {
                    return cand;
                }
            } else if let Some(bin_obj) = val.get("bin").and_then(|b| b.as_object()) {
                for v in bin_obj.values() {
                    if let Some(rel) = v.as_str() {
                        let cand = pkg_dir.join(rel);
                        if cand.exists() {
                            return cand;
                        }
                    }
                }
            }
            if let Some(main_str) = val.get("main").and_then(|m| m.as_str()) {
                let cand = pkg_dir.join(main_str);
                if cand.exists() {
                    return cand;
                }
            }
        }
    }
    final_dir.join("node_modules").join(&entry.entrypoint)
}

/// Install a curated allow-listed plugin/toolchain/skill via temp-folder staging.
pub async fn install_curated_plugin<F>(
    gate: &NetworkGate,
    paths: &BumboPaths,
    plugin_id: &str,
    mut on_log: F,
) -> anyhow::Result<PluginInstallOutcome>
where
    F: FnMut(u8, String) + Send,
{
    let entry = curated_plugin_catalog()
        .into_iter()
        .find(|p| p.id == plugin_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Plugin '{plugin_id}' is not on Cinder's curated allow-list."
            )
        })?;

    let stage_root = paths.app_data_dir.join("tmp_stage");

    match entry.kind.as_str() {
        "skill_pack" => {
            let (slug, md) = curated_skill_markdown(&entry.id)
                .ok_or_else(|| anyhow::anyhow!("Skill template missing for {}", entry.id))?;
            let stage = TempStageDir::create(&stage_root, &entry.id)?;
            on_log(
                30,
                format!("Staged Antigravity SKILL.md in temp folder: {}", stage.path().display()),
            );
            std::fs::write(stage.path().join("SKILL.md"), md)?;
            let dest = paths.skills_dir().join(slug);
            stage.commit_to(&dest)?;
            on_log(
                100,
                format!(
                    "Installed skill `/{slug}` to {} (Antigravity SKILL.md format).",
                    dest.display()
                ),
            );
            Ok(PluginInstallOutcome {
                summary: format!("Installed skill `/{slug}`"),
                mcp_entry: None,
                probe_result: None,
            })
        }
        "mcp_python" => {
            if gate.mode() == PrivacyMode::StrictLocal {
                anyhow::bail!(
                    "Strict Local Mode blocked PyPI download for '{}'. Switch to Hybrid Mode to install plugins.",
                    entry.name
                );
            }
            let _ = gate.request(Method::HEAD, "https://pypi.org", "plugin_install_pypi");
            let py = resolve_python_executable()
                .ok_or_else(|| anyhow::anyhow!("Python 3.10+ was not found on PATH."))?;

            let stage = TempStageDir::create(&stage_root, &entry.id)?;
            on_log(
                10,
                format!("Created temp staging folder: {}", stage.path().display()),
            );

            let venv_dir = stage.path().join("venv");
            on_log(20, "Creating isolated Python virtual environment in temp folder...".into());
            let mut mk = tokio::process::Command::new(&py);
            mk.args(["-m", "venv"]).arg(&venv_dir);
            crate::scrub_sensitive_child_env(&mut mk);
            configure_no_window_tokio(&mut mk);
            let mk_out = mk.output().await?;
            if !mk_out.status.success() {
                anyhow::bail!(
                    "Failed to create plugin venv: {}",
                    String::from_utf8_lossy(&mk_out.stderr).trim()
                );
            }

            #[cfg(windows)]
            let (pip_bin, venv_py_rel) = (
                venv_dir.join("Scripts").join("pip.exe"),
                PathBuf::from("venv").join("Scripts").join("python.exe"),
            );
            #[cfg(not(windows))]
            let (pip_bin, venv_py_rel) = (
                venv_dir.join("bin").join("pip"),
                PathBuf::from("venv").join("bin").join("python"),
            );

            on_log(
                45,
                format!("Installing pinned `{}` inside temp venv...", entry.package_spec),
            );
            let mut pip = tokio::process::Command::new(&pip_bin);
            pip.args([
                "install",
                "--only-binary=:all:",
                "--no-input",
                "--disable-pip-version-check",
                "-q",
                &entry.package_spec,
            ]);
            crate::scrub_sensitive_child_env(&mut pip);
            configure_no_window_tokio(&mut pip);
            let pip_out = pip.output().await?;
            if !pip_out.status.success() {
                anyhow::bail!(
                    "pip install failed in temp stage (nothing committed):\n{}",
                    String::from_utf8_lossy(&pip_out.stderr).trim()
                );
            }

            // Write plugin manifest inside stage
            let manifest = json!({
                "id": entry.id,
                "name": entry.name,
                "kind": entry.kind,
                "version": entry.version,
                "package_spec": entry.package_spec,
                "entrypoint": entry.entrypoint,
                "installed_at": Utc::now().to_rfc3339(),
            });
            std::fs::write(
                stage.path().join("cinder-plugin.json"),
                serde_json::to_string_pretty(&manifest)?,
            )?;

            let final_dir = paths.plugins_dir().join(&entry.id);
            on_log(
                75,
                format!("Moving staged plugin atomically to {}...", final_dir.display()),
            );
            stage.commit_to(&final_dir)?;

            let final_py = final_dir.join(&venv_py_rel);
            let mcp_entry = McpServerEntry {
                id: format!("plugin::{}", entry.id),
                name: entry.name.clone(),
                transport: "stdio".into(),
                target: format!("\"{}\" -m {}", final_py.display(), entry.entrypoint),
                enabled: true,
                command: final_py.to_string_lossy().to_string(),
                args: vec!["-m".into(), entry.entrypoint.clone()],
                env: HashMap::new(),
                url: String::new(),
                source: "catalog".into(),
                needs_network: entry.needs_network,
                read_only: entry.read_only,
            };

            on_log(88, "Verifying live MCP JSON-RPC handshake (`initialize` + `tools/list`)...".into());
            let probe = probe_mcp_server(gate, &mcp_entry).await;
            if !probe.ok {
                let _ = std::fs::remove_dir_all(&final_dir);
                anyhow::bail!(
                    "MCP handshake verification failed (rolled back): {}",
                    probe.error.unwrap_or_else(|| "unknown error".into())
                );
            }

            let summary = format!(
                "Installed {} ({} live MCP tools discovered)",
                entry.name,
                probe.tools.len()
            );
            on_log(100, summary.clone());
            Ok(PluginInstallOutcome {
                summary,
                mcp_entry: Some(mcp_entry),
                probe_result: Some(probe),
            })
        }
        "mcp_node" => {
            if gate.mode() == PrivacyMode::StrictLocal {
                anyhow::bail!(
                    "Strict Local Mode blocked npm download for '{}'. Switch to Hybrid Mode to install plugins.",
                    entry.name
                );
            }
            let _ = gate.request(
                Method::HEAD,
                "https://registry.npmjs.org",
                "plugin_install_npm",
            );
            let (node_path, npm_path, _) =
                ensure_node_and_npm(gate, paths, &stage_root, false, &mut on_log).await?;

            let stage = TempStageDir::create(&stage_root, &entry.id)?;
            on_log(
                58,
                format!("Created temp staging folder: {}", stage.path().display()),
            );
            std::fs::write(
                stage.path().join("package.json"),
                "{\n  \"name\": \"cinder-mcp-plugin\",\n  \"private\": true\n}\n",
            )?;

            on_log(
                65,
                format!("Installing pinned `{}` inside temp folder...", entry.package_spec),
            );
            let mut cmd = build_npm_tokio_command(&node_path, &npm_path);
            cmd.args([
                "install",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
                &entry.package_spec,
            ])
            .current_dir(stage.path());
            let out = cmd.output().await?;
            if !out.status.success() {
                anyhow::bail!(
                    "npm install failed in temp stage:\n{}{}",
                    String::from_utf8_lossy(&out.stdout).trim(),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }

            let manifest = json!({
                "id": entry.id,
                "name": entry.name,
                "kind": entry.kind,
                "version": entry.version,
                "package_spec": entry.package_spec,
                "entrypoint": entry.entrypoint,
                "installed_at": Utc::now().to_rfc3339(),
            });
            std::fs::write(
                stage.path().join("cinder-plugin.json"),
                serde_json::to_string_pretty(&manifest)?,
            )?;

            let final_dir = paths.plugins_dir().join(&entry.id);
            on_log(
                80,
                format!("Moving staged plugin atomically to {}...", final_dir.display()),
            );
            stage.commit_to(&final_dir)?;

            let script_path = resolve_node_package_entrypoint(&final_dir, &entry);
            if !script_path.exists() {
                let _ = std::fs::remove_dir_all(&final_dir);
                anyhow::bail!(
                    "Installed package is missing expected entrypoint '{}'",
                    script_path.display()
                );
            }

            let mcp_entry = McpServerEntry {
                id: format!("plugin::{}", entry.id),
                name: entry.name.clone(),
                transport: "stdio".into(),
                target: format!("\"{}\" \"{}\"", node_path.display(), script_path.display()),
                enabled: true,
                command: node_path.to_string_lossy().to_string(),
                args: vec![script_path.to_string_lossy().to_string()],
                env: HashMap::new(),
                url: String::new(),
                source: "catalog".into(),
                needs_network: entry.needs_network,
                read_only: entry.read_only,
            };

            on_log(88, "Verifying live MCP JSON-RPC handshake (`initialize` + `tools/list`)...".into());
            let probe = probe_mcp_server(gate, &mcp_entry).await;
            if !probe.ok {
                let _ = std::fs::remove_dir_all(&final_dir);
                anyhow::bail!(
                    "MCP handshake verification failed (rolled back): {}",
                    probe.error.unwrap_or_else(|| "unknown error".into())
                );
            }

            let summary = format!(
                "Installed {} ({} live MCP tools discovered)",
                entry.name,
                probe.tools.len()
            );
            on_log(100, summary.clone());
            Ok(PluginInstallOutcome {
                summary,
                mcp_entry: Some(mcp_entry),
                probe_result: Some(probe),
            })
        }
        "toolchain" if entry.id == "toolchain-nodejs" => {
            let (node_path, _, _) =
                ensure_node_and_npm(gate, paths, &stage_root, true, &mut on_log).await?;
            let ver = probe_cmd_version(&node_path.to_string_lossy(), &["--version"])
                .map(|(_, v)| v)
                .unwrap_or_else(|| "LTS".into());
            let final_dir = portable_node_dir(paths);
            let summary = format!("Installed Portable Node.js {ver} to {}", final_dir.display());
            on_log(100, summary.clone());
            Ok(PluginInstallOutcome {
                summary,
                mcp_entry: None,
                probe_result: None,
            })
        }
        other => anyhow::bail!("Unsupported plugin kind '{other}'"),
    }
}

fn extract_zip_preserving_tree(zip_path: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    std::fs::create_dir_all(dest_dir)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let out_path = dest_dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }
    }
    Ok(())
}

fn extract_targz_preserving_tree(archive_path: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest_dir)?;
    let canon_dest = crate::sandbox::strip_verbatim_prefix(
        std::fs::canonicalize(dest_dir).unwrap_or_else(|_| dest_dir.to_path_buf()),
    );
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry_res in archive.entries()? {
        let mut entry = entry_res?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_hard_link() {
            anyhow::bail!("Blocked unsafe hard link inside tar archive");
        }
        let rel_path = entry.path()?.into_owned();
        if rel_path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            anyhow::bail!(
                "Blocked path traversal entry inside tar archive: '{}'",
                rel_path.display()
            );
        }
        let out_path = canon_dest.join(&rel_path);
        if entry_type.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if entry_type.is_symlink() {
            let Ok(Some(link_target)) = entry.link_name() else {
                continue;
            };
            if link_target.is_absolute()
                || link_target
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                anyhow::bail!(
                    "Blocked unsafe symlink target '{}' in tar archive",
                    link_target.display()
                );
            }
            let resolved_target = out_path
                .parent()
                .map(|p| p.join(link_target.as_ref()))
                .unwrap_or_else(|| link_target.to_path_buf());
            if !resolved_target.starts_with(&canon_dest) {
                anyhow::bail!(
                    "Blocked escaping symlink '{}' -> '{}' in tar archive",
                    rel_path.display(),
                    link_target.display()
                );
            }
            #[cfg(unix)]
            {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let _ = std::fs::remove_file(&out_path);
                let _ = std::os::unix::fs::symlink(link_target.as_ref(), &out_path);
            }
            continue;
        }
        if !entry_type.is_file() {
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        let mode = entry.header().mode().ok();
        let _ = std::fs::remove_file(&out_path);
        let mut out_file = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out_file)?;
        #[cfg(unix)]
        if let Some(m) = mode {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(m & 0o777));
        }
    }
    Ok(())
}

pub fn uninstall_curated_plugin(paths: &BumboPaths, plugin_id: &str) -> anyhow::Result<Option<String>> {
    let entry = curated_plugin_catalog()
        .into_iter()
        .find(|p| p.id == plugin_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown plugin '{plugin_id}'"))?;

    match entry.kind.as_str() {
        "mcp_python" | "mcp_node" => {
            let dir = paths.plugins_dir().join(&entry.id);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            Ok(Some(format!("plugin::{}", entry.id)))
        }
        "skill_pack" => {
            let dir = paths.skills_dir().join(&entry.entrypoint);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            Ok(None)
        }
        "toolchain" if entry.id == "toolchain-nodejs" => {
            let dir = portable_node_dir(paths);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

// ============================================================================
// Antigravity Customization Sync & Export (mcp_config.json + SKILL.md)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravitySyncReport {
    pub scanned_mcp_files: Vec<String>,
    pub imported_mcp_servers: Vec<McpServerEntry>,
    pub scanned_skill_roots: Vec<String>,
    pub imported_skills: Vec<String>,
}

pub fn sync_from_antigravity(
    paths: &BumboPaths,
    workspace_root: &Path,
    include_builtin_skills: bool,
) -> anyhow::Result<AntigravitySyncReport> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mcp_candidates = vec![
        home.join(".gemini").join("config").join("mcp_config.json"),
        home.join(".gemini").join("antigravity").join("mcp_config.json"),
        workspace_root.join(".agents").join("mcp_config.json"),
    ];

    let mut scanned_mcp_files = Vec::new();
    let mut imported_mcp_servers = Vec::new();

    for cfg_path in mcp_candidates {
        if !cfg_path.exists() {
            continue;
        }
        let from_workspace =
            !workspace_root.as_os_str().is_empty() && cfg_path.starts_with(workspace_root);
        scanned_mcp_files.push(cfg_path.to_string_lossy().to_string());
        if let Ok(raw) = std::fs::read_to_string(&cfg_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                if let Some(servers_obj) = val.get("mcpServers").and_then(|v| v.as_object()) {
                    for (name, spec) in servers_obj {
                        if let Some(url) = spec
                            .get("url")
                            .or_else(|| spec.get("serverUrl"))
                            .and_then(|v| v.as_str())
                        {
                            imported_mcp_servers.push(McpServerEntry {
                                id: format!("agy::{}", crate::mcp::sanitize_mcp_slug(name)),
                                name: name.clone(),
                                transport: "http".into(),
                                target: url.to_string(),
                                enabled: !from_workspace,
                                command: String::new(),
                                args: Vec::new(),
                                env: HashMap::new(),
                                url: url.to_string(),
                                source: "antigravity".into(),
                                needs_network: !url.contains("127.0.0.1") && !url.contains("localhost"),
                                read_only: false,
                            });
                        } else if let Some(cmd) = spec.get("command").and_then(|v| v.as_str()) {
                            let args: Vec<String> = spec
                                .get("args")
                                .and_then(|v| v.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                        .collect()
                                })
                                .unwrap_or_default();
                            let env: HashMap<String, String> = spec
                                .get("env")
                                .and_then(|v| v.as_object())
                                .map(|m| {
                                    m.iter()
                                        .filter_map(|(k, v)| {
                                            v.as_str().map(|s| (k.clone(), s.to_string()))
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            let target = if args.is_empty() {
                                cmd.to_string()
                            } else {
                                format!("{cmd} {}", args.join(" "))
                            };
                            imported_mcp_servers.push(McpServerEntry {
                                id: format!("agy::{}", crate::mcp::sanitize_mcp_slug(name)),
                                name: name.clone(),
                                transport: "stdio".into(),
                                target,
                                enabled: !from_workspace,
                                command: cmd.to_string(),
                                args,
                                env,
                                url: String::new(),
                                source: "antigravity".into(),
                                needs_network: false,
                                read_only: false,
                            });
                        }
                    }
                }
            }
        }
    }

    let mut skill_roots = vec![
        home.join(".gemini").join("skills"),
        home.join(".gemini").join("config").join("skills"),
        home.join(".gemini").join("antigravity").join("skills"),
        home.join(".agents").join("skills"),
        workspace_root.join(".agents").join("skills"),
    ];
    if include_builtin_skills {
        skill_roots.push(
            home.join(".gemini")
                .join("antigravity")
                .join("builtin")
                .join("skills"),
        );
    }

    let mut scanned_skill_roots = Vec::new();
    let mut imported_skills = Vec::new();
    let dest_root = paths.skills_dir();
    std::fs::create_dir_all(&dest_root)?;

    for root in skill_roots {
        if !root.exists() {
            continue;
        }
        scanned_skill_roots.push(root.to_string_lossy().to_string());
        if let Ok(entries) = std::fs::read_dir(&root) {
            for ent in entries.flatten() {
                let p = ent.path();
                if !p.is_dir() {
                    continue;
                }
                let skill_md = p.join("SKILL.md");
                if !skill_md.exists() {
                    continue;
                }
                let slug = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("skill")
                    .to_string();
                let target_dir = dest_root.join(&slug);
                std::fs::create_dir_all(&target_dir)?;
                std::fs::copy(&skill_md, target_dir.join("SKILL.md"))?;
                if !imported_skills.contains(&slug) {
                    imported_skills.push(slug);
                }
            }
        }
    }

    Ok(AntigravitySyncReport {
        scanned_mcp_files,
        imported_mcp_servers,
        scanned_skill_roots,
        imported_skills,
    })
}

pub fn export_antigravity_mcp_json(servers: &[McpServerEntry]) -> Value {
    let mut map = serde_json::Map::new();
    for s in servers {
        if !s.enabled {
            continue;
        }
        let key = crate::mcp::sanitize_mcp_slug(&s.name);
        if s.transport.eq_ignore_ascii_case("http") {
            let url = if !s.url.is_empty() { &s.url } else { &s.target };
            map.insert(key, json!({ "url": url }));
        } else {
            let (cmd, args) = if !s.command.is_empty() {
                (s.command.clone(), s.args.clone())
            } else {
                let parts = crate::mcp::split_command_line(&s.target);
                if parts.is_empty() {
                    continue;
                }
                (parts[0].clone(), parts[1..].to_vec())
            };
            let mut obj = serde_json::Map::new();
            obj.insert("command".into(), json!(cmd));
            if !args.is_empty() {
                obj.insert("args".into(), json!(args));
            }
            if !s.env.is_empty() {
                obj.insert("env".into(), json!(s.env));
            }
            map.insert(key, Value::Object(obj));
        }
    }
    json!({ "mcpServers": Value::Object(map) })
}

// ============================================================================
// Tracked Background Install Jobs (live progress & step logs for the UI)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallJobRecord {
    pub id: String,
    pub kind: String, // "plugin" | "dependency"
    pub target: String,
    pub status: String, // "running" | "completed" | "failed"
    pub progress_pct: u8,
    pub current_step: String,
    pub logs: Vec<String>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Default, Clone)]
pub struct InstallJobManager {
    jobs: Arc<RwLock<Vec<InstallJobRecord>>>,
}

impl InstallJobManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn start_job(&self, kind: &str, target: &str) -> InstallJobRecord {
        let rec = InstallJobRecord {
            id: Uuid::new_v4().to_string(),
            kind: kind.into(),
            target: target.into(),
            status: "running".into(),
            progress_pct: 5,
            current_step: format!("Queued {kind} install for {target}..."),
            logs: vec![format!("Starting staged install for {target}")],
            summary: None,
            error: None,
            started_at: Utc::now(),
            finished_at: None,
        };
        let mut w = self.jobs.write().await;
        w.insert(0, rec.clone());
        if w.len() > 25 {
            w.truncate(25);
        }
        rec
    }

    pub async fn push_progress(&self, job_id: &str, pct: u8, step: String) {
        let mut w = self.jobs.write().await;
        if let Some(j) = w.iter_mut().find(|j| j.id == job_id) {
            j.progress_pct = pct;
            j.current_step = step.clone();
            j.logs.push(step);
        }
    }

    pub async fn finish_ok(&self, job_id: &str, summary: String) {
        let mut w = self.jobs.write().await;
        if let Some(j) = w.iter_mut().find(|j| j.id == job_id) {
            j.status = "completed".into();
            j.progress_pct = 100;
            j.current_step = summary.clone();
            j.summary = Some(summary);
            j.finished_at = Some(Utc::now());
        }
    }

    pub async fn finish_err(&self, job_id: &str, err: String) {
        let mut w = self.jobs.write().await;
        if let Some(j) = w.iter_mut().find(|j| j.id == job_id) {
            j.status = "failed".into();
            j.current_step = format!("Failed: {err}");
            j.logs.push(format!("ERROR: {err}"));
            j.error = Some(err);
            j.finished_at = Some(Utc::now());
        }
    }

    pub async fn list(&self) -> Vec<InstallJobRecord> {
        self.jobs.read().await.clone()
    }

    pub async fn clear_finished(&self) {
        let mut w = self.jobs.write().await;
        w.retain(|j| j.status == "running");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_allowlist_blocks_unapproved_packages() {
        assert!(is_dependency_allowlisted("cargo", "serde").is_some());
        assert!(is_dependency_allowlisted("python", "pytest").is_some());
        assert!(is_dependency_allowlisted("npm", "zod").is_some());
        assert!(is_dependency_allowlisted("cargo", "evil-random-crate").is_none());
        assert!(is_dependency_allowlisted("python", "requests && calc.exe").is_none());
    }

    #[test]
    fn temp_stage_dir_cleans_up_on_drop_and_commits_cleanly() {
        let root = std::env::temp_dir().join(format!("cinder_stage_unit_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        // 1. Aborted stage cleans itself up on drop
        let abandoned_path = {
            let stage = TempStageDir::create(&root, "abort").unwrap();
            let p = stage.path().to_path_buf();
            std::fs::write(p.join("probe.txt"), "hello").unwrap();
            assert!(p.exists());
            p
        };
        assert!(!abandoned_path.exists());

        // 2. Committed stage moves cleanly to destination
        let dest = root.join("final_plugin");
        let stage = TempStageDir::create(&root, "commit").unwrap();
        std::fs::write(stage.path().join("SKILL.md"), "# Skill").unwrap();
        stage.commit_to(&dest).unwrap();
        assert!(dest.join("SKILL.md").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exports_antigravity_mcp_config_schema() {
        let srv = McpServerEntry {
            id: "plugin::mcp-time".into(),
            name: "Time MCP".into(),
            transport: "stdio".into(),
            target: "python -m mcp_server_time".into(),
            enabled: true,
            command: "python".into(),
            args: vec!["-m".into(), "mcp_server_time".into()],
            env: HashMap::new(),
            url: String::new(),
            source: "catalog".into(),
            needs_network: false,
            read_only: true,
        };
        let exported = export_antigravity_mcp_json(&[srv]);
        assert_eq!(exported["mcpServers"]["time_mcp"]["command"], "python");
        assert_eq!(exported["mcpServers"]["time_mcp"]["args"][1], "mcp_server_time");
    }

    #[test]
    fn strict_local_blocks_outbound_dependency_install() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let gate = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let dir = std::env::temp_dir().join(format!("cinder_strict_dep_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let res = rt.block_on(install_project_dependency(
            &gate,
            &paths,
            &dir,
            "cargo",
            "serde",
            |_, _| {},
        ));
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Strict Local Mode"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_curated_skill_pack_via_temp_stage_offline() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let gate = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let dir = std::env::temp_dir().join(format!("cinder_skill_inst_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = BumboPaths {
            app_data_dir: dir.clone(),
            engines_dir: dir.join("engines"),
            state_file: dir.join("state.json"),
            default_vault_dir: dir.join("models"),
        };
        let out = rt
            .block_on(install_curated_plugin(
                &gate,
                &paths,
                "skill-rust-audit",
                |_, _| {},
            ))
            .unwrap();
        assert!(out.summary.contains("/rust-audit"));
        assert!(paths.skills_dir().join("rust-audit").join("SKILL.md").exists());

        uninstall_curated_plugin(&paths, "skill-rust-audit").unwrap();
        assert!(!paths.skills_dir().join("rust-audit").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toml_upsert_and_uninstall_handles_virtual_workspace_and_dependencies() {
        let dir = std::env::temp_dir().join(format!("cinder_toml_dep_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        // 1. Virtual workspace upsert + inspection + uninstall
        let ws_toml = dir.join("Cargo.toml");
        std::fs::write(
            &ws_toml,
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n\n[workspace.dependencies]\nanyhow = \"1.0\"\n",
        )
        .unwrap();

        let raw = std::fs::read_to_string(&ws_toml).unwrap();
        let updated = upsert_toml_dependency_line(
            &raw,
            "[workspace.dependencies]",
            "serde",
            "serde = { version = \"1.0\", features = [\"derive\"] }",
        );
        std::fs::write(&ws_toml, &updated).unwrap();

        let overview = inspect_project_dependencies(&dir);
        assert!(overview.detected_ecosystems.contains(&"cargo".to_string()));
        let cargo_deps = overview.installed_by_manager.get("cargo").unwrap();
        assert!(cargo_deps.contains(&"serde".to_string()));
        assert!(cargo_deps.contains(&"anyhow".to_string()));

        // Updating an existing entry replaces rather than duplicating
        let updated2 = upsert_toml_dependency_line(
            &updated,
            "[workspace.dependencies]",
            "serde",
            "serde = \"1.0.228\"",
        );
        assert_eq!(updated2.matches("serde =").count(), 1);
        std::fs::write(&ws_toml, &updated2).unwrap();

        let msg = uninstall_project_dependency(&dir, "cargo", "serde").unwrap();
        assert!(msg.contains("Removed `serde`"));
        let after = inspect_project_dependencies(&dir);
        assert!(!after.installed_by_manager["cargo"].contains(&"serde".to_string()));

        // 2. Python requirements.txt & npm package.json uninstall
        std::fs::write(dir.join("requirements.txt"), "requests\npytest>=8.0\n").unwrap();
        std::fs::write(
            dir.join("package.json"),
            "{\n  \"name\": \"t\",\n  \"dependencies\": { \"zod\": \"^3.24.0\", \"axios\": \"^1.7.0\" }\n}\n",
        )
        .unwrap();
        uninstall_project_dependency(&dir, "python", "pytest").unwrap();
        uninstall_project_dependency(&dir, "npm", "zod").unwrap();
        let after_all = inspect_project_dependencies(&dir);
        assert_eq!(after_all.installed_by_manager["python"], vec!["requests".to_string()]);
        assert_eq!(after_all.installed_by_manager["npm"], vec!["axios".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_package_validator_allows_safe_and_blocks_injection() {
        assert!(validate_custom_package_name("cargo", "itertools").is_ok());
        assert!(validate_custom_package_name("python", "polars").is_ok());
        assert!(validate_custom_package_name("npm", "@tanstack/react-query").is_ok());
        assert!(validate_custom_package_name("cargo", "--help").is_err());
        assert!(validate_custom_package_name("python", "requests && calc.exe").is_err());
        assert!(validate_custom_package_name("npm", "../evil").is_err());
        assert!(validate_custom_package_name("cargo", "@foo/bar").is_err());
    }

    #[test]
    fn nodejs_dist_spec_for_supports_windows_macos_intel_arm_and_linux() {
        assert_eq!(
            nodejs_dist_spec_for("windows", "x86_64"),
            Some(("win-x64-zip", "win-x64.zip", "zip"))
        );
        assert_eq!(
            nodejs_dist_spec_for("macos", "x86_64"),
            Some(("osx-x64-tar", "darwin-x64.tar.gz", "targz"))
        );
        assert_eq!(
            nodejs_dist_spec_for("macos", "aarch64"),
            Some(("osx-arm64-tar", "darwin-arm64.tar.gz", "targz"))
        );
        assert_eq!(
            nodejs_dist_spec_for("linux", "x86_64"),
            Some(("linux-x64", "linux-x64.tar.gz", "targz"))
        );
        assert_eq!(
            nodejs_dist_spec_for("linux", "aarch64"),
            Some(("linux-arm64", "linux-arm64.tar.gz", "targz"))
        );
    }

    #[test]
    fn parse_shasums256_for_file_matches_exact_filename_and_rejects_malformed() {
        let valid_hash = "e2edda75bfb089654705cf65a13b58933bfd5f57148e8ebda916f58a2474f1f7";
        let manifest = format!(
            "# checksums\n{valid_hash}  node-v26.13.1-win-x64.zip\ndeadbeef  node-v26.13.1-linux-x64.tar.gz\n{valid_hash}  *node-v26.13.1-darwin-arm64.tar.gz\n"
        );
        assert_eq!(
            parse_shasums256_for_file(&manifest, "node-v26.13.1-win-x64.zip"),
            Some(valid_hash.into())
        );
        assert_eq!(
            parse_shasums256_for_file(&manifest, "node-v26.13.1-darwin-arm64.tar.gz"),
            Some(valid_hash.into())
        );
        assert!(parse_shasums256_for_file(&manifest, "node-v26.13.1-linux-x64.tar.gz").is_none());
        assert!(parse_shasums256_for_file(&manifest, "missing.zip").is_none());
    }
}
