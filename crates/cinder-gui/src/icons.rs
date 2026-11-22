use crate::theme::CinderPalette;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Stroke, Ui};
use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Sidebar,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Search,
    Filter,
    Plus,
    Close,
    MoreHorizontal,
    Folder,
    FolderOpen,
    File,
    FileCode,
    FileImage,
    FileDoc,
    Flame,
    Blocks,
    Sliders,
    Key,
    Cpu,
    Shield,
    Palette,
    Gear,
    PanelRight,
    FilesTree,
    Terminal,
    Refresh,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Stop,
    NewFile,
    NewFolder,
    Check,
    Clock,
    Briefcase,
    Download,
    Pause,
    Play,
    WindowMinimize,
    WindowMaximize,
}

pub fn draw_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()).max(8.0);
    let c = rect.center();
    let p = |dx: f32, dy: f32| Pos2::new(c.x + dx * s, c.y + dy * s);
    let stroke = Stroke::new((s * 0.09).clamp(1.25, 1.8), color);

    match icon {
        Icon::Sidebar => {
            let box_r = Rect::from_center_size(c, egui::vec2(s * 0.78, s * 0.66));
            painter.rect_stroke(box_r, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            let div_x = box_r.left() + box_r.width() * 0.33;
            painter.line_segment(
                [Pos2::new(div_x, box_r.top()), Pos2::new(div_x, box_r.bottom())],
                stroke,
            );
        }
        Icon::PanelRight => {
            let box_r = Rect::from_center_size(c, egui::vec2(s * 0.78, s * 0.66));
            painter.rect_stroke(box_r, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            let div_x = box_r.right() - box_r.width() * 0.33;
            painter.line_segment(
                [Pos2::new(div_x, box_r.top()), Pos2::new(div_x, box_r.bottom())],
                stroke,
            );
        }
        Icon::ChevronLeft => {
            painter.line_segment([p(0.14, -0.26), p(-0.14, 0.0)], stroke);
            painter.line_segment([p(-0.14, 0.0), p(0.14, 0.26)], stroke);
        }
        Icon::ChevronRight => {
            painter.line_segment([p(-0.14, -0.26), p(0.14, 0.0)], stroke);
            painter.line_segment([p(0.14, 0.0), p(-0.14, 0.26)], stroke);
        }
        Icon::ChevronDown => {
            painter.line_segment([p(-0.26, -0.12), p(0.0, 0.14)], stroke);
            painter.line_segment([p(0.0, 0.14), p(0.26, -0.12)], stroke);
        }
        Icon::Search => {
            let center = p(-0.08, -0.08);
            painter.circle_stroke(center, s * 0.25, stroke);
            painter.line_segment([p(0.10, 0.10), p(0.34, 0.34)], stroke);
        }
        Icon::Filter => {
            painter.line_segment([p(-0.34, -0.22), p(0.34, -0.22)], stroke);
            painter.line_segment([p(-0.22, 0.0), p(0.22, 0.0)], stroke);
            painter.line_segment([p(-0.10, 0.22), p(0.10, 0.22)], stroke);
        }
        Icon::Plus => {
            painter.line_segment([p(-0.28, 0.0), p(0.28, 0.0)], stroke);
            painter.line_segment([p(0.0, -0.28), p(0.0, 0.28)], stroke);
        }
        Icon::Close => {
            painter.line_segment([p(-0.24, -0.24), p(0.24, 0.24)], stroke);
            painter.line_segment([p(0.24, -0.24), p(-0.24, 0.24)], stroke);
        }
        Icon::MoreHorizontal => {
            let r = (s * 0.065).max(1.1);
            painter.circle_filled(p(-0.25, 0.0), r, color);
            painter.circle_filled(p(0.0, 0.0), r, color);
            painter.circle_filled(p(0.25, 0.0), r, color);
        }
        Icon::Folder => {
            let pts = vec![
                p(-0.38, -0.28),
                p(-0.08, -0.28),
                p(-0.01, -0.18),
                p(0.38, -0.18),
                p(0.38, 0.28),
                p(-0.38, 0.28),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        Icon::FolderOpen => {
            let back = vec![
                p(-0.38, -0.28),
                p(-0.08, -0.28),
                p(0.0, -0.18),
                p(0.32, -0.18),
                p(0.32, -0.04),
            ];
            painter.add(egui::Shape::line(back, stroke));
            let front = vec![
                p(-0.38, -0.28),
                p(-0.38, 0.28),
                p(0.30, 0.28),
                p(0.42, -0.04),
                p(-0.24, -0.04),
                p(-0.38, 0.28),
            ];
            painter.add(egui::Shape::line(front, stroke));
        }
        Icon::File | Icon::FileCode | Icon::FileImage | Icon::FileDoc => {
            draw_doc_outline(painter, c, s, stroke);
            match icon {
                Icon::FileCode => {
                    painter.line_segment([p(-0.07, 0.02), p(-0.16, 0.11)], stroke);
                    painter.line_segment([p(-0.16, 0.11), p(-0.07, 0.20)], stroke);
                    painter.line_segment([p(0.07, 0.02), p(0.16, 0.11)], stroke);
                    painter.line_segment([p(0.16, 0.11), p(0.07, 0.20)], stroke);
                }
                Icon::FileImage => {
                    painter.circle_filled(p(-0.08, 0.04), s * 0.05, color);
                    painter.add(egui::Shape::line(
                        vec![
                            p(-0.18, 0.24),
                            p(-0.04, 0.12),
                            p(0.06, 0.20),
                            p(0.16, 0.10),
                            p(0.22, 0.24),
                        ],
                        stroke,
                    ));
                }
                Icon::FileDoc => {
                    painter.line_segment([p(-0.15, 0.06), p(0.15, 0.06)], stroke);
                    painter.line_segment([p(-0.15, 0.20), p(0.09, 0.20)], stroke);
                }
                _ => {}
            }
        }
        Icon::Flame => {
            let outer = vec![
                p(0.02, -0.38),
                p(0.28, -0.06),
                p(0.28, 0.20),
                p(0.10, 0.36),
                p(-0.12, 0.36),
                p(-0.28, 0.18),
                p(-0.26, -0.08),
                p(-0.08, -0.16),
                p(0.02, -0.38),
            ];
            painter.add(egui::Shape::line(outer, stroke));
            let inner = vec![
                p(0.0, 0.04),
                p(0.11, 0.18),
                p(0.0, 0.29),
                p(-0.11, 0.18),
                p(0.0, 0.04),
            ];
            painter.add(egui::Shape::line(inner, stroke));
        }
        Icon::Blocks => {
            let sq = s * 0.28;
            let r1 = Rect::from_center_size(p(-0.18, 0.18), egui::vec2(sq, sq));
            let r2 = Rect::from_center_size(p(-0.18, -0.18), egui::vec2(sq, sq));
            let r3 = Rect::from_center_size(p(0.18, 0.18), egui::vec2(sq, sq));
            let r4 = Rect::from_center_size(p(0.20, -0.20), egui::vec2(sq, sq));
            for r in [r1, r2, r3, r4] {
                painter.rect_stroke(r, CornerRadius::same(1), stroke, egui::StrokeKind::Middle);
            }
        }
        Icon::Sliders => {
            painter.line_segment([p(-0.34, -0.18), p(0.04, -0.18)], stroke);
            painter.line_segment([p(0.22, -0.18), p(0.34, -0.18)], stroke);
            painter.circle_stroke(p(0.13, -0.18), s * 0.09, stroke);

            painter.line_segment([p(-0.34, 0.18), p(-0.22, 0.18)], stroke);
            painter.line_segment([p(-0.04, 0.18), p(0.34, 0.18)], stroke);
            painter.circle_stroke(p(-0.13, 0.18), s * 0.09, stroke);
        }
        Icon::Key => {
            painter.circle_stroke(p(-0.18, 0.0), s * 0.16, stroke);
            painter.line_segment([p(-0.02, 0.0), p(0.35, 0.0)], stroke);
            painter.line_segment([p(0.20, 0.0), p(0.20, 0.17)], stroke);
            painter.line_segment([p(0.33, 0.0), p(0.33, 0.15)], stroke);
        }
        Icon::Cpu => {
            let body = Rect::from_center_size(c, egui::vec2(s * 0.50, s * 0.50));
            painter.rect_stroke(body, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            let die = Rect::from_center_size(c, egui::vec2(s * 0.20, s * 0.20));
            painter.rect_stroke(die, CornerRadius::same(1), stroke, egui::StrokeKind::Middle);
            for d in [-0.12, 0.12] {
                painter.line_segment([p(d, -0.25), p(d, -0.37)], stroke);
                painter.line_segment([p(d, 0.25), p(d, 0.37)], stroke);
                painter.line_segment([p(-0.25, d), p(-0.37, d)], stroke);
                painter.line_segment([p(0.25, d), p(0.37, d)], stroke);
            }
        }
        Icon::Shield => {
            let pts = vec![
                p(0.0, -0.36),
                p(0.31, -0.24),
                p(0.31, 0.04),
                p(0.0, 0.36),
                p(-0.31, 0.04),
                p(-0.31, -0.24),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        Icon::Palette => {
            let pts = vec![
                p(0.0, -0.34),
                p(0.28, -0.24),
                p(0.34, 0.02),
                p(0.14, 0.12),
                p(0.22, 0.26),
                p(0.04, 0.35),
                p(-0.24, 0.24),
                p(-0.34, -0.02),
                p(-0.22, -0.26),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
            let dr = (s * 0.055).max(1.0);
            painter.circle_filled(p(-0.14, -0.10), dr, color);
            painter.circle_filled(p(0.06, -0.16), dr, color);
            painter.circle_filled(p(-0.12, 0.12), dr, color);
        }
        Icon::Gear => {
            painter.circle_stroke(c, s * 0.15, stroke);
            painter.circle_stroke(c, s * 0.28, stroke);
            for i in 0..8 {
                let a = (i as f32) * (TAU / 8.0);
                let (sin_a, cos_a) = a.sin_cos();
                let a_pos = Pos2::new(c.x + cos_a * s * 0.28, c.y + sin_a * s * 0.28);
                let b_pos = Pos2::new(c.x + cos_a * s * 0.38, c.y + sin_a * s * 0.38);
                painter.line_segment([a_pos, b_pos], stroke);
            }
        }
        Icon::FilesTree => {
            let front = Rect::from_center_size(p(-0.07, 0.07), egui::vec2(s * 0.50, s * 0.58));
            painter.rect_stroke(front, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            painter.add(egui::Shape::line(
                vec![
                    p(-0.08, -0.24),
                    p(-0.08, -0.34),
                    p(0.32, -0.34),
                    p(0.32, 0.24),
                    p(0.20, 0.24),
                ],
                stroke,
            ));
        }
        Icon::Terminal => {
            let box_r = Rect::from_center_size(c, egui::vec2(s * 0.78, s * 0.64));
            painter.rect_stroke(box_r, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            painter.line_segment([p(-0.22, -0.11), p(-0.08, 0.01)], stroke);
            painter.line_segment([p(-0.08, 0.01), p(-0.22, 0.13)], stroke);
            painter.line_segment([p(0.02, 0.13), p(0.22, 0.13)], stroke);
        }
        Icon::Refresh => {
            let n = 14usize;
            let rad = s * 0.30;
            let mut arc_pts = Vec::with_capacity(n + 1);
            let start = -0.35_f32;
            let span = TAU * 0.76;
            for i in 0..=n {
                let a = start + (i as f32 / n as f32) * span;
                arc_pts.push(Pos2::new(c.x + a.cos() * rad, c.y + a.sin() * rad));
            }
            if let Some(&tip) = arc_pts.first() {
                painter.line_segment(
                    [tip, Pos2::new(tip.x - s * 0.16, tip.y + s * 0.02)],
                    stroke,
                );
                painter.line_segment(
                    [tip, Pos2::new(tip.x - s * 0.04, tip.y - s * 0.16)],
                    stroke,
                );
            }
            painter.add(egui::Shape::line(arc_pts, stroke));
        }
        Icon::ArrowLeft => {
            painter.line_segment([p(0.30, 0.0), p(-0.28, 0.0)], stroke);
            painter.line_segment([p(-0.04, -0.22), p(-0.28, 0.0)], stroke);
            painter.line_segment([p(-0.28, 0.0), p(-0.04, 0.22)], stroke);
        }
        Icon::ArrowRight => {
            painter.line_segment([p(-0.30, 0.0), p(0.28, 0.0)], stroke);
            painter.line_segment([p(0.04, -0.22), p(0.28, 0.0)], stroke);
            painter.line_segment([p(0.28, 0.0), p(0.04, 0.22)], stroke);
        }
        Icon::ArrowUp => {
            painter.line_segment([p(0.0, 0.30), p(0.0, -0.28)], stroke);
            painter.line_segment([p(-0.22, -0.05), p(0.0, -0.28)], stroke);
            painter.line_segment([p(0.0, -0.28), p(0.22, -0.05)], stroke);
        }
        Icon::Stop => {
            let sq = Rect::from_center_size(c, egui::vec2(s * 0.48, s * 0.48));
            painter.rect_filled(sq, CornerRadius::same(2), color);
        }
        Icon::NewFile => {
            let pts = vec![
                p(-0.28, -0.34),
                p(0.04, -0.34),
                p(0.22, -0.16),
                p(0.22, 0.34),
                p(-0.28, 0.34),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
            painter.line_segment([p(0.18, 0.10), p(0.38, 0.10)], stroke);
            painter.line_segment([p(0.28, 0.0), p(0.28, 0.20)], stroke);
        }
        Icon::NewFolder => {
            let pts = vec![
                p(-0.38, -0.28),
                p(-0.08, -0.28),
                p(-0.01, -0.18),
                p(0.38, -0.18),
                p(0.38, 0.28),
                p(-0.38, 0.28),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
            painter.line_segment([p(-0.12, 0.06), p(0.12, 0.06)], stroke);
            painter.line_segment([p(0.0, -0.06), p(0.0, 0.18)], stroke);
        }
        Icon::Check => {
            painter.line_segment([p(-0.28, 0.02), p(-0.06, 0.24)], stroke);
            painter.line_segment([p(-0.06, 0.24), p(0.30, -0.22)], stroke);
        }
        Icon::Clock => {
            painter.circle_stroke(c, s * 0.34, stroke);
            painter.line_segment([c, p(0.0, -0.20)], stroke);
            painter.line_segment([c, p(0.15, 0.08)], stroke);
        }
        Icon::Briefcase => {
            let body = Rect::from_center_size(
                Pos2::new(c.x, c.y + s * 0.06),
                egui::vec2(s * 0.72, s * 0.48),
            );
            painter.rect_stroke(body, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            let handle = vec![
                p(-0.14, -0.18),
                p(-0.14, -0.30),
                p(0.14, -0.30),
                p(0.14, -0.18),
            ];
            painter.add(egui::Shape::line(handle, stroke));
            painter.line_segment([p(-0.36, 0.03), p(0.36, 0.03)], stroke);
        }
        Icon::Download => {
            painter.line_segment([p(0.0, -0.34), p(0.0, 0.16)], stroke);
            let arrow = vec![p(-0.20, -0.02), p(0.0, 0.18), p(0.20, -0.02)];
            painter.add(egui::Shape::line(arrow, stroke));
            let tray = vec![
                p(-0.34, 0.18),
                p(-0.34, 0.34),
                p(0.34, 0.34),
                p(0.34, 0.18),
            ];
            painter.add(egui::Shape::line(tray, stroke));
        }
        Icon::Pause => {
            painter.line_segment([p(-0.14, -0.26), p(-0.14, 0.26)], stroke);
            painter.line_segment([p(0.14, -0.26), p(0.14, 0.26)], stroke);
        }
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![p(-0.20, -0.28), p(0.28, 0.0), p(-0.20, 0.28)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::WindowMinimize => {
            painter.line_segment([p(-0.28, 0.04), p(0.28, 0.04)], stroke);
        }
        Icon::WindowMaximize => {
            let box_rect = Rect::from_center_size(c, egui::vec2(s * 0.54, s * 0.54));
            painter.rect_stroke(
                box_rect,
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Middle,
            );
        }
    }
}

fn draw_doc_outline(painter: &Painter, c: Pos2, s: f32, stroke: Stroke) {
    let p = |dx: f32, dy: f32| Pos2::new(c.x + dx * s, c.y + dy * s);
    let outline = vec![
        p(-0.26, -0.36),
        p(0.08, -0.36),
        p(0.26, -0.18),
        p(0.26, 0.36),
        p(-0.26, 0.36),
    ];
    painter.add(egui::Shape::closed_line(outline, stroke));
    painter.line_segment([p(0.08, -0.36), p(0.08, -0.18)], stroke);
    painter.line_segment([p(0.08, -0.18), p(0.26, -0.18)], stroke);
}

pub fn icon_button(
    ui: &mut Ui,
    p: &CinderPalette,
    icon: Icon,
    active: bool,
    tooltip: &str,
) -> Response {
    icon_button_enabled(ui, p, icon, active, true, tooltip)
}

pub fn icon_button_enabled(
    ui: &mut Ui,
    p: &CinderPalette,
    icon: Icon,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), sense);
    if ui.is_rect_visible(rect) {
        let fill = if !enabled {
            Color32::TRANSPARENT
        } else if active {
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
        let col = if !enabled {
            p.muted.gamma_multiply(0.45)
        } else if active {
            p.accent_hot
        } else if resp.hovered() {
            p.text
        } else {
            p.muted
        };
        let icon_rect = Rect::from_center_size(rect.center(), egui::vec2(14.5, 14.5));
        draw_icon(ui.painter(), icon_rect, icon, col);
    }
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}
