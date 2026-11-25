use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl GitFileStatus {
    pub fn badge_letter(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "U",
            Self::Conflicted => "!",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Modified => "Modified",
            Self::Added => "Added",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::Untracked => "Untracked",
            Self::Conflicted => "Conflicted",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitWorkspaceStatus {
    pub is_repo: bool,
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub file_statuses: BTreeMap<String, GitFileStatus>,
    pub modified_dirs: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiffEntry {
    pub rel_path: String,
    pub status: GitFileStatus,
    pub diff: String,
}

#[cfg(windows)]
fn configure_no_window_std(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_no_window_std(_cmd: &mut Command) {}

fn run_git_sync(workspace_root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("--no-optional-locks")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "core.pager=cat",
            "-c",
            "diff.external=",
        ])
        .arg("-C")
        .arg(workspace_root)
        .args(args);
    configure_no_window_std(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn find_git_root(workspace_root: &Path) -> Option<PathBuf> {
    if !workspace_root.exists() {
        return None;
    }
    let mut cur = workspace_root.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn unquote_git_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(ch);
            }
        }
        out.replace('\\', "/")
    } else {
        trimmed.replace('\\', "/")
    }
}

fn parse_branch_header(header: &str) -> (String, u32, u32) {
    let rest = header.strip_prefix("## ").unwrap_or(header).trim();
    if let Some(branch) = rest.strip_prefix("No commits yet on ") {
        return (branch.trim().to_string(), 0, 0);
    }
    if rest.starts_with("HEAD (") {
        return ("HEAD (detached)".to_string(), 0, 0);
    }
    let (main_part, bracket_part) = match rest.rfind(" [") {
        Some(idx) if rest.ends_with(']') => (&rest[..idx], Some(&rest[idx + 2..rest.len() - 1])),
        _ => (rest, None),
    };
    let branch = main_part
        .split("...")
        .next()
        .unwrap_or(main_part)
        .trim()
        .to_string();

    let mut ahead = 0u32;
    let mut behind = 0u32;
    if let Some(meta) = bracket_part {
        for seg in meta.split(',') {
            let s = seg.trim();
            if let Some(n) = s.strip_prefix("ahead ") {
                ahead = n.trim().parse::<u32>().unwrap_or(0);
            } else if let Some(n) = s.strip_prefix("behind ") {
                behind = n.trim().parse::<u32>().unwrap_or(0);
            }
        }
    }
    (
        if branch.is_empty() {
            "HEAD".to_string()
        } else {
            branch
        },
        ahead,
        behind,
    )
}

fn classify_porcelain_xy(x: char, y: char) -> GitFileStatus {
    if x == '?' && y == '?' {
        return GitFileStatus::Untracked;
    }
    if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
        return GitFileStatus::Conflicted;
    }
    if x == 'R' || y == 'R' || x == 'C' {
        return GitFileStatus::Renamed;
    }
    if x == 'D' || y == 'D' {
        return GitFileStatus::Deleted;
    }
    if x == 'A' || y == 'A' {
        return GitFileStatus::Added;
    }
    GitFileStatus::Modified
}

pub fn parse_git_status_porcelain_v1_branch(output: &str) -> GitWorkspaceStatus {
    let mut status = GitWorkspaceStatus {
        is_repo: true,
        branch: "HEAD".into(),
        ..Default::default()
    };

    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if line.starts_with("## ") {
            let (branch, ahead, behind) = parse_branch_header(line);
            status.branch = branch;
            status.ahead = ahead;
            status.behind = behind;
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let mut chars = line.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        if x == '!' && y == '!' {
            continue;
        }
        let file_st = classify_porcelain_xy(x, y);
        let raw_path = &line[3..];
        let rel_raw = if (x == 'R' || x == 'C') && raw_path.contains(" -> ") {
            raw_path.rsplit(" -> ").next().unwrap_or(raw_path)
        } else {
            raw_path
        };
        let rel = unquote_git_path(rel_raw)
            .trim_start_matches("./")
            .trim_end_matches('/')
            .to_string();
        if rel.is_empty() {
            continue;
        }

        let mut cur = rel.as_str();
        while let Some(slash) = cur.rfind('/') {
            let dir = &cur[..slash];
            if !dir.is_empty() {
                status.modified_dirs.insert(dir.to_string());
            }
            cur = dir;
        }
        status.file_statuses.insert(rel, file_st);
    }

    status
}

pub fn inspect_git_status(workspace_root: &Path) -> GitWorkspaceStatus {
    if !workspace_root.is_dir() || find_git_root(workspace_root).is_none() {
        return GitWorkspaceStatus::default();
    }
    let Some(out) = run_git_sync(workspace_root, &["status", "--porcelain=v1", "-b", "-uall", "--", "."]) else {
        return GitWorkspaceStatus::default();
    };
    let mut parsed = parse_git_status_porcelain_v1_branch(&out);

    let show_prefix = run_git_sync(workspace_root, &["rev-parse", "--show-prefix"])
        .map(|s| s.trim().replace('\\', "/"))
        .unwrap_or_default();
    if !show_prefix.is_empty() {
        let mut adjusted = BTreeMap::new();
        let mut adj_dirs = BTreeSet::new();
        for (repo_rel, st) in parsed.file_statuses {
            let local_rel = repo_rel
                .strip_prefix(&show_prefix)
                .unwrap_or(&repo_rel)
                .to_string();
            let mut cur = local_rel.as_str();
            while let Some(slash) = cur.rfind('/') {
                let d = &cur[..slash];
                if !d.is_empty() {
                    adj_dirs.insert(d.to_string());
                }
                cur = d;
            }
            adjusted.insert(local_rel, st);
        }
        parsed.file_statuses = adjusted;
        parsed.modified_dirs = adj_dirs;
    }

    parsed
}

fn synthesize_untracked_diff(abs_path: &Path, rel_path: &str) -> String {
    let Ok(meta) = std::fs::metadata(abs_path) else {
        return format!("New untracked file '{rel_path}'");
    };
    if meta.len() > 48 * 1024 {
        return format!(
            "--- /dev/null\n+++ b/{rel_path}\n(untracked file {} bytes omitted from preview)",
            meta.len()
        );
    }
    let Ok(bytes) = std::fs::read(abs_path) else {
        return format!("(could not read '{rel_path}')");
    };
    if bytes.contains(&0) {
        return format!("--- /dev/null\n+++ b/{rel_path}\n(binary file, {} bytes)", bytes.len());
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return format!("--- /dev/null\n+++ b/{rel_path}\n(non-UTF-8 file)");
    };
    let lines: Vec<&str> = text.lines().take(400).collect();
    let total_lines = text.lines().count().max(1);
    let mut out = format!("--- /dev/null\n+++ b/{rel_path}\n@@ -0,0 +1,{total_lines} @@\n");
    for line in &lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if total_lines > lines.len() {
        out.push_str(&format!("... ({} more lines)\n", total_lines - lines.len()));
    }
    out
}

pub fn inspect_git_diff_head(
    workspace_root: &Path,
    path_filter: Option<&str>,
    max_total_bytes: usize,
) -> Vec<GitDiffEntry> {
    let status = inspect_git_status(workspace_root);
    if !status.is_repo || status.file_statuses.is_empty() {
        return Vec::new();
    }
    let has_head = run_git_sync(workspace_root, &["rev-parse", "--verify", "HEAD"]).is_some();
    let mut entries = Vec::new();
    let mut total_bytes = 0usize;

    for (rel_path, st) in &status.file_statuses {
        if let Some(filter) = path_filter {
            if rel_path != filter {
                continue;
            }
        }
        if total_bytes >= max_total_bytes {
            entries.push(GitDiffEntry {
                rel_path: rel_path.clone(),
                status: *st,
                diff: "(diff preview cap reached for remaining files)".into(),
            });
            continue;
        }

        let diff = if *st == GitFileStatus::Untracked {
            synthesize_untracked_diff(&workspace_root.join(rel_path), rel_path)
        } else if has_head {
            run_git_sync(
                workspace_root,
                &["diff", "--no-ext-diff", "--no-textconv", "HEAD", "--", rel_path],
            )
            .unwrap_or_default()
        } else {
            let staged = run_git_sync(
                workspace_root,
                &["diff", "--no-ext-diff", "--no-textconv", "--cached", "--", rel_path],
            )
            .unwrap_or_default();
            let unstaged = run_git_sync(
                workspace_root,
                &["diff", "--no-ext-diff", "--no-textconv", "--", rel_path],
            )
            .unwrap_or_default();
            format!("{staged}{unstaged}")
        };

        let final_diff = if diff.trim().is_empty() {
            format!("{} ({})", rel_path, st.label())
        } else {
            diff
        };
        total_bytes += final_diff.len();
        entries.push(GitDiffEntry {
            rel_path: rel_path.clone(),
            status: *st,
            diff: final_diff,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_porcelain_branch_ahead_behind_and_nested_statuses() {
        let sample = [
            "## main...origin/main [ahead 2, behind 1]",
            " M crates/cinder-gui/src/lib.rs",
            "A  new_file.rs",
            "R  old.rs -> src/renamed.rs",
            "?? \"docs/my notes.md\"",
            "UU conflict.txt",
            " D removed.txt",
        ]
        .join("\n");
        let st = parse_git_status_porcelain_v1_branch(&sample);
        assert!(st.is_repo);
        assert_eq!(st.branch, "main");
        assert_eq!(st.ahead, 2);
        assert_eq!(st.behind, 1);
        assert_eq!(
            st.file_statuses.get("crates/cinder-gui/src/lib.rs"),
            Some(&GitFileStatus::Modified)
        );
        assert_eq!(st.file_statuses.get("new_file.rs"), Some(&GitFileStatus::Added));
        assert_eq!(
            st.file_statuses.get("src/renamed.rs"),
            Some(&GitFileStatus::Renamed)
        );
        assert_eq!(
            st.file_statuses.get("docs/my notes.md"),
            Some(&GitFileStatus::Untracked)
        );
        assert_eq!(
            st.file_statuses.get("conflict.txt"),
            Some(&GitFileStatus::Conflicted)
        );
        assert_eq!(
            st.file_statuses.get("removed.txt"),
            Some(&GitFileStatus::Deleted)
        );
        assert!(st.modified_dirs.contains("crates"));
        assert!(st.modified_dirs.contains("crates/cinder-gui"));
        assert!(st.modified_dirs.contains("crates/cinder-gui/src"));
        assert!(st.modified_dirs.contains("docs"));

        let no_commits = parse_git_status_porcelain_v1_branch("## No commits yet on feat/init\n?? Cargo.toml\n");
        assert_eq!(no_commits.branch, "feat/init");
        assert_eq!(no_commits.file_statuses.get("Cargo.toml"), Some(&GitFileStatus::Untracked));
    }
}
