use crate::theme::{
    copy_to_clipboard, ghost_button, is_copied_recently, mark_copied_feedback, CinderPalette,
};
use egui::{Color32, CornerRadius, FontFamily, Frame, Margin, Stroke};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdBlock {
    Heading { level: u8, text: String },
    Paragraph(String),
    Bullet(String),
    OrderedItem { number: String, text: String },
    Task { checked: bool, text: String },
    Quote(String),
    Alert { kind: String, text: String },
    Code { lang: String, code: String },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    Rule,
}

pub fn clean_inline_markdown_heading(raw: &str) -> String {
    raw.replace("**", "").replace("__", "").trim().to_string()
}

pub fn normalize_inline_latex(input: &str) -> String {
    let mut s = input.to_string();
    for (pat, rep) in [
        ("$\\rightarrow$", "→"),
        ("\\rightarrow", "→"),
        ("$\\leftarrow$", "←"),
        ("\\leftarrow", "←"),
        ("$\\Rightarrow$", "⇒"),
        ("\\Rightarrow", "⇒"),
        ("$\\leftrightarrow$", "↔"),
        ("\\leftrightarrow", "↔"),
        ("$\\leq$", "≤"),
        ("\\leq", "≤"),
        ("$\\le$", "≤"),
        ("\\le", "≤"),
        ("$\\geq$", "≥"),
        ("\\geq", "≥"),
        ("$\\ge$", "≥"),
        ("\\ge", "≥"),
        ("$\\neq$", "≠"),
        ("\\neq", "≠"),
        ("$\\times$", "×"),
        ("\\times", "×"),
        ("$\\approx$", "≈"),
        ("\\approx", "≈"),
        ("$\\pm$", "±"),
        ("\\pm", "±"),
    ] {
        if s.contains(pat) {
            s = s.replace(pat, rep);
        }
    }
    s
}

fn parse_ordered_list_line(trimmed: &str) -> Option<(String, String)> {
    let dot = trimmed.find(". ")?;
    if dot == 0 || dot > 3 {
        return None;
    }
    let num = &trimmed[..dot];
    if !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = trimmed[dot + 2..].trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((num.to_string(), rest.to_string()))
}

pub fn parse_markdown_blocks(input: &str) -> Vec<MdBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block
        if let Some(fence_rest) = trimmed.strip_prefix("```") {
            let lang = fence_rest.trim().to_string();
            let mut code_lines = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            blocks.push(MdBlock::Code {
                lang,
                code: code_lines.join("\n"),
            });
            continue;
        }

        // Horizontal rule
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            blocks.push(MdBlock::Rule);
            i += 1;
            continue;
        }

        // Headings
        if let Some(rest) = trimmed
            .strip_prefix("##### ")
            .or_else(|| trimmed.strip_prefix("#### "))
        {
            blocks.push(MdBlock::Heading {
                level: 4,
                text: rest.trim().to_string(),
            });
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            blocks.push(MdBlock::Heading {
                level: 3,
                text: rest.trim().to_string(),
            });
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            blocks.push(MdBlock::Heading {
                level: 2,
                text: rest.trim().to_string(),
            });
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            blocks.push(MdBlock::Heading {
                level: 1,
                text: rest.trim().to_string(),
            });
            i += 1;
            continue;
        }

        // Markdown table (header + |---|)
        if trimmed.starts_with('|')
            && i + 1 < lines.len()
            && lines[i + 1].trim().starts_with('|')
            && lines[i + 1].contains('-')
        {
            let split_row = |line: &str| -> Vec<String> {
                line.trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|c| c.trim().to_string())
                    .collect()
            };
            let headers = split_row(lines[i]);
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('|') {
                rows.push(split_row(lines[i]));
                i += 1;
            }
            blocks.push(MdBlock::Table { headers, rows });
            continue;
        }

        // Blockquotes & GitHub Alerts
        if trimmed.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let inner = lines[i]
                    .trim_start()
                    .strip_prefix('>')
                    .unwrap_or("")
                    .trim_start();
                quote_lines.push(inner);
                i += 1;
            }
            if let Some(first) = quote_lines.first().copied() {
                if let Some(rest_tag) = first.strip_prefix("[!") {
                    if let Some(close) = rest_tag.find(']') {
                        let kind = rest_tag[..close].trim().to_ascii_lowercase();
                        let first_body = rest_tag[close + 1..].trim();
                        let mut body_parts = Vec::new();
                        if !first_body.is_empty() {
                            body_parts.push(first_body);
                        }
                        body_parts.extend_from_slice(&quote_lines[1..]);
                        blocks.push(MdBlock::Alert {
                            kind,
                            text: body_parts.join("\n"),
                        });
                        continue;
                    }
                }
            }
            blocks.push(MdBlock::Quote(quote_lines.join("\n")));
            continue;
        }

        // Task checkboxes
        if let Some(rest) = trimmed
            .strip_prefix("- [ ] ")
            .or_else(|| trimmed.strip_prefix("* [ ] "))
        {
            blocks.push(MdBlock::Task {
                checked: false,
                text: rest.to_string(),
            });
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
            .or_else(|| trimmed.strip_prefix("* [x] "))
        {
            blocks.push(MdBlock::Task {
                checked: true,
                text: rest.to_string(),
            });
            i += 1;
            continue;
        }

        // Bullets
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("• "))
        {
            blocks.push(MdBlock::Bullet(rest.trim_start().to_string()));
            i += 1;
            continue;
        }

        // Ordered list (e.g. `1.  Item`)
        if let Some((number, rest)) = parse_ordered_list_line(trimmed) {
            blocks.push(MdBlock::OrderedItem { number, text: rest });
            i += 1;
            continue;
        }

        // Paragraph (accumulate consecutive plain lines)
        let mut para = vec![lines[i]];
        i += 1;
        while i < lines.len() {
            let nxt = lines[i].trim();
            if nxt.is_empty()
                || nxt.starts_with("```")
                || nxt.starts_with("# ")
                || nxt.starts_with("## ")
                || nxt.starts_with("### ")
                || nxt.starts_with("#### ")
                || nxt.starts_with("##### ")
                || nxt.starts_with("- ")
                || nxt.starts_with("* ")
                || nxt.starts_with("• ")
                || parse_ordered_list_line(nxt).is_some()
                || nxt.starts_with('>')
                || nxt.starts_with('|')
                || nxt == "---"
            {
                break;
            }
            para.push(lines[i]);
            i += 1;
        }
        blocks.push(MdBlock::Paragraph(para.join("\n")));
    }

    blocks
}

fn inline_file_badge(token: &str, p: &CinderPalette) -> Option<(&'static str, Color32)> {
    let t = token.trim();
    if t.is_empty()
        || t.len() > 120
        || t.contains("://")
        || t.chars().any(char::is_whitespace)
    {
        return None;
    }
    let clean = t.split('#').next().unwrap_or(t);
    let ext = clean.rsplit('.').next()?.to_ascii_lowercase();
    if clean.len() == ext.len() {
        return None;
    }
    match ext.as_str() {
        "rs" => Some(("R", p.accent_hot)),
        "py" => Some(("Py", p.accent)),
        "ts" | "tsx" => Some(("TS", p.accent)),
        "js" | "jsx" => Some(("JS", p.amber)),
        "md" | "markdown" => Some(("MD", p.green)),
        "toml" | "json" | "yaml" | "yml" => Some(("{}", p.purple)),
        "pdf" => Some(("PDF", p.red)),
        "html" | "css" | "svg" => Some(("UI", p.accent)),
        "sh" | "ps1" | "bat" => Some(("SH", p.green)),
        _ => None,
    }
}

fn append_inline_file_badge(
    job: &mut egui::text::LayoutJob,
    token: &str,
    p: &CinderPalette,
    normal_font: &egui::FontId,
    base_color: Color32,
) {
    if let Some((badge, col)) = inline_file_badge(token, p) {
        let badge_bg = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 38);
        job.append(
            &format!(" {badge} "),
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::monospace((p.base_font_size - 2.0).max(10.0)),
                color: col,
                background: badge_bg,
                ..Default::default()
            },
        );
        job.append(
            " ",
            0.0,
            egui::TextFormat {
                font_id: normal_font.clone(),
                color: base_color,
                ..Default::default()
            },
        );
    }
}

pub(crate) fn normalize_clickable_url(raw: &str) -> Option<String> {
    let mut t = raw.trim();
    if t.starts_with('<') && t.ends_with('>') && t.len() >= 2 {
        t = t[1..t.len() - 1].trim();
    }
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let rest = t.split_once("://").map(|(_, r)| r).unwrap_or("");
        if !rest.is_empty() && !rest.chars().any(char::is_whitespace) {
            return Some(t.to_string());
        }
        return None;
    }
    if lower.starts_with("mailto:") {
        let rest = &t["mailto:".len()..];
        if rest.contains('@') && !rest.chars().any(char::is_whitespace) {
            return Some(t.to_string());
        }
        return None;
    }
    if lower.starts_with("www.") && !t.chars().any(char::is_whitespace) && t[4..].contains('.') {
        return Some(format!("https://{t}"));
    }
    None
}

pub(crate) fn parse_bare_url_end(chars: &[char], start: usize) -> Option<usize> {
    let slice: String = chars[start..chars.len().min(start + 8)].iter().collect();
    let min_len = if slice.starts_with("https://") {
        8
    } else if slice.starts_with("http://") {
        7
    } else {
        return None;
    };
    let mut end = start + min_len;
    while end < chars.len() {
        let ch = chars[end];
        if ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'' | ']') {
            break;
        }
        end += 1;
    }
    while end > start + min_len {
        let last = chars[end - 1];
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?') {
            end -= 1;
        } else if last == ')' {
            let opens = chars[start..end].iter().filter(|&&c| c == '(').count();
            let closes = chars[start..end].iter().filter(|&&c| c == ')').count();
            if closes > opens {
                end -= 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    (end > start + min_len).then_some(end)
}

fn hit_test_galley_link<'a>(
    galley: &egui::Galley,
    local_pos: egui::Vec2,
    links: &'a [(std::ops::Range<usize>, String)],
) -> Option<&'a str> {
    let local_pt = local_pos.to_pos2();
    let in_row = galley
        .rows
        .iter()
        .any(|r| r.rect.expand2(egui::vec2(2.0, 1.0)).contains(local_pt));
    if !in_row {
        return None;
    }
    let cursor = galley.cursor_from_pos(local_pos);
    let cidx = cursor.ccursor.index;
    for (range, url) in links {
        if (cidx >= range.start && cidx < range.end)
            || (cidx == range.end
                && range.end > range.start
                && local_pos.x <= galley.pos_from_cursor(&cursor).min.x + 1.5)
        {
            return Some(url.as_str());
        }
    }
    None
}

fn append_bold_segment_to_job(
    job: &mut egui::text::LayoutJob,
    slice: &str,
    p: &CinderPalette,
    bold_font: &egui::FontId,
    mono_font: &egui::FontId,
    base_color: Color32,
) {
    let chars: Vec<char> = slice.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end_rel) = chars[i + 1..].iter().position(|&c| c == '`') {
                let code_slice: String = chars[i + 1..i + 1 + end_rel].iter().collect();
                append_inline_file_badge(job, &code_slice, p, bold_font, base_color);
                job.append(
                    &code_slice,
                    0.0,
                    egui::TextFormat {
                        font_id: mono_font.clone(),
                        color: p.accent_hot,
                        background: p.bg_card,
                        ..Default::default()
                    },
                );
                i += end_rel + 2;
                continue;
            }
        }
        let start = i;
        i += 1;
        while i < chars.len() && chars[i] != '`' {
            i += 1;
        }
        let plain: String = chars[start..i].iter().collect();
        job.append(
            &plain,
            0.0,
            egui::TextFormat {
                font_id: bold_font.clone(),
                color: p.text,
                ..Default::default()
            },
        );
    }
}

pub(crate) fn build_inline_layout_job(
    p: &CinderPalette,
    text: &str,
    base_color: Color32,
    max_width: f32,
    default_bold: bool,
) -> (egui::text::LayoutJob, Vec<(std::ops::Range<usize>, String)>) {
    let mut job = egui::text::LayoutJob::default();
    let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let normal_font = egui::FontId::proportional(p.base_font_size);
    let bold_font = egui::FontId::new(
        p.base_font_size + 0.2,
        FontFamily::Name("semibold".into()),
    );
    let body_font = if default_bold {
        bold_font.clone()
    } else {
        normal_font.clone()
    };
    let mono_font = egui::FontId::monospace((p.base_font_size - 1.0).max(11.0));

    let normalized = normalize_inline_latex(text);
    let chars: Vec<char> = normalized.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        // Inline `code`
        if chars[i] == '`' {
            if let Some(end_rel) = chars[i + 1..].iter().position(|&c| c == '`') {
                let slice: String = chars[i + 1..i + 1 + end_rel].iter().collect();
                append_inline_file_badge(&mut job, &slice, p, &body_font, base_color);
                job.append(
                    &slice,
                    0.0,
                    egui::TextFormat {
                        font_id: mono_font.clone(),
                        color: p.accent_hot,
                        background: p.bg_card,
                        ..Default::default()
                    },
                );
                i += end_rel + 2;
                continue;
            }
        }
        // Bold **text** (including nested `code` spans)
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            let mut end_pos = None;
            let mut j = i + 2;
            while j + 1 < chars.len() {
                if chars[j] == '*' && chars[j + 1] == '*' {
                    end_pos = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = end_pos {
                let slice: String = chars[i + 2..end].iter().collect();
                append_bold_segment_to_job(
                    &mut job,
                    &slice,
                    p,
                    &bold_font,
                    &mono_font,
                    base_color,
                );
                i = end + 2;
                continue;
            }
        }
        // Italic *text*
        if chars[i] == '*'
            && i + 1 < chars.len()
            && !chars[i + 1].is_whitespace()
            && chars[i + 1] != '*'
        {
            let mut end_pos = None;
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\n' {
                    break;
                }
                if chars[j] == '*'
                    && (j + 1 >= chars.len() || chars[j + 1] != '*')
                    && !chars[j - 1].is_whitespace()
                {
                    end_pos = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = end_pos {
                let slice: String = chars[i + 1..end].iter().collect();
                job.append(
                    &slice,
                    0.0,
                    egui::TextFormat {
                        font_id: body_font.clone(),
                        color: base_color,
                        italics: true,
                        ..Default::default()
                    },
                );
                i = end + 1;
                continue;
            }
        }

        // Markdown link [label](url)
        if chars[i] == '[' {
            let mut close_br = None;
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\n' {
                    break;
                }
                if chars[j] == ']' {
                    close_br = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(cb) = close_br {
                if chars.get(cb + 1) == Some(&'(') {
                    let mut close_par = None;
                    let mut k = cb + 2;
                    while k < chars.len() {
                        if chars[k] == '\n' {
                            break;
                        }
                        if chars[k] == ')' {
                            close_par = Some(k);
                            break;
                        }
                        k += 1;
                    }
                    if let Some(cp) = close_par {
                        let label: String = chars[i + 1..cb].iter().collect();
                        let url_slice: String = chars[cb + 2..cp].iter().collect();
                        if !label.is_empty() && !url_slice.trim().is_empty() {
                            append_inline_file_badge(&mut job, &label, p, &body_font, base_color);
                            let start_c = job.text.chars().count();
                            job.append(
                                &label,
                                0.0,
                                egui::TextFormat {
                                    font_id: body_font.clone(),
                                    color: p.accent_hot,
                                    underline: Stroke::new(1.0_f32, p.accent_hot),
                                    ..Default::default()
                                },
                            );
                            let end_c = job.text.chars().count();
                            if let Some(clean_url) = normalize_clickable_url(&url_slice) {
                                links.push((start_c..end_c, clean_url));
                            }
                            i = cp + 1;
                            continue;
                        }
                    }
                }
            }
        }

        // Bare http:// or https:// URL
        if chars[i] == 'h' {
            if let Some(end) = parse_bare_url_end(&chars, i) {
                let raw_url: String = chars[i..end].iter().collect();
                if let Some(clean_url) = normalize_clickable_url(&raw_url) {
                    let start_c = job.text.chars().count();
                    job.append(
                        &raw_url,
                        0.0,
                        egui::TextFormat {
                            font_id: body_font.clone(),
                            color: p.accent_hot,
                            underline: Stroke::new(1.0_f32, p.accent_hot),
                            ..Default::default()
                        },
                    );
                    let end_c = job.text.chars().count();
                    links.push((start_c..end_c, clean_url));
                    i = end;
                    continue;
                }
            }
        }

        // Plain segment
        let start = i;
        i += 1;
        while i < chars.len() && chars[i] != '`' && chars[i] != '*' && chars[i] != '[' {
            if chars[i] == 'h' && parse_bare_url_end(&chars, i).is_some() {
                break;
            }
            i += 1;
        }
        let slice: String = chars[start..i].iter().collect();
        job.append(
            &slice,
            0.0,
            egui::TextFormat {
                font_id: body_font.clone(),
                color: base_color,
                ..Default::default()
            },
        );
    }

    job.wrap.max_width = max_width.max(40.0);
    (job, links)
}

fn render_inline_text_with_width(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    text: &str,
    base_color: Color32,
    max_width: f32,
    default_bold: bool,
) {
    let (job, links) = build_inline_layout_job(p, text, base_color, max_width, default_bold);
    if links.is_empty() {
        ui.add(egui::Label::new(job).wrap().selectable(true));
    } else {
        let galley = ui.fonts(|f| f.layout_job(job));
        let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
        ui.painter().galley(rect.min, galley.clone(), base_color);
        if let Some(pos) = resp.hover_pos() {
            if let Some(url) = hit_test_galley_link(&galley, pos - rect.min, &links) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                resp.clone().on_hover_text(format!("Open link: {url}"));
                if resp.clicked() {
                    let _ = harness_tools::open_url_safely(url);
                }
            }
        }
    }
}

fn render_inline_text(ui: &mut egui::Ui, p: &CinderPalette, text: &str, base_color: Color32) {
    render_inline_text_with_width(ui, p, text, base_color, ui.available_width().max(80.0), false);
}

pub fn render_markdown(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    text: &str,
    wrap_code: bool,
    _full_message_for_ctx: Option<&str>,
) {
    let blocks = parse_markdown_blocks(text);
    for (idx, blk) in blocks.into_iter().enumerate() {
        ui.push_id(idx, |ui| match blk {
            MdBlock::Heading { level, text } => {
                let sz = match level {
                    1 => p.base_font_size + 5.0,
                    2 => p.base_font_size + 3.0,
                    3 => p.base_font_size + 1.5,
                    _ => p.base_font_size + 0.5,
                };
                if idx > 0 {
                    ui.add_space(6.0);
                }
                let cleaned = clean_inline_markdown_heading(&text);
                ui.label(
                    egui::RichText::new(cleaned)
                        .font(egui::FontId::new(
                            sz,
                            FontFamily::Name("semibold".into()),
                        ))
                        .color(p.text),
                );
                ui.add_space(2.0);
            }
            MdBlock::Paragraph(txt) => {
                render_inline_text(ui, p, &txt, p.text);
                ui.add_space(2.0);
            }
            MdBlock::Bullet(txt) => {
                ui.horizontal_top(|ui| {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("•").strong().color(p.accent));
                    render_inline_text(ui, p, &txt, p.text);
                });
            }
            MdBlock::OrderedItem { number, text } => {
                ui.horizontal_top(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("{number}."))
                            .font(egui::FontId::new(
                                p.base_font_size,
                                FontFamily::Name("semibold".into()),
                            ))
                            .color(p.accent_hot),
                    );
                    render_inline_text(ui, p, &text, p.text);
                });
            }
            MdBlock::Task { checked, text } => {
                ui.horizontal_top(|ui| {
                    let marker = if checked { "[x]" } else { "[ ]" };
                    let col = if checked { p.green } else { p.amber };
                    ui.label(egui::RichText::new(marker).monospace().strong().color(col));
                    render_inline_text(ui, p, &text, p.text);
                });
            }
            MdBlock::Quote(txt) => {
                Frame::new()
                    .fill(p.bg_elev)
                    .stroke(Stroke::new(1.0_f32, p.border_strong))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        render_inline_text(ui, p, &txt, p.muted);
                    });
            }
            MdBlock::Alert { kind, text } => {
                let border_col = match kind.as_str() {
                    "note" | "tip" => p.green,
                    "important" => p.purple,
                    "caution" => p.red,
                    _ => p.amber,
                };
                Frame::new()
                    .fill(p.bg_elev)
                    .stroke(Stroke::new(1.5_f32, border_col))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(kind.to_ascii_uppercase())
                                .size(11.0)
                                .strong()
                                .color(border_col),
                        );
                        render_inline_text(ui, p, &text, p.text);
                    });
            }
            MdBlock::Code { lang, code } => {
                let code_clone = code.clone();
                let copy_id = ui.make_persistent_id(("md_code_copy", idx, &code_clone));
                let line_count = code.lines().count().max(1);
                let font_size = (p.base_font_size - 1.0).max(11.0);
                let (default_h, natural_h) =
                    code_box_default_height(line_count, font_size, 260.0, 72.0, 960.0);
                let size_id = ui.make_persistent_id(("md_code_size", idx, lang.as_str(), code.len()));
                let (mut cur_h, mut user_resized) = ui
                    .data(|d| d.get_temp::<(f32, bool)>(size_id))
                    .unwrap_or((default_h, false));
                cur_h = cur_h.clamp(72.0, 960.0);

                Frame::new()
                    .fill(p.bg_sidebar)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(9))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let tag = if lang.is_empty() { "code" } else { lang.as_str() };
                            let header_meta = if line_count > 1 {
                                format!("{tag} · {line_count} lines")
                            } else {
                                tag.to_string()
                            };
                            ui.label(
                                egui::RichText::new(header_meta)
                                    .size(11.0)
                                    .monospace()
                                    .color(p.muted),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let copied = is_copied_recently(ui.ctx(), copy_id);
                                let copy_rt = if copied {
                                    egui::RichText::new("Copied ✓").small().color(p.green)
                                } else {
                                    egui::RichText::new("Copy").small()
                                };
                                if ghost_button(ui, p, copy_rt).clicked() {
                                    copy_to_clipboard(ui.ctx(), &code_clone);
                                    mark_copied_feedback(ui.ctx(), copy_id);
                                }
                                if natural_h > default_h + 8.0 || user_resized {
                                    let is_expanded = cur_h > default_h + 8.0;
                                    let toggle_label = if is_expanded { "Collapse" } else { "Expand" };
                                    if ghost_button(
                                        ui,
                                        p,
                                        egui::RichText::new(toggle_label).small().color(p.muted),
                                    )
                                    .clicked()
                                    {
                                        if is_expanded {
                                            cur_h = default_h;
                                            user_resized = false;
                                        } else {
                                            cur_h = natural_h;
                                            user_resized = true;
                                        }
                                    }
                                }
                            });
                        });
                        ui.separator();
                        let lang_tag = lang.clone();
                        if wrap_code {
                            let mut scroll = egui::ScrollArea::vertical()
                                .id_salt(("code_v", idx))
                                .max_height(cur_h);
                            if user_resized {
                                scroll = scroll.min_scrolled_height(cur_h);
                            }
                            scroll.show(ui, |ui| {
                                if user_resized {
                                    ui.set_min_height((cur_h - 6.0).max(40.0));
                                }
                                let mut txt = code.clone();
                                let mut layouter =
                                    |ui: &egui::Ui, string: &str, wrap_width: f32| {
                                        let mut j = highlight_code_job(string, &lang_tag, p);
                                        j.wrap.max_width = wrap_width;
                                        ui.fonts(|f| f.layout_job(j))
                                    };
                                let lay = egui::TextEdit::multiline(&mut txt)
                                    .id_salt(("code_te", idx))
                                    .font(egui::TextStyle::Monospace)
                                    .layouter(&mut layouter)
                                    .frame(false)
                                    .desired_width(f32::INFINITY)
                                    .interactive(true);
                                let _ = ui.add_enabled(true, |ui: &mut egui::Ui| ui.add(lay));
                            });
                        } else {
                            let mut scroll = egui::ScrollArea::both()
                                .id_salt(("code_b", idx))
                                .max_height(cur_h);
                            if user_resized {
                                scroll = scroll.min_scrolled_height(cur_h);
                            }
                            scroll.show(ui, |ui| {
                                if user_resized {
                                    ui.set_min_height((cur_h - 6.0).max(40.0));
                                }
                                let job = highlight_code_job(&code, &lang_tag, p);
                                ui.add(egui::Label::new(job).selectable(true));
                            });
                        }
                        if natural_h > default_h + 8.0 || line_count >= 3 || user_resized {
                            render_vertical_resize_handle(
                                ui,
                                p,
                                &mut cur_h,
                                &mut user_resized,
                                default_h,
                                natural_h,
                                72.0,
                                960.0,
                                ("md_code_handle", idx, lang.as_str(), code.len()),
                            );
                        }
                    });
                ui.data_mut(|d| d.insert_temp(size_id, (cur_h, user_resized)));
            }
            MdBlock::Table { headers, rows } => {
                let table_avail_w = ui.available_width().max(260.0);
                let col_widths =
                    compute_table_column_widths(&headers, &rows, (table_avail_w - 2.0).max(220.0));
                let ncols = col_widths.len();
                let total_table_w: f32 = col_widths.iter().sum();

                ui.add_space(4.0);
                egui::ScrollArea::horizontal()
                    .id_salt(("md_tbl", idx))
                    .show(ui, |ui| {
                        Frame::new()
                            .fill(p.bg_elev)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::ZERO)
                            .show(ui, |ui| {
                                ui.set_width(total_table_w);
                                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                                // Header row
                                let hdr_resp = Frame::new()
                                    .fill(p.bg_card)
                                    .inner_margin(Margin::ZERO)
                                    .show(ui, |ui| {
                                        ui.set_width(total_table_w);
                                        ui.horizontal_top(|ui| {
                                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                            for (c_idx, &col_w) in col_widths.iter().enumerate() {
                                                let h_txt = headers
                                                    .get(c_idx)
                                                    .map(String::as_str)
                                                    .unwrap_or("");
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(col_w, 0.0),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.set_min_width(col_w);
                                                        ui.set_max_width(col_w);
                                                        Frame::new()
                                                            .inner_margin(Margin::symmetric(10, 7))
                                                            .show(ui, |ui| {
                                                                let inner_w = (col_w - 20.0).max(40.0);
                                                                render_inline_text_with_width(
                                                                    ui,
                                                                    p,
                                                                    h_txt,
                                                                    p.accent_hot,
                                                                    inner_w,
                                                                    true,
                                                                );
                                                            });
                                                    },
                                                );
                                            }
                                        });
                                    })
                                    .response;
                                let hdr_rect = hdr_resp.rect;
                                paint_table_column_hairlines(
                                    ui.painter(),
                                    hdr_rect,
                                    &col_widths,
                                    p.border,
                                );
                                ui.painter().hline(
                                    hdr_rect.x_range(),
                                    hdr_rect.bottom(),
                                    Stroke::new(1.0_f32, p.border_strong),
                                );

                                // Body rows
                                let stripe_bg = Color32::from_rgba_unmultiplied(
                                    p.bg_card.r(),
                                    p.bg_card.g(),
                                    p.bg_card.b(),
                                    110,
                                );
                                let row_count = rows.len();
                                for (r_idx, r) in rows.iter().enumerate() {
                                    let row_fill = if r_idx % 2 == 1 {
                                        stripe_bg
                                    } else {
                                        Color32::TRANSPARENT
                                    };
                                    let row_resp = Frame::new()
                                        .fill(row_fill)
                                        .inner_margin(Margin::ZERO)
                                        .show(ui, |ui| {
                                            ui.set_width(total_table_w);
                                            ui.horizontal_top(|ui| {
                                                ui.spacing_mut().item_spacing =
                                                    egui::vec2(0.0, 0.0);
                                                for c_idx in 0..ncols {
                                                    let col_w = col_widths[c_idx];
                                                    let cell_txt = r
                                                        .get(c_idx)
                                                        .map(String::as_str)
                                                        .unwrap_or("");
                                                    ui.allocate_ui_with_layout(
                                                        egui::vec2(col_w, 0.0),
                                                        egui::Layout::top_down(egui::Align::Min),
                                                        |ui| {
                                                            ui.set_min_width(col_w);
                                                            ui.set_max_width(col_w);
                                                            Frame::new()
                                                                .inner_margin(Margin::symmetric(
                                                                    10, 6,
                                                                ))
                                                                .show(ui, |ui| {
                                                                    let inner_w =
                                                                        (col_w - 20.0).max(40.0);
                                                                    render_inline_text_with_width(
                                                                        ui,
                                                                        p,
                                                                        cell_txt,
                                                                        p.text,
                                                                        inner_w,
                                                                        false,
                                                                    );
                                                                });
                                                        },
                                                    );
                                                }
                                            });
                                        })
                                        .response;
                                    let rr = row_resp.rect;
                                    paint_table_column_hairlines(
                                        ui.painter(),
                                        rr,
                                        &col_widths,
                                        p.border,
                                    );
                                    if r_idx + 1 < row_count {
                                        ui.painter().hline(
                                            rr.x_range(),
                                            rr.bottom(),
                                            Stroke::new(1.0_f32, p.border),
                                        );
                                    }
                                }
                            });
                    });
                ui.add_space(4.0);
            }
            MdBlock::Rule => {
                ui.add(egui::Separator::default());
            }
        });
    }
}

pub(crate) fn compute_table_column_widths(
    headers: &[String],
    rows: &[Vec<String>],
    avail_w: f32,
) -> Vec<f32> {
    let ncols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
        .max(1);
    let min_col_w = if ncols <= 4 { 130.0_f32 } else { 110.0_f32 };

    let mut scores = Vec::with_capacity(ncols);
    for c in 0..ncols {
        let h_len = headers
            .get(c)
            .map(|s| s.trim().chars().count())
            .unwrap_or(4);
        let mut sum_len = 0usize;
        let mut count = 0usize;
        let mut max_len = h_len;
        for r in rows {
            if let Some(cell) = r.get(c) {
                let len = cell.trim().chars().count();
                sum_len += len;
                count += 1;
                max_len = max_len.max(len);
            }
        }
        let avg_len = if count > 0 {
            sum_len as f32 / count as f32
        } else {
            h_len as f32
        };
        let raw_score =
            (h_len as f32 * 0.6).max(6.0) + (avg_len * 0.9 + (max_len.min(60) as f32) * 0.3);
        scores.push(raw_score.clamp(10.0, 64.0));
    }

    let min_total = min_col_w * ncols as f32;
    if avail_w <= min_total {
        return vec![min_col_w; ncols];
    }

    let extra_w = avail_w - min_total;
    let excess_weights: Vec<f32> = scores.iter().map(|s| (*s - 9.0).max(2.0)).collect();
    let weight_sum: f32 = excess_weights.iter().sum::<f32>().max(1.0);

    let mut widths: Vec<f32> = excess_weights
        .iter()
        .map(|w| (min_col_w + extra_w * (*w / weight_sum)).floor())
        .collect();
    let used: f32 = widths.iter().sum();
    if let Some(last) = widths.last_mut() {
        *last = (*last + (avail_w - used)).max(min_col_w);
    }
    widths
}

fn paint_table_column_hairlines(
    painter: &egui::Painter,
    row_rect: egui::Rect,
    col_widths: &[f32],
    color: Color32,
) {
    let mut x = row_rect.left();
    let stroke = Stroke::new(1.0_f32, color);
    for w in col_widths.iter().take(col_widths.len().saturating_sub(1)) {
        x += *w;
        painter.vline(x, row_rect.top()..=row_rect.bottom(), stroke);
    }
}

pub fn highlight_code_job(code: &str, lang: &str, p: &CinderPalette) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mono = egui::FontId::monospace((p.base_font_size - 1.0).max(11.0));
    let lang_lower = lang.trim().to_ascii_lowercase();
    let hash_comments = matches!(
        lang_lower.as_str(),
        "py" | "python" | "toml" | "sh" | "bash" | "zsh" | "ps1" | "powershell" | "yaml" | "yml"
    );

    let push_span = |job: &mut egui::text::LayoutJob, s: &str, color: Color32, italics: bool| {
        if s.is_empty() {
            return;
        }
        job.append(
            s,
            0.0,
            egui::TextFormat {
                font_id: mono.clone(),
                color,
                italics,
                ..Default::default()
            },
        );
    };

    let is_keyword = |w: &str| -> bool {
        matches!(
            w,
            "fn" | "pub"
                | "let"
                | "mut"
                | "const"
                | "static"
                | "async"
                | "await"
                | "impl"
                | "trait"
                | "struct"
                | "enum"
                | "type"
                | "match"
                | "if"
                | "else"
                | "for"
                | "while"
                | "loop"
                | "return"
                | "break"
                | "continue"
                | "use"
                | "mod"
                | "crate"
                | "self"
                | "Self"
                | "where"
                | "in"
                | "as"
                | "def"
                | "class"
                | "import"
                | "from"
                | "try"
                | "except"
                | "finally"
                | "with"
                | "lambda"
                | "yield"
                | "pass"
                | "raise"
                | "function"
                | "var"
                | "export"
                | "default"
                | "interface"
                | "extends"
                | "implements"
                | "new"
                | "typeof"
                | "true"
                | "false"
                | "null"
                | "None"
                | "Some"
                | "Ok"
                | "Err"
                | "and"
                | "or"
                | "not"
        )
    };

    for line in code.split_inclusive('\n') {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let rest = &line[i..];

            if rest.starts_with("//") {
                push_span(&mut job, rest, p.muted, true);
                break;
            }
            if bytes[i] == b'#'
                && hash_comments
                && !rest.starts_with("#[")
                && !rest.starts_with("#![")
            {
                push_span(&mut job, rest, p.muted, true);
                break;
            }
            if (rest.starts_with("#[") || rest.starts_with("#![")) && !hash_comments {
                if let Some(close) = rest.find(']') {
                    let end = i + close + 1;
                    push_span(&mut job, &line[i..end], p.purple, false);
                    i = end;
                    continue;
                }
            }

            if bytes[i] == b'"' || bytes[i] == b'`' || bytes[i] == b'\'' {
                let delim = bytes[i];
                let mut has_close = delim != b'\'' || !matches!(lang_lower.as_str(), "rs" | "rust");
                let mut j = i + 1;
                if !has_close {
                    // For Rust, check if `'x'` character literal vs lifetime `'a`
                    while j < bytes.len() {
                        if bytes[j] == b'\\' && j + 1 < bytes.len() {
                            j += 2;
                            continue;
                        }
                        if bytes[j] == b'\'' {
                            has_close = (j - i) <= 4;
                            break;
                        }
                        if bytes[j].is_ascii_whitespace() || j - i > 4 {
                            break;
                        }
                        j += 1;
                    }
                    j = i + 1;
                }
                if has_close {
                    while j < bytes.len() {
                        if bytes[j] == b'\\' && j + 1 < bytes.len() {
                            j += 2;
                            continue;
                        }
                        if bytes[j] == delim {
                            j += 1;
                            break;
                        }
                        if bytes[j] == b'\n' {
                            break;
                        }
                        j += 1;
                    }
                    push_span(&mut job, &line[i..j], p.green, false);
                    i = j;
                    continue;
                }
            }

            if bytes[i].is_ascii_digit()
                && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
            {
                let mut j = i + 1;
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric()
                        || bytes[j] == b'.'
                        || bytes[j] == b'_')
                {
                    j += 1;
                }
                push_span(&mut job, &line[i..j], p.purple, false);
                i = j;
                continue;
            }

            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                let word = &line[i..j];
                if j < bytes.len() && bytes[j] == b'!' {
                    j += 1;
                    push_span(&mut job, &line[i..j], p.purple, false);
                } else if is_keyword(word) {
                    push_span(&mut job, word, p.accent_hot, false);
                } else if word.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
                    && word.len() > 1
                {
                    push_span(&mut job, word, p.amber, false);
                } else {
                    push_span(&mut job, word, p.text, false);
                }
                i = j;
                continue;
            }

            let ch = rest.chars().next().unwrap();
            let next = i + ch.len_utf8();
            push_span(&mut job, &line[i..next], p.text, false);
            i = next;
        }
    }

    job
}

pub fn code_box_default_height(
    line_count: usize,
    font_size: f32,
    default_cap: f32,
    min_h: f32,
    max_h: f32,
) -> (f32, f32) {
    let line_h = (font_size + 5.0).max(15.0);
    let natural_h = (line_count.max(1) as f32 * line_h + 18.0).clamp(min_h, max_h);
    let default_h = natural_h.min(default_cap);
    (default_h, natural_h)
}

#[allow(clippy::too_many_arguments)]
pub fn render_vertical_resize_handle(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    height: &mut f32,
    user_resized: &mut bool,
    default_h: f32,
    natural_h: f32,
    min_h: f32,
    max_h: f32,
    id_salt: impl std::hash::Hash,
) {
    let width = ui.available_width().max(48.0);
    let (rect, resp) = ui
        .push_id(id_salt, |ui| {
            ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::click_and_drag())
        })
        .inner;
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Drag to resize · Double-click to fit or reset");
    if resp.dragged() {
        *height = (*height + resp.drag_delta().y).clamp(min_h, max_h);
        *user_resized = true;
    }
    if resp.double_clicked() {
        if (*height - natural_h).abs() > 8.0 && natural_h > default_h + 4.0 {
            *height = natural_h;
            *user_resized = true;
        } else {
            *height = default_h;
            *user_resized = false;
        }
    }
    let pill_col = if resp.hovered() || resp.dragged() {
        p.accent
    } else {
        p.border_strong
    };
    let pill_w = 38.0_f32.min((rect.width() - 8.0).max(12.0));
    let pill_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(pill_w, 3.0));
    ui.painter()
        .rect_filled(pill_rect, CornerRadius::same(2), pill_col);
}

pub fn render_resizable_text_box(
    ui: &mut egui::Ui,
    p: &CinderPalette,
    text: &str,
    id_key: &str,
    default_cap: f32,
    min_h: f32,
    max_h: f32,
) {
    let line_count = text.lines().count().max(1);
    let (default_h, natural_h) = code_box_default_height(line_count, 11.5, default_cap, min_h, max_h);
    let size_id = ui.make_persistent_id(("resiz_txt_box", id_key));
    let (mut cur_h, mut user_resized) = ui
        .data(|d| d.get_temp::<(f32, bool)>(size_id))
        .unwrap_or((default_h, false));
    cur_h = cur_h.clamp(min_h, max_h);

    let mut scroll = egui::ScrollArea::both()
        .id_salt(("resiz_txt_scroll", id_key))
        .max_height(cur_h);
    if user_resized {
        scroll = scroll.min_scrolled_height(cur_h);
    }
    scroll.show(ui, |ui| {
        if user_resized {
            ui.set_min_height((cur_h - 6.0).max(32.0));
        }
        ui.add(egui::Label::new(egui::RichText::new(text).monospace().small()).selectable(true));
    });

    if natural_h > default_h + 8.0 || line_count >= 3 || user_resized {
        render_vertical_resize_handle(
            ui,
            p,
            &mut cur_h,
            &mut user_resized,
            default_h,
            natural_h,
            min_h,
            max_h,
            ("resiz_txt_handle", id_key),
        );
    }
    ui.data_mut(|d| d.insert_temp(size_id, (cur_h, user_resized)));
}

pub fn render_diff(ui: &mut egui::Ui, p: &CinderPalette, diff_text: &str, id_key: &str) {
    let lines: Vec<&str> = diff_text.lines().take(1500).collect();
    let line_count = lines.len().max(1);
    let (default_h, natural_h) = code_box_default_height(line_count, 11.5, 240.0, 80.0, 960.0);
    let size_id = ui.make_persistent_id(("diff_size", id_key));
    let (mut cur_h, mut user_resized) = ui
        .data(|d| d.get_temp::<(f32, bool)>(size_id))
        .unwrap_or((default_h, false));
    cur_h = cur_h.clamp(80.0, 960.0);

    Frame::new()
        .fill(p.bg_sidebar)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            if natural_h > default_h + 8.0 || user_resized {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("diff · {line_count} lines"))
                            .size(10.5)
                            .monospace()
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let is_expanded = cur_h > default_h + 8.0;
                        let toggle_label = if is_expanded { "Collapse" } else { "Expand" };
                        if ghost_button(
                            ui,
                            p,
                            egui::RichText::new(toggle_label).small().color(p.muted),
                        )
                        .clicked()
                        {
                            if is_expanded {
                                cur_h = default_h;
                                user_resized = false;
                            } else {
                                cur_h = natural_h;
                                user_resized = true;
                            }
                        }
                    });
                });
                ui.add_space(2.0);
            }

            let mut scroll = egui::ScrollArea::both()
                .id_salt(("diff_scroll", id_key))
                .max_height(cur_h);
            if user_resized {
                scroll = scroll.min_scrolled_height(cur_h);
            }
            scroll.show(ui, |ui| {
                if user_resized {
                    ui.set_min_height((cur_h - 6.0).max(40.0));
                }
                for line in lines {
                    let color = if line.starts_with('+') && !line.starts_with("+++") {
                        p.green
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        p.red
                    } else if line.starts_with("@@") {
                        p.purple
                    } else {
                        p.muted
                    };
                    ui.label(
                        egui::RichText::new(line)
                            .monospace()
                            .size(11.5)
                            .color(color),
                    );
                }
            });

            if natural_h > default_h + 8.0 || line_count >= 3 || user_resized {
                render_vertical_resize_handle(
                    ui,
                    p,
                    &mut cur_h,
                    &mut user_resized,
                    default_h,
                    natural_h,
                    80.0,
                    960.0,
                    ("diff_handle", id_key),
                );
            }
        });
    ui.data_mut(|d| d.insert_temp(size_id, (cur_h, user_resized)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_box_default_height_scales_and_clamps() {
        let (d_short, n_short) = code_box_default_height(2, 12.0, 260.0, 72.0, 960.0);
        assert_eq!(d_short, 72.0);
        assert_eq!(n_short, 72.0);

        let (d_long, n_long) = code_box_default_height(40, 12.0, 260.0, 72.0, 960.0);
        assert_eq!(d_long, 260.0);
        assert!(n_long > 600.0 && n_long <= 960.0);

        let (d_huge, n_huge) = code_box_default_height(500, 12.0, 260.0, 72.0, 960.0);
        assert_eq!(d_huge, 260.0);
        assert_eq!(n_huge, 960.0);
    }

    #[test]
    fn parses_code_tables_and_alerts() {
        let md = "# Title\n\n> [!NOTE] Important note\n\n```rust\nfn main() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let blocks = parse_markdown_blocks(md);
        assert!(matches!(blocks[0], MdBlock::Heading { level: 1, .. }));
        assert!(matches!(blocks[1], MdBlock::Alert { ref kind, .. } if kind == "note"));
        assert!(matches!(blocks[2], MdBlock::Code { ref lang, .. } if lang == "rust"));
        assert!(matches!(blocks[3], MdBlock::Table { .. }));
    }

    #[test]
    fn parses_ordered_lists_cleans_headings_and_latex_arrows() {
        assert_eq!(clean_inline_markdown_heading("**Project Overview**"), "Project Overview");
        assert_eq!(
            normalize_inline_latex("Nudge $\\rightarrow$ Concept Check $\\rightarrow$ Micro-Step"),
            "Nudge → Concept Check → Micro-Step"
        );
        let blocks = parse_markdown_blocks("#### Subsection\n1.  **The 6 Modules:**\n2.  Schedule");
        assert!(matches!(blocks[0], MdBlock::Heading { level: 4, .. }));
        assert!(matches!(blocks[1], MdBlock::OrderedItem { ref number, .. } if number == "1"));
        assert!(matches!(blocks[2], MdBlock::OrderedItem { ref number, .. } if number == "2"));
    }

    #[test]
    fn syntax_highlighter_preserves_exact_source_text() {
        let p = CinderPalette::from_ui(&harness_store::UiCustomization::default());
        let sample = "// comment\n#[derive(Debug)]\npub fn hello(x: usize) -> String {\n    println!(\"hi {x}\");\n    \"ok\".to_string()\n}\n";
        let job = highlight_code_job(sample, "rust", &p);
        assert_eq!(job.text, sample);
        assert!(job.sections.len() >= 6);
    }

    #[test]
    fn extracts_markdown_and_bare_clickable_urls() {
        assert_eq!(
            normalize_clickable_url(" https://en.wikipedia.org/wiki/Langlands_program "),
            Some("https://en.wikipedia.org/wiki/Langlands_program".into())
        );
        assert_eq!(
            normalize_clickable_url("www.example.com/docs"),
            Some("https://www.example.com/docs".into())
        );
        assert_eq!(normalize_clickable_url("javascript:alert(1)"), None);

        let text = "See (https://en.wikipedia.org/wiki/Langlands_program_(math)) for more.";
        let chars: Vec<char> = text.chars().collect();
        let start = text.find("https://").unwrap();
        let start_c = text[..start].chars().count();
        let end_c = parse_bare_url_end(&chars, start_c).unwrap();
        let url: String = chars[start_c..end_c].iter().collect();
        assert_eq!(url, "https://en.wikipedia.org/wiki/Langlands_program_(math)");
    }

    #[test]
    fn table_column_widths_fill_canvas_and_respect_minimums() {
        let headers = vec![
            "Capability".to_string(),
            "Risk".to_string(),
            "Mitigation".to_string(),
        ];
        let rows = vec![vec![
            "Reading files".to_string(),
            "May expose secrets (keys, .env, tokens) to the model".to_string(),
            "Exclude sensitive paths; keep Strict Local mode for private code".to_string(),
        ]];
        let widths = compute_table_column_widths(&headers, &rows, 720.0);
        assert_eq!(widths.len(), 3);
        let sum: f32 = widths.iter().sum();
        assert!((sum - 720.0).abs() <= 2.0, "sum={sum}");
        assert!(widths[0] >= 130.0);
        assert!(widths[1] > widths[0] && widths[2] > widths[0]);

        // 6 columns in 420px triggers horizontal scroll minimums without crushing words
        let wide_headers = vec!["A", "B", "C", "D", "E", "F"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let wide_widths = compute_table_column_widths(&wide_headers, &[], 420.0);
        assert_eq!(wide_widths.len(), 6);
        assert!(wide_widths.iter().all(|&w| w >= 110.0));
    }

    #[test]
    fn bold_spans_render_nested_inline_code_without_literal_backticks() {
        let p = CinderPalette::from_ui(&harness_store::UiCustomization::default());
        let (job, _) = build_inline_layout_job(
            &p,
            "**Path sandbox (`resolve_workspace_path`):** holds up.",
            p.text,
            600.0,
            false,
        );
        assert!(!job.text.contains('`'), "unexpected literal backtick in {:?}", job.text);
        assert!(job.text.contains("resolve_workspace_path"));
        assert!(job.sections.len() >= 4);
    }
}
