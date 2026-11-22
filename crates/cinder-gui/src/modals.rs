use crate::app_state::{CustomTab, ForgeTab, GuiState, MainView, PromptModal, SettingsTab, StageTab};
use crate::backend::{BackendHandle, FolderPickTarget};
use crate::chat::format_session_markdown;
use crate::icons::{draw_icon, Icon};
use crate::theme::{
    copy_to_clipboard, danger_button, ghost_button, input_frame, primary_button, secondary_button,
    CinderPalette,
};
use egui::{CornerRadius, Frame, Margin, Stroke};
use std::sync::Arc;
use std::time::Duration;

pub fn render_modals_and_toasts(
    ctx: &egui::Context,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    render_command_palette(ctx, state, backend, p);
    render_prompt_modal(ctx, state, backend, p);
    render_toast_overlay(ctx, state, p);
}

fn palette_row(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    category: &str,
    label: &str,
    selected: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), p.bg_card);
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 5.0),
                egui::vec2(2.5, rect.height() - 10.0),
            );
            ui.painter()
                .rect_filled(bar, CornerRadius::same(2), p.accent);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), p.bg_hover);
        }
        let cat_galley = ui.painter().layout_no_wrap(
            category.to_string(),
            egui::FontId::proportional(11.0),
            p.muted,
        );
        let max_lbl_w = (rect.width() - cat_galley.size().x - 28.0).max(40.0);
        let lbl_galley = egui::WidgetText::from(label).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            max_lbl_w,
            egui::TextStyle::Body,
        );
        ui.painter().galley(
            egui::pos2(
                rect.left() + 10.0,
                rect.center().y - lbl_galley.size().y * 0.5,
            ),
            lbl_galley,
            p.text,
        );
        ui.painter().galley(
            egui::pos2(
                rect.right() - cat_galley.size().x - 10.0,
                rect.center().y - cat_galley.size().y * 0.5,
            ),
            cat_galley,
            p.muted,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

#[derive(Clone)]
enum PaletteAction {
    Static(&'static str),
    Workspace(String),
    Model(String),
    File(String),
}

fn render_command_palette(
    ctx: &egui::Context,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    if !state.show_cmd_palette {
        return;
    }
    let Some(snap) = state.snap.clone() else {
        return;
    };

    let mut open = state.show_cmd_palette;
    egui::Window::new("Command Palette (Ctrl+K)")
        .open(&mut open)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .default_size([580.0, 440.0])
        .anchor(egui::Align2::CENTER_TOP, [0.0, 72.0])
        .show(ctx, |ui| {
            let (down_pressed, up_pressed, enter_pressed) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::Enter),
                )
            });

            input_frame(p, false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (ir, _) =
                        ui.allocate_exact_size(egui::vec2(15.0, 15.0), egui::Sense::hover());
                    draw_icon(ui.painter(), ir, Icon::Search, p.muted);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut state.cmd_query)
                            .hint_text("Jump to file, project, model, view, or command... (↑↓ Enter)")
                            .desired_width((ui.available_width() - 56.0).max(120.0))
                            .frame(false),
                    );
                    if r.changed() {
                        state.cmd_selected_idx = 0;
                    }
                    r.request_focus();
                    if ghost_button(ui, p, egui::RichText::new("Esc").small()).clicked() {
                        state.show_cmd_palette = false;
                    }
                });
            });
            ui.add_space(6.0);
            ui.separator();

            let q = state.cmd_query.trim().to_ascii_lowercase();
            let mut entries: Vec<(String, String, PaletteAction)> = Vec::new();

            let static_cmds: Vec<(&str, &'static str)> = vec![
                ("View", "Automations & Playbooks (v5.0)"),
                ("Stage", "Code Editor & Terminal"),
                ("Stage", "Session Artifacts"),
                ("Stage", "Workspace Diffs"),
                ("Forge", "Local Model Vault"),
                ("Forge", "Hugging Face GGUF Hub"),
                ("Forge", "Cloud Quotas & API Reachability"),
                ("Customize", "Plugins & Dependencies"),
                ("Customize", "Skills & Codex RAG"),
                ("Customize", "MCP Servers"),
                ("Customize", "Workspace Rules & Projects"),
                ("Action", "Open Project Folder..."),
                ("Action", "New Conversation"),
                ("Action", "New One-Off Conversation"),
                ("Action", "Quench Local Models (Free VRAM)"),
                ("Action", "Sync Antigravity MCP & Skills (~/.gemini)"),
                ("Action", "Compact Chat Context (/compact)"),
                ("Action", "Export Session Markdown (.md)"),
                ("Settings", "Appearance & Theme (Ctrl+,)"),
                ("Settings", "Models & API Keys"),
                ("Settings", "Local Forge & GPU Settings"),
                ("Settings", "Agent & Autonomy Policy"),
                ("Settings", "Privacy & Network Audit Log"),
            ];
            for (kind, lbl) in static_cmds {
                if q.is_empty()
                    || lbl.to_ascii_lowercase().contains(&q)
                    || kind.to_ascii_lowercase().contains(&q)
                {
                    entries.push((kind.into(), lbl.into(), PaletteAction::Static(lbl)));
                }
            }
            for ws in &snap.workspaces {
                if q.is_empty() || ws.name.to_ascii_lowercase().contains(&q) {
                    entries.push((
                        "Project".into(),
                        ws.name.clone(),
                        PaletteAction::Workspace(ws.path.clone()),
                    ));
                }
            }
            for m in state.selectable_models(true) {
                if q.is_empty() || m.display_name.to_ascii_lowercase().contains(&q) {
                    entries.push((
                        format!("Model · {}", m.provider_label),
                        m.display_name.clone(),
                        PaletteAction::Model(m.id.clone()),
                    ));
                }
            }
            for f in snap.workspace_tree.iter().filter(|n| !n.is_dir).take(120) {
                if q.is_empty() || f.rel_path.to_ascii_lowercase().contains(&q) {
                    entries.push((
                        "File".into(),
                        f.rel_path.clone(),
                        PaletteAction::File(f.rel_path.clone()),
                    ));
                }
            }

            if !entries.is_empty() {
                if down_pressed {
                    state.cmd_selected_idx = (state.cmd_selected_idx + 1) % entries.len();
                } else if up_pressed {
                    state.cmd_selected_idx = if state.cmd_selected_idx == 0 {
                        entries.len() - 1
                    } else {
                        state.cmd_selected_idx - 1
                    };
                } else if state.cmd_selected_idx >= entries.len() {
                    state.cmd_selected_idx = 0;
                }
            }

            let mut triggered_action: Option<PaletteAction> = None;

            egui::ScrollArea::vertical()
                .id_salt("cmd_palette_scroll")
                .max_height(350.0)
                .show(ui, |ui| {
                    for (idx, (kind, lbl, action)) in entries.iter().enumerate() {
                        let is_sel = idx == state.cmd_selected_idx;
                        let resp = palette_row(ui, p, kind, lbl, is_sel);
                        if resp.hovered() {
                            state.cmd_selected_idx = idx;
                        }
                        if is_sel && (down_pressed || up_pressed) {
                            resp.scroll_to_me(Some(egui::Align::Center));
                        }
                        if resp.clicked() || (is_sel && enter_pressed) {
                            triggered_action = Some(action.clone());
                        }
                    }
                });

            if let Some(action) = triggered_action {
                match action {
                    PaletteAction::Static(lbl) => match lbl {
                        "Automations & Playbooks (v5.0)" => {
                            state.switch_main_view(MainView::Automations, true)
                        }
                        "Code Editor & Terminal" => state.open_stage_tab(StageTab::Editor),
                        "Session Artifacts" => state.open_stage_tab(StageTab::Artifacts),
                        "Workspace Diffs" => state.open_stage_tab(StageTab::Diffs),
                        "Local Model Vault" => {
                            state.switch_main_view(MainView::Forge(ForgeTab::Vault), true)
                        }
                        "Hugging Face GGUF Hub" => {
                            state.switch_main_view(MainView::Forge(ForgeTab::Hub), true)
                        }
                        "Cloud Quotas & API Reachability" => {
                            state.switch_main_view(MainView::Forge(ForgeTab::Quotas), true)
                        }
                        "Plugins & Dependencies" => {
                            state.switch_main_view(MainView::Customizations(CustomTab::Plugins), true)
                        }
                        "Skills & Codex RAG" => {
                            state.switch_main_view(MainView::Customizations(CustomTab::Skills), true)
                        }
                        "MCP Servers" => {
                            state.switch_main_view(MainView::Customizations(CustomTab::Mcp), true)
                        }
                        "Workspace Rules & Projects" => {
                            state.switch_main_view(MainView::Customizations(CustomTab::Rules), true)
                        }
                        "Open Project Folder..." => backend.open_folder_picker(
                            FolderPickTarget::Workspace,
                            "Select Project Folder for Cinder",
                        ),
                        "New Conversation" => {
                            backend.create_session(None, None);
                            state.switch_main_view(MainView::Chat, true);
                        }
                        "New One-Off Conversation" => {
                            backend.create_standalone_session(None);
                            state.switch_main_view(MainView::Chat, true);
                        }
                        "Quench Local Models (Free VRAM)" => backend.stop_all_local_models(),
                        "Sync Antigravity MCP & Skills (~/.gemini)" => backend.sync_antigravity(),
                        "Compact Chat Context (/compact)" => {
                            if let Some(ref s) = snap.active_session {
                                backend.compact_chat(s.id.clone());
                            }
                        }
                        "Export Session Markdown (.md)" => {
                            if let Some(ref s) = snap.active_session {
                                copy_to_clipboard(ctx, &format_session_markdown(s));
                                state.set_toast("Copied Markdown transcript.", false);
                            }
                        }
                        "Appearance & Theme (Ctrl+,)" => {
                            state.open_settings(SettingsTab::Appearance)
                        }
                        "Models & API Keys" => state.open_settings(SettingsTab::Models),
                        "Local Forge & GPU Settings" => state.open_settings(SettingsTab::Engine),
                        "Agent & Autonomy Policy" => state.open_settings(SettingsTab::Agent),
                        "Privacy & Network Audit Log" => state.open_settings(SettingsTab::Privacy),
                        _ => {}
                    },
                    PaletteAction::Workspace(path) => {
                        backend.select_workspace(path);
                        state.switch_main_view(MainView::Chat, true);
                    }
                    PaletteAction::Model(id) => {
                        backend.update_settings(move |s| {
                            s.active_model_id = id;
                        });
                    }
                    PaletteAction::File(rel) => {
                        backend.load_workspace_file(rel);
                    }
                }
                state.show_cmd_palette = false;
                state.cmd_query.clear();
                state.cmd_selected_idx = 0;
            }
        });

    if !open {
        state.show_cmd_palette = false;
    }
}

fn render_prompt_modal(
    ctx: &egui::Context,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let cur = state.prompt_modal.clone();
    match cur {
        PromptModal::None => {}
        PromptModal::NewFile {
            parent_rel,
            mut name,
        } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("New Workspace File")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(if parent_rel.is_empty() {
                        "Parent: workspace root".to_string()
                    } else {
                        format!("Parent folder: {parent_rel}")
                    });
                    ui.add_space(4.0);
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    input_frame(p, false).show(ui, |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut name)
                                .hint_text("e.g. main.rs or helper.py")
                                .desired_width(300.0)
                                .frame(false),
                        );
                        r.request_focus();
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, p, "Create File").clicked() || enter {
                            let joined = if parent_rel.is_empty() {
                                name.trim().to_string()
                            } else {
                                format!("{}/{}", parent_rel.trim_end_matches('/'), name.trim())
                            };
                            if !joined.is_empty() {
                                backend.create_file(joined);
                            }
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            } else {
                state.prompt_modal = PromptModal::NewFile { parent_rel, name };
            }
        }
        PromptModal::NewFolder {
            parent_rel,
            mut name,
        } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("New Workspace Folder")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(if parent_rel.is_empty() {
                        "Parent: workspace root".to_string()
                    } else {
                        format!("Parent folder: {parent_rel}")
                    });
                    ui.add_space(4.0);
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    input_frame(p, false).show(ui, |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut name)
                                .hint_text("folder_name")
                                .desired_width(300.0)
                                .frame(false),
                        );
                        r.request_focus();
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, p, "Create Folder").clicked() || enter {
                            let joined = if parent_rel.is_empty() {
                                name.trim().to_string()
                            } else {
                                format!("{}/{}", parent_rel.trim_end_matches('/'), name.trim())
                            };
                            if !joined.is_empty() {
                                backend.create_dir(joined);
                            }
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            } else {
                state.prompt_modal = PromptModal::NewFolder { parent_rel, name };
            }
        }
        PromptModal::RenameEntry {
            old_rel,
            mut new_name,
        } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("Rename Workspace Item")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Current: {old_rel}"));
                    ui.add_space(4.0);
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    input_frame(p, false).show(ui, |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut new_name)
                                .desired_width(300.0)
                                .frame(false),
                        );
                        r.request_focus();
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, p, "Rename").clicked() || enter {
                            backend.rename_entry(old_rel.clone(), new_name.clone());
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            } else {
                state.prompt_modal = PromptModal::RenameEntry { old_rel, new_name };
            }
        }
        PromptModal::DeleteEntry { rel_path, is_dir } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("Confirm Delete")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let what = if is_dir {
                        "folder and all its contents"
                    } else {
                        "file"
                    };
                    ui.label(
                        egui::RichText::new(format!("Permanently delete {what} '{rel_path}'?"))
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if danger_button(ui, p, "Delete Permanently").clicked() {
                            backend.delete_entry(rel_path.clone());
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            }
        }
        PromptModal::RenameSession {
            session_id,
            mut title,
        } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("Rename Conversation")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    input_frame(p, false).show(ui, |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut title)
                                .desired_width(300.0)
                                .frame(false),
                        );
                        r.request_focus();
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, p, "Save Title").clicked() || enter {
                            backend.rename_session(session_id.clone(), title.clone());
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            } else {
                state.prompt_modal = PromptModal::RenameSession { session_id, title };
            }
        }
        PromptModal::NewArtifact {
            mut title,
            mut kind,
            mut content,
        } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("New Session Artifact")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        input_frame(p, false).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut title)
                                    .hint_text("Title")
                                    .desired_width(220.0)
                                    .frame(false),
                            );
                        });
                        egui::ComboBox::from_id_salt("new_art_kind")
                            .selected_text(&kind)
                            .show_ui(ui, |ui| {
                                for k in ["report", "task_list", "plan", "walkthrough"] {
                                    ui.selectable_value(&mut kind, k.into(), k);
                                }
                            });
                    });
                    ui.add_space(6.0);
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut content)
                                .desired_rows(8)
                                .desired_width(420.0)
                                .hint_text("Markdown content...")
                                .frame(false),
                        );
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, p, "Save Artifact").clicked() {
                            if let Some(sid) = state
                                .snap
                                .as_ref()
                                .and_then(|s| s.active_session.as_ref())
                                .map(|s| s.id.clone())
                            {
                                backend.upsert_artifact(
                                    sid,
                                    if title.trim().is_empty() {
                                        "Notes".into()
                                    } else {
                                        title.clone()
                                    },
                                    kind.clone(),
                                    content.clone(),
                                );
                            }
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            } else {
                state.prompt_modal = PromptModal::NewArtifact {
                    title,
                    kind,
                    content,
                };
            }
        }
        PromptModal::EditPlan { mut markdown } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("Edit Implementation Plan")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut markdown)
                                .desired_rows(12)
                                .desired_width(520.0)
                                .frame(false),
                        );
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, p, "Approve & Execute Edited Plan").clicked() {
                            if let Some(sid) = state
                                .snap
                                .as_ref()
                                .and_then(|s| s.active_session.as_ref())
                                .map(|s| s.id.clone())
                            {
                                state.is_generating = true;
                                backend.plan_action(
                                    sid,
                                    "execute".into(),
                                    Some(markdown.clone()),
                                    None,
                                );
                            }
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            } else {
                state.prompt_modal = PromptModal::EditPlan { markdown };
            }
        }
        PromptModal::RequestPlanChanges { mut feedback } => {
            let mut open = true;
            let mut done = false;
            egui::Window::new("Request Plan Changes")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    input_frame(p, false).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut feedback)
                                .desired_rows(5)
                                .desired_width(420.0)
                                .hint_text("Describe changes to the plan...")
                                .frame(false),
                        );
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, p, "Send Feedback").clicked() {
                            if let Some(sid) = state
                                .snap
                                .as_ref()
                                .and_then(|s| s.active_session.as_ref())
                                .map(|s| s.id.clone())
                            {
                                state.is_generating = true;
                                backend.plan_action(
                                    sid,
                                    "request_changes".into(),
                                    None,
                                    Some(feedback.clone()),
                                );
                            }
                            done = true;
                        }
                        if secondary_button(ui, p, "Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            if done || !open {
                state.prompt_modal = PromptModal::None;
            } else {
                state.prompt_modal = PromptModal::RequestPlanChanges { feedback };
            }
        }
        PromptModal::LightboxImage { title, cache_key } => {
            let mut open = true;
            egui::Window::new(title)
                .open(&mut open)
                .collapsible(false)
                .default_size([760.0, 540.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if let Some(tex) = state.textures.get(&cache_key) {
                        egui::ScrollArea::both().show(ui, |ui| {
                            ui.add(
                                egui::Image::new(tex)
                                    .max_size(egui::vec2(720.0, 480.0))
                                    .corner_radius(CornerRadius::same(8)),
                            );
                        });
                    }
                });
            if !open {
                state.prompt_modal = PromptModal::None;
            }
        }
    }
}

fn render_toast_overlay(ctx: &egui::Context, state: &mut GuiState, p: &CinderPalette) {
    let Some((ref msg, is_err, created)) = state.toast else {
        return;
    };
    if created.elapsed() > Duration::from_secs(5) {
        state.toast = None;
        return;
    }

    let col = if is_err { p.red } else { p.accent_hot };
    egui::Area::new(egui::Id::new("cinder_toast_area"))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -18.0])
        .show(ctx, |ui| {
            Frame::new()
                .fill(p.bg_card)
                .stroke(Stroke::new(1.5_f32, col))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(14, 9))
                .show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(egui::RichText::new(msg).color(p.text));
                });
        });
}
