use crate::icons::{draw_icon, Icon};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Frame, Margin, Stroke, Visuals,
};
use harness_store::UiCustomization;

pub const HEADER_HEIGHT: f32 = 40.0;

#[derive(Debug, Clone, Copy)]
pub struct CinderPalette {
    pub is_light: bool,
    pub bg: Color32,
    pub bg_sidebar: Color32,
    pub bg_elev: Color32,
    pub bg_card: Color32,
    pub bg_hover: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hot: Color32,
    pub accent_soft: Color32,
    pub green: Color32,
    pub amber: Color32,
    pub red: Color32,
    pub purple: Color32,
    pub base_font_size: f32,
}

impl CinderPalette {
    pub fn from_ui(ui: &UiCustomization) -> Self {
        let mut p = match ui.theme.as_str() {
            "singularity_obsidian" => Self {
                is_light: false,
                bg: Color32::from_rgb(7, 8, 11),
                bg_sidebar: Color32::from_rgb(12, 13, 18),
                bg_elev: Color32::from_rgb(17, 19, 26),
                bg_card: Color32::from_rgb(22, 25, 35),
                bg_hover: Color32::from_rgb(29, 33, 48),
                border: Color32::from_rgb(44, 48, 60),
                border_strong: Color32::from_rgb(70, 76, 94),
                text: Color32::from_rgb(236, 238, 242),
                muted: Color32::from_rgb(142, 148, 164),
                accent: Color32::from_rgb(255, 110, 48),
                accent_hot: Color32::from_rgb(255, 163, 72),
                accent_soft: Color32::from_rgba_premultiplied(50, 24, 14, 220),
                green: Color32::from_rgb(61, 214, 140),
                amber: Color32::from_rgb(255, 156, 66),
                red: Color32::from_rgb(240, 82, 87),
                purple: Color32::from_rgb(179, 136, 255),
                base_font_size: 13.5,
            },
            "carbon_midnight" => Self {
                is_light: false,
                bg: Color32::from_rgb(7, 8, 11),
                bg_sidebar: Color32::from_rgb(11, 12, 16),
                bg_elev: Color32::from_rgb(16, 18, 24),
                bg_card: Color32::from_rgb(22, 25, 34),
                bg_hover: Color32::from_rgb(31, 35, 48),
                border: Color32::from_rgb(42, 46, 58),
                border_strong: Color32::from_rgb(68, 74, 92),
                text: Color32::from_rgb(236, 238, 242),
                muted: Color32::from_rgb(140, 146, 162),
                accent: Color32::from_rgb(255, 110, 48),
                accent_hot: Color32::from_rgb(255, 163, 72),
                accent_soft: Color32::from_rgba_premultiplied(50, 24, 14, 220),
                green: Color32::from_rgb(61, 214, 140),
                amber: Color32::from_rgb(255, 156, 66),
                red: Color32::from_rgb(240, 82, 87),
                purple: Color32::from_rgb(179, 136, 255),
                base_font_size: 13.5,
            },
            "nordic_slate" => Self {
                is_light: false,
                bg: Color32::from_rgb(20, 24, 32),
                bg_sidebar: Color32::from_rgb(24, 29, 38),
                bg_elev: Color32::from_rgb(31, 37, 49),
                bg_card: Color32::from_rgb(38, 46, 60),
                bg_hover: Color32::from_rgb(46, 55, 72),
                border: Color32::from_rgb(56, 66, 84),
                border_strong: Color32::from_rgb(80, 94, 118),
                text: Color32::from_rgb(236, 238, 242),
                muted: Color32::from_rgb(147, 160, 181),
                accent: Color32::from_rgb(255, 110, 48),
                accent_hot: Color32::from_rgb(255, 163, 72),
                accent_soft: Color32::from_rgba_premultiplied(54, 28, 18, 220),
                green: Color32::from_rgb(61, 214, 140),
                amber: Color32::from_rgb(255, 156, 66),
                red: Color32::from_rgb(240, 82, 87),
                purple: Color32::from_rgb(179, 136, 255),
                base_font_size: 13.5,
            },
            "paper_light" => Self {
                is_light: true,
                bg: Color32::from_rgb(247, 245, 243),
                bg_sidebar: Color32::from_rgb(255, 255, 255),
                bg_elev: Color32::from_rgb(255, 255, 255),
                bg_card: Color32::from_rgb(240, 236, 232),
                bg_hover: Color32::from_rgb(231, 225, 220),
                border: Color32::from_rgb(218, 212, 206),
                border_strong: Color32::from_rgb(184, 174, 166),
                text: Color32::from_rgb(27, 22, 20),
                muted: Color32::from_rgb(111, 98, 92),
                accent: Color32::from_rgb(228, 86, 28),
                accent_hot: Color32::from_rgb(245, 118, 42),
                accent_soft: Color32::from_rgb(252, 232, 222),
                green: Color32::from_rgb(18, 138, 80),
                amber: Color32::from_rgb(198, 108, 18),
                red: Color32::from_rgb(206, 44, 52),
                purple: Color32::from_rgb(122, 74, 218),
                base_font_size: 13.5,
            },
            // Default: "cinder_forge" — Calm Neutral Obsidian-Charcoal with Warm Cinder Glow
            _ => Self {
                is_light: false,
                bg: Color32::from_rgb(17, 18, 22),
                bg_sidebar: Color32::from_rgb(13, 14, 17),
                bg_elev: Color32::from_rgb(22, 24, 30),
                bg_card: Color32::from_rgb(27, 30, 38),
                bg_hover: Color32::from_rgb(34, 38, 48),
                border: Color32::from_rgb(38, 42, 52),
                border_strong: Color32::from_rgb(56, 62, 76),
                text: Color32::from_rgb(233, 237, 244),
                muted: Color32::from_rgb(138, 147, 165),
                accent: Color32::from_rgb(244, 113, 59),
                accent_hot: Color32::from_rgb(255, 166, 94),
                accent_soft: Color32::from_rgb(52, 32, 24),
                green: Color32::from_rgb(64, 206, 142),
                amber: Color32::from_rgb(245, 168, 72),
                red: Color32::from_rgb(236, 92, 96),
                purple: Color32::from_rgb(174, 138, 250),
                base_font_size: 13.5,
            },
        };

        let (accent, accent_hot, accent_soft) = match ui.accent_color.as_str() {
            "amber" => (
                Color32::from_rgb(229, 153, 62),
                Color32::from_rgb(255, 190, 107),
                if p.is_light {
                    Color32::from_rgb(252, 239, 218)
                } else {
                    Color32::from_rgb(56, 40, 20)
                },
            ),
            "blue" => (
                Color32::from_rgb(59, 130, 246),
                Color32::from_rgb(96, 165, 250),
                if p.is_light {
                    Color32::from_rgb(224, 236, 254)
                } else {
                    Color32::from_rgb(22, 38, 66)
                },
            ),
            "emerald" => (
                Color32::from_rgb(16, 185, 129),
                Color32::from_rgb(52, 211, 153),
                if p.is_light {
                    Color32::from_rgb(218, 248, 236)
                } else {
                    Color32::from_rgb(18, 52, 40)
                },
            ),
            "violet" => (
                Color32::from_rgb(139, 92, 246),
                Color32::from_rgb(167, 139, 250),
                if p.is_light {
                    Color32::from_rgb(236, 228, 254)
                } else {
                    Color32::from_rgb(42, 28, 66)
                },
            ),
            "mono" => (
                if p.is_light {
                    Color32::from_rgb(66, 60, 56)
                } else {
                    Color32::from_rgb(214, 204, 197)
                },
                if p.is_light {
                    Color32::from_rgb(36, 32, 30)
                } else {
                    Color32::from_rgb(243, 236, 231)
                },
                if p.is_light {
                    Color32::from_rgb(226, 222, 218)
                } else {
                    Color32::from_rgb(54, 48, 46)
                },
            ),
            _ => (p.accent, p.accent_hot, p.accent_soft),
        };
        p.accent = accent;
        p.accent_hot = accent_hot;
        p.accent_soft = accent_soft;

        p.base_font_size = match ui.font_scale.as_str() {
            "compact" => 12.5,
            "comfortable" => 15.0,
            _ => 13.5,
        };
        p
    }

    pub fn card_frame(&self) -> Frame {
        Frame::new()
            .fill(self.bg_elev)
            .stroke(Stroke::new(1.0_f32, self.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 12))
    }

    pub fn subcard_frame(&self) -> Frame {
        Frame::new()
            .fill(self.bg_card)
            .stroke(Stroke::new(1.0_f32, self.border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 9))
    }
}

pub fn install_system_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    #[cfg(windows)]
    {
        let proportional_candidates = [
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ];
        for path in proportional_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("win_ui".to_owned(), FontData::from_owned(bytes).into());
                if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                    fam.insert(0, "win_ui".to_owned());
                }
                break;
            }
        }

        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisb.ttf") {
            fonts
                .font_data
                .insert("win_ui_sb".to_owned(), FontData::from_owned(bytes).into());
            let mut sb_fam = fonts
                .families
                .get(&FontFamily::Proportional)
                .cloned()
                .unwrap_or_default();
            sb_fam.insert(0, "win_ui_sb".to_owned());
            fonts
                .families
                .insert(FontFamily::Name("semibold".into()), sb_fam);
        }

        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
            fonts
                .font_data
                .insert("win_sym".to_owned(), FontData::from_owned(bytes).into());
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                let idx = usize::min(1, fam.len());
                fam.insert(idx, "win_sym".to_owned());
            }
            if let Some(sb) = fonts.families.get_mut(&FontFamily::Name("semibold".into())) {
                let idx = usize::min(2, sb.len());
                sb.insert(idx, "win_sym".to_owned());
            }
        }

        let mono_candidates = [
            r"C:\Windows\Fonts\CascadiaMono.ttf",
            r"C:\Windows\Fonts\CascadiaCode.ttf",
            r"C:\Windows\Fonts\consola.ttf",
        ];
        for path in mono_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("win_mono".to_owned(), FontData::from_owned(bytes).into());
                if let Some(fam) = fonts.families.get_mut(&FontFamily::Monospace) {
                    fam.insert(0, "win_mono".to_owned());
                }
                break;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let proportional_candidates = [
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/HelveticaNeue.ttc",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ];
        for path in proportional_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("mac_ui".to_owned(), FontData::from_owned(bytes).into());
                if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                    fam.insert(0, "mac_ui".to_owned());
                }
                break;
            }
        }

        for path in [
            "/System/Library/Fonts/SFNSRounded.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("mac_ui_sb".to_owned(), FontData::from_owned(bytes).into());
                let mut sb_fam = fonts
                    .families
                    .get(&FontFamily::Proportional)
                    .cloned()
                    .unwrap_or_default();
                sb_fam.insert(0, "mac_ui_sb".to_owned());
                fonts
                    .families
                    .insert(FontFamily::Name("semibold".into()), sb_fam);
                break;
            }
        }

        if let Ok(bytes) = std::fs::read("/System/Library/Fonts/Apple Symbols.ttf") {
            fonts
                .font_data
                .insert("mac_sym".to_owned(), FontData::from_owned(bytes).into());
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                let idx = usize::min(1, fam.len());
                fam.insert(idx, "mac_sym".to_owned());
            }
            if let Some(sb) = fonts.families.get_mut(&FontFamily::Name("semibold".into())) {
                let idx = usize::min(2, sb.len());
                sb.insert(idx, "mac_sym".to_owned());
            }
        }

        let mono_candidates = [
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Monaco.ttf",
            "/System/Library/Fonts/SFNSMono.ttf",
        ];
        for path in mono_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("mac_mono".to_owned(), FontData::from_owned(bytes).into());
                if let Some(fam) = fonts.families.get_mut(&FontFamily::Monospace) {
                    fam.insert(0, "mac_mono".to_owned());
                }
                break;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let proportional_candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        ];
        for path in proportional_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("linux_ui".to_owned(), FontData::from_owned(bytes).into());
                if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                    fam.insert(0, "linux_ui".to_owned());
                }
                break;
            }
        }

        let sym_candidates = [
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ];
        for path in sym_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("linux_sym".to_owned(), FontData::from_owned(bytes).into());
                if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                    let idx = usize::min(1, fam.len());
                    fam.insert(idx, "linux_sym".to_owned());
                }
                break;
            }
        }

        let mono_candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
        ];
        for path in mono_candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("linux_mono".to_owned(), FontData::from_owned(bytes).into());
                if let Some(fam) = fonts.families.get_mut(&FontFamily::Monospace) {
                    fam.insert(0, "linux_mono".to_owned());
                }
                break;
            }
        }
    }

    if !fonts.families.contains_key(&FontFamily::Name("semibold".into())) {
        if let Some(prop) = fonts.families.get(&FontFamily::Proportional).cloned() {
            fonts
                .families
                .insert(FontFamily::Name("semibold".into()), prop);
        }
    }

    ctx.set_fonts(fonts);
}

pub fn apply_palette(ctx: &egui::Context, p: &CinderPalette) {
    let mut visuals = if p.is_light {
        Visuals::light()
    } else {
        Visuals::dark()
    };

    visuals.window_fill = p.bg_elev;
    visuals.panel_fill = p.bg;
    visuals.faint_bg_color = p.bg_card;
    visuals.extreme_bg_color = p.bg_sidebar;
    visuals.override_text_color = Some(p.text);
    visuals.selection.bg_fill =
        Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 44);
    visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);
    visuals.hyperlink_color = p.accent_hot;
    visuals.window_stroke = Stroke::new(1.0_f32, p.border_strong);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);

    visuals.widgets.noninteractive.bg_fill = p.bg_elev;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(7);

    visuals.widgets.inactive.bg_fill = p.bg_card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(7);

    visuals.widgets.hovered.bg_fill = p.bg_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(7);

    visuals.widgets.active.bg_fill = p.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.active.corner_radius = CornerRadius::same(7);

    visuals.widgets.open.bg_fill = p.bg_hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    visuals.widgets.open.corner_radius = CornerRadius::same(7);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(7.0, 6.0);
    style.spacing.button_padding = egui::vec2(9.0, 5.0);
    style.spacing.indent = 16.0;
    let mut scroll = egui::style::ScrollStyle::floating();
    scroll.bar_width = 8.0;
    scroll.floating_width = 4.0;
    scroll.handle_min_length = 28.0;
    scroll.dormant_handle_opacity = 0.22;
    scroll.active_handle_opacity = 0.65;
    scroll.interact_handle_opacity = 0.90;
    style.spacing.scroll = scroll;
    style.animation_time = 0.14;
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(p.base_font_size),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(p.base_font_size - 0.5),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional((p.base_font_size - 2.0).max(10.5)),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace((p.base_font_size - 1.0).max(11.0)),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(p.base_font_size + 4.5),
    );
    ctx.set_style(style);
    ctx.options_mut(|o| {
        o.line_scroll_speed = 56.0;
        o.reduce_texture_memory = true;
    });
}

/// Shared exact `40px` pane header ensuring continuous horizontal divider lines across all panes.
pub fn pane_header<R>(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let total_w = ui.available_width();
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(total_w, HEADER_HEIGHT), egui::Sense::hover());
    let inner_rect = rect.shrink2(egui::vec2(12.0, 0.0));
    let inner = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        add_contents,
    );
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0_f32, p.border),
    );
    inner.inner
}

pub fn input_frame(p: &CinderPalette, focused: bool) -> Frame {
    let border = if focused {
        Stroke::new(1.2_f32, p.accent)
    } else {
        Stroke::new(1.0_f32, p.border)
    };
    Frame::new()
        .fill(p.bg_sidebar)
        .stroke(border)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 6))
}

pub fn badge(ui: &mut egui::Ui, label: &str, fg: Color32, bg: Color32) {
    let spread = fg.r().max(fg.g()).max(fg.b()) - fg.r().min(fg.g()).min(fg.b());
    let is_light = !ui.style().visuals.dark_mode;
    let (pill_fill, pill_stroke) = if spread > 45 {
        (
            Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), if is_light { 26 } else { 22 }),
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(
                    fg.r(),
                    fg.g(),
                    fg.b(),
                    if is_light { 82 } else { 64 },
                ),
            ),
        )
    } else {
        (
            bg,
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), if is_light { 50 } else { 36 }),
            ),
        )
    };

    let rt = egui::RichText::new(label).size(11.0).strong().color(fg);
    let galley = egui::WidgetText::from(rt).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Small,
    );
    let pill_size = egui::vec2(
        (galley.size().x + 14.0).ceil(),
        (galley.size().y + 5.0).ceil().max(19.0),
    );
    let (rect, _) = ui.allocate_exact_size(pill_size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            pill_fill,
            pill_stroke,
            egui::StrokeKind::Inside,
        );
        let text_pos = egui::pos2(
            rect.left() + 7.0,
            rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(text_pos, galley, fg);
    }
}

pub fn ghost_button(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let widget_text: egui::WidgetText = text.into();
    let resp = ui
        .scope(|ui| {
            let vis = &mut ui.style_mut().visuals;
            vis.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            vis.widgets.inactive.bg_fill = Color32::TRANSPARENT;
            vis.widgets.inactive.bg_stroke = Stroke::NONE;
            vis.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.muted);
            vis.widgets.hovered.weak_bg_fill = p.bg_hover;
            vis.widgets.hovered.bg_fill = p.bg_hover;
            vis.widgets.hovered.bg_stroke = Stroke::NONE;
            vis.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
            vis.widgets.active.bg_fill = p.bg_card;
            vis.widgets.active.bg_stroke = Stroke::NONE;
            vis.widgets.open.bg_fill = p.bg_hover;
            vis.widgets.open.bg_stroke = Stroke::NONE;
            ui.add(
                egui::Button::new(widget_text)
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(7))
                    .wrap_mode(egui::TextWrapMode::Extend),
            )
        })
        .inner;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Writes `text` directly to the OS clipboard as well as egui's internal clipboard state.
pub fn copy_to_clipboard(ctx: &egui::Context, text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text.to_string());
    }
    ctx.copy_text(text.to_string());
}

/// Records a transient copy confirmation timestamp for a button ID.
pub fn mark_copied_feedback(ctx: &egui::Context, key: egui::Id) {
    let now = std::time::Instant::now();
    ctx.data_mut(|d| {
        d.insert_temp(key, now);
    });
    ctx.request_repaint_after(std::time::Duration::from_millis(1850));
}

/// Returns `true` within 1.8s after `mark_copied_feedback` was called for `key`.
pub fn is_copied_recently(ctx: &egui::Context, key: egui::Id) -> bool {
    ctx.data(|d| {
        d.get_temp::<std::time::Instant>(key)
            .map(|t| t.elapsed() < std::time::Duration::from_millis(1800))
            .unwrap_or(false)
    })
}

pub fn underline_tab(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    active: bool,
    label: &str,
) -> egui::Response {
    let fg = if active { p.text } else { p.muted };
    let text = egui::RichText::new(label).color(fg);
    let text = if active { text.strong() } else { text };
    let resp = ui.scope(|ui| {
        let vis = &mut ui.style_mut().visuals;
        vis.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        vis.widgets.inactive.bg_fill = Color32::TRANSPARENT;
        vis.widgets.inactive.bg_stroke = Stroke::NONE;
        vis.widgets.hovered.weak_bg_fill = p.bg_hover;
        vis.widgets.hovered.bg_fill = p.bg_hover;
        vis.widgets.hovered.bg_stroke = Stroke::NONE;
        vis.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
        ui.add(
            egui::Button::new(text)
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(6)),
        )
    })
    .inner;

    if active {
        ui.painter().hline(
            (resp.rect.left() + 4.0)..=(resp.rect.right() - 4.0),
            resp.rect.bottom() + 2.0,
            Stroke::new(2.0_f32, p.accent),
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn nav_row_with_icon(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    icon: Icon,
    active: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let widget_text: egui::WidgetText = text.into();
    let desired = egui::vec2(ui.available_width(), 30.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
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
                .rect_filled(rect, CornerRadius::same(7), fill);
        }
        if active {
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 6.0),
                egui::vec2(2.5, rect.height() - 12.0),
            );
            ui.painter()
                .rect_filled(bar, CornerRadius::same(2), p.accent);
        }
        let icon_col = if active {
            p.accent_hot
        } else if resp.hovered() {
            p.text
        } else {
            p.muted
        };
        let fg = if active || resp.hovered() {
            p.text
        } else {
            p.muted
        };

        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 17.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        );
        draw_icon(ui.painter(), icon_rect, icon, icon_col);

        let galley = widget_text.into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (rect.width() - 38.0).max(20.0),
            egui::TextStyle::Button,
        );
        let text_pos = egui::pos2(
            rect.left() + 30.0,
            rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(text_pos, galley, fg);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn segmented_control<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    _id_salt: &str,
    current: T,
    items: &[(T, &str)],
) -> Option<T> {
    let mut clicked = None;
    Frame::new()
        .fill(p.bg_sidebar)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for &(val, lbl) in items {
                    let active = val == current;
                    let fill = if active {
                        p.bg_card
                    } else {
                        Color32::TRANSPARENT
                    };
                    let stroke = if active {
                        Stroke::new(1.0_f32, p.border_strong)
                    } else {
                        Stroke::NONE
                    };
                    let fg = if active { p.text } else { p.muted };
                    let text = if active {
                        egui::RichText::new(lbl).small().strong().color(fg)
                    } else {
                        egui::RichText::new(lbl).small().color(fg)
                    };
                    let resp = ui.scope(|ui| {
                        let vis = &mut ui.style_mut().visuals;
                        vis.widgets.inactive.weak_bg_fill = fill;
                        vis.widgets.inactive.bg_fill = fill;
                        vis.widgets.inactive.bg_stroke = stroke;
                        vis.widgets.hovered.weak_bg_fill =
                            if active { p.bg_card } else { p.bg_hover };
                        vis.widgets.hovered.bg_fill = if active { p.bg_card } else { p.bg_hover };
                        vis.widgets.hovered.bg_stroke = stroke;
                        ui.add(
                            egui::Button::new(text)
                                .fill(fill)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(6)),
                        )
                    });
                    if resp.inner.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.inner.clicked() && !active {
                        clicked = Some(val);
                    }
                }
            });
        });
    clicked
}

pub fn setting_group<R>(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    title: &str,
    subtitle: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if !title.is_empty() {
        ui.label(
            egui::RichText::new(title)
                .size(p.base_font_size + 0.5)
                .strong()
                .color(p.text),
        );
    }
    if !subtitle.is_empty() {
        ui.label(egui::RichText::new(subtitle).small().color(p.muted));
    }
    if !title.is_empty() || !subtitle.is_empty() {
        ui.add_space(6.0);
    }
    Frame::new()
        .fill(p.bg_elev)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

pub fn setting_row_item(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    title: &str,
    subtitle: &str,
    with_divider: bool,
    right_ctrl: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right_ctrl(ui);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(title).strong().color(p.text));
                    if !subtitle.is_empty() {
                        ui.label(egui::RichText::new(subtitle).small().color(p.muted));
                    }
                });
            });
        });
    });
    ui.add_space(4.0);
    if with_divider {
        let y = ui.cursor().top();
        ui.painter()
            .hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, p.border));
        ui.add_space(4.0);
    }
}

pub fn empty_state_block(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    icon: Icon,
    title: &str,
    subtitle: &str,
    action_label: Option<&str>,
) -> bool {
    let mut action_clicked = false;
    p.card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 20.0, p.bg_card);
            ui.painter()
                .circle_stroke(r.center(), 20.0, Stroke::new(1.0_f32, p.border));
            draw_icon(
                ui.painter(),
                egui::Rect::from_center_size(r.center(), egui::vec2(18.0, 18.0)),
                icon,
                p.muted,
            );
            ui.add_space(8.0);
            ui.label(egui::RichText::new(title).strong().color(p.text));
            if !subtitle.is_empty() {
                ui.label(egui::RichText::new(subtitle).small().color(p.muted));
            }
            if let Some(lbl) = action_label {
                ui.add_space(10.0);
                if secondary_button(ui, p, lbl).clicked() {
                    action_clicked = true;
                }
            }
        });
        ui.add_space(16.0);
    });
    action_clicked
}

pub fn send_glow_button(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    generating: bool,
    enabled: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let c = rect.center();
        let icon_rect = egui::Rect::from_center_size(c, egui::vec2(14.0, 14.0));
        if generating {
            ui.painter().circle_filled(
                c,
                17.5,
                Color32::from_rgba_unmultiplied(p.red.r(), p.red.g(), p.red.b(), 26),
            );
            ui.painter().circle_filled(c, 13.5, p.red);
            draw_icon(ui.painter(), icon_rect, Icon::Stop, Color32::WHITE);
        } else if enabled {
            let outer_alpha = if resp.hovered() { 44 } else { 24 };
            let inner_alpha = if resp.hovered() { 68 } else { 40 };
            ui.painter().circle_filled(
                c,
                19.5,
                Color32::from_rgba_unmultiplied(
                    p.accent.r(),
                    p.accent.g(),
                    p.accent.b(),
                    outer_alpha,
                ),
            );
            ui.painter().circle_filled(
                c,
                15.5,
                Color32::from_rgba_unmultiplied(
                    p.accent_hot.r(),
                    p.accent_hot.g(),
                    p.accent_hot.b(),
                    inner_alpha,
                ),
            );
            ui.painter().circle_filled(
                c,
                13.5,
                if resp.hovered() { p.accent_hot } else { p.accent },
            );
            draw_icon(
                ui.painter(),
                icon_rect,
                Icon::ArrowUp,
                Color32::from_rgb(21, 14, 11),
            );
        } else {
            let idle_fill = if resp.hovered() { p.bg_hover } else { p.bg_card };
            ui.painter().circle_filled(c, 13.5, idle_fill);
            ui.painter()
                .circle_stroke(c, 13.5, Stroke::new(1.0_f32, p.border_strong));
            draw_icon(ui.painter(), icon_rect, Icon::ArrowUp, p.muted);
        }
    }
    if resp.hovered() && (enabled || generating) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn toggle_switch(ui: &mut egui::Ui, p: &CinderPalette, on: &mut bool) -> egui::Response {
    let desired = egui::vec2(34.0, 19.0);
    let (rect, mut resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let radius = rect.height() * 0.5;
        let how_on = ui.ctx().animate_bool_responsive(resp.id, *on);
        if *on {
            ui.painter().rect_stroke(
                rect.expand(1.5),
                CornerRadius::same(12),
                Stroke::new(
                    3.0_f32,
                    Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 28),
                ),
                egui::StrokeKind::Outside,
            );
        }
        let track_fill = Color32::from_rgb(
            egui::lerp((p.bg_hover.r() as f32)..=(p.accent.r() as f32), how_on) as u8,
            egui::lerp((p.bg_hover.g() as f32)..=(p.accent.g() as f32), how_on) as u8,
            egui::lerp((p.bg_hover.b() as f32)..=(p.accent.b() as f32), how_on) as u8,
        );
        let track_stroke = if *on {
            Stroke::NONE
        } else {
            Stroke::new(1.0_f32, p.border_strong)
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), track_fill);
        if track_stroke != Stroke::NONE {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(10),
                track_stroke,
                egui::StrokeKind::Inside,
            );
        }
        let knob_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let knob_col = if *on {
            Color32::from_rgb(20, 14, 12)
        } else {
            p.text
        };
        ui.painter()
            .circle_filled(egui::pos2(knob_x, rect.center().y), radius - 3.0, knob_col);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn primary_button(ui: &mut egui::Ui, p: &CinderPalette, label: &str) -> egui::Response {
    let text_color = Color32::from_rgb(21, 14, 11);
    let resp = ui
        .scope(|ui| {
            let vis = &mut ui.style_mut().visuals;
            vis.widgets.hovered.weak_bg_fill = p.accent_hot;
            vis.widgets.hovered.bg_fill = p.accent_hot;
            vis.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent_hot);
            ui.add(
                egui::Button::new(egui::RichText::new(label).strong().color(text_color))
                    .fill(p.accent)
                    .stroke(Stroke::new(1.0_f32, p.accent_hot))
                    .corner_radius(CornerRadius::same(7))
                    .wrap_mode(egui::TextWrapMode::Extend),
            )
        })
        .inner;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn secondary_button(ui: &mut egui::Ui, p: &CinderPalette, label: &str) -> egui::Response {
    let resp = ui
        .scope(|ui| {
            let vis = &mut ui.style_mut().visuals;
            vis.widgets.inactive.weak_bg_fill = p.bg_card;
            vis.widgets.inactive.bg_fill = p.bg_card;
            vis.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
            vis.widgets.hovered.weak_bg_fill = p.bg_hover;
            vis.widgets.hovered.bg_fill = p.bg_hover;
            vis.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
            ui.add(
                egui::Button::new(egui::RichText::new(label).color(p.text))
                    .fill(p.bg_card)
                    .stroke(Stroke::new(1.0_f32, p.border_strong))
                    .corner_radius(CornerRadius::same(7))
                    .wrap_mode(egui::TextWrapMode::Extend),
            )
        })
        .inner;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn danger_button(ui: &mut egui::Ui, p: &CinderPalette, label: &str) -> egui::Response {
    let resp = ui.add(
        egui::Button::new(egui::RichText::new(label).strong().color(Color32::WHITE))
            .fill(p.red)
            .stroke(Stroke::new(1.0_f32, p.red))
            .corner_radius(CornerRadius::same(7))
            .wrap_mode(egui::TextWrapMode::Extend),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| -> u8 {
        ((x as f32) * (1.0 - t) + (y as f32) * t).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

pub fn draw_cinder_flame(painter: &egui::Painter, center: egui::Pos2, r: f32, p: &CinderPalette) {
    let t = painter.ctx().input(|i| i.time) as f32;
    let pulse = (t * 2.6).sin() * 0.5 + 0.5;

    let outer_col = mix_color(p.accent, p.bg_sidebar, 0.22);
    let core_col = mix_color(p.accent, p.accent_hot, 0.44);
    let spark_col = mix_color(core_col, Color32::WHITE, 0.34);

    // Rich warm multi-step radial glow retained behind the orbiting arcs
    let glow_steps: &[(f32, u8)] = &[
        (1.92 + pulse * 0.22, 4),
        (1.58 + pulse * 0.16, 8),
        (1.30 + pulse * 0.10, 14),
        (1.06 + pulse * 0.06, 22),
        (0.84, 38),
    ];
    for &(scale, alpha) in glow_steps {
        painter.circle_filled(
            center,
            r * scale,
            Color32::from_rgba_unmultiplied(outer_col.r(), outer_col.g(), outer_col.b(), alpha),
        );
    }

    // Translucent dark-warm orb body + outer orbital track ring
    painter.circle_filled(
        center,
        r * 0.76,
        Color32::from_rgba_unmultiplied(outer_col.r(), outer_col.g(), outer_col.b(), 36),
    );
    painter.circle_stroke(
        center,
        r * 0.80,
        Stroke::new(
            (r * 0.04).clamp(0.8, 1.8),
            Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 72),
        ),
    );

    // Primary rotating energy arc mirroring the working indicator
    let arc_steps = 18usize;
    let outer_orbit_r = r * 0.80;
    let arc1_start = t * 2.15;
    let arc1_span = 2.15_f32;
    let mut pts1 = Vec::with_capacity(arc_steps + 1);
    for i in 0..=arc_steps {
        let a = arc1_start + (i as f32 / arc_steps as f32) * arc1_span;
        pts1.push(egui::pos2(
            center.x + a.cos() * outer_orbit_r,
            center.y + a.sin() * outer_orbit_r,
        ));
    }
    painter.add(egui::Shape::line(
        pts1,
        Stroke::new(
            (r * 0.095).clamp(1.2, 3.6),
            Color32::from_rgba_unmultiplied(core_col.r(), core_col.g(), core_col.b(), 220),
        ),
    ));
    let tip_angle = arc1_start + arc1_span;
    painter.circle_filled(
        egui::pos2(
            center.x + tip_angle.cos() * outer_orbit_r,
            center.y + tip_angle.sin() * outer_orbit_r,
        ),
        (r * 0.085).clamp(1.1, 3.2),
        Color32::from_rgba_unmultiplied(spark_col.r(), spark_col.g(), spark_col.b(), 235),
    );

    // Secondary counter-rotating inner arc
    let inner_orbit_r = r * 0.56;
    let arc2_start = -t * 2.95 + 1.35;
    let arc2_span = 1.70_f32;
    let mut pts2 = Vec::with_capacity(arc_steps + 1);
    for i in 0..=arc_steps {
        let a = arc2_start + (i as f32 / arc_steps as f32) * arc2_span;
        pts2.push(egui::pos2(
            center.x + a.cos() * inner_orbit_r,
            center.y + a.sin() * inner_orbit_r,
        ));
    }
    painter.add(egui::Shape::line(
        pts2,
        Stroke::new(
            (r * 0.065).clamp(1.0, 2.4),
            Color32::from_rgba_unmultiplied(spark_col.r(), spark_col.g(), spark_col.b(), 170),
        ),
    ));

    // Glowing ember orb core + hot heart + soft specular glint
    painter.circle_filled(
        center,
        r * (0.36 + pulse * 0.035),
        Color32::from_rgba_unmultiplied(core_col.r(), core_col.g(), core_col.b(), 215),
    );
    painter.circle_filled(
        center,
        r * (0.22 + pulse * 0.025),
        Color32::from_rgba_unmultiplied(spark_col.r(), spark_col.g(), spark_col.b(), 235),
    );
    if r >= 7.0 {
        painter.circle_filled(
            egui::pos2(center.x - r * 0.085, center.y - r * 0.085),
            (r * 0.075).max(0.9),
            Color32::from_rgba_unmultiplied(255, 245, 232, 165),
        );
    }
}

/// Request native VSync frame cadence while focused so active animations glide cleanly,
/// backing off to 4 Hz when the window is in the background.
pub fn request_smooth_anim_repaint(ctx: &egui::Context) {
    if ctx.input(|i| i.focused) {
        ctx.request_repaint();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// Draw the Cinder glowing orb animated smoothly at native VSync when focused.
pub fn draw_cinder_orb_animated(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    r: f32,
    p: &CinderPalette,
) {
    if ui.ctx().input(|i| i.focused) {
        ui.ctx().request_repaint();
    }
    draw_cinder_flame(ui.painter(), center, r, p);
}

/// Draw a compact animated orbiting-arc & pulsing-core working indicator.
pub fn draw_working_indicator(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    radius: f32,
    p: &CinderPalette,
) {
    request_smooth_anim_repaint(ui.ctx());
    let t = ui.input(|i| i.time) as f32;
    let pulse = (t * 3.4).sin() * 0.5 + 0.5;
    let painter = ui.painter();

    // Soft breathing halo
    let halo_alpha = (16.0 + pulse * 30.0) as u8;
    painter.circle_filled(
        center,
        radius * (0.98 + pulse * 0.42),
        Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), halo_alpha),
    );

    // Subtle track ring
    painter.circle_stroke(
        center,
        radius * 0.85,
        Stroke::new(
            1.1_f32,
            Color32::from_rgba_unmultiplied(p.border.r(), p.border.g(), p.border.b(), 130),
        ),
    );

    // Smooth rotating arc
    let arc_start = t * 4.2;
    let arc_span = 1.95_f32;
    let steps = 8usize;
    let mut pts = Vec::with_capacity(steps + 1);
    let orbit_r = radius * 0.85;
    for i in 0..=steps {
        let a = arc_start + (i as f32 / steps as f32) * arc_span;
        pts.push(egui::pos2(
            center.x + a.cos() * orbit_r,
            center.y + a.sin() * orbit_r,
        ));
    }
    painter.add(egui::Shape::line(
        pts,
        Stroke::new(1.5_f32, mix_color(p.accent, p.accent_hot, 0.5)),
    ));

    // Pulsing core dot
    painter.circle_filled(
        center,
        (radius * (0.30 + 0.08 * pulse)).max(1.4),
        p.accent_hot,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_all_five_themes_and_accents() {
        for th in [
            "cinder_forge",
            "singularity_obsidian",
            "carbon_midnight",
            "nordic_slate",
            "paper_light",
        ] {
            for acc in ["cinder", "amber", "blue", "emerald", "violet", "mono"] {
                let mut ui = UiCustomization::default();
                ui.theme = th.into();
                ui.accent_color = acc.into();
                let pal = CinderPalette::from_ui(&ui);
                assert_ne!(pal.bg, pal.text);
                if th == "paper_light" {
                    assert!(pal.is_light);
                } else {
                    assert!(!pal.is_light);
                }
            }
        }
    }

    #[test]
    fn badges_in_horizontal_wrapped_wrap_to_new_row_without_vertical_pillar() {
        let ctx = egui::Context::default();
        let pal = CinderPalette::from_ui(&UiCustomization::default());
        apply_palette(&ctx, &pal);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 500.0),
            )),
            ..Default::default()
        };
        let mut measured_h = 0.0_f32;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(360.0, 400.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let resp = ui.horizontal_wrapped(|ui| {
                            for name in [
                                "git_status",
                                "git_diff_unstaged",
                                "git_diff_staged",
                                "git_diff",
                                "git_commit",
                                "git_add",
                                "git_reset",
                                "git_log",
                                "git_create_branch",
                                "git_checkout",
                                "git_merge",
                                "git_show",
                                "git_branch",
                                "git_remote",
                                "open_nodes",
                            ] {
                                badge(ui, name, pal.green, pal.bg_elev);
                            }
                        });
                        measured_h = resp.response.rect.height();
                    },
                );
            });
        });
        assert!(
            measured_h > 30.0 && measured_h < 130.0,
            "badges stacked vertically or failed to wrap: height={measured_h}"
        );
    }

    #[test]
    fn buttons_in_horizontal_wrapped_never_collapse_into_vertical_pillar() {
        let ctx = egui::Context::default();
        let pal = CinderPalette::from_ui(&UiCustomization::default());
        apply_palette(&ctx, &pal);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 400.0),
            )),
            ..Default::default()
        };
        let mut btn_heights = Vec::new();
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(240.0, 300.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for lbl in [
                                "/plan · Plan First [Tab]",
                                "/grill-me · Grill Me (Interview)",
                                "/review · Code & Uni Review",
                                "/verify · Verify & Self-Fix",
                            ] {
                                let r = ghost_button(ui, &pal, egui::RichText::new(lbl).small());
                                btn_heights.push(r.rect.height());
                                let r2 = secondary_button(ui, &pal, lbl);
                                btn_heights.push(r2.rect.height());
                            }
                        });
                    },
                );
            });
        });
        assert!(!btn_heights.is_empty());
        for h in btn_heights {
            assert!(
                h <= 34.0,
                "button collapsed into vertical letter pillar: height={h}"
            );
        }
    }
}
