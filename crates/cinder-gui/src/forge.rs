use crate::app_state::{fmt_bytes, ForgeTab, GuiState, MainView, SettingsTab};
use crate::backend::BackendHandle;
use crate::icons::{icon_button, Icon};
use crate::theme::{
    badge, danger_button, empty_state_block, ghost_button, input_frame, pane_header,
    primary_button, secondary_button, segmented_control, setting_group, underline_tab,
    CinderPalette,
};
use harness_models::{DownloadStatus, FitTier};
use harness_store::QuotaTier;
use std::sync::Arc;

pub fn render_forge_page(
    _ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
    tab: ForgeTab,
) {
    let Some(snap) = state.snap.clone() else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };

    // Exact 40px aligned pane header
    pane_header(ui, p, |ui| {
        ui.label(
            egui::RichText::new("Forge & Models")
                .strong()
                .color(p.text),
        );
        ui.add_space(10.0);

        for (target, lbl) in [
            (ForgeTab::Vault, "Local Vault"),
            (ForgeTab::Hub, "Hugging Face"),
            (ForgeTab::Quotas, "Cloud & Quotas"),
        ] {
            if underline_tab(ui, p, tab == target, lbl).clicked() {
                state.switch_main_view(MainView::Forge(target), true);
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, p, "← Chat").clicked() {
                state.switch_main_view(MainView::Chat, true);
            }
            let target_settings = if tab == ForgeTab::Quotas {
                SettingsTab::Models
            } else {
                SettingsTab::Engine
            };
            if icon_button(ui, p, Icon::Gear, false, "Open Forge & Provider Settings").clicked() {
                state.open_settings(target_settings);
            }
            if secondary_button(ui, p, "Rescan").clicked() {
                backend.refresh_state(true);
            }
        });
    });

    let avail_w = ui.available_width();
    let col_w = (avail_w - 48.0).clamp(360.0, 920.0);
    let side_pad = ((avail_w - col_w) * 0.5).max(24.0);

    egui::ScrollArea::vertical()
        .id_salt(("forge_page_scroll", format!("{tab:?}")))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(side_pad);
                ui.allocate_ui_with_layout(
                    egui::vec2(col_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        match tab {
                            ForgeTab::Vault => {
                                render_vault_tab(ui, state, backend, &snap, p);
                            }
                            ForgeTab::Hub => {
                                render_hub_tab(ui, state, backend, &snap, p);
                            }
                            ForgeTab::Quotas => {
                                render_quotas_tab(ui, state, backend, &snap, p);
                            }
                        }
                        ui.add_space(28.0);
                    },
                );
            });
        });
}

fn split_model_display_name(name: &str) -> (&str, Option<&str>) {
    if let Some((base, tag)) = name.split_once(" (") {
        let clean_tag = tag.trim_end_matches(')').trim();
        let short_tag = if clean_tag.eq_ignore_ascii_case("Ollama GGUF") {
            "Ollama"
        } else {
            clean_tag
        };
        (base.trim(), Some(short_tag))
    } else {
        (name, None)
    }
}

fn short_path_tail(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn fit_tier_color(tier: FitTier, p: &CinderPalette) -> egui::Color32 {
    match tier {
        FitTier::FitsInVram => p.green,
        FitTier::PartialOffload => p.amber,
        FitTier::CpuHeavy => p.red,
    }
}

fn fmt_compact_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn infer_gguf_quant(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    if lower.contains("mmproj") {
        return "mmproj".into();
    }
    let known = [
        "iq1_m", "iq1_s", "iq2_xxs", "iq2_xs", "iq2_s", "iq2_m", "iq3_xxs", "iq3_xs", "iq3_s",
        "iq3_m", "iq4_nl", "iq4_xs", "q2_k_l", "q2_k", "q3_k_xl", "q3_k_l", "q3_k_m", "q3_k_s",
        "q4_0", "q4_1", "q4_k_m", "q4_k_s", "q4_k_l", "q5_0", "q5_1", "q5_k_m", "q5_k_s",
        "q5_k_l", "q6_k", "q8_0", "bf16", "f16", "f32", "nvfp4", "mxfp4",
    ];
    for k in known {
        if lower.contains(k) {
            return k.to_ascii_uppercase();
        }
    }
    "GGUF".into()
}

fn square_tile_grid_layout(grid_w: f32) -> (usize, f32, f32) {
    let w = grid_w.floor().max(200.0);
    let gap = 12.0_f32;
    let cols = if w >= 820.0 {
        4
    } else if w >= 580.0 {
        3
    } else if w >= 380.0 {
        2
    } else {
        1
    };
    let raw_side = ((w - gap * ((cols - 1) as f32)) / (cols as f32)).floor();
    let tile_side = if cols == 1 {
        raw_side.clamp(175.0, 280.0)
    } else {
        raw_side.max(175.0)
    };
    (cols, tile_side, gap)
}

fn render_hub_subcard_row(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    subtitle: &str,
    add_top_bar: impl FnOnce(&mut egui::Ui),
) {
    p.subcard_frame().show(ui, |ui| {
        let w = ui.available_width();
        ui.set_width(w);
        ui.allocate_ui_with_layout(
            egui::vec2(w, 26.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                add_top_bar(ui);
            },
        );
        ui.add(
            egui::Label::new(egui::RichText::new(subtitle).small().color(p.muted)).truncate(),
        );
    });
}

fn render_left_title_with_badges(
    ui: &mut egui::Ui,
    avail_w: f32,
    height: f32,
    badge_reserve: f32,
    title: &str,
    hover_text: Option<&str>,
    mono: bool,
    p: &CinderPalette,
    add_badges: impl FnOnce(&mut egui::Ui),
) {
    let w = avail_w.max(40.0);
    let max_title_w = (w - badge_reserve).max(72.0);
    ui.allocate_ui_with_layout(
        egui::vec2(w, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let row_clip = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            ui.set_clip_rect(row_clip);
            let title_resp = ui
                .scope(|ui| {
                    ui.set_max_width(max_title_w);
                    let mut rt = egui::RichText::new(title).strong().color(p.text);
                    if mono {
                        rt = rt.monospace();
                    }
                    ui.add(egui::Label::new(rt).truncate())
                })
                .inner;
            if let Some(h) = hover_text {
                title_resp.on_hover_text(h);
            }
            add_badges(ui);
        },
    );
}

fn render_local_model_square_tile(
    ui: &mut egui::Ui,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
    m: &harness_models::LocalModelEntry,
    tile_side: f32,
    _vram_total_mb: u64,
) {
    let is_active = snap.settings.active_model_id == m.id;
    let is_serving = snap.server_status.running
        && snap
            .server_status
            .model_name
            .as_deref()
            .map(|n| n == m.name)
            .unwrap_or(false);
    let fit_col = fit_tier_color(m.vram_fit.tier, p);

    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(tile_side, tile_side), egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let stroke = if is_serving {
            egui::Stroke::new(1.0_f32, p.green)
        } else if is_active {
            egui::Stroke::new(1.0_f32, p.accent)
        } else if resp.hovered() {
            egui::Stroke::new(1.0_f32, p.border_strong)
        } else {
            egui::Stroke::new(1.0_f32, p.border)
        };
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(8),
            p.bg_card,
            stroke,
            egui::StrokeKind::Inside,
        );
    }

    let inner = rect.shrink(11.0);
    let bot_h = 26.0_f32;
    let divider_gap = 7.0_f32;
    let top_rect = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(
            inner.right(),
            (inner.bottom() - bot_h - divider_gap).max(inner.top() + 40.0),
        ),
    );
    let bot_rect = egui::Rect::from_min_max(
        egui::pos2(inner.left(), (inner.bottom() - bot_h).max(top_rect.bottom() + 2.0)),
        inner.max,
    );

    let (base_name, source_opt) = split_model_display_name(&m.name);

    let mut top_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(top_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    top_ui.set_clip_rect(top_rect.intersect(ui.clip_rect()));
    top_ui.set_width(top_rect.width());
    top_ui.set_max_width(top_rect.width());
    top_ui.spacing_mut().item_spacing.y = 5.0;

    // Row 1: Single-line left-aligned truncated title
    top_ui
        .add(egui::Label::new(egui::RichText::new(base_name).strong().color(p.text)).truncate())
        .on_hover_text(&m.name);

    // Row 2: Clipped horizontal badges (source/quant + embedding/live)
    top_ui.allocate_ui_with_layout(
        egui::vec2(top_rect.width(), 20.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_clip_rect(ui.available_rect_before_wrap().intersect(top_rect));
            ui.spacing_mut().item_spacing.x = 4.0;
            if is_serving {
                badge(ui, "Serving", p.green, p.bg_elev);
            } else if is_active && !m.is_embedding {
                badge(ui, "Active", p.accent_hot, p.bg_elev);
            }
            if let Some(src) = source_opt {
                badge(ui, src, p.muted, p.bg_elev);
            } else {
                badge(ui, &m.quant, p.muted, p.bg_elev);
            }
            if m.is_embedding {
                badge(ui, "Embedding", p.purple, p.bg_elev);
            }
        },
    );

    // Row 3: Consistent VRAM fit badge matching app-wide card pills
    top_ui
        .allocate_ui_with_layout(
            egui::vec2(top_rect.width(), 20.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_clip_rect(ui.available_rect_before_wrap().intersect(top_rect));
                badge(ui, &m.vram_fit.label, fit_col, p.bg_elev);
            },
        )
        .response
        .on_hover_text(&m.vram_fit.label);

    // Row 4: Size & memory footprint (left-aligned, single-line truncated)
    top_ui.add(
        egui::Label::new(
            egui::RichText::new(format!(
                "{} · est {} MB",
                fmt_bytes(m.size_bytes),
                m.vram_fit.estimated_total_mb
            ))
            .small()
            .color(p.muted),
        )
        .truncate(),
    );

    // Row 5: Layers & file basename (left-aligned, single-line truncated, full path on hover)
    let tail = short_path_tail(&m.gguf_path);
    top_ui
        .add(
            egui::Label::new(
                egui::RichText::new(format!("NGL {} · {}", m.vram_fit.recommended_ngl, tail))
                    .small()
                    .color(p.muted),
            )
            .truncate(),
        )
        .on_hover_text(&m.gguf_path);

    // Divider above pinned bottom action bar
    if ui.is_rect_visible(bot_rect) {
        let div_y = bot_rect.top() - 4.0;
        ui.painter().line_segment(
            [
                egui::pos2(inner.left(), div_y),
                egui::pos2(inner.right(), div_y),
            ],
            egui::Stroke::new(1.0_f32, p.border),
        );
    }

    let mut bot_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bot_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bot_ui.set_clip_rect(bot_rect.intersect(ui.clip_rect()));
    bot_ui.set_width(bot_rect.width());

    if m.is_embedding {
        bot_ui.add(
            egui::Label::new(
                egui::RichText::new("RAG / Embedding model")
                    .small()
                    .color(p.muted),
            )
            .truncate(),
        );
    } else {
        bot_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let serve_lbl = if tile_side < 192.0 {
                "Serve"
            } else {
                "Serve & Use"
            };
            if primary_button(ui, p, serve_lbl).clicked() {
                backend.start_local_model(m.id.clone());
            }
            let use_lbl = if is_active { "Active" } else { "Use" };
            let btn_res = if is_active {
                ghost_button(
                    ui,
                    p,
                    egui::RichText::new(use_lbl).color(p.accent_hot),
                )
            } else {
                secondary_button(ui, p, use_lbl)
            };
            if btn_res.clicked() {
                let id = m.id.clone();
                backend.update_settings(move |s| {
                    s.active_model_id = id;
                });
            }
        });
    }
}

fn render_vault_tab(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    let vram_total_mb = snap
        .hardware
        .gpus
        .first()
        .map(|g| g.vram_total_mb)
        .unwrap_or(0)
        .max(1);

    // 1. GPU & VRAM Status
    setting_group(
        ui,
        p,
        "GPU & VRAM Status",
        "Live hardware memory residency and local GGUF serving state.",
        |ui| {
            let gpu = snap.hardware.gpus.first();
            let vram_used = gpu.map(|g| g.vram_used_mb).unwrap_or(0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(
                        gpu.map(|g| g.name.as_str()).unwrap_or("No discrete GPU detected"),
                    )
                    .strong()
                    .color(p.text),
                );
                if snap.engine_status.ollama_loopback_detected {
                    badge(ui, "Ollama Detected", p.green, p.bg_card);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if snap.server_status.running {
                        if danger_button(ui, p, "Quench & Free VRAM").clicked() {
                            backend.stop_all_local_models();
                        }
                        badge(
                            ui,
                            &format!(
                                "Serving {}",
                                snap.server_status
                                    .model_name
                                    .as_deref()
                                    .unwrap_or("Local")
                            ),
                            p.green,
                            p.bg_card,
                        );
                    } else {
                        badge(ui, "COLD", p.muted, p.bg_card);
                    }
                });
            });
            ui.add_space(6.0);
            ui.add(
                egui::ProgressBar::new((vram_used as f32 / vram_total_mb as f32).clamp(0.0, 1.0))
                    .text(format!("{vram_used} / {vram_total_mb} MB VRAM")),
            );
        },
    );

    ui.add_space(16.0);

    // 2. Installed Local Models (Square Tiles arranged in responsive rows)
    setting_group(
        ui,
        p,
        "Installed Local Models",
        "Strict Local inference with zero outbound network calls.",
        |ui| {
            if snap.local_models.is_empty() {
                if empty_state_block(
                    ui,
                    p,
                    Icon::Cpu,
                    "No local GGUF or Ollama models found",
                    "Browse Hugging Face to download models or configure your Model Vault directory in Settings.",
                    Some("Browse Hugging Face"),
                ) {
                    state.switch_main_view(MainView::Forge(ForgeTab::Hub), true);
                }
            } else {
                let (cols, tile_side, gap) = square_tile_grid_layout(ui.available_width());

                for (row_idx, row_models) in snap.local_models.chunks(cols).enumerate() {
                    if row_idx > 0 {
                        ui.add_space(gap);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for m in row_models {
                            render_local_model_square_tile(
                                ui,
                                backend,
                                snap,
                                p,
                                m,
                                tile_side,
                                vram_total_mb,
                            );
                        }
                    });
                }
            }
        },
    );

    ui.add_space(16.0);

    // 3. Quiet Vault & Runtime Summary Bar (single-source-of-truth pointer to Settings)
    p.card_frame().show(ui, |ui| {
        let w = ui.available_width();
        ui.set_width(w);
        ui.allocate_ui_with_layout(
            egui::vec2(w, 38.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if secondary_button(ui, p, "Configure Vault & llama-server in Settings →")
                    .clicked()
                {
                    state.open_settings(SettingsTab::Engine);
                }
                let left_w = ui.available_width().max(80.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(left_w, 38.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_clip_rect(
                            ui.available_rect_before_wrap().intersect(ui.clip_rect()),
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "Model Vault: {}",
                                    snap.settings.model_vault_dir
                                ))
                                .small()
                                .color(p.text),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&snap.settings.model_vault_dir);
                        let eng = if snap.engine_status.llama_server_installed {
                            "llama-server: Ready"
                        } else {
                            "llama-server: Not Installed"
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "{eng} · Ctx {} · NGL {}",
                                    snap.settings.local_ctx_size, snap.settings.local_gpu_layers
                                ))
                                .small()
                                .color(p.muted),
                            )
                            .truncate(),
                        );
                    },
                );
            },
        );
    });
}

fn render_hub_tab(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    if snap.settings.privacy_mode == harness_net::PrivacyMode::StrictLocal {
        p.card_frame().show(ui, |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            ui.allocate_ui_with_layout(
                egui::vec2(w, 26.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if primary_button(ui, p, "Switch to Hybrid Mode").clicked() {
                        backend.set_privacy(harness_net::PrivacyMode::Hybrid);
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width().max(40.0), 22.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_clip_rect(
                                ui.available_rect_before_wrap().intersect(ui.clip_rect()),
                            );
                            badge(ui, "Strict Local", p.green, p.bg_card);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        "Search & downloads automatically switch to Hybrid Mode when requested.",
                                    )
                                    .small()
                                    .color(p.text),
                                )
                                .truncate(),
                            );
                        },
                    );
                },
            );
        });
        ui.add_space(12.0);
    }

    if !snap.downloads.is_empty() {
        let any_active_dl = snap
            .downloads
            .iter()
            .any(|d| matches!(d.status, DownloadStatus::Downloading | DownloadStatus::Queued));
        if any_active_dl {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        setting_group(
            ui,
            p,
            "Active & Recent Downloads",
            "SHA-256 verified GGUF downloads staged in fixed slots directly into your Model Vault.",
            |ui| {
                for (idx, dl) in snap.downloads.iter().take(8).enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    p.subcard_frame().show(ui, |ui| {
                        let w = ui.available_width();
                        ui.set_width(w);
                        ui.allocate_ui_with_layout(
                            egui::vec2(w, 26.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if matches!(
                                    dl.status,
                                    DownloadStatus::Downloading
                                        | DownloadStatus::Queued
                                        | DownloadStatus::Paused
                                        | DownloadStatus::Failed
                                ) && danger_button(ui, p, "Cancel").clicked()
                                {
                                    backend.cancel_hf_download(dl.id.clone());
                                }
                                match dl.status {
                                    DownloadStatus::Downloading | DownloadStatus::Queued => {
                                        if secondary_button(ui, p, "Pause").clicked() {
                                            backend.pause_hf_download(dl.id.clone());
                                        }
                                    }
                                    DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Cancelled => {
                                        if primary_button(ui, p, "Resume").clicked() {
                                            backend.resume_hf_download(dl.id.clone());
                                        }
                                    }
                                    DownloadStatus::Verifying | DownloadStatus::Completed => {}
                                }
                                let meta = if matches!(dl.status, DownloadStatus::Downloading)
                                    && dl.speed_mbps > 0.05
                                {
                                    format!(
                                        "{:.1} MB/s · {} / {}",
                                        dl.speed_mbps,
                                        fmt_bytes(dl.bytes_downloaded),
                                        fmt_bytes(dl.total_bytes)
                                    )
                                } else {
                                    format!(
                                        "{} / {}",
                                        fmt_bytes(dl.bytes_downloaded),
                                        fmt_bytes(dl.total_bytes)
                                    )
                                };
                                ui.label(egui::RichText::new(meta).small().color(p.muted));
                                render_left_title_with_badges(
                                    ui,
                                    ui.available_width(),
                                    22.0,
                                    95.0,
                                    &dl.filename,
                                    None,
                                    true,
                                    p,
                                    |ui| {
                                        let (st_label, st_col) = match dl.status {
                                            DownloadStatus::Queued => ("Queued", p.muted),
                                            DownloadStatus::Downloading => {
                                                ("Downloading", p.accent_hot)
                                            }
                                            DownloadStatus::Verifying => ("Verifying", p.accent),
                                            DownloadStatus::Paused => ("Paused", p.amber),
                                            DownloadStatus::Completed => ("Completed", p.green),
                                            DownloadStatus::Failed => ("Failed", p.red),
                                            DownloadStatus::Cancelled => ("Cancelled", p.muted),
                                        };
                                        badge(ui, st_label, st_col, p.bg_elev);
                                    },
                                );
                            },
                        );
                        ui.add_space(4.0);
                        let pct = if dl.total_bytes > 0 {
                            (dl.bytes_downloaded as f32 / dl.total_bytes as f32).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        ui.add(egui::ProgressBar::new(pct));
                        if matches!(dl.status, DownloadStatus::Failed) {
                            if let Some(ref err) = dl.error {
                                if !err.trim().is_empty() {
                                    ui.add_space(3.0);
                                    ui.label(
                                        egui::RichText::new(err.trim()).small().color(p.red),
                                    );
                                }
                            }
                        }
                    });
                }
            },
        );
        ui.add_space(16.0);
    }

    // Search Header Card
    setting_group(
        ui,
        p,
        "Hugging Face GGUF Hub",
        "Search any public GGUF repository live on Hugging Face and check VRAM fit before downloading.",
        |ui| {
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let mut enter_search = false;
            let has_clear = !state.hf_results.is_empty() || state.hf_repo_details.is_some();
            let w = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(w, 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if has_clear && ghost_button(ui, p, "Clear").clicked() {
                        state.hf_results.clear();
                        state.hf_repo_details = None;
                    }
                    let clicked_search = primary_button(ui, p, "Search HF").clicked();
                    let input_w = (ui.available_width() - 24.0).max(100.0);
                    input_frame(p, false).show(ui, |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut state.hf_query)
                                .hint_text(
                                    "Search Hugging Face GGUFs (e.g. Qwen3-Coder, DeepSeek)...",
                                )
                                .desired_width(input_w)
                                .frame(false),
                        );
                        if r.lost_focus() && enter {
                            enter_search = true;
                        }
                    });
                    if enter_search || clicked_search {
                        backend.search_hf(state.hf_query.clone());
                    }
                },
            );
        },
    );

    // Inspected Repository Files (Prominent structured subcards right below Search)
    if let Some(det) = state.hf_repo_details.clone() {
        ui.add_space(16.0);
        setting_group(
            ui,
            p,
            &format!("Files in {}", det.repo_id),
            "Estimated GPU offload and memory footprint for your active hardware profile.",
            |ui| {
                let w = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(w, 24.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ghost_button(ui, p, "Close Inspected Repo").clicked() {
                            state.hf_repo_details = None;
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            badge(
                                ui,
                                &format!("{} GGUF files shown", det.files.len().min(16)),
                                p.accent_hot,
                                p.bg_card,
                            );
                        });
                    },
                );
                ui.add_space(6.0);
                for (idx, f) in det.files.iter().take(16).enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    let fit_col = fit_tier_color(f.vram_fit.tier, p);
                    let short_name = short_path_tail(&f.filename);
                    let quant = infer_gguf_quant(short_name);
                    let is_downloading = snap.downloads.iter().any(|d| {
                        d.filename == f.filename
                            && matches!(
                                d.status,
                                DownloadStatus::Downloading | DownloadStatus::Queued
                            )
                    });
                    let subtitle = format!(
                        "{} · est {} MB total · NGL {} · {}/{}",
                        fmt_bytes(f.size_bytes),
                        f.vram_fit.estimated_total_mb,
                        f.vram_fit.recommended_ngl,
                        det.repo_id,
                        f.filename
                    );
                    render_hub_subcard_row(ui, p, &subtitle, |ui| {
                        if is_downloading {
                            badge(ui, "Downloading", p.amber, p.bg_elev);
                        } else if primary_button(ui, p, "Download").clicked() {
                            backend.download_hf(
                                det.repo_id.clone(),
                                f.filename.clone(),
                                f.sha256.clone(),
                                Some(f.size_bytes),
                            );
                        }
                        render_left_title_with_badges(
                            ui,
                            ui.available_width(),
                            22.0,
                            260.0,
                            short_name,
                            Some(&f.filename),
                            false,
                            p,
                            |ui| {
                                badge(ui, &quant, p.muted, p.bg_elev);
                                badge(ui, &f.vram_fit.label, fit_col, p.bg_elev);
                            },
                        );
                    });
                }
            },
        );
    }

    // Hugging Face Search Results as 2-line structured subcards
    if !state.hf_results.is_empty() {
        ui.add_space(16.0);
        setting_group(
            ui,
            p,
            "Hugging Face Search Results",
            "Inspect repository files to check per-quantization VRAM fit before downloading.",
            |ui| {
                let inspected_repo = state
                    .hf_repo_details
                    .as_ref()
                    .map(|d| d.repo_id.as_str())
                    .unwrap_or("");
                for (idx, res) in state.hf_results.iter().enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    let is_inspected = inspected_repo == res.repo_id;
                    let subtitle = format!(
                        "Public Hugging Face repository · {} downloads · {} likes · Click Inspect Files to check quantizations",
                        res.downloads, res.likes
                    );
                    render_hub_subcard_row(ui, p, &subtitle, |ui| {
                        let btn_lbl = if is_inspected {
                            "Inspecting"
                        } else {
                            "Inspect Files"
                        };
                        let clicked = if is_inspected {
                            primary_button(ui, p, btn_lbl).clicked()
                        } else {
                            secondary_button(ui, p, btn_lbl).clicked()
                        };
                        if clicked {
                            backend.inspect_hf_repo(res.repo_id.clone());
                        }
                        render_left_title_with_badges(
                            ui,
                            ui.available_width(),
                            22.0,
                            225.0,
                            &res.repo_id,
                            None,
                            false,
                            p,
                            |ui| {
                                badge(ui, "GGUF Repo", p.muted, p.bg_elev);
                                badge(
                                    ui,
                                    &format!("↓ {}", fmt_compact_count(res.downloads)),
                                    p.green,
                                    p.bg_elev,
                                );
                                badge(
                                    ui,
                                    &format!("★ {}", fmt_compact_count(res.likes)),
                                    p.amber,
                                    p.bg_elev,
                                );
                            },
                        );
                    });
                }
            },
        );
    }

    if state.hf_results.is_empty() && state.hf_repo_details.is_none() {
        ui.add_space(16.0);
        if empty_state_block(
            ui,
            p,
            Icon::Search,
            "Search Hugging Face GGUF Hub",
            "Enter a model name above (e.g. Qwen3, Gemma, DeepSeek) or browse live popular GGUF repositories.",
            Some("Browse Popular GGUFs"),
        ) {
            backend.search_hf(String::new());
        }
    }
}

fn render_quotas_tab(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    setting_group(
        ui,
        p,
        "Quota Tier & Reachability",
        "Monitor provider reachability and daily quota limits across configured cloud models.",
        |ui| {
            ui.horizontal(|ui| {
                if let Some(next_tier) = segmented_control(
                    ui,
                    p,
                    "forge_quota_tier_seg",
                    snap.settings.quota_tier,
                    &[
                        (QuotaTier::GoogleAiPro, "Google AI Pro"),
                        (QuotaTier::FreeTier, "Free Tier"),
                        (QuotaTier::Custom, "Custom"),
                    ],
                ) {
                    backend.set_quota_tier(next_tier);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Probe Google API").clicked() {
                        backend.sync_google();
                    }
                    if secondary_button(ui, p, "Manage API Keys & Auto-Switch in Settings →")
                        .clicked()
                    {
                        state.open_settings(SettingsTab::Models);
                    }
                });
            });

            if !state.verify_statuses.is_empty() {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for st in &state.verify_statuses {
                        let col = if st.verified {
                            p.green
                        } else if st.configured {
                            p.amber
                        } else {
                            p.muted
                        };
                        badge(
                            ui,
                            &format!("{}: {}", st.provider, st.message),
                            col,
                            p.bg_card,
                        );
                    }
                });
            }
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Cloud Model Quotas",
        "Estimated remaining daily capacity and request rate limits.",
        |ui| {
            if snap.cloud_quotas.is_empty() {
                empty_state_block(
                    ui,
                    p,
                    Icon::Key,
                    "No cloud quotas tracked yet",
                    "Configure API credentials in Settings -> Models & Keys to track live quotas.",
                    Some("Open Models & Keys Settings"),
                )
                .then(|| state.open_settings(SettingsTab::Models));
            } else {
                for (idx, q) in snap.cloud_quotas.iter().enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    p.subcard_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&q.display_name).strong().color(p.text));
                            if q.reachable {
                                badge(ui, "Reachable", p.green, p.bg_elev);
                            } else {
                                badge(ui, &q.reachability_reason, p.muted, p.bg_elev);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{:.0}% remaining",
                                            q.remaining_daily_pct
                                        ))
                                        .small()
                                        .color(p.text),
                                    );
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.add(egui::ProgressBar::new(
                            (q.remaining_daily_pct as f32 / 100.0).clamp(0.0, 1.0),
                        ));
                        ui.label(
                            egui::RichText::new(format!(
                                "Today: {} reqs · {} / {} tokens · RPM {}/{}",
                                q.rpd_used,
                                q.tokens_today,
                                q.daily_token_limit,
                                q.rpm_used,
                                q.rpm_limit
                            ))
                            .small()
                            .color(p.muted),
                        );
                    });
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_tile_grid_layout_keeps_squares_across_resize_widths() {
        for (w, expected_cols) in [(350.0, 1), (460.0, 2), (660.0, 3), (920.0, 4)] {
            let (cols, side, gap) = square_tile_grid_layout(w);
            assert_eq!(cols, expected_cols, "width={w}");
            assert!(side >= 175.0, "side={side} too small at w={w}");
            let total_row_w = (cols as f32) * side + ((cols - 1) as f32) * gap;
            assert!(
                total_row_w <= w + 0.5,
                "row overflow: total_row_w={total_row_w} > w={w}"
            );
        }
    }

    #[test]
    fn forge_helpers_extract_quant_name_and_counts() {
        assert_eq!(
            split_model_display_name("gemma4:12b (Ollama GGUF)"),
            ("gemma4:12b", Some("Ollama"))
        );
        assert_eq!(
            infer_gguf_quant("Huihui-Qwen3.6-35B-ggml-model-Q4_K_M.gguf"),
            "Q4_K_M"
        );
        assert_eq!(infer_gguf_quant("mmproj-model-f16.gguf"), "mmproj");
        assert_eq!(fmt_compact_count(9_842_687), "9.8M");
        assert_eq!(fmt_compact_count(12_400), "12.4k");
        assert_eq!(fmt_compact_count(338), "338");
    }
}
