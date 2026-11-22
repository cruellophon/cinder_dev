pub mod app_state;
pub mod backend;
pub mod chat;
pub mod customizations;
pub mod editor;
pub mod explorer;
pub mod forge;
pub mod icons;
pub mod inspector;
pub mod markdown;
pub mod modals;
pub mod plugins;
pub mod settings;
pub mod sidebar;
pub mod theme;

use app_state::{CustomTab, EditorTab, ForgeTab, GuiState, MainView, SettingsTab, StageTab};
use backend::{BackendEvent, BackendHandle, FolderPickTarget};
use egui::{CornerRadius, Frame, Margin, Stroke};
use icons::{icon_button, Icon};
use image::ImageFormat;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use theme::{
    apply_palette, badge, draw_cinder_flame, install_system_fonts, pane_header, underline_tab,
    CinderPalette,
};

const CINDER_ICO_BYTES: &[u8] = include_bytes!("../../../bins/bumbo-app/ui/cinder.ico");

#[derive(Debug, Clone, Default)]
pub struct CinderLaunchOptions {
    pub screenshot_out: Option<PathBuf>,
    pub open_stage: bool,
    pub open_settings: bool,
    pub open_forge: bool,
    pub open_customizations: bool,
}

pub fn load_cinder_icon_data() -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(CINDER_ICO_BYTES, ImageFormat::Ico).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

pub fn run_native_cinder_with_launch(opts: CinderLaunchOptions) -> anyhow::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1560.0, 960.0])
        .with_min_inner_size([960.0, 640.0])
        .with_title("Cinder v5.5 — Local Agentic IDE & Model Forge")
        .with_drag_and_drop(true);
    #[cfg(target_os = "macos")]
    let mut viewport = viewport
        .with_decorations(true)
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(true);
    #[cfg(not(target_os = "macos"))]
    let mut viewport = viewport.with_decorations(false);
    if let Some(icon) = load_cinder_icon_data() {
        viewport = viewport.with_icon(Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Cinder",
        options,
        Box::new(move |cc| {
            install_system_fonts(&cc.egui_ctx);
            let mut app = CinderApp::new(&cc.egui_ctx)?;
            if opts.open_stage {
                app.state.show_workbench = true;
                app.state.show_explorer = true;
            }
            if opts.open_settings {
                app.state.open_settings(SettingsTab::Appearance);
            }
            if opts.open_forge {
                app.state
                    .switch_main_view(MainView::Forge(ForgeTab::Vault), false);
            }
            if opts.open_customizations {
                app.state
                    .switch_main_view(MainView::Customizations(CustomTab::Plugins), false);
            }
            if let Some(path) = opts.screenshot_out.clone() {
                app.screenshot = Some(ScreenshotCapture::new(path));
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe native window failed: {e}"))
}

struct ScreenshotCapture {
    out_path: PathBuf,
    frames_seen: u32,
    started_at: std::time::Instant,
    requested: bool,
    saved: bool,
}

impl ScreenshotCapture {
    fn new(out_path: PathBuf) -> Self {
        Self {
            out_path,
            frames_seen: 0,
            started_at: std::time::Instant::now(),
            requested: false,
            saved: false,
        }
    }
}

struct CinderApp {
    backend: Arc<BackendHandle>,
    rx: Receiver<BackendEvent>,
    state: GuiState,
    last_applied_theme_key: String,
    screenshot: Option<ScreenshotCapture>,
}

#[cfg(windows)]
fn enable_win32_drag_drop_filters() {
    #[link(name = "user32")]
    extern "system" {
        fn ChangeWindowMessageFilter(message: u32, dw_flag: u32) -> i32;
    }
    const WM_DROPFILES: u32 = 0x0233;
    const WM_COPYDATA: u32 = 0x004A;
    const WM_COPYGLOBALDATA: u32 = 0x0049;
    const MSGFLT_ADD: u32 = 1;
    unsafe {
        ChangeWindowMessageFilter(WM_DROPFILES, MSGFLT_ADD);
        ChangeWindowMessageFilter(WM_COPYDATA, MSGFLT_ADD);
        ChangeWindowMessageFilter(WM_COPYGLOBALDATA, MSGFLT_ADD);
    }
}

#[cfg(windows)]
fn poll_win32_ctrl_v_down() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(v_key: i32) -> i16;
    }
    const VK_CONTROL: i32 = 0x11;
    const VK_V: i32 = 0x56;
    unsafe {
        let ctrl_down = (GetAsyncKeyState(VK_CONTROL) as u16 & 0x8000) != 0;
        let v_down = (GetAsyncKeyState(VK_V) as u16 & 0x8000) != 0;
        ctrl_down && v_down
    }
}

impl CinderApp {
    fn new(ctx: &egui::Context) -> anyhow::Result<Self> {
        #[cfg(windows)]
        enable_win32_drag_drop_filters();
        let (backend, rx) = BackendHandle::start(ctx.clone())?;
        let state = GuiState::new();
        let default_ui = harness_store::UiCustomization::default();
        let p = CinderPalette::from_ui(&default_ui);
        apply_palette(ctx, &p);

        Ok(Self {
            backend,
            rx,
            state,
            last_applied_theme_key: String::new(),
            screenshot: None,
        })
    }

    fn process_screenshot(&mut self, ctx: &egui::Context) {
        let Some(ref mut cap) = self.screenshot else {
            return;
        };
        let captured = ctx.input(|i| {
            i.raw.events.iter().find_map(|ev| match ev {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(img) = captured {
            if !cap.saved {
                let w = img.size[0] as u32;
                let h = img.size[1] as u32;
                let mut rgba = Vec::with_capacity(img.pixels.len() * 4);
                for px in &img.pixels {
                    rgba.extend_from_slice(&[px.r(), px.g(), px.b(), 255]);
                }
                if let Some(buf) = image::RgbaImage::from_raw(w, h, rgba) {
                    if let Some(parent) = cap.out_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = buf.save(&cap.out_path);
                }
                cap.saved = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        if !cap.requested {
            cap.frames_seen += 1;
            if (self.state.snap.is_some() && cap.frames_seen >= 8)
                || cap.started_at.elapsed().as_secs() >= 6
            {
                cap.requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::UserData::default(),
                ));
            }
        }
        ctx.request_repaint();
    }

    fn drain_backend_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                BackendEvent::FullSnapshot(snap) => {
                    self.state.sync_from_snapshot(&snap);
                    self.state.snap = Some(*snap);
                }
                BackendEvent::LiveTick {
                    live_activity,
                    downloads,
                    install_jobs,
                    server_status,
                    privacy_audit,
                    was_downloading_or_installing_finished,
                } => {
                    if let Some(ref mut snap) = self.state.snap {
                        snap.settings.privacy_mode = privacy_audit.privacy_mode;
                        snap.live_activity = live_activity;
                        snap.downloads = downloads;
                        snap.install_jobs = install_jobs;
                        snap.server_status = server_status;
                        snap.privacy_audit = privacy_audit;
                    }
                    if was_downloading_or_installing_finished {
                        self.state.set_toast("Background task finished.", false);
                    }
                }
                BackendEvent::TurnFinished(res) => {
                    self.state.is_generating = false;
                    match res {
                        Ok(sess) => {
                            if let Some(ref mut snap) = self.state.snap {
                                snap.active_session = Some(sess);
                            }
                        }
                        Err(err_msg) => {
                            self.state.set_toast(err_msg, true);
                        }
                    }
                }
                BackendEvent::ChatRewound {
                    session,
                    prompt_text,
                    attached_files,
                    attachments,
                } => {
                    if let Some(ref mut snap) = self.state.snap {
                        snap.active_session = Some(session);
                    }
                    if !prompt_text.is_empty()
                        || !attached_files.is_empty()
                        || !attachments.is_empty()
                    {
                        self.state.prompt_text = prompt_text;
                        self.state.attached_files = attached_files;
                        self.state.attached_media = attachments;
                    }
                }
                BackendEvent::Toast { msg, is_error } => {
                    self.state.set_toast(msg, is_error);
                }
                BackendEvent::FolderPicked { target, path } => {
                    if target == FolderPickTarget::Vault {
                        self.state.vault_dir_draft = path;
                    }
                }
                BackendEvent::FilesPicked(paths) => {
                    let pbs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
                    if let Some(open_dir) = self.state.attach_external_paths(&pbs) {
                        self.backend.select_workspace(open_dir);
                    }
                    self.state.main_view = MainView::Chat;
                }
                BackendEvent::FileLoaded(payload) => {
                    let rel = payload.rel_path.clone();
                    if let Some(pos) = self.state.editor_tabs.iter().position(|t| t.rel_path == rel) {
                        self.state.editor_tabs[pos] = EditorTab::from_payload(payload);
                    } else {
                        self.state.editor_tabs.push(EditorTab::from_payload(payload));
                    }
                    self.state.active_tab_path = Some(rel);
                    self.state.open_stage_tab(StageTab::Editor);
                }
                BackendEvent::HfSearchResults(res) => {
                    self.state.hf_results = res;
                }
                BackendEvent::HfRepoInspected(det) => {
                    self.state.hf_repo_details = Some(det);
                }
                BackendEvent::ProviderVerifyResults(statuses) => {
                    self.state.verify_statuses = statuses;
                }
                BackendEvent::KnowledgeIndexed(st) => {
                    self.state.kb_stats = Some(st);
                }
                BackendEvent::KnowledgeHits(hits) => {
                    self.state.kb_hits = hits;
                }
                BackendEvent::TerminalOutput {
                    cmd,
                    exit_code,
                    output,
                    duration_ms,
                } => {
                    self.state.terminal_output = format!(
                        "$ {cmd}\n\n{output}\n\n[exit {exit_code} in {duration_ms}ms]"
                    );
                }
                BackendEvent::McpProbed(probe) => {
                    self.state.mcp_probes.insert(probe.server_id.clone(), probe);
                }
                BackendEvent::AntigravityExported(json_str) => {
                    self.state.agy_export_json = Some(json_str);
                }
                BackendEvent::EntryRenamed { old_rel, new_rel } => {
                    for tab in &mut self.state.editor_tabs {
                        if tab.rel_path == old_rel {
                            tab.rel_path = new_rel.clone();
                        }
                    }
                    if self.state.active_tab_path.as_deref() == Some(&old_rel) {
                        self.state.active_tab_path = Some(new_rel);
                    }
                }
                BackendEvent::EntryDeleted { rel_path } => {
                    self.state.editor_tabs.retain(|t| t.rel_path != rel_path);
                    if self.state.active_tab_path.as_deref() == Some(&rel_path) {
                        self.state.active_tab_path =
                            self.state.editor_tabs.last().map(|t| t.rel_path.clone());
                    }
                }
            }
        }
    }

    fn handle_shortcuts_and_dnd(&mut self, ctx: &egui::Context) {
        let wants_kbd = ctx.wants_keyboard_input();
        let (focused, ctrl_active, has_text_paste, fallback_paste_evt, dropped) = ctx.input(|i| {
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if cmd && i.key_pressed(egui::Key::B) {
                self.state.show_sidebar = !self.state.show_sidebar;
            }
            if self.state.main_view == MainView::Chat {
                if cmd && i.modifiers.shift && i.key_pressed(egui::Key::E) {
                    self.state.show_explorer = !self.state.show_explorer;
                } else if cmd && i.key_pressed(egui::Key::E) {
                    self.state.show_workbench = !self.state.show_workbench;
                }
                if cmd && i.key_pressed(egui::Key::Backtick) {
                    self.state.show_terminal = !self.state.show_terminal;
                    if self.state.show_terminal {
                        self.state.open_stage_tab(StageTab::Editor);
                    }
                }
            }
            if cmd && i.key_pressed(egui::Key::K) {
                self.state.show_cmd_palette = !self.state.show_cmd_palette;
            }
            if cmd && i.key_pressed(egui::Key::Comma) {
                if self.state.show_settings_modal {
                    self.state.show_settings_modal = false;
                } else {
                    self.state.open_settings(self.state.settings_tab);
                }
            }
            if cmd && i.key_pressed(egui::Key::S) {
                if let Some(ap) = self.state.active_tab_path.clone() {
                    if let Some(tab) = self.state.editor_tabs.iter_mut().find(|t| t.rel_path == ap)
                    {
                        if tab.kind == "text" {
                            let content = tab.content.clone();
                            tab.saved_content = content.clone();
                            self.backend.save_workspace_file(ap, content);
                        }
                    }
                }
            }
            if self.state.main_view == MainView::Chat
                && self.state.show_workbench
                && self.state.stage_tab == StageTab::Editor
            {
                if cmd && i.key_pressed(egui::Key::F) {
                    self.state.editor_find_open = !self.state.editor_find_open;
                }
                if cmd && i.key_pressed(egui::Key::W) {
                    if let Some(ap) = self.state.active_tab_path.clone() {
                        self.state.editor_tabs.retain(|t| t.rel_path != ap);
                        self.state.active_tab_path =
                            self.state.editor_tabs.last().map(|t| t.rel_path.clone());
                    }
                }
            }
            if i.key_pressed(egui::Key::Escape) {
                self.state.show_cmd_palette = false;
                self.state.show_status_popover = false;
                self.state.show_downloads_popover = false;
                self.state.show_settings_modal = false;
                self.state.dragging_explorer_item = None;
            }

            let ctrl_active = cmd;
            let has_text_paste = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Paste(s) if !s.is_empty()));
            let key_v_down = ctrl_active && i.key_pressed(egui::Key::V);
            let recent_ctrl = self
                .state
                .last_ctrl_down_at
                .map(|t| t.elapsed() <= std::time::Duration::from_millis(350))
                .unwrap_or(false);
            let key_v_released_after_ctrl = (ctrl_active || recent_ctrl)
                && i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::V,
                            pressed: false,
                            ..
                        }
                    )
                });
            let shift_insert = i.modifiers.shift && i.key_pressed(egui::Key::Insert);
            let empty_paste = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Paste(s) if s.is_empty()));
            let fallback_paste_evt =
                key_v_down || key_v_released_after_ctrl || shift_insert || empty_paste;

            (
                i.focused,
                ctrl_active,
                has_text_paste,
                fallback_paste_evt,
                i.raw.dropped_files.clone(),
            )
        });

        if ctrl_active {
            self.state.last_ctrl_down_at = Some(std::time::Instant::now());
        }

        // Ctrl+V Screenshot / Image / File paste into Chat
        // Note: egui-winit 0.31 swallows Ctrl+V key-down when clipboard has an image (no text),
        // so we poll Win32 GetAsyncKeyState and combine with key-release/empty-paste fallbacks.
        #[cfg(windows)]
        let win32_ctrl_v = focused && poll_win32_ctrl_v_down();
        #[cfg(not(windows))]
        let win32_ctrl_v = {
            let _ = focused;
            false
        };

        let rising_ctrl_v = win32_ctrl_v && !self.state.ctrl_v_latch;
        if win32_ctrl_v {
            self.state.ctrl_v_latch = true;
            self.state.last_ctrl_down_at = Some(std::time::Instant::now());
        } else {
            // Reset latch only after keys are released so key-release in the next frame won't double-fire.
            if self.state.ctrl_v_latch && !win32_ctrl_v && !fallback_paste_evt {
                self.state.ctrl_v_latch = false;
            }
        }

        let can_paste_in_chat = self.state.main_view == MainView::Chat
            && !self.state.show_settings_modal
            && !self.state.show_cmd_palette
            && (self.state.composer_focused || !wants_kbd);

        if can_paste_in_chat && !has_text_paste && (rising_ctrl_v || (!self.state.ctrl_v_latch && fallback_paste_evt)) {
            if win32_ctrl_v {
                self.state.ctrl_v_latch = true;
            }
            self.state.attach_clipboard_image();
        }

        // Native OS Drag & Drop onto window (paths AND raw dropped bytes)
        if !dropped.is_empty() {
            let mut paths = Vec::new();
            let mut any_attached = false;
            for df in dropped {
                if let Some(p) = df.path {
                    paths.push(p);
                } else if let Some(bytes) = df.bytes {
                    let fname = if df.name.trim().is_empty() {
                        "dropped-attachment.bin".to_string()
                    } else {
                        df.name.clone()
                    };
                    self.state.attach_dropped_bytes(fname, &bytes);
                    any_attached = true;
                }
            }
            if !paths.is_empty() {
                if let Some(open_dir) = self.state.attach_external_paths(&paths) {
                    self.backend.select_workspace(open_dir);
                }
                any_attached = true;
            }
            if any_attached {
                self.state.main_view = MainView::Chat;
                self.state.show_settings_modal = false;
                self.state.show_cmd_palette = false;
                ctx.request_repaint();
            }
        }
    }
}

impl eframe::App for CinderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_backend_events();
        self.handle_shortcuts_and_dnd(ctx);

        let ui_cfg = self
            .state
            .snap
            .as_ref()
            .map(|s| s.settings.ui.clone())
            .unwrap_or_default();
        let theme_key = format!(
            "{}|{}|{}",
            ui_cfg.theme, ui_cfg.accent_color, ui_cfg.font_scale
        );
        let p = CinderPalette::from_ui(&ui_cfg);
        if theme_key != self.last_applied_theme_key {
            apply_palette(ctx, &p);
            self.last_applied_theme_key = theme_key;
        }

        // App-styled frameless Title / Menu Bar with single 1px bottom hairline separator
        egui::TopBottomPanel::top("cinder_topbar")
            .exact_height(if cfg!(target_os = "macos") { 34.0 } else { 30.0 })
            .show_separator_line(true)
            .frame(
                Frame::new()
                    .fill(p.bg_sidebar)
                    .stroke(Stroke::NONE)
                    .inner_margin(Margin::symmetric(8, 3)),
            )
            .show(ctx, |ui| {
                sidebar::render_topbar(ctx, ui, &mut self.state, &self.backend, &p);
            });

        if self.state.show_settings_modal {
            egui::SidePanel::left("cinder_settings_rail")
                .resizable(false)
                .exact_width(242.0)
                .show_separator_line(true)
                .frame(
                    Frame::new()
                        .fill(p.bg_sidebar)
                        .stroke(Stroke::NONE)
                        .inner_margin(Margin::ZERO),
                )
                .show(ctx, |ui| {
                    settings::render_settings_rail(ui, &mut self.state, &p);
                });

            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg).inner_margin(Margin::ZERO))
                .show(ctx, |ui| {
                    settings::render_settings_content(
                        ctx,
                        ui,
                        &mut self.state,
                        &self.backend,
                        &p,
                    );
                });
        } else {
            // Left Navigation Sidebar
            if self.state.show_sidebar {
                egui::SidePanel::left("cinder_left_sidebar")
                    .resizable(true)
                    .default_width(self.state.sidebar_width)
                    .width_range(196.0..=340.0)
                    .show_separator_line(true)
                    .frame(
                        Frame::new()
                            .fill(p.bg_sidebar)
                            .stroke(Stroke::NONE)
                            .inner_margin(Margin::ZERO),
                    )
                    .show(ctx, |ui| {
                        sidebar::render_left_sidebar(ui, &mut self.state, &self.backend, &p);
                    });
            }

            let mut explorer_rect_opt: Option<egui::Rect> = None;
            let mut workbench_rect_opt: Option<egui::Rect> = None;
            if self.state.main_view == MainView::Chat {
                // Far-Right Workspace File Explorer (only in Chat view)
                if self.state.show_explorer {
                    let exp_resp = egui::SidePanel::right("cinder_right_explorer")
                        .resizable(true)
                        .default_width(self.state.explorer_width)
                        .width_range(185.0..=400.0)
                        .show_separator_line(true)
                        .frame(
                            Frame::new()
                                .fill(p.bg_sidebar)
                                .stroke(Stroke::NONE)
                                .inner_margin(Margin::ZERO),
                        )
                        .show(ctx, |ui| {
                            explorer::render_explorer_panel(ui, &mut self.state, &self.backend, &p);
                        });
                    explorer_rect_opt = Some(exp_resp.response.rect);
                }

                // Middle-Right Coding Stage (Editor, Artifacts, Diffs)
                if self.state.show_workbench {
                    let max_wb_w = (ctx.screen_rect().width() * 0.58).max(460.0);
                    let wb_resp = egui::SidePanel::right("cinder_workbench_stage")
                        .resizable(true)
                        .default_width((ctx.screen_rect().width() * 0.34).clamp(360.0, max_wb_w))
                        .width_range(340.0..=max_wb_w)
                        .show_separator_line(true)
                        .frame(
                            Frame::new()
                                .fill(p.bg_elev)
                                .stroke(Stroke::NONE)
                                .inner_margin(Margin::ZERO),
                        )
                        .show(ctx, |ui| {
                            render_coding_stage(ctx, ui, &mut self.state, &self.backend, &p);
                        });
                    workbench_rect_opt = Some(wb_resp.response.rect);
                }
            }

            // Center Full-Canvas Workspace View
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg).inner_margin(Margin::ZERO))
                .show(ctx, |ui| match self.state.main_view {
                    MainView::Chat => {
                        chat::render_chat_column(ctx, ui, &mut self.state, &self.backend, &p);
                    }
                    MainView::Automations => {
                        customizations::render_automations_page(
                            ctx,
                            ui,
                            &mut self.state,
                            &self.backend,
                            &p,
                        );
                    }
                    MainView::Forge(tab) => {
                        forge::render_forge_page(ctx, ui, &mut self.state, &self.backend, &p, tab);
                    }
                    MainView::Customizations(tab) => {
                        customizations::render_customizations_page(
                            ctx,
                            ui,
                            &mut self.state,
                            &self.backend,
                            &p,
                            tab,
                        );
                    }
                });

            // In-app File Explorer Drag & Drop -> attach to Chat or open in Editor
            if let Some((drag_rel, is_dir)) = self.state.dragging_explorer_item.clone() {
                if let Some(pos) = ctx.pointer_interact_pos().or_else(|| ctx.pointer_hover_pos()) {
                    let over_wb = workbench_rect_opt.map(|r| r.contains(pos)).unwrap_or(false);
                    let over_exp = explorer_rect_opt.map(|r| r.contains(pos)).unwrap_or(false);
                    let tip = if over_wb && !is_dir {
                        format!("Drop to open {drag_rel} in Editor")
                    } else if !over_exp {
                        format!("Drop to attach {drag_rel} to Chat")
                    } else {
                        drag_rel.clone()
                    };
                    egui::Area::new(egui::Id::new("cinder_explorer_drag_ghost"))
                        .order(egui::Order::Tooltip)
                        .fixed_pos(pos + egui::vec2(14.0, 14.0))
                        .interactable(false)
                        .show(ctx, |ui| {
                            Frame::new()
                                .fill(p.bg_card)
                                .stroke(Stroke::new(1.0_f32, p.accent))
                                .corner_radius(CornerRadius::same(7))
                                .inner_margin(Margin::symmetric(10, 5))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new(tip).small().color(p.text));
                                });
                        });
                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);

                    if ctx.input(|i| i.pointer.any_released()) {
                        self.state.dragging_explorer_item = None;
                        if over_wb && !is_dir {
                            self.backend.load_workspace_file(drag_rel);
                        } else if !over_exp {
                            let ws_root = self
                                .state
                                .snap
                                .as_ref()
                                .map(|s| s.settings.active_workspace_path.clone())
                                .unwrap_or_default();
                            if !ws_root.trim().is_empty() {
                                let abs = drag_rel
                                    .split('/')
                                    .fold(PathBuf::from(&ws_root), |acc, seg| acc.join(seg));
                                self.state.attach_external_paths(&[abs]);
                            } else if !self.state.attached_files.iter().any(|f| f == &drag_rel) {
                                self.state.attached_files.push(drag_rel.clone());
                                self.state.set_toast(format!("Attached {drag_rel} to chat."), false);
                            }
                        }
                    }
                } else if ctx.input(|i| !i.pointer.any_down()) {
                    self.state.dragging_explorer_item = None;
                }
            }
        }

        // Command Palette, Prompt Modals, Lightbox, Toasts
        modals::render_modals_and_toasts(ctx, &mut self.state, &self.backend, &p);

        // Native OS Drag-and-Drop Overlay when hovering files over window
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering_files {
            egui::Area::new(egui::Id::new("cinder_dnd_overlay"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .interactable(false)
                .show(ctx, |ui| {
                    Frame::new()
                        .fill(p.bg_card)
                        .stroke(Stroke::new(2.0_f32, p.accent))
                        .corner_radius(CornerRadius::same(14))
                        .inner_margin(Margin::symmetric(36, 26))
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(48.0, 48.0),
                                    egui::Sense::hover(),
                                );
                                draw_cinder_flame(ui.painter(), r.center(), 18.0, &p);
                                ui.label(
                                    egui::RichText::new("Drop Files or Pictures into Cinder")
                                        .heading()
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(
                                        "Images route to Vision · PDFs & code attach cleanly",
                                    )
                                    .color(p.muted),
                                );
                            });
                        });
                });
        }

        handle_frameless_resize_edges(ctx, &p);
        self.process_screenshot(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.backend.shutdown_local_models();
    }
}

fn render_coding_stage(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    state: &mut GuiState,
    backend: &Arc<BackendHandle>,
    p: &CinderPalette,
) {
    let art_count = state
        .snap
        .as_ref()
        .and_then(|s| s.active_session.as_ref())
        .map(|s| s.artifacts.len())
        .unwrap_or(0);

    pane_header(ui, p, |ui| {
        if underline_tab(ui, p, state.stage_tab == StageTab::Editor, "Editor").clicked() {
            state.stage_tab = StageTab::Editor;
        }
        let art_lbl = if art_count > 0 {
            format!("Artifacts ({art_count})")
        } else {
            "Artifacts".into()
        };
        if underline_tab(ui, p, state.stage_tab == StageTab::Artifacts, &art_lbl).clicked() {
            state.stage_tab = StageTab::Artifacts;
        }
        if underline_tab(ui, p, state.stage_tab == StageTab::Diffs, "Diffs").clicked() {
            state.stage_tab = StageTab::Diffs;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, p, Icon::Close, false, "Close Stage (Ctrl+E)").clicked() {
                state.show_workbench = false;
            }
            if icon_button(
                ui,
                p,
                Icon::Terminal,
                state.show_terminal,
                "Toggle Terminal (Ctrl+`)",
            )
            .clicked()
            {
                state.show_terminal = !state.show_terminal;
                if state.show_terminal {
                    state.stage_tab = StageTab::Editor;
                }
            }
            if let Some(ref snap) = state.snap {
                if snap.server_status.running {
                    badge(ui, "VRAM Live", p.green, p.bg_card);
                }
            }
        });
    });

    Frame::new()
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| match state.stage_tab {
            StageTab::Editor => editor::render_editor_view(ctx, ui, state, backend, p),
            StageTab::Artifacts | StageTab::Diffs => {
                inspector::render_inspector_view(ctx, ui, state, backend, p)
            }
        });
}

fn handle_frameless_resize_edges(ctx: &egui::Context, p: &CinderPalette) {
    let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let screen = ctx.screen_rect();
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("cinder_window_border"),
    ))
    .rect_stroke(
        screen,
        CornerRadius::ZERO,
        Stroke::new(1.0_f32, p.border),
        egui::StrokeKind::Inside,
    );
    if is_maximized {
        return;
    }

    let edge = 4.0_f32;
    let top_edge = 3.0_f32;
    let corner = 8.0_f32;
    if let Some(pos) = ctx.pointer_hover_pos() {
        let near_l = pos.x <= screen.left() + edge;
        let near_r = pos.x >= screen.right() - edge;
        let near_t = pos.y <= screen.top() + top_edge;
        let near_b = pos.y >= screen.bottom() - edge;
        let in_l_corner = pos.x <= screen.left() + corner;
        let in_r_corner = pos.x >= screen.right() - corner;
        let in_t_corner = pos.y <= screen.top() + corner;
        let in_b_corner = pos.y >= screen.bottom() - corner;

        let dir = if (near_t && in_l_corner) || (near_l && in_t_corner) {
            Some((
                egui::ResizeDirection::NorthWest,
                egui::CursorIcon::ResizeNwSe,
            ))
        } else if (near_t && in_r_corner) || (near_r && in_t_corner) {
            Some((
                egui::ResizeDirection::NorthEast,
                egui::CursorIcon::ResizeNeSw,
            ))
        } else if (near_b && in_l_corner) || (near_l && in_b_corner) {
            Some((
                egui::ResizeDirection::SouthWest,
                egui::CursorIcon::ResizeNeSw,
            ))
        } else if (near_b && in_r_corner) || (near_r && in_b_corner) {
            Some((
                egui::ResizeDirection::SouthEast,
                egui::CursorIcon::ResizeNwSe,
            ))
        } else if near_l {
            Some((
                egui::ResizeDirection::West,
                egui::CursorIcon::ResizeHorizontal,
            ))
        } else if near_r {
            Some((
                egui::ResizeDirection::East,
                egui::CursorIcon::ResizeHorizontal,
            ))
        } else if near_t {
            Some((
                egui::ResizeDirection::North,
                egui::CursorIcon::ResizeVertical,
            ))
        } else if near_b {
            Some((
                egui::ResizeDirection::South,
                egui::CursorIcon::ResizeVertical,
            ))
        } else {
            None
        };

        if let Some((resize_dir, cursor)) = dir {
            ctx.set_cursor_icon(cursor);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(resize_dir));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_embedded_cinder_ico_icon() {
        let icon = load_cinder_icon_data().expect("cinder.ico should decode cleanly");
        assert!(icon.width >= 16);
        assert!(icon.height >= 16);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
