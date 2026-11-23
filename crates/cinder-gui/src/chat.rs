use crate::app_state::{
    decode_base64_simple, fmt_bytes, fmt_rel_age, mode_label, norm_path,
    resolve_model_display_name, GuiState, PromptModal, SelectableModel, SettingsTab, StageTab,
};
use crate::backend::{BackendHandle, FolderPickTarget};
use crate::icons::{draw_icon, icon_button, Icon};
use crate::markdown::{render_diff, render_markdown, render_resizable_text_box};
use crate::theme::{
    badge, copy_to_clipboard, danger_button, draw_cinder_flame, draw_cinder_orb_animated,
    draw_working_indicator, ghost_button, input_frame, is_copied_recently, mark_copied_feedback,
    pane_header, primary_button, request_smooth_anim_repaint, secondary_button, segmented_control,
    send_glow_button, CinderPalette,
};
use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use harness_store::{AgentMode, PlanStatus};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

fn animated_working_stroke(ui: &egui::Ui, p: &CinderPalette) -> Stroke {
    let t = ui.input(|i| i.time) as f32;
    let pulse = (t * 3.2).sin() * 0.5 + 0.5;
    let alpha = (85.0 + pulse * 105.0) as u8;
    Stroke::new(
        1.0_f32,
        Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), alpha),
    )
}

fn draw_working_dots(ui: &mut egui::Ui, p: &CinderPalette) {
    let t = ui.input(|i| i.time) as f32;
    let (dots_rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 12.0), egui::Sense::hover());
    for d in 0..3 {
        let phase = t * 5.0 - (d as f32) * 0.85;
        let lift = phase.sin().max(0.0);
        let a = (90.0 + lift * 160.0) as u8;
        let x = dots_rect.left() + 3.0 + (d as f32) * 6.5;
        let y = dots_rect.center().y + 1.5 - lift * 2.8;
        ui.painter().circle_filled(
            egui::pos2(x, y),
            1.6,
            Color32::from_rgba_unmultiplied(p.accent_hot.r(), p.accent_hot.g(), p.accent_hot.b(), a),
        );
    }
}

fn paint_live_shimmer_bar(ui: &mut egui::Ui, p: &CinderPalette) {
    let avail_w = ui.available_width().max(80.0);
    let (track, _) = ui.allocate_exact_size(egui::vec2(avail_w, 3.5), egui::Sense::hover());
    if !ui.is_rect_visible(track) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(track, CornerRadius::same(2), p.bg_hover);

    let t = ui.input(|i| i.time) as f32;
    let cycle = (t * 0.85).fract();
    let glow_w = (avail_w * 0.28).clamp(36.0, 140.0);
    let travel = avail_w + glow_w;
    let x0 = (track.left() - glow_w + cycle * travel).max(track.left());
    let x1 = (track.left() + cycle * travel).min(track.right());
    if x1 > x0 {
        let seg = egui::Rect::from_min_max(
            egui::pos2(x0, track.top()),
            egui::pos2(x1, track.bottom()),
        );
        painter.rect_filled(seg, CornerRadius::same(2), p.accent);
    }
    request_smooth_anim_repaint(ui.ctx());
}

pub fn render_chat_column(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };
    let Some(sess) = snap.active_session.clone() else {
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new("No active conversation.").color(p.muted));
        });
        return;
    };

    state.sync_turn_timer();
    if state.is_busy() {
        request_smooth_anim_repaint(ctx);
    }

    let is_standalone = sess.workspace_path.trim().is_empty();
    let ws_name = if is_standalone {
        "Conversation".to_string()
    } else {
        snap.workspaces
            .iter()
            .find(|w| norm_path(&w.path) == norm_path(&sess.workspace_path))
            .map(|w| w.name.clone())
            .unwrap_or_else(|| {
                PathBuf::from(&sess.workspace_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Workspace")
                    .to_string()
            })
    };

    let has_messages = !sess.messages.is_empty();

    // Exact 40px aligned pane header
    pane_header(ui, p, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.scope(|ui| {
                let vis = &mut ui.style_mut().visuals;
                vis.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                vis.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                vis.widgets.inactive.bg_stroke = Stroke::NONE;
                vis.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.muted);
                vis.widgets.hovered.weak_bg_fill = p.bg_hover;
                vis.widgets.hovered.bg_fill = p.bg_hover;
                vis.widgets.hovered.bg_stroke = Stroke::NONE;
                vis.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
                vis.widgets.open.bg_fill = p.bg_card;
                vis.widgets.open.bg_stroke = Stroke::NONE;
                ui.menu_button("⋯", |ui| {
                    if has_messages && ui.button("Compact Context (/compact)").clicked() {
                        backend.compact_chat(sess.id.clone());
                        ui.close_menu();
                    }
                    if has_messages && ui.button("Export Markdown transcript").clicked() {
                        copy_to_clipboard(ctx, &format_session_markdown(&sess));
                        state.set_toast("Copied Markdown transcript to clipboard.", false);
                        ui.close_menu();
                    }
                    if has_messages {
                        ui.separator();
                    }
                    if ui.button("+ New Conversation").clicked() {
                        backend.create_session(None, None);
                        ui.close_menu();
                    }
                    if ui.button("New One-Off Conversation").clicked() {
                        backend.create_standalone_session(None);
                        ui.close_menu();
                    }
                    if ui.button("Open Project Folder...").clicked() {
                        backend.open_folder_picker(
                            FolderPickTarget::Workspace,
                            "Select Project Folder for Cinder",
                        );
                        ui.close_menu();
                    }
                });
            });

            if icon_text_toggle(
                ui,
                p,
                Icon::PanelRight,
                state.show_workbench,
                "Stage",
                "Toggle Coding Stage (Ctrl+E)",
            )
            .clicked()
            {
                state.show_workbench = !state.show_workbench;
            }
            if icon_text_toggle(
                ui,
                p,
                Icon::FilesTree,
                state.show_explorer,
                "Files",
                "Toggle Workspace Explorer (Ctrl+Shift+E)",
            )
            .clicked()
            {
                state.show_explorer = !state.show_explorer;
            }

            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if !state.show_sidebar
                    && icon_button(ui, p, Icon::Sidebar, false, "Show Sidebar (Ctrl+B)").clicked()
                {
                    state.show_sidebar = true;
                }
                ui.label(egui::RichText::new(&ws_name).small().color(p.muted));
                ui.label(egui::RichText::new("/").small().color(p.muted));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&sess.title).strong().color(p.text),
                    )
                    .truncate(),
                );
                if is_standalone && !snap.workspaces.is_empty() {
                    ui.scope(|ui| {
                        let vis = &mut ui.style_mut().visuals;
                        vis.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                        vis.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                        vis.widgets.inactive.bg_stroke = Stroke::NONE;
                        vis.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.muted);
                        vis.widgets.hovered.weak_bg_fill = p.bg_hover;
                        vis.widgets.hovered.bg_fill = p.bg_hover;
                        vis.widgets.hovered.bg_stroke = Stroke::NONE;
                        ui.menu_button(
                            egui::RichText::new("Attach to Project ▼").small(),
                            |ui| {
                                for ws in &snap.workspaces {
                                    if ui.button(&ws.name).clicked() {
                                        backend.move_session_to_workspace(
                                            sess.id.clone(),
                                            Some(ws.path.clone()),
                                        );
                                        ui.close_menu();
                                    }
                                }
                            },
                        );
                    });
                }
            });
        });
    });

    // Centered readable column metrics; dock bottom banners & composer before message stream
    let panel_w = ui.max_rect().width().max(280.0);
    let col_w = (panel_w - 32.0).clamp(240.0, 800.0);

    let (changed_files, added_lines, removed_lines) = summarize_checkpoints(&sess);
    let has_changes_bar = changed_files > 0;
    let mut clear_pending_question = false;
    let mut clear_pending_approval = false;
    let mut clear_pending_fallback = false;

    egui::TopBottomPanel::bottom(egui::Id::new(("chat_bottom_dock", &sess.id)))
        .frame(Frame::NONE)
        .show_separator_line(false)
        .show_inside(ui, |ui| {
            ui.add_space(4.0);
            with_centered_column(ui, panel_w, col_w, |ui| {
                if let Some(ref pq) = sess.pending_question {
                    if pq.id.starts_with("step_contingent") {
                        Frame::new()
                            .fill(p.bg_card)
                            .stroke(Stroke::new(1.2_f32, p.amber))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    badge(ui, "AUTO STEP CONTINGENT", p.amber, p.bg_elev);
                                    ui.label(egui::RichText::new(&pq.question).strong());
                                });
                                ui.add_space(6.0);
                                ui.horizontal_wrapped(|ui| {
                                    for opt in &pq.options {
                                        let clicked = if opt.to_ascii_lowercase().contains("stop") {
                                            danger_button(ui, p, opt).clicked()
                                        } else if opt == "+4 Steps" || opt == "+8 Steps" {
                                            primary_button(ui, p, opt).clicked()
                                        } else {
                                            secondary_button(ui, p, opt).clicked()
                                        };
                                        if clicked {
                                            let note = std::mem::take(&mut state.question_custom_text);
                                            state.selected_question_opts.clear();
                                            state.is_generating = true;
                                            clear_pending_question = true;
                                            backend.answer_question(
                                                sess.id.clone(),
                                                vec![opt.clone()],
                                                note,
                                            );
                                        }
                                    }
                                    input_frame(p, false).show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut state.question_custom_text,
                                            )
                                            .hint_text("Steering note or step count...")
                                            .desired_width(230.0)
                                            .frame(false),
                                        );
                                    });
                                    if !state.question_custom_text.trim().is_empty()
                                        && primary_button(ui, p, "Continue").clicked()
                                    {
                                        let txt = std::mem::take(&mut state.question_custom_text);
                                        state.selected_question_opts.clear();
                                        state.is_generating = true;
                                        clear_pending_question = true;
                                        backend.answer_question(sess.id.clone(), Vec::new(), txt);
                                    }
                                });
                            });
                    } else {
                        Frame::new()
                            .fill(p.bg_card)
                            .stroke(Stroke::new(1.2_f32, p.purple))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    badge(ui, "INPUT REQUIRED", p.purple, p.bg_elev);
                                    ui.label(egui::RichText::new(&pq.question).strong());
                                });
                                ui.horizontal_wrapped(|ui| {
                                    for opt in &pq.options {
                                        let sel = state.selected_question_opts.contains(opt);
                                        let clicked = if sel {
                                            primary_button(ui, p, opt).clicked()
                                        } else {
                                            secondary_button(ui, p, opt).clicked()
                                        };
                                        if clicked {
                                            if !pq.allow_multiple {
                                                state.selected_question_opts.clear();
                                            }
                                            if sel {
                                                state.selected_question_opts.remove(opt);
                                            } else {
                                                state.selected_question_opts.insert(opt.clone());
                                            }
                                        }
                                    }
                                });
                                ui.horizontal(|ui| {
                                    input_frame(p, false).show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut state.question_custom_text,
                                            )
                                            .hint_text("Optional custom answer...")
                                            .desired_width(220.0)
                                            .frame(false),
                                        );
                                    });
                                    if primary_button(ui, p, "Submit Answer").clicked() {
                                        let opts: Vec<String> =
                                            state.selected_question_opts.iter().cloned().collect();
                                        let txt = state.question_custom_text.clone();
                                        state.selected_question_opts.clear();
                                        state.question_custom_text.clear();
                                        state.is_generating = true;
                                        clear_pending_question = true;
                                        backend.answer_question(sess.id.clone(), opts, txt);
                                    }
                                });
                            });
                    }
                    ui.add_space(4.0);
                }

                if let Some(ref pa) = sess.pending_approval {
                    Frame::new()
                        .fill(p.bg_card)
                        .stroke(Stroke::new(1.2_f32, p.amber))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                badge(ui, &format!("{:?}", pa.risk_tier), p.amber, p.bg_elev);
                                ui.label(
                                    egui::RichText::new(format!("Approve `{}`?", pa.tool_name))
                                        .strong(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if danger_button(ui, p, "Reject").clicked() {
                                            state.is_generating = true;
                                            clear_pending_approval = true;
                                            backend.approve_tool(sess.id.clone(), false);
                                        }
                                        if primary_button(ui, p, "Approve & Continue").clicked() {
                                            state.is_generating = true;
                                            clear_pending_approval = true;
                                            backend.approve_tool(sess.id.clone(), true);
                                        }
                                    },
                                );
                            });
                            if let Some(ref diff) = pa.diff_preview {
                                render_diff(ui, p, diff, &pa.id);
                            } else {
                                ui.label(
                                    egui::RichText::new(&pa.arguments_json)
                                        .monospace()
                                        .small()
                                        .color(p.muted),
                                );
                            }
                        });
                    ui.add_space(4.0);
                }

                if let Some(ref pf) = sess.pending_fallback {
                    let failed_lbl = resolve_model_display_name(&snap, &pf.failed_model_id);
                    Frame::new()
                        .fill(p.bg_card)
                        .stroke(Stroke::new(1.2_f32, p.amber))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                badge(ui, "MODEL FALLBACK", p.amber, p.bg_elev);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{failed_lbl} unavailable ({})",
                                        pf.reason
                                    ))
                                    .strong()
                                    .color(p.text),
                                );
                            });
                            if !pf.error_summary.trim().is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(&pf.error_summary)
                                        .small()
                                        .color(p.muted),
                                );
                            }
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if primary_button(ui, p, &format!("Retry {failed_lbl}")).clicked() {
                                    state.is_generating = true;
                                    state.turn_started_at = Some(std::time::Instant::now());
                                    clear_pending_fallback = true;
                                    backend.resolve_fallback(
                                        sess.id.clone(),
                                        Some(pf.failed_model_id.clone()),
                                    );
                                }
                                for cand_id in &pf.candidates {
                                    let cand_lbl = resolve_model_display_name(&snap, cand_id);
                                    if secondary_button(
                                        ui,
                                        p,
                                        &format!("Continue with {cand_lbl}"),
                                    )
                                    .on_hover_text(cand_id.as_str())
                                    .clicked()
                                    {
                                        state.is_generating = true;
                                        state.turn_started_at = Some(std::time::Instant::now());
                                        clear_pending_fallback = true;
                                        backend.resolve_fallback(
                                            sess.id.clone(),
                                            Some(cand_id.clone()),
                                        );
                                    }
                                }
                                if danger_button(ui, p, "Cancel").clicked() {
                                    clear_pending_fallback = true;
                                    backend.resolve_fallback(sess.id.clone(), None);
                                }
                            });
                        });
                    ui.add_space(4.0);
                }

                if state.is_busy() {
                    let elapsed = state.format_elapsed_since_turn_start();
                    Frame::new()
                        .fill(p.bg_elev)
                        .stroke(animated_working_stroke(ui, p))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (ind_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(14.0, 14.0),
                                    egui::Sense::hover(),
                                );
                                draw_working_indicator(ui, ind_rect.center(), 5.2, p);
                                let detail = if snap.live_activity.detail.is_empty() {
                                    "Forging response"
                                } else {
                                    snap.live_activity.detail.trim_end_matches('.')
                                };
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(detail).color(p.accent_hot),
                                    )
                                    .truncate(),
                                );
                                draw_working_dots(ui, p);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if danger_button(ui, p, "Stop").clicked() {
                                            state.is_generating = false;
                                            backend.abort_turn();
                                        }
                                        if let Some(ref el) = elapsed {
                                            ui.label(
                                                egui::RichText::new(el)
                                                    .small()
                                                    .monospace()
                                                    .color(p.muted),
                                            );
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(4.0);
                }

                if has_changes_bar {
                    let file_word = if changed_files == 1 { "file" } else { "files" };
                    Frame::new()
                        .fill(p.bg_elev)
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .corner_radius(CornerRadius::same(9))
                        .inner_margin(Margin::symmetric(12, 5))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{changed_files} {file_word} changed"
                                    ))
                                    .small()
                                    .color(p.text),
                                );
                                ui.label(
                                    egui::RichText::new(format!("+{added_lines}"))
                                        .small()
                                        .color(p.green),
                                );
                                ui.label(
                                    egui::RichText::new(format!("-{removed_lines}"))
                                        .small()
                                        .color(p.red),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if secondary_button(ui, p, "Review").clicked() {
                                            state.open_stage_tab(StageTab::Diffs);
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(4.0);
                }

                render_composer(ctx, ui, state, backend, &snap, &sess, p);
                ui.add_space(10.0);
            });
        });

    if clear_pending_question || clear_pending_approval || clear_pending_fallback {
        if let Some(ref mut snap_arc) = state.snap {
            let snap_mut = Arc::make_mut(snap_arc);
            if let Some(ref mut active_sess) = snap_mut.active_session {
                if active_sess.id == sess.id {
                    if clear_pending_question {
                        active_sess.pending_question = None;
                    }
                    if clear_pending_approval {
                        active_sess.pending_approval = None;
                    }
                    if clear_pending_fallback {
                        active_sess.pending_fallback = None;
                    }
                }
            }
        }
        ctx.request_repaint();
    }

    let stream_max_h = ui.available_height().max(120.0);

    let mut hero_clicked_prompt: Option<String> = None;
    if !has_messages && !state.is_busy() {
        // Calm minimal empty state
        egui::ScrollArea::vertical()
            .id_salt("home_hero_scroll")
            .max_height(stream_max_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space((stream_max_h * 0.14).max(20.0));
                with_centered_column(ui, panel_w, col_w, |ui| {
                    ui.vertical_centered(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(140.0, 140.0), egui::Sense::hover());
                        draw_cinder_orb_animated(ui, rect.center(), 40.0, p);
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(if is_standalone {
                                "Quick conversation"
                            } else {
                                "What are we building?"
                            })
                            .size(p.base_font_size + 7.5)
                            .strong()
                            .color(p.text),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(if is_standalone {
                                "One-off conversation · No project folder attached"
                            } else {
                                "Drop or Ctrl+V screenshots, type @file or /skill, or right-click anywhere"
                            })
                            .small()
                            .color(p.muted),
                        );
                        ui.add_space(20.0);
                    });

                    // 3 subtle hairline starter prompt chips, centered in balanced rows beneath the orb
                    let starters: &[&str] = if is_standalone {
                        &[
                            "Explain a concept step by step",
                            "Review and refactor a code snippet",
                            "Compare tradeoffs between approaches",
                        ]
                    } else {
                        &[
                            "Summarize workspace architecture",
                            "Find bugs or warnings in this project",
                            "Draft a step-by-step plan",
                        ]
                    };
                    let mut prepared = Vec::with_capacity(starters.len());
                    for &prompt in starters {
                        let rt = egui::RichText::new(prompt).small().color(p.muted);
                        let galley = egui::WidgetText::from(rt).into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Small,
                        );
                        let chip_size = egui::vec2(
                            (galley.size().x + 24.0).ceil(),
                            (galley.size().y + 14.0).ceil().max(26.0),
                        );
                        prepared.push((prompt, galley, chip_size));
                    }

                    let max_row_w = ui.available_width().max(120.0);
                    let chip_gap = 8.0_f32;
                    let mut rows: Vec<Vec<(usize, f32)>> = Vec::new();
                    let mut cur_row: Vec<(usize, f32)> = Vec::new();
                    let mut cur_w = 0.0_f32;
                    for (idx, (_, _, sz)) in prepared.iter().enumerate() {
                        let next_w = if cur_row.is_empty() {
                            sz.x
                        } else {
                            cur_w + chip_gap + sz.x
                        };
                        if !cur_row.is_empty() && next_w > max_row_w {
                            rows.push(cur_row);
                            cur_row = vec![(idx, sz.x)];
                            cur_w = sz.x;
                        } else {
                            cur_row.push((idx, sz.x));
                            cur_w = next_w;
                        }
                    }
                    if !cur_row.is_empty() {
                        rows.push(cur_row);
                    }

                    let mut chips_opt: Vec<Option<(&'static str, Arc<egui::Galley>, egui::Vec2)>> =
                        prepared.into_iter().map(Some).collect();

                    for (r_idx, row) in rows.iter().enumerate() {
                        if r_idx > 0 {
                            ui.add_space(6.0);
                        }
                        let row_w: f32 = row.iter().map(|(_, w)| *w).sum::<f32>()
                            + chip_gap * (row.len().saturating_sub(1) as f32);
                        let left_pad = ((max_row_w - row_w) * 0.5).max(0.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = chip_gap;
                            if left_pad > 0.5 {
                                ui.add_space(left_pad - chip_gap.min(left_pad));
                            }
                            for &(idx, _) in row {
                                let Some((prompt, galley, chip_size)) = chips_opt[idx].take()
                                else {
                                    continue;
                                };
                                let (rect, resp) =
                                    ui.allocate_exact_size(chip_size, egui::Sense::click());
                                if ui.is_rect_visible(rect) {
                                    let hovered = resp.hovered();
                                    let stroke = if hovered {
                                        Stroke::new(1.0_f32, p.border_strong)
                                    } else {
                                        Stroke::new(1.0_f32, p.border)
                                    };
                                    ui.painter().rect(
                                        rect,
                                        CornerRadius::same(8),
                                        if hovered { p.bg_hover } else { p.bg_elev },
                                        stroke,
                                        egui::StrokeKind::Inside,
                                    );
                                    let text_pos = egui::pos2(
                                        rect.left() + 12.0,
                                        rect.center().y - galley.size().y * 0.5,
                                    );
                                    ui.painter().galley(
                                        text_pos,
                                        galley,
                                        if hovered { p.text } else { p.muted },
                                    );
                                }
                                if resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if resp.clicked() {
                                    hero_clicked_prompt = Some(prompt.to_string());
                                }
                            }
                        });
                    }
                });
            });
    } else {
        // Editorial Scrollable Message Stream
        let busy = state.is_busy();
        let live_elapsed = state.format_elapsed_since_turn_start();
        let mut mark_plan_approved = false;
        egui::ScrollArea::vertical()
            .id_salt(("chat_stream_scroll", &sess.id))
            .max_height(stream_max_h)
            .auto_shrink([false, false])
            .stick_to_bottom(busy)
            .show(ui, |ui| {
                ui.add_space(14.0);
                with_centered_column(ui, panel_w, col_w, |ui| {
                    let msg_count = sess.messages.len();
                    for (msg_idx, msg) in sess.messages.iter().enumerate() {
                        ui.push_id(&msg.id, |ui| {
                            let plan_sig = msg.plan_artifact.as_ref().map(|pa| match pa.status {
                                PlanStatus::AwaitingReview => 0u8,
                                PlanStatus::Approved => 1u8,
                                PlanStatus::Executed => 2u8,
                                PlanStatus::ChangesRequested => 3u8,
                            });
                            let h_id = ui.make_persistent_id((
                                "chat_msg_h",
                                &msg.id,
                                col_w.round() as i32,
                                (p.base_font_size * 10.0).round() as i32,
                                msg.content.len(),
                                msg.tool_calls.len(),
                                plan_sig,
                            ));
                            let is_last_msg = msg_idx + 1 == msg_count;
                            if !is_last_msg {
                                if let Some(cached_h) = ui.data(|d| d.get_temp::<f32>(h_id)) {
                                    let cur_y = ui.cursor().top();
                                    let clip = ui.clip_rect();
                                    if cur_y + cached_h < clip.top() - 450.0
                                        || cur_y > clip.bottom() + 450.0
                                    {
                                        ui.allocate_space(egui::vec2(col_w, cached_h));
                                        ui.add_space(10.0);
                                        return;
                                    }
                                }
                            }

                            let is_user = msg.role == "user";
                            let turn_user_idx = if is_user {
                                Some(msg_idx)
                            } else {
                                sess.messages[..=msg_idx]
                                    .iter()
                                    .rposition(|m| m.role == "user")
                            };
                            let has_checkpoints_after = turn_user_idx
                                .map(|u_idx| {
                                    sess.messages[u_idx..]
                                        .iter()
                                        .any(|m| m.checkpoint_id.is_some())
                                })
                                .unwrap_or(false);

                            let msg_frame = if is_user {
                                Frame::new()
                                    .fill(p.bg_card)
                                    .stroke(Stroke::new(1.0_f32, p.border))
                                    .corner_radius(CornerRadius::same(12))
                                    .inner_margin(Margin::symmetric(14, 10))
                            } else {
                                Frame::new()
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE)
                                    .inner_margin(Margin::symmetric(4, 6))
                            };

                            let frame_resp = msg_frame.show(ui, |ui| {
                                ui.set_width(ui.available_width());

                                if !is_user {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        let (orb_rect, _) = ui.allocate_exact_size(
                                            egui::vec2(18.0, 18.0),
                                            egui::Sense::hover(),
                                        );
                                        let is_latest_busy =
                                            busy && msg_idx + 1 == sess.messages.len();
                                        if is_latest_busy {
                                            draw_cinder_orb_animated(
                                                ui,
                                                orb_rect.center(),
                                                7.5,
                                                p,
                                            );
                                        } else {
                                            draw_cinder_flame(
                                                ui.painter(),
                                                orb_rect.center(),
                                                7.5,
                                                p,
                                            );
                                        }
                                        ui.label(
                                            egui::RichText::new("Cinder")
                                                .small()
                                                .strong()
                                                .color(p.text),
                                        );
                                    });
                                    ui.add_space(3.0);
                                }

                                // Clean leaked <think> / </think> traces out of assistant body
                                let (clean_body, leaked_thought) = if is_user {
                                    (msg.content.clone(), None)
                                } else {
                                    harness_providers::extract_think_tags(&msg.content)
                                };
                                let combined_thinking =
                                    match (msg.thinking.as_deref(), leaked_thought.as_deref()) {
                                        (Some(a), Some(b))
                                            if !a.trim().is_empty() && !b.trim().is_empty() =>
                                        {
                                            Some(format!("{}\n\n{}", a.trim(), b.trim()))
                                        }
                                        (Some(a), _) if !a.trim().is_empty() => {
                                            Some(a.trim().to_string())
                                        }
                                        (_, Some(b)) if !b.trim().is_empty() => {
                                            Some(b.trim().to_string())
                                        }
                                        _ => None,
                                    };

                                // Activity & Thought Trace (hierarchical Antigravity trace view)
                                let has_thought = combined_thinking
                                    .as_ref()
                                    .map(|t| !t.trim().is_empty())
                                    .unwrap_or(false);
                                let has_tools = !msg.tool_calls.is_empty();
                                if has_thought || has_tools {
                                    if let Some(open_rel) = render_activity_and_thought_trace(
                                        ui,
                                        p,
                                        &msg.id,
                                        msg.latency_ms,
                                        false,
                                        0,
                                        0,
                                        combined_thinking.as_deref().unwrap_or(""),
                                        &msg.tool_calls,
                                    ) {
                                        backend.load_workspace_file(open_rel);
                                    }
                                    ui.add_space(6.0);
                                }

                                // Attached image thumbnails & file badges
                                if !msg.attached_files.is_empty() || !msg.attachments.is_empty() {
                                    ui.horizontal_wrapped(|ui| {
                                        for f in &msg.attached_files {
                                            badge(ui, f, p.text, p.bg_elev);
                                        }
                                        for att in &msg.attachments {
                                            if let Some(ref b64) = att.data_base64 {
                                                if let Some(bytes) = decode_base64_simple(b64) {
                                                    let cache_key = format!("att_{}", att.id);
                                                    if let Some(tex) = state.get_or_load_texture(
                                                        ctx,
                                                        &cache_key,
                                                        &bytes,
                                                    ) {
                                                        let thumb = egui::Image::new(&tex)
                                                            .max_size(egui::vec2(140.0, 96.0))
                                                            .corner_radius(CornerRadius::same(7))
                                                            .sense(egui::Sense::click());
                                                        if ui
                                                            .add(thumb)
                                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                            .on_hover_text("Click to zoom")
                                                            .clicked()
                                                        {
                                                            state.prompt_modal =
                                                                PromptModal::LightboxImage {
                                                                    title: att.name.clone(),
                                                                    cache_key,
                                                                };
                                                        }
                                                    }
                                                }
                                            } else {
                                                badge(ui, &att.name, p.text, p.bg_elev);
                                            }
                                        }
                                    });
                                }

                                // Orchestrator phases compact summary if present
                                if !msg.orchestrator_phases.is_empty() {
                                    let trace: Vec<String> = msg
                                        .orchestrator_phases
                                        .iter()
                                        .map(|ph| format!("{} ({})", ph.phase, ph.model_id))
                                        .collect();
                                    ui.label(
                                        egui::RichText::new(trace.join(" → "))
                                            .small()
                                            .color(p.muted),
                                    );
                                }

                                // Main Body
                                let display_body = if !is_user
                                    && clean_body.trim().is_empty()
                                    && !msg.tool_calls.is_empty()
                                {
                                    format!(
                                        "Completed {} step(s) ({}).",
                                        msg.tool_calls.len(),
                                        summarize_tool_activity_group(&msg.tool_calls)
                                    )
                                } else {
                                    clean_body
                                };
                                if !display_body.trim().is_empty() {
                                    if is_user {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&display_body).color(p.text),
                                            )
                                            .wrap()
                                            .selectable(true),
                                        );
                                    } else {
                                        render_markdown(
                                            ui,
                                            p,
                                            &display_body,
                                            snap.settings.ui.wrap_code_blocks,
                                            Some(&display_body),
                                        );
                                    }
                                }

                                // Compare partner response if present
                                if let Some(ref cp) = msg.compare_partner {
                                    ui.add_space(6.0);
                                    p.subcard_frame().show(ui, |ui| {
                                        let cp_label =
                                            resolve_model_display_name(&snap, &cp.model_id);
                                        ui.horizontal(|ui| {
                                            badge(
                                                ui,
                                                &format!("Compare: {cp_label}"),
                                                p.purple,
                                                p.bg_card,
                                            );
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{}ms · {} tok",
                                                    cp.latency_ms, cp.total_tokens
                                                ))
                                                .small()
                                                .color(p.muted),
                                            );
                                        });
                                        render_markdown(
                                            ui,
                                            p,
                                            &cp.content,
                                            snap.settings.ui.wrap_code_blocks,
                                            Some(&cp.content),
                                        );
                                    });
                                }

                                // Plan card actions if attached
                                if let Some(ref plan) = msg.plan_artifact {
                                    ui.add_space(6.0);
                                    p.subcard_frame().show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(&plan.title).strong());
                                            let pcol = match plan.status {
                                                PlanStatus::Approved | PlanStatus::Executed => {
                                                    p.green
                                                }
                                                PlanStatus::AwaitingReview => p.amber,
                                                PlanStatus::ChangesRequested => p.red,
                                            };
                                            badge(
                                                ui,
                                                &format!("{:?}", plan.status),
                                                pcol,
                                                p.bg_card,
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if secondary_button(
                                                        ui,
                                                        p,
                                                        "Open in Artifacts",
                                                    )
                                                    .clicked()
                                                    {
                                                        state.open_stage_tab(StageTab::Artifacts);
                                                    }
                                                },
                                            );
                                        });
                                        if plan.status == PlanStatus::AwaitingReview {
                                            ui.add_space(4.0);
                                            ui.horizontal(|ui| {
                                                if primary_button(
                                                    ui,
                                                    p,
                                                    "Approve & Execute Plan",
                                                )
                                                .clicked()
                                                {
                                                    state.is_generating = true;
                                                    mark_plan_approved = true;
                                                    backend.plan_action(
                                                        sess.id.clone(),
                                                        "execute".into(),
                                                        None,
                                                        None,
                                                    );
                                                }
                                                if secondary_button(ui, p, "Edit Plan").clicked() {
                                                    state.prompt_modal = PromptModal::EditPlan {
                                                        markdown: plan.markdown.clone(),
                                                    };
                                                }
                                                if secondary_button(ui, p, "Request Changes")
                                                    .clicked()
                                                {
                                                    state.prompt_modal =
                                                        PromptModal::RequestPlanChanges {
                                                            feedback: String::new(),
                                                        };
                                                }
                                            });
                                        }
                                    });
                                }

                                // Quiet editorial footer row
                                ui.add_space(4.0);
                                ui.horizontal_wrapped(|ui| {
                                    let copy_id =
                                        ui.make_persistent_id(("chat_msg_copy", &msg.id));
                                    let copied = is_copied_recently(ctx, copy_id);
                                    let copy_rt = if copied {
                                        egui::RichText::new("Copied ✓").small().color(p.green)
                                    } else {
                                        egui::RichText::new("Copy").small()
                                    };
                                    if ghost_button(ui, p, copy_rt)
                                        .on_hover_text("Copy message to clipboard")
                                        .clicked()
                                    {
                                        copy_to_clipboard(ctx, &msg.content);
                                        mark_copied_feedback(ctx, copy_id);
                                        state.set_toast("Copied message to clipboard.", false);
                                    }
                                    if !busy {
                                        if let Some(u_idx) = turn_user_idx {
                                            if ghost_button(
                                                ui,
                                                p,
                                                egui::RichText::new("Retry").small(),
                                            )
                                            .on_hover_text("Rewind to this prompt and regenerate")
                                            .clicked()
                                            {
                                                state.is_generating = true;
                                                backend.rewind_chat(
                                                    sess.id.clone(),
                                                    u_idx,
                                                    false,
                                                    true,
                                                );
                                            }
                                            if ghost_button(
                                                ui,
                                                p,
                                                egui::RichText::new("Rewind").small(),
                                            )
                                            .on_hover_text(
                                                "Rewind conversation here and restore prompt to composer",
                                            )
                                            .clicked()
                                            {
                                                backend.rewind_chat(
                                                    sess.id.clone(),
                                                    u_idx,
                                                    false,
                                                    false,
                                                );
                                            }
                                            if has_checkpoints_after
                                                && ghost_button(
                                                    ui,
                                                    p,
                                                    egui::RichText::new("Revert Files")
                                                        .small()
                                                        .color(p.amber),
                                                )
                                                .on_hover_text(
                                                    "Rewind conversation here and restore modified workspace files",
                                                )
                                                .clicked()
                                            {
                                                backend.rewind_chat(
                                                    sess.id.clone(),
                                                    u_idx,
                                                    true,
                                                    false,
                                                );
                                            }
                                        }
                                    }
                                    if !is_user {
                                        if let Some(ref mid) = msg.model_id {
                                            let mlabel = resolve_model_display_name(&snap, mid);
                                            ui.label(
                                                egui::RichText::new(mlabel)
                                                    .small()
                                                    .color(p.muted),
                                            )
                                            .on_hover_text(mid.as_str());
                                        }
                                        if let Some(ref orig_mid) = msg.fallback_from_model_id {
                                            let orig_lbl =
                                                resolve_model_display_name(&snap, orig_mid);
                                            let actual_lbl = msg
                                                .model_id
                                                .as_deref()
                                                .map(|m| resolve_model_display_name(&snap, m))
                                                .unwrap_or_else(|| "fallback".into());
                                            ui.scope(|ui| {
                                                badge(
                                                    ui,
                                                    &format!("Fallback: {orig_lbl} → {actual_lbl}"),
                                                    p.amber,
                                                    p.bg_card,
                                                );
                                            })
                                            .response
                                            .on_hover_text(
                                                msg.fallback_reason
                                                    .as_deref()
                                                    .unwrap_or("Turn-scoped model fallback"),
                                            );
                                            if !busy {
                                                if let Some(u_idx) = turn_user_idx {
                                                    if ghost_button(
                                                        ui,
                                                        p,
                                                        egui::RichText::new(format!(
                                                            "Retry with {orig_lbl}"
                                                        ))
                                                        .small()
                                                        .color(p.amber),
                                                    )
                                                    .on_hover_text(format!(
                                                        "Rewind this turn and retry on {orig_mid}"
                                                    ))
                                                    .clicked()
                                                    {
                                                        state.is_generating = true;
                                                        state.turn_started_at =
                                                            Some(std::time::Instant::now());
                                                        backend.retry_turn_with_model(
                                                            sess.id.clone(),
                                                            u_idx,
                                                            orig_mid.clone(),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        if snap.settings.ui.show_token_metrics {
                                            if let Some(ms) = msg.latency_ms {
                                                ui.label(
                                                    egui::RichText::new(format!("· {ms}ms"))
                                                        .small()
                                                        .color(p.muted),
                                                );
                                            }
                                            if let (Some(pt), Some(ct)) =
                                                (msg.prompt_tokens, msg.completion_tokens)
                                            {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "· {} tok",
                                                        pt + ct
                                                    ))
                                                    .small()
                                                    .color(p.muted),
                                                );
                                            }
                                        }
                                    }
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "· {}",
                                            fmt_rel_age(msg.created_at)
                                        ))
                                        .small()
                                        .color(p.muted),
                                    );
                                });
                            });
                            let measured_h = frame_resp.response.rect.height();
                            if measured_h > 0.0 {
                                ui.data_mut(|d| d.insert_temp(h_id, measured_h));
                            }

                            ui.add_space(10.0);
                        });
                    }

                    // Live streaming turn preview (tokens + thought trace + completed tool steps)
                    let live_for_this_sess = state.is_busy()
                        && snap
                            .live_activity
                            .session_id
                            .as_deref()
                            .unwrap_or(&sess.id)
                            == sess.id;
                    if live_for_this_sess {
                        let act = &snap.live_activity;
                        Frame::new()
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .inner_margin(Margin::symmetric(4, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let (orb_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(18.0, 18.0),
                                        egui::Sense::hover(),
                                    );
                                    draw_cinder_orb_animated(ui, orb_rect.center(), 7.5, p);
                                    ui.label(
                                        egui::RichText::new("Cinder")
                                            .small()
                                            .strong()
                                            .color(p.text),
                                    );
                                    let raw_mid = if act.active_model.is_empty() {
                                        &snap.settings.active_model_id
                                    } else {
                                        &act.active_model
                                    };
                                    let live_model_label =
                                        resolve_model_display_name(&snap, raw_mid);
                                    if !live_model_label.is_empty() {
                                        badge(ui, &live_model_label, p.muted, p.bg_card);
                                    }
                                    if act.step > 0 {
                                        let turn_meta = if act.completed_tools.is_empty() {
                                            format!("Turn {}", act.step)
                                        } else {
                                            format!(
                                                "Turn {} · {} steps",
                                                act.step,
                                                act.completed_tools.len()
                                            )
                                        };
                                        ui.label(
                                            egui::RichText::new(turn_meta)
                                                .small()
                                                .color(p.muted),
                                        );
                                    }
                                    if let Some(ref el) = live_elapsed {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    egui::RichText::new(el)
                                                        .small()
                                                        .monospace()
                                                        .color(p.muted),
                                                );
                                            },
                                        );
                                    }
                                });
                                ui.add_space(4.0);

                                let (live_clean_content, live_leaked_th) =
                                    harness_providers::extract_think_tags_streaming(&act.partial_content);
                                let live_combined_th = match (
                                    act.partial_thinking.trim().is_empty(),
                                    live_leaked_th.as_deref(),
                                ) {
                                    (false, Some(b)) if !b.trim().is_empty() => {
                                        format!("{}\n\n{}", act.partial_thinking.trim(), b.trim())
                                    }
                                    (false, _) => act.partial_thinking.clone(),
                                    (true, Some(b)) => b.trim().to_string(),
                                    (true, None) => String::new(),
                                };
                                let has_th = !live_combined_th.trim().is_empty();
                                let has_tools = !act.completed_tools.is_empty();
                                let has_content = !live_clean_content.trim().is_empty();

                                if !has_content {
                                    let stage_caption = act
                                        .detail
                                        .rsplit_once('—')
                                        .or_else(|| act.detail.rsplit_once('·'))
                                        .map(|(_, tail)| tail.trim().trim_end_matches('.'))
                                        .filter(|s| !s.is_empty())
                                        .unwrap_or(match act.phase.as_str() {
                                            "orchestrator_planner" => {
                                                "Inspecting workspace & drafting plan"
                                            }
                                            "orchestrator_executor" => {
                                                "Executing approved steps in workspace"
                                            }
                                            "summarizing" => "Synthesizing response",
                                            _ => "Processing context & generating",
                                        });
                                    Frame::new()
                                        .fill(p.bg_card)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .corner_radius(CornerRadius::same(10))
                                        .inner_margin(Margin::symmetric(12, 9))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal_wrapped(|ui| {
                                                ui.spacing_mut().item_spacing.x = 6.0;
                                                if act.phase.starts_with("orchestrator_") {
                                                    let in_exec =
                                                        act.phase == "orchestrator_executor";
                                                    badge(
                                                        ui,
                                                        if in_exec {
                                                            "✓ 1. Planner"
                                                        } else {
                                                            "● 1. Planner"
                                                        },
                                                        if in_exec { p.green } else { p.accent_hot },
                                                        p.bg_elev,
                                                    );
                                                    ui.label(
                                                        egui::RichText::new("→")
                                                            .small()
                                                            .color(p.muted),
                                                    );
                                                    badge(
                                                        ui,
                                                        if in_exec {
                                                            "● 2. Executor"
                                                        } else {
                                                            "○ 2. Executor"
                                                        },
                                                        if in_exec { p.accent_hot } else { p.muted },
                                                        p.bg_elev,
                                                    );
                                                } else {
                                                    let phase_tag = match act.phase.as_str() {
                                                        "summarizing" => "SUMMARIZING",
                                                        "thinking" => "WORKING",
                                                        "" => "ACTIVE",
                                                        other => other,
                                                    };
                                                    badge(
                                                        ui,
                                                        phase_tag,
                                                        p.accent_hot,
                                                        p.bg_elev,
                                                    );
                                                }
                                                ui.label(
                                                    egui::RichText::new(stage_caption)
                                                        .small()
                                                        .color(p.text),
                                                );
                                                draw_working_dots(ui, p);
                                            });
                                            ui.add_space(6.0);
                                            paint_live_shimmer_bar(ui, p);
                                        });
                                    ui.add_space(6.0);
                                }

                                if has_th || has_tools {
                                    let live_scope = format!("live_{}", sess.id);
                                    if let Some(open_rel) = render_activity_and_thought_trace(
                                        ui,
                                        p,
                                        &live_scope,
                                        None,
                                        true,
                                        act.step,
                                        act.max_steps,
                                        &live_combined_th,
                                        &act.completed_tools,
                                    ) {
                                        backend.load_workspace_file(open_rel);
                                    }
                                    ui.add_space(6.0);
                                }

                                if !act.live_tool_name.trim().is_empty()
                                    || !act.live_tool_output.trim().is_empty()
                                {
                                    Frame::new()
                                        .fill(p.bg_sidebar)
                                        .stroke(Stroke::new(1.0_f32, p.border_strong))
                                        .corner_radius(CornerRadius::same(8))
                                        .inner_margin(Margin::symmetric(10, 7))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal(|ui| {
                                                badge(ui, "LIVE SHELL", p.accent_hot, p.bg_elev);
                                                if !act.live_tool_name.trim().is_empty() {
                                                    ui.add(
                                                        egui::Label::new(
                                                            egui::RichText::new(&act.live_tool_name)
                                                                .small()
                                                                .monospace()
                                                                .color(p.text),
                                                        )
                                                        .truncate(),
                                                    );
                                                }
                                            });
                                            if !act.live_tool_output.trim().is_empty() {
                                                ui.add_space(4.0);
                                                egui::ScrollArea::vertical()
                                                    .id_salt(("live_shell_scroll", &sess.id))
                                                    .max_height(150.0)
                                                    .stick_to_bottom(true)
                                                    .show(ui, |ui| {
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(
                                                                    &act.live_tool_output,
                                                                )
                                                                .size(
                                                                    (p.base_font_size - 1.5)
                                                                        .max(10.5),
                                                                )
                                                                .monospace()
                                                                .color(p.muted),
                                                            )
                                                            .wrap()
                                                            .selectable(true),
                                                        );
                                                    });
                                            }
                                        });
                                    ui.add_space(6.0);
                                }

                                if has_content {
                                    ui.push_id(("live_md", &sess.id), |ui| {
                                        render_markdown(
                                            ui,
                                            p,
                                            &live_clean_content,
                                            snap.settings.ui.wrap_code_blocks,
                                            None,
                                        );
                                    });
                                }
                            });
                        ui.add_space(10.0);
                    }
                });
            });
        if mark_plan_approved {
            state.approve_active_plan_optimistically(&sess.id);
            ctx.request_repaint();
        }
    }

    if let Some(prompt) = hero_clicked_prompt {
        let files = std::mem::take(&mut state.attached_files);
        let media = std::mem::take(&mut state.attached_media);
        state.prompt_text.clear();
        dispatch_chat_prompt(
            ctx,
            state,
            backend,
            &sess.id,
            sess.mode,
            &snap.settings.active_model_id,
            &snap.settings.compare_model_b_id,
            prompt,
            files,
            media,
        );
    }
}

fn icon_text_toggle(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    icon: Icon,
    active: bool,
    label: &str,
    tooltip: &str,
) -> egui::Response {
    let galley = egui::WidgetText::from(egui::RichText::new(label).small()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        120.0,
        egui::TextStyle::Button,
    );
    let w = 24.0 + galley.size().x + 8.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if active {
            p.bg_card
        } else if resp.hovered() {
            p.bg_hover
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), fill);
        }
        let fg = if active {
            p.accent_hot
        } else if resp.hovered() {
            p.text
        } else {
            p.muted
        };
        draw_icon(
            ui.painter(),
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 13.0, rect.center().y),
                egui::vec2(13.5, 13.5),
            ),
            icon,
            fg,
        );
        ui.painter().galley(
            egui::pos2(
                rect.left() + 24.0,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            fg,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(tooltip)
}

fn with_centered_column<R>(
    ui: &mut egui::Ui,
    panel_w: f32,
    col_w: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let pad = ((panel_w - col_w) * 0.5).max(16.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(
            egui::vec2(col_w, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            add_contents,
        )
        .inner
    })
    .inner
}

fn summarize_checkpoints(sess: &harness_store::Session) -> (usize, usize, usize) {
    let mut paths = BTreeSet::new();
    let mut added = 0usize;
    let mut removed = 0usize;
    for cp in &sess.checkpoints {
        for f in &cp.files {
            paths.insert(f.rel_path.clone());
            for line in f.diff.lines() {
                if line.starts_with('+') && !line.starts_with("+++") {
                    added += 1;
                } else if line.starts_with('-') && !line.starts_with("---") {
                    removed += 1;
                }
            }
        }
    }
    (paths.len(), added, removed)
}

fn composer_return_key_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)
}

#[allow(clippy::too_many_arguments)]
fn render_composer(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    sess: &harness_store::Session,
    p: &CinderPalette,
) {
    let focused = state.composer_focused;
    let composer_stroke = if focused {
        Stroke::new(
            1.2_f32,
            Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 155),
        )
    } else {
        Stroke::new(1.0_f32, p.border_strong)
    };

    // Subtle ember glow halo painted behind composer when focused
    let glow_idx = ui.painter().add(egui::Shape::Noop);
    let mut mode_model_trigger_rect: Option<egui::Rect> = None;
    let models = state.selectable_models(true);
    let active_id = if sess.mode == AgentMode::Orchestrator {
        snap.settings.orchestrator.planner_model_id.clone()
    } else {
        snap.settings.active_model_id.clone()
    };

    let inner_res = Frame::new()
        .fill(p.bg_elev)
        .stroke(composer_stroke)
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            // Attached files / media chips
            if !state.attached_files.is_empty() || !state.attached_media.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    let mut remove_file_idx = None;
                    for (idx, f) in state.attached_files.iter().enumerate() {
                        if chip_remove_button(ui, p, Icon::File, f).clicked() {
                            remove_file_idx = Some(idx);
                        }
                    }
                    if let Some(idx) = remove_file_idx {
                        state.attached_files.remove(idx);
                    }

                    let mut remove_media_idx = None;
                    for idx in 0..state.attached_media.len() {
                        let (att_id, att_name, att_mime, att_b64, att_size) = {
                            let m = &state.attached_media[idx];
                            (
                                m.id.clone(),
                                m.name.clone(),
                                m.mime_type.clone(),
                                m.data_base64.clone(),
                                m.size_bytes,
                            )
                        };
                        let lbl = format!("{} ({})", att_name, fmt_bytes(att_size));
                        let tex = if att_mime.starts_with("image/") {
                            att_b64
                                .as_deref()
                                .and_then(decode_base64_simple)
                                .and_then(|bytes| {
                                    state.get_or_load_texture(ctx, &format!("att:{att_id}"), &bytes)
                                })
                        } else {
                            None
                        };
                        if let Some(tex) = tex {
                            if image_chip_remove_button(ui, p, &tex, &lbl).clicked() {
                                remove_media_idx = Some(idx);
                            }
                        } else if chip_remove_button(ui, p, Icon::FileImage, &lbl).clicked() {
                            remove_media_idx = Some(idx);
                        }
                    }
                    if let Some(idx) = remove_media_idx {
                        state.attached_media.remove(idx);
                    }
                });
                ui.add_space(4.0);
            }

            // Autocomplete: `/` Slash Skills and `@` Workspace Files (compact vertical popup, Up/Down + Tab/click)
            let trimmed_prompt = state.prompt_text.trim_start();
            if trimmed_prompt.starts_with('/') && !trimmed_prompt.contains(' ') {
                let needle = trimmed_prompt[1..].to_ascii_lowercase();
                let matches: Vec<_> = snap
                    .skills
                    .iter()
                    .filter(|s| s.slug.to_ascii_lowercase().contains(&needle))
                    .take(8)
                    .collect();
                if !matches.is_empty() {
                    if state.autocomplete_idx >= matches.len() {
                        state.autocomplete_idx = 0;
                    }
                    let (up_p, dn_p, tab_p) = if state.composer_focused {
                        ctx.input_mut(|i| {
                            (
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                            )
                        })
                    } else {
                        (false, false, false)
                    };
                    if dn_p {
                        state.autocomplete_idx = (state.autocomplete_idx + 1) % matches.len();
                    }
                    if up_p {
                        state.autocomplete_idx = if state.autocomplete_idx == 0 {
                            matches.len() - 1
                        } else {
                            state.autocomplete_idx - 1
                        };
                    }
                    let mut chosen_slug: Option<String> = None;
                    if tab_p {
                        chosen_slug = Some(matches[state.autocomplete_idx].slug.clone());
                    }
                    Frame::new()
                        .fill(p.bg_card)
                        .stroke(Stroke::new(1.0_f32, p.border_strong))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::ScrollArea::vertical()
                                .id_salt("slash_skill_popup_scroll")
                                .max_height(152.0)
                                .show(ui, |ui| {
                                    for (idx, sk) in matches.iter().enumerate() {
                                        let is_sel = idx == state.autocomplete_idx;
                                        let subtitle = if sk.description.trim().is_empty() {
                                            sk.name.as_str()
                                        } else {
                                            sk.description.as_str()
                                        };
                                        let resp = autocomplete_row(
                                            ui,
                                            p,
                                            is_sel,
                                            &format!("/{}", sk.slug),
                                            subtitle,
                                        );
                                        if resp.hovered() {
                                            state.autocomplete_idx = idx;
                                        }
                                        if resp.clicked() {
                                            chosen_slug = Some(sk.slug.clone());
                                        }
                                    }
                                });
                        });
                    ui.add_space(6.0);
                    if let Some(slug) = chosen_slug {
                        state.prompt_text = format!("/{slug} ");
                        state.autocomplete_idx = 0;
                    }
                }
            } else if let Some(at_pos) = state.prompt_text.rfind('@') {
                let after = &state.prompt_text[at_pos + 1..];
                if !after.contains(' ') && !after.contains('\n') {
                    let needle = after.to_ascii_lowercase();
                    let matches: Vec<_> = snap
                        .workspace_tree
                        .iter()
                        .filter(|n| !n.is_dir && n.rel_path.to_ascii_lowercase().contains(&needle))
                        .take(8)
                        .collect();
                    if !matches.is_empty() {
                        if state.autocomplete_idx >= matches.len() {
                            state.autocomplete_idx = 0;
                        }
                        let (up_p, dn_p, tab_p) = if state.composer_focused {
                            ctx.input_mut(|i| {
                                (
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                                )
                            })
                        } else {
                            (false, false, false)
                        };
                        if dn_p {
                            state.autocomplete_idx = (state.autocomplete_idx + 1) % matches.len();
                        }
                        if up_p {
                            state.autocomplete_idx = if state.autocomplete_idx == 0 {
                                matches.len() - 1
                            } else {
                                state.autocomplete_idx - 1
                            };
                        }
                        let mut chosen_path: Option<String> = None;
                        if tab_p {
                            chosen_path = Some(matches[state.autocomplete_idx].rel_path.clone());
                        }
                        Frame::new()
                            .fill(p.bg_card)
                            .stroke(Stroke::new(1.0_f32, p.border_strong))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(6, 4))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                egui::ScrollArea::vertical()
                                    .id_salt("at_file_popup_scroll")
                                    .max_height(152.0)
                                    .show(ui, |ui| {
                                        for (idx, n) in matches.iter().enumerate() {
                                            let is_sel = idx == state.autocomplete_idx;
                                            let resp = autocomplete_row(
                                                ui,
                                                p,
                                                is_sel,
                                                &format!("@{}", n.rel_path),
                                                &fmt_bytes(n.size_bytes),
                                            );
                                            if resp.hovered() {
                                                state.autocomplete_idx = idx;
                                            }
                                            if resp.clicked() {
                                                chosen_path = Some(n.rel_path.clone());
                                            }
                                        }
                                    });
                            });
                        ui.add_space(6.0);
                        if let Some(rel) = chosen_path {
                            state.prompt_text.truncate(at_pos);
                            state.prompt_text.push('@');
                            state.prompt_text.push_str(&rel);
                            state.prompt_text.push(' ');
                            if !state.attached_files.iter().any(|f| f == &rel) {
                                state.attached_files.push(rel);
                            }
                            state.autocomplete_idx = 0;
                        }
                    }
                }
            }

            // Borderless multiline prompt input:
            // - Bare Enter sends (or is suppressed when empty)
            // - Shift+Enter inserts a newline cleanly at the cursor
            let can_send_now = !state.prompt_text.trim().is_empty()
                || !state.attached_files.is_empty()
                || !state.attached_media.is_empty();
            let bare_enter = state.composer_focused
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let send_pressed = bare_enter && can_send_now;
            let text_resp = egui::ScrollArea::vertical()
                .id_salt("composer_input_scroll")
                .max_height(180.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut state.prompt_text)
                            .hint_text("Ask Cinder anything... (Shift+Enter for new line)")
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .return_key(composer_return_key_shortcut())
                            .frame(false),
                    )
                })
                .inner;
            state.composer_focused = text_resp.has_focus();
            let want_send = send_pressed && can_send_now;

            text_resp.context_menu(|ui| {
                if ui.button("Copy Prompt").clicked() {
                    copy_to_clipboard(ui.ctx(), &state.prompt_text);
                    ui.close_menu();
                }
                if ui.button("Paste from Clipboard").clicked() {
                    let pasted_txt = arboard::Clipboard::new()
                        .and_then(|mut c| c.get_text())
                        .ok()
                        .filter(|t| !t.is_empty());
                    if let Some(txt) = pasted_txt {
                        state.prompt_text.push_str(&txt);
                    } else if !state.attach_clipboard_image() {
                        state.set_toast("Clipboard does not contain text or an image.", true);
                    }
                    ui.close_menu();
                }
                if ui.button("Paste Screenshot / File (Ctrl+V)").clicked() {
                    if !state.attach_clipboard_image() {
                        state.set_toast("No image or file found on clipboard.", true);
                    }
                    ui.close_menu();
                }
                if ui.button("Attach Files or Pictures...").clicked() {
                    backend.open_file_picker();
                    ui.close_menu();
                }
                if ui.button("Clear Prompt").clicked() {
                    state.prompt_text.clear();
                    ui.close_menu();
                }
            });

            ui.add_space(6.0);

            // Single clean frameless controls row
            ui.horizontal(|ui| {
                ui.scope(|ui| {
                    let vis = &mut ui.style_mut().visuals;
                    vis.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    vis.widgets.inactive.bg_fill = Color32::TRANSPARENT;
                    vis.widgets.inactive.bg_stroke = Stroke::NONE;
                    vis.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.muted);
                    vis.widgets.hovered.weak_bg_fill = p.bg_hover;
                    vis.widgets.hovered.bg_fill = p.bg_hover;
                    vis.widgets.hovered.bg_stroke = Stroke::NONE;
                    vis.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
                    vis.widgets.open.bg_fill = p.bg_card;
                    vis.widgets.open.bg_stroke = Stroke::NONE;

                    // `+` Attach Files / Pictures menu
                    ui.menu_button("+", |ui| {
                        if ui.button("Paste Screenshot / File (Ctrl+V)").clicked() {
                            if !state.attach_clipboard_image() {
                                state.set_toast("No image or file found on clipboard.", true);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Upload Picture / File from PC...").clicked() {
                            backend.open_file_picker();
                            ui.close_menu();
                        }
                        ui.separator();
                        input_frame(p, false).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut state.attach_ws_filter)
                                    .hint_text("Filter workspace files...")
                                    .desired_width(220.0)
                                    .frame(false),
                            );
                        });
                        let q = state.attach_ws_filter.to_ascii_lowercase();
                        egui::ScrollArea::vertical()
                            .id_salt("attach_ws_menu_scroll")
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for n in snap
                                    .workspace_tree
                                    .iter()
                                    .filter(|x| {
                                        !x.is_dir
                                            && (q.is_empty()
                                                || x.rel_path.to_ascii_lowercase().contains(&q))
                                    })
                                    .take(40)
                                {
                                    if ui.button(&n.rel_path).clicked() {
                                        if !state.attached_files.iter().any(|f| f == &n.rel_path) {
                                            state.attached_files.push(n.rel_path.clone());
                                        }
                                        ui.close_menu();
                                    }
                                }
                            });
                    });

                    // Unified frameless Mode & Model Popover Trigger with inline Quota indicator
                    let active_entry = models.iter().find(|m| m.id == active_id);
                    let active_label = if active_id.trim().is_empty()
                        || active_id.eq_ignore_ascii_case("auto")
                    {
                        "Auto".to_string()
                    } else {
                        active_entry
                            .map(|m| m.display_name.clone())
                            .unwrap_or_else(|| resolve_model_display_name(snap, &active_id))
                    };
                    let active_quota_badge = active_entry.map(|m| m.quota_badge.as_str());
                    let active_quota_col = active_entry
                        .map(|m| quota_pill_color(p, m))
                        .unwrap_or(p.muted);

                    let trig_resp = unified_model_trigger_pill(
                        ui,
                        p,
                        mode_label(sess.mode),
                        &active_label,
                        active_quota_badge,
                        active_quota_col,
                        state.show_model_popover,
                    );
                    mode_model_trigger_rect = Some(trig_resp.rect);
                    let trig_tip = active_entry
                        .map(|m| format!("{} ({})\n{}", m.canonical_name, m.id, m.quota_tooltip))
                        .unwrap_or_else(|| "Select reasoning mode & active model".to_string());
                    if trig_resp.on_hover_text(trig_tip).clicked() {
                        let next = !state.show_model_popover;
                        state.close_transient_popovers();
                        state.show_model_popover = next;
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_gen = state.is_busy();
                    let can_send = !state.prompt_text.trim().is_empty()
                        || !state.attached_files.is_empty()
                        || !state.attached_media.is_empty();

                    let send_resp = send_glow_button(ui, p, is_gen, can_send);
                    if is_gen {
                        if send_resp.on_hover_text("Stop Generation").clicked() {
                            state.is_generating = false;
                            backend.abort_turn();
                        }
                    } else if (send_resp.clicked() || want_send) && can_send {
                        let txt = state.prompt_text.trim().to_string();
                        let files = std::mem::take(&mut state.attached_files);
                        let media = std::mem::take(&mut state.attached_media);
                        state.prompt_text.clear();
                        dispatch_chat_prompt(
                            ctx,
                            state,
                            backend,
                            &sess.id,
                            sess.mode,
                            &snap.settings.active_model_id,
                            &snap.settings.compare_model_b_id,
                            txt,
                            files,
                            media,
                        );
                    }

                    let rev_lbl =
                        if snap.settings.auto_approve_edits && snap.settings.auto_approve_commands {
                            "Review: Auto"
                        } else {
                            "Review: Ask"
                        };
                    if ghost_button(ui, p, egui::RichText::new(rev_lbl).small())
                        .on_hover_text("Toggle manual approval vs auto-approve")
                        .clicked()
                    {
                        let next_auto = !(snap.settings.auto_approve_edits
                            && snap.settings.auto_approve_commands);
                        backend.update_settings(move |s| {
                            s.auto_approve_edits = next_auto;
                            s.auto_approve_commands = next_auto;
                        });
                    }
                });
            });
        });

    if focused {
        let frect = inner_res.response.rect;
        ui.painter().set(
            glow_idx,
            egui::Shape::rect_stroke(
                frect.expand(2.5),
                CornerRadius::same(18),
                Stroke::new(
                    4.0_f32,
                    Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 20),
                ),
                egui::StrokeKind::Outside,
            ),
        );
    }

    // Floating Mode & Model Popover anchored cleanly above & to the left of the composer
    if state.show_model_popover {
        if let Some(trig_rect) = mode_model_trigger_rect {
            let screen = ctx.screen_rect();
            let composer_rect = inner_res.response.rect;
            let pop_w = 460.0_f32.min(screen.width() - 28.0);
            let left_x = (trig_rect.right() - pop_w)
                .clamp(screen.left() + 14.0, (screen.right() - pop_w - 14.0).max(screen.left() + 14.0));
            let bottom_y = (composer_rect.top() - 12.0)
                .min(screen.bottom() - 36.0)
                .max(screen.top() + 260.0);

            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.show_model_popover = false;
            }

            let area_resp = egui::Area::new(egui::Id::new("composer_mode_model_popover"))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::LEFT_BOTTOM)
                .fixed_pos(egui::pos2(left_x, bottom_y))
                .show(ctx, |ui| {
                    Frame::new()
                        .fill(p.bg_elev)
                        .stroke(Stroke::new(1.0_f32, p.border_strong))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::same(14))
                        .show(ui, |ui| {
                            ui.set_width(pop_w - 28.0);

                            // Title + Close
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Mode & Model")
                                        .strong()
                                        .color(p.text),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icon_button(ui, p, Icon::Close, false, "Close").clicked() {
                                            state.show_model_popover = false;
                                        }
                                    },
                                );
                            });
                            ui.add_space(6.0);

                            // Aligned 3-row controls grid
                            let label_w = 78.0;
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(label_w, 24.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_size(egui::vec2(label_w, 24.0));
                                        ui.label(egui::RichText::new("Mode").small().color(p.muted));
                                    },
                                );
                                if let Some(next_mode) = segmented_control(
                                    ui,
                                    p,
                                    "composer_mode_seg",
                                    sess.mode,
                                    &[
                                        (AgentMode::Auto, "Auto"),
                                        (AgentMode::Chat, "Chat"),
                                        (AgentMode::Plan, "Plan"),
                                        (AgentMode::Agent, "Agent"),
                                        (AgentMode::Orchestrator, "Orch"),
                                        (AgentMode::Compare, "Compare"),
                                    ],
                                ) {
                                    backend.set_session_mode(sess.id.clone(), next_mode);
                                }
                            });

                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(label_w, 24.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_size(egui::vec2(label_w, 24.0));
                                        ui.label(
                                            egui::RichText::new("Reasoning").small().color(p.muted),
                                        );
                                    },
                                );
                                let cur_eff = match snap.settings.ui.thinking_effort.as_str() {
                                    "standard" => "standard",
                                    "deep" => "deep",
                                    _ => "off",
                                };
                                if let Some(next_eff) = segmented_control(
                                    ui,
                                    p,
                                    "composer_effort_seg",
                                    cur_eff,
                                    &[("off", "Off"), ("standard", "Standard"), ("deep", "Deep")],
                                ) {
                                    let mut next_ui = snap.settings.ui.clone();
                                    next_ui.thinking_effort = next_eff.into();
                                    backend.save_ui_and_orch(Some(next_ui), None);
                                }
                            });

                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(label_w, 24.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_size(egui::vec2(label_w, 24.0));
                                        ui.label(
                                            egui::RichText::new("Auto Steps")
                                                .small()
                                                .color(p.muted),
                                        );
                                    },
                                );
                                let mut max_steps =
                                    snap.settings.orchestrator.max_autonomous_steps.clamp(2, 32);
                                if ui
                                    .add(egui::Slider::new(&mut max_steps, 2..=32).text("steps"))
                                    .on_hover_text(
                                        "Autonomous tool steps allowed per turn before prompting to continue",
                                    )
                                    .changed()
                                {
                                    let mut orch = snap.settings.orchestrator.clone();
                                    orch.max_autonomous_steps = max_steps;
                                    backend.save_ui_and_orch(None, Some(orch));
                                }
                            });

                            ui.add_space(6.0);
                            let div_y = ui.cursor().top();
                            ui.painter().hline(
                                ui.max_rect().x_range(),
                                div_y,
                                Stroke::new(1.0_f32, p.border),
                            );
                            ui.add_space(8.0);

                            // Models header + contextual Aliases link + inline category filter
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Models")
                                        .small()
                                        .strong()
                                        .color(p.text),
                                );
                                if ghost_button(
                                    ui,
                                    p,
                                    egui::RichText::new("Aliases →").small().color(p.accent_hot),
                                )
                                .on_hover_text("Assign custom model aliases & API keys in Settings")
                                .clicked()
                                {
                                    state.open_settings(SettingsTab::Models);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let cur_cat = match state.model_cat.as_str() {
                                            "local" => "local",
                                            "google" => "google",
                                            "cloud" => "cloud",
                                            _ => "all",
                                        };
                                        if let Some(next_cat) = segmented_control(
                                            ui,
                                            p,
                                            "composer_model_cat",
                                            cur_cat,
                                            &[
                                                ("all", "All"),
                                                ("local", "Local"),
                                                ("google", "Gemini"),
                                                ("cloud", "Other"),
                                            ],
                                        ) {
                                            state.model_cat = next_cat.into();
                                        }
                                    },
                                );
                            });
                            ui.add_space(6.0);

                            input_frame(p, false).show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut state.model_search)
                                        .hint_text("Search active models...")
                                        .desired_width(f32::INFINITY)
                                        .frame(false),
                                );
                            });
                            ui.add_space(6.0);

                            let q = state.model_search.trim().to_ascii_lowercase();
                            let list_max_h = (bottom_y - screen.top() - 250.0).clamp(150.0, 260.0);
                            let mut any_shown = false;
                            egui::ScrollArea::vertical()
                                .id_salt("unified_model_menu_scroll")
                                .max_height(list_max_h)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 3.0;
                                    for m in &models {
                                        if state.model_cat != "all" && m.category != state.model_cat
                                        {
                                            continue;
                                        }
                                        if !q.is_empty()
                                            && !m.display_name.to_ascii_lowercase().contains(&q)
                                            && !m.canonical_name.to_ascii_lowercase().contains(&q)
                                            && !m.id.to_ascii_lowercase().contains(&q)
                                        {
                                            continue;
                                        }
                                        any_shown = true;
                                        if model_picker_row(ui, p, m, m.id == active_id).clicked()
                                        {
                                            let chosen = m.id.clone();
                                            if sess.mode == AgentMode::Orchestrator {
                                                let mut o = snap.settings.orchestrator.clone();
                                                o.planner_model_id = chosen;
                                                backend.save_ui_and_orch(None, Some(o));
                                            } else {
                                                backend.update_settings(move |s| {
                                                    s.active_model_id = chosen;
                                                });
                                            }
                                            state.show_model_popover = false;
                                        }
                                    }
                                    if !any_shown {
                                        ui.add_space(8.0);
                                        ui.label(
                                            egui::RichText::new("No matching active models.")
                                                .small()
                                                .color(p.muted),
                                        );
                                    }
                                });
                        });
                });

            let clicked_outside = ctx.input(|i| {
                i.pointer.primary_pressed()
                    && i.pointer
                        .interact_pos()
                        .map(|pos| !area_resp.response.rect.contains(pos) && !trig_rect.contains(pos))
                        .unwrap_or(false)
            });
            if clicked_outside {
                state.show_model_popover = false;
            }
        } else {
            state.show_model_popover = false;
        }
    }
}

fn quota_pill_color(p: &CinderPalette, m: &SelectableModel) -> Color32 {
    if m.is_local {
        if m.quota_badge.starts_with("100%") {
            p.green
        } else if m.quota_badge.starts_with("Hybrid") {
            p.amber
        } else {
            p.muted
        }
    } else if let Some(pct) = m.quota_pct {
        if pct >= 50.0 {
            p.green
        } else if pct >= 20.0 {
            p.amber
        } else {
            p.red
        }
    } else {
        p.green
    }
}

fn unified_model_trigger_pill(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    mode_lbl: &str,
    model_lbl: &str,
    quota_badge: Option<&str>,
    quota_col: Color32,
    open: bool,
) -> egui::Response {
    let title = format!("{mode_lbl} · {model_lbl}");
    let title_galley = egui::WidgetText::from(
        egui::RichText::new(&title)
            .small()
            .color(if open { p.text } else { p.muted }),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        220.0,
        egui::TextStyle::Small,
    );
    let q_galley = quota_badge.filter(|s| !s.is_empty()).map(|q_txt| {
        egui::WidgetText::from(egui::RichText::new(q_txt).size(10.5).strong().color(quota_col))
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                100.0,
                egui::TextStyle::Small,
            )
    });
    let chev_galley = egui::WidgetText::from(
        egui::RichText::new(if open { "▲" } else { "▼" })
            .size(9.5)
            .color(p.muted),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        24.0,
        egui::TextStyle::Small,
    );

    let q_pad_w = q_galley.as_ref().map(|g| g.size().x + 18.0).unwrap_or(0.0);
    let total_w = 10.0 + title_galley.size().x + 6.0 + q_pad_w + chev_galley.size().x + 10.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(total_w, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if open {
            p.bg_card
        } else if resp.hovered() {
            p.bg_hover
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(7), fill);
        }
        if open {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(7),
                Stroke::new(1.0_f32, p.border_strong),
                egui::StrokeKind::Inside,
            );
        }
        let fg = if open || resp.hovered() {
            p.text
        } else {
            p.muted
        };
        let mut x = rect.left() + 8.0;
        let tw = title_galley.size().x;
        let th = title_galley.size().y;
        ui.painter()
            .galley(egui::pos2(x, rect.center().y - th * 0.5), title_galley, fg);
        x += tw + 6.0;
        if let Some(qg) = q_galley {
            let pill_w = qg.size().x + 12.0;
            let pill_h = 17.0;
            let pill_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - pill_h * 0.5),
                egui::vec2(pill_w, pill_h),
            );
            ui.painter().rect(
                pill_rect,
                CornerRadius::same(5),
                Color32::from_rgba_unmultiplied(quota_col.r(), quota_col.g(), quota_col.b(), 24),
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(
                        quota_col.r(),
                        quota_col.g(),
                        quota_col.b(),
                        72,
                    ),
                ),
                egui::StrokeKind::Inside,
            );
            ui.painter().galley(
                egui::pos2(
                    pill_rect.left() + 6.0,
                    pill_rect.center().y - qg.size().y * 0.5,
                ),
                qg,
                quota_col,
            );
            x += pill_w + 6.0;
        }
        ui.painter().galley(
            egui::pos2(x, rect.center().y - chev_galley.size().y * 0.5),
            chev_galley,
            p.muted,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn model_picker_row(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    m: &SelectableModel,
    selected: bool,
) -> egui::Response {
    let w = ui.available_width().max(220.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 29.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            p.accent_soft
        } else if resp.hovered() {
            p.bg_hover
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), fill);
        }
        if selected {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(6),
                Stroke::new(1.0_f32, p.accent),
                egui::StrokeKind::Inside,
            );
        }

        let tag_col = if m.is_local {
            p.accent_hot
        } else if m.category == "google" {
            p.green
        } else {
            p.purple
        };
        let tag_galley = egui::WidgetText::from(
            egui::RichText::new(&m.provider_label)
                .size(10.5)
                .strong()
                .color(tag_col),
        )
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            90.0,
            egui::TextStyle::Small,
        );
        let tag_w = (tag_galley.size().x + 12.0).ceil();
        let tag_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.center().y - 9.0),
            egui::vec2(tag_w, 18.0),
        );
        ui.painter().rect(
            tag_rect,
            CornerRadius::same(5),
            Color32::from_rgba_unmultiplied(tag_col.r(), tag_col.g(), tag_col.b(), 22),
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(tag_col.r(), tag_col.g(), tag_col.b(), 64),
            ),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            egui::pos2(
                tag_rect.left() + 6.0,
                tag_rect.center().y - tag_galley.size().y * 0.5,
            ),
            tag_galley,
            tag_col,
        );

        let q_col = quota_pill_color(p, m);
        let q_galley = egui::WidgetText::from(
            egui::RichText::new(&m.quota_badge)
                .size(10.5)
                .strong()
                .color(q_col),
        )
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            90.0,
            egui::TextStyle::Small,
        );
        let q_w = (q_galley.size().x + 12.0).ceil();
        let q_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 8.0 - q_w, rect.center().y - 9.0),
            egui::vec2(q_w, 18.0),
        );
        ui.painter().rect(
            q_rect,
            CornerRadius::same(5),
            Color32::from_rgba_unmultiplied(q_col.r(), q_col.g(), q_col.b(), 22),
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(q_col.r(), q_col.g(), q_col.b(), 68),
            ),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            egui::pos2(
                q_rect.left() + 6.0,
                q_rect.center().y - q_galley.size().y * 0.5,
            ),
            q_galley,
            q_col,
        );

        let name_left = tag_rect.right() + 8.0;
        let name_max_w = (q_rect.left() - 8.0 - name_left).max(40.0);
        let name_fg = if selected { p.text } else { p.text };
        let name_galley = egui::WidgetText::from(
            egui::RichText::new(&m.display_name)
                .small()
                .color(name_fg),
        )
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            name_max_w,
            egui::TextStyle::Small,
        );
        ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).galley(
            egui::pos2(name_left, rect.center().y - name_galley.size().y * 0.5),
            name_galley,
            name_fg,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tip = if m.display_name != m.canonical_name {
        format!("Alias: {} (Canonical: {})\n{}", m.display_name, m.canonical_name, m.quota_tooltip)
    } else {
        m.quota_tooltip.clone()
    };
    resp.on_hover_text(tip)
}

fn chip_remove_button(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    icon: Icon,
    label: &str,
) -> egui::Response {
    let txt = format!("{label}  ×");
    let galley = egui::WidgetText::from(egui::RichText::new(&txt).small()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        240.0,
        egui::TextStyle::Small,
    );
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(galley.size().x + 28.0, 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            if resp.hovered() { p.bg_hover } else { p.bg_card },
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, p.border),
            egui::StrokeKind::Inside,
        );
        draw_icon(
            ui.painter(),
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 11.0, rect.center().y),
                egui::vec2(12.0, 12.0),
            ),
            icon,
            p.muted,
        );
        ui.painter().galley(
            egui::pos2(
                rect.left() + 21.0,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            p.text,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn image_chip_remove_button(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    tex: &egui::TextureHandle,
    label: &str,
) -> egui::Response {
    let txt = format!("{label}  ×");
    let galley = egui::WidgetText::from(egui::RichText::new(&txt).small()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        220.0,
        egui::TextStyle::Small,
    );
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(galley.size().x + 42.0, 28.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(7),
            if resp.hovered() { p.bg_hover } else { p.bg_card },
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(1.0_f32, p.border_strong),
            egui::StrokeKind::Inside,
        );
        let thumb_rect =
            egui::Rect::from_min_size(egui::pos2(rect.left() + 4.0, rect.top() + 3.0), egui::vec2(26.0, 22.0));
        ui.painter().image(
            tex.id(),
            thumb_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        ui.painter().galley(
            egui::pos2(
                rect.left() + 35.0,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            p.text,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn autocomplete_row(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    selected: bool,
    primary_lbl: &str,
    secondary_lbl: &str,
) -> egui::Response {
    let w = ui.available_width().max(160.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if selected || resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(5), p.bg_hover);
        }
        let row_clip = rect.intersect(ui.clip_rect());
        let hint_space = if selected { 42.0 } else { 8.0 };
        let key_galley = egui::WidgetText::from(
            egui::RichText::new(primary_lbl)
                .small()
                .strong()
                .color(if selected { p.accent_hot } else { p.text }),
        )
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Small,
        );
        let key_w = key_galley.size().x;
        let sub_max_w = (w - 16.0 - key_w - 10.0 - hint_space).max(0.0);
        let painter = ui.painter().with_clip_rect(row_clip);
        painter.galley(
            egui::pos2(rect.left() + 8.0, rect.center().y - key_galley.size().y * 0.5),
            key_galley,
            if selected { p.accent_hot } else { p.text },
        );
        if sub_max_w > 24.0 && !secondary_lbl.trim().is_empty() {
            let sub_galley = egui::WidgetText::from(
                egui::RichText::new(secondary_lbl).small().color(p.muted),
            )
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Truncate),
                sub_max_w,
                egui::TextStyle::Small,
            );
            painter.galley(
                egui::pos2(
                    rect.left() + 8.0 + key_w + 10.0,
                    rect.center().y - sub_galley.size().y * 0.5,
                ),
                sub_galley,
                p.muted,
            );
        }
        if selected {
            let tab_galley =
                egui::WidgetText::from(egui::RichText::new("[Tab]").small().color(p.muted))
                    .into_galley(
                        ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::TextStyle::Small,
                    );
            painter.galley(
                egui::pos2(
                    rect.right() - 8.0 - tab_galley.size().x,
                    rect.center().y - tab_galley.size().y * 0.5,
                ),
                tab_galley,
                p.muted,
            );
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_chat_prompt(
    ctx: &egui::Context,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    session_id: &str,
    mode: AgentMode,
    active_model_id: &str,
    compare_model_b_id: &str,
    prompt_text: String,
    files: Vec<String>,
    media: Vec<harness_store::MediaAttachment>,
) {
    let send_msg = if prompt_text.trim().is_empty() {
        "Analyze the attached files/images cleanly.".to_string()
    } else {
        prompt_text.trim().to_string()
    };
    if let Some(ref mut snap_arc) = state.snap {
        let snap_mut = Arc::make_mut(snap_arc);
        if let Some(ref mut active_sess) = snap_mut.active_session {
            if active_sess.id == session_id {
                if active_sess.messages.is_empty() && !send_msg.is_empty() {
                    active_sess.title = send_msg.chars().take(42).collect();
                }
                active_sess.messages.push(harness_store::ChatMessage {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "user".into(),
                    content: send_msg.clone(),
                    thinking: None,
                    model_id: None,
                    latency_ms: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    tool_calls: Vec::new(),
                    compare_partner: None,
                    checkpoint_id: None,
                    plan_artifact: None,
                    orchestrator_phases: Vec::new(),
                    attached_files: files.clone(),
                    attachments: media.clone(),
                    artifact_id: None,
                    fallback_from_model_id: None,
                    fallback_reason: None,
                    created_at: chrono::Utc::now(),
                });
            }
        }
        let max_steps = snap_mut
            .settings
            .orchestrator
            .max_autonomous_steps
            .clamp(2, 64);
        let sent_name = resolve_model_display_name(snap_mut, active_model_id);
        snap_mut.live_activity = harness_store::LiveActivityStatus {
            active: true,
            session_id: Some(session_id.to_string()),
            phase: "thinking".into(),
            detail: format!("Sending to {sent_name}..."),
            step: 1,
            max_steps,
            active_model: active_model_id.to_string(),
            ..Default::default()
        };
    }
    state.is_generating = true;
    ctx.request_repaint();
    backend.send_prompt(
        session_id.to_string(),
        send_msg,
        mode,
        active_model_id.to_string(),
        if mode == AgentMode::Compare {
            Some(compare_model_b_id.to_string())
        } else {
            None
        },
        files,
        media,
    );
}

pub fn format_session_markdown(sess: &harness_store::Session) -> String {
    let mut out = format!("# {}\n\nWorkspace: `{}`\n\n", sess.title, sess.workspace_path);
    for m in &sess.messages {
        let who = if m.role == "user" { "User" } else { "Cinder" };
        out.push_str(&format!("## {who}\n\n{}\n\n", m.content));
    }
    out
}

pub(crate) fn fmt_worked_header(
    latency_ms: Option<u64>,
    is_live: bool,
    step: usize,
    _max_steps: usize,
    tools: &[harness_store::ToolCallRecord],
) -> String {
    if is_live {
        return if step > 0 {
            format!("Working · Turn {step}")
        } else {
            "Working...".into()
        };
    }
    let ms = latency_ms.unwrap_or(0);
    let duration = if ms == 0 {
        String::new()
    } else if ms < 1000 {
        format!("{ms}ms")
    } else {
        let total_secs = ms / 1000;
        if total_secs < 60 {
            format!("{total_secs}s")
        } else {
            let mins = total_secs / 60;
            let rem = total_secs % 60;
            if rem == 0 {
                format!("{mins}m")
            } else {
                format!("{mins}m {rem}s")
            }
        }
    };
    let err_count = tools.iter().filter(|t| t.status != "completed").count();
    let mut parts = Vec::new();
    if err_count > 0 {
        parts.push(format!("Completed with {err_count} error(s)"));
    } else if !tools.is_empty() {
        parts.push("✓ Finished".to_string());
    } else {
        parts.push("✓ Thought trace".to_string());
    }
    if !duration.is_empty() {
        parts.push(duration);
    }
    if !tools.is_empty() {
        parts.push(summarize_tool_activity_group(tools));
    }
    parts.join(" · ")
}

pub(crate) fn summarize_tool_activity_group(tools: &[harness_store::ToolCallRecord]) -> String {
    let mut cats: Vec<&'static str> = Vec::new();
    for tc in tools {
        let cat = match tc.name.as_str() {
            "read_file" | "list_dir" | "search_files" | "extract_pdf_text"
            | "search_knowledge" => "explored files",
            "run_command" | "verify_workspace" => "ran commands",
            "edit_file" | "write_file" | "propose_edit" => "edited files",
            "delegate_vision" | "delegate_research" => "delegated sub-agents",
            "submit_plan" | "write_artifact" | "switch_mode" | "ask_user" => "planned steps",
            n if n.starts_with("mcp__") => "ran MCP tools",
            _ => "ran tools",
        };
        if !cats.contains(&cat) {
            cats.push(cat);
        }
    }
    if cats.is_empty() {
        return "Activity trace".into();
    }
    let joined = cats.join(", ");
    let mut chars = joined.chars();
    match chars.next() {
        None => "Activity trace".into(),
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
    }
}

fn derive_thought_headline(text: &str) -> String {
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Thinking");
    let sentence = first_line
        .split_terminator(&['.', '!', '?'][..])
        .next()
        .unwrap_or(first_line)
        .trim();
    let candidate = if sentence.is_empty() {
        first_line
    } else {
        sentence
    };
    if candidate.chars().count() <= 72 {
        candidate.to_string()
    } else {
        let cut: String = candidate.chars().take(71).collect();
        format!("{cut}…")
    }
}

pub(crate) fn parse_thought_trace_steps(raw: &str) -> Vec<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if !trimmed.contains("[Step ") {
        return vec![(derive_thought_headline(trimmed), trimmed.to_string())];
    }

    let mut steps: Vec<(String, String)> = Vec::new();
    let mut cur_head: Option<String> = None;
    let mut cur_lines: Vec<String> = Vec::new();

    for line in trimmed.lines() {
        let lt = line.trim();
        if lt.starts_with("[Step ") && lt.ends_with(']') && lt.len() > 2 {
            let body = cur_lines.join("\n").trim().to_string();
            if let Some(head) = cur_head.take() {
                steps.push((head, body));
            } else if !body.is_empty() {
                steps.push((derive_thought_headline(&body), body));
            }
            cur_head = Some(lt[1..lt.len() - 1].to_string());
            cur_lines.clear();
        } else {
            cur_lines.push(line.to_string());
        }
    }

    let tail_body = cur_lines.join("\n").trim().to_string();
    if let Some(head) = cur_head {
        steps.push((head, tail_body));
    } else if !tail_body.is_empty() {
        steps.push((derive_thought_headline(&tail_body), tail_body));
    }
    steps
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormattedToolStep {
    pub verb: &'static str,
    pub badge_label: Option<&'static str>,
    pub badge_color: Color32,
    pub subject: String,
    pub suffix: String,
    pub rel_path_to_open: Option<String>,
    pub is_error: bool,
}

fn json_arg_str<'a>(val: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    for &k in keys {
        if let Some(s) = val.get(k).and_then(|v| v.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn path_basename(rel: &str) -> String {
    let norm = rel.trim().replace('\\', "/");
    norm.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(norm.as_str())
        .to_string()
}

fn file_lang_badge(rel: &str, p: &CinderPalette) -> (&'static str, Color32) {
    let ext = rel
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => ("R", p.accent_hot),
        "py" => ("Py", p.accent),
        "ts" | "tsx" => ("TS", p.accent),
        "js" | "jsx" => ("JS", p.amber),
        "md" | "markdown" => ("MD", p.green),
        "toml" | "json" | "yaml" | "yml" => ("{}", p.purple),
        "pdf" => ("PDF", p.red),
        "sh" | "ps1" | "bat" => ("SH", p.green),
        "html" | "css" | "svg" => ("UI", p.accent),
        _ => ("F", p.muted),
    }
}

fn infer_read_line_range(args: &serde_json::Value, output: &str) -> String {
    match (args["start_line"].as_u64(), args["end_line"].as_u64()) {
        (Some(s), Some(e)) => return format!("#L{s}-{e}"),
        (Some(s), None) => {
            if let Some(last) = infer_last_numbered_line(output) {
                return format!("#L{s}-{last}");
            }
            return format!("#L{s}+");
        }
        _ => {}
    }
    let mut first: Option<u64> = None;
    let mut last: Option<u64> = None;
    for line in output.lines() {
        if let Some((lhs, _)) = line.split_once(": ") {
            if let Ok(n) = lhs.trim().parse::<u64>() {
                if first.is_none() {
                    first = Some(n);
                }
                last = Some(n);
            }
        }
    }
    match (first, last) {
        (Some(a), Some(b)) if b >= a => format!("#L{a}-{b}"),
        _ => String::new(),
    }
}

fn infer_last_numbered_line(output: &str) -> Option<u64> {
    let mut last = None;
    for line in output.lines() {
        if let Some((lhs, _)) = line.split_once(": ") {
            if let Ok(n) = lhs.trim().parse::<u64>() {
                last = Some(n);
            }
        }
    }
    last
}

fn count_diff_stats(diff: Option<&str>) -> String {
    let Some(d) = diff else {
        return String::new();
    };
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in d.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
    }
    if added == 0 && removed == 0 {
        String::new()
    } else {
        format!("+{added} -{removed}")
    }
}

fn extract_subagent_model(tc: &harness_store::ToolCallRecord) -> Option<&str> {
    if let Some(m) = tc
        .subagent_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(m);
    }
    let first_line = tc.output.lines().next()?.trim();
    for prefix in ["[Vision Sub-Agent · ", "[Research Sub-Agent · "] {
        if let Some(rest) = first_line.strip_prefix(prefix) {
            let model_part = rest
                .split(" · ")
                .next()
                .unwrap_or(rest)
                .trim_end_matches(']')
                .trim();
            if !model_part.is_empty() {
                return Some(model_part);
            }
        }
    }
    None
}

pub(crate) fn format_antigravity_tool_step(
    tc: &harness_store::ToolCallRecord,
    p: &CinderPalette,
) -> FormattedToolStep {
    let args: serde_json::Value =
        serde_json::from_str(&tc.arguments_json).unwrap_or(serde_json::Value::Null);
    let is_error = tc.status != "completed";

    match tc.name.as_str() {
        "read_file" => {
            let rel = tc
                .file_path
                .as_deref()
                .or_else(|| json_arg_str(&args, &["path", "file_path", "filepath", "file"]))
                .unwrap_or("file");
            let (bl, bc) = file_lang_badge(rel, p);
            FormattedToolStep {
                verb: "Analyzed",
                badge_label: Some(bl),
                badge_color: bc,
                subject: path_basename(rel),
                suffix: infer_read_line_range(&args, &tc.output),
                rel_path_to_open: Some(rel.to_string()),
                is_error,
            }
        }
        "extract_pdf_text" => {
            let rel = tc
                .file_path
                .as_deref()
                .or_else(|| json_arg_str(&args, &["path", "file_path", "filepath"]))
                .unwrap_or("document.pdf");
            FormattedToolStep {
                verb: "Analyzed",
                badge_label: Some("PDF"),
                badge_color: p.red,
                subject: path_basename(rel),
                suffix: String::new(),
                rel_path_to_open: Some(rel.to_string()),
                is_error,
            }
        }
        "edit_file" | "write_file" | "propose_edit" => {
            let rel = tc
                .file_path
                .as_deref()
                .or_else(|| json_arg_str(&args, &["path", "file_path", "filepath"]))
                .unwrap_or("file");
            let (bl, bc) = file_lang_badge(rel, p);
            let verb = match tc.name.as_str() {
                "write_file" => "Wrote",
                "propose_edit" => "Proposed edit",
                _ => "Edited",
            };
            FormattedToolStep {
                verb,
                badge_label: Some(bl),
                badge_color: bc,
                subject: path_basename(rel),
                suffix: count_diff_stats(tc.diff.as_deref()),
                rel_path_to_open: Some(rel.to_string()),
                is_error,
            }
        }
        "search_files" | "search_knowledge" => {
            let pat = json_arg_str(&args, &["pattern", "query", "regex", "text"])
                .unwrap_or("query");
            let sub = json_arg_str(&args, &["sub_path", "path", "dir", "directory"])
                .filter(|s| *s != ".");
            let count = tc
                .output
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with("... ") && !l.starts_with("No "))
                .count();
            let mut suffix = String::new();
            if let Some(s) = sub {
                suffix.push_str(&format!("in {s}"));
            }
            if !is_error && count > 0 {
                if !suffix.is_empty() {
                    suffix.push(' ');
                }
                suffix.push_str(&format!("· {count} matches"));
            }
            FormattedToolStep {
                verb: "Searched",
                badge_label: None,
                badge_color: Color32::TRANSPARENT,
                subject: format!("\"{pat}\""),
                suffix,
                rel_path_to_open: None,
                is_error,
            }
        }
        "list_dir" => {
            let rel = json_arg_str(&args, &["path", "dir", "directory", "folder"]).unwrap_or(".");
            let count = tc
                .output
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('(') && !l.starts_with("..."))
                .count();
            FormattedToolStep {
                verb: "Explored",
                badge_label: None,
                badge_color: Color32::TRANSPARENT,
                subject: if rel == "." {
                    "workspace root".into()
                } else {
                    rel.to_string()
                },
                suffix: if !is_error && count > 0 {
                    format!("· {count} entries")
                } else {
                    String::new()
                },
                rel_path_to_open: None,
                is_error,
            }
        }
        "run_command" | "verify_workspace" => {
            let raw_cmd = json_arg_str(&args, &["command", "cmd"]).unwrap_or("command");
            let single = raw_cmd.split_whitespace().collect::<Vec<_>>().join(" ");
            let cmd = if single.chars().count() <= 54 {
                single
            } else {
                let cut: String = single.chars().take(53).collect();
                format!("{cut}…")
            };
            let exit_suffix = tc
                .output
                .lines()
                .find_map(|l| {
                    l.trim()
                        .strip_prefix("Exit code:")
                        .and_then(|rest| rest.trim().parse::<i32>().ok())
                })
                .map(|code| format!("· Exited with code {code}"))
                .unwrap_or_else(|| {
                    if is_error {
                        "· Failed".into()
                    } else {
                        "· Exited with code 0".into()
                    }
                });
            FormattedToolStep {
                verb: "Ran",
                badge_label: None,
                badge_color: Color32::TRANSPARENT,
                subject: cmd,
                suffix: exit_suffix,
                rel_path_to_open: None,
                is_error,
            }
        }
        "delegate_vision" => {
            let q = json_arg_str(&args, &["prompt"]).unwrap_or("Inspect visual attachment");
            let short_q = if q.chars().count() <= 56 {
                q.to_string()
            } else {
                let cut: String = q.chars().take(55).collect();
                format!("{cut}…")
            };
            let subj = match extract_subagent_model(tc) {
                Some(m) => format!("Vision Sub-Agent → {m}"),
                None => "Vision Sub-Agent".into(),
            };
            FormattedToolStep {
                verb: "Delegated",
                badge_label: Some("VS"),
                badge_color: p.accent,
                subject: subj,
                suffix: format!("· {short_q}"),
                rel_path_to_open: None,
                is_error,
            }
        }
        "delegate_research" => {
            let t = json_arg_str(&args, &["task"]).unwrap_or("Inspect files");
            let short_t = if t.chars().count() <= 56 {
                t.to_string()
            } else {
                let cut: String = t.chars().take(55).collect();
                format!("{cut}…")
            };
            let subj = match extract_subagent_model(tc) {
                Some(m) => format!("Research Sub-Agent → {m}"),
                None => "Research Sub-Agent".into(),
            };
            FormattedToolStep {
                verb: "Delegated",
                badge_label: Some("RS"),
                badge_color: p.purple,
                subject: subj,
                suffix: format!("· {short_t}"),
                rel_path_to_open: None,
                is_error,
            }
        }
        "web_search" => {
            let q = json_arg_str(&args, &["query"]).unwrap_or("query");
            FormattedToolStep {
                verb: if is_error {
                    "Web search failed"
                } else {
                    "Searched web"
                },
                badge_label: Some("WEB"),
                badge_color: p.accent,
                subject: format!("\"{q}\""),
                suffix: String::new(),
                rel_path_to_open: None,
                is_error,
            }
        }
        "fetch_url" => {
            let url = json_arg_str(&args, &["url", "query", "href"]).unwrap_or("URL");
            FormattedToolStep {
                verb: if is_error {
                    "Fetch failed"
                } else {
                    "Fetched URL"
                },
                badge_label: Some("WEB"),
                badge_color: p.accent,
                subject: url.to_string(),
                suffix: String::new(),
                rel_path_to_open: None,
                is_error,
            }
        }
        "switch_mode" => {
            let target = json_arg_str(&args, &["target_mode"]).unwrap_or("next");
            let reason = json_arg_str(&args, &["reason"]).unwrap_or("");
            FormattedToolStep {
                verb: if is_error {
                    "Blocked mode switch"
                } else {
                    "Switched mode"
                },
                badge_label: None,
                badge_color: Color32::TRANSPARENT,
                subject: target.to_string(),
                suffix: if reason.is_empty() {
                    String::new()
                } else {
                    format!("· {reason}")
                },
                rel_path_to_open: None,
                is_error,
            }
        }
        "submit_plan" | "write_artifact" => {
            let title = json_arg_str(&args, &["title"]).unwrap_or("artifact");
            FormattedToolStep {
                verb: if tc.name == "submit_plan" {
                    "Planned"
                } else {
                    "Wrote artifact"
                },
                badge_label: None,
                badge_color: Color32::TRANSPARENT,
                subject: title.to_string(),
                suffix: String::new(),
                rel_path_to_open: None,
                is_error,
            }
        }
        _ => FormattedToolStep {
            verb: if is_error { "Failed" } else { "Ran" },
            badge_label: None,
            badge_color: Color32::TRANSPARENT,
            subject: tc.name.clone(),
            suffix: format!("· {}", tc.status),
            rel_path_to_open: tc.file_path.clone(),
            is_error,
        },
    }
}

pub(crate) fn window_trace_slice(
    total: usize,
    keep_recent: usize,
    expanded: bool,
) -> (usize, usize) {
    if expanded || total <= keep_recent + 1 {
        (0, 0)
    } else {
        let start = total.saturating_sub(keep_recent);
        (start, start)
    }
}

#[allow(clippy::too_many_arguments)]
fn render_activity_and_thought_trace(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    scope_id: &str,
    latency_ms: Option<u64>,
    is_live: bool,
    step: usize,
    max_steps: usize,
    thinking_raw: &str,
    tools: &[harness_store::ToolCallRecord],
) -> Option<String> {
    let mut file_to_open: Option<String> = None;
    let worked_title = fmt_worked_header(latency_ms, is_live, step, max_steps, tools);
    let err_count = tools.iter().filter(|t| t.status != "completed").count();
    let header_col = if is_live {
        p.muted
    } else if err_count > 0 {
        p.amber
    } else if !tools.is_empty() {
        p.green
    } else {
        p.muted
    };
    let thought_steps = parse_thought_trace_steps(thinking_raw);
    let root_id = ui.make_persistent_id(("trace_root", scope_id));
    let mut trace_state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), root_id, is_live);

    Frame::new()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .inner_margin(Margin {
            left: 0,
            right: 0,
            top: 2,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let is_open = trace_state.is_open();
            let header_resp = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let chevron = if is_open { "▾" } else { "▸" };
                    ui.add(
                        egui::Label::new(egui::RichText::new(chevron).small().color(header_col))
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .selectable(false),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&worked_title).small().color(header_col),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .selectable(false),
                    );
                    if is_live {
                        draw_working_dots(ui, p);
                        request_smooth_anim_repaint(ui.ctx());
                    }
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if header_resp.clicked() {
                trace_state.toggle(ui);
            }

            trace_state.show_body_unindented(ui, |ui| {
                Frame::new()
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .inner_margin(Margin {
                        left: 10,
                        right: 0,
                        top: 2,
                        bottom: 2,
                    })
                    .show(ui, |ui| {
                        // 1. Per-step planning Thought Traces (windowed cleanly when long)
                        if !thought_steps.is_empty() {
                            ui.add_space(2.0);
                            let th_exp_id =
                                ui.make_persistent_id(("trace_thoughts_expanded", scope_id));
                            let mut th_exp =
                                ui.data(|d| d.get_temp::<bool>(th_exp_id).unwrap_or(false));
                            let (th_hidden, th_start) =
                                window_trace_slice(thought_steps.len(), 3, th_exp);
                            if thought_steps.len() > 4 {
                                let toggle_lbl = if th_exp {
                                    "▾ Show fewer earlier thoughts".to_string()
                                } else {
                                    format!("▸ Show {th_hidden} earlier thoughts")
                                };
                                if ghost_button(ui, p, egui::RichText::new(toggle_lbl).small())
                                    .clicked()
                                {
                                    th_exp = !th_exp;
                                    ui.data_mut(|d| d.insert_temp(th_exp_id, th_exp));
                                }
                            }
                            for (sidx, (head, body)) in
                                thought_steps.iter().enumerate().skip(th_start)
                            {
                                Frame::new()
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE)
                                    .inner_margin(Margin {
                                        left: 8,
                                        right: 2,
                                        top: 2,
                                        bottom: 3,
                                    })
                                    .show(ui, |ui| {
                                        let bar_resp = ui.vertical(|ui| {
                                            ui.set_width(ui.available_width());
                                            ui.label(
                                                egui::RichText::new(format!("Thought · {head}"))
                                                    .small()
                                                    .strong()
                                                    .color(p.text),
                                            );
                                            if !body.trim().is_empty() {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(body)
                                                            .small()
                                                            .color(p.muted),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                        });
                                        let left_x = bar_resp.response.rect.left() - 6.0;
                                        let top_y = bar_resp.response.rect.top() + 2.0;
                                        let bot_y = bar_resp.response.rect.bottom() - 2.0;
                                        if bot_y > top_y {
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(left_x, top_y),
                                                    egui::pos2(left_x, bot_y),
                                                ],
                                                Stroke::new(
                                                    2.0_f32,
                                                    if is_live && sidx + 1 == thought_steps.len() {
                                                        p.accent
                                                    } else {
                                                        p.border_strong
                                                    },
                                                ),
                                            );
                                        }
                                    });
                            }
                            if !tools.is_empty() {
                                ui.add_space(4.0);
                            }
                        }

                        // 2. Hierarchical Activity Summary Group (windowed when long, errors always visible)
                        if !tools.is_empty() {
                            let group_title = summarize_tool_activity_group(tools);
                            egui::CollapsingHeader::new(
                                egui::RichText::new(group_title).small().color(p.muted),
                            )
                            .id_salt(("act_group", scope_id))
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 3.0;
                                let tool_exp_id =
                                    ui.make_persistent_id(("trace_tools_expanded", scope_id));
                                let mut tool_exp =
                                    ui.data(|d| d.get_temp::<bool>(tool_exp_id).unwrap_or(false));
                                let (_, tool_start) = window_trace_slice(tools.len(), 5, tool_exp);
                                let hidden_tools = (0..tool_start)
                                    .filter(|&i| tools[i].status == "completed")
                                    .count();
                                if hidden_tools > 0 || (tool_exp && tools.len() > 6) {
                                    let toggle_lbl = if tool_exp {
                                        "▾ Show fewer tool steps".to_string()
                                    } else {
                                        format!("▸ Show {hidden_tools} earlier tool steps")
                                    };
                                    if ghost_button(ui, p, egui::RichText::new(toggle_lbl).small())
                                        .clicked()
                                    {
                                        tool_exp = !tool_exp;
                                        ui.data_mut(|d| d.insert_temp(tool_exp_id, tool_exp));
                                    }
                                }
                                for (tidx, tc) in tools.iter().enumerate() {
                                    if tidx < tool_start && tc.status == "completed" {
                                        continue;
                                    }
                                    let step_fmt = format_antigravity_tool_step(tc, p);
                                    let det_id =
                                        ui.make_persistent_id(("act_step_open", scope_id, tidx));
                                    let mut open =
                                        ui.data(|d| d.get_temp::<bool>(det_id).unwrap_or(false));

                                    let row_resp = ui
                                        .horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 5.0;
                                            ui.add_space(4.0);
                                            let verb_col = if step_fmt.is_error {
                                                p.red
                                            } else {
                                                p.muted
                                            };
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(step_fmt.verb)
                                                        .small()
                                                        .color(verb_col),
                                                )
                                                .wrap_mode(egui::TextWrapMode::Extend)
                                                .selectable(false),
                                            );
                                            if let Some(bl) = step_fmt.badge_label {
                                                let bg = Color32::from_rgba_unmultiplied(
                                                    step_fmt.badge_color.r(),
                                                    step_fmt.badge_color.g(),
                                                    step_fmt.badge_color.b(),
                                                    38,
                                                );
                                                Frame::new()
                                                    .fill(bg)
                                                    .corner_radius(CornerRadius::same(4))
                                                    .inner_margin(Margin::symmetric(4, 1))
                                                    .show(ui, |ui| {
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(bl)
                                                                    .small()
                                                                    .strong()
                                                                    .color(step_fmt.badge_color),
                                                            )
                                                            .wrap_mode(egui::TextWrapMode::Extend)
                                                            .selectable(false),
                                                        );
                                                    });
                                            }
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&step_fmt.subject)
                                                        .small()
                                                        .color(if step_fmt.is_error {
                                                            p.red
                                                        } else {
                                                            p.text
                                                        }),
                                                )
                                                .wrap_mode(egui::TextWrapMode::Extend)
                                                .selectable(false),
                                            );
                                            if !step_fmt.suffix.is_empty() {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(&step_fmt.suffix)
                                                            .small()
                                                            .color(p.muted),
                                                    )
                                                    .wrap_mode(egui::TextWrapMode::Extend)
                                                    .selectable(false),
                                                );
                                            }
                                            let arrow = if open { "⌄" } else { "›" };
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(arrow)
                                                        .small()
                                                        .color(p.muted),
                                                )
                                                .wrap_mode(egui::TextWrapMode::Extend)
                                                .selectable(false),
                                            );
                                        })
                                        .response
                                        .interact(egui::Sense::click())
                                        .on_hover_cursor(egui::CursorIcon::PointingHand);

                                    if let Some(ref rel_tip) = step_fmt.rel_path_to_open {
                                        if row_resp.hovered() {
                                            row_resp.clone().on_hover_text(format!(
                                                "{rel_tip} — click to inspect details"
                                            ));
                                        }
                                    }

                                    if row_resp.clicked() {
                                        open = !open;
                                        ui.data_mut(|d| d.insert_temp(det_id, open));
                                    }

                                    if open {
                                        Frame::new()
                                            .fill(p.bg_card)
                                            .stroke(Stroke::new(1.0_f32, p.border))
                                            .corner_radius(CornerRadius::same(6))
                                            .inner_margin(Margin::symmetric(8, 6))
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                if let Some(sub_m) = extract_subagent_model(tc) {
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            egui::RichText::new("Delegated model:")
                                                                .small()
                                                                .color(p.muted),
                                                        );
                                                        badge(
                                                            ui,
                                                            sub_m,
                                                            p.purple,
                                                            Color32::from_rgba_unmultiplied(
                                                                p.purple.r(),
                                                                p.purple.g(),
                                                                p.purple.b(),
                                                                32,
                                                            ),
                                                        );
                                                    });
                                                    ui.add_space(3.0);
                                                }
                                                if let Some(ref rel) = step_fmt.rel_path_to_open {
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            egui::RichText::new(rel)
                                                                .monospace()
                                                                .small()
                                                                .color(p.muted),
                                                        );
                                                        if secondary_button(
                                                            ui,
                                                            p,
                                                            "Open in Editor",
                                                        )
                                                        .clicked()
                                                        {
                                                            file_to_open = Some(rel.clone());
                                                        }
                                                    });
                                                    ui.add_space(3.0);
                                                }
                                                let args_trimmed = tc.arguments_json.trim();
                                                if !args_trimmed.is_empty() && args_trimmed != "{}"
                                                {
                                                    if args_trimmed.chars().count() <= 400 {
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(args_trimmed)
                                                                    .monospace()
                                                                    .small()
                                                                    .color(p.muted),
                                                            )
                                                            .wrap(),
                                                        );
                                                    } else {
                                                        let capped_args: String = args_trimmed
                                                            .chars()
                                                            .take(8_000)
                                                            .collect();
                                                        render_resizable_text_box(
                                                            ui,
                                                            p,
                                                            &capped_args,
                                                            &format!(
                                                                "{scope_id}_arg_{tidx}_{}",
                                                                tc.id
                                                            ),
                                                            130.0,
                                                            72.0,
                                                            640.0,
                                                        );
                                                    }
                                                }
                                                if let Some(ref diff) = tc.diff {
                                                    render_diff(ui, p, diff, &tc.id);
                                                } else if !tc.output.trim().is_empty() {
                                                    let capped_out: String =
                                                        tc.output.chars().take(12_000).collect();
                                                    render_resizable_text_box(
                                                        ui,
                                                        p,
                                                        &capped_out,
                                                        &format!(
                                                            "{scope_id}_out_{tidx}_{}",
                                                            tc.id
                                                        ),
                                                        180.0,
                                                        80.0,
                                                        760.0,
                                                    );
                                                }
                                            });
                                    }
                                }
                            });
                        }
                    });
            });
        });

    file_to_open
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::CinderPalette;

    #[test]
    fn formats_worked_header_and_activity_group_summary() {
        let mk = |name: &str| harness_store::ToolCallRecord {
            id: "1".into(),
            name: name.into(),
            arguments_json: "{}".into(),
            status: "completed".into(),
            output: String::new(),
            diff: None,
            file_path: None,
            risk_tier: None,
            subagent_summary: None,
            subagent_model: None,
        };
        assert_eq!(fmt_worked_header(Some(840), false, 0, 0, &[]), "✓ Thought trace · 840ms");
        assert_eq!(
            fmt_worked_header(Some(480_000), false, 0, 0, &[mk("read_file")]),
            "✓ Finished · 8m · Explored files"
        );
        assert_eq!(
            fmt_worked_header(Some(135_000), false, 0, 0, &[]),
            "✓ Thought trace · 2m 15s"
        );
        assert_eq!(
            fmt_worked_header(None, true, 2, 18, &[]),
            "Working · Turn 2"
        );

        let summary = summarize_tool_activity_group(&[
            mk("read_file"),
            mk("run_command"),
            mk("edit_file"),
        ]);
        assert_eq!(summary, "Explored files, ran commands, edited files");
    }

    #[test]
    fn formats_antigravity_tool_steps_and_parses_thought_steps() {
        let p = CinderPalette::from_ui(&harness_store::UiCustomization::default());
        let read_rec = harness_store::ToolCallRecord {
            id: "1".into(),
            name: "read_file".into(),
            arguments_json: serde_json::json!({
                "path": "crates/harness-core/src/lib.rs",
                "start_line": 1400,
                "end_line": 1740
            })
            .to_string(),
            status: "completed".into(),
            output: String::new(),
            diff: None,
            file_path: Some("crates/harness-core/src/lib.rs".into()),
            risk_tier: None,
            subagent_summary: None,
            subagent_model: None,
        };
        let step = format_antigravity_tool_step(&read_rec, &p);
        assert_eq!(step.verb, "Analyzed");
        assert_eq!(step.badge_label, Some("R"));
        assert_eq!(step.subject, "lib.rs");
        assert_eq!(step.suffix, "#L1400-1740");

        let cmd_rec = harness_store::ToolCallRecord {
            id: "2".into(),
            name: "run_command".into(),
            arguments_json: serde_json::json!({ "command": "cargo clippy audit" }).to_string(),
            status: "error".into(),
            output: "Exit code: 1\nSTDERR:\nerror".into(),
            diff: None,
            file_path: None,
            risk_tier: None,
            subagent_summary: None,
            subagent_model: None,
        };
        let cmd_step = format_antigravity_tool_step(&cmd_rec, &p);
        assert_eq!(cmd_step.verb, "Ran");
        assert_eq!(cmd_step.subject, "cargo clippy audit");
        assert_eq!(cmd_step.suffix, "· Exited with code 1");

        let res_rec = harness_store::ToolCallRecord {
            id: "3".into(),
            name: "delegate_research".into(),
            arguments_json: serde_json::json!({ "task": "Look up Langlands" }).to_string(),
            status: "completed".into(),
            output: "[Research Sub-Agent · gemini-3.5-flash]\nSummary".into(),
            diff: None,
            file_path: None,
            risk_tier: None,
            subagent_summary: None,
            subagent_model: Some("gemini-3.5-flash".into()),
        };
        let res_step = format_antigravity_tool_step(&res_rec, &p);
        assert_eq!(res_step.verb, "Delegated");
        assert_eq!(res_step.subject, "Research Sub-Agent → gemini-3.5-flash");

        let web_rec = harness_store::ToolCallRecord {
            id: "4".into(),
            name: "web_search".into(),
            arguments_json: serde_json::json!({ "query": "Langlands program wikipedia" }).to_string(),
            status: "completed".into(),
            output: "[Live Web Search]".into(),
            diff: None,
            file_path: None,
            risk_tier: None,
            subagent_summary: None,
            subagent_model: None,
        };
        let web_step = format_antigravity_tool_step(&web_rec, &p);
        assert_eq!(web_step.verb, "Searched web");
        assert_eq!(web_step.badge_label, Some("WEB"));
        assert_eq!(web_step.subject, "\"Langlands program wikipedia\"");

        let parsed = parse_thought_trace_steps(
            "[Step 1 · search for '*.ico']\nSearching for the embedded icon.\n\n[Step 2 · analyze lib.rs]\nInspecting window icon loader.",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "Step 1 · search for '*.ico'");
        assert_eq!(parsed[0].1, "Searching for the embedded icon.");
        assert_eq!(parsed[1].0, "Step 2 · analyze lib.rs");

        assert_eq!(window_trace_slice(4, 3, false), (0, 0));
        assert_eq!(window_trace_slice(10, 3, false), (7, 7));
        assert_eq!(window_trace_slice(10, 3, true), (0, 0));

        assert_eq!(
            composer_return_key_shortcut(),
            egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)
        );
    }
}
