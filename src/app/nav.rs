//! Navigation history (Alt+←/→) and "Go to symbol" (Strg+R).

use eframe::egui::{self, Key, Modifiers, RichText};

use super::NebulaApp;
use crate::editor::document::Selection;

/// Lines the cursor has to move at once to count as a jump.
const JUMP_LINES: usize = 10;
const MAX_HISTORY: usize = 100;

#[derive(Clone, Copy, PartialEq)]
struct NavPos {
    doc_id: u64,
    pos: usize,
    line: usize,
}

#[derive(Default)]
pub(super) struct NavHistory {
    back: Vec<NavPos>,
    forward: Vec<NavPos>,
    last: Option<NavPos>,
    /// The current move comes from going back/forward (not a new jump).
    navigating: bool,
}

pub(super) struct SymbolSearch {
    pub query: String,
    pub selected: usize,
    pub focus: bool,
}

impl NebulaApp {
    fn current_pos(&self) -> Option<NavPos> {
        let doc = self.docs.get(self.active?)?;
        let pos = doc.primary_selection().head;
        Some(NavPos { doc_id: doc.id, pos, line: doc.char_to_line(pos) })
    }

    /// Records the previous position when the cursor jumped (other file or far away).
    pub(super) fn track_navigation(&mut self) {
        let Some(now) = self.current_pos() else { return };
        let nav = &mut self.nav;
        if let Some(last) = nav.last
            && (last.doc_id != now.doc_id || last.line.abs_diff(now.line) > JUMP_LINES)
        {
            if nav.navigating {
                nav.navigating = false;
            } else {
                nav.back.push(last);
                nav.forward.clear();
                if nav.back.len() > MAX_HISTORY {
                    nav.back.remove(0);
                }
            }
        }
        nav.last = Some(now);
    }

    /// Goes back (or forward) to an earlier position; closed files are skipped.
    pub(super) fn navigate(&mut self, forward: bool) {
        loop {
            let target = if forward { self.nav.forward.pop() } else { self.nav.back.pop() };
            let Some(target) = target else { return };
            let Some(i) = self.doc_index(target.doc_id) else { continue };
            if let Some(now) = self.current_pos() {
                if forward { self.nav.back.push(now) } else { self.nav.forward.push(now) }
            }
            self.nav.navigating = true;
            self.activate(i);
            let doc = &mut self.docs[i];
            let pos = target.pos.min(doc.len_chars());
            doc.set_selection(Selection::cursor(pos));
            doc.view.center_line = Some(doc.char_to_line(pos));
            self.focus_editor = true;
            return;
        }
    }

    pub(super) fn open_goto_symbol(&mut self) {
        if self.active.is_none() {
            return;
        }
        self.dialog = Some(super::Dialog::GotoSymbol(SymbolSearch { query: String::new(), selected: 0, focus: true }));
    }

    /// Searchable list of the active document's symbols; returns false when closed.
    pub(super) fn goto_symbol_dialog(&mut self, ctx: &egui::Context, state: &mut SymbolSearch) -> bool {
        let Some(i) = self.active else { return false };
        let list = self.doc_symbols(i);
        let query = state.query.to_lowercase();
        let mut results: Vec<(usize, i32)> = list
            .iter()
            .enumerate()
            .filter_map(|(n, s)| {
                let name = s.name.to_lowercase();
                if query.is_empty() {
                    return Some((n, 0));
                }
                let pos = name.find(&query)?;
                Some((n, if pos == 0 { 1000 } else { 500 - pos as i32 }))
            })
            .collect();
        results.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        results.truncate(200);
        let mut keep = true;
        let mut jump = None;
        let theme = self.theme.clone();
        super::modal(ctx, egui::Id::unique("goto-symbol")).show(ctx, |ui| {
            ui.set_width(520.0);
            let resp = ui.add(egui::TextEdit::singleline(&mut state.query).hint_text(t!("Symbol suchen…")).desired_width(f32::INFINITY));
            if std::mem::take(&mut state.focus) || resp.changed() {
                resp.request_focus();
            }
            if resp.changed() {
                state.selected = 0;
            }
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    state.selected = (state.selected + 1).min(results.len().saturating_sub(1));
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    state.selected = state.selected.saturating_sub(1);
                }
            });
            ui.add_space(4.0);
            if results.is_empty() {
                ui.label(RichText::new(if list.is_empty() { t!("Keine Symbole in dieser Datei") } else { t!("Kein passendes Symbol") }).color(theme.text_faint));
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (row, (n, _)) in results.iter().enumerate() {
                    let s = &list[*n];
                    let mut job = egui::text::LayoutJob::default();
                    let indent = if query.is_empty() { "    ".repeat(s.depth.min(6)) } else { String::new() };
                    job.append(&format!("{indent}{} ", s.kind.badge()), 0.0, egui::TextFormat::simple(egui::FontId::monospace(12.0), theme.accent));
                    job.append(&s.name, 0.0, egui::TextFormat::simple(egui::FontId::proportional(14.0), ui.visuals().text_color()));
                    job.append(&format!("   {}", s.line + 1), 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.5), theme.text_faint));
                    let r = ui.add(egui::Button::selectable(row == state.selected, job).min_size(egui::vec2(500.0, 26.0)));
                    if row == state.selected {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        jump = Some(s.line);
                    }
                }
            });
            if ui.input(|i| i.key_pressed(Key::Enter))
                && let Some((n, _)) = results.get(state.selected)
            {
                jump = Some(list[*n].line);
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                keep = false;
            }
        });
        if let Some(line) = jump {
            self.jump_to_line(i, line);
            return false;
        }
        keep
    }
}
