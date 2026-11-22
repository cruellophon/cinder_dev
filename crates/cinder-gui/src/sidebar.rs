use crate::app_state::{
    fmt_bytes, fmt_rel_age, norm_path, CustomTab, ForgeTab, GuiState, MainView, PromptModal,
    SettingsTab, StageTab,
};
use crate::backend::{BackendHandle, FolderPickTarget};
use crate::icons::{draw_icon, icon_button, icon_button_enabled, Icon};
use crate::theme::{
    copy_to_clipboard, danger_button, draw_cinder_orb_animated, draw_working_indicator,
    ghost_button, input_frame, nav_row_with_icon, secondary_button, toggle_switch, CinderPalette,
};
use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use harness_net::PrivacyMode;
use harness_store::WorkspaceEntry;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

pub fn render_topbar(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        return;
    };

    let is_hybrid = snap.privacy_audit.privacy_mode == PrivacyMode::Hybrid;
    let srv_running = snap.server_status.running;
    let dl_active_cnt = snap
        .downloads
        .iter()
        .filter(|d| {
            matches!(
                d.status,
                harness_models::DownloadStatus::Downloading
                    | harness_models::DownloadStatus::Verifying
                    | harness_models::DownloadStatus::Queued
            )
        })
        .count();
    let dl_paused_cnt = snap
        .downloads
        .iter()
        .filter(|d| matches!(d.status, harness_models::DownloadStatus::Paused))
        .count();
    if dl_active_cnt > 0 {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }

    let min_quota_pct = snap
        .cloud_quotas
        .iter()
        .filter(|q| q.reachable)
        .map(|q| q.remaining_daily_pct.round() as i64)
        .min()
        .unwrap_or(100);

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
        ui.style_mut().spacing.button_padding = egui::vec2(8.0, 2.5);

        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            // Top-left animated Cinder Glowing Orb (version hidden cleanly inside tooltip)
            let (orb_rect, orb_resp) =
                ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
            if orb_resp.hovered() {
                ui.painter()
                    .rect_filled(orb_rect, CornerRadius::same(6), p.bg_hover);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            draw_cinder_orb_animated(ui, orb_rect.center(), 9.0, p);
            if orb_resp
                .on_hover_text("Cinder v5.5 — Local Agentic IDE & Model Forge\nClick to return to Chat")
                .clicked()
            {
                state.switch_main_view(MainView::Chat, true);
                state.show_settings_modal = false;
                state.show_status_popover = false;
                state.show_downloads_popover = false;
            }

            ui.menu_button("Cinder", |ui| {
                if ui.button("+ New Conversation").clicked() {
                    backend.create_session(None, None);
                    state.switch_main_view(MainView::Chat, true);
                    ui.close_menu();
                }
                if ui.button("New One-Off Conversation").clicked() {
                    backend.create_standalone_session(None);
                    state.switch_main_view(MainView::Chat, true);
                    ui.close_menu();
                }
                if ui.button("Command Palette (Ctrl+K)").clicked() {
                    state.show_cmd_palette = true;
                    ui.close_menu();
                }
                if ui.button("Settings (Ctrl+,)").clicked() {
                    state.open_settings(state.settings_tab);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Automations").clicked() {
                    state.switch_main_view(MainView::Automations, true);
                    ui.close_menu();
                }
                if ui.button("Forge & Models").clicked() {
                    state.switch_main_view(MainView::Forge(ForgeTab::Vault), true);
                    ui.close_menu();
                }
                if ui.button("Customizations").clicked() {
                    state.switch_main_view(MainView::Customizations(CustomTab::Plugins), true);
                    ui.close_menu();
                }
                if srv_running {
                    ui.separator();
                    if ui
                        .button(egui::RichText::new("Quench Local VRAM").color(p.red))
                        .clicked()
                    {
                        backend.stop_all_local_models();
                        ui.close_menu();
                    }
                }
            });

            ui.menu_button("Workspace", |ui| {
                if ui.button("Open Project Folder...").clicked() {
                    backend.open_folder_picker(
                        FolderPickTarget::Workspace,
                        "Select Project Folder for Cinder",
                    );
                    ui.close_menu();
                }
                if ui.button("Toggle File Explorer (Ctrl+Shift+E)").clicked() {
                    state.main_view = MainView::Chat;
                    state.show_settings_modal = false;
                    state.show_explorer = !state.show_explorer;
                    ui.close_menu();
                }
                if ui.button("Workspace Rules & Projects").clicked() {
                    state.switch_main_view(MainView::Customizations(CustomTab::Rules), true);
                    ui.close_menu();
                }
                if ui.button("Refresh Workspace").clicked() {
                    backend.refresh_state(false);
                    ui.close_menu();
                }
            });

            ui.menu_button("View", |ui| {
                if ui.button("Toggle Sidebar (Ctrl+B)").clicked() {
                    state.show_sidebar = !state.show_sidebar;
                    ui.close_menu();
                }
                if ui.button("Toggle Coding Stage (Ctrl+E)").clicked() {
                    state.main_view = MainView::Chat;
                    state.show_settings_modal = false;
                    state.show_workbench = !state.show_workbench;
                    ui.close_menu();
                }
                if ui.button("Toggle File Explorer (Ctrl+Shift+E)").clicked() {
                    state.main_view = MainView::Chat;
                    state.show_settings_modal = false;
                    state.show_explorer = !state.show_explorer;
                    ui.close_menu();
                }
                if ui.button("Toggle Terminal (Ctrl+`)").clicked() {
                    state.show_terminal = !state.show_terminal;
                    if state.show_terminal {
                        state.open_stage_tab(StageTab::Editor);
                    }
                    ui.close_menu();
                }
                ui.separator();
                ui.menu_button("Theme", |ui| {
                    for (k, l) in [
                        ("cinder_forge", "Cinder Slate Dark"),
                        ("singularity_obsidian", "Obsidian Classic"),
                        ("nordic_slate", "Nordic Slate"),
                        ("paper_light", "Paper Light"),
                    ] {
                        if ui
                            .selectable_label(snap.settings.ui.theme == k, l)
                            .clicked()
                        {
                            let mut next_ui = snap.settings.ui.clone();
                            next_ui.theme = k.into();
                            backend.save_ui_and_orch(Some(next_ui), None);
                            ui.close_menu();
                        }
                    }
                });
            });

            // Right-side controls + draggable spacer + browser-style download progress pill
            let rest_w = ui.available_width().max(1.0);
            ui.allocate_ui_with_layout(
                egui::vec2(rest_w, 24.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));

                    // Window Close button
                    let (close_rect, close_resp) =
                        ui.allocate_exact_size(egui::vec2(28.0, 22.0), egui::Sense::click());
                    if close_resp.hovered() {
                        ui.painter().rect_filled(
                            close_rect,
                            CornerRadius::same(5),
                            Color32::from_rgb(196, 43, 43),
                        );
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    draw_icon(
                        ui.painter(),
                        egui::Rect::from_center_size(close_rect.center(), egui::vec2(15.0, 15.0)),
                        Icon::Close,
                        if close_resp.hovered() {
                            Color32::WHITE
                        } else {
                            p.muted
                        },
                    );
                    if close_resp.on_hover_text("Close Window").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }

                    // Window Maximize / Restore button
                    let (max_rect, max_resp) =
                        ui.allocate_exact_size(egui::vec2(28.0, 22.0), egui::Sense::click());
                    if max_resp.hovered() {
                        ui.painter()
                            .rect_filled(max_rect, CornerRadius::same(5), p.bg_hover);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    draw_icon(
                        ui.painter(),
                        egui::Rect::from_center_size(max_rect.center(), egui::vec2(15.0, 15.0)),
                        Icon::WindowMaximize,
                        if max_resp.hovered() { p.text } else { p.muted },
                    );
                    if max_resp
                        .on_hover_text(if is_max { "Restore" } else { "Maximize" })
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                    }

                    // Window Minimize button
                    let (min_rect, min_resp) =
                        ui.allocate_exact_size(egui::vec2(28.0, 22.0), egui::Sense::click());
                    if min_resp.hovered() {
                        ui.painter()
                            .rect_filled(min_rect, CornerRadius::same(5), p.bg_hover);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    draw_icon(
                        ui.painter(),
                        egui::Rect::from_center_size(min_rect.center(), egui::vec2(15.0, 15.0)),
                        Icon::WindowMinimize,
                        if min_resp.hovered() { p.text } else { p.muted },
                    );
                    if min_resp.on_hover_text("Minimize").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }

                    // Browser-style Download Progress Pill in top-right titlebar
                    if !snap.downloads.is_empty() {
                        let in_flight: Vec<&harness_models::DownloadJob> = snap
                            .downloads
                            .iter()
                            .filter(|d| {
                                matches!(
                                    d.status,
                                    harness_models::DownloadStatus::Downloading
                                        | harness_models::DownloadStatus::Verifying
                                        | harness_models::DownloadStatus::Queued
                                        | harness_models::DownloadStatus::Paused
                                )
                            })
                            .collect();
                        let (pill_label, pct, bar_col) = if !in_flight.is_empty() {
                            let got: u64 = in_flight.iter().map(|d| d.bytes_downloaded).sum();
                            let tot: u64 = in_flight.iter().map(|d| d.total_bytes).sum();
                            let pct_f = if tot > 0 {
                                (got as f32 / tot as f32).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            let total_mbps: f64 = in_flight
                                .iter()
                                .filter(|d| {
                                    matches!(
                                        d.status,
                                        harness_models::DownloadStatus::Downloading
                                    )
                                })
                                .map(|d| d.speed_mbps)
                                .sum();
                            let lbl = if dl_active_cnt > 0 {
                                if total_mbps > 0.05 {
                                    format!("{}% · {:.1} MB/s", (pct_f * 100.0).round() as u32, total_mbps)
                                } else if tot > 0 {
                                    format!("{}% ({dl_active_cnt})", (pct_f * 100.0).round() as u32)
                                } else {
                                    format!("Downloading ({dl_active_cnt})")
                                }
                            } else {
                                format!("Paused ({dl_paused_cnt}) · {}%", (pct_f * 100.0).round() as u32)
                            };
                            let col = if dl_active_cnt > 0 {
                                p.accent_hot
                            } else {
                                p.amber
                            };
                            (lbl, pct_f, col)
                        } else {
                            let done_cnt = snap
                                .downloads
                                .iter()
                                .filter(|d| {
                                    matches!(d.status, harness_models::DownloadStatus::Completed)
                                })
                                .count();
                            let lbl = if done_cnt > 0 {
                                format!("Downloads ({done_cnt})")
                            } else {
                                "Downloads".to_string()
                            };
                            (lbl, if done_cnt > 0 { 1.0 } else { 0.0 }, p.green)
                        };

                        let galley = egui::WidgetText::from(
                            egui::RichText::new(&pill_label).small().color(p.text),
                        )
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Small,
                        );
                        let pill_w = (galley.size().x + 30.0).clamp(90.0, 200.0).ceil();
                        let (pill_rect, pill_resp) =
                            ui.allocate_exact_size(egui::vec2(pill_w, 22.0), egui::Sense::click());
                        if ui.is_rect_visible(pill_rect) {
                            let active_open = state.show_downloads_popover;
                            let fill = if active_open || pill_resp.hovered() {
                                p.bg_hover
                            } else {
                                p.bg_elev
                            };
                            let stroke = if active_open || dl_active_cnt > 0 {
                                Stroke::new(1.0_f32, p.border_strong)
                            } else {
                                Stroke::new(1.0_f32, p.border)
                            };
                            ui.painter().rect(
                                pill_rect,
                                CornerRadius::same(6),
                                fill,
                                stroke,
                                egui::StrokeKind::Inside,
                            );
                            if pct > 0.0 {
                                let bar_h = 2.0;
                                let bar_bg = egui::Rect::from_min_max(
                                    egui::pos2(pill_rect.left() + 4.0, pill_rect.bottom() - bar_h - 1.5),
                                    egui::pos2(pill_rect.right() - 4.0, pill_rect.bottom() - 1.5),
                                );
                                let bar_fill = egui::Rect::from_min_max(
                                    bar_bg.min,
                                    egui::pos2(
                                        bar_bg.left() + bar_bg.width() * pct,
                                        bar_bg.bottom(),
                                    ),
                                );
                                ui.painter().rect_filled(bar_bg, CornerRadius::same(1), p.border);
                                ui.painter().rect_filled(bar_fill, CornerRadius::same(1), bar_col);
                            }
                            draw_icon(
                                ui.painter(),
                                egui::Rect::from_center_size(
                                    egui::pos2(pill_rect.left() + 12.0, pill_rect.center().y - 1.0),
                                    egui::vec2(14.0, 14.0),
                                ),
                                Icon::Download,
                                if dl_active_cnt > 0 { p.accent_hot } else { p.muted },
                            );
                            ui.painter().galley(
                                egui::pos2(
                                    pill_rect.left() + 22.0,
                                    pill_rect.center().y - galley.size().y * 0.5 - 1.0,
                                ),
                                galley,
                                p.text,
                            );
                        }
                        if pill_resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if pill_resp
                            .on_hover_text("Toggle Model Downloads manager")
                            .clicked()
                        {
                            state.show_downloads_popover = !state.show_downloads_popover;
                            if state.show_downloads_popover {
                                state.show_status_popover = false;
                            }
                        }
                    }

                    // Remaining middle bar area: drag window + double-click maximize
                    let drag_w = ui.available_width().max(1.0);
                    let (_drag_rect, drag_resp) =
                        ui.allocate_exact_size(egui::vec2(drag_w, 24.0), egui::Sense::click_and_drag());
                    if drag_resp.drag_started_by(egui::PointerButton::Primary) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag_resp.double_clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                    }
                },
            );
        });
    });

    // Browser-style anchored Downloads Popover under top-right pill
    if state.show_downloads_popover && !state.show_settings_modal {
        egui::Area::new(egui::Id::new("cinder_downloads_popover_area"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, [-10.0, 34.0])
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.bg_card)
                    .stroke(Stroke::new(1.0_f32, p.border_strong))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(360.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Model Downloads")
                                    .strong()
                                    .color(p.text),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, p, Icon::Close, false, "Close").clicked() {
                                    state.show_downloads_popover = false;
                                }
                                if ghost_button(ui, p, egui::RichText::new("Open Forge").small())
                                    .clicked()
                                {
                                    state.switch_main_view(
                                        MainView::Forge(ForgeTab::Hub),
                                        true,
                                    );
                                    state.show_downloads_popover = false;
                                }
                            });
                        });
                        ui.add_space(6.0);
                        if snap.downloads.is_empty() {
                            ui.label(
                                egui::RichText::new("No active or recent downloads.")
                                    .small()
                                    .color(p.muted),
                            );
                        } else {
                            for (idx, dl) in snap.downloads.iter().take(6).enumerate() {
                                if idx > 0 {
                                    ui.add_space(6.0);
                                }
                                Frame::new()
                                    .fill(p.bg_elev)
                                    .stroke(Stroke::new(1.0_f32, p.border))
                                    .corner_radius(CornerRadius::same(7))
                                    .inner_margin(Margin::same(8))
                                    .show(ui, |ui| {
                                        let w = ui.available_width();
                                        ui.set_width(w);
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(w, 22.0),
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.spacing_mut().item_spacing.x = 4.0;
                                                if matches!(
                                                    dl.status,
                                                    harness_models::DownloadStatus::Downloading
                                                        | harness_models::DownloadStatus::Queued
                                                        | harness_models::DownloadStatus::Paused
                                                        | harness_models::DownloadStatus::Failed
                                                ) && icon_button(
                                                    ui,
                                                    p,
                                                    Icon::Close,
                                                    false,
                                                    "Cancel & delete partial file",
                                                )
                                                .clicked()
                                                {
                                                    backend.cancel_hf_download(dl.id.clone());
                                                }
                                                match dl.status {
                                                    harness_models::DownloadStatus::Downloading
                                                    | harness_models::DownloadStatus::Queued => {
                                                        if icon_button(
                                                            ui,
                                                            p,
                                                            Icon::Pause,
                                                            false,
                                                            "Pause download",
                                                        )
                                                        .clicked()
                                                        {
                                                            backend.pause_hf_download(dl.id.clone());
                                                        }
                                                    }
                                                    harness_models::DownloadStatus::Paused
                                                    | harness_models::DownloadStatus::Failed
                                                    | harness_models::DownloadStatus::Cancelled => {
                                                        if icon_button(
                                                            ui,
                                                            p,
                                                            Icon::Play,
                                                            false,
                                                            "Resume download",
                                                        )
                                                        .clicked()
                                                        {
                                                            backend.resume_hf_download(dl.id.clone());
                                                        }
                                                    }
                                                    harness_models::DownloadStatus::Verifying
                                                    | harness_models::DownloadStatus::Completed => {}
                                                }
                                                ui.with_layout(
                                                    egui::Layout::left_to_right(egui::Align::Center),
                                                    |ui| {
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(&dl.filename)
                                                                    .small()
                                                                    .strong()
                                                                    .color(p.text),
                                                            )
                                                            .truncate(),
                                                        );
                                                    },
                                                );
                                            },
                                        );
                                        ui.add_space(4.0);
                                        let pct = if dl.total_bytes > 0 {
                                            (dl.bytes_downloaded as f32 / dl.total_bytes as f32)
                                                .clamp(0.0, 1.0)
                                        } else {
                                            0.0
                                        };
                                        ui.add(egui::ProgressBar::new(pct));
                                        ui.add_space(3.0);
                                        let sub = match dl.status {
                                            harness_models::DownloadStatus::Downloading => {
                                                if dl.speed_mbps > 0.05 {
                                                    format!(
                                                        "{:.1} MB/s · {} / {}",
                                                        dl.speed_mbps,
                                                        fmt_bytes(dl.bytes_downloaded),
                                                        fmt_bytes(dl.total_bytes)
                                                    )
                                                } else {
                                                    format!(
                                                        "Downloading · {} / {}",
                                                        fmt_bytes(dl.bytes_downloaded),
                                                        fmt_bytes(dl.total_bytes)
                                                    )
                                                }
                                            }
                                            harness_models::DownloadStatus::Queued => {
                                                "Queued".into()
                                            }
                                            harness_models::DownloadStatus::Verifying => {
                                                "Verifying SHA-256...".into()
                                            }
                                            harness_models::DownloadStatus::Paused => format!(
                                                "Paused · {} / {}",
                                                fmt_bytes(dl.bytes_downloaded),
                                                fmt_bytes(dl.total_bytes)
                                            ),
                                            harness_models::DownloadStatus::Completed => format!(
                                                "Completed · {}",
                                                fmt_bytes(dl.bytes_downloaded)
                                            ),
                                            harness_models::DownloadStatus::Failed => {
                                                "Failed — click Play to resume".into()
                                            }
                                            harness_models::DownloadStatus::Cancelled => {
                                                "Cancelled".into()
                                            }
                                        };
                                        ui.label(
                                            egui::RichText::new(sub).small().color(p.muted),
                                        );
                                    });
                            }
                        }
                    });
            });
    }

    // Unified Telemetry / Forge / Downloads / Quota Popover anchored above bottom-left footer
    if state.show_status_popover && !state.show_settings_modal {
        egui::Area::new(egui::Id::new("cinder_status_popover_area"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -44.0])
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.bg_elev)
                    .stroke(Stroke::new(1.0_f32, p.border_strong))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(340.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("System & Forge Telemetry")
                                    .strong()
                                    .color(p.text),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, p, Icon::Close, false, "Close").clicked() {
                                    state.show_status_popover = false;
                                }
                            });
                        });
                        ui.separator();

                        // Privacy Mode
                        ui.horizontal(|ui| {
                            let priv_col = if is_hybrid { p.amber } else { p.green };
                            let priv_title = if is_hybrid {
                                "Hybrid (Cloud + Local)"
                            } else {
                                "Strict Local Mode"
                            };
                            ui.label(egui::RichText::new("●").color(priv_col));
                            ui.label(egui::RichText::new(priv_title).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let next_lbl = if is_hybrid { "Go Local" } else { "Allow Hybrid" };
                                if secondary_button(ui, p, next_lbl).clicked() {
                                    backend.set_privacy(if is_hybrid {
                                        PrivacyMode::StrictLocal
                                    } else {
                                        PrivacyMode::Hybrid
                                    });
                                }
                            });
                        });

                        ui.add_space(8.0);

                        // GPU VRAM & Forge Serving
                        let gpu = snap.hardware.gpus.first();
                        let vram_used = gpu.map(|g| g.vram_used_mb).unwrap_or(0);
                        let vram_tot = gpu.map(|g| g.vram_total_mb).unwrap_or(0).max(1);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(
                                    gpu.map(|g| g.name.as_str()).unwrap_or("GPU VRAM"),
                                )
                                .small()
                                .strong(),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, p, Icon::Refresh, false, "Probe GPU & VRAM")
                                    .clicked()
                                {
                                    backend.refresh_state(true);
                                }
                                ui.label(
                                    egui::RichText::new(format!("{vram_used}/{vram_tot} MB"))
                                        .small()
                                        .color(p.muted),
                                );
                            });
                        });
                        ui.add(egui::ProgressBar::new(vram_used as f32 / vram_tot as f32));

                        if srv_running {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Serving: {}",
                                        snap.server_status
                                            .model_name
                                            .as_deref()
                                            .unwrap_or("Local Model")
                                    ))
                                    .small()
                                    .color(p.accent_hot),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if danger_button(ui, p, "Quench").clicked() {
                                            backend.stop_all_local_models();
                                        }
                                    },
                                );
                            });
                        } else {
                            ui.label(
                                egui::RichText::new("Forge is cold — 0 resident local models.")
                                    .small()
                                    .color(p.muted),
                            );
                        }

                        ui.add_space(4.0);
                        let mut unload_after = snap.settings.ui.unload_after_turn;
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Auto-quench after reply")
                                    .small()
                                    .color(p.text),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if toggle_switch(ui, p, &mut unload_after).changed() {
                                    let mut next_ui = snap.settings.ui.clone();
                                    next_ui.unload_after_turn = unload_after;
                                    backend.save_ui_and_orch(Some(next_ui), None);
                                }
                            });
                        });

                        ui.separator();

                        // Engine & Downloads
                        let eng_lbl = if snap.engine_status.llama_server_installed {
                            "llama-server Ready"
                        } else {
                            "llama-server Not Installed"
                        };
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(eng_lbl).small().color(p.muted));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ghost_button(ui, p, egui::RichText::new("Forge Hub ->").small())
                                    .clicked()
                                {
                                    state.switch_main_view(MainView::Forge(ForgeTab::Vault), true);
                                    state.show_status_popover = false;
                                }
                                if !snap.engine_status.llama_server_installed
                                    && secondary_button(ui, p, "Install CUDA").clicked()
                                {
                                    backend.install_engine("cuda".into());
                                }
                            });
                        });

                        if dl_active_cnt > 0 || !snap.downloads.is_empty() {
                            for dl in snap.downloads.iter().take(2) {
                                let pct = if dl.total_bytes > 0 {
                                    (dl.bytes_downloaded as f32 / dl.total_bytes as f32)
                                        .clamp(0.0, 1.0)
                                } else {
                                    0.0
                                };
                                ui.add(egui::ProgressBar::new(pct).text(format!(
                                    "{} ({}/{})",
                                    dl.filename,
                                    fmt_bytes(dl.bytes_downloaded),
                                    fmt_bytes(dl.total_bytes),
                                )));
                                if matches!(dl.status, harness_models::DownloadStatus::Downloading)
                                    && secondary_button(ui, p, "Cancel").clicked()
                                {
                                    backend.cancel_hf_download(dl.id.clone());
                                }
                            }
                        }

                        ui.separator();

                        // Cloud Quotas & Privacy Audit links
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Cloud Quotas: {min_quota_pct}% daily"
                                ))
                                .small()
                                .color(p.muted),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ghost_button(ui, p, egui::RichText::new("Audit Log").small())
                                    .clicked()
                                {
                                    state.open_settings(SettingsTab::Privacy);
                                }
                                if ghost_button(ui, p, egui::RichText::new("Quotas").small())
                                    .clicked()
                                {
                                    state
                                        .switch_main_view(MainView::Forge(ForgeTab::Quotas), true);
                                    state.show_status_popover = false;
                                }
                            });
                        });
                    });
            });
    }
}

pub fn render_left_sidebar(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        ui.label("Loading...");
        return;
    };
    let active_sid = snap.settings.active_session_id.clone();
    let is_active_standalone = snap
        .active_session
        .as_ref()
        .map(|s| s.workspace_path.trim().is_empty())
        .unwrap_or(false);
    let is_hybrid = snap.privacy_audit.privacy_mode == PrivacyMode::Hybrid;
    let srv_running = snap.server_status.running;
    let dl_active = snap.downloads.iter().any(|d| {
        matches!(
            d.status,
            harness_models::DownloadStatus::Downloading
                | harness_models::DownloadStatus::Verifying
                | harness_models::DownloadStatus::Queued
        )
    });

    // Borderless top navigation bar: Collapse | Back | Forward | Search
    Frame::new()
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                if icon_button_enabled(
                    ui,
                    p,
                    Icon::Sidebar,
                    false,
                    true,
                    "Collapse Sidebar (Ctrl+B)",
                )
                .clicked()
                {
                    state.show_sidebar = false;
                }
                let can_back = state.nav_idx > 0;
                if icon_button_enabled(ui, p, Icon::ArrowLeft, false, can_back, "Back").clicked() {
                    state.navigate_history(-1, backend);
                }
                let can_fwd = (state.nav_idx + 1) < state.nav_history.len() as isize;
                if icon_button_enabled(ui, p, Icon::ArrowRight, false, can_fwd, "Forward")
                    .clicked()
                {
                    state.navigate_history(1, backend);
                }
                if icon_button_enabled(
                    ui,
                    p,
                    Icon::Search,
                    false,
                    true,
                    "Command Palette (Ctrl+K)",
                )
                .clicked()
                {
                    state.show_cmd_palette = true;
                }
            });
        });

    let footer_h = 36.0;

    Frame::new()
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            // Full-width elevated "+ New Conversation" pill button
            let (pill_rect, pill_resp) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
            if ui.is_rect_visible(pill_rect) {
                let fill = if pill_resp.hovered() {
                    p.bg_hover
                } else {
                    p.bg_card
                };
                ui.painter().rect(
                    pill_rect,
                    CornerRadius::same(7),
                    fill,
                    Stroke::new(1.0_f32, p.border),
                    egui::StrokeKind::Inside,
                );
                draw_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(
                        egui::pos2(pill_rect.left() + 14.0, pill_rect.center().y),
                        egui::vec2(13.0, 13.0),
                    ),
                    Icon::Plus,
                    p.text,
                );
                ui.painter().text(
                    egui::pos2(pill_rect.left() + 28.0, pill_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "New Conversation",
                    egui::FontId::proportional(p.base_font_size - 0.5),
                    p.text,
                );
            }
            if pill_resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if pill_resp.clicked() {
                if is_active_standalone {
                    backend.create_standalone_session(None);
                } else {
                    backend.create_session(None, None);
                }
                state.switch_main_view(MainView::Chat, true);
            }
            pill_resp.context_menu(|ui| {
                if ui.button("New One-Off Chat (No Project)").clicked() {
                    backend.create_standalone_session(None);
                    state.switch_main_view(MainView::Chat, true);
                    ui.close_menu();
                }
                if ui.button("New Chat in Active Project").clicked() {
                    backend.create_session(None, None);
                    state.switch_main_view(MainView::Chat, true);
                    ui.close_menu();
                }
            });

            ui.add_space(6.0);

            // Clean top navigation links matching minimal hierarchy
            if nav_row_with_icon(
                ui,
                p,
                Icon::Clock,
                state.main_view == MainView::Automations,
                "Automations",
            )
            .clicked()
            {
                state.switch_main_view(MainView::Automations, true);
            }
            if nav_row_with_icon(
                ui,
                p,
                Icon::Briefcase,
                matches!(state.main_view, MainView::Customizations(_)),
                "Customizations",
            )
            .clicked()
            {
                state.switch_main_view(MainView::Customizations(CustomTab::Plugins), true);
            }
            if nav_row_with_icon(
                ui,
                p,
                Icon::Flame,
                matches!(state.main_view, MainView::Forge(_)),
                "Forge & Models",
            )
            .clicked()
            {
                state.switch_main_view(MainView::Forge(ForgeTab::Vault), true);
            }

            ui.add_space(14.0);

            // Projects header with quiet Filter + New Folder icons
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if icon_button(
                        ui,
                        p,
                        Icon::NewFolder,
                        false,
                        "Open or Add Project Folder...",
                    )
                    .clicked()
                    {
                        backend.open_folder_picker(
                            FolderPickTarget::Workspace,
                            "Select Project Folder for Cinder",
                        );
                    }
                    if icon_button(
                        ui,
                        p,
                        Icon::Filter,
                        state.show_session_filter,
                        "Filter projects or chats",
                    )
                    .clicked()
                    {
                        state.show_session_filter = !state.show_session_filter;
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("Projects")
                                    .size(11.5)
                                    .color(p.muted),
                            )
                            .truncate(),
                        );
                    });
                });
            });

            if state.show_session_filter {
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.session_filter)
                            .hint_text("Filter projects or chats...")
                            .desired_width(f32::INFINITY)
                            .frame(false),
                    );
                });
                ui.add_space(4.0);
            }

            let filter_q = state.session_filter.trim().to_ascii_lowercase();
            let active_ws_norm = norm_path(&snap.settings.active_workspace_path);
            let known_ws_norms: HashSet<String> = snap
                .workspaces
                .iter()
                .map(|w| norm_path(&w.path))
                .collect();

            let scroll_h = (ui.available_height() - footer_h - 8.0).max(80.0);

            egui::ScrollArea::vertical()
                .id_salt("sidebar_projects_chats_scroll")
                .min_scrolled_height(scroll_h)
                .max_height(scroll_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut assigned = HashSet::new();

                    for ws in &snap.workspaces {
                        let ws_n = norm_path(&ws.path);
                        let is_active_ws = ws_n == active_ws_norm && !is_active_standalone;
                        let ws_sessions: Vec<_> = snap
                            .sessions
                            .iter()
                            .filter(|s| {
                                !s.workspace_path.trim().is_empty()
                                    && norm_path(&s.workspace_path) == ws_n
                            })
                            .filter(|s| {
                                filter_q.is_empty()
                                    || s.title.to_ascii_lowercase().contains(&filter_q)
                                    || ws.name.to_ascii_lowercase().contains(&filter_q)
                            })
                            .collect();
                        for s in &ws_sessions {
                            assigned.insert(s.id.clone());
                        }

                        let is_open =
                            state.expanded_projects.contains(&ws_n) || !filter_q.is_empty();
                        let (proj_resp, add_clicked) =
                            render_project_row(ui, p, is_active_ws, is_open, &ws.name);
                        if add_clicked {
                            state.expanded_projects.insert(ws_n.clone());
                            backend.create_session(None, Some(ws.path.clone()));
                            state.switch_main_view(MainView::Chat, true);
                        } else if proj_resp.clicked() {
                            if is_open {
                                state.expanded_projects.remove(&ws_n);
                            } else {
                                state.expanded_projects.insert(ws_n.clone());
                            }
                            backend.select_workspace(ws.path.clone());
                        }

                        let ws_path_clone = ws.path.clone();
                        let ws_name_clone = ws.name.clone();
                        let can_remove = snap.workspaces.len() > 1;
                        proj_resp.context_menu(|ui| {
                            if ui.button("New Chat in Project").clicked() {
                                state.expanded_projects.insert(ws_n.clone());
                                backend.create_session(None, Some(ws_path_clone.clone()));
                                state.switch_main_view(MainView::Chat, true);
                                ui.close_menu();
                            }
                            if ui.button("Set Active Project").clicked() {
                                backend.select_workspace(ws_path_clone.clone());
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Reveal in Explorer").clicked() {
                                backend.reveal_entry(PathBuf::from(&ws_path_clone));
                                ui.close_menu();
                            }
                            if ui.button("Copy Project Path").clicked() {
                                copy_to_clipboard(ui.ctx(), &ws_path_clone);
                                state.set_toast(format!("Copied {ws_name_clone} path"), false);
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    can_remove,
                                    egui::Button::new(
                                        egui::RichText::new("Remove Project").color(p.red),
                                    ),
                                )
                                .clicked()
                            {
                                backend.remove_workspace(ws_path_clone.clone());
                                ui.close_menu();
                            }
                        });

                        if is_open {
                            let show_all = state.expanded_project_all.contains(&ws_n)
                                || !filter_q.is_empty();
                            let cap = if show_all { ws_sessions.len() } else { 6 };
                            let visible_sessions = &ws_sessions[..ws_sessions.len().min(cap)];
                            let row_w = ui.available_width();
                            let child_w = (row_w - 18.0).max(40.0);

                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.add_space(18.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(child_w, 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        if visible_sessions.is_empty() {
                                            ui.label(
                                                egui::RichText::new("No chats yet")
                                                    .small()
                                                    .color(p.muted),
                                            );
                                        } else {
                                            for s in visible_sessions {
                                                render_session_row(
                                                    ui,
                                                    state,
                                                    backend,
                                                    p,
                                                    s,
                                                    &active_sid,
                                                    &snap.workspaces,
                                                );
                                            }
                                        }
                                        if ws_sessions.len() > 6 && filter_q.is_empty() {
                                            let more_lbl =
                                                if show_all { "See less" } else { "See more" };
                                            let (m_rect, m_resp) = ui.allocate_exact_size(
                                                egui::vec2(ui.available_width(), 22.0),
                                                egui::Sense::click(),
                                            );
                                            if ui.is_rect_visible(m_rect) {
                                                ui.painter().text(
                                                    egui::pos2(m_rect.left() + 9.0, m_rect.center().y),
                                                    egui::Align2::LEFT_CENTER,
                                                    more_lbl,
                                                    egui::FontId::proportional(12.0),
                                                    if m_resp.hovered() { p.text } else { p.muted },
                                                );
                                            }
                                            if m_resp.hovered() {
                                                ui.ctx().set_cursor_icon(
                                                    egui::CursorIcon::PointingHand,
                                                );
                                            }
                                            if m_resp.clicked() {
                                                if show_all {
                                                    state.expanded_project_all.remove(&ws_n);
                                                } else {
                                                    state.expanded_project_all.insert(ws_n.clone());
                                                }
                                            }
                                        }
                                    },
                                );
                            });
                        }
                    }

                    // Always-visible dedicated segment for standalone / one-off conversations
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(
                                ui,
                                p,
                                Icon::Plus,
                                false,
                                "New One-Off Conversation (Not in any Project)",
                            )
                            .clicked()
                            {
                                backend.create_standalone_session(None);
                                state.switch_main_view(MainView::Chat, true);
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add_space(4.0);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new("Conversations")
                                            .size(11.5)
                                            .color(p.muted),
                                    )
                                    .truncate(),
                                );
                            });
                        });
                    });

                    let standalone_sessions: Vec<_> = snap
                        .sessions
                        .iter()
                        .filter(|s| {
                            let standalone_ws = s.workspace_path.trim().is_empty()
                                || !known_ws_norms.contains(&norm_path(&s.workspace_path));
                            standalone_ws && !assigned.contains(&s.id)
                        })
                        .filter(|s| {
                            filter_q.is_empty()
                                || s.title.to_ascii_lowercase().contains(&filter_q)
                        })
                        .collect();

                    if standalone_sessions.is_empty() {
                        if ghost_button(
                            ui,
                            p,
                            egui::RichText::new("Start a one-off chat...")
                                .small()
                                .color(p.muted),
                        )
                        .clicked()
                        {
                            backend.create_standalone_session(None);
                            state.switch_main_view(MainView::Chat, true);
                        }
                    } else {
                        for s in standalone_sessions {
                            render_session_row(
                                ui,
                                state,
                                backend,
                                p,
                                s,
                                &active_sid,
                                &snap.workspaces,
                            );
                        }
                    }
                });
        });

    // Clean pinned bottom footer (no heavy hairline separator)
    let foot_rect = egui::Rect::from_min_size(
        egui::pos2(ui.max_rect().left(), ui.max_rect().bottom() - footer_h),
        egui::vec2(ui.max_rect().width(), footer_h),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(foot_rect.shrink2(egui::vec2(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            // Vector Gear + "Settings" clickable row
            let (s_rect, s_resp) =
                ui.allocate_exact_size(egui::vec2(92.0, 26.0), egui::Sense::click());
            if ui.is_rect_visible(s_rect) {
                if s_resp.hovered() {
                    ui.painter()
                        .rect_filled(s_rect, CornerRadius::same(6), p.bg_hover);
                }
                let col = if s_resp.hovered() { p.text } else { p.muted };
                draw_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(
                        egui::pos2(s_rect.left() + 13.0, s_rect.center().y),
                        egui::vec2(14.0, 14.0),
                    ),
                    Icon::Gear,
                    col,
                );
                ui.painter().text(
                    egui::pos2(s_rect.left() + 26.0, s_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Settings",
                    egui::FontId::proportional(p.base_font_size - 0.5),
                    col,
                );
            }
            if s_resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if s_resp.on_hover_text("Settings (Ctrl+,)").clicked() {
                state.open_settings(state.settings_tab);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dot_col = if is_hybrid { p.amber } else { p.green };
                let lbl = match (is_hybrid, srv_running || dl_active) {
                    (true, true) => "● Hybrid · Live",
                    (true, false) => "● Hybrid",
                    (false, true) => "● Local · Live",
                    (false, false) => "● Local",
                };
                let tip = if is_hybrid {
                    "Network Mode: Hybrid (Cloud + Local) — click to switch or view telemetry"
                } else {
                    "Network Mode: Strict Local (Offline Only) — click to switch or view telemetry"
                };
                if ghost_button(ui, p, egui::RichText::new(lbl).small().color(dot_col))
                    .on_hover_text(tip)
                    .clicked()
                {
                    state.show_status_popover = !state.show_status_popover;
                }
            });
        },
    );
}

fn render_project_row(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    is_active: bool,
    is_open: bool,
    name: &str,
) -> (egui::Response, bool) {
    let total_w = ui.available_width();
    let plus_w = 24.0;
    let main_w = (total_w - plus_w).max(40.0);

    let mut add_clicked = false;
    let res = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (main_rect, main_resp) =
            ui.allocate_exact_size(egui::vec2(main_w, 26.0), egui::Sense::click());
        let (plus_rect, plus_resp) =
            ui.allocate_exact_size(egui::vec2(plus_w, 26.0), egui::Sense::click());

        let row_rect = main_rect.union(plus_rect);
        let row_hovered = main_resp.hovered() || plus_resp.hovered();
        if ui.is_rect_visible(row_rect) {
            if row_hovered {
                ui.painter()
                    .rect_filled(row_rect, CornerRadius::same(7), p.bg_hover);
            }

            let fg = if is_active || row_hovered {
                p.text
            } else {
                p.muted
            };
            let folder_icon = if is_open {
                Icon::FolderOpen
            } else {
                Icon::Folder
            };
            draw_icon(
                ui.painter(),
                egui::Rect::from_center_size(
                    egui::pos2(main_rect.left() + 11.0, main_rect.center().y),
                    egui::vec2(13.5, 13.5),
                ),
                folder_icon,
                fg,
            );

            let mut rt = egui::RichText::new(name);
            if is_active {
                rt = rt.strong();
            }
            let title_galley = egui::WidgetText::from(rt).into_galley(
                ui,
                Some(egui::TextWrapMode::Truncate),
                (main_rect.width() - 28.0).max(20.0),
                egui::TextStyle::Button,
            );
            ui.painter().galley(
                egui::pos2(
                    main_rect.left() + 26.0,
                    main_rect.center().y - title_galley.size().y * 0.5,
                ),
                title_galley,
                fg,
            );

            if row_hovered {
                let plus_col = if plus_resp.hovered() {
                    p.accent_hot
                } else {
                    p.text
                };
                draw_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(plus_rect.center(), egui::vec2(12.0, 12.0)),
                    Icon::Plus,
                    plus_col,
                );
            }
        }

        if row_hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let plus_tipped = plus_resp.on_hover_text(format!("New Chat in {name}"));
        if plus_tipped.clicked() {
            add_clicked = true;
        }
        main_resp
    });
    (res.inner, add_clicked)
}

fn render_session_row(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
    s: &crate::backend::SessionSummary,
    active_sid: &str,
    workspaces: &[WorkspaceEntry],
) {
    let is_current = s.id == active_sid;
    let is_active_chat = is_current && state.main_view == MainView::Chat;
    let age = fmt_rel_age(s.updated_at);
    let (live_active, live_sid) = state
        .snap
        .as_ref()
        .map(|sn| (sn.live_activity.active, sn.live_activity.session_id.clone()))
        .unwrap_or((false, None));
    let is_working = (live_active && live_sid.as_deref() == Some(s.id.as_str()))
        || (state.is_generating && live_sid.as_deref().unwrap_or(active_sid) == s.id);

    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if is_active_chat {
            p.bg_card
        } else if is_working {
            p.bg_elev
        } else if resp.hovered() {
            p.bg_hover
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(7), fill);
        }
        if is_working {
            let pulse = ((ui.input(|i| i.time) as f32) * 3.4).sin() * 0.5 + 0.5;
            let a = (150.0 + pulse * 105.0) as u8;
            let tick_col =
                Color32::from_rgba_unmultiplied(p.accent_hot.r(), p.accent_hot.g(), p.accent_hot.b(), a);
            let tick = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 5.0),
                egui::vec2(2.0, rect.height() - 10.0),
            );
            ui.painter()
                .rect_filled(tick, CornerRadius::same(1), tick_col);
        }

        let age_galley =
            ui.painter()
                .layout_no_wrap(age, egui::FontId::proportional(11.0), p.muted);
        let right_reserve = if is_working {
            28.0
        } else {
            age_galley.size().x + 22.0
        };
        let max_title_w = (rect.width() - right_reserve).max(20.0);
        let fg = if is_active_chat || is_working || resp.hovered() {
            p.text
        } else {
            p.muted
        };
        let title_galley = egui::WidgetText::from(s.title.as_str()).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            max_title_w,
            egui::TextStyle::Button,
        );
        ui.painter().galley(
            egui::pos2(
                rect.left() + 9.0,
                rect.center().y - title_galley.size().y * 0.5,
            ),
            title_galley,
            fg,
        );
        if is_working {
            draw_working_indicator(
                ui,
                egui::pos2(rect.right() - 12.5, rect.center().y),
                4.6,
                p,
            );
        } else {
            ui.painter().galley(
                egui::pos2(
                    rect.right() - age_galley.size().x - 8.0,
                    rect.center().y - age_galley.size().y * 0.5,
                ),
                age_galley,
                p.muted,
            );
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        backend.select_session(s.id.clone());
        state.switch_main_view(MainView::Chat, true);
    }

    let sid = s.id.clone();
    let stitle = s.title.clone();
    let is_cur_standalone = s.workspace_path.trim().is_empty();
    let cur_ws_norm = norm_path(&s.workspace_path);
    resp.context_menu(|ui| {
        if ui.button("Open Conversation").clicked() {
            backend.select_session(sid.clone());
            state.switch_main_view(MainView::Chat, true);
            ui.close_menu();
        }
        if ui.button("Rename Conversation...").clicked() {
            state.prompt_modal = PromptModal::RenameSession {
                session_id: sid.clone(),
                title: stitle.clone(),
            };
            ui.close_menu();
        }
        ui.menu_button("Move to Project", |ui| {
            if ui
                .selectable_label(is_cur_standalone, "Standalone (No Project)")
                .clicked()
            {
                backend.move_session_to_workspace(sid.clone(), None);
                ui.close_menu();
            }
            if !workspaces.is_empty() {
                ui.separator();
            }
            for ws in workspaces {
                let selected = !is_cur_standalone && norm_path(&ws.path) == cur_ws_norm;
                if ui.selectable_label(selected, &ws.name).clicked() {
                    backend.move_session_to_workspace(sid.clone(), Some(ws.path.clone()));
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if ui
            .button(egui::RichText::new("Delete Conversation").color(p.red))
            .clicked()
        {
            backend.delete_session(sid.clone());
            ui.close_menu();
        }
    });
}
