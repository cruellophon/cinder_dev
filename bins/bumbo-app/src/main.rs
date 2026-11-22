#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
fn augment_macos_gui_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = current
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let mut candidates = vec![
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy();
        candidates.push(format!("{home_str}/.cargo/bin"));
        candidates.push(format!("{home_str}/.local/bin"));
    }

    let mut changed = false;
    for dir in candidates {
        if !parts.iter().any(|p| p == &dir) {
            parts.push(dir);
            changed = true;
        }
    }
    if changed {
        std::env::set_var("PATH", parts.join(":"));
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    augment_macos_gui_path();
    bumbo_app::run_cinder_entry()
}
