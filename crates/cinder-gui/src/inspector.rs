use crate::app_state::{GuiState, PromptModal, StageTab};
use crate::backend::BackendHandle;
use crate::icons::Icon;
use crate::markdown::{render_diff, render_markdown, render_vertical_resize_handle};
use crate::theme::{
    badge, copy_to_clipboard, danger_button, empty_state_block, input_frame, primary_button,
    secondary_button, CinderPalette,
};
use harness_store::FileChangeStatus;
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
                    for art in &sess.artifacts {
                        p.card_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&art.title).strong());
                                badge(ui, &art.kind, p.accent_hot, p.accent_soft);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
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
                                            && secondary_button(ui, p, "Open in Browser").clicked()
                                        {
                                            let tmp = std::env::temp_dir()
                                                .join(format!("cinder_widget_{}.html", art.id));
                                            if std::fs::write(&tmp, &art.content).is_ok() {
                                                let _ = harness_tools::open_with_system_default(&tmp);
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
                }
            }

            StageTab::Diffs => {
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
                                        egui::RichText::new(&f.rel_path).monospace().strong(),
                                    );
                                    badge(ui, &format!("{:?}", f.status), scol, p.bg_card);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if f.status != FileChangeStatus::Reverted
                                                && secondary_button(ui, p, "Revert").clicked()
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

            StageTab::Editor => {}
        });
}
