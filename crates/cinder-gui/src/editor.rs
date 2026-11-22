use crate::app_state::GuiState;
use crate::backend::BackendHandle;
use crate::icons::Icon;
use crate::markdown::{highlight_code_job, render_markdown};
use crate::theme::{
    copy_to_clipboard, empty_state_block, ghost_button, input_frame, primary_button,
    secondary_button, CinderPalette,
};
use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use std::sync::Arc;

pub fn render_editor_view(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    if !state.editor_tabs.is_empty() {
        ui.horizontal_wrapped(|ui| {
            let mut close_idx = None;
            let mut next_active = None;
            for (idx, tab) in state.editor_tabs.iter().enumerate() {
                let is_active = state.active_tab_path.as_deref() == Some(&tab.rel_path);
                let name = tab
                    .rel_path
                    .split('/')
                    .next_back()
                    .unwrap_or(&tab.rel_path);
                let title = if tab.is_dirty() {
                    format!("{name} *")
                } else {
                    name.to_string()
                };
                Frame::new()
                    .fill(if is_active {
                        p.bg_card
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(if is_active {
                        Stroke::new(1.0_f32, p.border_strong)
                    } else {
                        Stroke::NONE
                    })
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let lbl_col = if is_active { p.text } else { p.muted };
                            if ui
                                .add(
                                    egui::Label::new(egui::RichText::new(&title).color(lbl_col))
                                        .wrap_mode(egui::TextWrapMode::Extend)
                                        .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                next_active = Some(tab.rel_path.clone());
                            }
                            if ghost_button(ui, p, egui::RichText::new("×").small()).clicked() {
                                close_idx = Some(idx);
                            }
                        });
                    });
            }
            if let Some(p_act) = next_active {
                state.active_tab_path = Some(p_act);
            }
            if let Some(cidx) = close_idx {
                let removed = state.editor_tabs.remove(cidx);
                if state.active_tab_path.as_deref() == Some(&removed.rel_path) {
                    state.active_tab_path = state.editor_tabs.last().map(|t| t.rel_path.clone());
                }
            }
        });
        ui.separator();
    }

    let term_reserve = if state.show_terminal {
        state.terminal_height + 16.0
    } else {
        0.0
    };
    let editor_avail_h = (ui.available_height() - term_reserve).max(160.0);

    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), editor_avail_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let active_idx = state
                .active_tab_path
                .as_ref()
                .and_then(|ap| state.editor_tabs.iter().position(|t| &t.rel_path == ap));

            if let Some(idx) = active_idx {
                let rel_path = state.editor_tabs[idx].rel_path.clone();
                let abs_path = state.editor_tabs[idx].abs_path.clone();
                let kind = state.editor_tabs[idx].kind.clone();
                let is_dirty = state.editor_tabs[idx].is_dirty();
                let preview_mode = state.editor_tabs[idx].preview_mode;
                let is_markdown = {
                    let lower = rel_path.to_ascii_lowercase();
                    lower.ends_with(".md") || lower.ends_with(".markdown")
                };

                // Toolbar
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&rel_path).monospace().color(p.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if kind == "text" {
                            let save_clicked = if is_dirty {
                                primary_button(ui, p, "Save (Ctrl+S)").clicked()
                            } else {
                                secondary_button(ui, p, "Save (Ctrl+S)").clicked()
                            };
                            if save_clicked {
                                let txt = state.editor_tabs[idx].content.clone();
                                state.editor_tabs[idx].saved_content = txt.clone();
                                backend.save_workspace_file(rel_path.clone(), txt);
                            }
                            if is_markdown {
                                let toggle_lbl = if preview_mode { "Edit" } else { "Preview" };
                                if secondary_button(ui, p, toggle_lbl).clicked() {
                                    state.editor_tabs[idx].preview_mode = !preview_mode;
                                }
                            }
                            if secondary_button(ui, p, "Find (Ctrl+F)").clicked() {
                                state.editor_find_open = !state.editor_find_open;
                            }
                        } else if secondary_button(ui, p, "Open Externally").clicked() {
                            let _ = harness_tools::open_with_system_default(&abs_path);
                        }
                        if secondary_button(ui, p, "Reveal").clicked() {
                            backend.reveal_entry(abs_path.clone());
                        }
                        if secondary_button(ui, p, "Attach to Chat").clicked() {
                            if !state.attached_files.iter().any(|f| f == &rel_path) {
                                state.attached_files.push(rel_path.clone());
                                state.set_toast(format!("Attached {rel_path} to chat"), false);
                            }
                        }
                    });
                });

                if kind == "text" && state.editor_find_open {
                    ui.add_space(4.0);
                    let match_count = if state.editor_find_query.is_empty() {
                        0
                    } else {
                        state.editor_tabs[idx]
                            .content
                            .matches(&state.editor_find_query)
                            .count()
                    };
                    Frame::new()
                        .fill(p.bg_elev)
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .corner_radius(CornerRadius::same(7))
                        .inner_margin(Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                input_frame(p, false).show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut state.editor_find_query)
                                            .hint_text("Find...")
                                            .desired_width(140.0)
                                            .frame(false),
                                    );
                                });
                                let cnt_txt = if state.editor_find_query.is_empty() {
                                    "0 matches".to_string()
                                } else {
                                    format!("{match_count} matches")
                                };
                                ui.label(egui::RichText::new(cnt_txt).small().color(p.muted));
                                input_frame(p, false).show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut state.editor_replace_query)
                                            .hint_text("Replace...")
                                            .desired_width(130.0)
                                            .frame(false),
                                    );
                                });
                                if !state.editor_find_query.is_empty() && match_count > 0 {
                                    if secondary_button(ui, p, "Replace Next").clicked() {
                                        let next = state.editor_tabs[idx].content.replacen(
                                            &state.editor_find_query,
                                            &state.editor_replace_query,
                                            1,
                                        );
                                        state.editor_tabs[idx].content = next;
                                    }
                                    if secondary_button(ui, p, "Replace All").clicked() {
                                        let next = state.editor_tabs[idx].content.replace(
                                            &state.editor_find_query,
                                            &state.editor_replace_query,
                                        );
                                        state.editor_tabs[idx].content = next;
                                    }
                                }
                                if ghost_button(ui, p, "×").clicked() {
                                    state.editor_find_open = false;
                                }
                            });
                        });
                }

                ui.separator();

                if kind == "image" {
                    let bytes = state.editor_tabs[idx].bytes.clone();
                    let tex_key = format!("ed_img_{rel_path}");
                    if let Some(tex) = state.get_or_load_texture(ctx, &tex_key, &bytes) {
                        egui::ScrollArea::both()
                            .id_salt(("ed_img_scroll", &rel_path))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Image::new(&tex)
                                        .max_width(ui.available_width() - 16.0)
                                        .corner_radius(CornerRadius::same(8)),
                                );
                            });
                    } else {
                        ui.label("Could not decode image preview.");
                    }
                } else if kind == "pdf" {
                    egui::ScrollArea::vertical()
                        .id_salt(("ed_pdf_scroll", &rel_path))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&state.editor_tabs[idx].content)
                                    .monospace()
                                    .size(12.5),
                            );
                        });
                } else if kind == "text" && preview_mode && is_markdown {
                    egui::ScrollArea::vertical()
                        .id_salt(("ed_md_prev", &rel_path))
                        .show(ui, |ui| {
                            let txt = state.editor_tabs[idx].content.clone();
                            render_markdown(ui, p, &txt, true, None);
                        });
                } else if kind == "text" {
                    let ext = rel_path
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let line_count = state.editor_tabs[idx].content.lines().count().max(1);
                    let gutter: String = (1..=line_count)
                        .map(|n| format!("{n:>4}"))
                        .collect::<Vec<_>>()
                        .join("\n");

                    egui::ScrollArea::both()
                        .id_salt(("ed_code_scroll", &rel_path))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    egui::RichText::new(gutter)
                                        .monospace()
                                        .color(p.muted),
                                );
                                let mut layouter =
                                    |ui: &egui::Ui, string: &str, wrap_width: f32| {
                                        let mut j = highlight_code_job(string, &ext, p);
                                        j.wrap.max_width = wrap_width;
                                        ui.fonts(|f| f.layout_job(j))
                                    };
                                let ed_resp = ui.add(
                                    egui::TextEdit::multiline(&mut state.editor_tabs[idx].content)
                                        .font(egui::TextStyle::Monospace)
                                        .layouter(&mut layouter)
                                        .code_editor()
                                        .desired_width(f32::INFINITY)
                                        .frame(false),
                                );
                                ed_resp.context_menu(|ui| {
                                    if ui.button("Copy File Contents").clicked() {
                                        copy_to_clipboard(ctx, &state.editor_tabs[idx].content);
                                        ui.close_menu();
                                    }
                                    if ui.button("Cut (Copy + Clear)").clicked() {
                                        copy_to_clipboard(ctx, &state.editor_tabs[idx].content);
                                        state.editor_tabs[idx].content.clear();
                                        ui.close_menu();
                                    }
                                    if ui.button("Paste from Clipboard").clicked() {
                                        if let Ok(txt) =
                                            arboard::Clipboard::new().and_then(|mut c| c.get_text())
                                        {
                                            state.editor_tabs[idx].content.push_str(&txt);
                                        }
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button("Save File (Ctrl+S)").clicked() {
                                        let txt = state.editor_tabs[idx].content.clone();
                                        state.editor_tabs[idx].saved_content = txt.clone();
                                        backend.save_workspace_file(rel_path.clone(), txt);
                                        ui.close_menu();
                                    }
                                    if ui.button("Attach to Chat").clicked() {
                                        if !state.attached_files.iter().any(|f| f == &rel_path) {
                                            state.attached_files.push(rel_path.clone());
                                        }
                                        ui.close_menu();
                                    }
                                    if ui.button("Reveal in Explorer").clicked() {
                                        backend.reveal_entry(abs_path.clone());
                                        ui.close_menu();
                                    }
                                });
                            });
                        });
                } else {
                    ui.label(&state.editor_tabs[idx].content);
                }
            } else {
                empty_state_block(
                    ui,
                    p,
                    Icon::FileCode,
                    "No file open",
                    "Select a file in Workspace Explorer (Ctrl+Shift+E) or drag and drop files anywhere.",
                    if state.show_explorer {
                        None
                    } else {
                        Some("Open File Explorer")
                    },
                )
                .then(|| state.show_explorer = true);
            }
        },
    );

    if state.show_terminal {
        ui.separator();
        render_terminal_drawer(ctx, ui, state, backend, p);
    }
}

fn render_terminal_drawer(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    Frame::new()
        .fill(p.bg_sidebar)
        .stroke(Stroke::new(1.0_f32, p.border_strong))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_height(state.terminal_height);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Workspace Terminal").strong());
                for qcmd in ["git status", "cargo check", "cargo test", "pytest"] {
                    if secondary_button(ui, p, qcmd).clicked() {
                        state.terminal_cmd = qcmd.into();
                        backend.run_terminal(qcmd.into());
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(ui, p, "×").clicked() {
                        state.show_terminal = false;
                    }
                    if secondary_button(ui, p, "Send Output to Chat").clicked() {
                        state.prompt_text.push_str(&format!(
                            "\n\n```text\n{}\n```",
                            state.terminal_output
                        ));
                    }
                });
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("$")
                        .monospace()
                        .strong()
                        .color(p.accent),
                );
                let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                input_frame(p, false).show(ui, |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut state.terminal_cmd)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("Run PowerShell/shell command in active workspace...")
                            .desired_width((ui.available_width() - 96.0).max(140.0))
                            .frame(false),
                    );
                    if resp.lost_focus() && enter {
                        let cmd = state.terminal_cmd.clone();
                        backend.run_terminal(cmd);
                    }
                });
                if primary_button(ui, p, "Run").clicked() {
                    let cmd = state.terminal_cmd.clone();
                    backend.run_terminal(cmd);
                }
            });

            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("term_output_scroll")
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&state.terminal_output)
                            .monospace()
                            .size(11.5)
                            .color(p.muted),
                    );
                });
        });
}
