use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFileNode {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub extension: String,
}

/// Strip Windows `\\?\` or `//?/` verbatim path prefixes so paths compare consistently.
pub fn strip_verbatim_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    if let Some(rest) = s.strip_prefix("//?/UNC/") {
        return PathBuf::from(format!("//{rest}"));
    }
    if let Some(rest) = s.strip_prefix("//?/") {
        return PathBuf::from(rest);
    }
    p
}

#[cfg(windows)]
fn strip_prefix_case_insensitive_win(root: &Path, entry: &Path) -> Option<String> {
    let r_str = root
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    let e_str = entry.to_string_lossy().replace('\\', "/");
    if e_str.eq_ignore_ascii_case(&r_str) {
        return Some(String::new());
    }
    let r_prefix = format!("{r_str}/");
    if e_str.len() > r_prefix.len()
        && e_str.is_char_boundary(r_prefix.len())
        && e_str[..r_prefix.len()].eq_ignore_ascii_case(&r_prefix)
    {
        return Some(e_str[r_prefix.len()..].to_string());
    }
    None
}

/// Compute `entry_path` relative to `workspace_root` using forward slashes,
/// tolerating Windows `\\?\` verbatim mismatches, symlinks (e.g. macOS `/var` -> `/private/var`),
/// short 8.3 paths, uncreated descendant paths, and Windows drive/case differences.
pub fn relative_to_workspace(workspace_root: &Path, entry_path: &Path) -> Option<String> {
    if let Ok(rel) = entry_path.strip_prefix(workspace_root) {
        return Some(rel.to_string_lossy().replace('\\', "/"));
    }

    let clean_root = strip_verbatim_prefix(workspace_root.to_path_buf());
    let clean_entry = strip_verbatim_prefix(entry_path.to_path_buf());
    if let Ok(rel) = clean_entry.strip_prefix(&clean_root) {
        return Some(rel.to_string_lossy().replace('\\', "/"));
    }

    let cr_clean_opt = std::fs::canonicalize(workspace_root)
        .ok()
        .map(strip_verbatim_prefix);
    if let Some(ref cr_clean) = cr_clean_opt {
        if let Ok(rel) = clean_entry.strip_prefix(cr_clean) {
            return Some(rel.to_string_lossy().replace('\\', "/"));
        }
        if let Ok(ce) = std::fs::canonicalize(entry_path) {
            let ce_clean = strip_verbatim_prefix(ce);
            if let Ok(rel) = ce_clean.strip_prefix(cr_clean) {
                return Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        // Uncreated descendant under an uncanonicalized root symlink or 8.3 short path:
        // canonicalize the nearest existing ancestor and re-attach remaining components.
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        let mut cur: Option<&Path> = Some(clean_entry.as_path());
        while let Some(probe) = cur {
            if let Ok(meta) = std::fs::symlink_metadata(probe) {
                if meta.file_type().is_symlink() && !probe.exists() {
                    return None;
                }
                if probe.exists() {
                    if let Ok(canon_anc) = std::fs::canonicalize(probe) {
                        let mut reconstructed = strip_verbatim_prefix(canon_anc);
                        for part in tail.iter().rev() {
                            reconstructed.push(part);
                        }
                        if let Ok(rel) = reconstructed.strip_prefix(cr_clean) {
                            return Some(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                    break;
                }
            }
            if let Some(name) = probe.file_name() {
                tail.push(name.to_os_string());
                cur = probe.parent();
            } else {
                break;
            }
        }
    }

    #[cfg(windows)]
    {
        if let Some(rel) = strip_prefix_case_insensitive_win(&clean_root, &clean_entry) {
            return Some(rel);
        }
        if let Some(ref cr_clean) = cr_clean_opt {
            if let Some(rel) = strip_prefix_case_insensitive_win(cr_clean, &clean_entry) {
                return Some(rel);
            }
        }
    }

    None
}

fn umlaut_for_vowel(v: char) -> Option<char> {
    match v {
        'a' => Some('ä'),
        'o' => Some('ö'),
        'u' => Some('ü'),
        'A' => Some('Ä'),
        'O' => Some('Ö'),
        'U' => Some('Ü'),
        _ => None,
    }
}

fn is_standalone_german_word_before_upper(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "der"
            | "die"
            | "das"
            | "den"
            | "dem"
            | "des"
            | "ein"
            | "eine"
            | "einer"
            | "einen"
            | "einem"
            | "eines"
            | "kein"
            | "keine"
            | "und"
            | "oder"
            | "mit"
            | "von"
            | "zu"
            | "zur"
            | "zum"
            | "bei"
            | "aus"
            | "auf"
            | "in"
            | "im"
            | "am"
            | "an"
            | "als"
            | "wie"
            | "nach"
            | "vor"
            | "unter"
            | "ist"
            | "sind"
            | "war"
            | "wird"
            | "auch"
            | "nur"
            | "noch"
            | "sehr"
            | "viel"
            | "mehr"
    )
}

/// Normalize PDF text artifacts common in German LaTeX/Type1 PDFs (`w ¤ ahlen` -> `wählen`,
/// `¨ a` -> `ä`, combining diaeresis, and `fi`/`fl` ligatures).
pub fn clean_latex_pdf_text(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\u{FB00}' => normalized.push_str("ff"),
            '\u{FB01}' => normalized.push_str("fi"),
            '\u{FB02}' => normalized.push_str("fl"),
            '\u{FB03}' => normalized.push_str("ffi"),
            '\u{FB04}' => normalized.push_str("ffl"),
            '\u{00AD}' => {}
            '\u{00A0}' => normalized.push(' '),
            _ => normalized.push(ch),
        }
    }

    let chars: Vec<char> = normalized.chars().collect();
    let mut out = String::with_capacity(normalized.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];

        // Combining diaeresis after vowel (`a\u{0308}` -> `ä`)
        if i + 1 < chars.len() && chars[i + 1] == '\u{0308}' {
            if let Some(u) = umlaut_for_vowel(ch) {
                out.push(u);
                i += 2;
                continue;
            }
        }

        // Accent marker (`¤`, `¨`, `\u{0308}`) before vowel (with optional single space)
        if matches!(ch, '¤' | '¨' | '\u{0308}') {
            let (vowel_opt, next_idx, had_space_after) =
                if i + 1 < chars.len() && umlaut_for_vowel(chars[i + 1]).is_some() {
                    (umlaut_for_vowel(chars[i + 1]), i + 2, false)
                } else if i + 2 < chars.len()
                    && chars[i + 1] == ' '
                    && umlaut_for_vowel(chars[i + 2]).is_some()
                {
                    (umlaut_for_vowel(chars[i + 2]), i + 3, true)
                } else {
                    (None, i + 1, false)
                };

            if let Some(umlaut) = vowel_opt {
                let raw_vowel = chars[next_idx - 1];
                // If `out` ends with `<letter> ` AND the accent had a space after it too (e.g. `w ¤ ahlen`),
                // collapse that spurious PDF kerning space unless `Ü/Ä/Ö` follows an article/preposition word.
                if had_space_after && out.ends_with(' ') {
                    let without_space = &out[..out.len() - 1];
                    if let Some(prev_ch) = without_space.chars().next_back() {
                        if prev_ch.is_ascii_alphabetic() {
                            let prev_word = without_space
                                .rsplit(|c: char| !c.is_alphabetic())
                                .next()
                                .unwrap_or("");
                            let keep_word_space = (raw_vowel.is_uppercase()
                                || prev_word.chars().count() >= 3)
                                && is_standalone_german_word_before_upper(prev_word);
                            if !keep_word_space {
                                out.pop();
                            }
                        }
                    }
                }
                out.push(umlaut);
                i = next_idx;
                continue;
            }
        }

        out.push(ch);
        i += 1;
    }

    out
}

pub fn list_workspace_tree(workspace_root: &Path, max_entries: usize) -> Vec<WorkspaceFileNode> {
    let mut nodes = Vec::new();
    let walk_root = strip_verbatim_prefix(
        std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf()),
    );
    let walker = WalkDir::new(&walk_root)
        .max_depth(5)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
            )
        });

    for entry in walker.flatten() {
        let p = entry.path();
        let Some(rel_raw) = relative_to_workspace(&walk_root, p) else {
            continue;
        };
        if rel_raw.is_empty() {
            continue;
        }
        let rel = Path::new(&rel_raw);
        let is_dir = entry.file_type().is_dir();
        let size_bytes = if is_dir {
            0
        } else {
            entry.metadata().map(|m| m.len()).unwrap_or(0)
        };
        let extension = if is_dir {
            String::new()
        } else {
            p.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        };
        nodes.push(WorkspaceFileNode {
            name: entry.file_name().to_string_lossy().to_string(),
            rel_path: rel.to_string_lossy().replace('\\', "/"),
            is_dir,
            size_bytes,
            extension,
        });
        if nodes.len() >= max_entries {
            break;
        }
    }
    nodes
}

/// Strictly confines any path access inside workspace_root.
pub fn resolve_workspace_path(workspace_root: &Path, user_path: &str) -> anyhow::Result<PathBuf> {
    if workspace_root.as_os_str().is_empty() {
        anyhow::bail!(
            "No project workspace is attached to this conversation. Open or select a project folder in the sidebar to access files."
        );
    }
    let canon_root = std::fs::canonicalize(workspace_root).map_err(|e| {
        anyhow::anyhow!("Invalid workspace root '{}': {e}", workspace_root.display())
    })?;
    let clean_canon_root = strip_verbatim_prefix(canon_root.clone());

    // 1. Strip surrounding quotes/backticks, file:// URI prefixes, and verbatim prefixes.
    let mut cleaned = user_path.trim().to_string();
    for (open, close) in [('"', '"'), ('\'', '\''), ('`', '`')] {
        if cleaned.len() >= 2 && cleaned.starts_with(open) && cleaned.ends_with(close) {
            cleaned = cleaned[1..cleaned.len() - 1].trim().to_string();
        }
    }
    if cleaned.contains("%20") {
        cleaned = cleaned.replace("%20", " ");
    }
    if let Some(rest) = cleaned.strip_prefix("file:///") {
        let has_win_drive = rest.as_bytes().get(1) == Some(&b':');
        cleaned = if cfg!(windows) || has_win_drive {
            rest.to_string()
        } else {
            format!("/{rest}")
        };
    } else if let Some(rest) = cleaned.strip_prefix("file://") {
        cleaned = rest.to_string();
    }

    let trimmed = cleaned.trim();
    let no_verbatim = if let Some(rest) = trimmed.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = trimmed.strip_prefix(r"\\?\") {
        rest.to_string()
    } else if let Some(rest) = trimmed.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("//?/") {
        rest.to_string()
    } else {
        trimmed.to_string()
    };

    let mut candidate_raw = no_verbatim.trim().to_string();
    if let Some(after_at) = candidate_raw.strip_prefix('@') {
        let direct = after_at.trim_start_matches(['/', '\\']);
        if !candidate_raw.is_empty() && !clean_canon_root.join(&candidate_raw).exists() && !direct.is_empty() {
            candidate_raw = after_at.trim().to_string();
        }
    }

    let candidate_buf: PathBuf = {
        let p = Path::new(candidate_raw.as_str());
        if !p.is_absolute()
            && !candidate_raw.starts_with(r"\\")
            && !candidate_raw.starts_with("//")
        {
            PathBuf::from(
                candidate_raw
                    .trim_start_matches(['/', '\\'])
                    .replace('\\', "/"),
            )
        } else {
            #[cfg(not(windows))]
            let mapped = if p.is_absolute()
                && !candidate_raw.starts_with("//")
                && !p.starts_with(&clean_canon_root)
                && !p.starts_with(workspace_root)
                && !p.starts_with(&strip_verbatim_prefix(workspace_root.to_path_buf()))
            {
                let rel = candidate_raw.trim_start_matches('/').replace('\\', "/");
                if !rel.is_empty()
                    && !Path::new(&rel)
                        .components()
                        .any(|c| matches!(c, Component::ParentDir))
                    && !p.exists()
                {
                    let ws_probe = clean_canon_root.join(&rel);
                    let os_parent_exists = p
                        .parent()
                        .is_some_and(|pp| pp != Path::new("/") && pp.exists());
                    let ws_parent_exists = ws_probe
                        .parent()
                        .is_some_and(|wp| wp != clean_canon_root && wp.exists());
                    if ws_probe.exists() || (!os_parent_exists && ws_parent_exists) {
                        Some(PathBuf::from(rel))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            #[cfg(windows)]
            let mapped: Option<PathBuf> = None;

            mapped.unwrap_or_else(|| p.to_path_buf())
        }
    };
    let candidate = candidate_buf.as_path();

    if candidate_raw.is_empty()
        || candidate_raw == "."
        || candidate.as_os_str().is_empty()
        || candidate == Path::new(".")
    {
        return Ok(clean_canon_root);
    }

    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        anyhow::bail!("Path traversal ('..') outside workspace is blocked: '{user_path}'");
    }

    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        let mut normal_parts: Vec<std::ffi::OsString> = Vec::new();
        for comp in candidate.components() {
            match comp {
                Component::CurDir => {}
                Component::Normal(part) => normal_parts.push(part.to_os_string()),
                Component::ParentDir => {
                    anyhow::bail!(
                        "Path traversal ('..') outside workspace is blocked: '{user_path}'"
                    );
                }
                Component::RootDir | Component::Prefix(_) => {
                    anyhow::bail!("Absolute path prefix blocked: '{user_path}'");
                }
            }
        }

        let mut joined_raw = clean_canon_root.clone();
        for part in &normal_parts {
            joined_raw.push(part);
        }

        // If the model redundantly prefixed the workspace folder name (e.g. `<root_name>/sub/file`),
        // strip that redundant prefix when the prefixed path does not exist and the unprefixed one (or its parent) does.
        let mut resolved = joined_raw.clone();
        if let (Some(first), Some(ws_name)) = (
            normal_parts.first().and_then(|s| s.to_str()),
            clean_canon_root.file_name().and_then(|s| s.to_str()),
        ) {
            let matches_ws_name = if cfg!(windows) {
                first.eq_ignore_ascii_case(ws_name)
            } else {
                first == ws_name
            };
            if matches_ws_name && !joined_raw.exists() {
                let mut joined_stripped = clean_canon_root.clone();
                for part in normal_parts.iter().skip(1) {
                    joined_stripped.push(part);
                }
                let stripped_parent_exists = joined_stripped
                    .parent()
                    .map(|p| p.exists())
                    .unwrap_or(false);
                let raw_parent_exists = joined_raw
                    .parent()
                    .map(|p| p.exists())
                    .unwrap_or(false);
                if normal_parts.len() == 1
                    || joined_stripped.exists()
                    || (stripped_parent_exists && !raw_parent_exists)
                {
                    resolved = joined_stripped;
                }
            }
        }
        resolved
    };

    let mut probe = joined.as_path();
    let existing = loop {
        if let Ok(meta) = std::fs::symlink_metadata(probe) {
            if meta.file_type().is_symlink() {
                if !probe.exists() {
                    anyhow::bail!(
                        "Sandbox policy blocked dangling symlink in workspace: '{user_path}'"
                    );
                }
            }
            if probe.exists() {
                break std::fs::canonicalize(probe)?;
            }
        }
        match probe.parent() {
            Some(p) => probe = p,
            None => anyhow::bail!("Path escapes workspace root: '{user_path}'"),
        }
    };

    let clean_existing = strip_verbatim_prefix(existing.clone());
    let inside = existing.starts_with(&canon_root)
        || clean_existing.starts_with(&clean_canon_root)
        || relative_to_workspace(&clean_canon_root, &clean_existing).is_some();
    if !inside {
        anyhow::bail!(
            "Sandbox policy blocked access outside workspace '{}': '{user_path}'",
            clean_canon_root.display()
        );
    }

    // Verify no intermediate path component along `joined` is an escaping/dangling symlink.
    if let Some(rel) = relative_to_workspace(&clean_canon_root, &joined) {
        let mut walk = clean_canon_root.clone();
        for seg in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
            walk.push(seg);
            if let Ok(meta) = std::fs::symlink_metadata(&walk) {
                if meta.file_type().is_symlink() {
                    if !walk.exists() {
                        anyhow::bail!(
                            "Sandbox policy blocked dangling symlink in workspace: '{user_path}'"
                        );
                    }
                    let target_canon = strip_verbatim_prefix(std::fs::canonicalize(&walk)?);
                    if !target_canon.starts_with(&clean_canon_root) {
                        anyhow::bail!(
                            "Sandbox policy blocked symlink outside workspace '{}': '{user_path}'",
                            clean_canon_root.display()
                        );
                    }
                }
            }
        }
    }

    if candidate.is_absolute() {
        if let Some(rel) = relative_to_workspace(&clean_canon_root, &joined) {
            let mut out = clean_canon_root.clone();
            for seg in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
                out.push(seg);
            }
            return Ok(out);
        }
    }

    Ok(strip_verbatim_prefix(joined))
}

pub fn extract_pdf_file_text(abs_path: &Path, max_pages: usize) -> anyhow::Result<String> {
    let doc = lopdf::Document::load(abs_path)?;
    let pages = doc.get_pages();
    let mut page_nums: Vec<u32> = pages.keys().copied().collect();
    page_nums.sort_unstable();
    let total_pages = page_nums.len();
    page_nums.truncate(max_pages);

    let mut out = String::new();
    out.push_str(&format!(
        "PDF Pages extracted: {}/{total_pages}\n\n",
        page_nums.len()
    ));
    for p in page_nums {
        match doc.extract_text(&[p]) {
            Ok(txt) => {
                let normalized = clean_latex_pdf_text(&txt);
                let cleaned = normalized.trim();
                out.push_str(&format!("--- Page {p} ---\n{cleaned}\n\n"));
            }
            Err(e) => {
                out.push_str(&format!(
                    "--- Page {p} ---\n[Unable to extract text: {e}]\n\n"
                ));
            }
        }
    }
    if out.len() > 24_000 {
        let mut end = 24_000;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str("\n... [truncated at 24,000 chars]");
    }
    Ok(out)
}

pub fn create_workspace_file(workspace_root: &Path, rel_path: &str) -> anyhow::Result<PathBuf> {
    let trimmed = rel_path.trim().trim_matches('/').trim_matches('\\');
    if trimmed.is_empty() {
        anyhow::bail!("File name cannot be empty.");
    }
    let abs = resolve_workspace_path(workspace_root, trimmed)?;
    if abs.exists() {
        anyhow::bail!("'{}' already exists.", trimmed);
    }
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&abs, "")?;
    Ok(abs)
}

pub fn create_workspace_dir(workspace_root: &Path, rel_path: &str) -> anyhow::Result<PathBuf> {
    let trimmed = rel_path.trim().trim_matches('/').trim_matches('\\');
    if trimmed.is_empty() {
        anyhow::bail!("Folder name cannot be empty.");
    }
    let abs = resolve_workspace_path(workspace_root, trimmed)?;
    if abs.exists() {
        anyhow::bail!("'{}' already exists.", trimmed);
    }
    std::fs::create_dir_all(&abs)?;
    Ok(abs)
}

pub fn rename_workspace_entry(
    workspace_root: &Path,
    old_rel: &str,
    new_name: &str,
) -> anyhow::Result<String> {
    let clean_name = new_name.trim();
    if clean_name.is_empty()
        || clean_name.contains('/')
        || clean_name.contains('\\')
        || clean_name.contains("..")
        || clean_name == "."
    {
        anyhow::bail!("Invalid target name '{new_name}'. Provide a single file or folder name.");
    }

    let abs_old = resolve_workspace_path(workspace_root, old_rel)?;
    let canon_root = strip_verbatim_prefix(std::fs::canonicalize(workspace_root)?);
    if abs_old == canon_root {
        anyhow::bail!("Cannot rename the workspace root itself.");
    }
    if !abs_old.exists() {
        anyhow::bail!("Source path '{}' does not exist.", old_rel);
    }

    let parent = abs_old
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing parent directory"))?;
    let abs_new = parent.join(clean_name);
    if !abs_new.starts_with(&canon_root) {
        anyhow::bail!("Rename target escapes workspace root.");
    }
    if abs_new.exists() {
        anyhow::bail!("Destination '{}' already exists.", clean_name);
    }

    std::fs::rename(&abs_old, &abs_new)?;
    let rel_new = relative_to_workspace(workspace_root, &abs_new).unwrap_or_else(|| {
        strip_verbatim_prefix(abs_new.clone())
            .to_string_lossy()
            .replace('\\', "/")
    });
    Ok(rel_new)
}

pub fn delete_workspace_entry(workspace_root: &Path, rel_path: &str) -> anyhow::Result<()> {
    let abs = resolve_workspace_path(workspace_root, rel_path)?;
    let canon_root = std::fs::canonicalize(workspace_root)?;
    if abs == canon_root || abs == strip_verbatim_prefix(canon_root) {
        anyhow::bail!("Refusing to delete the workspace root directory.");
    }
    if !abs.exists() {
        anyhow::bail!("Path '{}' does not exist.", rel_path);
    }
    if abs.is_dir() {
        std::fs::remove_dir_all(&abs)?;
    } else {
        std::fs::remove_file(&abs)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_blocks_parent_traversal() {
        let root = std::env::temp_dir();
        let err = resolve_workspace_path(&root, "../secret.txt");
        assert!(err.is_err());
    }

    #[test]
    fn relative_to_workspace_handles_verbatim_prefix_mismatch() {
        #[cfg(windows)]
        {
            let root = PathBuf::from(r"C:\Users\alice\Workspace");
            let entry = PathBuf::from(r"\\?\C:\Users\alice\Workspace\Vorlesung\notes.md");
            assert_eq!(
                relative_to_workspace(&root, &entry).as_deref(),
                Some("Vorlesung/notes.md")
            );
            let root_verb = PathBuf::from(r"\\?\C:\Users\alice\Workspace");
            let entry_plain = PathBuf::from(r"C:\Users\alice\Workspace\README.md");
            assert_eq!(
                relative_to_workspace(&root_verb, &entry_plain).as_deref(),
                Some("README.md")
            );
        }

        let posix_root = PathBuf::from("/Users/alice/Workspace");
        let posix_entry = PathBuf::from("/Users/alice/Workspace/Vorlesung/notes.md");
        assert_eq!(
            relative_to_workspace(&posix_root, &posix_entry).as_deref(),
            Some("Vorlesung/notes.md")
        );

        let verbatim_root = PathBuf::from("Users/alice/Workspace");
        let verbatim_entry = PathBuf::from("//?/Users/alice/Workspace/Vorlesung/notes.md");
        assert_eq!(
            relative_to_workspace(&verbatim_root, &verbatim_entry).as_deref(),
            Some("Vorlesung/notes.md")
        );
    }

    #[test]
    fn resolve_workspace_path_allows_leading_slash_relative_paths() {
        let root = std::env::temp_dir().join(format!(
            "cinder_sandbox_relslash_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("note.md"), "hi").unwrap();

        let p1 = resolve_workspace_path(&root, "/sub/note.md").unwrap();
        assert_eq!(
            relative_to_workspace(&root, &p1).as_deref(),
            Some("sub/note.md")
        );
        let p2 = resolve_workspace_path(&root, ".").unwrap();
        assert_eq!(relative_to_workspace(&root, &p2).as_deref(), Some(""));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_workspace_path_normalizes_quotes_uri_and_duplicated_root_name() {
        let ws_folder = format!("cinder_sandbox_norm_{}", uuid::Uuid::new_v4());
        let root = std::env::temp_dir().join(&ws_folder);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("note.md"), "hi").unwrap();

        // Quotes and backticks
        let p_quote = resolve_workspace_path(&root, " `sub/note.md` ").unwrap();
        assert_eq!(
            relative_to_workspace(&root, &p_quote).as_deref(),
            Some("sub/note.md")
        );

        // Leading '@' reference
        let p_at = resolve_workspace_path(&root, "@sub/note.md").unwrap();
        assert_eq!(
            relative_to_workspace(&root, &p_at).as_deref(),
            Some("sub/note.md")
        );

        // Redundant workspace folder prefix
        let dup_rel = format!("{ws_folder}/sub/note.md");
        let p_dup = resolve_workspace_path(&root, &dup_rel).unwrap();
        assert_eq!(
            relative_to_workspace(&root, &p_dup).as_deref(),
            Some("sub/note.md")
        );

        // Redundant workspace folder alone resolves to root
        let p_root = resolve_workspace_path(&root, &ws_folder).unwrap();
        assert_eq!(relative_to_workspace(&root, &p_root).as_deref(), Some(""));

        // Creating a new file with duplicated workspace prefix
        let new_dup = format!("{ws_folder}/sub/fresh.css");
        let p_new = resolve_workspace_path(&root, &new_dup).unwrap();
        assert_eq!(
            relative_to_workspace(&root, &p_new).as_deref(),
            Some("sub/fresh.css")
        );

        // Absolute path inside workspace via file:/// URI
        let abs_norm = root.join("sub").join("note.md").to_string_lossy().replace('\\', "/");
        let uri = if abs_norm.starts_with('/') {
            format!("file://{abs_norm}")
        } else {
            format!("file:///{abs_norm}")
        };
        let p_uri = resolve_workspace_path(&root, &uri).unwrap();
        assert_eq!(
            relative_to_workspace(&root, &p_uri).as_deref(),
            Some("sub/note.md")
        );

        // Outside-workspace path must still be blocked
        let outside = std::env::temp_dir().join("outside_secret_file.txt");
        assert!(resolve_workspace_path(&root, &outside.to_string_lossy()).is_err());
        #[cfg(not(windows))]
        {
            assert!(resolve_workspace_path(&root, "/etc/hosts").is_err());
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clean_latex_pdf_text_repairs_german_umlauts_and_ligatures() {
        let raw = "W ¤ ahlen Sie die richtige L ¤ osung f ¤ ur eine ¤ Ubung \u{FB01}nden a\u{0308} ¨o";
        let cleaned = clean_latex_pdf_text(raw);
        assert_eq!(
            cleaned,
            "Wählen Sie die richtige Lösung für eine Übung finden ä ö"
        );
    }

    #[test]
    fn sandbox_create_rename_and_delete_entries() {
        let root = std::env::temp_dir().join(format!(
            "cinder_sandbox_crud_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();

        create_workspace_dir(&root, "sub").unwrap();
        assert!(root.join("sub").is_dir());

        create_workspace_file(&root, "sub/hello.txt").unwrap();
        assert!(root.join("sub").join("hello.txt").is_file());

        // Traversal or slash in new_name must be rejected
        assert!(rename_workspace_entry(&root, "sub/hello.txt", "../escape.txt").is_err());
        assert!(rename_workspace_entry(&root, "sub/hello.txt", "a/b.txt").is_err());

        let renamed = rename_workspace_entry(&root, "sub/hello.txt", "world.txt").unwrap();
        assert_eq!(renamed, "sub/world.txt");
        assert!(root.join("sub").join("world.txt").is_file());
        assert!(!root.join("sub").join("hello.txt").exists());

        delete_workspace_entry(&root, "sub/world.txt").unwrap();
        assert!(!root.join("sub").join("world.txt").exists());

        // Refusing to delete workspace root itself
        assert!(delete_workspace_entry(&root, ".").is_err());

        delete_workspace_entry(&root, "sub").unwrap();
        assert!(!root.join("sub").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_workspace_path_blocks_symlinks_pointing_outside_workspace() {
        let base = std::env::temp_dir().join(format!("cinder_symlink_test_{}", uuid::Uuid::new_v4()));
        let ws = base.join("ws");
        let outside_dir = base.join("outside");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        let secret = outside_dir.join("secret.txt");
        std::fs::write(&secret, "top_secret").unwrap();

        let link = ws.join("escape_link.txt");
        #[cfg(unix)]
        let link_res = std::os::unix::fs::symlink(&secret, &link);
        #[cfg(windows)]
        let link_res = std::os::windows::fs::symlink_file(&secret, &link);

        // Non-elevated Windows accounts may lack SeCreateSymbolicLinkPrivilege; assert whenever creation succeeds.
        if link_res.is_ok() {
            assert!(
                resolve_workspace_path(&ws, "escape_link.txt").is_err(),
                "symlink pointing outside workspace must be rejected"
            );
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_workspace_path_blocks_dangling_symlinks_pointing_outside_workspace() {
        let base =
            std::env::temp_dir().join(format!("cinder_dangling_symlink_{}", uuid::Uuid::new_v4()));
        let ws = base.join("ws");
        let outside_nonexistent = base.join("outside_missing.txt");
        std::fs::create_dir_all(&ws).unwrap();

        let link = ws.join("dangling.txt");
        #[cfg(unix)]
        let link_res = std::os::unix::fs::symlink(&outside_nonexistent, &link);
        #[cfg(windows)]
        let link_res = std::os::windows::fs::symlink_file(&outside_nonexistent, &link);

        if link_res.is_ok() {
            assert!(
                resolve_workspace_path(&ws, "dangling.txt").is_err(),
                "dangling symlink pointing outside workspace must be rejected"
            );
        }

        let _ = std::fs::remove_dir_all(&base);
    }
}
