//! Vector icons drawn with the painter, crisp at any scale.

use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::theme::{Theme, bold_family};

/// Badge color and label for a file name.
pub fn file_kind(name: &str) -> (Color32, &'static str) {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" | "xhtml" | "shtml" | "tpl" => (Color32::from_rgb(235, 104, 52), "<>"),
        "css" => (Color32::from_rgb(52, 132, 228), "#"),
        "scss" | "sass" | "less" => (Color32::from_rgb(205, 90, 160), "#"),
        "js" | "mjs" | "cjs" | "jsx" => (Color32::from_rgb(232, 190, 40), "JS"),
        "ts" | "tsx" => (Color32::from_rgb(50, 120, 200), "TS"),
        "php" | "phtml" | "inc" => (Color32::from_rgb(122, 110, 210), "php"),
        "json" | "webmanifest" => (Color32::from_rgb(90, 170, 110), "{}"),
        "xml" | "svg" => (Color32::from_rgb(230, 140, 60), "</"),
        "md" | "markdown" => (Color32::from_rgb(110, 120, 135), "M"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "avif" | "bmp" => (Color32::from_rgb(196, 92, 196), "img"),
        "txt" | "log" => (Color32::from_rgb(130, 136, 145), "txt"),
        "htaccess" | "ini" | "conf" | "env" | "yml" | "yaml" | "toml" => (Color32::from_rgb(120, 128, 140), "cfg"),
        "rs" => (Color32::from_rgb(222, 120, 70), "rs"),
        "py" => (Color32::from_rgb(60, 120, 190), "py"),
        "sql" => (Color32::from_rgb(220, 150, 40), "sql"),
        "go" => (Color32::from_rgb(60, 170, 200), "go"),
        "java" => (Color32::from_rgb(200, 90, 60), "jv"),
        "c" | "h" => (Color32::from_rgb(90, 110, 190), "C"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => (Color32::from_rgb(70, 100, 180), "C++"),
        "cs" => (Color32::from_rgb(110, 80, 180), "C#"),
        "sh" | "bash" | "zsh" => (Color32::from_rgb(80, 150, 90), "sh"),
        "rb" | "erb" => (Color32::from_rgb(200, 60, 70), "rb"),
        "lua" => (Color32::from_rgb(40, 60, 160), "lua"),
        "diff" | "patch" => (Color32::from_rgb(100, 160, 120), "+-"),
        "woff" | "woff2" | "ttf" | "otf" => (Color32::from_rgb(150, 150, 160), "Aa"),
        _ => (Color32::from_rgb(140, 146, 156), ""),
    }
}

/// A document icon with a folded corner and a type label.
pub fn file(painter: &Painter, center: Pos2, name: &str) {
    let (color, label) = file_kind(name);
    let r = Rect::from_center_size(center, vec2(13.0, 16.0));
    let fold = 4.5;
    // Page with the top-right corner cut off, plus the folded flap.
    painter.add(Shape::convex_polygon(
        vec![
            r.left_top(),
            pos2(r.right() - fold, r.top()),
            pos2(r.right(), r.top() + fold),
            r.right_bottom(),
            r.left_bottom(),
        ],
        color,
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        vec![
            pos2(r.right() - fold, r.top()),
            pos2(r.right(), r.top() + fold),
            pos2(r.right() - fold, r.top() + fold),
        ],
        Color32::from_white_alpha(110),
        Stroke::NONE,
    ));
    if label.is_empty() {
        for i in 0..3 {
            let y = r.top() + 7.0 + i as f32 * 2.5;
            painter.line_segment(
                [pos2(r.left() + 3.0, y), pos2(r.right() - 3.0, y)],
                Stroke::new(1.0, Color32::from_white_alpha(170)),
            );
        }
    } else {
        let size = match label.len() {
            1 => 9.0,
            2 => 7.5,
            _ => 6.2,
        };
        painter.text(
            pos2(r.center().x, r.center().y + 1.5),
            Align2::CENTER_CENTER,
            label,
            FontId::new(size, bold_family()),
            Color32::WHITE,
        );
    }
}

pub fn folder(painter: &Painter, center: Pos2, color: Color32, open: bool) {
    let back = Rect::from_min_size(pos2(center.x - 8.0, center.y - 6.0), vec2(16.0, 12.5));
    let tab = Rect::from_min_size(pos2(center.x - 8.0, center.y - 7.5), vec2(7.0, 4.0));
    painter.rect_filled(tab, 1.5, color.gamma_multiply(0.85));
    painter.rect_filled(back, 2.5, color.gamma_multiply(0.85));
    if open {
        painter.add(Shape::convex_polygon(
            vec![
                pos2(center.x - 6.5, center.y - 2.0),
                pos2(center.x + 9.0, center.y - 2.0),
                pos2(center.x + 7.0, center.y + 6.5),
                pos2(center.x - 8.0, center.y + 6.5),
            ],
            color,
            Stroke::NONE,
        ));
    } else {
        let front = Rect::from_min_size(pos2(center.x - 8.0, center.y - 3.5), vec2(16.0, 10.0));
        painter.rect_filled(front, 2.5, color);
    }
}

pub fn server(painter: &Painter, center: Pos2, color: Color32) {
    for dy in [-3.5, 3.5] {
        let r = Rect::from_center_size(center + vec2(0.0, dy), vec2(15.0, 6.0));
        painter.rect_filled(r, 1.8, color);
        painter.circle_filled(pos2(r.right() - 3.0, r.center().y), 1.0, Color32::from_black_alpha(140));
        painter.line_segment(
            [pos2(r.left() + 2.5, r.center().y), pos2(r.left() + 7.0, r.center().y)],
            Stroke::new(1.0, Color32::from_black_alpha(100)),
        );
    }
}

pub fn cloud(painter: &Painter, center: Pos2, color: Color32) {
    painter.circle_filled(center + vec2(-3.0, 1.5), 3.2, color);
    painter.circle_filled(center + vec2(1.0, -1.0), 4.2, color);
    painter.circle_filled(center + vec2(4.5, 1.8), 2.9, color);
    painter.rect_filled(
        Rect::from_min_max(center + vec2(-3.0, 1.0), center + vec2(4.5, 4.7)),
        0.0,
        color,
    );
}

pub fn chevron(painter: &Painter, c: Pos2, open: bool, color: Color32) {
    let pts = if open {
        vec![pos2(c.x - 3.5, c.y - 1.8), pos2(c.x + 3.5, c.y - 1.8), pos2(c.x, c.y + 2.5)]
    } else {
        vec![pos2(c.x - 1.8, c.y - 3.5), pos2(c.x + 2.5, c.y), pos2(c.x - 1.8, c.y + 3.5)]
    };
    painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Thin `›` used between breadcrumb segments.
pub fn chevron_thin(painter: &Painter, c: Pos2, color: Color32) {
    painter.line_segment([pos2(c.x - 1.5, c.y - 3.5), pos2(c.x + 2.0, c.y)], Stroke::new(1.2, color));
    painter.line_segment([pos2(c.x + 2.0, c.y), pos2(c.x - 1.5, c.y + 3.5)], Stroke::new(1.2, color));
}

pub fn sidebar_toggle(painter: &Painter, c: Pos2, color: Color32) {
    let r = Rect::from_center_size(c, vec2(16.0, 13.0));
    painter.rect_stroke(r, 2.5, Stroke::new(1.3, color), eframe::egui::StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(r.min, vec2(5.5, r.height())), 2.0, color);
}

pub fn close(painter: &Painter, c: Pos2, size: f32, color: Color32) {
    let s = size / 2.0;
    let stroke = Stroke::new(1.3, color);
    painter.line_segment([c + vec2(-s, -s), c + vec2(s, s)], stroke);
    painter.line_segment([c + vec2(s, -s), c + vec2(-s, s)], stroke);
}

pub fn minimize(painter: &Painter, c: Pos2, color: Color32) {
    painter.line_segment([c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)], Stroke::new(1.0, color));
}

pub fn maximize(painter: &Painter, c: Pos2, color: Color32, restored: bool) {
    let stroke = Stroke::new(1.0, color);
    if restored {
        let back = Rect::from_center_size(c + vec2(1.5, -1.5), vec2(8.0, 8.0));
        painter.rect_stroke(back, 1.0, stroke, eframe::egui::StrokeKind::Inside);
        let front = Rect::from_center_size(c + vec2(-1.0, 1.0), vec2(8.0, 8.0));
        painter.rect_filled(front, 1.0, Color32::TRANSPARENT);
        painter.rect_stroke(front, 1.0, stroke, eframe::egui::StrokeKind::Inside);
    } else {
        let r = Rect::from_center_size(c, vec2(10.0, 10.0));
        painter.rect_stroke(r, 1.0, stroke, eframe::egui::StrokeKind::Inside);
    }
}

pub fn search(painter: &Painter, c: Pos2, color: Color32) {
    painter.circle_stroke(c + vec2(-1.0, -1.0), 4.0, Stroke::new(1.3, color));
    painter.line_segment([c + vec2(2.0, 2.0), c + vec2(5.0, 5.0)], Stroke::new(1.5, color));
}

/// Eye ("preview").
pub fn eye(painter: &Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    // Two arcs form the lid shape.
    let lid = |sign: f32| -> Vec<Pos2> {
        (0..=16)
            .map(|i| {
                let t = i as f32 / 16.0;
                let x = -7.0 + 14.0 * t;
                c + vec2(x, sign * 4.6 * (1.0 - (x / 7.0).powi(2)))
            })
            .collect()
    };
    painter.add(Shape::line(lid(-1.0), stroke));
    painter.add(Shape::line(lid(1.0), stroke));
    painter.circle_stroke(c, 2.3, stroke);
}

/// Indented list with bullets ("outline").
pub fn outline(painter: &Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    for (indent, dy) in [(0.0, -4.5), (3.5, 0.0), (3.5, 4.5)] {
        let x = c.x - 6.5 + indent;
        painter.circle_filled(pos2(x, c.y + dy), 1.2, color);
        painter.line_segment([pos2(x + 3.0, c.y + dy), pos2(c.x + 6.5, c.y + dy)], stroke);
    }
}

pub fn plus(painter: &Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    painter.line_segment([c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)], stroke);
    painter.line_segment([c + vec2(0.0, -5.0), c + vec2(0.0, 5.0)], stroke);
}

pub fn refresh(painter: &Painter, c: Pos2, color: Color32) {
    let r = 5.0;
    let pts: Vec<Pos2> = (0..=20)
        .map(|i| {
            let a = -0.3 + i as f32 / 20.0 * 4.9;
            c + vec2(a.cos() * r, a.sin() * r)
        })
        .collect();
    let end = *pts.last().unwrap();
    painter.add(Shape::line(pts, Stroke::new(1.3, color)));
    painter.add(Shape::convex_polygon(
        vec![end + vec2(-3.0, -1.5), end + vec2(2.5, -2.5), end + vec2(0.5, 2.5)],
        color,
        Stroke::NONE,
    ));
}

pub fn menu(painter: &Painter, c: Pos2, color: Color32) {
    for dy in [-4.0, 0.0, 4.0] {
        painter.line_segment([c + vec2(-6.0, dy), c + vec2(6.0, dy)], Stroke::new(1.4, color));
    }
}

pub fn new_file(painter: &Painter, c: Pos2, color: Color32) {
    let r = Rect::from_center_size(c, vec2(12.0, 15.0));
    painter.rect_stroke(r, 2.0, Stroke::new(1.3, color), eframe::egui::StrokeKind::Inside);
    let stroke = Stroke::new(1.3, color);
    let c = c + vec2(0.0, 0.5);
    painter.line_segment([c + vec2(-3.0, 0.0), c + vec2(3.0, 0.0)], stroke);
    painter.line_segment([c + vec2(0.0, -3.0), c + vec2(0.0, 3.0)], stroke);
}

/// Small rounded "pill" button with a painted icon; returns the response.
pub fn icon_button(
    ui: &mut eframe::egui::Ui,
    theme: &Theme,
    size: eframe::egui::Vec2,
    selected: bool,
    paint: impl FnOnce(&Painter, Pos2, Color32),
) -> eframe::egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, eframe::egui::Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 6.0, theme.hover.gamma_multiply(2.2));
    } else if resp.hovered() {
        painter.rect_filled(rect, 6.0, theme.hover);
    }
    let color = if selected || resp.hovered() { theme.foreground } else { theme.text_muted };
    paint(painter, rect.center(), color);
    resp
}

/// Cloud with an upward arrow ("publish").
pub fn upload(painter: &Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    painter.line_segment([c + vec2(0.0, 5.0), c + vec2(0.0, -4.0)], stroke);
    painter.line_segment([c + vec2(-3.5, -0.5), c + vec2(0.0, -4.0)], stroke);
    painter.line_segment([c + vec2(3.5, -0.5), c + vec2(0.0, -4.0)], stroke);
    painter.line_segment([c + vec2(-6.0, 6.0), c + vec2(6.0, 6.0)], stroke);
}

/// Square with an arrow leaving it ("open externally").
pub fn external(painter: &Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    let r = Rect::from_center_size(c + vec2(-1.0, 1.0), vec2(11.0, 11.0));
    painter.line_segment([r.left_top(), pos2(r.center().x, r.top())], stroke);
    painter.line_segment([r.left_top(), r.left_bottom()], stroke);
    painter.line_segment([r.left_bottom(), r.right_bottom()], stroke);
    painter.line_segment([r.right_bottom(), pos2(r.right(), r.center().y)], stroke);
    let tip = c + vec2(6.0, -6.0);
    painter.line_segment([tip, c + vec2(0.0, 0.0)], stroke);
    painter.line_segment([tip, tip + vec2(-4.5, 0.0)], stroke);
    painter.line_segment([tip, tip + vec2(0.0, 4.5)], stroke);
}

/// Angle brackets ("developer tools").
pub fn devtools(painter: &Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    painter.line_segment([c + vec2(-3.0, -4.5), c + vec2(-7.0, 0.0)], stroke);
    painter.line_segment([c + vec2(-7.0, 0.0), c + vec2(-3.0, 4.5)], stroke);
    painter.line_segment([c + vec2(3.0, -4.5), c + vec2(7.0, 0.0)], stroke);
    painter.line_segment([c + vec2(7.0, 0.0), c + vec2(3.0, 4.5)], stroke);
    painter.line_segment([c + vec2(1.5, -5.5), c + vec2(-1.5, 5.5)], stroke);
}

/// Four blocks, one offset ("extensions").
pub fn blocks(painter: &Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    for (dx, dy) in [(-3.5, 3.5), (3.5, 3.5), (-3.5, -3.5)] {
        let r = Rect::from_center_size(c + vec2(dx, dy), vec2(5.5, 5.5));
        painter.rect_stroke(r, 1.5, stroke, eframe::egui::StrokeKind::Inside);
    }
    let r = Rect::from_center_size(c + vec2(4.5, -4.5), vec2(5.5, 5.5));
    painter.rect_filled(r, 1.5, color);
}

/// The file icon of a language (as for a file with its main extension).
pub fn language(painter: &Painter, center: Pos2, lang: crate::lang::Language) {
    let ext = lang.default_extensions().first().copied().unwrap_or("txt");
    file(painter, center, &format!("x.{ext}"));
}

/// A selectable list entry with the language's icon in front of `label`.
pub fn language_item(ui: &mut egui::Ui, selected: bool, lang: crate::lang::Language, label: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, ui.visuals().text_color());
    // Popups offer far more width than a short name needs.
    let width = ui.available_width().min(220.0).max(galley.size().x + 40.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), egui::Sense::click());
    let visuals = ui.visuals();
    if selected {
        ui.painter().rect_filled(rect, 4.0, visuals.selection.bg_fill);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let color = if selected { visuals.selection.stroke.color } else { visuals.text_color() };
    language(ui.painter(), pos2(rect.left() + 13.0, rect.center().y), lang);
    ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0), galley, color);
    resp
}

/// Window with a filled strip on the right (right panel toggle).
pub fn panel_right(painter: &Painter, c: Pos2, color: Color32) {
    let r = Rect::from_center_size(c, vec2(16.0, 13.0));
    painter.rect_stroke(r, 2.5, Stroke::new(1.3, color), eframe::egui::StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_max(pos2(r.right() - 5.5, r.top()), r.max), 2.0, color);
}

/// Window split into two halves (split view toggle).
pub fn split_view(painter: &Painter, c: Pos2, color: Color32) {
    let r = Rect::from_center_size(c, vec2(16.0, 13.0));
    let stroke = Stroke::new(1.3, color);
    painter.rect_stroke(r, 2.5, stroke, eframe::egui::StrokeKind::Inside);
    painter.line_segment([pos2(c.x, r.top() + 1.0), pos2(c.x, r.bottom() - 1.0)], stroke);
}
