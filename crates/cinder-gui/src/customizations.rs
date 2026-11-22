use crate::app_state::{AutomationPlaybook, CustomTab, GuiState, MainView, SettingsTab};
use crate::backend::{BackendHandle, FolderPickTarget};
use crate::icons::Icon;
use crate::plugins;
use crate::theme::{
    badge, danger_button, empty_state_block, ghost_button, input_frame, pane_header,
    primary_button, secondary_button, segmented_control, setting_group, toggle_switch,
    underline_tab, CinderPalette,
};
use harness_store::{AgentMode, SkillDefinition};
use std::path::PathBuf;
use std::sync::Arc;

pub const HARNESS_MODULE_DEFS: &[(&str, &str, &str)] = &[
    ("fs_read", "Filesystem Read", "list_dir, read_file, search_files, PDF extract"),
    ("fs_write", "Filesystem Write", "write_file, edit_file with TurnCheckpoints"),
    ("shell", "Shell & Verification", "run_command & verify_workspace inside project"),
    ("rag", "Offline Codex RAG", "BM25 offline search_knowledge index"),
    ("subagents", "Research Subagents", "delegate_research isolated context helper"),
    ("web", "Brave Web Search", "web_search (blocked under Strict Local mode)"),
];

pub fn render_customizations_page(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
    tab: CustomTab,
) {
    // Exact 40px aligned pane header
    pane_header(ui, p, |ui| {
        ui.label(
            egui::RichText::new("Customizations")
                .strong()
                .color(p.text),
        );
        ui.add_space(10.0);

        for (target, lbl) in [
            (CustomTab::Plugins, "Plugins & Deps"),
            (CustomTab::Skills, "Skills & Codex"),
            (CustomTab::Mcp, "MCP Servers"),
            (CustomTab::Rules, "Rules & Projects"),
        ] {
            if underline_tab(ui, p, tab == target, lbl).clicked() {
                state.switch_main_view(MainView::Customizations(target), true);
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, p, "← Chat").clicked() {
                state.switch_main_view(MainView::Chat, true);
            }
            if matches!(tab, CustomTab::Plugins | CustomTab::Mcp) {
                if secondary_button(ui, p, "Export MCP").clicked() {
                    backend.export_antigravity_mcp();
                }
                if primary_button(ui, p, "Sync Antigravity").clicked() {
                    backend.sync_antigravity();
                }
            }
        });
    });

    let avail_w = ui.available_width();
    let col_w = (avail_w - 48.0).clamp(360.0, 920.0);
    let side_pad = ((avail_w - col_w) * 0.5).max(24.0);

    egui::ScrollArea::vertical()
        .id_salt(("cust_page_scroll", format!("{tab:?}")))
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
                            CustomTab::Plugins => {
                                plugins::render_plugins_body(ctx, ui, state, backend, p);
                            }
                            CustomTab::Skills => render_skills_tab(ui, state, backend, p),
                            CustomTab::Mcp => render_mcp_tab(ui, state, backend, p),
                            CustomTab::Rules => render_rules_tab(ui, state, backend, p),
                        }
                        ui.add_space(28.0);
                    },
                );
            });
        });
}

fn render_skills_tab(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        return;
    };

    setting_group(
        ui,
        p,
        "Offline Knowledge Index (Codex RAG)",
        "BM25 local full-text search across source files inside your active project.",
        |ui| {
            ui.horizontal(|ui| {
                if let Some(ref st) = state.kb_stats {
                    ui.label(
                        egui::RichText::new(format!(
                            "Indexed {} files · {} chunks",
                            st.total_files, st.total_chunks
                        ))
                        .small()
                        .color(p.green),
                    );
                    if st.has_dense_embeddings {
                        badge(
                            ui,
                            &format!(
                                "Hybrid BM25 + {}",
                                st.embedding_model.as_deref().unwrap_or("Local Embed")
                            ),
                            p.green,
                            p.bg_elev,
                        );
                    } else {
                        badge(ui, "BM25 Keyword Index", p.muted, p.bg_elev);
                    }
                } else {
                    ui.label(
                        egui::RichText::new("Build an offline index for fast local retrieval.")
                            .small()
                            .color(p.muted),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Index Active Workspace Offline").clicked() {
                        backend.index_kb();
                    }
                });
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.kb_query)
                            .hint_text("Search offline knowledge index...")
                            .desired_width((ui.available_width() - 96.0).max(180.0))
                            .frame(false),
                    );
                });
                if secondary_button(ui, p, "Search").clicked() && !state.kb_query.trim().is_empty()
                {
                    backend.search_kb(state.kb_query.clone());
                }
            });

            for hit in &state.kb_hits {
                ui.add_space(6.0);
                p.subcard_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&hit.rel_path).monospace().strong());
                        badge(ui, &hit.location, p.amber, p.bg_elev);
                    });
                    ui.label(egui::RichText::new(&hit.snippet).small().color(p.muted));
                });
            }
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Slash Skills (/skill)",
        "Reusable prompt workflows triggered via `/slug` in the chat composer.",
        |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} installed skills", snap.skills.len()))
                        .small()
                        .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let lbl = if state.show_new_skill_form {
                        "Close Form"
                    } else {
                        "+ New Custom Skill"
                    };
                    if secondary_button(ui, p, lbl).clicked() {
                        state.show_new_skill_form = !state.show_new_skill_form;
                    }
                });
            });

            if state.show_new_skill_form {
                ui.add_space(8.0);
                p.subcard_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new("Create Custom Workspace Skill").strong());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        input_frame(p, false).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut state.skill_slug)
                                    .hint_text("slug (e.g. proof)")
                                    .desired_width(130.0)
                                    .frame(false),
                            );
                        });
                        input_frame(p, false).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut state.skill_name)
                                    .hint_text("Display Name")
                                    .desired_width(160.0)
                                    .frame(false),
                            );
                        });
                        egui::ComboBox::from_id_salt("skill_mode_combo")
                            .selected_text(format!("{:?}", state.skill_mode))
                            .show_ui(ui, |ui| {
                                for m in [
                                    AgentMode::Auto,
                                    AgentMode::Plan,
                                    AgentMode::Agent,
                                    AgentMode::Orchestrator,
                                    AgentMode::Chat,
                                ] {
                                    ui.selectable_value(&mut state.skill_mode, m, format!("{:?}", m));
                                }
                            });
                    });
                    ui.add_space(6.0);
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.skill_desc)
                                .hint_text("Short description...")
                                .desired_width(f32::INFINITY)
                                .frame(false),
                        );
                    });
                    ui.add_space(6.0);
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut state.skill_prompt)
                                .hint_text("Prompt template (use {input})...")
                                .desired_rows(3)
                                .desired_width(f32::INFINITY)
                                .frame(false),
                        );
                    });
                    ui.add_space(8.0);
                    if primary_button(ui, p, "Save Custom Skill").clicked() {
                        let slug = state.skill_slug.trim().to_string();
                        if !slug.is_empty() {
                            let name = if state.skill_name.trim().is_empty() {
                                slug.clone()
                            } else {
                                state.skill_name.trim().to_string()
                            };
                            backend.save_skill(SkillDefinition {
                                slug,
                                name,
                                description: state.skill_desc.clone(),
                                prompt_template: state.skill_prompt.clone(),
                                recommended_mode: state.skill_mode,
                                is_builtin: false,
                            });
                            state.skill_slug.clear();
                            state.skill_name.clear();
                            state.skill_desc.clear();
                            state.skill_prompt.clear();
                            state.show_new_skill_form = false;
                        }
                    }
                });
            }

            ui.add_space(8.0);
            for sk in &snap.skills {
                p.subcard_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("/{}", sk.slug))
                                .monospace()
                                .strong()
                                .color(p.accent_hot),
                        );
                        ui.label(egui::RichText::new(&sk.name).strong());
                        badge(
                            ui,
                            &format!("{:?}", sk.recommended_mode),
                            if sk.is_builtin { p.muted } else { p.amber },
                            p.bg_elev,
                        );
                    });
                    ui.label(egui::RichText::new(&sk.description).small().color(p.muted));
                });
                ui.add_space(4.0);
            }
        },
    );
}

fn render_mcp_tab(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        return;
    };

    setting_group(
        ui,
        p,
        "Registered MCP Servers",
        "Inspect live tool schemas, toggle availability, or import Antigravity servers.",
        |ui| {
            if snap.mcp_servers.is_empty() {
                if empty_state_block(
                    ui,
                    p,
                    Icon::Blocks,
                    "No MCP servers registered",
                    "Install pre-verified MCP servers in Plugins & Deps or add a custom stdio/HTTP server below.",
                    Some("Browse Curated Plugins"),
                ) {
                    state.switch_main_view(MainView::Customizations(CustomTab::Plugins), true);
                }
            } else {
                for (idx, s) in snap.mcp_servers.iter().enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    p.subcard_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&s.name).strong().color(p.text));
                            badge(ui, &s.transport, p.muted, p.bg_elev);
                            badge(ui, &s.source, p.purple, p.bg_elev);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if danger_button(ui, p, "Remove").clicked() {
                                        backend.remove_mcp(s.id.clone());
                                    }
                                    if secondary_button(ui, p, "Test Tools").clicked() {
                                        backend.probe_mcp(s.id.clone());
                                    }
                                    let mut en = s.enabled;
                                    if toggle_switch(ui, p, &mut en).changed() {
                                        backend.toggle_mcp(s.id.clone(), en);
                                    }
                                },
                            );
                        });
                        ui.label(
                            egui::RichText::new(&s.target)
                                .monospace()
                                .small()
                                .color(p.muted),
                        );
                        if let Some(probe) = state.mcp_probes.get(&s.id) {
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                if probe.ok {
                                    badge(
                                        ui,
                                        &format!("{} · {}ms", probe.server_info, probe.latency_ms),
                                        p.muted,
                                        p.bg_elev,
                                    );
                                    for t in &probe.tools {
                                        badge(ui, &t.tool_name, p.green, p.bg_elev);
                                    }
                                } else {
                                    ui.label(
                                        egui::RichText::new(
                                            probe.error.as_deref().unwrap_or("Error"),
                                        )
                                        .small()
                                        .color(p.red),
                                    );
                                }
                            });
                        }
                    });
                }
            }
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Add Custom MCP Server",
        "Connect a local stdio command or remote Streamable HTTP MCP endpoint.",
        |ui| {
            ui.horizontal(|ui| {
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.mcp_name)
                            .hint_text("Server name")
                            .desired_width(130.0)
                            .frame(false),
                    );
                });
                egui::ComboBox::from_id_salt("cust_mcp_trans")
                    .selected_text(&state.mcp_transport)
                    .width(74.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut state.mcp_transport, "stdio".into(), "stdio");
                        ui.selectable_value(&mut state.mcp_transport, "http".into(), "http");
                    });
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.mcp_target)
                            .hint_text("Command or URL...")
                            .desired_width((ui.available_width() - 150.0).max(180.0))
                            .frame(false),
                    );
                });
                if primary_button(ui, p, "Add Server").clicked()
                    && !state.mcp_name.trim().is_empty()
                    && !state.mcp_target.trim().is_empty()
                {
                    backend.save_mcp(
                        state.mcp_name.clone(),
                        state.mcp_transport.clone(),
                        state.mcp_target.clone(),
                    );
                    state.mcp_name.clear();
                    state.mcp_target.clear();
                }
            });
        },
    );

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, p, "Configure Local OpenAI API exposure in Settings →")
                .clicked()
            {
                state.open_settings(SettingsTab::Engine);
            }
        });
    });
}

fn render_rules_tab(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        return;
    };

    setting_group(
        ui,
        p,
        "Registered Workspaces & Projects",
        "Manage indexed project roots shown in the sidebar.",
        |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} registered projects", snap.workspaces.len()))
                        .small()
                        .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Browse Folder...").clicked() {
                        backend.open_folder_picker(
                            FolderPickTarget::Workspace,
                            "Select Project Folder for Cinder",
                        );
                    }
                });
            });
            ui.add_space(8.0);
            for (idx, ws) in snap.workspaces.iter().enumerate() {
                if idx > 0 {
                    ui.add_space(4.0);
                }
                let active = ws.path == snap.settings.active_workspace_path;
                p.subcard_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&ws.name).strong());
                        if active {
                            badge(ui, "Active", p.green, p.bg_elev);
                        }
                        ui.label(egui::RichText::new(&ws.path).small().color(p.muted));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if snap.workspaces.len() > 1
                                    && secondary_button(ui, p, "Remove").clicked()
                                {
                                    backend.remove_workspace(ws.path.clone());
                                }
                                if secondary_button(ui, p, "Reveal").clicked() {
                                    backend.reveal_entry(PathBuf::from(&ws.path));
                                }
                                if !active && secondary_button(ui, p, "Open").clicked() {
                                    backend.select_workspace(ws.path.clone());
                                }
                            },
                        );
                    });
                });
            }
        },
    );

    ui.add_space(16.0);

    setting_group(
        ui,
        p,
        "Project System Rules (.cinder/rules.md)",
        "Persistent project conventions automatically included in project conversations.",
        |ui| {
            ui.horizontal(|ui| {
                badge(
                    ui,
                    &snap.settings.active_workspace_path,
                    p.accent_hot,
                    p.accent_soft,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Save Rules").clicked() {
                        state.rules_dirty = false;
                        backend.save_rules(state.rules_draft.clone());
                    }
                });
            });
            ui.add_space(6.0);
            input_frame(p, false).show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut state.rules_draft)
                        .desired_rows(7)
                        .desired_width(f32::INFINITY)
                        .hint_text("Add persistent workspace instructions...")
                        .frame(false),
                );
                if r.changed() {
                    state.rules_dirty = true;
                }
            });
        },
    );
}

fn render_automation_subcard_row(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    subtitle: &str,
    hover_prompt: Option<&str>,
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
        let resp = ui.add(
            egui::Label::new(egui::RichText::new(subtitle).small().color(p.muted)).truncate(),
        );
        if let Some(hp) = hover_prompt {
            resp.on_hover_text(hp);
        }
    });
}

fn render_automation_left_title_and_badges(
    ui: &mut egui::Ui,
    avail_w: f32,
    badge_reserve: f32,
    title: &str,
    p: &CinderPalette,
    add_badges: impl FnOnce(&mut egui::Ui),
) {
    let w = avail_w.max(40.0);
    let max_title_w = (w - badge_reserve).max(72.0);
    ui.allocate_ui_with_layout(
        egui::vec2(w, 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let clip = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            ui.set_clip_rect(clip);
            ui.scope(|ui| {
                ui.set_max_width(max_title_w);
                ui.add(
                    egui::Label::new(egui::RichText::new(title).strong().color(p.text)).truncate(),
                );
            });
            add_badges(ui);
        },
    );
}

pub fn render_automations_page(
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

    // Exact 40px aligned pane header matching Forge & Customizations
    pane_header(ui, p, |ui| {
        ui.label(egui::RichText::new("Automations").strong().color(p.text));
        ui.add_space(6.0);
        badge(ui, "v5.0 Playbooks", p.accent_hot, p.bg_card);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, p, "← Chat").clicked() {
                state.switch_main_view(MainView::Chat, true);
            }
            let form_lbl = if state.show_new_automation_form {
                "Close Form"
            } else {
                "+ New Automation"
            };
            if primary_button(ui, p, form_lbl).clicked() {
                state.show_new_automation_form = !state.show_new_automation_form;
            }
        });
    });

    let avail_w = ui.available_width();
    let col_w = (avail_w - 48.0).clamp(360.0, 920.0);
    let side_pad = ((avail_w - col_w) * 0.5).max(24.0);

    egui::ScrollArea::vertical()
        .id_salt("automations_page_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(side_pad);
                ui.allocate_ui_with_layout(
                    egui::vec2(col_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if let Some((ref title, ref status)) = state.last_automation_run.clone() {
                            p.subcard_frame().show(ui, |ui| {
                                let w = ui.available_width();
                                ui.set_width(w);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(w, 24.0),
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ghost_button(ui, p, "Dismiss").clicked() {
                                            state.last_automation_run = None;
                                        }
                                        render_automation_left_title_and_badges(
                                            ui,
                                            ui.available_width(),
                                            110.0,
                                            &format!("Last run: {title}"),
                                            p,
                                            |ui| {
                                                badge(ui, status, p.green, p.bg_elev);
                                            },
                                        );
                                    },
                                );
                            });
                            ui.add_space(12.0);
                        }

                        if state.show_new_automation_form {
                            setting_group(
                                ui,
                                p,
                                "Create Custom Automation Playbook",
                                "Define reusable one-click workflows tailored to your courses or repositories.",
                                |ui| {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new("Title").small().color(p.muted),
                                            );
                                            input_frame(p, false).show(ui, |ui| {
                                                ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut state.auto_title_draft,
                                                    )
                                                    .hint_text("e.g. Weekly Lecture Flashcards")
                                                    .desired_width((col_w * 0.48).max(160.0))
                                                    .frame(false),
                                                );
                                            });
                                        });
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new("Category")
                                                    .small()
                                                    .color(p.muted),
                                            );
                                            input_frame(p, false).show(ui, |ui| {
                                                ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut state.auto_category_draft,
                                                    )
                                                    .hint_text("Uni Helper / Engineering")
                                                    .desired_width(180.0)
                                                    .frame(false),
                                                );
                                            });
                                        });
                                    });
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new("Short Description")
                                            .small()
                                            .color(p.muted),
                                    );
                                    input_frame(p, false).show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut state.auto_desc_draft)
                                                .hint_text(
                                                    "One-line summary shown on the playbook card...",
                                                )
                                                .desired_width(f32::INFINITY)
                                                .frame(false),
                                        );
                                    });
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("Execution Mode:")
                                                .small()
                                                .color(p.muted),
                                        );
                                        if let Some(m) = segmented_control(
                                            ui,
                                            p,
                                            "auto_mode_seg",
                                            state.auto_mode_draft,
                                            &[
                                                (AgentMode::Agent, "Agent"),
                                                (AgentMode::Plan, "Plan"),
                                                (AgentMode::Auto, "Auto"),
                                            ],
                                        ) {
                                            state.auto_mode_draft = m;
                                        }
                                    });
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new("Prompt Template")
                                            .small()
                                            .color(p.muted),
                                    );
                                    input_frame(p, false).show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::multiline(
                                                &mut state.auto_prompt_draft,
                                            )
                                            .desired_rows(3)
                                            .desired_width(f32::INFINITY)
                                            .hint_text(
                                                "Instructions executed when Run in Active Project is clicked...",
                                            )
                                            .frame(false),
                                        );
                                    });
                                    ui.add_space(8.0);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if primary_button(ui, p, "Save Automation").clicked() {
                                                let title = state.auto_title_draft.trim().to_string();
                                                let prompt =
                                                    state.auto_prompt_draft.trim().to_string();
                                                if title.is_empty() || prompt.is_empty() {
                                                    state.set_toast(
                                                        "Provide both a title and prompt template.",
                                                        true,
                                                    );
                                                } else {
                                                    let cat = if state
                                                        .auto_category_draft
                                                        .trim()
                                                        .is_empty()
                                                    {
                                                        "Custom".to_string()
                                                    } else {
                                                        state.auto_category_draft.trim().to_string()
                                                    };
                                                    let desc = if state
                                                        .auto_desc_draft
                                                        .trim()
                                                        .is_empty()
                                                    {
                                                        "Custom automation workflow".to_string()
                                                    } else {
                                                        state.auto_desc_draft.trim().to_string()
                                                    };
                                                    state.automations.insert(
                                                        0,
                                                        AutomationPlaybook {
                                                            id: format!(
                                                                "custom_{}",
                                                                uuid::Uuid::new_v4()
                                                            ),
                                                            title: title.clone(),
                                                            category: cat,
                                                            description: desc,
                                                            prompt,
                                                            mode: state.auto_mode_draft,
                                                            is_builtin: false,
                                                        },
                                                    );
                                                    state.auto_title_draft.clear();
                                                    state.auto_desc_draft.clear();
                                                    state.auto_prompt_draft.clear();
                                                    state.show_new_automation_form = false;
                                                    state.set_toast(
                                                        format!("Saved automation '{title}'."),
                                                        false,
                                                    );
                                                }
                                            }
                                        },
                                    );
                                },
                            );
                            ui.add_space(16.0);
                        }

                        // Section 1: Standing Workspace Automations
                        setting_group(
                            ui,
                            p,
                            "Standing Workspace Automations",
                            "Background conveniences active across your projects and local Forge.",
                            |ui| {
                                let kb_sub = if let Some(ref st) = state.kb_stats {
                                    format!(
                                        "Active project: {} ({} files · {} chunks indexed)",
                                        snap.settings.active_workspace_path,
                                        st.total_files,
                                        st.total_chunks
                                    )
                                } else {
                                    format!(
                                        "Builds local BM25 + embedding retrieval index for {}",
                                        snap.settings.active_workspace_path
                                    )
                                };
                                render_automation_subcard_row(ui, p, &kb_sub, None, |ui| {
                                    if secondary_button(ui, p, "Index Now").clicked() {
                                        backend.index_kb();
                                    }
                                    toggle_switch(ui, p, &mut state.auto_index_on_workspace_switch);
                                    render_automation_left_title_and_badges(
                                        ui,
                                        ui.available_width(),
                                        110.0,
                                        "Auto-Index Project Knowledge (Offline Codex RAG)",
                                        p,
                                        |ui| {
                                            badge(ui, "RAG", p.green, p.bg_elev);
                                        },
                                    );
                                });

                                ui.add_space(6.0);

                                let mut unload_after = snap.settings.ui.unload_after_turn;
                                render_automation_subcard_row(
                                    ui,
                                    p,
                                    "Automatically quenches resident GGUF weights after each response to keep GPU VRAM free.",
                                    None,
                                    |ui| {
                                        if toggle_switch(ui, p, &mut unload_after).changed() {
                                            let mut next_ui = snap.settings.ui.clone();
                                            next_ui.unload_after_turn = unload_after;
                                            backend.save_ui_and_orch(Some(next_ui), None);
                                        }
                                        render_automation_left_title_and_badges(
                                            ui,
                                            ui.available_width(),
                                            110.0,
                                            "Auto-Quench Local GPU VRAM After Reply",
                                            p,
                                            |ui| {
                                                badge(ui, "Forge", p.accent_hot, p.bg_elev);
                                            },
                                        );
                                    },
                                );
                            },
                        );

                        ui.add_space(16.0);

                        // Section 2: Uni & Engineering Workflow Playbooks
                        setting_group(
                            ui,
                            p,
                            "Uni & Engineering Workflow Playbooks",
                            &format!(
                                "One-click structured workflows dispatched straight into your active project ({}).",
                                snap.settings.active_workspace_path
                            ),
                            |ui| {
                                let playbooks = state.automations.clone();
                                let mut remove_id: Option<String> = None;
                                let mut trigger_pb: Option<AutomationPlaybook> = None;

                                for (idx, pb) in playbooks.iter().enumerate() {
                                    if idx > 0 {
                                        ui.add_space(6.0);
                                    }
                                    let (mode_lbl, mode_col) = match pb.mode {
                                        AgentMode::Agent | AgentMode::Orchestrator => {
                                            ("AGENT", p.accent_hot)
                                        }
                                        AgentMode::Plan => ("PLAN", p.purple),
                                        _ => ("AUTO", p.green),
                                    };
                                    let cat_col = if pb.category == "Uni Helper" {
                                        p.green
                                    } else if pb.category == "Engineering" {
                                        p.amber
                                    } else {
                                        p.muted
                                    };
                                    render_automation_subcard_row(
                                        ui,
                                        p,
                                        &pb.description,
                                        Some(&pb.prompt),
                                        |ui| {
                                            if primary_button(ui, p, "Run in Active Project")
                                                .clicked()
                                            {
                                                trigger_pb = Some(pb.clone());
                                            }
                                            if !pb.is_builtin
                                                && ghost_button(ui, p, "Delete").clicked()
                                            {
                                                remove_id = Some(pb.id.clone());
                                            }
                                            render_automation_left_title_and_badges(
                                                ui,
                                                ui.available_width(),
                                                180.0,
                                                &pb.title,
                                                p,
                                                |ui| {
                                                    badge(ui, &pb.category, cat_col, p.bg_elev);
                                                    badge(ui, mode_lbl, mode_col, p.bg_elev);
                                                },
                                            );
                                        },
                                    );
                                }

                                if let Some(rid) = remove_id {
                                    state.automations.retain(|a| a.id != rid);
                                }
                                if let Some(pb) = trigger_pb {
                                    let sid = snap.settings.active_session_id.clone();
                                    if sid.is_empty() {
                                        state.set_toast("No active chat session ready.", true);
                                    } else {
                                        crate::chat::dispatch_chat_prompt(
                                            ctx,
                                            state,
                                            backend,
                                            &sid,
                                            pb.mode,
                                            &snap.settings.active_model_id,
                                            &snap.settings.compare_model_b_id,
                                            pb.prompt.clone(),
                                            Vec::new(),
                                            Vec::new(),
                                        );
                                        state.last_automation_run =
                                            Some((pb.title, "Running in Chat".into()));
                                        state.switch_main_view(MainView::Chat, true);
                                    }
                                }
                            },
                        );

                        ui.add_space(28.0);
                    },
                );
            });
        });
}
