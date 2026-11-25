use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use harness_net::NetworkGate;
use harness_store::{probe_storage_drives, DriveInfo};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::Child;
use tokio::sync::{watch, RwLock};
use uuid::Uuid;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vram_total_mb: u64,
    pub vram_used_mb: u64,
    pub vram_free_mb: u64,
    pub driver_version: String,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareReport {
    pub os: String,
    pub arch: String,
    pub cpu_cores: usize,
    pub system_ram_mb: u64,
    pub gpus: Vec<GpuInfo>,
    pub primary_vram_mb: u64,
    pub drives: Vec<DriveInfo>,
    pub probed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitTier {
    FitsInVram,
    PartialOffload,
    CpuHeavy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VramFitAssessment {
    pub tier: FitTier,
    pub label: String,
    pub weights_mb: u64,
    pub kv_cache_mb: u64,
    pub compute_buffer_mb: u64,
    pub estimated_total_mb: u64,
    pub recommended_ngl: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalModelSource {
    VaultGguf,
    OllamaBlob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModelEntry {
    pub id: String,
    pub name: String,
    pub source: LocalModelSource,
    pub gguf_path: String,
    pub ollama_tag: Option<String>,
    #[serde(default)]
    pub mmproj_path: Option<String>,
    pub size_bytes: u64,
    pub quant: String,
    pub is_embedding: bool,
    pub is_valid_gguf: bool,
    pub vram_fit: VramFitAssessment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfModelSummary {
    pub repo_id: String,
    pub author: String,
    pub downloads: u64,
    pub likes: u64,
    pub tags: Vec<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfGgufFile {
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub quant: String,
    pub download_url: String,
    pub vram_fit: VramFitAssessment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfRepoDetails {
    pub repo_id: String,
    pub downloads: u64,
    pub likes: u64,
    pub files: Vec<HfGgufFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Verifying,
    Completed,
    Paused,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadJob {
    pub id: String,
    #[serde(default)]
    pub seq: u64,
    pub repo_id: String,
    pub filename: String,
    pub url: String,
    pub dest_path: String,
    pub part_path: String,
    pub expected_sha256: Option<String>,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub speed_mbps: f64,
    pub status: DownloadStatus,
    pub error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineStatusKind {
    NotInstalled,
    Installed,
    Installing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatus {
    pub llama_server_installed: bool,
    pub llama_server_path: Option<String>,
    pub install_status: EngineStatusKind,
    pub install_message: String,
    pub install_progress_pct: f64,
    pub ollama_loopback_detected: bool,
    pub ollama_models: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveServerBackend {
    None,
    LlamaServer,
    OllamaLoopback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub running: bool,
    pub backend: ActiveServerBackend,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub endpoint_url: Option<String>,
    pub port: u16,
    pub ctx_size: u32,
    pub gpu_layers: i32,
    pub expose_local_api: bool,
    pub api_token_required: bool,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub last_active_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub loaded_models: Vec<String>,
    #[serde(default)]
    pub vram_bytes: u64,
    #[serde(default)]
    pub has_mmproj: bool,
    #[serde(default)]
    pub recent_logs: Vec<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

pub fn probe_hardware() -> HardwareReport {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let system_ram_mb = probe_system_ram_mb();
    let gpus = probe_gpus();
    let primary_vram_mb = gpus.iter().map(|g| g.vram_total_mb).max().unwrap_or(0);
    let drives = probe_storage_drives();

    HardwareReport {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_cores,
        system_ram_mb,
        gpus,
        primary_vram_mb,
        drives,
        probed_at: Utc::now(),
    }
}

#[cfg(windows)]
fn probe_system_ram_mb() -> u64 {
    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(lpBuffer: *mut MemoryStatusEx) -> i32;
    }

    let mut status = MemoryStatusEx {
        dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
        dw_memory_load: 0,
        ull_total_phys: 0,
        ull_avail_phys: 0,
        ull_total_page_file: 0,
        ull_avail_page_file: 0,
        ull_total_virtual: 0,
        ull_avail_virtual: 0,
        ull_avail_extended_virtual: 0,
    };
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok != 0 {
        status.ull_total_phys / (1024 * 1024)
    } else {
        0
    }
}

#[cfg(target_os = "linux")]
fn probe_system_ram_mb() -> u64 {
    if let Ok(raw) = std::fs::read_to_string("/proc/meminfo") {
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                if let Some(kb_str) = rest.split_whitespace().next() {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return kb / 1024;
                    }
                }
            }
        }
    }
    0
}

#[cfg(target_os = "macos")]
fn probe_system_ram_mb() -> u64 {
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Ok(bytes) = s.parse::<u64>() {
                return bytes / (1024 * 1024);
            }
        }
    }
    0
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

fn probe_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    let mut smi_cmd = std::process::Command::new("nvidia-smi");
    smi_cmd.args([
        "--query-gpu=name,memory.total,memory.used,memory.free,driver_version",
        "--format=csv,noheader,nounits",
    ]);
    configure_no_window_std(&mut smi_cmd);
    if let Ok(out) = smi_cmd.output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
                if cols.len() >= 5 {
                    gpus.push(GpuInfo {
                        name: cols[0].to_string(),
                        vram_total_mb: cols[1].parse().unwrap_or(0),
                        vram_used_mb: cols[2].parse().unwrap_or(0),
                        vram_free_mb: cols[3].parse().unwrap_or(0),
                        driver_version: cols[4].to_string(),
                        backend: "CUDA".to_string(),
                    });
                }
            }
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if gpus.is_empty() {
        let ram = probe_system_ram_mb();
        if ram > 0 {
            gpus.push(GpuInfo {
                name: "Apple Silicon Unified Memory".to_string(),
                vram_total_mb: (ram * 75) / 100,
                vram_used_mb: 0,
                vram_free_mb: (ram * 75) / 100,
                driver_version: "Metal".to_string(),
                backend: "Metal".to_string(),
            });
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    if gpus.is_empty() {
        if let Ok(out) = std::process::Command::new("system_profiler")
            .args(["SPDisplaysDataType"])
            .output()
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                gpus.extend(parse_macos_spdisplays_gpus(&text));
            }
        }
    }

    #[cfg(target_os = "linux")]
    if gpus.is_empty() {
        gpus.extend(probe_linux_drm_gpus());
    }

    gpus
}

/// Parses `system_profiler SPDisplaysDataType` output into `GpuInfo` entries (used on Intel Macs).
#[cfg(any(all(target_os = "macos", target_arch = "x86_64"), test))]
fn parse_macos_spdisplays_gpus(text: &str) -> Vec<GpuInfo> {
    let mut gpus: Vec<GpuInfo> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_vram_mb: u64 = 0;

    let flush_current = |gpus: &mut Vec<GpuInfo>, name_opt: &mut Option<String>, vram_mb: &mut u64| {
        if let Some(name) = name_opt.take() {
            let lower = name.to_ascii_lowercase();
            let vendor = if lower.contains("radeon") || lower.contains("amd") {
                "AMD"
            } else if lower.contains("intel") {
                "Intel"
            } else if lower.contains("nvidia") || lower.contains("geforce") || lower.contains("quadro") {
                "NVIDIA"
            } else {
                "Apple"
            };
            gpus.push(GpuInfo {
                name,
                vram_total_mb: *vram_mb,
                vram_used_mb: 0,
                vram_free_mb: *vram_mb,
                driver_version: vendor.to_string(),
                backend: "Metal".to_string(),
            });
        }
        *vram_mb = 0;
    };

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("Chipset Model:") {
            flush_current(&mut gpus, &mut current_name, &mut current_vram_mb);
            let name = rest.trim().to_string();
            if !name.is_empty() {
                current_name = Some(name);
            }
        } else if current_name.is_some()
            && (line.starts_with("VRAM (Total):") || line.starts_with("VRAM (Dynamic, Max):"))
        {
            if let Some((_, val_part)) = line.split_once(':') {
                let mb = parse_vram_size_to_mb(val_part);
                if mb > current_vram_mb {
                    current_vram_mb = mb;
                }
            }
        }
    }
    flush_current(&mut gpus, &mut current_name, &mut current_vram_mb);
    gpus
}

#[cfg(any(all(target_os = "macos", target_arch = "x86_64"), test))]
fn parse_vram_size_to_mb(raw: &str) -> u64 {
    let mut tokens = raw.split_whitespace();
    let Some(num_str) = tokens.next() else {
        return 0;
    };
    let clean_num = num_str.replace(',', "");
    let Ok(val) = clean_num.parse::<f64>() else {
        return 0;
    };
    let unit = tokens.next().unwrap_or("").to_ascii_uppercase();
    if unit.starts_with("GB") {
        (val * 1024.0).round() as u64
    } else if unit.starts_with("MB") {
        val.round() as u64
    } else if unit.starts_with("KB") {
        (val / 1024.0).round() as u64
    } else {
        val.round() as u64
    }
}

#[cfg(target_os = "linux")]
fn probe_linux_drm_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return gpus;
    };
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("card") || fname.contains('-') {
            continue;
        }
        let dev_dir = entry.path().join("device");
        let vram_path = dev_dir.join("mem_info_vram_total");
        let Ok(vram_raw) = std::fs::read_to_string(&vram_path) else {
            continue;
        };
        let Ok(vram_bytes) = vram_raw.trim().parse::<u64>() else {
            continue;
        };
        if vram_bytes == 0 {
            continue;
        }
        let vram_total_mb = vram_bytes / (1024 * 1024);
        let vram_used_mb = std::fs::read_to_string(dev_dir.join("mem_info_vram_used"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|b| b / (1024 * 1024))
            .unwrap_or(0);
        let vendor_hex = std::fs::read_to_string(dev_dir.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let (vendor_name, backend) = match vendor_hex.as_str() {
            "0x1002" => ("AMD Radeon", "Vulkan/ROCm"),
            "0x8086" => ("Intel Graphics", "Vulkan"),
            "0x10de" => ("NVIDIA", "Vulkan/CUDA"),
            _ => ("Linux DRM GPU", "Vulkan"),
        };
        let name = std::fs::read_to_string(dev_dir.join("product_name"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{vendor_name} ({fname})"));
        gpus.push(GpuInfo {
            name,
            vram_total_mb,
            vram_used_mb,
            vram_free_mb: vram_total_mb.saturating_sub(vram_used_mb),
            driver_version: vendor_name.to_string(),
            backend: backend.to_string(),
        });
    }
    gpus
}

pub fn assess_vram_fit(
    file_size_bytes: u64,
    ctx_size: u32,
    vram_total_mb: u64,
) -> VramFitAssessment {
    let weights_mb = file_size_bytes / (1024 * 1024);
    let kv_cache_mb = (((ctx_size as f64 / 4096.0) * (weights_mb as f64 * 0.055).max(192.0))
        .round() as u64)
        .max(128);
    let compute_buffer_mb = 600u64;
    let estimated_total_mb = weights_mb + kv_cache_mb + compute_buffer_mb;

    if vram_total_mb == 0 {
        return VramFitAssessment {
            tier: FitTier::CpuHeavy,
            label: "CPU Inference · No discrete GPU detected".into(),
            weights_mb,
            kv_cache_mb,
            compute_buffer_mb,
            estimated_total_mb,
            recommended_ngl: 0,
        };
    }

    let full_fit_threshold = (vram_total_mb * 88) / 100; // ~10,813 MiB on 12,288 MiB
    let partial_threshold = (vram_total_mb * 150) / 100; // ~18,432 MiB on 12,288 MiB
    let vram_gb = (vram_total_mb as f64 / 1024.0).round() as u64;

    if estimated_total_mb <= full_fit_threshold {
        VramFitAssessment {
            tier: FitTier::FitsInVram,
            label: format!("100% GPU Offload · Fast on {vram_gb}GB VRAM"),
            weights_mb,
            kv_cache_mb,
            compute_buffer_mb,
            estimated_total_mb,
            recommended_ngl: 99,
        }
    } else if estimated_total_mb <= partial_threshold {
        let ratio = ((vram_total_mb.saturating_sub(kv_cache_mb + compute_buffer_mb)) as f64
            / (weights_mb.max(1) as f64))
            .clamp(0.2, 0.9);
        let ngl = (ratio * 40.0).round() as i32;
        VramFitAssessment {
            tier: FitTier::PartialOffload,
            label: format!("Hybrid GPU+RAM Offload · ~{ngl} layers on GPU"),
            weights_mb,
            kv_cache_mb,
            compute_buffer_mb,
            estimated_total_mb,
            recommended_ngl: ngl,
        }
    } else {
        VramFitAssessment {
            tier: FitTier::CpuHeavy,
            label: format!("Exceeds {vram_gb}GB VRAM · Heavy RAM offload"),
            weights_mb,
            kv_cache_mb,
            compute_buffer_mb,
            estimated_total_mb,
            recommended_ngl: 12,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OomRiskLevel {
    Safe,
    VramTight,
    HighOomRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OomRiskAssessment {
    pub level: OomRiskLevel,
    pub summary: String,
    pub detail: String,
    pub estimated_total_mb: u64,
    pub available_pool_mb: u64,
}

pub fn assess_oom_risk(
    fit: &VramFitAssessment,
    ctx_size: u32,
    requested_gpu_layers: i32,
    vram_total_mb: u64,
    system_ram_mb: u64,
) -> OomRiskAssessment {
    if vram_total_mb == 0 {
        let usable_ram_mb = (system_ram_mb * 80) / 100;
        if system_ram_mb > 0 && fit.estimated_total_mb > usable_ram_mb {
            return OomRiskAssessment {
                level: OomRiskLevel::HighOomRisk,
                summary: "RAM OOM Risk".into(),
                detail: format!(
                    "Estimated ~{} MB ({} MB weights + {} MB KV cache @ {} ctx) exceeds safe system RAM headroom (~{} MB). Lower context size or choose a smaller quant.",
                    fit.estimated_total_mb,
                    fit.weights_mb,
                    fit.kv_cache_mb,
                    ctx_size,
                    usable_ram_mb
                ),
                estimated_total_mb: fit.estimated_total_mb,
                available_pool_mb: usable_ram_mb,
            };
        }
        if system_ram_mb > 0 && fit.estimated_total_mb > (system_ram_mb * 65) / 100 {
            return OomRiskAssessment {
                level: OomRiskLevel::VramTight,
                summary: "Tight RAM".into(),
                detail: format!(
                    "Estimated ~{} MB uses most of available system RAM ({} MB total) @ {} ctx.",
                    fit.estimated_total_mb, system_ram_mb, ctx_size
                ),
                estimated_total_mb: fit.estimated_total_mb,
                available_pool_mb: usable_ram_mb,
            };
        }
        return OomRiskAssessment {
            level: OomRiskLevel::Safe,
            summary: "Memory OK".into(),
            detail: format!(
                "Estimated ~{} MB fits comfortably in {} MB system RAM.",
                fit.estimated_total_mb, system_ram_mb
            ),
            estimated_total_mb: fit.estimated_total_mb,
            available_pool_mb: usable_ram_mb,
        };
    }

    let usable_vram_mb = (vram_total_mb * 90) / 100;
    let usable_ram_mb = (system_ram_mb * 80) / 100;
    let combined_pool_mb = if system_ram_mb > 0 {
        usable_vram_mb + usable_ram_mb
    } else {
        usable_vram_mb
    };

    if system_ram_mb > 0 && fit.estimated_total_mb > combined_pool_mb {
        return OomRiskAssessment {
            level: OomRiskLevel::HighOomRisk,
            summary: "High OOM Risk".into(),
            detail: format!(
                "Estimated ~{} MB ({} MB weights + {} MB KV cache @ {} ctx) exceeds combined GPU+RAM headroom (~{} MB). Lower context size or choose a smaller quant.",
                fit.estimated_total_mb,
                fit.weights_mb,
                fit.kv_cache_mb,
                ctx_size,
                combined_pool_mb
            ),
            estimated_total_mb: fit.estimated_total_mb,
            available_pool_mb: combined_pool_mb,
        };
    }

    if requested_gpu_layers >= 99
        && fit.estimated_total_mb > vram_total_mb
        && fit.tier == FitTier::CpuHeavy
    {
        return OomRiskAssessment {
            level: OomRiskLevel::HighOomRisk,
            summary: "VRAM OOM Risk".into(),
            detail: format!(
                "Estimated ~{} MB with NGL {} far exceeds {} MB VRAM (Serve & Use caps NGL to {} to protect VRAM). Reduce context size or NGL if loading fails.",
                fit.estimated_total_mb,
                requested_gpu_layers,
                vram_total_mb,
                fit.recommended_ngl
            ),
            estimated_total_mb: fit.estimated_total_mb,
            available_pool_mb: combined_pool_mb,
        };
    }

    if fit.estimated_total_mb > usable_vram_mb
        || (ctx_size >= 32768 && fit.estimated_total_mb > (vram_total_mb * 82) / 100)
    {
        return OomRiskAssessment {
            level: OomRiskLevel::VramTight,
            summary: "Tight VRAM".into(),
            detail: format!(
                "Estimated ~{} MB vs ~{} MB usable VRAM (@ {} ctx). Partial RAM spill or slower tokens possible.",
                fit.estimated_total_mb, usable_vram_mb, ctx_size
            ),
            estimated_total_mb: fit.estimated_total_mb,
            available_pool_mb: combined_pool_mb,
        };
    }

    OomRiskAssessment {
        level: OomRiskLevel::Safe,
        summary: "Memory OK".into(),
        detail: format!(
            "Estimated ~{} MB fits comfortably within {} MB VRAM / {} MB RAM headroom.",
            fit.estimated_total_mb, vram_total_mb, system_ram_mb
        ),
        estimated_total_mb: fit.estimated_total_mb,
        available_pool_mb: combined_pool_mb,
    }
}

pub fn extract_quant_tag(filename: &str) -> String {
    let upper = filename.to_ascii_uppercase();
    for tag in [
        "IQ2_XXS", "IQ2_XS", "IQ3_XXS", "IQ3_XS", "IQ4_NL", "IQ4_XS", "Q2_K", "Q3_K_S",
        "Q3_K_M", "Q3_K_L", "Q4_0", "Q4_K_S", "Q4_K_M", "Q5_0", "Q5_K_S", "Q5_K_M", "Q6_K",
        "Q8_0", "F16", "BF16",
    ] {
        if upper.contains(tag) {
            return tag.to_string();
        }
    }
    "GGUF".to_string()
}

fn check_gguf_magic(path: &Path) -> bool {
    if let Ok(mut f) = std::fs::File::open(path) {
        let mut buf = [0u8; 4];
        if f.read_exact(&mut buf).is_ok() {
            return &buf == b"GGUF";
        }
    }
    false
}

#[derive(Debug, Deserialize)]
struct OllamaManifest {
    layers: Option<Vec<OllamaLayer>>,
}

#[derive(Debug, Deserialize)]
struct OllamaLayer {
    #[serde(rename = "mediaType")]
    media_type: Option<String>,
    digest: Option<String>,
    size: Option<u64>,
}

pub fn scan_local_models(
    vault_dir: &Path,
    vram_total_mb: u64,
    ctx_size: u32,
) -> Vec<LocalModelEntry> {
    let mut models = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    // 1. Scan Bumbo Model Vault (.gguf files)
    if vault_dir.exists() {
        for entry in WalkDir::new(vault_dir).max_depth(3).into_iter().flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("gguf"))
                    .unwrap_or(false)
            {
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("model")
                    .to_string();
                let lower_name = name.to_ascii_lowercase();
                if lower_name.starts_with("mmproj") {
                    continue;
                }
                let quant = extract_quant_tag(&name);
                let is_embedding =
                    lower_name.contains("embed") || lower_name.contains("bge-");
                let is_valid_gguf = check_gguf_magic(path);
                let path_str = path.to_string_lossy().to_string();
                seen_paths.insert(path_str.clone());
                let mmproj_path = find_companion_mmproj(path)
                    .map(|p| p.to_string_lossy().to_string());

                models.push(LocalModelEntry {
                    id: format!("local::vault::{name}"),
                    name,
                    source: LocalModelSource::VaultGguf,
                    gguf_path: path_str,
                    ollama_tag: None,
                    mmproj_path,
                    size_bytes,
                    quant,
                    is_embedding,
                    is_valid_gguf,
                    vram_fit: assess_vram_fit(size_bytes, ctx_size, vram_total_mb),
                });
            }
        }
    }

    // 2. Auto-discover existing Ollama GGUF blobs (~/.ollama/models) without duplicating files
    if let Some(home) = dirs::home_dir() {
        let ollama_root = home.join(".ollama").join("models");
        let manifests_dir = ollama_root.join("manifests");
        let blobs_dir = ollama_root.join("blobs");

        if manifests_dir.exists() && blobs_dir.exists() {
            for entry in WalkDir::new(&manifests_dir).into_iter().flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let rel = match path.strip_prefix(&manifests_dir) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let comps: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect();
                if comps.len() < 2 {
                    continue;
                }
                let tag = comps.last().cloned().unwrap_or_else(|| "latest".into());
                let repo = comps[comps.len() - 2].clone();
                let model_tag = format!("{repo}:{tag}");

                if let Ok(raw) = std::fs::read_to_string(path) {
                    if let Ok(manifest) = serde_json::from_str::<OllamaManifest>(&raw) {
                        if let Some(ref layers) = manifest.layers {
                            let mmproj_path = layers
                                .iter()
                                .find(|l| {
                                    l.media_type.as_deref()
                                        == Some("application/vnd.ollama.image.projector")
                                })
                                .and_then(|l| l.digest.as_deref())
                                .map(|digest| blobs_dir.join(digest.replace(':', "-")))
                                .filter(|p| p.exists())
                                .map(|p| p.to_string_lossy().to_string());

                            if let Some(model_layer) = layers.iter().find(|l| {
                                l.media_type.as_deref() == Some("application/vnd.ollama.image.model")
                            }) {
                                if let Some(ref digest) = model_layer.digest {
                                    let blob_name = digest.replace(':', "-");
                                    let blob_path = blobs_dir.join(&blob_name);
                                    if blob_path.exists() {
                                        let path_str = blob_path.to_string_lossy().to_string();
                                        if seen_paths.contains(&path_str) {
                                            continue;
                                        }
                                        seen_paths.insert(path_str.clone());
                                        let size_bytes = blob_path
                                            .metadata()
                                            .map(|m| m.len())
                                            .unwrap_or(model_layer.size.unwrap_or(0));
                                        let is_valid_gguf = check_gguf_magic(&blob_path);
                                        let lower = model_tag.to_ascii_lowercase();
                                        let is_embedding =
                                            lower.contains("embed") || lower.contains("bge-");

                                        models.push(LocalModelEntry {
                                            id: format!("local::{model_tag}"),
                                            name: format!("{model_tag} (Ollama GGUF)"),
                                            source: LocalModelSource::OllamaBlob,
                                            gguf_path: path_str,
                                            ollama_tag: Some(model_tag),
                                            mmproj_path,
                                            size_bytes,
                                            quant: "GGUF Blob".into(),
                                            is_embedding,
                                            is_valid_gguf,
                                            vram_fit: assess_vram_fit(
                                                size_bytes,
                                                ctx_size,
                                                vram_total_mb,
                                            ),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    models.sort_by(|a, b| a.is_embedding.cmp(&b.is_embedding).then(a.name.cmp(&b.name)));
    models
}

#[derive(Debug, Deserialize)]
struct HfApiModelRaw {
    id: Option<String>,
    author: Option<String>,
    downloads: Option<u64>,
    likes: Option<u64>,
    tags: Option<Vec<String>>,
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
    siblings: Option<Vec<HfSiblingRaw>>,
}

#[derive(Debug, Deserialize)]
struct HfSiblingRaw {
    rfilename: String,
    size: Option<u64>,
    lfs: Option<HfLfsRaw>,
}

#[derive(Debug, Deserialize)]
struct HfLfsRaw {
    sha256: Option<String>,
    size: Option<u64>,
}

pub async fn search_hf_gguf(
    gate: &NetworkGate,
    query: &str,
    hf_token: &str,
) -> anyhow::Result<Vec<HfModelSummary>> {
    let encoded_q: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("search", query.trim())
        .append_pair("filter", "gguf")
        .append_pair("sort", "downloads")
        .append_pair("direction", "-1")
        .append_pair("limit", "15")
        .finish();
    let api_url = format!("https://huggingface.co/api/models?{encoded_q}");

    let mut req = gate.request(Method::GET, &api_url, "hf_hub_search")?;
    if !hf_token.trim().is_empty() {
        req = req.bearer_auth(hf_token.trim());
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Hugging Face search error {status}: {body}");
    }

    let raw: Vec<HfApiModelRaw> = resp.json().await?;
    Ok(raw
        .into_iter()
        .filter_map(|m| {
            let repo_id = m.id?;
            let author = m
                .author
                .unwrap_or_else(|| repo_id.split('/').next().unwrap_or("").to_string());
            Some(HfModelSummary {
                repo_id,
                author,
                downloads: m.downloads.unwrap_or(0),
                likes: m.likes.unwrap_or(0),
                tags: m.tags.unwrap_or_default(),
                last_modified: m.last_modified,
            })
        })
        .collect())
}

pub async fn inspect_hf_repo(
    gate: &NetworkGate,
    repo_id: &str,
    hf_token: &str,
    vram_total_mb: u64,
    ctx_size: u32,
) -> anyhow::Result<HfRepoDetails> {
    let clean_repo = repo_id.trim().trim_matches('/');
    let api_url = format!("https://huggingface.co/api/models/{clean_repo}?blobs=true");

    let mut req = gate.request(Method::GET, &api_url, "hf_repo_inspect")?;
    if !hf_token.trim().is_empty() {
        req = req.bearer_auth(hf_token.trim());
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Hugging Face repo inspect error {status}: {body}");
    }

    let raw: HfApiModelRaw = resp.json().await?;
    let mut files = Vec::new();
    for s in raw.siblings.unwrap_or_default() {
        if !s.rfilename.to_ascii_lowercase().ends_with(".gguf") {
            continue;
        }
        let size_bytes = s
            .lfs
            .as_ref()
            .and_then(|l| l.size)
            .or(s.size)
            .unwrap_or(0);
        let sha256 = s.lfs.and_then(|l| l.sha256);
        let quant = extract_quant_tag(&s.rfilename);
        let download_url = format!(
            "https://huggingface.co/{clean_repo}/resolve/main/{}",
            s.rfilename
        );
        let vram_fit = assess_vram_fit(size_bytes, ctx_size, vram_total_mb);

        files.push(HfGgufFile {
            filename: s.rfilename,
            size_bytes,
            sha256,
            quant,
            download_url,
            vram_fit,
        });
    }

    files.sort_by(|a, b| a.size_bytes.cmp(&b.size_bytes));
    Ok(HfRepoDetails {
        repo_id: clean_repo.to_string(),
        downloads: raw.downloads.unwrap_or(0),
        likes: raw.likes.unwrap_or(0),
        files,
    })
}

pub struct DownloadManager {
    jobs: Arc<RwLock<HashMap<String, DownloadJob>>>,
    cancels: Arc<RwLock<HashMap<String, (u64, watch::Sender<bool>)>>>,
    generations: Arc<RwLock<HashMap<String, u64>>>,
    next_seq: AtomicU64,
    next_gen: AtomicU64,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            cancels: Arc::new(RwLock::new(HashMap::new())),
            generations: Arc::new(RwLock::new(HashMap::new())),
            next_seq: AtomicU64::new(1),
            next_gen: AtomicU64::new(1),
        }
    }

    pub async fn list_jobs(&self) -> Vec<DownloadJob> {
        let guard = self.jobs.read().await;
        let mut list: Vec<DownloadJob> = guard.values().cloned().collect();
        // Fixed insertion slot ordering so concurrent downloads never swap positions in the UI.
        list.sort_by(|a, b| a.seq.cmp(&b.seq).then_with(|| a.id.cmp(&b.id)));
        list
    }

    pub async fn pause_job(&self, id: &str) -> bool {
        if let Some((_, tx)) = self.cancels.write().await.remove(id) {
            let _ = tx.send(true);
            let mut guard = self.jobs.write().await;
            if let Some(j) = guard.get_mut(id) {
                if matches!(j.status, DownloadStatus::Queued | DownloadStatus::Downloading) {
                    j.status = DownloadStatus::Paused;
                    j.speed_mbps = 0.0;
                    j.updated_at = Utc::now();
                }
            }
            return true;
        }
        false
    }

    pub async fn cancel_job(&self, id: &str) -> bool {
        if let Some((_, tx)) = self.cancels.write().await.remove(id) {
            let _ = tx.send(true);
        }
        self.generations.write().await.remove(id);
        let removed = self.jobs.write().await.remove(id);
        if let Some(job) = removed {
            let _ = std::fs::remove_file(&job.part_path);
            return true;
        }
        false
    }

    pub async fn resume_job(
        &self,
        id: &str,
        gate: Arc<NetworkGate>,
        hf_token: String,
    ) -> anyhow::Result<DownloadJob> {
        if self.cancels.read().await.contains_key(id) {
            if let Some(existing) = self.jobs.read().await.get(id).cloned() {
                return Ok(existing);
            }
        }

        let job = {
            let mut guard = self.jobs.write().await;
            let Some(j) = guard.get_mut(id) else {
                anyhow::bail!("Download job not found.");
            };
            let _ = gate.download_request(Method::GET, &j.url, "hf_download")?;
            let existing_bytes = Path::new(&j.part_path)
                .metadata()
                .map(|m| m.len())
                .unwrap_or(j.bytes_downloaded);
            j.bytes_downloaded = existing_bytes;
            j.speed_mbps = 0.0;
            j.status = DownloadStatus::Queued;
            j.error = None;
            j.updated_at = Utc::now();
            j.clone()
        };

        self.spawn_download_task(job.clone(), gate, hf_token).await;
        Ok(job)
    }

    pub async fn start_hf_download(
        &self,
        gate: Arc<NetworkGate>,
        vault_dir: PathBuf,
        repo_id: String,
        filename: String,
        expected_sha256: Option<String>,
        expected_size: Option<u64>,
        hf_token: String,
    ) -> anyhow::Result<DownloadJob> {
        std::fs::create_dir_all(&vault_dir)?;

        let safe_filename = Path::new(&filename)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("model.gguf")
            .to_string();

        let dest_path = vault_dir.join(&safe_filename);
        let part_path = vault_dir.join(format!("{safe_filename}.part"));
        let clean_repo = repo_id.trim().trim_matches('/').to_string();
        let url = format!("https://huggingface.co/{clean_repo}/resolve/main/{filename}");

        // Validate network gate eagerly so StrictLocal fails immediately before spawning background task.
        let _ = gate.download_request(Method::GET, &url, "hf_download")?;

        // Reuse the existing slot if this repo/file is already tracked.
        let existing_slot = {
            let guard = self.jobs.read().await;
            guard
                .values()
                .find(|j| j.repo_id == clean_repo && j.filename == safe_filename)
                .map(|j| (j.id.clone(), j.seq))
        };
        if let Some((ref old_id, _)) = existing_slot {
            if let Some((_, tx)) = self.cancels.write().await.remove(old_id) {
                let _ = tx.send(true);
            }
        }

        let (job_id, seq) = existing_slot.unwrap_or_else(|| {
            (
                Uuid::new_v4().to_string(),
                self.next_seq.fetch_add(1, Ordering::Relaxed),
            )
        });

        let existing_bytes = part_path.metadata().map(|m| m.len()).unwrap_or(0);
        let job = DownloadJob {
            id: job_id.clone(),
            seq,
            repo_id: clean_repo,
            filename: safe_filename,
            url: url.clone(),
            dest_path: dest_path.to_string_lossy().to_string(),
            part_path: part_path.to_string_lossy().to_string(),
            expected_sha256: expected_sha256.clone(),
            bytes_downloaded: existing_bytes,
            total_bytes: expected_size.unwrap_or(0),
            speed_mbps: 0.0,
            status: DownloadStatus::Queued,
            error: None,
            updated_at: Utc::now(),
        };

        self.jobs.write().await.insert(job_id.clone(), job.clone());
        self.spawn_download_task(job.clone(), gate, hf_token).await;

        Ok(job)
    }

    async fn spawn_download_task(
        &self,
        job: DownloadJob,
        gate: Arc<NetworkGate>,
        hf_token: String,
    ) {
        let my_gen = self.next_gen.fetch_add(1, Ordering::Relaxed);
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        self.generations
            .write()
            .await
            .insert(job.id.clone(), my_gen);
        self.cancels
            .write()
            .await
            .insert(job.id.clone(), (my_gen, cancel_tx));

        let jobs_ref = Arc::clone(&self.jobs);
        let cancels_ref = Arc::clone(&self.cancels);
        let generations_ref = Arc::clone(&self.generations);
        let job_id_bg = job.id.clone();
        let url = job.url;
        let part_path = PathBuf::from(&job.part_path);
        let dest_path = PathBuf::from(&job.dest_path);
        let expected_sha256 = job.expected_sha256;

        tokio::spawn(async move {
            let res = run_resumable_download(
                &gate,
                &jobs_ref,
                &job_id_bg,
                &url,
                &part_path,
                &dest_path,
                expected_sha256.as_deref(),
                &hf_token,
                &mut cancel_rx,
            )
            .await;

            finish_download_task(
                &jobs_ref,
                &cancels_ref,
                &generations_ref,
                &job_id_bg,
                my_gen,
                &part_path,
                res,
            )
            .await;
        });
    }
}

async fn finish_download_task(
    jobs_ref: &Arc<RwLock<HashMap<String, DownloadJob>>>,
    cancels_ref: &Arc<RwLock<HashMap<String, (u64, watch::Sender<bool>)>>>,
    generations_ref: &Arc<RwLock<HashMap<String, u64>>>,
    job_id: &str,
    my_gen: u64,
    part_path: &Path,
    res: anyhow::Result<DownloadOutcome>,
) {
    let current_gen = generations_ref.read().await.get(job_id).copied();
    if current_gen.is_none() {
        // Cancelled/removed while handle was open: delete partial file after file dropped.
        let _ = tokio::fs::remove_file(part_path).await;
        return;
    }
    if current_gen != Some(my_gen) {
        // Superseded by a newer resume/restart task; never overwrite newer state or strip its cancel handle.
        return;
    }

    {
        let mut jobs = jobs_ref.write().await;
        if let Some(j) = jobs.get_mut(job_id) {
            j.updated_at = Utc::now();
            match res {
                Ok(DownloadOutcome::Completed) => {
                    j.status = DownloadStatus::Completed;
                    j.speed_mbps = 0.0;
                }
                Ok(DownloadOutcome::Cancelled) => {
                    j.status = DownloadStatus::Paused;
                    j.speed_mbps = 0.0;
                }
                Err(e) => {
                    j.status = DownloadStatus::Failed;
                    j.speed_mbps = 0.0;
                    j.error = Some(e.to_string());
                }
            }
        }
    }

    let mut cancels = cancels_ref.write().await;
    if cancels.get(job_id).map(|(g, _)| *g) == Some(my_gen) {
        cancels.remove(job_id);
    }
}

enum DownloadOutcome {
    Completed,
    Cancelled,
}

async fn send_hf_get(
    gate: &NetworkGate,
    url: &str,
    hf_token: &str,
    resume_offset: u64,
    cancel_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<Option<reqwest::Response>> {
    if *cancel_rx.borrow() {
        return Ok(None);
    }
    let mut req = gate.download_request(Method::GET, url, "hf_download")?;
    if !hf_token.trim().is_empty() {
        req = req.bearer_auth(hf_token.trim());
    }
    if resume_offset > 0 {
        req = req.header("Range", format!("bytes={resume_offset}-"));
    }
    let send_fut = req.send();
    tokio::pin!(send_fut);
    let resp = loop {
        tokio::select! {
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    return Ok(None);
                }
            }
            r = &mut send_fut => break r?,
        }
    };
    Ok(Some(resp))
}

async fn run_resumable_download(
    gate: &NetworkGate,
    jobs: &Arc<RwLock<HashMap<String, DownloadJob>>>,
    job_id: &str,
    url: &str,
    part_path: &Path,
    dest_path: &Path,
    expected_sha256: Option<&str>,
    hf_token: &str,
    cancel_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<DownloadOutcome> {
    if *cancel_rx.borrow() {
        return Ok(DownloadOutcome::Cancelled);
    }

    let mut resume_offset = part_path.metadata().map(|m| m.len()).unwrap_or(0);
    let Some(mut resp) = send_hf_get(gate, url, hf_token, resume_offset, cancel_rx).await? else {
        return Ok(DownloadOutcome::Cancelled);
    };
    let mut status = resp.status();

    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // Server returned 416: either the .part file already holds the full payload or the offset is stale.
        if let Some(expected) = expected_sha256 {
            if part_path.exists() {
                let part_clone = part_path.to_path_buf();
                if let Ok(Ok(computed)) =
                    tokio::task::spawn_blocking(move || compute_sha256(&part_clone)).await
                {
                    if computed.eq_ignore_ascii_case(expected.trim()) {
                        if *cancel_rx.borrow() {
                            return Ok(DownloadOutcome::Cancelled);
                        }
                        if dest_path.exists() {
                            tokio::fs::remove_file(dest_path).await?;
                        }
                        tokio::fs::rename(part_path, dest_path).await?;
                        return Ok(DownloadOutcome::Completed);
                    }
                }
            }
        }
        drop(resp);
        resume_offset = 0;
        let Some(fresh_resp) = send_hf_get(gate, url, hf_token, 0, cancel_rx).await? else {
            return Ok(DownloadOutcome::Cancelled);
        };
        resp = fresh_resp;
        status = resp.status();
    }

    if !status.is_success() {
        anyhow::bail!("HTTP {status} downloading from Hugging Face");
    }

    let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if !is_partial {
        resume_offset = 0;
    }

    let content_len = resp.content_length().unwrap_or(0);
    let total_bytes = if is_partial {
        resume_offset + content_len
    } else {
        content_len
    };

    {
        let mut guard = jobs.write().await;
        if let Some(j) = guard.get_mut(job_id) {
            j.status = DownloadStatus::Downloading;
            j.bytes_downloaded = resume_offset;
            if total_bytes > 0 {
                j.total_bytes = total_bytes;
            }
            j.updated_at = Utc::now();
        }
    }

    let mut file = if is_partial && resume_offset > 0 {
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(part_path)
            .await?
    } else {
        tokio::fs::File::create(part_path).await?
    };

    let mut stream = resp.bytes_stream();
    let mut downloaded = resume_offset;
    let mut last_tick = std::time::Instant::now();
    let mut bytes_since_tick = 0u64;

    loop {
        tokio::select! {
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    file.flush().await?;
                    drop(file);
                    let mut guard = jobs.write().await;
                    if let Some(j) = guard.get_mut(job_id) {
                        j.bytes_downloaded = downloaded;
                        j.speed_mbps = 0.0;
                    }
                    return Ok(DownloadOutcome::Cancelled);
                }
            }
            next_chunk = stream.next() => {
                match next_chunk {
                    Some(Ok(chunk)) => {
                        file.write_all(&chunk).await?;
                        let len = chunk.len() as u64;
                        downloaded += len;
                        bytes_since_tick += len;

                        let elapsed = last_tick.elapsed().as_secs_f64();
                        if elapsed >= 0.35 {
                            let mbps = (bytes_since_tick as f64 / (1024.0 * 1024.0)) / elapsed;
                            let mut guard = jobs.write().await;
                            if let Some(j) = guard.get_mut(job_id) {
                                j.bytes_downloaded = downloaded;
                                j.speed_mbps = mbps;
                                j.updated_at = Utc::now();
                            }
                            last_tick = std::time::Instant::now();
                            bytes_since_tick = 0;
                        }
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                }
            }
        }
    }

    file.flush().await?;
    drop(file);

    {
        let mut guard = jobs.write().await;
        if let Some(j) = guard.get_mut(job_id) {
            j.bytes_downloaded = downloaded;
            j.status = DownloadStatus::Verifying;
            j.updated_at = Utc::now();
        }
    }

    if let Some(expected) = expected_sha256 {
        let part_clone = part_path.to_path_buf();
        let computed = tokio::task::spawn_blocking(move || compute_sha256(&part_clone)).await??;
        if !computed.eq_ignore_ascii_case(expected.trim()) {
            anyhow::bail!(
                "SHA-256 verification failed: expected {}, got {}",
                expected,
                computed
            );
        }
    }

    if *cancel_rx.borrow() {
        return Ok(DownloadOutcome::Cancelled);
    }

    if dest_path.exists() {
        tokio::fs::remove_file(dest_path).await?;
    }
    tokio::fs::rename(part_path, dest_path).await?;

    Ok(DownloadOutcome::Completed)
}

fn compute_sha256(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedLlamaAsset {
    name: String,
    url: String,
    expected_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResp {
    models: Option<Vec<OllamaTagModel>>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagModel {
    name: String,
}

const MAX_SERVER_LOG_LINES: usize = 40;

fn truncate_chars_with_ellipsis(s: &str, max_chars: usize) -> String {
    let mut count = 0usize;
    for (idx, _) in s.char_indices() {
        if count == max_chars {
            return format!("{}…", s[..idx].trim_end());
        }
        count += 1;
    }
    s.to_string()
}

pub fn push_server_log_line(buf: &mut VecDeque<String>, raw_line: &str) {
    let cleaned = raw_line.trim_end_matches(&['\r', '\n'][..]).trim_end();
    if cleaned.trim().is_empty() {
        return;
    }
    let line = truncate_chars_with_ellipsis(cleaned, 320);
    while buf.len() >= MAX_SERVER_LOG_LINES {
        buf.pop_front();
    }
    buf.push_back(line);
}

pub fn summarize_recent_server_logs(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    const ERROR_KEYWORDS: [&str; 14] = [
        "error",
        "failed",
        "oom",
        "out of memory",
        "alloc",
        "cuda",
        "vulkan",
        "invalid",
        "cannot",
        "fatal",
        "unable",
        "unknown",
        "ggml_assert",
        "not enough",
    ];
    let error_matches: Vec<&String> = lines
        .iter()
        .filter(|line| {
            let low = line.to_ascii_lowercase();
            ERROR_KEYWORDS.iter().any(|kw| low.contains(kw))
        })
        .collect();
    let chosen: Vec<&str> = if !error_matches.is_empty() {
        error_matches
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|s| s.as_str())
            .collect()
    } else {
        lines
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|s| s.as_str())
            .collect()
    };
    let joined = chosen.join(" | ");
    if joined.trim().is_empty() {
        None
    } else {
        Some(truncate_chars_with_ellipsis(&joined, 320))
    }
}

pub struct ServerManager {
    engines_dir: PathBuf,
    install_state: Arc<RwLock<(EngineStatusKind, String, f64)>>,
    running: Arc<RwLock<Option<ManagedProcess>>>,
    recent_logs: Arc<RwLock<VecDeque<String>>>,
    last_error: Arc<RwLock<Option<String>>>,
}

struct ManagedProcess {
    backend: ActiveServerBackend,
    child: Option<Child>,
    model_id: String,
    model_name: String,
    endpoint_url: String,
    port: u16,
    ctx_size: u32,
    gpu_layers: i32,
    expose_local_api: bool,
    ephemeral_token: Option<String>,
    has_mmproj: bool,
    started_at: DateTime<Utc>,
    last_active_at: DateTime<Utc>,
}

impl ServerManager {
    pub fn new(engines_dir: PathBuf) -> Self {
        Self {
            engines_dir,
            install_state: Arc::new(RwLock::new((
                EngineStatusKind::NotInstalled,
                "Ready".into(),
                0.0,
            ))),
            running: Arc::new(RwLock::new(None)),
            recent_logs: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_SERVER_LOG_LINES))),
            last_error: Arc::new(RwLock::new(None)),
        }
    }

    pub fn llama_server_binary(&self) -> PathBuf {
        let dir = self.engines_dir.join("llama-cpp");
        if cfg!(windows) {
            dir.join("llama-server.exe")
        } else {
            dir.join("llama-server")
        }
    }

    pub async fn engine_status(&self, gate: &NetworkGate) -> EngineStatus {
        let bin = self.llama_server_binary();
        let installed = bin.exists();
        let (mut kind, msg, pct) = self.install_state.read().await.clone();
        if installed && kind == EngineStatusKind::NotInstalled {
            kind = EngineStatusKind::Installed;
        }

        // Probe local loopback Ollama only if port 11434 is actually listening (avoids 450ms timeout & audit spam)
        let mut ollama_detected = false;
        let mut ollama_models = Vec::new();
        let port_open = tokio::time::timeout(
            std::time::Duration::from_millis(60),
            tokio::net::TcpStream::connect(("127.0.0.1", 11434)),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
        if port_open {
            if let Ok(req) =
                gate.request(Method::GET, "http://127.0.0.1:11434/api/tags", "ollama_probe")
            {
                if let Ok(resp) =
                    tokio::time::timeout(std::time::Duration::from_millis(450), req.send()).await
                {
                    if let Ok(ok_resp) = resp {
                        if ok_resp.status().is_success() {
                            ollama_detected = true;
                            if let Ok(parsed) = ok_resp.json::<OllamaTagsResp>().await {
                                ollama_models = parsed
                                    .models
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|m| m.name)
                                    .collect();
                            }
                        }
                    }
                }
            }
        }

        EngineStatus {
            llama_server_installed: installed,
            llama_server_path: if installed {
                Some(bin.to_string_lossy().to_string())
            } else {
                None
            },
            install_status: kind,
            install_message: msg,
            install_progress_pct: pct,
            ollama_loopback_detected: ollama_detected,
            ollama_models,
        }
    }

    pub async fn install_llama_server(
        &self,
        gate: Arc<NetworkGate>,
        backend_preference: &str,
    ) -> anyhow::Result<String> {
        {
            let mut st = self.install_state.write().await;
            *st = (
                EngineStatusKind::Installing,
                "Querying official ggml-org/llama.cpp releases...".into(),
                5.0,
            );
        }

        let res = self.do_install_llama_server(&gate, backend_preference).await;
        match &res {
            Ok(msg) => {
                let mut st = self.install_state.write().await;
                *st = (EngineStatusKind::Installed, msg.clone(), 100.0);
            }
            Err(e) => {
                let mut st = self.install_state.write().await;
                *st = (
                    EngineStatusKind::NotInstalled,
                    format!("Install failed: {e}"),
                    0.0,
                );
            }
        }
        res
    }

    async fn do_install_llama_server(
        &self,
        gate: &NetworkGate,
        backend_preference: &str,
    ) -> anyhow::Result<String> {
        let mut checked_tags: Vec<String> = Vec::new();
        let mut chosen: Option<(String, Vec<SelectedLlamaAsset>, Option<String>)> = None;

        // GitHub `/releases/latest` now points to a placeholder `v1.0.0` release whose only
        // asset is `nightly-tag.txt`; scan recent releases for the newest matching binary build.
        let list_url = "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=10";
        let list_resp = gate
            .request(Method::GET, list_url, "engine_install")?
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "cinder-ide")
            .send()
            .await?;

        if list_resp.status().is_success() {
            let releases: Vec<GitHubRelease> = list_resp.json().await?;
            checked_tags.extend(releases.iter().map(|r| r.tag_name.clone()));
            chosen = pick_release_assets(&releases, backend_preference);
        }

        if chosen.is_none() {
            let latest_url = "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";
            let latest_resp = gate
                .request(Method::GET, latest_url, "engine_install")?
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "cinder-ide")
                .send()
                .await?;
            if !latest_resp.status().is_success() {
                anyhow::bail!("GitHub release lookup failed: HTTP {}", latest_resp.status());
            }
            let release: GitHubRelease = latest_resp.json().await?;
            if !checked_tags.contains(&release.tag_name) {
                checked_tags.push(release.tag_name.clone());
            }
            let assets = select_llama_assets(&release.assets, backend_preference);
            if !assets.is_empty() {
                let manifest_url = find_checksum_manifest_url(&release.assets);
                chosen = Some((release.tag_name, assets, manifest_url));
            }
        }

        let Some((release_tag, mut selected_assets, manifest_url)) = chosen else {
            anyhow::bail!(
                "No compatible llama.cpp prebuilt archive found in recent releases ({}) for OS={} ARCH={} (backend={})",
                checked_tags.join(", "),
                std::env::consts::OS,
                std::env::consts::ARCH,
                backend_preference
            );
        };

        if selected_assets.iter().any(|a| a.expected_sha256.is_none()) {
            if let Some(ref m_url) = manifest_url {
                if let Ok(req) = gate.request(Method::GET, m_url, "engine_install_checksums") {
                    if let Ok(m_resp) = req.header("User-Agent", "cinder-ide").send().await {
                        if m_resp.status().is_success() {
                            if let Ok(manifest_txt) = m_resp.text().await {
                                for asset in &mut selected_assets {
                                    if asset.expected_sha256.is_none() {
                                        asset.expected_sha256 =
                                            parse_checksum_manifest_for_file(&manifest_txt, &asset.name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        for asset in &selected_assets {
            if asset.expected_sha256.is_none() {
                anyhow::bail!(
                    "Refusing to install unverified llama.cpp asset '{}' without a SHA-256 digest.",
                    asset.name
                );
            }
        }

        let target_dir = self.engines_dir.join("llama-cpp");
        std::fs::create_dir_all(&target_dir)?;

        let total_assets = selected_assets.len();
        for (idx, asset) in selected_assets.iter().enumerate() {
            let name = &asset.name;
            let url = &asset.url;
            let expected_sha = asset.expected_sha256.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Missing SHA-256 digest for '{}'", asset.name)
            })?;
            {
                let mut st = self.install_state.write().await;
                *st = (
                    EngineStatusKind::Installing,
                    format!("Downloading {name} ({}/{total_assets})...", idx + 1),
                    15.0 + (idx as f64 / total_assets.max(1) as f64) * 70.0,
                );
            }
            let dl_resp = gate
                .download_request(Method::GET, url, "engine_install")?
                .header("User-Agent", "cinder-ide")
                .send()
                .await?;
            if !dl_resp.status().is_success() {
                anyhow::bail!("Failed downloading {name}: HTTP {}", dl_resp.status());
            }

            let total_bytes = dl_resp.content_length();
            let part_path = target_dir.join(format!("{name}.part"));
            let zip_path = target_dir.join(name);
            let mut file = tokio::fs::File::create(&part_path).await?;
            let mut stream = dl_resp.bytes_stream();
            let mut hasher = Sha256::new();
            let mut downloaded: u64 = 0;
            let mut last_reported: u64 = 0;

            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res?;
                hasher.update(&chunk);
                file.write_all(&chunk).await?;
                downloaded += chunk.len() as u64;
                if downloaded.saturating_sub(last_reported) >= 512 * 1024
                    || total_bytes.map(|t| downloaded >= t).unwrap_or(false)
                {
                    last_reported = downloaded;
                    let mb_done = downloaded as f64 / (1024.0 * 1024.0);
                    let frac = total_bytes
                        .filter(|&t| t > 0)
                        .map(|t| (downloaded as f64 / t as f64).clamp(0.0, 1.0))
                        .unwrap_or(0.5);
                    let pct =
                        15.0 + ((idx as f64 + frac) / total_assets.max(1) as f64) * 70.0;
                    let msg = if let Some(tot) = total_bytes.filter(|&t| t > 0) {
                        let mb_tot = tot as f64 / (1024.0 * 1024.0);
                        format!(
                            "Downloading {name} ({}/{total_assets}) — {mb_done:.1} / {mb_tot:.1} MB",
                            idx + 1
                        )
                    } else {
                        format!(
                            "Downloading {name} ({}/{total_assets}) — {mb_done:.1} MB",
                            idx + 1
                        )
                    };
                    let mut st = self.install_state.write().await;
                    *st = (EngineStatusKind::Installing, msg, pct);
                }
            }
            file.flush().await?;
            drop(file);

            let computed_sha = hex::encode(hasher.finalize());
            if !computed_sha.eq_ignore_ascii_case(expected_sha) {
                let _ = tokio::fs::remove_file(&part_path).await;
                anyhow::bail!(
                    "SHA-256 verification failed for {name}: expected {expected_sha}, got {computed_sha}"
                );
            }
            if zip_path.exists() {
                let _ = tokio::fs::remove_file(&zip_path).await;
            }
            tokio::fs::rename(&part_path, &zip_path).await?;

            {
                let mut st = self.install_state.write().await;
                *st = (
                    EngineStatusKind::Installing,
                    format!("Verifying & extracting {name}..."),
                    86.0 + (idx as f64 / total_assets.max(1) as f64) * 10.0,
                );
            }

            let target_clone = target_dir.clone();
            let archive_clone = zip_path.clone();
            tokio::task::spawn_blocking(move || {
                extract_archive_flattened(&archive_clone, &target_clone)
            })
            .await??;
            let _ = std::fs::remove_file(&zip_path);
        }

        let bin = self.llama_server_binary();
        if !bin.exists() {
            anyhow::bail!(
                "Archive extracted, but {} was not found in {}",
                bin.file_name().unwrap_or_default().to_string_lossy(),
                target_dir.display()
            );
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bin)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&bin, perms)?;
        }

        Ok(format!(
            "Installed llama.cpp {} ({})",
            release_tag,
            bin.display()
        ))
    }

    pub async fn server_status(&self) -> ServerStatus {
        let mut guard = self.running.write().await;
        if let Some(proc) = guard.as_mut() {
            if let Some(child) = proc.child.as_mut() {
                if let Ok(Some(exit)) = child.try_wait() {
                    let logs_snapshot: Vec<String> =
                        self.recent_logs.read().await.iter().cloned().collect();
                    let exit_desc = exit.to_string();
                    let err_msg = match summarize_recent_server_logs(&logs_snapshot) {
                        Some(tail) => {
                            format!("llama-server exited unexpectedly ({exit_desc}): {tail}")
                        }
                        None => format!("llama-server exited unexpectedly ({exit_desc})"),
                    };
                    *self.last_error.write().await = Some(err_msg.clone());
                    *guard = None;
                    return ServerStatus {
                        running: false,
                        backend: ActiveServerBackend::None,
                        model_id: None,
                        model_name: None,
                        endpoint_url: None,
                        port: 0,
                        ctx_size: 0,
                        gpu_layers: 0,
                        expose_local_api: false,
                        api_token_required: false,
                        pid: None,
                        started_at: None,
                        last_active_at: None,
                        loaded_models: Vec::new(),
                        vram_bytes: 0,
                        has_mmproj: false,
                        recent_logs: logs_snapshot,
                        last_error: Some(err_msg),
                    };
                }
            }
            let recent_logs: Vec<String> =
                self.recent_logs.read().await.iter().cloned().collect();
            let last_error = self.last_error.read().await.clone();
            return ServerStatus {
                running: true,
                backend: proc.backend,
                model_id: Some(proc.model_id.clone()),
                model_name: Some(proc.model_name.clone()),
                endpoint_url: Some(proc.endpoint_url.clone()),
                port: proc.port,
                ctx_size: proc.ctx_size,
                gpu_layers: proc.gpu_layers,
                expose_local_api: proc.expose_local_api,
                api_token_required: proc.ephemeral_token.is_some(),
                pid: proc.child.as_ref().and_then(|c| c.id()),
                started_at: Some(proc.started_at),
                last_active_at: Some(proc.last_active_at),
                loaded_models: vec![proc.model_name.clone()],
                vram_bytes: 0,
                has_mmproj: proc.has_mmproj,
                recent_logs,
                last_error,
            };
        }

        let recent_logs: Vec<String> = self.recent_logs.read().await.iter().cloned().collect();
        let last_error = self.last_error.read().await.clone();
        ServerStatus {
            running: false,
            backend: ActiveServerBackend::None,
            model_id: None,
            model_name: None,
            endpoint_url: None,
            port: 0,
            ctx_size: 0,
            gpu_layers: 0,
            expose_local_api: false,
            api_token_required: false,
            pid: None,
            started_at: None,
            last_active_at: None,
            loaded_models: Vec::new(),
            vram_bytes: 0,
            has_mmproj: false,
            recent_logs,
            last_error,
        }
    }

    async fn probe_ollama_ps(&self, gate: &NetworkGate) -> Vec<(String, u64)> {
        let port_open = tokio::time::timeout(
            std::time::Duration::from_millis(60),
            tokio::net::TcpStream::connect(("127.0.0.1", 11434)),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
        if !port_open {
            return Vec::new();
        }
        let Ok(req) = gate.request(Method::GET, "http://127.0.0.1:11434/api/ps", "ollama_ps") else {
            return Vec::new();
        };
        let Ok(resp) = req.timeout(std::time::Duration::from_millis(500)).send().await else {
            return Vec::new();
        };
        if !resp.status().is_success() {
            return Vec::new();
        }
        #[derive(Deserialize)]
        struct PsResp {
            models: Option<Vec<PsModel>>,
        }
        #[derive(Deserialize)]
        struct PsModel {
            name: Option<String>,
            model: Option<String>,
            size_vram: Option<u64>,
            size: Option<u64>,
        }
        let Ok(parsed) = resp.json::<PsResp>().await else {
            return Vec::new();
        };
        parsed
            .models
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| {
                let name = m.name.or(m.model)?;
                if name.trim().is_empty() {
                    return None;
                }
                let bytes = m.size_vram.or(m.size).unwrap_or(0);
                Some((name, bytes))
            })
            .collect()
    }

    /// Server status enriched with live Ollama `/api/ps` resident model inspection (loopback only).
    pub async fn server_status_with_probe(&self, gate: &NetworkGate) -> ServerStatus {
        let mut st = self.server_status().await;
        let ps = self.probe_ollama_ps(gate).await;
        if ps.is_empty() {
            return st;
        }
        let vram_bytes: u64 = ps.iter().map(|(_, b)| *b).sum();
        let ps_names: Vec<String> = ps.into_iter().map(|(n, _)| n).collect();
        if st.running {
            for n in ps_names {
                if !st.loaded_models.contains(&n) {
                    st.loaded_models.push(n);
                }
            }
            st.vram_bytes = vram_bytes;
        } else {
            st.running = true;
            st.backend = ActiveServerBackend::OllamaLoopback;
            st.model_name = Some(ps_names.join(", "));
            st.endpoint_url = Some("http://127.0.0.1:11434".to_string());
            st.port = 11434;
            st.loaded_models = ps_names;
            st.vram_bytes = vram_bytes;
        }
        st
    }

    pub async fn active_endpoint_and_token(&self) -> Option<(ActiveServerBackend, String, Option<String>, String)> {
        let mut guard = self.running.write().await;
        if let Some(proc) = guard.as_mut() {
            proc.last_active_at = Utc::now();
            return Some((
                proc.backend,
                proc.endpoint_url.clone(),
                proc.ephemeral_token.clone(),
                proc.model_name.clone(),
            ));
        }
        None
    }

    pub async fn stop_server(&self) -> anyhow::Result<()> {
        let mut guard = self.running.write().await;
        if let Some(mut proc) = guard.take() {
            if let Some(mut child) = proc.child.take() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        *self.last_error.write().await = None;
        Ok(())
    }

    /// Stops any managed `llama-server` / `ollama` child AND evicts all resident Ollama models (`keep_alive: 0`).
    pub async fn stop_all_local_models(&self, gate: &NetworkGate) -> anyhow::Result<Vec<String>> {
        let mut unloaded = Vec::new();
        let managed = {
            let mut guard = self.running.write().await;
            guard.take()
        };

        let ps = self.probe_ollama_ps(gate).await;
        for (name, _) in &ps {
            if let Ok(req) = gate.request(
                Method::POST,
                "http://127.0.0.1:11434/api/generate",
                "ollama_unload",
            ) {
                let _ = req
                    .timeout(std::time::Duration::from_secs(5))
                    .json(&serde_json::json!({ "model": name, "keep_alive": 0 }))
                    .send()
                    .await;
            }
            if !unloaded.contains(name) {
                unloaded.push(name.clone());
            }
        }

        if let Some(mut proc) = managed {
            if !proc.model_name.is_empty() && !unloaded.contains(&proc.model_name) {
                if proc.backend == ActiveServerBackend::OllamaLoopback {
                    if let Ok(req) = gate.request(
                        Method::POST,
                        "http://127.0.0.1:11434/api/generate",
                        "ollama_unload",
                    ) {
                        let _ = req
                            .timeout(std::time::Duration::from_secs(5))
                            .json(&serde_json::json!({ "model": proc.model_name, "keep_alive": 0 }))
                            .send()
                            .await;
                    }
                }
                unloaded.push(proc.model_name.clone());
            }
            if let Some(mut child) = proc.child.take() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }

        *self.last_error.write().await = None;
        Ok(unloaded)
    }

    pub async fn start_local_model(
        &self,
        gate: &NetworkGate,
        model: &LocalModelEntry,
        requested_port: u16,
        ctx_size: u32,
        gpu_layers: i32,
        expose_local_api: bool,
    ) -> anyhow::Result<ServerStatus> {
        self.stop_server().await?;
        self.recent_logs.write().await.clear();
        *self.last_error.write().await = None;

        let bin = self.llama_server_binary();
        if bin.exists() && model.is_valid_gguf {
            let port = find_free_loopback_port(requested_port).await;
            let token = if expose_local_api {
                None
            } else {
                Some(format!("bumbo-{}", Uuid::new_v4()))
            };

            let effective_ngl = if gpu_layers >= 99 && model.vram_fit.tier != FitTier::FitsInVram {
                model.vram_fit.recommended_ngl.max(1)
            } else {
                gpu_layers
            };

            let mmproj_opt = model
                .mmproj_path
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| p.is_file())
                .or_else(|| find_companion_mmproj(Path::new(&model.gguf_path)));

            let launch_res = spawn_and_wait_llama_server(
                gate,
                &bin,
                model,
                port,
                ctx_size,
                effective_ngl,
                token.as_deref(),
                mmproj_opt.as_deref(),
                Arc::clone(&self.recent_logs),
            )
            .await;

            let (child, has_mmproj) = match launch_res {
                Ok(child) => (child, mmproj_opt.is_some()),
                Err(first_err) if mmproj_opt.is_some() => {
                    {
                        let mut guard = self.recent_logs.write().await;
                        push_server_log_line(
                            &mut guard,
                            &format!(
                                "[cinder] Launch with --mmproj failed ({first_err}); retrying without --mmproj..."
                            ),
                        );
                    }
                    match spawn_and_wait_llama_server(
                        gate,
                        &bin,
                        model,
                        port,
                        ctx_size,
                        effective_ngl,
                        token.as_deref(),
                        None,
                        Arc::clone(&self.recent_logs),
                    )
                    .await
                    {
                        Ok(child) => (child, false),
                        Err(err) => {
                            *self.last_error.write().await = Some(err.to_string());
                            return Err(err);
                        }
                    }
                }
                Err(e) => {
                    *self.last_error.write().await = Some(e.to_string());
                    return Err(e);
                }
            };

            let endpoint = format!("http://127.0.0.1:{port}");
            let now = Utc::now();
            *self.running.write().await = Some(ManagedProcess {
                backend: ActiveServerBackend::LlamaServer,
                child: Some(child),
                model_id: model.id.clone(),
                model_name: model.name.clone(),
                endpoint_url: endpoint,
                port,
                ctx_size,
                gpu_layers: effective_ngl,
                expose_local_api,
                ephemeral_token: token,
                has_mmproj,
                started_at: now,
                last_active_at: now,
            });

            return Ok(self.server_status().await);
        }

        // Fallback: if llama-server isn't installed yet and model is an Ollama model, connect to 127.0.0.1:11434 (auto-starting local Ollama if needed)
        if let Some(ref ollama_tag) = model.ollama_tag {
            let tags_url = "http://127.0.0.1:11434/api/tags";
            let already_up = match gate.request(Method::GET, tags_url, "local_ollama_connect") {
                Ok(req) => matches!(
                    tokio::time::timeout(std::time::Duration::from_millis(450), req.send()).await,
                    Ok(Ok(r)) if r.status().is_success()
                ),
                Err(_) => false,
            };

            let mut spawned_ollama: Option<Child> = None;
            let mut ready = already_up;

            if !already_up {
                if let Some(ollama_bin) = find_local_ollama_binary() {
                    let mut cmd = tokio::process::Command::new(&ollama_bin);
                    cmd.arg("serve")
                        .env("OLLAMA_HOST", "127.0.0.1:11434")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                    configure_no_window_tokio(&mut cmd);
                    if let Ok(child) = cmd.spawn() {
                        spawned_ollama = Some(child);
                        for _ in 0..20 {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            if let Ok(req) =
                                gate.request(Method::GET, tags_url, "local_ollama_connect")
                            {
                                if let Ok(Ok(r)) = tokio::time::timeout(
                                    std::time::Duration::from_millis(450),
                                    req.send(),
                                )
                                .await
                                {
                                    if r.status().is_success() {
                                        ready = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if ready {
                let tag_l = ollama_tag.to_ascii_lowercase();
                let ollama_has_vision = tag_l.contains("vision")
                    || tag_l.contains("-vl")
                    || tag_l.contains("gemma3")
                    || tag_l.contains("gemma-3")
                    || tag_l.contains("gemma4")
                    || tag_l.contains("llava")
                    || tag_l.contains("minicpm-v")
                    || tag_l.contains("pixtral");
                let now = Utc::now();
                *self.running.write().await = Some(ManagedProcess {
                    backend: ActiveServerBackend::OllamaLoopback,
                    child: spawned_ollama,
                    model_id: model.id.clone(),
                    model_name: ollama_tag.clone(),
                    endpoint_url: "http://127.0.0.1:11434".into(),
                    port: 11434,
                    ctx_size,
                    gpu_layers,
                    expose_local_api: true,
                    ephemeral_token: None,
                    has_mmproj: ollama_has_vision,
                    started_at: now,
                    last_active_at: now,
                });
                return Ok(self.server_status().await);
            } else if let Some(mut child) = spawned_ollama {
                let _ = child.kill().await;
            }
        }

        let msg = "llama-server is not installed yet and local Ollama did not start. Install llama-server in Settings → Local Engine & HF Hub.".to_string();
        *self.last_error.write().await = Some(msg.clone());
        anyhow::bail!("{msg}");
    }

    pub async fn check_idle_unload(&self, gate: &NetworkGate, idle_unload_minutes: u64) -> bool {
        if idle_unload_minutes == 0 {
            return false;
        }
        let should_stop = {
            let guard = self.running.read().await;
            if let Some(proc) = guard.as_ref() {
                if matches!(
                    proc.backend,
                    ActiveServerBackend::LlamaServer | ActiveServerBackend::OllamaLoopback
                ) {
                    let elapsed = (Utc::now() - proc.last_active_at).num_minutes();
                    elapsed >= idle_unload_minutes as i64
                } else {
                    false
                }
            } else {
                false
            }
        };
        if should_stop {
            let _ = self.stop_all_local_models(gate).await;
            return true;
        }
        false
    }
}

fn find_companion_mmproj(model_path: &Path) -> Option<PathBuf> {
    let dir = model_path.parent()?;
    let stem = model_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let entries = std::fs::read_dir(dir).ok()?;
    let mut fallback: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.starts_with("mmproj") && name.ends_with(".gguf") {
            let family_token = stem.split('-').next().unwrap_or("");
            if !family_token.is_empty() && name.contains(family_token) {
                return Some(p);
            }
            if fallback.is_none() {
                fallback = Some(p);
            }
        }
    }
    fallback
}

#[allow(clippy::too_many_arguments)]
async fn spawn_and_wait_llama_server(
    gate: &NetworkGate,
    bin: &Path,
    model: &LocalModelEntry,
    port: u16,
    ctx_size: u32,
    effective_ngl: i32,
    token: Option<&str>,
    mmproj_opt: Option<&Path>,
    recent_logs: Arc<RwLock<VecDeque<String>>>,
) -> anyhow::Result<Child> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args([
        "--model",
        &model.gguf_path,
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "-c",
        &ctx_size.to_string(),
        "-ngl",
        &effective_ngl.to_string(),
        "-fa",
        "on",
        "--cache-type-k",
        "q8_0",
        "--cache-type-v",
        "q8_0",
        "--no-webui",
        "--jinja",
    ]);
    if let Some(mmproj_path) = mmproj_opt {
        cmd.args(["--mmproj", &mmproj_path.to_string_lossy()]);
    }
    if let Some(t) = token {
        cmd.args(["--api-key", t]);
    }
    if let Some(parent) = bin.parent() {
        cmd.current_dir(parent);
        #[cfg(target_os = "macos")]
        {
            let existing = std::env::var("DYLD_LIBRARY_PATH").unwrap_or_default();
            let combined = if existing.is_empty() {
                parent.to_string_lossy().to_string()
            } else {
                format!("{}:{existing}", parent.display())
            };
            cmd.env("DYLD_LIBRARY_PATH", combined);
        }
        #[cfg(target_os = "linux")]
        {
            let existing = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
            let combined = if existing.is_empty() {
                parent.to_string_lossy().to_string()
            } else {
                format!("{}:{existing}", parent.display())
            };
            cmd.env("LD_LIBRARY_PATH", combined);
        }
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    configure_no_window_tokio(&mut cmd);

    let mut child = cmd.spawn()?;
    if let Some(stderr) = child.stderr.take() {
        let buf = Arc::clone(&recent_logs);
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut guard = buf.write().await;
                push_server_log_line(&mut guard, &line);
            }
        });
    }
    if let Some(stdout) = child.stdout.take() {
        let buf = Arc::clone(&recent_logs);
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut guard = buf.write().await;
                push_server_log_line(&mut guard, &line);
            }
        });
    }

    let health_url = format!("http://127.0.0.1:{port}/health");

    for _ in 0..120 {
        if let Ok(Some(exit)) = child.try_wait() {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            let logs_snapshot: Vec<String> = recent_logs.read().await.iter().cloned().collect();
            if let Some(tail) = summarize_recent_server_logs(&logs_snapshot) {
                anyhow::bail!("llama-server exited prematurely with status {exit} — {tail}");
            }
            anyhow::bail!("llama-server exited prematurely with status {exit}");
        }
        if let Ok(req) = gate.request(Method::GET, &health_url, "local_server_health") {
            if let Ok(resp) = req.send().await {
                if resp.status().is_success() {
                    return Ok(child);
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let _ = child.kill().await;
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    let logs_snapshot: Vec<String> = recent_logs.read().await.iter().cloned().collect();
    if let Some(tail) = summarize_recent_server_logs(&logs_snapshot) {
        anyhow::bail!(
            "Timed out waiting for llama-server to load {} — {tail}",
            model.name
        );
    }
    anyhow::bail!("Timed out waiting for llama-server to load {}", model.name)
}

fn find_local_ollama_binary() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(local_app) = dirs::data_local_dir() {
            let p = local_app.join("Programs").join("Ollama").join("ollama.exe");
            if p.exists() {
                return Some(p);
            }
        }
        return Some(PathBuf::from("ollama.exe"));
    }
    #[cfg(not(windows))]
    {
        for candidate in [
            "/Applications/Ollama.app/Contents/Resources/ollama",
            "/opt/homebrew/bin/ollama",
            "/usr/local/bin/ollama",
            "/usr/bin/ollama",
        ] {
            if Path::new(candidate).exists() {
                return Some(PathBuf::from(candidate));
            }
        }
        Some(PathBuf::from("ollama"))
    }
}

async fn find_free_loopback_port(preferred: u16) -> u16 {
    if tokio::net::TcpListener::bind(("127.0.0.1", preferred))
        .await
        .is_ok()
    {
        return preferred;
    }
    if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    preferred
}

fn parse_github_sha256_digest(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    let hex_part = s
        .strip_prefix("sha256:")
        .or_else(|| s.strip_prefix("SHA256:"))
        .unwrap_or(s)
        .trim();
    if hex_part.len() == 64 && hex_part.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hex_part.to_ascii_lowercase())
    } else {
        None
    }
}

fn parse_checksum_manifest_for_file(text: &str, filename: &str) -> Option<String> {
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
        let base_match = Path::new(clean_name)
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == filename);
        if (clean_name == filename || base_match)
            && hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

fn find_checksum_manifest_url(assets: &[GitHubAsset]) -> Option<String> {
    assets
        .iter()
        .find(|a| {
            let low = a.name.to_ascii_lowercase();
            (low.contains("sha256sum") || (low.contains("sha256") && low.ends_with(".txt")))
                && !low.ends_with(".sig")
        })
        .map(|a| a.browser_download_url.clone())
}

fn asset_to_selected(a: &GitHubAsset) -> SelectedLlamaAsset {
    SelectedLlamaAsset {
        name: a.name.clone(),
        url: a.browser_download_url.clone(),
        expected_sha256: parse_github_sha256_digest(a.digest.as_deref()),
    }
}

fn pick_release_assets(
    releases: &[GitHubRelease],
    backend_preference: &str,
) -> Option<(String, Vec<SelectedLlamaAsset>, Option<String>)> {
    for rel in releases {
        let selected = select_llama_assets(&rel.assets, backend_preference);
        if !selected.is_empty() {
            return Some((
                rel.tag_name.clone(),
                selected,
                find_checksum_manifest_url(&rel.assets),
            ));
        }
    }
    None
}

fn select_llama_assets(
    assets: &[GitHubAsset],
    backend_preference: &str,
) -> Vec<SelectedLlamaAsset> {
    select_llama_assets_for(
        assets,
        std::env::consts::OS,
        std::env::consts::ARCH,
        backend_preference,
    )
}

fn find_asset_matching_needles<'a>(
    assets: &'a [GitHubAsset],
    needles: &[&str],
) -> Option<&'a GitHubAsset> {
    for needle in needles {
        if let Some(found) = assets
            .iter()
            .find(|a| a.name.to_ascii_lowercase().contains(needle))
        {
            return Some(found);
        }
    }
    None
}

fn select_llama_assets_for(
    assets: &[GitHubAsset],
    os: &str,
    arch: &str,
    backend_preference: &str,
) -> Vec<SelectedLlamaAsset> {
    let mut picked = Vec::new();

    if os == "windows" {
        let want_vulkan = backend_preference.eq_ignore_ascii_case("vulkan");
        if !want_vulkan {
            let cuda_bin = assets
                .iter()
                .find(|a| {
                    let n = a.name.to_ascii_lowercase();
                    n.contains("bin-win-cuda-12") && n.ends_with("-x64.zip") && !n.starts_with("cudart")
                })
                .or_else(|| {
                    assets.iter().find(|a| {
                        let n = a.name.to_ascii_lowercase();
                        n.contains("bin-win-cuda") && n.ends_with("-x64.zip") && !n.starts_with("cudart")
                    })
                });
            let cuda_rt = assets
                .iter()
                .find(|a| {
                    let n = a.name.to_ascii_lowercase();
                    n.starts_with("cudart-llama-bin-win-cuda-12") && n.ends_with("-x64.zip")
                })
                .or_else(|| {
                    assets.iter().find(|a| {
                        let n = a.name.to_ascii_lowercase();
                        n.starts_with("cudart-llama-bin-win-cuda") && n.ends_with("-x64.zip")
                    })
                });

            if let Some(bin) = cuda_bin {
                picked.push(asset_to_selected(bin));
                if let Some(rt) = cuda_rt {
                    picked.push(asset_to_selected(rt));
                }
                return picked;
            }
        }

        if let Some(vk) = find_asset_matching_needles(assets, &["bin-win-vulkan-x64.zip"]) {
            picked.push(asset_to_selected(vk));
            return picked;
        }

        if let Some(cpu) = find_asset_matching_needles(assets, &["bin-win-cpu-x64.zip"]) {
            picked.push(asset_to_selected(cpu));
            return picked;
        }
    } else if os == "macos" {
        let needles: &[&str] = if arch == "aarch64" {
            &[
                "bin-macos-arm64.tar.gz",
                "bin-macos-arm64.tgz",
                "bin-macos-arm64.zip",
            ]
        } else {
            &[
                "bin-macos-x64.tar.gz",
                "bin-macos-x64.tgz",
                "bin-macos-x64.zip",
            ]
        };
        if let Some(a) = find_asset_matching_needles(assets, needles) {
            picked.push(asset_to_selected(a));
            return picked;
        }
    } else if os == "linux" {
        let want_vulkan = backend_preference.eq_ignore_ascii_case("vulkan");
        let found = if arch == "aarch64" {
            let vk = if want_vulkan {
                find_asset_matching_needles(
                    assets,
                    &[
                        "bin-ubuntu-vulkan-arm64.tar.gz",
                        "bin-ubuntu-vulkan-arm64.tgz",
                        "bin-ubuntu-vulkan-arm64.zip",
                    ],
                )
            } else {
                None
            };
            vk.or_else(|| {
                find_asset_matching_needles(
                    assets,
                    &[
                        "bin-ubuntu-arm64.tar.gz",
                        "bin-ubuntu-arm64.tgz",
                        "bin-ubuntu-arm64.zip",
                    ],
                )
            })
        } else {
            let vk = if want_vulkan {
                find_asset_matching_needles(
                    assets,
                    &[
                        "bin-ubuntu-vulkan-x64.tar.gz",
                        "bin-ubuntu-vulkan-x64.tgz",
                        "bin-ubuntu-vulkan-x64.zip",
                    ],
                )
            } else {
                None
            };
            vk.or_else(|| {
                find_asset_matching_needles(
                    assets,
                    &[
                        "bin-ubuntu-x64.tar.gz",
                        "bin-ubuntu-x64.tgz",
                        "bin-ubuntu-x64.zip",
                    ],
                )
            })
        };
        if let Some(a) = found {
            picked.push(asset_to_selected(a));
            return picked;
        }
    }

    picked
}

#[cfg(unix)]
fn apply_unix_runtime_perms(path: &Path, archive_mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    let fname = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_exec_name = fname == "llama-server"
        || fname.ends_with(".dylib")
        || fname.contains(".dylib.")
        || fname.ends_with(".so")
        || fname.contains(".so.");
    let archive_exec = archive_mode.map(|m| (m & 0o111) != 0).unwrap_or(false);
    let target_mode = if is_exec_name || archive_exec {
        0o755
    } else {
        0o644
    };
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(target_mode);
        let _ = std::fs::set_permissions(path, perms);
    }
}

fn extract_archive_flattened(archive_path: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    let lower = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        extract_targz_flattened(archive_path, dest_dir)
    } else {
        extract_zip_flattened(archive_path, dest_dir)
    }
}

fn extract_targz_flattened(archive_path: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry_res in archive.entries()? {
        let mut entry = entry_res?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        let rel_path = entry.path()?.into_owned();
        if rel_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let Some(entry_name) = rel_path.file_name().map(|n| n.to_os_string()) else {
            continue;
        };
        let out_path = dest_dir.join(&entry_name);

        if entry_type.is_symlink() {
            #[cfg(unix)]
            if let Ok(Some(link_target)) = entry.link_name() {
                if let Some(target_name) = link_target.file_name() {
                    let _ = std::fs::remove_file(&out_path);
                    let _ = std::os::unix::fs::symlink(target_name, &out_path);
                }
            }
            continue;
        }

        if !entry_type.is_file() {
            continue;
        }

        #[cfg(unix)]
        let mode = entry.header().mode().ok();
        let _ = std::fs::remove_file(&out_path);
        let mut out_file = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out_file)?;
        #[cfg(unix)]
        apply_unix_runtime_perms(&out_path, mode);
    }
    Ok(())
}

fn extract_zip_flattened(zip_path: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(entry_name) = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        else {
            continue;
        };
        #[cfg(unix)]
        let mode = entry.unix_mode();
        let out_path = dest_dir.join(entry_name);
        let _ = std::fs::remove_file(&out_path);
        let mut out_file = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out_file)?;
        #[cfg(unix)]
        apply_unix_runtime_perms(&out_path, mode);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vram_fit_estimates_3080ti_accurately() {
        // 5.5 GB model at 16k ctx should fit comfortably in 12GB (12288 MB)
        let small = assess_vram_fit(5_500 * 1024 * 1024, 16384, 12288);
        assert_eq!(small.tier, FitTier::FitsInVram);
        assert_eq!(small.recommended_ngl, 99);

        // 13 GB model at 8k ctx should be partial offload
        let medium = assess_vram_fit(13_000 * 1024 * 1024, 8192, 12288);
        assert_eq!(medium.tier, FitTier::PartialOffload);

        // 38 GB model should be CPU heavy
        let large = assess_vram_fit(38_000 * 1024 * 1024, 8192, 12288);
        assert_eq!(large.tier, FitTier::CpuHeavy);
    }

    #[test]
    fn parse_macos_spdisplays_detects_intel_mac_discrete_and_integrated_gpus() {
        let sample = r#"
Graphics/Displays:

    Intel UHD Graphics 630:

      Chipset Model: Intel UHD Graphics 630
      Type: GPU
      Bus: Built-In
      VRAM (Dynamic, Max): 1536 MB
      Vendor: Intel
      Metal Support: Metal 3

    AMD Radeon Pro 5500M:

      Chipset Model: AMD Radeon Pro 5500M
      Type: GPU
      Bus: PCIe
      PCIe Lane Width: x8
      VRAM (Total): 4 GB
      Vendor: AMD (0x1002)
      Metal Support: Metal 3
"#;
        let gpus = parse_macos_spdisplays_gpus(sample);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "Intel UHD Graphics 630");
        assert_eq!(gpus[0].vram_total_mb, 1536);
        assert_eq!(gpus[0].driver_version, "Intel");
        assert_eq!(gpus[0].backend, "Metal");

        assert_eq!(gpus[1].name, "AMD Radeon Pro 5500M");
        assert_eq!(gpus[1].vram_total_mb, 4096);
        assert_eq!(gpus[1].driver_version, "AMD");
        assert_eq!(gpus[1].backend, "Metal");
    }

    #[test]
    fn select_llama_assets_for_supports_windows_macos_intel_arm_and_linux() {
        let d = Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into());
        let assets = vec![
            GitHubAsset {
                name: "llama-b12800-bin-win-cuda-12.4-x64.zip".into(),
                browser_download_url: "https://example.com/win-cuda.zip".into(),
                digest: d.clone(),
            },
            GitHubAsset {
                name: "cudart-llama-bin-win-cuda-12.4-x64.zip".into(),
                browser_download_url: "https://example.com/win-cudart.zip".into(),
                digest: d.clone(),
            },
            GitHubAsset {
                name: "llama-b12800-bin-win-vulkan-x64.zip".into(),
                browser_download_url: "https://example.com/win-vk.zip".into(),
                digest: d.clone(),
            },
            GitHubAsset {
                name: "llama-b12800-bin-macos-arm64.tar.gz".into(),
                browser_download_url: "https://example.com/mac-arm64.tar.gz".into(),
                digest: d.clone(),
            },
            GitHubAsset {
                name: "llama-b12800-bin-macos-x64.tar.gz".into(),
                browser_download_url: "https://example.com/mac-x64.tar.gz".into(),
                digest: d.clone(),
            },
            GitHubAsset {
                name: "llama-b12800-bin-ubuntu-x64.tar.gz".into(),
                browser_download_url: "https://example.com/ubuntu-x64.tar.gz".into(),
                digest: d.clone(),
            },
            GitHubAsset {
                name: "llama-b12800-bin-ubuntu-vulkan-x64.zip".into(),
                browser_download_url: "https://example.com/ubuntu-vk.zip".into(),
                digest: d.clone(),
            },
        ];

        let win_cuda = select_llama_assets_for(&assets, "windows", "x86_64", "cuda");
        assert_eq!(win_cuda.len(), 2);
        assert_eq!(win_cuda[0].name, "llama-b12800-bin-win-cuda-12.4-x64.zip");
        assert!(win_cuda[0].expected_sha256.is_some());

        let win_vk = select_llama_assets_for(&assets, "windows", "x86_64", "vulkan");
        assert_eq!(win_vk.len(), 1);
        assert_eq!(win_vk[0].name, "llama-b12800-bin-win-vulkan-x64.zip");

        let mac_intel = select_llama_assets_for(&assets, "macos", "x86_64", "cuda");
        assert_eq!(mac_intel.len(), 1);
        assert_eq!(mac_intel[0].name, "llama-b12800-bin-macos-x64.tar.gz");

        let mac_arm = select_llama_assets_for(&assets, "macos", "aarch64", "cuda");
        assert_eq!(mac_arm.len(), 1);
        assert_eq!(mac_arm[0].name, "llama-b12800-bin-macos-arm64.tar.gz");

        let linux_cpu = select_llama_assets_for(&assets, "linux", "x86_64", "cuda");
        assert_eq!(linux_cpu.len(), 1);
        assert_eq!(linux_cpu[0].name, "llama-b12800-bin-ubuntu-x64.tar.gz");

        let linux_vk = select_llama_assets_for(&assets, "linux", "x86_64", "vulkan");
        assert_eq!(linux_vk.len(), 1);
        assert_eq!(linux_vk[0].name, "llama-b12800-bin-ubuntu-vulkan-x64.zip");

        // Legacy .zip format still matches on macOS Intel when .tar.gz is absent
        let legacy_mac_zip = vec![GitHubAsset {
            name: "llama-b12000-bin-macos-x64.zip".into(),
            browser_download_url: "https://example.com/mac-x64.zip".into(),
            digest: None,
        }];
        let mac_zip_picked = select_llama_assets_for(&legacy_mac_zip, "macos", "x86_64", "cuda");
        assert_eq!(mac_zip_picked[0].name, "llama-b12000-bin-macos-x64.zip");
        assert!(mac_zip_picked[0].expected_sha256.is_none());
    }

    #[test]
    fn pick_release_assets_skips_dummy_v1_tag_and_picks_compatible_build() {
        let dummy = GitHubRelease {
            tag_name: "v1.0.0".into(),
            assets: vec![GitHubAsset {
                name: "nightly-tag.txt".into(),
                browser_download_url: "https://example.com/nightly-tag.txt".into(),
                digest: None,
            }],
        };
        let b12621 = GitHubRelease {
            tag_name: "b12621".into(),
            assets: vec![
                GitHubAsset {
                    name: "llama-b12621-bin-win-cuda-12.4-x64.zip".into(),
                    browser_download_url: "https://example.com/cuda12.zip".into(),
                    digest: Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
                },
                GitHubAsset {
                    name: "cudart-llama-bin-win-cuda-12.4-x64.zip".into(),
                    browser_download_url: "https://example.com/cudart12.zip".into(),
                    digest: Some("sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                },
                GitHubAsset {
                    name: "llama-b12621-bin-macos-arm64.zip".into(),
                    browser_download_url: "https://example.com/macos-arm64.zip".into(),
                    digest: Some("sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into()),
                },
                GitHubAsset {
                    name: "llama-b12621-bin-macos-x64.tar.gz".into(),
                    browser_download_url: "https://example.com/macos-x64.tar.gz".into(),
                    digest: Some("sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into()),
                },
                GitHubAsset {
                    name: "llama-b12621-bin-ubuntu-x64.zip".into(),
                    browser_download_url: "https://example.com/ubuntu.zip".into(),
                    digest: Some("sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into()),
                },
            ],
        };
        let b12620 = GitHubRelease {
            tag_name: "b12620".into(),
            assets: vec![
                GitHubAsset {
                    name: "llama-b12620-bin-win-vulkan-x64.zip".into(),
                    browser_download_url: "https://example.com/vulkan.zip".into(),
                    digest: None,
                },
                GitHubAsset {
                    name: "llama-b12620-bin-ubuntu-vulkan-x64.zip".into(),
                    browser_download_url: "https://example.com/ubuntu-vulkan.zip".into(),
                    digest: None,
                },
            ],
        };

        let releases = vec![dummy, b12621, b12620];
        let picked = pick_release_assets(&releases, "cuda").unwrap();
        assert_eq!(picked.0, "b12621");
        assert!(!picked.1.is_empty());

        #[cfg(target_os = "windows")]
        {
            assert_eq!(picked.1.len(), 2);
            let vk = pick_release_assets(&releases, "vulkan").unwrap();
            assert_eq!(vk.0, "b12620");
            assert_eq!(vk.1[0].name, "llama-b12620-bin-win-vulkan-x64.zip");
        }
    }

    #[test]
    fn llama_asset_digest_parser_and_checksum_manifest_verify_strictly() {
        assert_eq!(
            parse_github_sha256_digest(Some(
                "sha256:ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"
            )),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into())
        );
        assert!(parse_github_sha256_digest(Some("sha256:tooshort")).is_none());
        assert!(parse_github_sha256_digest(Some("md5:0123456789abcdef0123456789abcdef")).is_none());

        let manifest = "\
# release checksums
1111111111111111111111111111111111111111111111111111111111111111  *llama-b12800-bin-win-vulkan-x64.zip
nothex  llama-b12800-bin-ubuntu-x64.tar.gz
";
        assert_eq!(
            parse_checksum_manifest_for_file(manifest, "llama-b12800-bin-win-vulkan-x64.zip"),
            Some("1111111111111111111111111111111111111111111111111111111111111111".into())
        );
        assert!(parse_checksum_manifest_for_file(manifest, "llama-b12800-bin-ubuntu-x64.tar.gz").is_none());
    }

    #[tokio::test]
    async fn download_jobs_preserve_stable_slot_order_across_progress_updates() {
        let mgr = DownloadManager::new();
        let make_job = |id: &str, seq: u64, updated_secs_offset: i64| DownloadJob {
            id: id.into(),
            seq,
            repo_id: "org/repo".into(),
            filename: format!("model-{seq}.gguf"),
            url: format!("https://huggingface.co/org/repo/resolve/main/model-{seq}.gguf"),
            dest_path: format!("/tmp/model-{seq}.gguf"),
            part_path: format!("/tmp/model-{seq}.gguf.part"),
            expected_sha256: None,
            bytes_downloaded: 100 * seq,
            total_bytes: 1000,
            speed_mbps: 12.0,
            status: DownloadStatus::Downloading,
            error: None,
            updated_at: Utc::now() + chrono::Duration::seconds(updated_secs_offset),
        };
        {
            let mut guard = mgr.jobs.write().await;
            guard.insert("a".into(), make_job("a", 1, 5));
            guard.insert("b".into(), make_job("b", 2, 1));
            guard.insert("c".into(), make_job("c", 3, 20));
        }
        // Progress updates mutating updated_at must NOT swap row positions.
        {
            let mut guard = mgr.jobs.write().await;
            if let Some(j) = guard.get_mut("a") {
                j.updated_at = Utc::now() + chrono::Duration::seconds(99);
            }
            if let Some(j) = guard.get_mut("b") {
                j.updated_at = Utc::now() - chrono::Duration::seconds(50);
            }
        }
        let ordered = mgr.list_jobs().await;
        let ids: Vec<&str> = ordered.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn download_manager_pause_unblocks_resume_and_isolates_generations() {
        let mgr = DownloadManager::new();
        let dir = std::env::temp_dir().join(format!("cinder_dl_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let part_path = dir.join("model.gguf.part");
        std::fs::write(&part_path, b"partial").unwrap();

        let job = DownloadJob {
            id: "job_1".into(),
            seq: 1,
            repo_id: "org/repo".into(),
            filename: "model.gguf".into(),
            url: "https://huggingface.co/org/repo/resolve/main/model.gguf".into(),
            dest_path: dir.join("model.gguf").to_string_lossy().to_string(),
            part_path: part_path.to_string_lossy().to_string(),
            expected_sha256: None,
            bytes_downloaded: 7,
            total_bytes: 100,
            speed_mbps: 1.0,
            status: DownloadStatus::Downloading,
            error: None,
            updated_at: Utc::now(),
        };

        let (tx1, _rx1) = watch::channel(false);
        let (tx2, _rx2) = watch::channel(false);
        mgr.jobs.write().await.insert("job_1".into(), job);
        mgr.generations.write().await.insert("job_1".into(), 1);
        mgr.cancels.write().await.insert("job_1".into(), (1, tx1));

        // Pause must immediately remove the cancel handle so rapid resume isn't blocked.
        assert!(mgr.pause_job("job_1").await);
        assert!(!mgr.cancels.read().await.contains_key("job_1"));
        assert_eq!(
            mgr.jobs.read().await.get("job_1").unwrap().status,
            DownloadStatus::Paused
        );

        // Simulate a rapid resume under generation 2 before generation 1 finishes unwinding.
        mgr.generations.write().await.insert("job_1".into(), 2);
        mgr.cancels.write().await.insert("job_1".into(), (2, tx2));
        if let Some(j) = mgr.jobs.write().await.get_mut("job_1") {
            j.status = DownloadStatus::Downloading;
        }

        // Old generation 1 finishing must NOT overwrite gen 2 status, remove tx2, or delete part_path.
        finish_download_task(
            &mgr.jobs,
            &mgr.cancels,
            &mgr.generations,
            "job_1",
            1,
            &part_path,
            Ok(DownloadOutcome::Cancelled),
        )
        .await;

        assert!(mgr.cancels.read().await.contains_key("job_1"));
        assert_eq!(mgr.cancels.read().await.get("job_1").unwrap().0, 2);
        assert_eq!(
            mgr.jobs.read().await.get("job_1").unwrap().status,
            DownloadStatus::Downloading
        );
        assert!(part_path.exists());

        // Active generation 2 completing cleanly updates status and removes tx2.
        finish_download_task(
            &mgr.jobs,
            &mgr.cancels,
            &mgr.generations,
            "job_1",
            2,
            &part_path,
            Ok(DownloadOutcome::Completed),
        )
        .await;

        assert!(!mgr.cancels.read().await.contains_key("job_1"));
        assert_eq!(
            mgr.jobs.read().await.get("job_1").unwrap().status,
            DownloadStatus::Completed
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn start_local_model_preserves_hybrid_privacy_mode() {
        use harness_net::PrivacyMode;

        let dir = std::env::temp_dir().join(format!(
            "cinder_models_privacy_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = ServerManager::new(dir.clone());
        let gate = NetworkGate::new(PrivacyMode::Hybrid).unwrap();
        let model = LocalModelEntry {
            id: "local::nonexistent-privacy-check".into(),
            name: "Nonexistent Check".into(),
            source: LocalModelSource::VaultGguf,
            gguf_path: dir.join("missing.gguf").to_string_lossy().to_string(),
            ollama_tag: None,
            mmproj_path: None,
            size_bytes: 0,
            quant: "Q4_K_M".into(),
            is_embedding: false,
            is_valid_gguf: false,
            vram_fit: assess_vram_fit(0, 4096, 0),
        };

        let _ = mgr
            .start_local_model(&gate, &model, 18099, 4096, 0, false)
            .await;
        assert_eq!(gate.mode(), PrivacyMode::Hybrid);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn server_log_ring_buffer_truncates_utf8_and_summarizes_errors() {
        let mut buf = VecDeque::new();
        push_server_log_line(&mut buf, "   \r\n");
        assert!(buf.is_empty());

        let long_utf8 = format!("{}é", "a".repeat(330));
        push_server_log_line(&mut buf, &long_utf8);
        assert_eq!(buf.len(), 1);
        assert!(buf[0].ends_with('…'));
        assert!(buf[0].is_char_boundary(buf[0].len()));

        for i in 0..MAX_SERVER_LOG_LINES + 5 {
            push_server_log_line(&mut buf, &format!("line {i}\r"));
        }
        assert_eq!(buf.len(), MAX_SERVER_LOG_LINES);
        assert!(!buf.iter().any(|l| l.ends_with('\r')));
        assert_eq!(buf.back().map(String::as_str), Some("line 44"));

        let logs = vec![
            "ggml_cuda_init: found 1 CUDA devices".to_string(),
            "llama_kv_cache_init: size = 2048 MiB".to_string(),
            "ggml_backend_cuda_buffer_type_alloc_buffer: failed to allocate 14200.00 MiB".to_string(),
            "error: out of memory loading weights".to_string(),
        ];
        let summary = summarize_recent_server_logs(&logs).unwrap();
        assert!(summary.contains("failed to allocate"));
        assert!(summary.contains("out of memory"));

        let benign = vec!["srv: listening".to_string(), "ready".to_string()];
        assert_eq!(
            summarize_recent_server_logs(&benign).as_deref(),
            Some("srv: listening | ready")
        );
        assert_eq!(summarize_recent_server_logs(&[]), None);
    }

    #[test]
    fn assess_oom_risk_identifies_safe_tight_and_high_risk_configs() {
        let small = assess_vram_fit(4 * 1024 * 1024 * 1024, 8192, 12_288);
        let safe_res = assess_oom_risk(&small, 8192, 99, 12_288, 32_768);
        assert_eq!(safe_res.level, OomRiskLevel::Safe);

        let tight_model = assess_vram_fit(10 * 1024 * 1024 * 1024, 65_536, 12_288);
        let tight_res = assess_oom_risk(&tight_model, 65_536, 36, 12_288, 65_536);
        assert_eq!(tight_res.level, OomRiskLevel::VramTight);

        let huge_model = assess_vram_fit(40 * 1024 * 1024 * 1024, 32_768, 12_288);
        let oom_res = assess_oom_risk(&huge_model, 32_768, 99, 12_288, 16_384);
        assert_eq!(oom_res.level, OomRiskLevel::HighOomRisk);

        let cpu_huge = assess_vram_fit(20 * 1024 * 1024 * 1024, 8192, 0);
        let cpu_oom = assess_oom_risk(&cpu_huge, 8192, 0, 0, 16_384);
        assert_eq!(cpu_oom.level, OomRiskLevel::HighOomRisk);
    }
}
