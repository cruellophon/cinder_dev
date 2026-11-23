use crate::app_state::{fmt_bytes, ForgeTab, GuiState, MainView, SettingsTab};
use crate::backend::{BackendHandle, FolderPickTarget, KeysPatch};
use crate::customizations::HARNESS_MODULE_DEFS;
use crate::icons::{draw_icon, icon_button, Icon};
use crate::theme::{
    badge, empty_state_block, input_frame, nav_row_with_icon, pane_header, primary_button,
    secondary_button, segmented_control, setting_group, setting_row_item, toggle_switch,
    CinderPalette,
};
use egui::{Color32, CornerRadius, Stroke};
use harness_net::{AuditOutcome, PrivacyMode};
use harness_store::FallbackPolicy;
use std::sync::Arc;

pub fn render_settings_rail(ui: &mut egui::Ui, state: &mut GuiState, p: &CinderPalette) {
    // Exact 40px aligned pane header matching CentralPanel
    pane_header(ui, p, |ui| {
        let (back_rect, back_resp) =
            ui.allocate_exact_size(egui::vec2(72.0, 26.0), egui::Sense::click());
        if ui.is_rect_visible(back_rect) {
            if back_resp.hovered() {
                ui.painter()
                    .rect_filled(back_rect, CornerRadius::same(6), p.bg_hover);
            }
            let col = if back_resp.hovered() { p.text } else { p.muted };
            draw_icon(
                ui.painter(),
                egui::Rect::from_center_size(
                    egui::pos2(back_rect.left() + 12.0, back_rect.center().y),
                    egui::vec2(13.5, 13.5),
                ),
                Icon::ArrowLeft,
                col,
            );
            ui.painter().text(
                egui::pos2(back_rect.left() + 24.0, back_rect.center().y),
                egui::Align2::LEFT_CENTER,
                "Back",
                egui::FontId::proportional(p.base_font_size - 0.5),
                col,
            );
        }
        if back_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if back_resp.clicked() {
            state.show_settings_modal = false;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new("Esc").small().color(p.muted));
        });
    });

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 14))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("Settings")
                    .size(p.base_font_size + 4.5)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(10.0);

            input_frame(p, false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon_r, _) =
                        ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    draw_icon(ui.painter(), icon_r, Icon::Search, p.muted);
                    let clear_w = if state.settings_search.is_empty() {
                        0.0
                    } else {
                        24.0
                    };
                    let w = (ui.available_width() - clear_w - 4.0).max(60.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut state.settings_search)
                            .hint_text("Search settings...")
                            .desired_width(w)
                            .frame(false),
                    );
                    if !state.settings_search.is_empty()
                        && icon_button(ui, p, Icon::Close, false, "Clear Search").clicked()
                    {
                        state.settings_search.clear();
                    }
                });
            });

            ui.add_space(12.0);
            for (tab, icon, lbl) in [
                (SettingsTab::Appearance, Icon::Palette, "Appearance"),
                (SettingsTab::Models, Icon::Key, "Models & Keys"),
                (SettingsTab::Engine, Icon::Cpu, "Local Forge & GPU"),
                (SettingsTab::Agent, Icon::Sliders, "Agent & Autonomy"),
                (SettingsTab::Privacy, Icon::Shield, "Privacy & Audit"),
            ] {
                let active = state.settings_search.trim().is_empty() && state.settings_tab == tab;
                if nav_row_with_icon(ui, p, icon, active, lbl).clicked() {
                    state.settings_search.clear();
                    state.settings_tab = tab;
                }
            }
        });
}

pub fn render_settings_content(
    _ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        ui.centered_and_justified(|ui| ui.spinner());
        return;
    };

    let q_raw = state.settings_search.trim().to_string();
    let q = q_raw.to_ascii_lowercase();

    // Exact 40px aligned pane header matching Settings Rail
    pane_header(ui, p, |ui| {
        if !q.is_empty() {
            ui.label(
                egui::RichText::new("Search Results")
                    .strong()
                    .color(p.text),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("Matches for \"{q_raw}\""))
                    .small()
                    .color(p.muted),
            );
        } else {
            ui.label(
                egui::RichText::new(state.settings_tab.title())
                    .strong()
                    .color(p.text),
            );
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(state.settings_tab.subtitle())
                        .small()
                        .color(p.muted),
                )
                .truncate(),
            );
        }
    });

    let avail_w = ui.available_width();
    let col_w = (avail_w - 56.0).clamp(340.0, 760.0);
    let side_pad = ((avail_w - col_w) * 0.5).max(24.0);

    egui::ScrollArea::vertical()
        .id_salt(("settings_page_scroll", format!("{:?}", state.settings_tab)))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(side_pad);
                ui.allocate_ui_with_layout(
                    egui::vec2(col_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if !q.is_empty() {
                            render_search_results(ui, state, backend, &snap, p, &q);
                        } else {
                            match state.settings_tab {
                                SettingsTab::Appearance => {
                                    render_appearance_section(ui, backend, &snap, p);
                                }
                                SettingsTab::Models => {
                                    render_models_section(ui, state, backend, &snap, p);
                                }
                                SettingsTab::Engine => {
                                    render_engine_section(ui, state, backend, &snap, p);
                                }
                                SettingsTab::Agent => {
                                    render_agent_section(ui, state, backend, &snap, p);
                                }
                                SettingsTab::Privacy => {
                                    render_privacy_section(ui, backend, &snap, p);
                                }
                            }
                        }
                        ui.add_space(28.0);
                    },
                );
            });
        });
}

fn render_search_results(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
    q: &str,
) {
    let mut hits = 0usize;
    if [
        "theme",
        "appearance",
        "accent",
        "font",
        "scale",
        "code",
        "token",
        "light",
        "dark",
        "slate",
        "wrap",
    ]
    .iter()
    .any(|k| k.contains(q) || q.contains(k))
    {
        hits += 1;
        render_appearance_section(ui, backend, snap, p);
        ui.add_space(18.0);
    }
    if [
        "model",
        "provider",
        "key",
        "gemini",
        "anthropic",
        "openrouter",
        "openai",
        "auto-switch",
        "fallback",
        "reachable",
        "custom",
        "alias",
        "aliases",
    ]
    .iter()
    .any(|k| k.contains(q) || q.contains(k))
    {
        hits += 1;
        render_models_section(ui, state, backend, snap, p);
        ui.add_space(18.0);
    }
    if [
        "forge",
        "gpu",
        "vram",
        "vault",
        "gguf",
        "ctx",
        "context",
        "ngl",
        "cuda",
        "vulkan",
        "quench",
        "idle",
        "llama",
        "api",
    ]
    .iter()
    .any(|k| k.contains(q) || q.contains(k))
    {
        hits += 1;
        render_engine_section(ui, state, backend, snap, p);
        ui.add_space(18.0);
    }
    if [
        "agent",
        "autonomy",
        "approve",
        "safety",
        "orchestrator",
        "planner",
        "executor",
        "module",
        "shell",
        "rag",
        "subagent",
        "web",
        "instructions",
        "prompt",
    ]
    .iter()
    .any(|k| k.contains(q) || q.contains(k))
    {
        hits += 1;
        render_agent_section(ui, state, backend, snap, p);
        ui.add_space(18.0);
    }
    if ["privacy", "local", "hybrid", "audit", "log", "network", "loopback"]
        .iter()
        .any(|k| k.contains(q) || q.contains(k))
    {
        hits += 1;
        render_privacy_section(ui, backend, snap, p);
    }
    if hits == 0 {
        empty_state_block(
            ui,
            p,
            Icon::Search,
            "No matching settings",
            "Try searching \"theme\", \"api\", \"vram\", \"agent\", or \"privacy\".",
            Some("Reset Search"),
        )
        .then(|| state.settings_search.clear());
    }
}

fn render_appearance_section(
    ui: &mut egui::Ui,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    let mut ui_cfg = snap.settings.ui.clone();
    let mut changed = false;

    setting_group(
        ui,
        p,
        "Theme",
        "Four crafted colorways paired with an optional ember accent glow.",
        |ui| {
            // 4 genuinely distinct miniature-IDE Theme Swatch Cards
            let swatches = [
                (
                    "cinder_forge",
                    "Cinder Slate",
                    Color32::from_rgb(16, 18, 23),
                    Color32::from_rgb(20, 23, 29),
                    Color32::from_rgb(40, 46, 58),
                    Color32::from_rgb(244, 113, 59),
                ),
                (
                    "singularity_obsidian",
                    "Obsidian",
                    Color32::from_rgb(12, 13, 18),
                    Color32::from_rgb(7, 8, 11),
                    Color32::from_rgb(44, 48, 60),
                    Color32::from_rgb(255, 110, 48),
                ),
                (
                    "nordic_slate",
                    "Nordic Slate",
                    Color32::from_rgb(24, 29, 38),
                    Color32::from_rgb(31, 37, 49),
                    Color32::from_rgb(56, 66, 84),
                    Color32::from_rgb(255, 110, 48),
                ),
                (
                    "paper_light",
                    "Paper Light",
                    Color32::from_rgb(255, 255, 255),
                    Color32::from_rgb(244, 241, 238),
                    Color32::from_rgb(218, 212, 206),
                    Color32::from_rgb(228, 86, 28),
                ),
            ];

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (k, lbl, sb_col, bg_col, line_col, dot_col) in swatches {
                    let selected = ui_cfg.theme == k;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(146.0, 76.0), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        if selected {
                            ui.painter().rect_stroke(
                                rect.expand(2.0),
                                CornerRadius::same(10),
                                Stroke::new(
                                    3.0_f32,
                                    Color32::from_rgba_unmultiplied(
                                        p.accent.r(),
                                        p.accent.g(),
                                        p.accent.b(),
                                        34,
                                    ),
                                ),
                                egui::StrokeKind::Outside,
                            );
                        }
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), bg_col);
                        // Miniature sidebar
                        let sb_rect = egui::Rect::from_min_size(
                            rect.min,
                            egui::vec2(32.0, rect.height() - 20.0),
                        );
                        ui.painter()
                            .rect_filled(sb_rect, CornerRadius::same(6), sb_col);
                        ui.painter().vline(
                            rect.left() + 32.0,
                            rect.top()..=(rect.bottom() - 20.0),
                            Stroke::new(1.0_f32, line_col),
                        );
                        // Miniature 40px header hairline + chat skeleton + composer bar
                        ui.painter().hline(
                            rect.left()..=rect.right(),
                            rect.top() + 14.0,
                            Stroke::new(1.0_f32, line_col),
                        );
                        ui.painter().circle_filled(
                            egui::pos2(rect.right() - 12.0, rect.top() + 7.5),
                            3.5,
                            dot_col,
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(rect.left() + 42.0, rect.top() + 22.0),
                                egui::vec2(64.0, 5.0),
                            ),
                            CornerRadius::same(2),
                            line_col,
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(rect.left() + 42.0, rect.top() + 31.0),
                                egui::vec2(44.0, 4.0),
                            ),
                            CornerRadius::same(2),
                            line_col,
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(rect.left() + 42.0, rect.bottom() - 32.0),
                                egui::vec2(rect.width() - 52.0, 9.0),
                            ),
                            CornerRadius::same(3),
                            sb_col,
                        );

                        // Outer frame + label strip
                        let stroke_col = if selected {
                            p.accent
                        } else if resp.hovered() {
                            p.border_strong
                        } else {
                            p.border
                        };
                        ui.painter().rect_stroke(
                            rect,
                            CornerRadius::same(8),
                            Stroke::new(if selected { 1.5_f32 } else { 1.0_f32 }, stroke_col),
                            egui::StrokeKind::Inside,
                        );
                        let txt_col = if k == "paper_light" {
                            Color32::from_rgb(27, 22, 20)
                        } else {
                            Color32::from_rgb(233, 237, 244)
                        };
                        ui.painter().text(
                            egui::pos2(rect.left() + 10.0, rect.bottom() - 10.0),
                            egui::Align2::LEFT_CENTER,
                            lbl,
                            egui::FontId::proportional(11.5),
                            txt_col,
                        );
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        ui_cfg.theme = k.into();
                        changed = true;
                    }
                }
            });

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Accent Glow").strong().color(p.text));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        for (k, lbl, col) in [
                            ("cinder", "Ember", Color32::from_rgb(244, 113, 59)),
                            ("amber", "Gold", Color32::from_rgb(229, 153, 62)),
                            ("blue", "Cobalt", Color32::from_rgb(59, 130, 246)),
                            ("emerald", "Emerald", Color32::from_rgb(16, 185, 129)),
                            ("violet", "Violet", Color32::from_rgb(139, 92, 246)),
                            ("mono", "Mono", Color32::from_rgb(200, 204, 212)),
                        ] {
                            let sel = ui_cfg.accent_color == k;
                            let (r, resp) = ui.allocate_exact_size(
                                egui::vec2(26.0, 26.0),
                                egui::Sense::click(),
                            );
                            if ui.is_rect_visible(r) {
                                if sel {
                                    ui.painter().circle_filled(
                                        r.center(),
                                        12.5,
                                        Color32::from_rgba_unmultiplied(
                                            col.r(),
                                            col.g(),
                                            col.b(),
                                            50,
                                        ),
                                    );
                                    ui.painter().circle_stroke(
                                        r.center(),
                                        11.0,
                                        Stroke::new(1.5_f32, p.text),
                                    );
                                }
                                ui.painter().circle_filled(r.center(), 8.0, col);
                            }
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if resp.on_hover_text(lbl).clicked() {
                                ui_cfg.accent_color = k.into();
                                changed = true;
                            }
                        }
                    });
                });

                ui.add_space(28.0);

                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Text Scale").strong().color(p.text));
                    ui.add_space(6.0);
                    let cur_scale = match ui_cfg.font_scale.as_str() {
                        "compact" => "compact",
                        "comfortable" => "comfortable",
                        _ => "default",
                    };
                    if let Some(next_scale) = segmented_control(
                        ui,
                        p,
                        "settings_text_scale_seg",
                        cur_scale,
                        &[
                            ("compact", "Compact"),
                            ("default", "Default"),
                            ("comfortable", "Comfortable"),
                        ],
                    ) {
                        ui_cfg.font_scale = next_scale.into();
                        changed = true;
                    }
                });
            });
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Reading & Chat",
        "Typography and response footer visibility inside the chat canvas.",
        |ui| {
            let mut wrap = ui_cfg.wrap_code_blocks;
            setting_row_item(
                ui,
                p,
                "Wrap long code lines",
                "Soft-wrap long lines inside assistant code blocks.",
                true,
                |ui| {
                    if toggle_switch(ui, p, &mut wrap).changed() {
                        ui_cfg.wrap_code_blocks = wrap;
                        changed = true;
                    }
                },
            );
            let mut metrics = ui_cfg.show_token_metrics;
            setting_row_item(
                ui,
                p,
                "Show latency & token counts",
                "Display response timing and token usage in the message footer.",
                false,
                |ui| {
                    if toggle_switch(ui, p, &mut metrics).changed() {
                        ui_cfg.show_token_metrics = metrics;
                        changed = true;
                    }
                },
            );
        },
    );

    if changed {
        backend.save_ui_and_orch(Some(ui_cfg), None);
    }
}

fn render_models_section(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    setting_group(
        ui,
        p,
        "Model Routing & Failover",
        "Control model filtering and automatic multi-provider switching on quota exhaustion.",
        |ui| {
            let mut only_reach = snap.settings.orchestrator.only_show_reachable_models;
            setting_row_item(
                ui,
                p,
                "Only show reachable models",
                "Hide unconfigured cloud providers or missing local weights from model pickers.",
                true,
                |ui| {
                    if toggle_switch(ui, p, &mut only_reach).changed() {
                        let mut o = snap.settings.orchestrator.clone();
                        o.only_show_reachable_models = only_reach;
                        backend.save_ui_and_orch(None, Some(o));
                    }
                },
            );

            let cur_policy = if !snap.settings.auto_switch_models {
                FallbackPolicy::Strict
            } else {
                snap.settings.fallback_policy
            };
            let cur_policy_key = match cur_policy {
                FallbackPolicy::Ask => "ask",
                FallbackPolicy::AutoChain => "auto_chain",
                FallbackPolicy::Strict => "strict",
            };
            setting_row_item(
                ui,
                p,
                "Fallback policy on failure",
                "Ask before switching models, follow an ordered chain automatically, or fail cleanly without switching.",
                true,
                |ui| {
                    if let Some(next_key) = segmented_control(
                        ui,
                        p,
                        "settings_fallback_policy_seg",
                        cur_policy_key,
                        &[
                            ("ask", "Ask First"),
                            ("auto_chain", "Auto-Chain"),
                            ("strict", "Never"),
                        ],
                    ) {
                        let next_policy = match next_key {
                            "auto_chain" => FallbackPolicy::AutoChain,
                            "strict" => FallbackPolicy::Strict,
                            _ => FallbackPolicy::Ask,
                        };
                        backend.update_settings(move |s| {
                            s.fallback_policy = next_policy;
                            s.auto_switch_models = next_policy != FallbackPolicy::Strict;
                        });
                    }
                },
            );

            if cur_policy != FallbackPolicy::Strict {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Ordered Fallback Chain")
                        .strong()
                        .color(p.text),
                );
                ui.label(
                    egui::RichText::new(match cur_policy {
                        FallbackPolicy::Ask => "Ask First lists these models first as quick-continue choices. Leave empty to suggest healthy free models automatically.",
                        FallbackPolicy::AutoChain => "Auto-Chain tries only these models in order without jumping elsewhere. Leave empty for automatic healthy-model discovery.",
                        FallbackPolicy::Strict => "",
                    })
                    .small()
                    .color(p.muted),
                );
                ui.add_space(6.0);

                if snap.settings.fallback_chain.is_empty() {
                    ui.label(
                        egui::RichText::new("No custom fallback chain configured (automatic healthy-model discovery active).")
                            .small()
                            .color(p.muted),
                    );
                } else {
                    let chain_len = snap.settings.fallback_chain.len();
                    let all_models = state.selectable_models(false);
                    let mut chain_action: Option<(usize, &str)> = None;
                    for (idx, mid) in snap.settings.fallback_chain.iter().enumerate() {
                        let display = all_models
                            .iter()
                            .find(|m| m.id == *mid)
                            .map(|m| format!("[{}] {}", m.provider_label, m.canonical_name))
                            .unwrap_or_else(|| mid.clone());
                        p.subcard_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                badge(ui, &format!("#{}", idx + 1), p.amber, p.bg_elev);
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(display).strong().color(p.text));
                                    ui.label(
                                        egui::RichText::new(mid)
                                            .monospace()
                                            .small()
                                            .color(p.muted),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if secondary_button(ui, p, "×")
                                            .on_hover_text("Remove from fallback chain")
                                            .clicked()
                                        {
                                            chain_action = Some((idx, "remove"));
                                        }
                                        if idx + 1 < chain_len
                                            && secondary_button(ui, p, "↓")
                                                .on_hover_text("Move lower")
                                                .clicked()
                                        {
                                            chain_action = Some((idx, "down"));
                                        }
                                        if idx > 0
                                            && secondary_button(ui, p, "↑")
                                                .on_hover_text("Move higher")
                                                .clicked()
                                        {
                                            chain_action = Some((idx, "up"));
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(4.0);
                    }
                    if let Some((idx, act)) = chain_action {
                        backend.update_settings(move |s| {
                            match act {
                                "up" if idx > 0 && idx < s.fallback_chain.len() => {
                                    s.fallback_chain.swap(idx, idx - 1);
                                }
                                "down" if idx + 1 < s.fallback_chain.len() => {
                                    s.fallback_chain.swap(idx, idx + 1);
                                }
                                "remove" if idx < s.fallback_chain.len() => {
                                    s.fallback_chain.remove(idx);
                                }
                                _ => {}
                            }
                        });
                    }
                }

                ui.add_space(4.0);
                let selectable: Vec<_> = state
                    .selectable_models(true)
                    .into_iter()
                    .filter(|m| !snap.settings.fallback_chain.contains(&m.id))
                    .collect();
                if !selectable.is_empty()
                    && (state.fallback_chain_add_draft.trim().is_empty()
                        || !selectable
                            .iter()
                            .any(|m| m.id == state.fallback_chain_add_draft))
                {
                    state.fallback_chain_add_draft = selectable[0].id.clone();
                } else if selectable.is_empty() {
                    state.fallback_chain_add_draft.clear();
                }
                let add_lbl = selectable
                    .iter()
                    .find(|m| m.id == state.fallback_chain_add_draft)
                    .map(|m| format!("[{}] {}", m.provider_label, m.canonical_name))
                    .unwrap_or_else(|| "No more models available...".into());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    let combo_w = (ui.available_width() - 138.0).clamp(180.0, 360.0);
                    egui::ComboBox::from_id_salt("settings_fallback_chain_add_combo")
                        .selected_text(add_lbl)
                        .width(combo_w)
                        .show_ui(ui, |ui| {
                            for m in &selectable {
                                let row_lbl = format!("[{}] {}", m.provider_label, m.canonical_name);
                                if ui
                                    .selectable_label(state.fallback_chain_add_draft == m.id, row_lbl)
                                    .clicked()
                                {
                                    state.fallback_chain_add_draft = m.id.clone();
                                }
                            }
                        });
                    let can_add = !state.fallback_chain_add_draft.trim().is_empty()
                        && selectable
                            .iter()
                            .any(|m| m.id == state.fallback_chain_add_draft);
                    if ui
                        .add_enabled_ui(can_add, |ui| primary_button(ui, p, "+ Add Fallback"))
                        .inner
                        .clicked()
                    {
                        let add_id = state.fallback_chain_add_draft.clone();
                        backend.update_settings(move |s| {
                            if !s.fallback_chain.contains(&add_id) {
                                s.fallback_chain.push(add_id);
                            }
                        });
                        state.fallback_chain_add_draft.clear();
                    }
                });
                ui.add_space(6.0);
                let y = ui.cursor().top();
                ui.painter()
                    .hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, p.border));
                ui.add_space(4.0);
            }

            let mut fb_local = snap.settings.fallback_to_local;
            setting_row_item(
                ui,
                p,
                "Allow local GPU fallback",
                "Fall back to a local GGUF / Ollama model if cloud providers are unavailable.",
                false,
                |ui| {
                    if toggle_switch(ui, p, &mut fb_local).changed() {
                        backend.update_settings(move |s| {
                            s.fallback_to_local = fb_local;
                        });
                    }
                },
            );
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Provider Credentials",
        "Keys stay in local storage and are masked on screen.",
        |ui| {
            if !state.verify_statuses.is_empty() {
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
                ui.add_space(8.0);
            }

            let kf_g = !snap.settings.api_keys.gemini_api_key.trim().is_empty();
            let kf_a = !snap.settings.api_keys.anthropic_api_key.trim().is_empty();
            let kf_o = !snap.settings.api_keys.openrouter_api_key.trim().is_empty();
            let kf_u = !snap.settings.api_keys.openai_compat_base_url.trim().is_empty()
                || !snap.settings.api_keys.openai_compat_api_key.trim().is_empty();
            let kf_h = !snap.settings.api_keys.huggingface_token.trim().is_empty();
            let kf_b = !snap.settings.api_keys.brave_search_api_key.trim().is_empty();

            key_row(
                ui,
                p,
                "Google Gemini",
                kf_g,
                &mut state.key_gemini,
                "gemini",
                backend,
            );
            key_row(
                ui,
                p,
                "Anthropic Claude",
                kf_a,
                &mut state.key_anthropic,
                "anthropic",
                backend,
            );
            key_row(
                ui,
                p,
                "OpenRouter",
                kf_o,
                &mut state.key_openrouter,
                "openrouter",
                backend,
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                fixed_label_col(ui, 128.0, "OpenAI-Compat", p.text);
                // Combined outer width = (188 + 20) + 7 + (105 + 20) = 340 px, matching key_row exactly
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.key_oai_url)
                            .hint_text("Base URL (127.0.0.1:1234/v1)")
                            .desired_width(188.0)
                            .frame(false),
                    );
                });
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.key_oai_key)
                            .password(true)
                            .hint_text(if kf_u { "Configured" } else { "API Key" })
                            .desired_width(105.0)
                            .frame(false),
                    );
                });
                fixed_status_col(ui, p, kf_u);
                if kf_u && secondary_button(ui, p, "Clear").clicked() {
                    state.key_oai_url.clear();
                    state.key_oai_key.clear();
                    backend.save_keys(
                        KeysPatch {
                            clear_keys: vec!["openai_compat".into()],
                            ..Default::default()
                        },
                        false,
                    );
                }
            });
            ui.add_space(4.0);
            key_row(
                ui,
                p,
                "Hugging Face",
                kf_h,
                &mut state.key_hf,
                "huggingface",
                backend,
            );
            key_row(
                ui,
                p,
                "Brave Search",
                kf_b,
                &mut state.key_brave,
                "brave",
                backend,
            );

            ui.add_space(10.0);
            let build_keys_patch = |st: &GuiState| KeysPatch {
                gemini_api_key: opt(&st.key_gemini),
                anthropic_api_key: opt(&st.key_anthropic),
                openrouter_api_key: opt(&st.key_openrouter),
                openai_compat_base_url: Some(st.key_oai_url.clone()),
                openai_compat_api_key: opt(&st.key_oai_key),
                huggingface_token: opt(&st.key_hf),
                brave_search_api_key: opt(&st.key_brave),
                clear_keys: Vec::new(),
            };
            ui.horizontal(|ui| {
                // Swapped order & colors: Save Keys Offline is primary orange left; Save & Live-Probe Models is secondary grey right
                if primary_button(ui, p, "Save Keys Offline").clicked() {
                    backend.save_keys(build_keys_patch(state), false);
                }
                if secondary_button(ui, p, "Save & Live-Probe Models").clicked() {
                    backend.save_keys(build_keys_patch(state), true);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, p, "View Quotas in Forge →").clicked() {
                        state.switch_main_view(MainView::Forge(ForgeTab::Quotas), true);
                    }
                });
            });
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Model Aliases",
        "Assign custom display names to active local or cloud models across Chat, Orchestrator, and Forge.",
        |ui| {
            let active_models = state.selectable_models(true);
            if !active_models.is_empty()
                && (state.alias_target_model_id.trim().is_empty()
                    || !active_models.iter().any(|m| m.id == state.alias_target_model_id))
            {
                state.alias_target_model_id = active_models[0].id.clone();
            }

            let selected_lbl = active_models
                .iter()
                .find(|m| m.id == state.alias_target_model_id)
                .map(|m| format!("[{}] {}", m.provider_label, m.canonical_name))
                .unwrap_or_else(|| "Select an active model...".into());

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let avail = ui.available_width();
                let combo_w = ((avail - 124.0) * 0.50).clamp(160.0, 280.0);
                let alias_input_w = (avail - combo_w - 140.0).clamp(110.0, 240.0);
                egui::ComboBox::from_id_salt("settings_alias_model_combo")
                    .selected_text(selected_lbl)
                    .width(combo_w)
                    .show_ui(ui, |ui| {
                        for m in &active_models {
                            let row_lbl = format!("[{}] {}", m.provider_label, m.canonical_name);
                            if ui
                                .selectable_label(state.alias_target_model_id == m.id, row_lbl)
                                .clicked()
                            {
                                state.alias_target_model_id = m.id.clone();
                            }
                        }
                    });
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.alias_name_draft)
                            .hint_text("Assign alias (e.g. Fast Coder)")
                            .desired_width(alias_input_w)
                            .frame(false),
                    );
                });
                let can_assign = !state.alias_target_model_id.trim().is_empty()
                    && !state.alias_name_draft.trim().is_empty();
                if ui
                    .add_enabled_ui(can_assign, |ui| primary_button(ui, p, "Assign Alias"))
                    .inner
                    .clicked()
                {
                    let target_id = state.alias_target_model_id.clone();
                    let alias = state.alias_name_draft.trim().to_string();
                    backend.update_settings(move |s| {
                        s.model_aliases.insert(target_id, alias);
                    });
                    state.alias_name_draft.clear();
                    state.set_toast("Assigned model alias.", false);
                }
            });

            ui.add_space(10.0);
            let y = ui.cursor().top();
            ui.painter()
                .hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, p.border));
            ui.add_space(8.0);

            ui.label(
                egui::RichText::new(format!(
                    "Assigned Aliases Ledger ({})",
                    snap.settings.model_aliases.len()
                ))
                .small()
                .strong()
                .color(p.text),
            );
            ui.add_space(4.0);

            if snap.settings.model_aliases.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "No model aliases assigned yet. Choose an active model above and give it an alias.",
                    )
                    .small()
                    .color(p.muted),
                );
            } else {
                let mut remove_id: Option<String> = None;
                for (mid, alias) in &snap.settings.model_aliases {
                    let canonical = snap
                        .local_models
                        .iter()
                        .find(|m| m.id == *mid)
                        .map(|m| m.name.as_str())
                        .or_else(|| {
                            snap.cloud_catalog
                                .iter()
                                .find(|m| m.id == *mid)
                                .map(|m| m.display_name.as_str())
                        })
                        .unwrap_or(mid.as_str());
                    p.subcard_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(canonical).strong().color(p.text));
                                    ui.label(egui::RichText::new("→").color(p.muted));
                                    ui.label(
                                        egui::RichText::new(alias)
                                            .strong()
                                            .color(p.accent_hot),
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(mid)
                                        .monospace()
                                        .small()
                                        .color(p.muted),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if secondary_button(ui, p, "Remove").clicked() {
                                        remove_id = Some(mid.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(4.0);
                }
                if let Some(id_to_rm) = remove_id {
                    backend.update_settings(move |s| {
                        s.model_aliases.remove(&id_to_rm);
                    });
                    state.set_toast("Removed model alias.", false);
                }
            }

            ui.add_space(8.0);
            egui::CollapsingHeader::new(
                egui::RichText::new("Register Custom Cloud Model ID (Advanced)")
                    .small()
                    .color(p.muted),
            )
            .id_salt("settings_register_custom_model_collapse")
            .default_open(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.custom_model_id)
                                .hint_text("gemini-3-flash-preview")
                                .desired_width(240.0)
                                .frame(false),
                        );
                    });
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.custom_model_name)
                                .hint_text("Display alias (optional)")
                                .desired_width(210.0)
                                .frame(false),
                        );
                    });
                    if primary_button(ui, p, "Add & Verify").clicked()
                        && !state.custom_model_id.trim().is_empty()
                    {
                        let alias = opt(&state.custom_model_name);
                        backend.add_custom_model(state.custom_model_id.clone(), alias);
                        state.custom_model_id.clear();
                        state.custom_model_name.clear();
                    }
                });
            });
        },
    );
}

fn fixed_label_col(ui: &mut egui::Ui, width: f32, text: &str, color: Color32) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 26.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, 26.0));
            ui.set_max_width(width);
            ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate());
        },
    );
}

fn fixed_status_col(ui: &mut egui::Ui, p: &CinderPalette, configured: bool) {
    ui.allocate_ui_with_layout(
        egui::vec2(88.0, 26.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(88.0, 26.0));
            ui.set_max_width(88.0);
            status_chip(ui, p, configured);
        },
    );
}

fn key_row(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    label: &str,
    configured: bool,
    buf: &mut String,
    clear_name: &str,
    backend: &Arc<BackendHandle>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        fixed_label_col(ui, 128.0, label, p.text);
        input_frame(p, false).show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(buf)
                    .password(true)
                    .hint_text(if configured {
                        "Configured (enter to replace)"
                    } else {
                        "Not set"
                    })
                    .desired_width(320.0)
                    .frame(false),
            );
        });
        fixed_status_col(ui, p, configured);
        if configured && secondary_button(ui, p, "Clear").clicked() {
            buf.clear();
            backend.save_keys(
                KeysPatch {
                    clear_keys: vec![clear_name.to_string()],
                    ..Default::default()
                },
                false,
            );
        }
    });
    ui.add_space(4.0);
}

fn status_chip(ui: &mut egui::Ui, p: &CinderPalette, configured: bool) {
    if configured {
        badge(ui, "Configured", p.green, p.bg_card);
    } else {
        badge(ui, "Not set", p.muted, p.bg_card);
    }
}

fn render_engine_section(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    setting_group(
        ui,
        p,
        "GGUF Model Vault",
        "Folder scanned for local GGUF models and Hugging Face downloads.",
        |ui| {
            ui.horizontal(|ui| {
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.vault_dir_draft)
                            .desired_width((ui.available_width() - 196.0).max(200.0))
                            .frame(false),
                    );
                });
                if secondary_button(ui, p, "Browse...").clicked() {
                    backend.open_folder_picker(
                        FolderPickTarget::Vault,
                        "Select GGUF Model Vault Folder",
                    );
                }
                if primary_button(ui, p, "Set Vault").clicked() {
                    backend.set_vault_dir(state.vault_dir_draft.clone());
                }
            });

            if !snap.hardware.drives.is_empty() {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for d in &snap.hardware.drives {
                        let lbl = format!(
                            "{} {} free{}",
                            d.name,
                            fmt_bytes(d.free_bytes),
                            if d.is_recommended_for_models {
                                " ★"
                            } else {
                                ""
                            }
                        );
                        if secondary_button(ui, p, &lbl).clicked() {
                            state.vault_dir_draft = std::path::PathBuf::from(&d.mount_point)
                                .join("Project_Bumbo_Models")
                                .to_string_lossy()
                                .to_string();
                        }
                    }
                });
            }
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Inference Defaults & Serving Policy",
        "Tune context window, GPU layer offload, idle unloading, and loopback OpenAI API exposure.",
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(104.0, 26.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_size(egui::vec2(104.0, 26.0));
                        ui.label(egui::RichText::new("Context Window").small().color(p.muted));
                    },
                );
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.ctx_size_draft)
                            .desired_width(68.0)
                            .frame(false),
                    );
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("GPU Layers").small().color(p.muted));
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.ngl_draft)
                            .desired_width(52.0)
                            .frame(false),
                    );
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Idle Unload (m)").small().color(p.muted));
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.idle_mins_draft)
                            .desired_width(48.0)
                            .frame(false),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Apply").clicked() {
                        let ctx_v = state.ctx_size_draft.trim().parse::<u32>().unwrap_or(16384);
                        let ngl_v = state.ngl_draft.trim().parse::<i32>().unwrap_or(99);
                        let idle_v = state.idle_mins_draft.trim().parse::<u64>().unwrap_or(15);
                        backend.update_settings(move |s| {
                            s.local_ctx_size = ctx_v.clamp(2048, 131072);
                            s.local_gpu_layers = ngl_v.clamp(0, 999);
                            s.idle_unload_minutes = idle_v;
                        });
                        state.set_toast("Applied local forge settings.", false);
                    }
                });
            });

            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("VRAM Presets:").small().color(p.muted));
                let cur_ctx = snap.settings.local_ctx_size;
                for &(val, label) in &[
                    (8192_u32, "8k"),
                    (16384_u32, "16k"),
                    (32768_u32, "32k (3080 Ti ★)"),
                    (49152_u32, "48k"),
                    (65536_u32, "64k"),
                ] {
                    let clicked = if cur_ctx == val {
                        primary_button(ui, p, label).clicked()
                    } else {
                        secondary_button(ui, p, label).clicked()
                    };
                    if clicked {
                        state.ctx_size_draft = val.to_string();
                        backend.update_settings(move |s| {
                            s.local_ctx_size = val;
                        });
                        state.set_toast(
                            format!("Context window set to {val} tokens (effective on next model load)."),
                            false,
                        );
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "llama-server launches with Flash Attention (-fa on) + q8_0 KV-cache (~50% less KV VRAM): 12 GB GPUs comfortably run 32k–48k on 12B or 48k–64k on 9B models.",
                )
                .small()
                .color(p.muted),
            );

            ui.add_space(8.0);
            let y = ui.cursor().top();
            ui.painter()
                .hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, p.border));
            ui.add_space(4.0);

            let mut unload_after = snap.settings.ui.unload_after_turn;
            setting_row_item(
                ui,
                p,
                "Auto-quench local model after every reply",
                "Immediately release GPU VRAM when a local response completes.",
                true,
                |ui| {
                    if toggle_switch(ui, p, &mut unload_after).changed() {
                        let mut next_ui = snap.settings.ui.clone();
                        next_ui.unload_after_turn = unload_after;
                        backend.save_ui_and_orch(Some(next_ui), None);
                    }
                },
            );

            let mut expose = snap.settings.expose_local_api;
            setting_row_item(
                ui,
                p,
                "Expose local loopback OpenAI API",
                "Serve OpenAI-compatible HTTP endpoints on 127.0.0.1 without an auth token.",
                false,
                |ui| {
                    if toggle_switch(ui, p, &mut expose).changed() {
                        backend.update_settings(move |s| {
                            s.expose_local_api = expose;
                        });
                    }
                },
            );
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "llama-server Engine Runtime",
        "Manage bundled llama.cpp binaries and detect running Ollama loopback endpoints.",
        |ui| {
            let eng_lbl = if snap.engine_status.llama_server_installed {
                format!(
                    "Installed: {}",
                    snap.engine_status
                        .llama_server_path
                        .as_deref()
                        .unwrap_or("llama-server")
                )
            } else {
                snap.engine_status.install_message.clone()
            };
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&eng_lbl).small().color(p.text));
                    if snap.engine_status.ollama_loopback_detected {
                        ui.label(
                            egui::RichText::new("Ollama loopback endpoint detected on 127.0.0.1:11434")
                                .small()
                                .color(p.green),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, p, "Install Vulkan").clicked() {
                        backend.install_engine("vulkan".into());
                    }
                    if primary_button(ui, p, "Install CUDA 12").clicked() {
                        backend.install_engine("cuda".into());
                    }
                });
            });
        },
    );
}

fn render_agent_section(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    setting_group(
        ui,
        p,
        "Network Posture & Tool Approvals",
        "Control outbound sockets and workspace confirmation prompts.",
        |ui| {
            let is_hybrid = snap.settings.privacy_mode == PrivacyMode::Hybrid;
            setting_row_item(
                ui,
                p,
                "Network Mode",
                "Read-only status. Switch between Strict Local and Hybrid exclusively via the bottom-left System & Forge Telemetry popover.",
                true,
                |ui| {
                    badge(
                        ui,
                        if is_hybrid { "Hybrid" } else { "Strict Local" },
                        if is_hybrid { p.amber } else { p.green },
                        p.bg_card,
                    );
                },
            );

            let mut auto_edits = snap.settings.auto_approve_edits;
            setting_row_item(
                ui,
                p,
                "Auto-approve workspace edits",
                "Write or patch files inside the active workspace without pausing for confirmation.",
                true,
                |ui| {
                    if toggle_switch(ui, p, &mut auto_edits).changed() {
                        backend.update_settings(move |s| {
                            s.auto_approve_edits = auto_edits;
                        });
                    }
                },
            );

            let mut auto_cmds = snap.settings.auto_approve_commands;
            setting_row_item(
                ui,
                p,
                "Auto-approve workspace commands",
                "Run non-destructive build/test shell commands in the active workspace automatically.",
                false,
                |ui| {
                    if toggle_switch(ui, p, &mut auto_cmds).changed() {
                        backend.update_settings(move |s| {
                            s.auto_approve_commands = auto_cmds;
                        });
                    }
                },
            );
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Orchestrator & Sub-Agent Models",
        "Pair a dedicated Planner, Executor, and Vision Sub-Agent (for OCR, screenshots, and images).",
        |ui| {
            let models = state.selectable_models(true);
            let mut orch = snap.settings.orchestrator.clone();
            let mut orch_changed = false;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                fixed_label_col(ui, 128.0, "Planner Model", p.text);
                let cur_planner_lbl = if orch.planner_model_id.trim().is_empty()
                    || orch.planner_model_id.eq_ignore_ascii_case("auto")
                {
                    "Auto (Best Planner Model)".to_string()
                } else {
                    crate::app_state::resolve_model_display_name(snap, &orch.planner_model_id)
                };
                egui::ComboBox::from_id_salt("settings_orch_planner")
                    .selected_text(cur_planner_lbl)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(
                                orch.planner_model_id.trim().is_empty()
                                    || orch.planner_model_id.eq_ignore_ascii_case("auto"),
                                "Auto (Best Planner Model)",
                            )
                            .clicked()
                        {
                            orch.planner_model_id.clear();
                            orch_changed = true;
                        }
                        for m in &models {
                            let lbl = format!("[{}] {}", m.provider_label, m.display_name);
                            if ui
                                .selectable_label(orch.planner_model_id == m.id, lbl)
                                .clicked()
                            {
                                orch.planner_model_id = m.id.clone();
                                orch_changed = true;
                            }
                        }
                    });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                fixed_label_col(ui, 128.0, "Executor Model", p.text);
                let cur_exec_lbl = if orch.executor_model_id.trim().is_empty()
                    || orch.executor_model_id.eq_ignore_ascii_case("auto")
                {
                    "Auto (Best Execution Model)".to_string()
                } else {
                    crate::app_state::resolve_model_display_name(snap, &orch.executor_model_id)
                };
                egui::ComboBox::from_id_salt("settings_orch_executor")
                    .selected_text(cur_exec_lbl)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(
                                orch.executor_model_id.trim().is_empty()
                                    || orch.executor_model_id.eq_ignore_ascii_case("auto"),
                                "Auto (Best Execution Model)",
                            )
                            .clicked()
                        {
                            orch.executor_model_id.clear();
                            orch_changed = true;
                        }
                        for m in &models {
                            let lbl = format!("[{}] {}", m.provider_label, m.display_name);
                            if ui
                                .selectable_label(orch.executor_model_id == m.id, lbl)
                                .clicked()
                            {
                                orch.executor_model_id = m.id.clone();
                                orch_changed = true;
                            }
                        }
                    });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                fixed_label_col(ui, 128.0, "Vision Sub-Agent", p.text);
                let cur_vision_lbl = if orch.vision_model_id.trim().is_empty() {
                    "Auto (Best Vision Model)".to_string()
                } else {
                    crate::app_state::resolve_model_display_name(snap, &orch.vision_model_id)
                };
                egui::ComboBox::from_id_salt("settings_orch_vision")
                    .selected_text(cur_vision_lbl)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(
                                orch.vision_model_id.trim().is_empty(),
                                "Auto (Best Vision Model)",
                            )
                            .clicked()
                        {
                            orch.vision_model_id.clear();
                            orch_changed = true;
                        }
                        for m in &models {
                            let lbl = format!("[{}] {}", m.provider_label, m.display_name);
                            if ui
                                .selectable_label(orch.vision_model_id == m.id, lbl)
                                .clicked()
                            {
                                orch.vision_model_id = m.id.clone();
                                orch_changed = true;
                            }
                        }
                    });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                fixed_label_col(ui, 128.0, "Max Auto Steps", p.text);
                if ui
                    .add(
                        egui::Slider::new(&mut orch.max_autonomous_steps, 2..=32)
                            .text("steps"),
                    )
                    .on_hover_text(
                        "Autonomous tool steps allowed per turn before asking to grant additional contingent",
                    )
                    .changed()
                {
                    orch_changed = true;
                }
            });

            ui.add_space(6.0);
            let y = ui.cursor().top();
            ui.painter()
                .hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, p.border));
            ui.add_space(4.0);

            setting_row_item(
                ui,
                p,
                "Pause for Plan Review before Executor runs",
                "Require approval of the generated plan before applying code edits.",
                true,
                |ui| {
                    if toggle_switch(ui, p, &mut orch.review_before_execute).changed() {
                        orch_changed = true;
                    }
                },
            );
            setting_row_item(
                ui,
                p,
                "Auto-verify workspace after edits",
                "Automatically run workspace build/check hooks when the Executor finishes.",
                false,
                |ui| {
                    if toggle_switch(ui, p, &mut orch.auto_verify_after_edits).changed() {
                        orch_changed = true;
                    }
                },
            );

            if orch_changed {
                backend.save_ui_and_orch(None, Some(orch));
            }
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Enabled Runtime Modules",
        "Control which agent tool capabilities are exposed to models.",
        |ui| {
            let mut enabled = snap.settings.ui.enabled_modules.clone();
            let mut chg = false;
            let len = HARNESS_MODULE_DEFS.len();
            for (idx, (mid, title, desc)) in HARNESS_MODULE_DEFS.iter().enumerate() {
                let mut on = enabled.iter().any(|x| x == mid);
                let with_div = idx + 1 < len;
                setting_row_item(ui, p, title, desc, with_div, |ui| {
                    if toggle_switch(ui, p, &mut on).changed() {
                        chg = true;
                        if on {
                            if !enabled.iter().any(|x| x == mid) {
                                enabled.push((*mid).to_string());
                            }
                        } else {
                            enabled.retain(|x| x != mid);
                        }
                    }
                });
            }
            if chg {
                let mut u = snap.settings.ui.clone();
                u.enabled_modules = enabled;
                backend.save_ui_and_orch(Some(u), None);
            }
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Global Custom Instructions",
        "Appended automatically to every conversation system prompt.",
        |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Save Instructions").clicked() {
                        let mut u = snap.settings.ui.clone();
                        u.custom_system_prompt = state.custom_sys_draft.clone();
                        backend.save_ui_and_orch(Some(u), None);
                        state.set_toast("Saved custom instructions.", false);
                    }
                });
            });
            ui.add_space(6.0);
            input_frame(p, false).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut state.custom_sys_draft)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text("Appended to every conversation...")
                        .frame(false),
                );
            });
        },
    );
}

fn render_privacy_section(
    ui: &mut egui::Ui,
    backend: &Arc<BackendHandle>,
    snap: &crate::backend::UiSnapshot,
    p: &CinderPalette,
) {
    let audit = &snap.privacy_audit;

    p.card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let mode_txt = if audit.privacy_mode == PrivacyMode::StrictLocal {
                "Strict Local (0 External Calls)"
            } else {
                "Hybrid (Cloud Allowed)"
            };
            badge(
                ui,
                mode_txt,
                if audit.privacy_mode == PrivacyMode::StrictLocal {
                    p.green
                } else {
                    p.amber
                },
                p.bg_card,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, p, "Clear Audit Log").clicked() {
                    backend.clear_audit();
                }
            });
        });
    });

    ui.add_space(12.0);

    ui.columns(3, |cols| {
        metric_card(
            &mut cols[0],
            p,
            "Loopback Calls",
            audit.total_loopback_requests,
            p.green,
        );
        metric_card(
            &mut cols[1],
            p,
            "External Allowed",
            audit.external_requests_allowed,
            p.amber,
        );
        metric_card(
            &mut cols[2],
            p,
            "Blocked Outbound",
            audit.external_requests_blocked,
            p.red,
        );
    });

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Network Audit Log",
        "Every outbound and local loopback request mediated by NetworkGate.",
        |ui| {
            if audit.entries.is_empty() {
                empty_state_block(
                    ui,
                    p,
                    Icon::Shield,
                    "No network requests logged yet",
                    "Local inference and outbound HTTP requests will appear here with allow/block verdicts.",
                    None,
                );
            } else {
                for (idx, ent) in audit.entries.iter().take(60).enumerate() {
                    if idx > 0 {
                        ui.add_space(4.0);
                    }
                    let col = match ent.outcome {
                        AuditOutcome::AllowedLoopback => p.green,
                        AuditOutcome::AllowedExternal => p.amber,
                        AuditOutcome::BlockedByStrictLocal => p.red,
                    };
                    p.subcard_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            badge(ui, &format!("{:?}", ent.outcome), col, p.bg_elev);
                            ui.label(egui::RichText::new(&ent.method).strong());
                            ui.label(egui::RichText::new(&ent.purpose).color(p.muted));
                        });
                        ui.label(
                            egui::RichText::new(&ent.url)
                                .monospace()
                                .small()
                                .color(p.muted),
                        );
                    });
                }
            }
        },
    );
}

fn metric_card(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    label: &str,
    count: u64,
    accent_col: Color32,
) {
    p.card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(label).small().color(p.muted));
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(format!("{count}"))
                .size(p.base_font_size + 7.0)
                .strong()
                .color(accent_col),
        );
    });
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
