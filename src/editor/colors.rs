//! Color picker for CSS color values: a click on the color bar under a value
//! opens it, and changes are written back in the value's own notation.

use std::ops::Range;

use eframe::egui::{self, Color32, Id, Pos2, Rect, Ui};

use super::document::{Change, Document, EditKind};

/// Width of a short hex value's digits, e.g. `#abc` → 1.
fn hex_digits(len: usize) -> usize {
    if len <= 5 { 1 } else { 2 }
}

/// `c` written like `original` (hex, rgb()/rgba(), hsl()/hsla(); commas or spaces).
pub fn format_like(original: &str, c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let o = original.trim();
    let alpha = a as f32 / 255.0;
    let alpha_text = format!("{}", (alpha * 100.0).round() / 100.0);
    if let Some(hex) = o.strip_prefix('#') {
        let upper = hex.chars().any(|ch| ch.is_ascii_uppercase());
        let had_alpha = hex.len() == 4 || hex.len() == 8;
        // Keep the short form when the color fits it.
        let short = hex_digits(hex.len()) == 1 && [r, g, b, a].iter().all(|v| v % 17 == 0);
        let mut out = if short {
            format!("{:x}{:x}{:x}", r / 17, g / 17, b / 17)
        } else {
            format!("{r:02x}{g:02x}{b:02x}")
        };
        if had_alpha || a < 255 {
            out.push_str(&if short { format!("{:x}", a / 17) } else { format!("{a:02x}") });
        }
        if upper {
            out = out.to_uppercase();
        }
        return format!("#{out}");
    }
    let lower = o.to_lowercase();
    let commas = o.contains(',');
    let with_alpha = |parts: [String; 3], name: &str, legacy_a: &str| -> String {
        if commas {
            if a < 255 || lower.starts_with(legacy_a) {
                format!("{legacy_a}({}, {}, {}, {alpha_text})", parts[0], parts[1], parts[2])
            } else {
                format!("{name}({}, {}, {})", parts[0], parts[1], parts[2])
            }
        } else if a < 255 || o.contains('/') {
            format!("{name}({} {} {} / {alpha_text})", parts[0], parts[1], parts[2])
        } else {
            format!("{name}({} {} {})", parts[0], parts[1], parts[2])
        }
    };
    if lower.starts_with("hsl") {
        let (h, s, l) = to_hsl(r, g, b);
        return with_alpha([format!("{}", h.round()), format!("{}%", s.round()), format!("{}%", l.round())], "hsl", "hsla");
    }
    with_alpha([r.to_string(), g.to_string(), b.to_string()], "rgb", "rgba")
}

/// Hue in degrees, saturation and lightness in percent.
fn to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l * 100.0);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s * 100.0, l * 100.0)
}

/// Opens the picker if the click at `p` hit a color bar; true if it did.
pub fn click(doc: &mut Document, p: Pos2) -> bool {
    let Some((rect, range)) = doc.view.color_hits.iter().find(|(r, _)| r.contains(p)).cloned() else { return false };
    doc.view.color_edit = Some((range, rect.left_bottom()));
    // Picker changes form one undo step of their own.
    doc.break_undo_group();
    true
}

/// Draws the open picker and applies changes to the document.
pub fn show(ui: &Ui, doc: &mut Document, id: Id) {
    // Pointing hand over the color bars.
    if let Some(p) = ui.ctx().pointer_hover_pos()
        && doc.view.color_hits.iter().any(|(r, _)| r.contains(p))
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let Some((range, anchor)) = doc.view.color_edit.clone() else { return };
    let original = if range.end <= doc.len_chars() { doc.slice(range.clone()) } else { String::new() };
    let Some(mut color) = crate::theme_import::parse_color(&original) else {
        doc.view.color_edit = None;
        return;
    };
    let before = color;
    let area = egui::Area::new(id.with("color-picker")).order(egui::Order::Foreground).fixed_pos(anchor + egui::vec2(0.0, 4.0)).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            egui::widgets::color_picker::color_picker_color32(ui, &mut color, egui::widgets::color_picker::Alpha::OnlyBlend);
        });
    });
    if color != before {
        let text = format_like(&original, color);
        let len = text.chars().count();
        doc.apply(vec![Change::new(range.clone(), text)], None, EditKind::Typing);
        doc.view.color_edit = Some((range.start..range.start + len, anchor));
    }
    // Closes with Esc or a click elsewhere.
    let clicked_outside = ui.input(|i| i.pointer.any_pressed())
        && ui.ctx().pointer_interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !hit_rect(doc, p));
    if clicked_outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        doc.view.color_edit = None;
    }
}

fn hit_rect(doc: &Document, p: Pos2) -> bool {
    doc.view.color_hits.iter().any(|(r, _): &(Rect, Range<usize>)| r.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_notation() {
        let red = Color32::from_rgb(255, 0, 0);
        assert_eq!(format_like("#00f", red), "#f00");
        assert_eq!(format_like("#00F", Color32::from_rgb(18, 52, 86)), "#123456");
        assert_eq!(format_like("#0000ff", red), "#ff0000");
        assert_eq!(format_like("rgb(0, 0, 255)", red), "rgb(255, 0, 0)");
        assert_eq!(format_like("rgb(0 0 255)", red), "rgb(255 0 0)");
        let half = Color32::from_rgba_unmultiplied(255, 0, 0, 128);
        assert_eq!(format_like("rgb(0, 0, 255)", half), "rgba(255, 0, 0, 0.5)");
        assert_eq!(format_like("#0000ff", half), "#ff000080");
        assert_eq!(format_like("hsl(240, 100%, 50%)", red), "hsl(0, 100%, 50%)");
        assert_eq!(format_like("hsla(240, 100%, 50%, 1)", red), "hsla(0, 100%, 50%, 1)");
    }
}
