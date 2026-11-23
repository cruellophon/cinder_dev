use crate::app_state::{
    fmt_bytes, ForgeTab, GuiState, MainView, QuotasTabFilter, SettingsTab,
};
use crate::backend::{BackendHandle, KeysPatch};
use crate::icons::{icon_button, Icon};
use crate::theme::{
    badge, danger_button, empty_state_block, ghost_button, input_frame, pane_header,
    primary_button, secondary_button, segmented_control, setting_group, underline_tab,
    CinderPalette,
};
use harness_models::{DownloadStatus, FitTier, VramFitAssessment};
use harness_store::{ModelQuotaSnapshot, ProviderKind, QuotaTier};
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
    let col_w = (avail_w - 48.0).clamp(360.0, 1140.0);
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

pub(crate) fn concise_vram_badge(
    vram_fit: &VramFitAssessment,
    p: &CinderPalette,
) -> (String, egui::Color32) {
    match vram_fit.tier {
        FitTier::FitsInVram => ("100% GPU".to_string(), p.green),
        FitTier::PartialOffload => (
            format!("Hybrid GPU · {}L", vram_fit.recommended_ngl),
            p.amber,
        ),
        FitTier::CpuHeavy => ("CPU Heavy".to_string(), p.red),
    }
}

pub(crate) fn model_tile_grid_layout(grid_w: f32) -> (usize, f32, f32, f32) {
    let w = grid_w.floor().max(240.0);
    let gap = 12.0_f32;
    let min_tile_w = 280.0_f32;
    let cols = (((w + gap) / (min_tile_w + gap)).floor() as usize).clamp(1, 3);
    let tile_w = ((w - gap * ((cols - 1) as f32)) / (cols as f32))
        .floor()
        .max(220.0);
    let tile_h = 192.0_f32;
    (cols, tile_w, tile_h, gap)
}

#[derive(Debug, Clone)]
struct ModelTileBadge {
    label: String,
    color: egui::Color32,
    tooltip: Option<String>,
}

#[derive(Debug, Clone)]
struct ModelTileProgress {
    fraction: f32,
    color: egui::Color32,
    right_label: String,
}

#[derive(Debug, Clone)]
struct ModelTileSpec {
    title: String,
    title_tooltip: String,
    subtitle: Option<String>,
    badges: Vec<ModelTileBadge>,
    progress: Option<ModelTileProgress>,
    meta_lines: Vec<(String, String)>,
    border_highlight: Option<egui::Color32>,
}

fn render_model_tile(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    tile_w: f32,
    tile_h: f32,
    spec: &ModelTileSpec,
    add_actions: impl FnOnce(&mut egui::Ui),
) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(tile_w, tile_h), egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let stroke = if let Some(hl) = spec.border_highlight {
            egui::Stroke::new(1.2_f32, hl)
        } else if resp.hovered() {
            egui::Stroke::new(1.0_f32, p.border_strong)
        } else {
            egui::Stroke::new(1.0_f32, p.border)
        };
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(9),
            p.bg_card,
            stroke,
            egui::StrokeKind::Inside,
        );
    }

    let inner = rect.shrink(12.0);
    let bot_h = 26.0_f32;
    let divider_gap = 7.0_f32;
    let top_rect = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(
            inner.right(),
            (inner.bottom() - bot_h - divider_gap).max(inner.top() + 44.0),
        ),
    );
    let bot_rect = egui::Rect::from_min_max(
        egui::pos2(inner.left(), (inner.bottom() - bot_h).max(top_rect.bottom() + 2.0)),
        inner.max,
    );

    let mut top_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(top_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    top_ui.set_clip_rect(top_rect.intersect(ui.clip_rect()));
    top_ui.set_width(top_rect.width());
    top_ui.set_max_width(top_rect.width());
    top_ui.spacing_mut().item_spacing.y = 5.0;

    // 1. Single-line title (with hover tooltip)
    let title_resp = top_ui.add(
        egui::Label::new(egui::RichText::new(&spec.title).strong().color(p.text)).truncate(),
    );
    if !spec.title_tooltip.is_empty() {
        title_resp.on_hover_text(&spec.title_tooltip);
    }

    // 2. Optional muted subtitle (alias canonical name, HF author, or model ID)
    if let Some(ref sub) = spec.subtitle {
        if !sub.trim().is_empty() {
            top_ui
                .add(
                    egui::Label::new(egui::RichText::new(sub).small().color(p.muted)).truncate(),
                )
                .on_hover_text(sub);
        }
    }

    // 3. Non-clipping wrapped badges row
    if !spec.badges.is_empty() {
        top_ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for b in &spec.badges {
                let r = ui.scope(|ui| badge(ui, &b.label, b.color, p.bg_elev)).response;
                if let Some(ref tip) = b.tooltip {
                    r.on_hover_text(tip);
                }
            }
        });
    }

    // 4. Optional sleek quota / capacity progress bar
    if let Some(ref prog) = spec.progress {
        top_ui.horizontal(|ui| {
            let lbl_w = 64.0_f32;
            let bar_w = (top_rect.width() - lbl_w - 8.0).max(50.0);
            let (bar_r, _) =
                ui.allocate_exact_size(egui::vec2(bar_w, 6.0), egui::Sense::hover());
            if ui.is_rect_visible(bar_r) {
                ui.painter()
                    .rect_filled(bar_r, egui::CornerRadius::same(3), p.bg_sidebar);
                let frac = prog.fraction.clamp(0.0, 1.0);
                if frac > 0.005 {
                    let fill_r = egui::Rect::from_min_size(
                        bar_r.min,
                        egui::vec2(bar_r.width() * frac, bar_r.height()),
                    );
                    ui.painter()
                        .rect_filled(fill_r, egui::CornerRadius::same(3), prog.color);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(&prog.right_label)
                        .small()
                        .strong()
                        .color(prog.color),
                );
            });
        });
    }

    // 5. Compact metadata lines
    for (line, tip) in spec.meta_lines.iter().take(2) {
        if !line.trim().is_empty() {
            let r = top_ui.add(
                egui::Label::new(egui::RichText::new(line).small().color(p.muted)).truncate(),
            );
            if !tip.trim().is_empty() {
                r.on_hover_text(tip);
            }
        }
    }

    // Hairline divider above pinned action bar
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
    add_actions(&mut bot_ui);
}

fn provider_pill_style(provider: ProviderKind, p: &CinderPalette) -> (&'static str, egui::Color32) {
    match provider {
        ProviderKind::GoogleGemini => ("Gemini", p.green),
        ProviderKind::Anthropic => ("Claude", p.amber),
        ProviderKind::OpenRouter => ("OpenRouter", p.purple),
        ProviderKind::OpenAiCompat => ("Custom API", p.accent_hot),
        ProviderKind::LocalLlama | ProviderKind::LocalOllama => ("Local", p.accent_hot),
    }
}

pub(crate) fn partition_quotas_tab(
    quotas: &[ModelQuotaSnapshot],
) -> (
    Vec<&ModelQuotaSnapshot>,
    Vec<&ModelQuotaSnapshot>,
    Vec<&ModelQuotaSnapshot>,
) {
    let mut avail = Vec::new();
    let mut paid = Vec::new();
    let mut unavail = Vec::new();
    for q in quotas {
        if q.requires_payment {
            paid.push(q);
        } else if q.reachable {
            avail.push(q);
        } else {
            unavail.push(q);
        }
    }
    (avail, paid, unavail)
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

fn render_local_model_tile(
    ui: &mut egui::Ui,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
    m: &harness_models::LocalModelEntry,
    tile_w: f32,
    tile_h: f32,
) {
    let is_active = snap.settings.active_model_id == m.id;
    let is_serving = snap.server_status.running
        && snap
            .server_status
            .model_name
            .as_deref()
            .map(|n| n == m.name)
            .unwrap_or(false);

    let (base_name, source_opt) = split_model_display_name(&m.name);
    let display_title = snap
        .settings
        .resolve_model_alias(&m.id, base_name)
        .to_string();
    let subtitle = if display_title != base_name {
        Some(base_name.to_string())
    } else {
        None
    };

    let mut badges = Vec::new();
    if is_serving {
        badges.push(ModelTileBadge {
            label: "Serving".into(),
            color: p.green,
            tooltip: None,
        });
    } else if is_active && !m.is_embedding {
        badges.push(ModelTileBadge {
            label: "Active".into(),
            color: p.accent_hot,
            tooltip: None,
        });
    }
    if let Some(src) = source_opt {
        badges.push(ModelTileBadge {
            label: src.to_string(),
            color: p.muted,
            tooltip: None,
        });
    } else {
        badges.push(ModelTileBadge {
            label: m.quant.clone(),
            color: p.muted,
            tooltip: None,
        });
    }
    if m.is_embedding {
        badges.push(ModelTileBadge {
            label: "Embedding".into(),
            color: p.purple,
            tooltip: None,
        });
    }
    let (vram_short, vram_col) = concise_vram_badge(&m.vram_fit, p);
    badges.push(ModelTileBadge {
        label: vram_short,
        color: vram_col,
        tooltip: Some(m.vram_fit.label.clone()),
    });

    let tail = short_path_tail(&m.gguf_path);
    let spec = ModelTileSpec {
        title: display_title,
        title_tooltip: format!("{} ({})", m.name, m.id),
        subtitle,
        badges,
        progress: None,
        meta_lines: vec![
            (
                format!(
                    "{} · est {} MB VRAM · NGL {}",
                    fmt_bytes(m.size_bytes),
                    m.vram_fit.estimated_total_mb,
                    m.vram_fit.recommended_ngl
                ),
                m.vram_fit.label.clone(),
            ),
            (tail.to_string(), m.gguf_path.clone()),
        ],
        border_highlight: if is_serving {
            Some(p.green)
        } else if is_active {
            Some(p.accent)
        } else {
            None
        },
    };

    render_model_tile(ui, p, tile_w, tile_h, &spec, |ui| {
        if m.is_embedding {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("RAG / Embedding model")
                        .small()
                        .color(p.muted),
                )
                .truncate(),
            );
        } else {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if primary_button(ui, p, "Serve & Use").clicked() {
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
    });
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
                let (cols, tile_w, tile_h, gap) = model_tile_grid_layout(ui.available_width());

                for (row_idx, row_models) in snap.local_models.chunks(cols).enumerate() {
                    if row_idx > 0 {
                        ui.add_space(gap);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for m in row_models {
                            render_local_model_tile(
                                ui,
                                backend,
                                snap,
                                p,
                                m,
                                tile_w,
                                tile_h,
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
            ui.horizontal(|ui| {
                badge(ui, "Strict Local", p.green, p.bg_card);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Hugging Face search & downloads are blocked in Strict Local Mode. Use the bottom-left System & Forge Telemetry popover → Allow Hybrid to enable them.",
                        )
                        .small()
                        .color(p.text),
                    )
                    .truncate(),
                );
            });
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
                            210.0,
                            short_name,
                            Some(&f.filename),
                            false,
                            p,
                            |ui| {
                                let (vram_short, vram_col) = concise_vram_badge(&f.vram_fit, p);
                                badge(ui, &quant, p.muted, p.bg_elev);
                                badge(ui, &vram_short, vram_col, p.bg_elev);
                            },
                        );
                    });
                }
            },
        );
    }

    // Hugging Face Search Results using the same unified model tile grid as Local Vault
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
                    .unwrap_or("")
                    .to_string();
                let results = state.hf_results.clone();
                let (cols, tile_w, tile_h, gap) = model_tile_grid_layout(ui.available_width());

                for (row_idx, row_items) in results.chunks(cols).enumerate() {
                    if row_idx > 0 {
                        ui.add_space(gap);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for res in row_items {
                            let is_inspected = inspected_repo == res.repo_id;
                            let short_repo = res
                                .repo_id
                                .rsplit('/')
                                .next()
                                .unwrap_or(&res.repo_id)
                                .to_string();
                            let owner = if !res.author.trim().is_empty() {
                                res.author.clone()
                            } else {
                                res.repo_id
                                    .split('/')
                                    .next()
                                    .unwrap_or("")
                                    .to_string()
                            };
                            let interesting_tags: Vec<&str> = res
                                .tags
                                .iter()
                                .map(String::as_str)
                                .filter(|t| {
                                    !t.eq_ignore_ascii_case("gguf")
                                        && !t.starts_with("region:")
                                        && !t.starts_with("endpoints_")
                                        && !t.starts_with("arxiv:")
                                        && !t.starts_with("base_model:")
                                })
                                .take(4)
                                .collect();
                            let tags_str = if interesting_tags.is_empty() {
                                "GGUF weights repository".to_string()
                            } else {
                                interesting_tags.join(" · ")
                            };
                            let updated_str = res
                                .last_modified
                                .as_deref()
                                .and_then(|s| s.get(..10))
                                .map(|d| format!("Updated {d} · {}", res.repo_id))
                                .unwrap_or_else(|| res.repo_id.clone());

                            let spec = ModelTileSpec {
                                title: short_repo,
                                title_tooltip: res.repo_id.clone(),
                                subtitle: if owner.is_empty() { None } else { Some(owner) },
                                badges: vec![
                                    ModelTileBadge {
                                        label: "GGUF".into(),
                                        color: p.muted,
                                        tooltip: None,
                                    },
                                    ModelTileBadge {
                                        label: format!("↓ {}", fmt_compact_count(res.downloads)),
                                        color: p.green,
                                        tooltip: Some(format!("{} total downloads", res.downloads)),
                                    },
                                    ModelTileBadge {
                                        label: format!("★ {}", fmt_compact_count(res.likes)),
                                        color: p.amber,
                                        tooltip: Some(format!("{} likes", res.likes)),
                                    },
                                ],
                                progress: None,
                                meta_lines: vec![
                                    (tags_str, res.tags.join(", ")),
                                    (updated_str, res.repo_id.clone()),
                                ],
                                border_highlight: if is_inspected { Some(p.accent) } else { None },
                            };

                            render_model_tile(ui, p, tile_w, tile_h, &spec, |ui| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
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
                                    },
                                );
                            });
                        }
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
        "Probe configured cloud APIs for live reachability, free/paid tier access, and daily limits.",
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
                    if primary_button(ui, p, "Probe All Providers").clicked() {
                        backend.save_keys(KeysPatch::default(), true);
                    }
                    if secondary_button(ui, p, "Manage API Keys & Aliases in Settings →")
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
        "Models that are free/reachable can be selected in Chat; paid or unreachable models are filed under separate tabs.",
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
                let (avail_list, paid_list, unavail_list) =
                    partition_quotas_tab(&snap.cloud_quotas);
                let avail_lbl = format!("Available ({})", avail_list.len());
                let paid_lbl = format!("Paid / Credits ({})", paid_list.len());
                let unavail_lbl = format!("Unreachable ({})", unavail_list.len());

                if let Some(next_tab) = segmented_control(
                    ui,
                    p,
                    "forge_quotas_filter_tab",
                    state.quotas_filter,
                    &[
                        (QuotasTabFilter::Available, avail_lbl.as_str()),
                        (QuotasTabFilter::Paid, paid_lbl.as_str()),
                        (QuotasTabFilter::Unreachable, unavail_lbl.as_str()),
                    ],
                ) {
                    state.quotas_filter = next_tab;
                }
                ui.add_space(10.0);

                let active_bucket: Vec<&ModelQuotaSnapshot> = match state.quotas_filter {
                    QuotasTabFilter::Available => avail_list,
                    QuotasTabFilter::Paid => paid_list,
                    QuotasTabFilter::Unreachable => unavail_list,
                };

                if active_bucket.is_empty() {
                    let (title, hint) = match state.quotas_filter {
                        QuotasTabFilter::Available => (
                            "No available cloud models yet",
                            "Add an API key in Settings and click Probe All Providers to discover reachable models.",
                        ),
                        QuotasTabFilter::Paid => (
                            "No payment-gated models detected",
                            "OpenRouter or provider models requiring paid credits will be separated into this tab automatically.",
                        ),
                        QuotasTabFilter::Unreachable => (
                            "No unreachable models",
                            "Models missing an API key or failing live verification appear here.",
                        ),
                    };
                    empty_state_block(ui, p, Icon::Shield, title, hint, None);
                } else {
                    let (cols, tile_w, tile_h, gap) = model_tile_grid_layout(ui.available_width());
                    for (row_idx, row_items) in active_bucket.chunks(cols).enumerate() {
                        if row_idx > 0 {
                            ui.add_space(gap);
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for q in row_items {
                                let is_active = snap.settings.active_model_id == q.model_id;
                                let resolved = snap
                                    .settings
                                    .resolve_model_alias(&q.model_id, &q.display_name)
                                    .to_string();
                                let subtitle = if resolved != q.display_name {
                                    Some(format!("{} · {}", q.display_name, q.model_id))
                                } else {
                                    Some(q.model_id.clone())
                                };
                                let (prov_lbl, prov_col) = provider_pill_style(q.provider, p);
                                let mut badges = vec![ModelTileBadge {
                                    label: prov_lbl.into(),
                                    color: prov_col,
                                    tooltip: None,
                                }];
                                if q.requires_payment {
                                    badges.push(ModelTileBadge {
                                        label: "Paid Credits".into(),
                                        color: p.amber,
                                        tooltip: Some(q.reachability_reason.clone()),
                                    });
                                } else if q.reachable {
                                    badges.push(ModelTileBadge {
                                        label: "Reachable".into(),
                                        color: p.green,
                                        tooltip: Some(q.reachability_reason.clone()),
                                    });
                                } else {
                                    badges.push(ModelTileBadge {
                                        label: "Unreachable".into(),
                                        color: p.red,
                                        tooltip: Some(q.reachability_reason.clone()),
                                    });
                                }
                                if is_active {
                                    badges.push(ModelTileBadge {
                                        label: "Active".into(),
                                        color: p.accent_hot,
                                        tooltip: None,
                                    });
                                }

                                let prog_col = if q.remaining_daily_pct >= 50.0 {
                                    p.green
                                } else if q.remaining_daily_pct >= 20.0 {
                                    p.amber
                                } else {
                                    p.red
                                };
                                let progress = if q.reachable && !q.requires_payment {
                                    Some(ModelTileProgress {
                                        fraction: (q.remaining_daily_pct as f32 / 100.0)
                                            .clamp(0.0, 1.0),
                                        color: prog_col,
                                        right_label: format!("{:.0}% left", q.remaining_daily_pct),
                                    })
                                } else {
                                    None
                                };

                                let meta_lines = if q.reachable && !q.requires_payment {
                                    vec![
                                        (
                                            format!(
                                                "Today: {} reqs · {} / {} tok",
                                                q.rpd_used, q.tokens_today, q.daily_token_limit
                                            ),
                                            String::new(),
                                        ),
                                        (
                                            format!(
                                                "RPM {}/{} · Ctx {}k",
                                                q.rpm_used,
                                                q.rpm_limit,
                                                q.context_window / 1000
                                            ),
                                            q.reachability_reason.clone(),
                                        ),
                                    ]
                                } else {
                                    vec![
                                        (
                                            q.reachability_reason.clone(),
                                            q.reachability_reason.clone(),
                                        ),
                                        (
                                            format!(
                                                "Ctx {}k · RPD limit {}",
                                                q.context_window / 1000,
                                                q.rpd_limit
                                            ),
                                            q.model_id.clone(),
                                        ),
                                    ]
                                };

                                let spec = ModelTileSpec {
                                    title: resolved,
                                    title_tooltip: format!("{} ({})", q.display_name, q.model_id),
                                    subtitle,
                                    badges,
                                    progress,
                                    meta_lines,
                                    border_highlight: if is_active { Some(p.accent) } else { None },
                                };

                                render_model_tile(ui, p, tile_w, tile_h, &spec, |ui| {
                                    if q.reachable && !q.requires_payment {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let lbl = if is_active { "Active" } else { "Use" };
                                                let btn = if is_active {
                                                    ghost_button(
                                                        ui,
                                                        p,
                                                        egui::RichText::new(lbl)
                                                            .color(p.accent_hot),
                                                    )
                                                } else {
                                                    secondary_button(ui, p, lbl)
                                                };
                                                if btn.clicked() {
                                                    let id = q.model_id.clone();
                                                    backend.update_settings(move |s| {
                                                        s.active_model_id = id;
                                                    });
                                                }
                                            },
                                        );
                                    } else {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new("Hidden from Chat pickers")
                                                    .small()
                                                    .color(p.muted),
                                            )
                                            .truncate(),
                                        );
                                    }
                                });
                            }
                        });
                    }
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_tile_grid_layout_fits_cards_without_clipping_across_widths() {
        for (w, expected_cols) in [(350.0, 1), (600.0, 2), (920.0, 3), (1120.0, 3)] {
            let (cols, tile_w, tile_h, gap) = model_tile_grid_layout(w);
            assert_eq!(cols, expected_cols, "width={w}");
            assert!(tile_w >= 280.0, "tile_w={tile_w} too narrow at w={w}");
            assert!(tile_h >= 180.0, "tile_h={tile_h}");
            let total_row_w = (cols as f32) * tile_w + ((cols - 1) as f32) * gap;
            assert!(
                total_row_w <= w + 0.5,
                "row overflow: total_row_w={total_row_w} > w={w}"
            );
        }
    }

    #[test]
    fn concise_vram_badge_and_quota_tab_partition_work() {
        let p = CinderPalette::from_ui(&harness_store::UiCustomization::default());
        let fit = VramFitAssessment {
            tier: FitTier::PartialOffload,
            label: "Hybrid GPU+RAM Offload · ~36 layers on GPU".into(),
            weights_mb: 8000,
            kv_cache_mb: 1024,
            compute_buffer_mb: 512,
            estimated_total_mb: 9536,
            recommended_ngl: 36,
        };
        let (lbl, _col) = concise_vram_badge(&fit, &p);
        assert_eq!(lbl, "Hybrid GPU · 36L");

        let mk_q = |id: &str, reachable: bool, requires_payment: bool| ModelQuotaSnapshot {
            model_id: id.into(),
            display_name: id.into(),
            provider: ProviderKind::OpenRouter,
            context_window: 128_000,
            supports_thinking: false,
            supports_tools: true,
            remaining_daily_pct: 100.0,
            rpm_used: 0,
            rpm_limit: 20,
            tpm_used: 0,
            tpm_limit: 200_000,
            rpd_used: 0,
            rpd_limit: 200,
            tokens_today: 0,
            daily_token_limit: 2_000_000,
            prompt_tokens_today: 0,
            completion_tokens_today: 0,
            thinking_tokens_today: 0,
            reset_in_seconds: 3600,
            header_source: false,
            reachable,
            reachability_reason: String::new(),
            antigravity_alias: None,
            requires_payment,
        };
        let list = vec![
            mk_q("free:free", true, false),
            mk_q("paid-pro", false, true),
            mk_q("broken", false, false),
        ];
        let (a, paid, u) = partition_quotas_tab(&list);
        assert_eq!(a.len(), 1);
        assert_eq!(paid.len(), 1);
        assert_eq!(u.len(), 1);
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
