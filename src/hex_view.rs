//! Hex view and editor for binary files: offset, 16 bytes in hex and as text.
//! Bytes are overwritten, or inserted and deleted (files held in memory).
//! Large local files are read on demand; saving writes only the changed bytes
//! into the file, or the whole file once its size changed.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use eframe::egui::{self, Align2, Event, FontId, Key, Rect, Sense, pos2, vec2};

use crate::theme::Theme;

const PER_ROW: usize = 16;

/// Where the original bytes come from.
pub enum Source {
    Memory,
    /// A local file read in pieces.
    File { path: PathBuf, len: u64 },
}

/// One undoable change.
#[derive(Clone, Debug)]
enum Op {
    Set { offset: u64, old: u8, new: u8 },
    Insert { offset: u64, bytes: Vec<u8> },
    Delete { offset: u64, bytes: Vec<u8> },
}

struct State {
    /// Original bytes (memory source).
    base: Vec<u8>,
    /// Unsaved changes: offset -> new value.
    edits: BTreeMap<u64, u8>,
    undo: Vec<Op>,
    redo: Vec<Op>,
    /// Inserts and deletes since the last save (undo counts back); the file
    /// then is written as a whole.
    resized: isize,
    /// Typing inserts bytes instead of overwriting them (Insert key).
    insert_mode: bool,
    /// Search field in the header.
    search: String,
    search_focus: bool,
    /// Take the keyboard focus next frame (after a dialog closed).
    want_focus: bool,
    cursor: u64,
    /// Next hex digit goes into the low half of the byte.
    low_nibble: bool,
    /// Typing goes to the character column.
    text_side: bool,
    scroll_to_cursor: bool,
}

pub struct HexData {
    pub source: Source,
    state: Mutex<State>,
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
    fn with(source: Source, base: Vec<u8>) -> Self {
        let state = State {
            base,
            edits: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            resized: 0,
            insert_mode: false,
            search: String::new(),
            search_focus: false,
            want_focus: false,
            cursor: 0,
            low_nibble: false,
            text_side: false,
            scroll_to_cursor: false,
        };
        Self { source, state: Mutex::new(state), cache: Mutex::new(None) }
    }

    pub fn memory(bytes: Vec<u8>) -> Self {
        Self::with(Source::Memory, bytes)
    }

    pub fn file(path: PathBuf, len: u64) -> Self {
        Self::with(Source::File { path, len }, Vec::new())
    }

    pub fn len(&self) -> u64 {
        self.len_of(&self.state.lock().unwrap())
    }

    /// The length while the state is locked already (`len` would deadlock).
    fn len_of(&self, state: &State) -> u64 {
        match &self.source {
            Source::Memory => state.base.len() as u64,
            Source::File { len, .. } => *len,
        }
    }

    /// Original bytes, without the unsaved changes.
    fn read_base(&self, state: &State, offset: u64, count: usize) -> Vec<u8> {
        match &self.source {
            Source::Memory => {
                let b = &state.base;
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

    /// Bytes from `offset` on (at most `count`), with the unsaved changes.
    pub fn read(&self, offset: u64, count: usize) -> Vec<u8> {
        let state = self.state.lock().unwrap();
        let mut out = self.read_base(&state, offset, count);
        for (&o, &v) in state.edits.range(offset..offset + out.len() as u64) {
            out[(o - offset) as usize] = v;
        }
        out
    }

    pub fn is_dirty(&self) -> bool {
        let s = self.state.lock().unwrap();
        !s.edits.is_empty() || s.resized != 0
    }

    /// Inserting and deleting needs all bytes in memory.
    pub fn can_resize(&self) -> bool {
        matches!(self.source, Source::Memory)
    }

    pub fn is_edited(&self, offset: u64) -> bool {
        self.state.lock().unwrap().edits.contains_key(&offset)
    }

    fn apply(&self, state: &mut State, offset: u64, value: u8) {
        if self.read_base(state, offset, 1).first() == Some(&value) {
            state.edits.remove(&offset);
        } else {
            state.edits.insert(offset, value);
        }
    }

    /// Overwrites one byte (undoable).
    pub fn set(&self, offset: u64, value: u8) {
        if offset >= self.len() {
            return;
        }
        let old = self.read(offset, 1)[0];
        if old == value {
            return;
        }
        let mut state = self.state.lock().unwrap();
        state.undo.push(Op::Set { offset, old, new: value });
        state.redo.clear();
        self.apply(&mut state, offset, value);
    }

    /// Inserts bytes before `offset` (memory only; undoable).
    pub fn insert(&self, offset: u64, bytes: &[u8]) {
        if !self.can_resize() || offset > self.len() || bytes.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap();
        Self::splice(&mut state, offset, 0, bytes);
        state.undo.push(Op::Insert { offset, bytes: bytes.to_vec() });
        state.redo.clear();
        state.resized += 1;
    }

    /// Deletes `count` bytes from `offset` on (memory only; undoable).
    pub fn delete(&self, offset: u64, count: usize) {
        let len = self.len();
        if !self.can_resize() || offset >= len || count == 0 {
            return;
        }
        let count = count.min((len - offset) as usize);
        let mut state = self.state.lock().unwrap();
        let removed = Self::splice(&mut state, offset, count, &[]);
        state.undo.push(Op::Delete { offset, bytes: removed });
        state.redo.clear();
        state.resized += 1;
    }

    /// Replaces `count` bytes at `offset` with `bytes` in the memory copy;
    /// pending overwrites are taken into it first. Returns the removed bytes.
    fn splice(state: &mut State, offset: u64, count: usize, bytes: &[u8]) -> Vec<u8> {
        let edits: Vec<(u64, u8)> = state.edits.iter().map(|(&o, &v)| (o, v)).collect();
        Self::commit(state, &edits);
        let at = offset as usize;
        state.base.splice(at..at + count, bytes.iter().copied()).collect()
    }

    pub fn undo(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(op) = state.undo.pop() else { return false };
        let at = match &op {
            Op::Set { offset, old, .. } => {
                self.apply(&mut state, *offset, *old);
                *offset
            }
            Op::Insert { offset, bytes } => {
                Self::splice(&mut state, *offset, bytes.len(), &[]);
                state.resized -= 1;
                *offset
            }
            Op::Delete { offset, bytes } => {
                Self::splice(&mut state, *offset, 0, bytes);
                state.resized -= 1;
                *offset
            }
        };
        state.redo.push(op);
        state.cursor = at;
        state.scroll_to_cursor = true;
        true
    }

    pub fn redo(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(op) = state.redo.pop() else { return false };
        let at = match &op {
            Op::Set { offset, new, .. } => {
                self.apply(&mut state, *offset, *new);
                *offset
            }
            Op::Insert { offset, bytes } => {
                Self::splice(&mut state, *offset, 0, bytes);
                state.resized += 1;
                *offset
            }
            Op::Delete { offset, bytes } => {
                Self::splice(&mut state, *offset, bytes.len(), &[]);
                state.resized += 1;
                *offset
            }
        };
        state.undo.push(op);
        state.cursor = at;
        state.scroll_to_cursor = true;
        true
    }

    /// Moves the cursor to `offset` (clamped) and scrolls there.
    pub fn go_to(&self, offset: u64) {
        let last = self.len().saturating_sub(1);
        let mut s = self.state.lock().unwrap();
        s.cursor = offset.min(last);
        s.low_nibble = false;
        s.scroll_to_cursor = true;
    }

    /// Puts the keyboard into the search field (Ctrl+F).
    /// Gives the bytes the keyboard focus (e.g. after "go to offset").
    pub fn focus(&self) {
        self.state.lock().unwrap().want_focus = true;
    }

    pub fn focus_search(&self) {
        self.state.lock().unwrap().search_focus = true;
    }

    /// Next (or previous) place of `needle` after (before) `from`, wrapping around.
    pub fn find(&self, needle: &[u8], from: u64, forward: bool) -> Option<u64> {
        let len = self.len();
        if needle.is_empty() || needle.len() as u64 > len {
            return None;
        }
        // In pieces, so large files are not read at once.
        const CHUNK: u64 = 1 << 20;
        let n = needle.len() as u64;
        let scan = |start: u64, end: u64| -> Option<u64> {
            let mut a = start;
            while a + n <= end {
                let b = (a + CHUNK + n - 1).min(end);
                let buf = self.read(a, (b - a) as usize);
                if let Some(i) = buf.windows(needle.len()).position(|w| w == needle) {
                    return Some(a + i as u64);
                }
                a += CHUNK;
            }
            None
        };
        let scan_back = |start: u64, end: u64| -> Option<u64> {
            let mut b = end;
            while b >= start + n {
                let a = b.saturating_sub(CHUNK + n - 1).max(start);
                let buf = self.read(a, (b - a) as usize);
                if let Some(i) = buf.windows(needle.len()).rposition(|w| w == needle) {
                    return Some(a + i as u64);
                }
                if a == start {
                    break;
                }
                b = a + n - 1;
            }
            None
        };
        if forward {
            scan(from + 1, len).or_else(|| scan(0, (from + n).min(len)))
        } else {
            scan_back(0, (from + n - 1).min(len)).or_else(|| scan_back(from, len))
        }
    }

    /// All bytes with the changes (for uploading; memory source).
    pub fn bytes(&self) -> Vec<u8> {
        self.read(0, self.len() as usize)
    }

    /// Writes the changed bytes into the file at `path` and makes them the new original.
    pub fn save_in_place(&self, path: &Path) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.resized != 0 {
            // The size changed: the whole file is written anew.
            let edits: Vec<(u64, u8)> = state.edits.iter().map(|(&o, &v)| (o, v)).collect();
            Self::commit(&mut state, &edits);
            crate::safe_write::write(path, &state.base)?;
            state.resized = 0;
            return Ok(());
        }
        let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
        // Consecutive changes are written together.
        let edits: Vec<(u64, u8)> = state.edits.iter().map(|(&o, &v)| (o, v)).collect();
        let mut i = 0;
        while i < edits.len() {
            let start = edits[i].0;
            let mut run = vec![edits[i].1];
            while i + 1 < edits.len() && edits[i + 1].0 == edits[i].0 + 1 {
                i += 1;
                run.push(edits[i].1);
            }
            file.seek(SeekFrom::Start(start))?;
            file.write_all(&run)?;
            i += 1;
        }
        file.flush()?;
        Self::commit(&mut state, &edits);
        *self.cache.lock().unwrap() = None;
        Ok(())
    }

    /// The bytes `saved` reached the server: they are the new original.
    pub fn saved(&self, saved: &[u8]) {
        let mut state = self.state.lock().unwrap();
        // Changes made while uploading stay unsaved.
        let edits: Vec<(u64, u8)> = state.edits.iter().map(|(&o, &v)| (o, v)).collect();
        let mut current = state.base.clone();
        for (o, v) in edits {
            if let Some(x) = current.get_mut(o as usize) {
                *x = v;
            }
        }
        if current == saved {
            state.resized = 0;
        }
        state.base = saved.to_vec();
        state.edits.retain(|&o, v| saved.get(o as usize) != Some(v));
    }

    fn commit(state: &mut State, edits: &[(u64, u8)]) {
        for &(o, v) in edits {
            if let Some(b) = state.base.get_mut(o as usize) {
                *b = v;
            }
        }
        state.edits.clear();
    }
}

fn human(n: u64) -> String {
    match n {
        n if n < 1024 => t!("{n} Bytes", n = n),
        n if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.0),
        n if n < 1024 * 1024 * 1024 => format!("{:.1} MB", n as f64 / 1024.0 / 1024.0),
        n => format!("{:.2} GB", n as f64 / 1024.0 / 1024.0 / 1024.0),
    }
}

fn s_not_found() -> String {
    t!("Nicht gefunden").to_string()
}

/// What to search for: hex bytes ("DE AD", "dead"), else the text itself
/// (in quotes to search for text that looks like hex).
pub fn parse_needle(input: &str) -> Vec<u8> {
    let t = input.trim();
    if let Some(inner) = t.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return inner.as_bytes().to_vec();
    }
    let digits: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    let is_hex = !digits.is_empty() && digits.len() % 2 == 0 && digits.chars().all(|c| c.is_ascii_hexdigit());
    if is_hex {
        (0..digits.len()).step_by(2).filter_map(|i| u8::from_str_radix(&digits[i..i + 2], 16).ok()).collect()
    } else {
        t.as_bytes().to_vec()
    }
}

/// An offset as typed: "0x1F00", "1F00h" or "$1F00" are hex, plain digits decimal.
pub fn parse_offset(input: &str) -> Option<u64> {
    let t = input.trim().replace(['_', ' ', '.'], "");
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).or_else(|| t.strip_prefix('$')).or_else(|| t.strip_suffix('h')).or_else(|| t.strip_suffix('H')) {
        return u64::from_str_radix(h, 16).ok();
    }
    if t.chars().any(|c| c.is_ascii_hexdigit() && !c.is_ascii_digit()) {
        return u64::from_str_radix(&t, 16).ok();
    }
    t.parse().ok()
}

/// Column (in characters) of byte `i` of a row in the hex part: gap after eight.
fn hex_col(i: usize) -> f32 {
    (i * 3 + if i >= 8 { 1 } else { 0 }) as f32
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

/// Keyboard input for the editor: moving and overwriting.
fn handle_keys(ui: &egui::Ui, data: &HexData, page_rows: u64) {
    let len = data.len();
    if len == 0 {
        return;
    }
    let events = ui.input(|i| i.events.clone());
    for ev in events {
        let mut state = data.state.lock().unwrap();
        let cur = state.cursor;
        let mut moved = None;
        match ev {
            Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.alt => match key {
                Key::ArrowLeft => moved = Some(cur.saturating_sub(1)),
                Key::ArrowRight => moved = Some(cur + 1),
                Key::ArrowUp => moved = Some(cur.saturating_sub(PER_ROW as u64)),
                Key::ArrowDown => moved = Some(cur + PER_ROW as u64),
                Key::PageUp => moved = Some(cur.saturating_sub(PER_ROW as u64 * page_rows)),
                Key::PageDown => moved = Some(cur + PER_ROW as u64 * page_rows),
                Key::Home => moved = Some(cur - cur % PER_ROW as u64),
                Key::End => moved = Some(cur - cur % PER_ROW as u64 + PER_ROW as u64 - 1),
                Key::Tab => {
                    state.text_side = !state.text_side;
                    state.low_nibble = false;
                }
                Key::Insert if data.can_resize() => {
                    state.insert_mode = !state.insert_mode;
                    state.low_nibble = false;
                }
                Key::Delete if data.can_resize() => {
                    drop(state);
                    data.delete(cur, 1);
                    let mut state = data.state.lock().unwrap();
                    state.cursor = cur.min(data.len_of(&state).saturating_sub(1));
                    state.low_nibble = false;
                    state.scroll_to_cursor = true;
                    continue;
                }
                Key::Backspace if data.can_resize() && cur > 0 => {
                    drop(state);
                    data.delete(cur - 1, 1);
                    let mut state = data.state.lock().unwrap();
                    state.cursor = cur - 1;
                    state.low_nibble = false;
                    state.scroll_to_cursor = true;
                    continue;
                }
                _ => {}
            },
            Event::Key { key: Key::Z, pressed: true, modifiers, .. } if modifiers.command => {
                drop(state);
                if modifiers.shift { data.redo() } else { data.undo() };
                continue;
            }
            Event::Key { key: Key::Y, pressed: true, modifiers, .. } if modifiers.command => {
                drop(state);
                data.redo();
                continue;
            }
            Event::Key { key: Key::Home, pressed: true, modifiers, .. } if modifiers.command => moved = Some(0),
            Event::Key { key: Key::End, pressed: true, modifiers, .. } if modifiers.command => moved = Some(len - 1),
            Event::Text(text) => {
                for c in text.chars() {
                    let cur = state.cursor;
                    let inserting = state.insert_mode && data.can_resize();
                    if state.text_side {
                        if !c.is_ascii() || c.is_ascii_control() {
                            continue;
                        }
                        drop(state);
                        if inserting {
                            data.insert(cur, &[c as u8]);
                        } else {
                            data.set(cur, c as u8);
                        }
                        state = data.state.lock().unwrap();
                        state.cursor = (cur + 1).min(data.len_of(&state) - 1);
                    } else if inserting && !state.low_nibble && let Some(d) = c.to_digit(16) {
                        // First digit of a new byte; the second one fills it in.
                        drop(state);
                        data.insert(cur, &[(d as u8) << 4]);
                        state = data.state.lock().unwrap();
                        state.low_nibble = true;
                    } else if let Some(d) = c.to_digit(16) {
                        let old = data.read_base(&state, cur, 1).first().copied().unwrap_or(0);
                        let current = state.edits.get(&cur).copied().unwrap_or(old);
                        let value = if state.low_nibble { (current & 0xF0) | d as u8 } else { ((d as u8) << 4) | (current & 0x0F) };
                        let low = state.low_nibble;
                        drop(state);
                        data.set(cur, value);
                        state = data.state.lock().unwrap();
                        state.low_nibble = !low;
                        if low {
                            state.cursor = (cur + 1).min(data.len_of(&state) - 1);
                        }
                    }
                    state.scroll_to_cursor = true;
                }
            }
            _ => {}
        }
        if let Some(to) = moved {
            state.cursor = to.min(len - 1);
            state.low_nibble = false;
            state.scroll_to_cursor = true;
        }
    }
}

/// Draws the view; `can_open_as_text` offers the switch to the text editor.
pub fn show(ui: &mut egui::Ui, rect: Rect, data: &HexData, theme: &Theme, id: egui::Id, can_open_as_text: bool) -> Action {
    let mut action = Action::None;
    ui.painter().rect_filled(rect, 0.0, theme.background);
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0));
    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    ui.painter().rect_filled(header, 0.0, theme.jumpbar_bg);
    let cursor = data.state.lock().unwrap().cursor;
    ui.scope_builder(egui::UiBuilder::new().max_rect(header.shrink2(vec2(14.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.label(egui::RichText::new(t!("Binärdatei · {human} · Offset {cursor} ({cursor2})", human = human(data.len()), cursor = format!("{:08X}", cursor), cursor2 = cursor)).color(theme.text_muted));
        if can_open_as_text && !data.is_dirty() {
            ui.add_space(8.0);
            if ui.small_button(t!("Als Text öffnen")).on_hover_text(t!("Falls die Datei doch Text enthält")).clicked() {
                action = Action::OpenAsText;
            }
        }
        ui.add_space(12.0);
        let mut s = data.state.lock().unwrap();
        let mode = if s.insert_mode { t!("Einfügen") } else { t!("Überschreiben") };
        let hint = if data.can_resize() {
            t!("Einfg wechselt zwischen Überschreiben und Einfügen, Entf/Rück löschen Bytes · Tab wechselt die Spalte")
        } else {
            t!("Datei zu groß zum Einfügen oder Löschen – Bytes lassen sich überschreiben · Tab wechselt die Spalte")
        };
        if ui.add_enabled(data.can_resize(), egui::Button::new(egui::RichText::new(mode).small())).on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
            s.insert_mode = !s.insert_mode;
            s.low_nibble = false;
        }
        ui.add_space(12.0);
        let field = ui.add(egui::TextEdit::singleline(&mut s.search).id(id.with("search")).hint_text(t!("Suchen: DE AD BE EF oder Text")).desired_width(220.0));
        if std::mem::take(&mut s.search_focus) {
            field.request_focus();
        }
        let mut step = None;
        if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            step = Some(!ui.input(|i| i.modifiers.shift));
            field.request_focus();
        }
        if ui.small_button("▲").on_hover_text(t!("Vorheriger (Umschalt+Enter)")).clicked() {
            step = Some(false);
        }
        if ui.small_button("▼").on_hover_text(t!("Nächster (Enter)")).clicked() {
            step = Some(true);
        }
        let needle = parse_needle(&s.search);
        drop(s);
        let note_id = id.with("search-note");
        if let Some(forward) = step {
            let found = data.find(&needle, cursor, forward);
            match found {
                Some(at) => {
                    data.go_to(at);
                    ui.data_mut(|d| d.remove::<String>(note_id));
                }
                None => {
                    ui.data_mut(|d| d.insert_temp(note_id, s_not_found()));
                }
            }
        }
        if let Some(note) = ui.data(|d| d.get_temp::<String>(note_id)) {
            ui.label(egui::RichText::new(note).small().color(theme.danger));
        }
    });

    let font = FontId::monospace(13.0);
    let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 4.0;
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let rows = data.len().div_ceil(PER_ROW as u64) as usize;
    let page_rows = ((body.height() / row_h) as u64).saturating_sub(1).max(1);

    // The whole area takes keyboard focus; clicks place the cursor.
    let focus_id = id.with("focus");
    // One widget over the whole body takes clicks and keyboard focus (rows only
    // look at where it was clicked); a click on another widget would drop the focus.
    let focus_resp = ui.interact(body, focus_id, Sense::click());
    if focus_resp.clicked() || std::mem::take(&mut data.state.lock().unwrap().want_focus) {
        focus_resp.request_focus();
    }
    let click_at = focus_resp.clicked().then(|| focus_resp.interact_pointer_pos()).flatten();
    let focused = ui.memory(|m| m.has_focus(focus_id));
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(focus_id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
        handle_keys(ui, data, page_rows);
    }
    let (state_cursor, text_side, low, scroll) = {
        let mut s = data.state.lock().unwrap();
        let scroll = std::mem::take(&mut s.scroll_to_cursor);
        (s.cursor, s.text_side, s.low_nibble, scroll)
    };
    let x0 = body.left() + 14.0;
    let hx = x0 + char_w * 11.0;
    let ax = hx + char_w * (PER_ROW as f32 * 3.0 + 3.0);

    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        let mut area = egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]);
        if scroll {
            // Keep the cursor row in view.
            let row = (state_cursor / PER_ROW as u64) as f32;
            let current = ui.ctx().memory(|m| m.data.get_temp::<f32>(id.with("offset"))).unwrap_or(0.0);
            let top = row * row_h;
            let visible = body.height() - row_h * 2.0;
            if top < current {
                area = area.vertical_scroll_offset(top);
            } else if top > current + visible {
                area = area.vertical_scroll_offset(top - visible);
            }
        }
        let out = area.show_rows(ui, row_h, rows, |ui, range| {
            let first = range.start as u64 * PER_ROW as u64;
            let bytes = data.read(first, range.len() * PER_ROW);
            let width = (ax - body.left()) + char_w * PER_ROW as f32 + 40.0;
            for (n, row) in range.enumerate() {
                let (r, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), row_h), Sense::hover());
                let row_offset = row as u64 * PER_ROW as u64;
                let chunk = &bytes[(n * PER_ROW).min(bytes.len())..((n + 1) * PER_ROW).min(bytes.len())];
                let shift = r.left() - body.left();
                let (rx_hex, rx_text) = (hx + shift, ax + shift);
                // Clicking a byte moves the cursor there.
                if let Some(p) = click_at.filter(|p| r.y_range().contains(p.y)) {
                    let mut s = data.state.lock().unwrap();
                    if p.x >= rx_text {
                        let i = ((p.x - rx_text) / char_w) as usize;
                        if i < chunk.len() {
                            s.cursor = row_offset + i as u64;
                            s.text_side = true;
                        }
                    } else if p.x >= rx_hex {
                        let col = (p.x - rx_hex) / char_w;
                        let i = (0..PER_ROW).rev().find(|&i| col >= hex_col(i)).unwrap_or(0);
                        if i < chunk.len() {
                            s.cursor = row_offset + i as u64;
                            s.text_side = false;
                        }
                    }
                    s.low_nibble = false;
                }
                let y = r.center().y;
                let painter = ui.painter();
                let (offset, _, _) = format_row(row_offset, chunk);
                painter.text(pos2(x0 + shift, y), Align2::LEFT_CENTER, offset, font.clone(), theme.gutter_fg);
                for (i, &b) in chunk.iter().enumerate() {
                    let at = row_offset + i as u64;
                    let edited = data.is_edited(at);
                    let color = if edited { theme.modified } else { theme.foreground };
                    let cell_hex = Rect::from_min_size(pos2(rx_hex + hex_col(i) * char_w - 1.0, r.top() + 1.0), vec2(char_w * 2.0 + 2.0, row_h - 2.0));
                    let cell_text = Rect::from_min_size(pos2(rx_text + i as f32 * char_w, r.top() + 1.0), vec2(char_w, row_h - 2.0));
                    if at == state_cursor {
                        let (strong, weak) = if text_side { (cell_text, cell_hex) } else { (cell_hex, cell_text) };
                        let alpha = if focused { 0.35 } else { 0.15 };
                        painter.rect_filled(strong, 2.0, theme.accent.gamma_multiply(alpha));
                        painter.rect_stroke(weak, 2.0, egui::Stroke::new(1.0, theme.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
                        if focused && !text_side && low {
                            // Second digit comes next: underline it.
                            let x = cell_hex.left() + 1.0 + char_w;
                            painter.line_segment([pos2(x, cell_hex.bottom() - 1.0), pos2(x + char_w, cell_hex.bottom() - 1.0)], egui::Stroke::new(1.5, theme.accent));
                        }
                    }
                    painter.text(pos2(rx_hex + hex_col(i) * char_w, y), Align2::LEFT_CENTER, format!("{b:02X}"), font.clone(), color);
                    let ch = if (0x20..0x7f).contains(&b) { b as char } else { '·' };
                    let text_color = if edited { theme.modified } else { theme.text_muted };
                    painter.text(pos2(cell_text.left(), y), Align2::LEFT_CENTER, ch.to_string(), font.clone(), text_color);
                }
            }
        });
        ui.ctx().memory_mut(|m| m.data.insert_temp(id.with("offset"), out.state.clamped_offset().y));
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
        assert_eq!(data.read(250_100, 16), bytes[250_100..250_116]);
        assert_eq!(data.read(299_990, 32), bytes[299_990..]);
        let mem = HexData::memory(bytes.clone());
        assert_eq!(mem.read(10, 4), bytes[10..14]);
    }

    #[test]
    fn edit_undo_and_save_in_place() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let bytes: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
        std::fs::write(tmp.path(), &bytes).unwrap();
        let data = HexData::file(tmp.path().to_path_buf(), bytes.len() as u64);
        data.set(150_000, 0xAA);
        data.set(150_001, 0xBB);
        assert!(data.is_dirty());
        assert_eq!(data.read(150_000, 2), [0xAA, 0xBB]);
        // Undoing back to the original value leaves nothing to save.
        assert!(data.undo());
        assert!(data.undo());
        assert!(!data.is_dirty());
        assert!(data.redo());
        data.set(10, b'X');
        data.save_in_place(tmp.path()).unwrap();
        assert!(!data.is_dirty());
        let disk = std::fs::read(tmp.path()).unwrap();
        assert_eq!(disk.len(), bytes.len());
        assert_eq!((disk[10], disk[150_000], disk[150_001]), (b'X', 0xAA, bytes[150_001]));
        assert_eq!(data.read(150_000, 1), [0xAA]);
    }

    #[test]
    fn insert_delete_undo_and_save() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"ABCDEF").unwrap();
        let data = HexData::memory(b"ABCDEF".to_vec());
        data.set(0, b'a');
        data.insert(2, b"xy");
        assert_eq!(data.bytes(), b"aBxyCDEF");
        data.delete(5, 2);
        assert_eq!(data.bytes(), b"aBxyCF");
        assert!(data.undo());
        assert!(data.undo());
        assert_eq!(data.bytes(), b"aBCDEF");
        assert!(data.redo());
        data.save_in_place(tmp.path()).unwrap();
        assert!(!data.is_dirty());
        assert_eq!(std::fs::read(tmp.path()).unwrap(), b"aBxyCDEF");
        // Big files read in pieces keep their size.
        let file = HexData::file(tmp.path().to_path_buf(), 8);
        file.insert(0, b"z");
        assert_eq!(file.len(), 8);
    }

    #[test]
    fn find_and_parse() {
        let mut bytes = vec![0u8; 3_000_000];
        bytes[10..14].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        bytes[2_500_000..2_500_004].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let data = HexData::memory(bytes);
        let needle = parse_needle("de ad BE EF");
        assert_eq!(needle, [0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(data.find(&needle, 0, true), Some(10));
        assert_eq!(data.find(&needle, 10, true), Some(2_500_000));
        assert_eq!(data.find(&needle, 2_500_000, true), Some(10));
        assert_eq!(data.find(&needle, 2_500_000, false), Some(10));
        assert_eq!(data.find(&needle, 10, false), Some(2_500_000));
        assert_eq!(data.find(b"nope", 0, true), None);
        assert_eq!(parse_needle("Hallo"), b"Hallo");
        assert_eq!(parse_needle("\"cafe\""), b"cafe");
        assert_eq!(parse_offset("0x1F"), Some(31));
        assert_eq!(parse_offset("1Fh"), Some(31));
        assert_eq!(parse_offset("31"), Some(31));
        assert_eq!(parse_offset("ff"), Some(255));
        assert_eq!(parse_offset("x"), None);
    }

    #[test]
    fn remote_save_keeps_later_changes() {
        let data = HexData::memory(vec![0; 8]);
        data.set(1, 1);
        let uploaded = data.bytes();
        data.set(2, 2);
        data.saved(&uploaded);
        assert!(data.is_dirty());
        assert_eq!(data.bytes(), [0, 1, 2, 0, 0, 0, 0, 0]);
    }
}
