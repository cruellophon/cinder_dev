use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    pub is_recommended_for_models: bool,
}

#[cfg(windows)]
pub fn probe_storage_drives() -> Vec<DriveInfo> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalDrives() -> u32;
        fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }
    const DRIVE_FIXED: u32 = 3;

    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for i in 0..26u8 {
        if (mask & (1 << i)) == 0 {
            continue;
        }
        let letter = (b'A' + i) as char;
        let root_str = format!("{letter}:\\");
        let wide: Vec<u16> = root_str.encode_utf16().chain(std::iter::once(0)).collect();
        let dtype = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if dtype != DRIVE_FIXED {
            continue;
        }
        let mut avail: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
        if ok != 0 && total > 0 {
            drives.push(DriveInfo {
                name: format!("{letter}:"),
                mount_point: root_str,
                total_bytes: total,
                free_bytes: avail,
                used_bytes: total.saturating_sub(avail),
                is_recommended_for_models: false,
            });
        }
    }

    if let Some((best_idx, _)) = drives.iter().enumerate().max_by_key(|(_, d)| d.free_bytes) {
        if let Some(d) = drives.get_mut(best_idx) {
            d.is_recommended_for_models = true;
        }
    }
    drives
}

#[cfg(not(windows))]
pub fn probe_storage_drives() -> Vec<DriveInfo> {
    let mut drives = Vec::new();
    if let Ok(out) = std::process::Command::new("df").args(["-Pk"]).output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 6 {
                    let mount = parts[5];
                    if mount == "/"
                        || mount.starts_with("/Volumes/")
                        || mount.starts_with("/mnt/")
                        || mount.starts_with("/home")
                    {
                        let total_kb: u64 = parts[1].parse().unwrap_or(0);
                        let avail_kb: u64 = parts[3].parse().unwrap_or(0);
                        if total_kb > 0 {
                            drives.push(DriveInfo {
                                name: mount.to_string(),
                                mount_point: mount.to_string(),
                                total_bytes: total_kb * 1024,
                                free_bytes: avail_kb * 1024,
                                used_bytes: (total_kb.saturating_sub(avail_kb)) * 1024,
                                is_recommended_for_models: false,
                            });
                        }
                    }
                }
            }
        }
    }
    if let Some((best_idx, _)) = drives.iter().enumerate().max_by_key(|(_, d)| d.free_bytes) {
        if let Some(d) = drives.get_mut(best_idx) {
            d.is_recommended_for_models = true;
        }
    }
    drives
}

#[derive(Debug, Clone)]
pub struct BumboPaths {
    pub app_data_dir: PathBuf,
    pub engines_dir: PathBuf,
    pub state_file: PathBuf,
    pub default_vault_dir: PathBuf,
}

impl BumboPaths {
    pub fn discover() -> Self {
        // `CINDER_DATA_DIR` lets a portable install (or a test run) keep its state
        // somewhere other than the per-user default.
        let base = std::env::var_os("CINDER_DATA_DIR")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| {
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("Project_Bumbo")
            });
        let engines_dir = base.join("engines");
        let state_file = base.join("state.json");

        let drives = probe_storage_drives();
        let default_vault_dir = choose_smart_vault_dir(&base, &drives);

        Self {
            app_data_dir: base,
            engines_dir,
            state_file,
            default_vault_dir,
        }
    }

    /// Installed MCP plugins (one folder per add-on).
    pub fn plugins_dir(&self) -> PathBuf {
        self.app_data_dir.join("plugins")
    }

    /// Cinder-managed portable toolchains (e.g. Node.js).
    pub fn toolchains_dir(&self) -> PathBuf {
        self.app_data_dir.join("toolchains")
    }

    /// Global skills (`<name>/SKILL.md`, Antigravity-compatible layout).
    pub fn skills_dir(&self) -> PathBuf {
        self.app_data_dir.join("skills")
    }
}

fn choose_smart_vault_dir(app_data_dir: &Path, drives: &[DriveInfo]) -> PathBuf {
    const GB: u64 = 1024 * 1024 * 1024;
    let c_drive_low = drives
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case("C:"))
        .map(|d| d.free_bytes < 25 * GB)
        .unwrap_or(false);

    if c_drive_low {
        if let Some(best_other) = drives
            .iter()
            .filter(|d| !d.name.eq_ignore_ascii_case("C:") && d.free_bytes > 50 * GB)
            .max_by_key(|d| d.free_bytes)
        {
            return PathBuf::from(&best_other.mount_point).join("Project_Bumbo_Models");
        }
    }

    app_data_dir.join("models")
}
