use crate::app_state::{fmt_bytes, DiffsViewMode, FsTreeNode, GuiState, PromptModal, StageTab};
use crate::backend::{BackendHandle, FolderPickTarget};
use crate::icons::{draw_icon, icon_button, Icon};
use crate::theme::{badge, copy_to_clipboard, ghost_button, input_frame, pane_header, CinderPalette};
use egui::{Color32, CornerRadius, Frame, Margin};
use harness_tools::{GitFileStatus, GitWorkspaceStatus};
use std::path::PathBuf;
use std::sync::Arc;

pub fn render_explorer_panel(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let Some(snap) = state.snap.as_ref() else {
        ui.label("Loading workspace...");
        return;
    };
    let ws_root = PathBuf::from(&snap.settings.active_workspace_path);
    let ws_name = ws_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Workspace")
        .to_string();

    // Exact 40px aligned pane header (single row)
    pane_header(ui, p, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, p, Icon::Close, false, "Hide Explorer (Ctrl+Shift+E)").clicked() {
                state.show_explorer = false;
            }
            if icon_button(ui, p, Icon::Refresh, false, "Refresh Workspace Files").clicked() {
                backend.refresh_state(false);
            }
            if icon_button(ui, p, Icon::NewFolder, false, "New Folder in Root").clicked() {
                state.prompt_modal = PromptModal::NewFolder {
                    parent_rel: String::new(),
                    name: String::new(),
                };
            }
            if icon_button(ui, p, Icon::NewFile, false, "New File in Root").clicked() {
                state.prompt_modal = PromptModal::NewFile {
                    parent_rel: String::new(),
                    name: String::new(),
                };
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(&ws_name).strong().color(p.text))
                        .truncate(),
                )
                .on_hover_text(&ws_name);
            });
        });
    });

    let tree_nodes = snap.workspace_tree.clone();
    let git_status = snap.git_status.clone();
    let filter_q = state.explorer_filter.trim().to_ascii_lowercase();

    Frame::new()
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            input_frame(p, false).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.explorer_filter)
                        .hint_text("Filter workspace files...")
                        .desired_width(f32::INFINITY)
                        .frame(false),
                );
            });
            if git_status.is_repo {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let mut branch_lbl = git_status.branch.clone();
                    if git_status.ahead > 0 {
                        branch_lbl.push_str(&format!(" ↑{}", git_status.ahead));
                    }
                    if git_status.behind > 0 {
                        branch_lbl.push_str(&format!(" ↓{}", git_status.behind));
                    }
                    badge(ui, &branch_lbl, p.accent_hot, p.bg_card);
                    let dirty_cnt = git_status.file_statuses.len();
                    if dirty_cnt > 0 {
                        badge(ui, &format!("{dirty_cnt} changed"), p.amber, p.bg_card);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ghost_button(ui, p, egui::RichText::new("Diff").small()).clicked() {
                                state.diffs_view_mode = DiffsViewMode::GitHead;
                                state.git_diff_filter = None;
                                state.git_diff_loading = true;
                                state.open_stage_tab(StageTab::Diffs);
                                backend.load_git_diff(None);
                            }
                        });
                    } else {
                        badge(ui, "clean", p.green, p.bg_card);
                    }
                });
            }
            ui.add_space(6.0);

            egui::ScrollArea::vertical()
                .id_salt("workspace_explorer_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if !filter_q.is_empty() {
                        let matching: Vec<_> = tree_nodes
                            .iter()
                            .filter(|n| {
                                !n.is_dir && n.rel_path.to_ascii_lowercase().contains(&filter_q)
                            })
                            .take(150)
                            .collect();
                        if matching.is_empty() {
                            ui.label(egui::RichText::new("No matching files.").color(p.muted));
                        }
                        for n in matching {
                            render_leaf_row(
                                ui,
                                state,
                                backend,
                                p,
                                &ws_root,
                                &git_status,
                                &n.name,
                                &n.rel_path,
                                n.size_bytes,
                                &n.extension,
                                0,
                            );
                        }
                        return;
                    }

                    let roots = FsTreeNode::build_tree(&tree_nodes);
                    if roots.is_empty() {
                        ui.label(egui::RichText::new("Workspace folder is empty.").color(p.muted));
                    }
                    for node in &roots {
                        render_tree_node(ui, state, backend, p, &ws_root, &git_status, node, 0);
                    }

                    // Fill remaining space inside scroll area for background right-click actions
                    let rem_h = ui.available_height().max(48.0);
                    let (_, bg_resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), rem_h),
                        egui::Sense::click(),
                    );
                    bg_resp.context_menu(|ui| {
                        if ui.button("New File...").clicked() {
                            state.prompt_modal = PromptModal::NewFile {
                                parent_rel: String::new(),
                                name: String::new(),
                            };
                            ui.close_menu();
                        }
                        if ui.button("New Folder...").clicked() {
                            state.prompt_modal = PromptModal::NewFolder {
                                parent_rel: String::new(),
                                name: String::new(),
                            };
                            ui.close_menu();
                        }
                        if ui.button("Open Another Project Folder...").clicked() {
                            backend.open_folder_picker(
                                FolderPickTarget::Workspace,
                                "Select Project Folder for Cinder",
                            );
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Refresh Explorer").clicked() {
                            backend.refresh_state(false);
                            ui.close_menu();
                        }
                        if ui.button("Open Workspace in Explorer").clicked() {
                            backend.reveal_entry(ws_root.clone());
                            ui.close_menu();
                        }
                    });
                });
        });
}

#[allow(clippy::too_many_arguments)]
fn render_tree_node(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
    ws_root: &PathBuf,
    git_status: &GitWorkspaceStatus,
    node: &FsTreeNode,
    depth: usize,
) {
    if node.is_dir {
        let is_open = state.explorer_expanded.contains(&node.rel_path);
        let dir_dirty = git_status.modified_dirs.contains(&node.rel_path);
        let indent = (depth as f32) * 14.0;
        let (rect, row_resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 26.0),
            egui::Sense::click_and_drag(),
        );
        if ui.is_rect_visible(rect) {
            if row_resp.hovered() {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(6), p.bg_hover);
            }
            let fg = if row_resp.hovered() || is_open {
                p.text
            } else {
                p.muted
            };
            let x = rect.left() + indent + 8.0;
            draw_icon(
                ui.painter(),
                egui::Rect::from_center_size(
                    egui::pos2(x, rect.center().y),
                    egui::vec2(10.5, 10.5),
                ),
                if is_open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                fg,
            );
            draw_icon(
                ui.painter(),
                egui::Rect::from_center_size(
                    egui::pos2(x + 15.0, rect.center().y),
                    egui::vec2(13.5, 13.5),
                ),
                if is_open {
                    Icon::FolderOpen
                } else {
                    Icon::Folder
                },
                p.accent_hot,
            );
            let right_pad = if dir_dirty { 18.0 } else { 6.0 };
            let galley = egui::WidgetText::from(node.name.as_str()).into_galley(
                ui,
                Some(egui::TextWrapMode::Truncate),
                (rect.right() - (x + 27.0) - right_pad).max(20.0),
                egui::TextStyle::Button,
            );
            ui.painter().galley(
                egui::pos2(x + 26.0, rect.center().y - galley.size().y * 0.5),
                galley,
                fg,
            );
            if dir_dirty {
                ui.painter().circle_filled(
                    egui::pos2(rect.right() - 10.0, rect.center().y),
                    2.8,
                    p.amber,
                );
            }
        }
        if row_resp.drag_started() {
            state.dragging_explorer_item = Some((node.rel_path.clone(), true));
        }
        if row_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if row_resp.clicked() {
            if is_open {
                state.explorer_expanded.remove(&node.rel_path);
            } else {
                state.explorer_expanded.insert(node.rel_path.clone());
            }
        }

        let rel_clone = node.rel_path.clone();
        let abs_dir = rel_clone
            .split('/')
            .fold(ws_root.to_path_buf(), |acc, seg| acc.join(seg));
        row_resp.context_menu(|ui| {
            let toggle_txt = if is_open {
                "Collapse Folder"
            } else {
                "Expand Folder"
            };
            if ui.button(toggle_txt).clicked() {
                if is_open {
                    state.explorer_expanded.remove(&rel_clone);
                } else {
                    state.explorer_expanded.insert(rel_clone.clone());
                }
                ui.close_menu();
            }
            if ui.button("New File Here...").clicked() {
                state.explorer_expanded.insert(rel_clone.clone());
                state.prompt_modal = PromptModal::NewFile {
                    parent_rel: rel_clone.clone(),
                    name: String::new(),
                };
                ui.close_menu();
            }
            if ui.button("New Folder Here...").clicked() {
                state.explorer_expanded.insert(rel_clone.clone());
                state.prompt_modal = PromptModal::NewFolder {
                    parent_rel: rel_clone.clone(),
                    name: String::new(),
                };
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Rename...").clicked() {
                state.prompt_modal = PromptModal::RenameEntry {
                    old_rel: rel_clone.clone(),
                    new_name: node.name.clone(),
                };
                ui.close_menu();
            }
            if ui
                .button(egui::RichText::new("Delete Folder...").color(p.red))
                .clicked()
            {
                state.prompt_modal = PromptModal::DeleteEntry {
                    rel_path: rel_clone.clone(),
                    is_dir: true,
                };
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Copy Relative Path").clicked() {
                copy_to_clipboard(ui.ctx(), &rel_clone);
                state.set_toast(format!("Copied {}", rel_clone), false);
                ui.close_menu();
            }
            if ui.button("Copy Full Path").clicked() {
                let full = abs_dir.to_string_lossy().to_string();
                copy_to_clipboard(ui.ctx(), &full);
                state.set_toast(format!("Copied {}", full), false);
                ui.close_menu();
            }
            if ui.button("Reveal in Explorer").clicked() {
                backend.reveal_entry(abs_dir.clone());
                ui.close_menu();
            }
        });

        if is_open {
            for child in node.sorted_children() {
                render_tree_node(
                    ui,
                    state,
                    backend,
                    p,
                    ws_root,
                    git_status,
                    child,
                    depth + 1,
                );
            }
        }
    } else {
        render_leaf_row(
            ui,
            state,
            backend,
            p,
            ws_root,
            git_status,
            &node.name,
            &node.rel_path,
            node.size_bytes,
            &node.extension,
            depth,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_leaf_row(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
    ws_root: &PathBuf,
    git_status: &GitWorkspaceStatus,
    name: &str,
    rel_path: &str,
    size_bytes: u64,
    ext: &str,
    depth: usize,
) {
    let is_active = state.active_tab_path.as_deref() == Some(rel_path);
    let git_st = git_status.file_statuses.get(rel_path).copied();
    let icon = match ext {
        "rs" | "toml" | "py" | "js" | "ts" | "tsx" | "jsx" | "json" | "yaml" | "yml" | "sh"
        | "ps1" | "html" | "css" | "c" | "cpp" | "h" => Icon::FileCode,
        "png" | "jpg" | "jpeg" | "webp" | "svg" | "ico" | "gif" => Icon::FileImage,
        "pdf" | "md" | "txt" | "doc" | "docx" => Icon::FileDoc,
        _ => Icon::File,
    };

    let indent = (depth as f32) * 14.0 + 16.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 26.0),
        egui::Sense::click_and_drag(),
    );
    if ui.is_rect_visible(rect) {
        let fill = if is_active {
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
        if is_active {
            let tick = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 5.0),
                egui::vec2(2.0, rect.height() - 10.0),
            );
            ui.painter()
                .rect_filled(tick, CornerRadius::same(1), p.accent);
        }
        let icon_col = if is_active { p.accent_hot } else { p.muted };
        let fg = if is_active || resp.hovered() {
            p.text
        } else {
            p.muted
        };
        let x = rect.left() + indent;
        draw_icon(
            ui.painter(),
            egui::Rect::from_center_size(
                egui::pos2(x, rect.center().y),
                egui::vec2(13.0, 13.0),
            ),
            icon,
            icon_col,
        );
        let badge_pad = if git_st.is_some() { 20.0 } else { 6.0 };
        let galley = egui::WidgetText::from(name).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (rect.right() - (x + 14.0) - badge_pad).max(20.0),
            egui::TextStyle::Button,
        );
        ui.painter().galley(
            egui::pos2(x + 12.0, rect.center().y - galley.size().y * 0.5),
            galley,
            fg,
        );
        if let Some(st) = git_st {
            let scol = match st {
                GitFileStatus::Modified => p.amber,
                GitFileStatus::Added | GitFileStatus::Untracked => p.green,
                GitFileStatus::Deleted | GitFileStatus::Conflicted => p.red,
                GitFileStatus::Renamed => p.purple,
            };
            ui.painter().text(
                egui::pos2(rect.right() - 7.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                st.badge_letter(),
                egui::FontId::monospace(10.5),
                scol,
            );
        }
    }
    if resp.drag_started() {
        state.dragging_explorer_item = Some((rel_path.to_string(), false));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let hover_txt = match git_st {
        Some(st) => format!(
            "{rel_path} ({}) · Git: {}\nRight-click for actions",
            fmt_bytes(size_bytes),
            st.label()
        ),
        None => format!("{rel_path} ({})\nRight-click for actions", fmt_bytes(size_bytes)),
    };
    let resp = resp.on_hover_text(hover_txt);

    if resp.clicked() {
        backend.load_workspace_file(rel_path.to_string());
    }

    let rel_clone = rel_path.to_string();
    let abs_file = rel_clone
        .split('/')
        .fold(ws_root.to_path_buf(), |acc, seg| acc.join(seg));
    resp.context_menu(|ui| {
        if ui.button("Open in Editor").clicked() {
            backend.load_workspace_file(rel_clone.clone());
            ui.close_menu();
        }
        if git_st.is_some() && ui.button("View Git Diff vs HEAD").clicked() {
            state.git_diff_filter = Some(rel_clone.clone());
            state.diffs_view_mode = DiffsViewMode::GitHead;
            state.git_diff_loading = true;
            state.open_stage_tab(StageTab::Diffs);
            backend.load_git_diff(Some(rel_clone.clone()));
            ui.close_menu();
        }
        if ui.button("Attach to Chat").clicked() {
            if !state.attached_files.iter().any(|f| f == &rel_clone) {
                state.attached_files.push(rel_clone.clone());
                state.set_toast(format!("Attached {}", name), false);
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Rename...").clicked() {
            state.prompt_modal = PromptModal::RenameEntry {
                old_rel: rel_clone.clone(),
                new_name: name.to_string(),
            };
            ui.close_menu();
        }
        if ui
            .button(egui::RichText::new("Delete...").color(p.red))
            .clicked()
        {
            state.prompt_modal = PromptModal::DeleteEntry {
                rel_path: rel_clone.clone(),
                is_dir: false,
            };
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Copy Relative Path").clicked() {
            copy_to_clipboard(ui.ctx(), &rel_clone);
            state.set_toast(format!("Copied {}", rel_clone), false);
            ui.close_menu();
        }
        if ui.button("Copy Full Path").clicked() {
            let full = abs_file.to_string_lossy().to_string();
            copy_to_clipboard(ui.ctx(), &full);
            state.set_toast(format!("Copied {}", full), false);
            ui.close_menu();
        }
        if ui.button("Reveal in Explorer").clicked() {
            backend.reveal_entry(abs_file.clone());
            ui.close_menu();
        }
    });
}
