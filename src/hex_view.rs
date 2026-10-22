//! Read-only hex view for binary files: offset, 16 bytes in hex and as text.
//! Large local files are read on demand instead of loaded completely.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align2, FontId, Rect, Sense, pos2, vec2};

use crate::theme::Theme;

const PER_ROW: usize = 16;

pub enum Source {
    Memory(Arc<Vec<u8>>),
    /// A local file read in pieces.
    File { path: PathBuf, len: u64 },
}

pub struct HexData {
    pub source: Source,
    /// Last window read from a file: (offset, bytes).
    cache: Mutex<Option<(u64, Vec<u8>)>>,
}

/// What the user asked for in the hex view.
pub enum Action {
    None,
    /// Show the file as text after all (binary detection was wrong).
    OpenAsText,
}

impl HexData {
    pub fn memory(bytes: Vec<u8>) -> Self {
        Self { source: Source::Memory(Arc::new(bytes)), cache: Mutex::new(None) }
    }

    pub fn file(path: PathBuf, len: u64) -> Self {
        Self { source: Source::File { path, len }, cache: Mutex::new(None) }
    }

    pub fn len(&self) -> u64 {
        match &self.source {
            Source::Memory(b) => b.len() as u64,
            Source::File { len, .. } => *len,
        }
    }

    /// Bytes from `offset` on, at most `count`.
    pub fn read(&self, offset: u64, count: usize) -> Vec<u8> {
        match &self.source {
            Source::Memory(b) => {
                let start = (offset as usize).min(b.len());
                b[start..(start + count).min(b.len())].to_vec()
            }
            Source::File { path, .. } => {
                let mut cache = self.cache.lock().unwrap();
                if let Some((at, data)) = cache.as_ref()
                    && *at <= offset
                    && offset + count as u64 <= *at + data.len() as u64
                {
                    let s = (offset - at) as usize;
                    return data[s..s + count].to_vec();
                }
                // Read a bit around the visible part so scrolling stays smooth.
                let at = offset.saturating_sub(64 * 1024);
                let want = count + 128 * 1024;
                let mut data = Vec::new();
                if let Ok(mut f) = std::fs::File::open(path)
                    && f.seek(SeekFrom::Start(at)).is_ok()
                {
                    let _ = f.take(want as u64).read_to_end(&mut data);
                }
                let s = ((offset - at) as usize).min(data.len());
                let out = data[s..(s + count).min(data.len())].to_vec();
                *cache = Some((at, data));
                out
            }
        }
    }
}

fn human(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} Bytes"),
        n if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.0),
        n if n < 1024 * 1024 * 1024 => format!("{:.1} MB", n as f64 / 1024.0 / 1024.0),
        n => format!("{:.2} GB", n as f64 / 1024.0 / 1024.0 / 1024.0),
    }
}

/// One row as text: offset, hex bytes (gap after eight), printable characters.
pub fn format_row(offset: u64, bytes: &[u8]) -> (String, String, String) {
    let mut hex = String::with_capacity(PER_ROW * 3 + 1);
    for i in 0..PER_ROW {
        if i == 8 {
            hex.push(' ');
        }
        match bytes.get(i) {
            Some(b) => hex.push_str(&format!("{b:02X} ")),
            None => hex.push_str("   "),
        }
    }
    let ascii: String = bytes.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '·' }).collect();
    (format!("{offset:08X}"), hex, ascii)
}

/// Draws the view; `can_open_as_text` offers the switch to the text editor.
pub fn show(ui: &mut egui::Ui, rect: Rect, data: &HexData, theme: &Theme, id: egui::Id, can_open_as_text: bool) -> Action {
    let mut action = Action::None;
    ui.painter().rect_filled(rect, 0.0, theme.background);
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0));
    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    ui.painter().rect_filled(header, 0.0, theme.jumpbar_bg);
    ui.scope_builder(egui::UiBuilder::new().max_rect(header.shrink2(vec2(14.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.label(egui::RichText::new(format!("Binärdatei · {} · schreibgeschützt", human(data.len()))).color(theme.text_muted));
        if can_open_as_text {
            ui.add_space(8.0);
            if ui.small_button("Als Text öffnen").on_hover_text("Falls die Datei doch Text enthält").clicked() {
                action = Action::OpenAsText;
            }
        }
    });

    let font = FontId::monospace(13.0);
    let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 4.0;
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let rows = data.len().div_ceil(PER_ROW as u64) as usize;
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show_rows(ui, row_h, rows, |ui, range| {
            let first = range.start as u64 * PER_ROW as u64;
            let bytes = data.read(first, range.len() * PER_ROW);
            let width = char_w * (8.0 + 3.0 + PER_ROW as f32 * 3.0 + 1.0 + 3.0 + PER_ROW as f32) + 40.0;
            for (n, row) in range.enumerate() {
                let (r, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), row_h), Sense::hover());
                let chunk = &bytes[(n * PER_ROW).min(bytes.len())..((n + 1) * PER_ROW).min(bytes.len())];
                let (offset, hex, ascii) = format_row(row as u64 * PER_ROW as u64, chunk);
                let y = r.center().y;
                let x = r.left() + 14.0;
                let painter = ui.painter();
                painter.text(pos2(x, y), Align2::LEFT_CENTER, offset, font.clone(), theme.gutter_fg);
                let hx = x + char_w * 11.0;
                painter.text(pos2(hx, y), Align2::LEFT_CENTER, hex, font.clone(), theme.foreground);
                let ax = hx + char_w * (PER_ROW as f32 * 3.0 + 3.0);
                painter.text(pos2(ax, y), Align2::LEFT_CENTER, ascii, font.clone(), theme.text_muted);
            }
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_reading() {
        let (o, h, a) = format_row(0x20, b"Hi\x00\x7f\xffABC");
        assert_eq!(o, "00000020");
        assert!(h.starts_with("48 69 00 7F FF 41 42 43  "));
        assert_eq!(a, "Hi···ABC");

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let bytes: Vec<u8> = (0..=255u8).cycle().take(300_000).collect();
        std::fs::write(tmp.path(), &bytes).unwrap();
        let data = HexData::file(tmp.path().to_path_buf(), bytes.len() as u64);
        assert_eq!(data.read(250_000, 16), bytes[250_000..250_016]);
        // From the cache, and clipped at the end.
        assert_eq!(data.read(250_100, 16), bytes[250_100..250_116]);
        assert_eq!(data.read(299_990, 32), bytes[299_990..]);
        let mem = HexData::memory(bytes.clone());
        assert_eq!(mem.read(10, 4), bytes[10..14]);
    }
}
