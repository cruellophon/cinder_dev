use crate::app_state::GuiState;
use crate::backend::BackendHandle;
use crate::theme::{
    badge, copy_to_clipboard, danger_button, input_frame, primary_button, secondary_button,
    segmented_control, setting_group, toggle_switch, CinderPalette,
};
use harness_net::PrivacyMode;
use std::path::Path;
use std::sync::Arc;

pub fn render_plugins_body(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.clone() else {
        return;
    };
    let is_strict_local = snap.settings.privacy_mode == PrivacyMode::StrictLocal;

    // 0. Strict Local Network Guard Notice (1-click switch to Hybrid Mode)
    if is_strict_local {
        p.card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                badge(ui, "Strict Local", p.green, p.bg_card);
                ui.label(
                    egui::RichText::new(
                        "Outbound PyPI, npm, crates.io & runtime downloads are blocked until Hybrid Mode is enabled (offline SKILL.md packs still install locally).",
                    )
                    .small()
                    .color(p.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Switch to Hybrid Mode").clicked() {
                        backend.set_privacy(PrivacyMode::Hybrid);
                    }
                });
            });
        });
        ui.add_space(12.0);
    }

    // 1. Live Staged Install Jobs
    if !snap.install_jobs.is_empty() {
        let has_finished = snap.install_jobs.iter().any(|j| j.status != "running");
        setting_group(
            ui,
            p,
            "Staged Temp-Folder Install Activity",
            "Every package is downloaded, staged, and verified in isolation before applying.",
            |ui| {
                if has_finished {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if secondary_button(ui, p, "Clear Finished").clicked() {
                                backend.clear_install_jobs();
                            }
                        });
                    });
                    ui.add_space(4.0);
                }
                for (idx, j) in snap.install_jobs.iter().take(4).enumerate() {
                    if idx > 0 {
                        ui.add_space(6.0);
                    }
                    let col = match j.status.as_str() {
                        "completed" => p.green,
                        "failed" => p.red,
                        _ => p.amber,
                    };
                    p.subcard_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if j.status == "running" {
                                ui.spinner();
                            }
                            ui.label(egui::RichText::new(&j.target).strong());
                            badge(ui, &j.kind, p.muted, p.bg_elev);
                            badge(
                                ui,
                                &format!("{} · {}%", j.status, j.progress_pct),
                                col,
                                p.bg_elev,
                            );
                        });
                        ui.add_space(4.0);
                        ui.add(egui::ProgressBar::new(
                            (j.progress_pct as f32 / 100.0).clamp(0.02, 1.0),
                        ));
                        ui.label(
                            egui::RichText::new(&j.current_step)
                                .small()
                                .color(if j.status == "failed" { p.red } else { p.muted }),
                        );
                    });
                }
            },
        );
        ui.add_space(16.0);
    }

    // 2. Antigravity mcp_config.json Export Drawer
    if let Some(ref json_txt) = state.agy_export_json.clone() {
        setting_group(
            ui,
            p,
            "Antigravity mcp_config.json Export",
            "Ready to paste into ~/.gemini/antigravity/mcp_config.json.",
            |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if secondary_button(ui, p, "Close").clicked() {
                            state.agy_export_json = None;
                        }
                        if primary_button(ui, p, "Copy JSON").clicked() {
                            copy_to_clipboard(ctx, json_txt);
                            state.set_toast("Copied Antigravity mcp_config.json.", false);
                        }
                    });
                });
                ui.add_space(6.0);
                let mut txt = json_txt.clone();
                input_frame(p, false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut txt)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(6)
                            .desired_width(f32::INFINITY)
                            .frame(false),
                    );
                });
            },
        );
        ui.add_space(16.0);
    }

    // 3. Toolchains Strip (clean 2-column grid)
    setting_group(
        ui,
        p,
        "Local Toolchains & Managed Runtimes",
        "Detected system SDKs and isolated portable runtimes managed by Cinder.",
        |ui| {
            for (row_idx, pair) in snap.toolchains.chunks(2).enumerate() {
                if row_idx > 0 {
                    ui.add_space(8.0);
                }
                ui.columns(2, |cols| {
                    for (cidx, tc) in pair.iter().enumerate() {
                        cols[cidx].scope(|ui| {
                            let running_tc = tc.installable_plugin_id.as_deref().and_then(|pid| {
                                snap.install_jobs.iter().find(|j| {
                                    j.kind == "plugin" && j.status == "running" && j.target == pid
                                })
                            });
                            p.subcard_frame().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&tc.name).strong());
                                    badge(
                                        ui,
                                        if tc.installed { &tc.source } else { "Missing" },
                                        if tc.installed { p.green } else { p.amber },
                                        p.bg_elev,
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if let Some(job) = running_tc {
                                                badge(
                                                    ui,
                                                    &format!("Installing {}%", job.progress_pct),
                                                    p.amber,
                                                    p.bg_elev,
                                                );
                                                ui.spinner();
                                            } else if !tc.installed {
                                                if let Some(ref pid) = tc.installable_plugin_id {
                                                    let btn_lbl = if is_strict_local {
                                                        "Hybrid + Install Node.js"
                                                    } else {
                                                        "Install Portable Node.js"
                                                    };
                                                    if primary_button(ui, p, btn_lbl).clicked() {
                                                        backend.install_plugin_with_auto_hybrid(
                                                            pid.clone(),
                                                            is_strict_local,
                                                        );
                                                    }
                                                }
                                            }
                                        },
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(if let Some(job) = running_tc {
                                        &job.current_step
                                    } else {
                                        &tc.version
                                    })
                                    .small()
                                    .monospace()
                                    .color(p.muted),
                                );
                                if let Some(job) = running_tc {
                                    ui.add_space(4.0);
                                    ui.add(egui::ProgressBar::new(
                                        (job.progress_pct as f32 / 100.0).clamp(0.03, 1.0),
                                    ));
                                }
                            });
                        });
                    }
                });
            }
        },
    );

    ui.add_space(16.0);

    // 4. Curated Allow-List Plugins & Antigravity Skills
    setting_group(
        ui,
        p,
        "Curated Allow-List Plugins & Antigravity Skills",
        "Version-pinned MCP servers, SKILL.md packs, and runtimes.",
        |ui| {
            let cur_cat = match state.plugin_cat_filter.as_str() {
                "mcp" => "mcp",
                "skill_pack" => "skill_pack",
                "toolchain" => "toolchain",
                _ => "all",
            };
            if let Some(next_cat) = segmented_control(
                ui,
                p,
                "plugins_cat_seg",
                cur_cat,
                &[
                    ("all", "All"),
                    ("mcp", "MCP Servers"),
                    ("skill_pack", "Skills"),
                    ("toolchain", "Runtimes"),
                ],
            ) {
                state.plugin_cat_filter = next_cat.into();
            }
            ui.add_space(8.0);

            for pl in &snap.plugins_catalog {
                let e = &pl.entry;
                let show = match state.plugin_cat_filter.as_str() {
                    "all" => true,
                    "mcp" => e.kind == "mcp_python" || e.kind == "mcp_node",
                    other => e.kind == other,
                };
                if !show {
                    continue;
                }

                let running_job = snap.install_jobs.iter().find(|j| {
                    j.kind == "plugin" && j.status == "running" && j.target == e.id
                });
                let last_failed = if running_job.is_none() && !pl.installed {
                    snap.install_jobs.iter().find(|j| {
                        j.kind == "plugin" && j.status == "failed" && j.target == e.id
                    })
                } else {
                    None
                };

                p.subcard_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&e.name).strong().color(p.text));
                        badge(ui, &e.category, p.muted, p.bg_elev);
                        badge(
                            ui,
                            if e.needs_network { "Hybrid" } else { "Offline" },
                            if e.needs_network { p.amber } else { p.green },
                            p.bg_elev,
                        );
                        if e.antigravity_compatible {
                            badge(ui, "Antigravity", p.purple, p.bg_elev);
                        }
                        if pl.installed {
                            badge(ui, "Installed", p.green, p.bg_elev);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(job) = running_job {
                                badge(
                                    ui,
                                    &format!("Installing {}%", job.progress_pct),
                                    p.amber,
                                    p.bg_elev,
                                );
                                ui.spinner();
                            } else if pl.installed {
                                if danger_button(ui, p, "Uninstall").clicked() {
                                    backend.uninstall_plugin(e.id.clone());
                                }
                                if let Some(ref mcp_id) = pl.mcp_server_id {
                                    let mut en = pl.enabled;
                                    if toggle_switch(ui, p, &mut en).changed() {
                                        backend.toggle_mcp(mcp_id.clone(), en);
                                    }
                                    if secondary_button(ui, p, "Test Tools").clicked() {
                                        backend.probe_mcp(mcp_id.clone());
                                    }
                                }
                            } else {
                                let needs_net_for_install =
                                    is_strict_local && e.kind != "skill_pack";
                                let lbl = if needs_net_for_install {
                                    "Hybrid + Install"
                                } else {
                                    "Install (1-Click)"
                                };
                                if primary_button(ui, p, lbl).clicked() {
                                    backend.install_plugin_with_auto_hybrid(
                                        e.id.clone(),
                                        needs_net_for_install,
                                    );
                                }
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&e.description).small().color(p.muted));

                    if let Some(job) = running_job {
                        ui.add_space(4.0);
                        ui.add(egui::ProgressBar::new(
                            (job.progress_pct as f32 / 100.0).clamp(0.03, 1.0),
                        ));
                        ui.label(egui::RichText::new(&job.current_step).small().color(p.amber));
                    } else if let Some(fail_job) = last_failed {
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new(
                                fail_job.error.as_deref().unwrap_or(&fail_job.current_step),
                            )
                            .small()
                            .color(p.red),
                        );
                    }

                    if let Some(ref mcp_id) = pl.mcp_server_id {
                        if let Some(probe) = state.mcp_probes.get(mcp_id) {
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
                                        egui::RichText::new(format!(
                                            "Probe error: {}",
                                            probe.error.as_deref().unwrap_or("unknown")
                                        ))
                                        .small()
                                        .color(p.red),
                                    );
                                }
                            });
                        }
                    }
                });
                ui.add_space(6.0);
            }
        },
    );

    ui.add_space(16.0);

    // 5. Curated Project Dependencies
    let ws_name = Path::new(&snap.settings.active_workspace_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");

    setting_group(
        ui,
        p,
        &format!("Project Dependencies ({ws_name})"),
        "Curated Cargo, pip, and npm packages staged safely in an isolated temp folder.",
        |ui| {
            let cur_mgr = match state.dep_mgr_filter.as_deref() {
                Some("python") => "python",
                Some("npm") => "npm",
                _ => "cargo",
            };
            ui.horizontal(|ui| {
                if let Some(next_mgr) = segmented_control(
                    ui,
                    p,
                    "dep_mgr_seg",
                    cur_mgr,
                    &[
                        ("cargo", "Cargo (Rust)"),
                        ("python", "Python (pip)"),
                        ("npm", "Node (npm)"),
                    ],
                ) {
                    state.dep_mgr_filter = Some(next_mgr.into());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.dep_search)
                                .hint_text("Filter packages...")
                                .desired_width(180.0)
                                .frame(false),
                        );
                    });
                });
            });

            ui.add_space(6.0);
            let detected = &snap.project_deps.detected_ecosystems;
            let det_str = if detected.is_empty() {
                format!("No Cargo.toml, requirements.txt, or package.json detected yet in {ws_name} (installing creates it cleanly).")
            } else {
                format!("Detected ecosystems: {}", detected.join(", "))
            };
            ui.label(egui::RichText::new(det_str).small().color(p.muted));
            ui.add_space(8.0);

            // Quick Install Any Custom Package (validated + temp-staged)
            ui.horizontal(|ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let mut enter_submit = false;
                let placeholder = match cur_mgr {
                    "python" => "Install any PyPI package (e.g. polars, fastapi)...",
                    "npm" => "Install any npm package (e.g. @tanstack/react-query)...",
                    _ => "Install any Cargo crate (e.g. itertools, clap)...",
                };
                input_frame(p, false).show(ui, |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut state.custom_dep_input)
                            .hint_text(placeholder)
                            .desired_width((ui.available_width() - 180.0).max(160.0))
                            .frame(false),
                    );
                    if r.lost_focus() && enter {
                        enter_submit = true;
                    }
                });
                let custom_lbl = if is_strict_local {
                    "Hybrid + Stage & Install"
                } else {
                    "+ Stage & Install"
                };
                if (enter_submit || primary_button(ui, p, custom_lbl).clicked())
                    && !state.custom_dep_input.trim().is_empty()
                {
                    let pkg = state.custom_dep_input.trim().to_string();
                    state.custom_dep_input.clear();
                    backend.install_custom_dependency_with_auto_hybrid(
                        cur_mgr.into(),
                        pkg,
                        is_strict_local,
                    );
                }
            });
            ui.add_space(6.0);

            // Installed custom (non-allowlisted) packages in this project
            let installed_list = snap
                .project_deps
                .installed_by_manager
                .get(cur_mgr)
                .cloned()
                .unwrap_or_default();
            let custom_installed: Vec<String> = installed_list
                .iter()
                .filter(|name| {
                    !snap
                        .project_deps
                        .allowlist
                        .iter()
                        .any(|a| a.manager == cur_mgr && a.name.eq_ignore_ascii_case(name))
                })
                .cloned()
                .collect();
            if !custom_installed.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("In Project (custom):")
                            .small()
                            .color(p.muted),
                    );
                    for pkg_name in &custom_installed {
                        badge(ui, pkg_name, p.green, p.bg_elev);
                        if danger_button(ui, p, "×")
                            .on_hover_text(format!("Remove {pkg_name}"))
                            .clicked()
                        {
                            backend.uninstall_dependency(cur_mgr.into(), pkg_name.clone());
                        }
                    }
                });
                ui.add_space(6.0);
            }

            let already: std::collections::HashSet<String> = installed_list
                .into_iter()
                .map(|s| s.to_ascii_lowercase())
                .collect();
            let needle = state.dep_search.trim().to_ascii_lowercase();

            for d in &snap.project_deps.allowlist {
                if d.manager != cur_mgr {
                    continue;
                }
                if !needle.is_empty()
                    && !format!("{} {} {}", d.name, d.category, d.description)
                        .to_ascii_lowercase()
                        .contains(&needle)
                {
                    continue;
                }
                let in_proj = already.contains(&d.name.to_ascii_lowercase());
                let target_key = format!("{}:{}", d.manager, d.name);
                let running_job = snap.install_jobs.iter().find(|j| {
                    j.kind == "dependency" && j.status == "running" && j.target == target_key
                });
                let last_failed = if running_job.is_none() {
                    snap.install_jobs.iter().find(|j| {
                        j.kind == "dependency" && j.status == "failed" && j.target == target_key
                    })
                } else {
                    None
                };

                p.subcard_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&d.name).monospace().strong());
                        if in_proj {
                            badge(ui, "In Project", p.green, p.bg_elev);
                        } else {
                            badge(ui, &d.category, p.muted, p.bg_elev);
                        }
                        if let Some(ref feat) = d.features_hint {
                            badge(ui, feat, p.muted, p.bg_elev);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(job) = running_job {
                                badge(
                                    ui,
                                    &format!("Installing {}%", job.progress_pct),
                                    p.amber,
                                    p.bg_elev,
                                );
                                ui.spinner();
                            } else if in_proj {
                                if danger_button(ui, p, "Remove").clicked() {
                                    backend.uninstall_dependency(d.manager.clone(), d.name.clone());
                                }
                                let upd_lbl = if is_strict_local {
                                    "Hybrid + Update"
                                } else {
                                    "Update"
                                };
                                if secondary_button(ui, p, upd_lbl).clicked() {
                                    backend.install_dependency_with_auto_hybrid(
                                        d.manager.clone(),
                                        d.name.clone(),
                                        is_strict_local,
                                    );
                                }
                            } else {
                                let inst_lbl = if is_strict_local {
                                    "Hybrid + Install"
                                } else {
                                    "+ Install"
                                };
                                if primary_button(ui, p, inst_lbl).clicked() {
                                    backend.install_dependency_with_auto_hybrid(
                                        d.manager.clone(),
                                        d.name.clone(),
                                        is_strict_local,
                                    );
                                }
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&d.description).small().color(p.muted));

                    if let Some(job) = running_job {
                        ui.add_space(4.0);
                        ui.add(egui::ProgressBar::new(
                            (job.progress_pct as f32 / 100.0).clamp(0.03, 1.0),
                        ));
                        ui.label(egui::RichText::new(&job.current_step).small().color(p.amber));
                    } else if let Some(fail_job) = last_failed {
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new(
                                fail_job.error.as_deref().unwrap_or(&fail_job.current_step),
                            )
                            .small()
                            .color(p.red),
                        );
                    }
                });
                ui.add_space(6.0);
            }
        },
    );
}
