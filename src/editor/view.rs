//! The code editor widget.
//!
//! Only the visible lines (and for long lines only the visible columns) are
//! laid out and painted each frame, so cost is independent of file size.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, CursorIcon, Event, EventFilter, FontId, Id, Key, Modifiers, Pos2, Rect,
    ScrollArea, Sense, Stroke, TextFormat, Ui, pos2, text::LayoutJob, vec2,
};

use super::commands as cmd;
use super::complete;
use super::tags;
use super::folding::{self, Rows};
use super::document::{Document, Selection};
use crate::highlight::Span;
use crate::settings::EditorSettings;
use crate::theme::Theme;

const TEXT_PADDING: f32 = 12.0;
const TOP_PADDING: f32 = 6.0;
const DOUBLE_CLICK: f64 = 0.4;

#[derive(Default)]
pub struct EditorOutput {
    /// Text to put on the clipboard.
    pub copied: Option<String>,
    pub has_focus: bool,
    /// The editor got the keyboard focus this frame.
    pub gained_focus: bool,
}

/// Mouse state kept in egui's temp memory between frames.
#[derive(Clone, Copy, Default)]
struct PointerState {
    dragging: bool,
    last_click_time: f64,
    last_click_pos: usize,
    click_count: u32,
    /// Word or line range the drag started from (for double/triple click drags).
    origin: Option<(usize, usize)>,
    /// Alt+drag: block selection from this cell.
    block: Option<BlockStart>,
    /// Line and time of the last click on a line number (double click: bookmark).
    gutter_click: Option<(usize, f64)>,
}

#[derive(Clone, Copy)]
struct BlockStart {
    line: usize,
    col: usize,
}

pub fn editor_id(doc_id: u64, pane: u8) -> Id {
    Id::unique(("editor", doc_id, pane))
}

pub struct EditorView<'a> {
    pub doc: &'a mut Document,
    pub settings: &'a EditorSettings,
    pub theme: &'a Theme,
    /// Find matches (char ranges, sorted).
    pub matches: &'a [Range<usize>],
    pub current_match: Option<usize>,
    /// 0: main editor, 1: second half of the split view.
    pub pane: u8,
    /// Scroll to the cursor after edits (false for the unfocused copy of a document shown twice).
    pub follow: bool,
}

struct Metrics {
    row_h: f32,
    char_w: f32,
    gutter_w: f32,
    font: FontId,
}

impl EditorView<'_> {
    pub fn show(self, ui: &mut Ui) -> EditorOutput {
        let EditorView {
            doc,
            settings,
            theme,
            matches,
            current_match,
            pane,
            follow,
        } = self;
        let id = editor_id(doc.id, pane);
        let font = FontId::monospace(settings.font_size);
        let (glyph_h, char_w) =
            ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, 'M')));
        let row_h = (glyph_h * settings.line_height.clamp(1.0, 2.5)).round();
        let digits = doc.len_lines().to_string().len().max(3) as f32;
        let m = Metrics {
            row_h,
            char_w,
            // Fold marker, bookmark flag, numbers, Git bar.
            gutter_w: (digits * char_w + 44.0).round(),
            font,
        };
        let tab = settings.tab_size;

        let mut out = EditorOutput {
            copied: None,
            has_focus: false,
            gained_focus: false,
        };

        // The minimap takes a strip on the right (not in narrow panes).
        let full = ui.available_rect_before_wrap();
        let map_w = if settings.minimap && full.width() > 480.0 { MINIMAP_W } else { 0.0 };
        let map_rect = Rect::from_min_max(pos2(full.right() - map_w, full.top()), full.max);
        let mut text_ui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(full.min, pos2(full.right() - map_w, full.bottom()))));
        let ui = &mut text_ui;

        // Wrapped lines fill the editor width (scroll bars float over the text).
        let wrap = settings
            .word_wrap
            .then(|| ((ui.available_width() - m.gutter_w - TEXT_PADDING * 2.0 - 8.0) / m.char_w).floor().max(10.0) as usize);
        let content_w = match wrap {
            Some(_) => 0.0,
            None => m.gutter_w + TEXT_PADDING * 2.0 + (doc.max_line_columns(tab) as f32 + 4.0) * m.char_w,
        };

        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, theme.background);

        update_git_marks(doc);
        let rows = folding::rows(doc, tab, wrap);
        let mut area = ScrollArea::both().id_salt(("editor-scroll", doc.id, pane)).auto_shrink([false, false]);
        if let Some(y) = doc.view.scroll_request.take() {
            area = area.vertical_scroll_offset(y);
        }
        let scroll = area.show_viewport(ui, |ui, viewport| {
                let content_h = TOP_PADDING + rows.count() as f32 * m.row_h + (viewport.height() - m.row_h * 3.0).max(0.0);
                let size = vec2(content_w.max(viewport.width()), content_h.max(viewport.height()));
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let origin = rect.min + vec2(0.0, TOP_PADDING);
                let visible = Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size());
                let text_x0 = origin.x + m.gutter_w + TEXT_PADDING;

                let response = ui.interact(visible, id, Sense::click_and_drag());
                if response.hovered() {
                    let over_gutter = ui
                        .input(|i| i.pointer.hover_pos())
                        .is_some_and(|p| p.x < visible.left() + m.gutter_w);
                    if !over_gutter {
                        ui.ctx().set_cursor_icon(CursorIcon::Text);
                    }
                }

                // ---- mouse ----------------------------------------------------
                let hit_rows = rows.clone();
                let hit = move |doc: &Document, p: Pos2| -> usize {
                    let row = ((p.y - origin.y) / m.row_h).floor().max(0.0) as usize;
                    let row = row.min(hit_rows.count() - 1);
                    let col = ((p.x - text_x0) / m.char_w).round().max(0.0) as usize;
                    hit_rows.pos_at(doc, row, col, tab)
                };
                // Line and display column under the pointer, also beyond the line end (block selection).
                let cell_rows = rows.clone();
                let hit_ref = &hit;
                let shown = move |line: usize| !cell_rows.is_hidden(line);
                let cell = move |doc: &Document, p: Pos2| -> (usize, usize) {
                    let pos = hit_ref(doc, p);
                    let line = doc.char_to_line(pos);
                    let col = match wrap {
                        Some(_) => doc.column_of(pos, tab),
                        None => ((p.x - text_x0) / m.char_w).round().max(0.0) as usize,
                    };
                    (line, col)
                };
                handle_pointer(ui, &response, doc, id, &hit, &cell, &shown, visible, &m, tab);

                response.context_menu(|ui| {
                    if ui.button(t!("Ausschneiden")).clicked() {
                        out.copied = Some(cmd::cut(doc));
                        ui.close();
                    }
                    if ui.button(t!("Kopieren")).clicked() {
                        out.copied = Some(cmd::copy_text(doc).0);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(t!("Alles auswählen")).clicked() {
                        cmd::select_all(doc);
                        ui.close();
                    }
                    if ui.button(t!("Kommentar umschalten")).clicked() {
                        cmd::toggle_comment(doc);
                        ui.close();
                    }
                    ui.menu_button(t!("Text"), |ui| {
                        type Tool = fn(&mut Document);
                        let tools: [(&str, Tool); 9] = [
                            (t!("Zeilen sortieren (A–Z)"), |d| cmd::sort_lines(d, false)),
                            (t!("Zeilen sortieren (Z–A)"), |d| cmd::sort_lines(d, true)),
                            (t!("Doppelte Zeilen entfernen"), cmd::unique_lines),
                            (t!("Zeilen umkehren"), cmd::reverse_lines),
                            (t!("Zeilen verbinden"), cmd::join_lines),
                            ("GROSSBUCHSTABEN", |d| cmd::change_case(d, cmd::Case::Upper)),
                            ("kleinbuchstaben", |d| cmd::change_case(d, cmd::Case::Lower)),
                            (t!("Wörter Großschreiben"), |d| cmd::change_case(d, cmd::Case::Title)),
                            (t!("Mit Tag umschließen"), cmd::wrap_in_tag),
                        ];
                        for (label, tool) in tools {
                            if ui.button(label).clicked() {
                                tool(doc);
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button(t!("Block ein-/ausklappen")).clicked() {
                        folding::toggle_at_cursor(doc, tab);
                        ui.close();
                    }
                    if ui.button(t!("Alles einklappen")).clicked() {
                        folding::fold_all(doc, tab);
                        ui.close();
                    }
                    if ui.button(t!("Alles ausklappen")).clicked() {
                        folding::unfold_all(doc);
                        ui.close();
                    }
                });

                // ---- keyboard -------------------------------------------------
                let focused = response.has_focus();
                out.has_focus = focused;
                out.gained_focus = response.gained_focus();
                if focused {
                    ui.memory_mut(|mem| {
                        mem.set_focus_lock_filter(
                            id,
                            EventFilter {
                                tab: true,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                ..Default::default()
                            },
                        )
                    });
                    let page = ((viewport.height() / m.row_h) as isize - 2).max(1);
                    if let Some(copied) = handle_keyboard(ui, doc, settings, page, wrap) {
                        out.copied = Some(copied);
                    }
                }
                // A cursor that moved into a folded block opens it.
                let head_line = doc.char_to_line(doc.primary_selection().head);
                folding::unfold_containing(doc, head_line, tab);
                let rows = folding::rows(doc, tab, wrap);

                // ---- scrolling ------------------------------------------------
                // The first frame can have a tiny viewport; scrolling then would
                // leave the view shifted, so wait until it has its real size.
                let viewport_ready = viewport.height() >= m.row_h * 4.0;
                if !viewport_ready {
                    ui.ctx().request_repaint();
                } else if !follow {
                } else if let Some(line) = doc.view.center_line.take() {
                    let y = origin.y + rows.row_of_line(line) as f32 * m.row_h;
                    let r = Rect::from_min_size(pos2(visible.left(), y), vec2(1.0, m.row_h));
                    ui.scroll_to_rect(r, Some(Align::Center));
                    doc.view.scroll_to_cursor = false;
                } else if std::mem::take(&mut doc.view.scroll_to_cursor) {
                    let head = doc.primary_selection().head;
                    let (row, col) = rows.locate(doc, head, tab);
                    let x = text_x0 + col as f32 * m.char_w;
                    let y = origin.y + row as f32 * m.row_h;
                    let r = Rect::from_min_max(
                        pos2(x - m.gutter_w - TEXT_PADDING - m.char_w * 2.0, y - m.row_h),
                        pos2(x + m.char_w * 4.0, y + m.row_h * 2.0),
                    );
                    ui.scroll_to_rect(r, None);
                }

                paint(
                    ui, doc, settings, theme, matches, current_match, &m, origin, visible, focused, &rows,
                );

                show_hover(ui, &response, doc, theme, id, &hit, visible, &m);
                super::colors::show(ui, doc, id);

                if !focused {
                    doc.view.completion = None;
                } else if doc.view.completion.is_some() {
                    let head = doc.primary_selection().head;
                    let start = doc.view.completion.as_ref().map_or(head, |c| c.start);
                    let (row, _) = rows.locate(doc, head, tab);
                    let (start_row, col) = rows.locate(doc, start, tab);
                    let col = if start_row == row { col } else { 0 };
                    let anchor = pos2(
                        (text_x0 + col as f32 * m.char_w - 26.0).max(visible.left()),
                        origin.y + (row + 1) as f32 * m.row_h + 2.0,
                    );
                    show_completion(ui, doc, theme, anchor, id);
                }
            });
        overview_ruler(ui, doc, theme, matches, scroll.inner_rect, tab, wrap);
        if map_w > 0.0 {
            minimap(ui, doc, theme, map_rect, &scroll, m.row_h, tab, wrap, id);
        }
        out
    }
}

/// Marks along the right edge for the whole document: find matches,
/// problems, Git changes, bookmarks and cursors.
fn overview_ruler(ui: &Ui, doc: &mut Document, theme: &Theme, matches: &[Range<usize>], area: Rect, tab: usize, wrap: Option<usize>) {
    let rows = folding::rows(doc, tab, wrap);
    let total = rows.count().max(1) as f32;
    // Only worth it when the document is longer than the view.
    let painter = ui.painter_at(area);
    let x1 = area.right() - 1.0;
    let mut used = std::collections::HashSet::new();
    let mut mark = |line: usize, x0: f32, color: Color32, h: f32| {
        let y = area.top() + rows.row_of_line(line) as f32 / total * area.height();
        let key = (x0 as i32, y as i32, color.to_array());
        if used.insert(key) {
            painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + h)), 1.0, color);
        }
    };
    let w = area.right();
    if let Some((_, marks)) = &doc.view.git_marks {
        let marks = marks.clone();
        for (line, &m) in marks.iter().enumerate() {
            let color = match m {
                1 => theme.success,
                2 => theme.modified,
                3 => theme.danger,
                _ => continue,
            };
            mark(line, w - 12.0, color.gamma_multiply(0.8), 3.0);
        }
    }
    for b in doc.view.bookmarks.clone() {
        mark(b, w - 12.0, theme.bookmark(), 3.0);
    }
    for m in matches.iter().take(20_000) {
        mark(doc.char_to_line(m.start), w - 8.0, theme.find_match.gamma_multiply(2.5), 2.0);
    }
    for d in &doc.view.diagnostics.clone() {
        mark(doc.char_to_line(d.range.start), w - 5.0, severity_color(theme, d.severity), 3.0);
    }
    for s in doc.selections.clone() {
        mark(doc.char_to_line(s.head), w - 12.0, theme.foreground.gamma_multiply(0.7), 2.0);
    }
}

/// Width of the minimap and the height of one line in it.
const MINIMAP_W: f32 = 90.0;
const MINIMAP_ROW: f32 = 2.0;

/// Shrunken view of the document to the right of the text: colored blocks
/// per token, the visible part highlighted; click or drag to scroll.
#[allow(clippy::too_many_arguments)]
fn minimap(
    ui: &Ui,
    doc: &mut Document,
    theme: &Theme,
    area: Rect,
    scroll: &egui::scroll_area::ScrollAreaOutput<()>,
    row_h: f32,
    tab: usize,
    wrap: Option<usize>,
    id: Id,
) {
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0.0, theme.background);
    painter.line_segment([area.left_top(), area.left_bottom()], Stroke::new(1.0, theme.separator.gamma_multiply(0.5)));
    let rows = folding::rows(doc, tab, wrap);
    let total = rows.count();
    if total == 0 {
        return;
    }
    let view_h = scroll.inner_rect.height();
    let max_scroll = (scroll.content_size.y - view_h).max(0.0);
    let offset = scroll.state.clamped_offset().y;
    let visible_rows = view_h / row_h;
    let first_visible = ((offset - TOP_PADDING) / row_h).max(0.0);
    // Long documents scroll inside the minimap along with the editor.
    let fits = area.height() / MINIMAP_ROW;
    let map_top = if total as f32 > fits && max_scroll > 0.0 { offset / max_scroll * (total as f32 - fits) } else { 0.0 };

    let first = map_top.floor() as usize;
    let last = ((map_top + fits).ceil() as usize + 1).min(total);
    let cw = 1.0; // one point per column
    let spans = doc.highlights.clone();
    for r in first..last {
        let row = rows.row(r);
        let y = area.top() + (r as f32 - map_top) * MINIMAP_ROW;
        let text = doc.line_text(row.line);
        // This row's part of the line (wrapped lines span several rows).
        let end = if r + 1 < total && rows.row(r + 1).line == row.line { rows.row(r + 1).byte } else { text.len() };
        let segment = text.get(row.byte..end.min(text.len())).unwrap_or("");
        let line_spans: &[crate::highlight::Span] = spans.get(row.line).map_or(&[], |v| v.as_slice());
        let mut col = row.col;
        let mut run: Option<(usize, usize, Color32)> = None; // (start col, end col, color)
        let flush = |run: &mut Option<(usize, usize, Color32)>| {
            if let Some((s, e, c)) = run.take() {
                let x0 = area.left() + 6.0 + s as f32 * cw;
                if x0 < area.right() {
                    let rect = Rect::from_min_max(pos2(x0, y), pos2((area.left() + 6.0 + e as f32 * cw).min(area.right()), y + MINIMAP_ROW - 0.5));
                    painter.rect_filled(rect, 0.0, c.gamma_multiply(0.7));
                }
            }
        };
        for (i, ch) in segment.char_indices() {
            let byte = row.byte + i;
            let width = if ch == '\t' { tab - col % tab } else { 1 };
            if ch.is_whitespace() {
                flush(&mut run);
            } else {
                let style = line_spans.iter().find(|s| (s.start as usize) <= byte && byte < s.end as usize);
                let color = style.map_or(theme.foreground, |s| theme.syntax_color(s.style));
                match &mut run {
                    Some((_, e, c)) if *c == color && *e == col => *e = col + width,
                    _ => {
                        flush(&mut run);
                        run = Some((col, col + width, color));
                    }
                }
            }
            col += width;
            if area.left() + 6.0 + col as f32 * cw > area.right() {
                break;
            }
        }
        flush(&mut run);
    }

    // The part shown in the editor.
    let view_rect = Rect::from_min_size(
        pos2(area.left() + 1.0, area.top() + (first_visible - map_top) * MINIMAP_ROW),
        vec2(area.width() - 1.0, (visible_rows * MINIMAP_ROW).max(4.0)),
    );
    let resp = ui.interact(area, id.with("minimap"), Sense::click_and_drag());
    let active = resp.hovered() || resp.dragged();
    painter.rect_filled(view_rect, 0.0, theme.foreground.gamma_multiply(if active { 0.12 } else { 0.07 }));
    // Click or drag: center the editor on that spot.
    if (resp.clicked() || resp.dragged())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let row = map_top + (p.y - area.top()) / MINIMAP_ROW;
        let target = ((row - visible_rows / 2.0) * row_h + TOP_PADDING).clamp(0.0, max_scroll);
        doc.view.scroll_request = Some(target);
        ui.ctx().request_repaint();
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_pointer(
    ui: &Ui,
    response: &egui::Response,
    doc: &mut Document,
    id: Id,
    hit: &dyn Fn(&Document, Pos2) -> usize,
    cell: &dyn Fn(&Document, Pos2) -> (usize, usize),
    shown: &dyn Fn(usize) -> bool,
    visible: Rect,
    m: &Metrics,
    tab: usize,
) {
    let state_id = id.with("pointer");
    let mut state: PointerState = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let (pressed, down, pos, mods, time) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.interact_pos(),
            i.modifiers,
            i.time,
        )
    });

    if pressed && response.hovered() {
        let Some(p) = pos else { return };
        // A click on the color bar under a CSS color opens the picker.
        if super::colors::click(doc, p) {
            return;
        }
        doc.view.completion = None;
        response.request_focus();
        let in_gutter = p.x < visible.left() + m.gutter_w;
        let at = hit(doc, p);
        if time - state.last_click_time < DOUBLE_CLICK && state.last_click_pos == at {
            state.click_count += 1;
        } else {
            state.click_count = 1;
        }
        state.last_click_time = time;
        state.last_click_pos = at;
        state.dragging = true;
        state.origin = None;
        state.block = None;

        if in_gutter && p.x < visible.left() + 17.0 {
            // Fold marker.
            let line = doc.char_to_line(at);
            if doc.view.folds.contains(&line) || folding::range_at(doc, line, tab).is_some() {
                folding::toggle(doc, line, tab);
                state.dragging = false;
                ui.data_mut(|d| d.insert_temp(state_id, state));
                return;
            }
        }
        let gutter_click = state.gutter_click.take();
        if in_gutter {
            let line = doc.char_to_line(at);
            // Double click on a line number sets or removes a bookmark.
            if let Some((last, t)) = gutter_click
                && last == line
                && time - t < DOUBLE_CLICK
            {
                if !doc.view.bookmarks.remove(&line) {
                    doc.view.bookmarks.insert(line);
                }
                state.dragging = false;
                ui.data_mut(|d| d.insert_temp(state_id, state));
                return;
            }
            state.gutter_click = Some((line, time));
            // Clicking a line number selects the line.
            let (s, e) = (doc.line_to_char(line), doc.line_to_char(line + 1));
            if mods.shift {
                let anchor = doc.primary_selection().anchor;
                doc.set_selection(Selection::new(anchor.min(s), e.max(anchor)));
            } else {
                doc.set_selection(Selection::new(s, e));
            }
            state.origin = Some((s, e));
            state.click_count = 3;
        } else if mods.command && !mods.shift && state.click_count == 1 {
            doc.set_selection(Selection::cursor(at));
            doc.view.definition_request = Some(at);
        } else if mods.alt {
            // Alt+click adds a cursor; dragging turns it into a block selection.
            let (line, col) = cell(doc, p);
            state.block = Some(BlockStart { line, col });
            doc.selections.push(Selection::cursor(at));
            doc.primary = doc.selections.len() - 1;
            doc.normalize_selections();
        } else if mods.shift {
            cmd::move_to(doc, at, true);
        } else {
            match state.click_count {
                2 => {
                    let r = cmd::word_at(doc, at).unwrap_or(at..at);
                    doc.set_selection(Selection::new(r.start, r.end));
                    state.origin = Some((r.start, r.end));
                }
                3.. => {
                    let line = doc.char_to_line(at);
                    let (s, e) = (doc.line_to_char(line), doc.line_to_char(line + 1));
                    doc.set_selection(Selection::new(s, e));
                    state.origin = Some((s, e));
                }
                _ => doc.set_selection(Selection::cursor(at)),
            }
        }
        doc.view.scroll_to_cursor = false;
    } else if state.dragging && down && state.block.is_some() {
        if let Some(p) = pos {
            let start = state.block.unwrap();
            let (line, col) = cell(doc, p);
            if (line, col) != (start.line, start.col) {
                // One selection per line between the start and the pointer.
                let mut sels: Vec<Selection> = Vec::new();
                let lines: Vec<usize> = if line >= start.line {
                    (start.line..=line).collect()
                } else {
                    (line..=start.line).rev().collect()
                };
                for l in lines.into_iter().filter(|&l| shown(l)) {
                    let anchor = doc.pos_at_column(l, start.col, tab);
                    let head = doc.pos_at_column(l, col, tab);
                    // Lines that end left of the block only get a selection if the block is a plain column of cursors.
                    if start.col != col && doc.column_of(anchor, tab) < start.col.min(col) {
                        continue;
                    }
                    sels.push(Selection::new(anchor, head));
                }
                if !sels.is_empty() {
                    doc.selections = sels;
                    doc.primary = doc.selections.len() - 1;
                    doc.normalize_selections();
                }
            }
            if !visible.shrink(4.0).contains(p) {
                let r = Rect::from_center_size(p, vec2(m.char_w * 2.0, m.row_h));
                ui.scroll_to_rect(r, None);
                ui.ctx().request_repaint();
            }
        }
    } else if state.dragging && down {
        if let Some(p) = pos {
            let at = hit(doc, p);
            let sel = doc.primary_selection();
            let new = match state.origin {
                Some((s, e)) => {
                    // Extend by words/lines from the original unit.
                    if at < s {
                        let start = if state.click_count >= 3 {
                            doc.line_to_char(doc.char_to_line(at))
                        } else {
                            cmd::word_at(doc, at).map_or(at, |r| r.start)
                        };
                        Selection::new(e, start)
                    } else {
                        let end = if state.click_count >= 3 {
                            doc.line_to_char(doc.char_to_line(at) + 1)
                        } else {
                            cmd::word_at(doc, at).map_or(at, |r| r.end)
                        };
                        Selection::new(s, end.max(e))
                    }
                }
                None => Selection::new(sel.anchor, at),
            };
            if new != sel {
                let primary = doc.primary;
                doc.selections[primary] = new;
                doc.normalize_selections();
            }
            // Auto-scroll while dragging outside the viewport.
            if !visible.shrink(4.0).contains(p) {
                let r = Rect::from_center_size(p, vec2(m.char_w * 2.0, m.row_h));
                ui.scroll_to_rect(r, None);
                ui.ctx().request_repaint();
            }
        }
    } else if !down {
        state.dragging = false;
    }
    ui.data_mut(|d| d.insert_temp(state_id, state));
}

/// Processes keyboard input; returns text for the clipboard on copy/cut.
fn handle_keyboard(
    ui: &Ui,
    doc: &mut Document,
    settings: &EditorSettings,
    page: isize,
    wrap: Option<usize>,
) -> Option<String> {
    let tab = settings.tab_size;
    let events = ui.input(|i| i.events.clone());
    let mut copied = None;
    // Renaming one tag of a pair renames its partner too.
    let mut link = None;
    for event in events {
        sync_tag_partner(doc, link.take());
        link = tag_link(doc);
        // What the completion popup should do after this event.
        let mut refresh = false;
        match event {
            Event::Text(text) | Event::Ime(egui::ImeEvent::Commit(text)) => {
                cmd::type_text(doc, &text, settings);
                let last = text.chars().last();
                refresh = last.is_some_and(|c| cmd::is_word_char(c) || c == '-');
                if let Some(c) = last.filter(|c| !c.is_whitespace())
                    && doc.selections.len() == 1
                {
                    let head = doc.primary_selection().head;
                    doc.view.completion_request = Some((head, (!cmd::is_word_char(c)).then_some(c)));
                } else {
                    doc.view.ext_items = None;
                }
            }
            Event::Copy => copied = Some(cmd::copy_text(doc).0),
            Event::Cut => copied = Some(cmd::cut(doc)),
            Event::Paste(text) => cmd::paste(doc, &text),
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if handle_completion_key(doc, key, modifiers, settings) {
                    continue;
                }
                if key == Key::Space && modifiers.command {
                    doc.view.completion = complete::compute(doc);
                    let head = doc.primary_selection().head;
                    doc.view.completion_request = Some((head, None));
                    continue;
                }
                if key == Key::F12 {
                    doc.view.definition_request = Some(doc.primary_selection().head);
                    continue;
                }
                let was_open = doc.view.completion.is_some();
                handle_key(doc, settings, key, modifiers, page, tab, wrap);
                // Letter keys arrive alongside their Text event; only keys that
                // move the cursor or edit should affect the popup.
                let affects_popup = modifiers.command
                    || modifiers.alt
                    || matches!(
                        key,
                        Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
                            | Key::Home | Key::End | Key::PageUp | Key::PageDown
                            | Key::Enter | Key::Tab | Key::Delete | Key::Backspace | Key::Escape
                    );
                if !affects_popup {
                    continue;
                }
                refresh = was_open && key == Key::Backspace;
            }
            Event::Key { pressed: false, .. } | Event::ModifiersChanged(_) => continue,
            Event::PointerMoved(_) | Event::MouseMoved(_) | Event::MouseWheel { .. } => continue,
            _ => continue,
        }
        doc.view.completion = if refresh { complete::compute(doc) } else { None };
    }
    sync_tag_partner(doc, link);
    copied
}

/// Tag pair at the cursor before an edit: (pair, cursor, version, length).
type TagLink = ((Range<usize>, Range<usize>), usize, u64, usize);

fn tag_link(doc: &Document) -> Option<TagLink> {
    if !tags::supported(doc.language) || doc.selections.len() != 1 {
        return None;
    }
    let sel = doc.primary_selection();
    let head = sel.head;
    // Only typing inside a name keeps the pair; a selection must lie within it.
    let pair = tags::pair_at(doc, head)?;
    let r = sel.range();
    let inside = |n: &Range<usize>| n.start <= r.start && r.end <= n.end;
    (inside(&pair.0) || inside(&pair.1)).then_some((pair, head, doc.version, doc.len_chars()))
}

fn sync_tag_partner(doc: &mut Document, link: Option<TagLink>) {
    let Some((pair, head, version, len)) = link else { return };
    if doc.version == version {
        return;
    }
    let delta = doc.len_chars() as isize - len as isize;
    if let Some(change) = tags::sync_partner(doc, &pair, head, delta) {
        doc.apply_linked(change);
    }
}

/// Keys that navigate the completion popup. Returns true if consumed.
fn handle_completion_key(doc: &mut Document, key: Key, mods: Modifiers, settings: &EditorSettings) -> bool {
    let Some(c) = doc.view.completion.as_mut() else {
        return false;
    };
    if mods.command || mods.alt {
        return false;
    }
    let n = c.items.len();
    match key {
        Key::ArrowDown => c.selected = (c.selected + 1) % n,
        Key::ArrowUp => c.selected = (c.selected + n - 1) % n,
        Key::PageDown => c.selected = (c.selected + 8).min(n - 1),
        Key::PageUp => c.selected = c.selected.saturating_sub(8),
        Key::Tab if settings.emmet && cmd::expand_emmet(doc, settings) => {
            doc.view.completion = None;
        }
        Key::Enter | Key::Tab => {
            let c = doc.view.completion.take().unwrap();
            complete::accept(doc, &c);
        }
        Key::Escape => doc.view.completion = None,
        _ => return false,
    }
    true
}

/// Draws the completion popup below the primary cursor.
fn show_completion(ui: &Ui, doc: &mut Document, theme: &Theme, anchor: Pos2, editor: Id) {
    let Some(c) = doc.view.completion.as_ref() else { return };
    let mut clicked = None;
    egui::Area::new(editor.with("completion"))
        .fixed_pos(anchor)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(4.0)
                .show(ui, |ui| {
                    ui.set_min_width(220.0);
                    let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .min_scrolled_height((c.items.len() as f32 * row_h).min(240.0))
                        .show(ui, |ui| {
                        for (i, item) in c.items.iter().enumerate() {
                            let (tag, color) = match item.kind {
                                complete::Kind::Tag => ("<>", theme.color_of("tag.name")),
                                complete::Kind::Attribute => ("at", theme.color_of("tag.attribute.name")),
                                complete::Kind::Property => ("pr", theme.color_of("style.attribute.name")),
                                complete::Kind::Value => ("va", theme.color_of("style.value.keyword")),
                                complete::Kind::Keyword => ("kw", theme.color_of("keyword")),
                                complete::Kind::Snippet => ("{}", theme.accent),
                                complete::Kind::Word => ("ab", theme.gutter_fg),
                                complete::Kind::Ext => ("ex", theme.color_of("identifier.function")),
                            };
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.label(egui::RichText::new(tag).monospace().size(11.0).color(color));
                                    let r = ui.add(egui::Button::selectable(
                                        i == c.selected,
                                        egui::RichText::new(&item.label).monospace(),
                                    ));
                                    if !item.detail.is_empty() {
                                        ui.label(egui::RichText::new(&item.detail).size(11.0).color(theme.text_faint));
                                    }
                                    r
                                })
                                .inner;
                            if i == c.selected {
                                resp.scroll_to_me(None);
                            }
                            if resp.clicked() {
                                clicked = Some(i);
                            }
                        }
                    });
                });
        });
    if let Some(i) = clicked {
        let mut c = doc.view.completion.take().unwrap();
        c.selected = i;
        complete::accept(doc, &c);
        ui.ctx().memory_mut(|m| m.request_focus(editor));
    }
}

fn handle_key(
    doc: &mut Document,
    settings: &EditorSettings,
    key: Key,
    mods: Modifiers,
    page: isize,
    tab: usize,
    wrap: Option<usize>,
) {
    let ctrl = mods.command;
    let shift = mods.shift;
    let alt = mods.alt;
    // With wrapped lines, Up/Down move by visual rows.
    let vertical = |doc: &mut Document, delta: isize| match wrap {
        Some(_) => move_visual(doc, delta, shift, tab, wrap),
        None => cmd::move_vertical(doc, delta, shift, tab),
    };
    match key {
        Key::Enter if ctrl => {
            // Insert a new line below without splitting the current one.
            cmd::move_end(doc, false);
            cmd::type_text(doc, "\n", settings);
        }
        Key::Enter => cmd::type_text(doc, "\n", settings),
        // Inside a snippet, Tab moves between its placeholders.
        Key::Tab if doc.view.snippet.is_some() && doc.snippet_step(!shift) => {}
        Key::Tab if shift => cmd::outdent(doc, settings),
        Key::Tab => {
            if !(settings.emmet && cmd::expand_emmet(doc, settings)) {
                cmd::indent(doc, settings);
            }
        }
        Key::Backspace => cmd::backspace(doc, ctrl, settings),
        Key::Delete => cmd::delete_forward(doc, ctrl),
        Key::K if ctrl && shift => cmd::delete_lines(doc),
        Key::I if alt && shift && !ctrl => cmd::cursors_at_line_ends(doc),
        Key::ArrowLeft => cmd::move_horizontal(doc, false, ctrl, shift),
        Key::ArrowRight => cmd::move_horizontal(doc, true, ctrl, shift),
        Key::ArrowUp if alt && !ctrl => cmd::move_lines(doc, true),
        Key::ArrowDown if alt && !ctrl => cmd::move_lines(doc, false),
        Key::ArrowUp if ctrl && alt => cmd::add_cursor_vertical(doc, false, tab),
        Key::ArrowDown if ctrl && alt => cmd::add_cursor_vertical(doc, true, tab),
        Key::ArrowUp => vertical(doc, -1),
        Key::ArrowDown => vertical(doc, 1),
        Key::PageUp => vertical(doc, -page),
        Key::PageDown => vertical(doc, page),
        Key::Home if ctrl => cmd::move_to(doc, 0, shift),
        Key::End if ctrl => {
            let len = doc.len_chars();
            cmd::move_to(doc, len, shift)
        }
        Key::Home => cmd::move_home(doc, shift),
        Key::End => cmd::move_end(doc, shift),
        Key::A if ctrl => cmd::select_all(doc),
        Key::Z if ctrl && shift => doc.redo(),
        Key::Z if ctrl => doc.undo(),
        Key::Y if ctrl => doc.redo(),
        Key::D if ctrl && shift => cmd::duplicate_lines(doc),
        Key::D if ctrl => cmd::select_next_occurrence(doc),
        Key::L if ctrl && shift => cmd::select_all_occurrences(doc),
        Key::L if ctrl => cmd::select_line(doc),
        Key::Slash if ctrl => cmd::toggle_comment(doc),
        Key::Escape => {
            // First Esc leaves the snippet, keeping the selection.
            if doc.view.snippet.take().is_none() {
                cmd::collapse_selections(doc);
            }
        }
        _ => {}
    }
}

/// Moves every cursor by visual rows (used when lines wrap).
fn move_visual(doc: &mut Document, delta: isize, extend: bool, tab: usize, wrap: Option<usize>) {
    let rows = folding::rows(doc, tab, wrap);
    let mut sels = doc.selections.clone();
    for s in &mut sels {
        let (row, col) = rows.locate(doc, s.head, tab);
        let goal = s.goal.unwrap_or(col);
        let target = row as isize + delta;
        let head = if target < 0 {
            0
        } else if target as usize >= rows.count() {
            doc.len_chars()
        } else {
            rows.pos_at(doc, target as usize, goal, tab)
        };
        *s = Selection {
            anchor: if extend { s.anchor } else { head },
            head,
            goal: Some(goal),
        };
    }
    doc.selections = sels;
    doc.normalize_selections();
    doc.view.scroll_to_cursor = true;
}

/// Screen geometry of the visual rows drawn this frame.
struct Geo<'a> {
    rows: &'a Rows,
    origin: Pos2,
    text_x0: f32,
    row_h: f32,
    char_w: f32,
    tab: usize,
    /// Rows painted this frame.
    vis: Range<usize>,
}

impl Geo<'_> {
    fn row_y(&self, row: usize) -> f32 {
        self.origin.y + row as f32 * self.row_h
    }

    fn col_x(&self, col: usize) -> f32 {
        self.text_x0 + col as f32 * self.char_w
    }

    /// Top left corner of the char cell at `pos`.
    fn at(&self, doc: &Document, pos: usize) -> Pos2 {
        let (row, col) = self.rows.locate(doc, pos, self.tab);
        pos2(self.col_x(col), self.row_y(row))
    }

    /// Visible rows covered by a char range: (row, x0, x1, line break included).
    fn spans(&self, doc: &Document, range: Range<usize>) -> Vec<(usize, f32, f32, bool)> {
        let (r0, c0) = self.rows.locate(doc, range.start, self.tab);
        let (r1, c1) = self.rows.locate(doc, range.end, self.tab);
        let mut out = Vec::new();
        for r in r0.max(self.vis.start)..=r1.min(self.vis.end.saturating_sub(1)) {
            let x0 = if r == r0 { c0 } else { self.rows.row(r).pad };
            let (x1, newline) = if r == r1 {
                (c1, false)
            } else {
                (self.rows.row_width(doc, r, self.tab), self.rows.continues(r).is_none())
            };
            out.push((r, self.col_x(x0), self.col_x(x1), newline));
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &Ui,
    doc: &mut Document,
    settings: &EditorSettings,
    theme: &Theme,
    matches: &[Range<usize>],
    current_match: Option<usize>,
    m: &Metrics,
    origin: Pos2,
    visible: Rect,
    focused: bool,
    rows: &Rows,
) {
    // Matching tag names at the cursor (cached per version and cursor).
    let head = doc.primary_selection().head;
    let tag_key = (doc.version, head);
    if !tags::supported(doc.language) || doc.selections.len() != 1 {
        doc.view.tag_pair = None;
    } else if doc.view.tag_pair.as_ref().is_none_or(|(k, _)| *k != tag_key) {
        doc.view.tag_pair = Some((tag_key, tags::pair_at(doc, head)));
    }
    let fold_ranges = folding::fold_ranges(doc, settings.tab_size);
    let mut color_hits = Vec::new();
    paint_doc(ui, doc, settings, theme, matches, current_match, m, origin, visible, focused, rows, &fold_ranges, &mut color_hits);
    doc.view.color_hits = color_hits;
}

#[allow(clippy::too_many_arguments)]
fn paint_doc(
    ui: &Ui,
    doc: &Document,
    settings: &EditorSettings,
    theme: &Theme,
    matches: &[Range<usize>],
    current_match: Option<usize>,
    m: &Metrics,
    origin: Pos2,
    visible: Rect,
    focused: bool,
    rows: &Rows,
    fold_ranges: &[(usize, usize)],
    color_hits: &mut Vec<(Rect, Range<usize>)>,
) {
    let painter = ui.painter_at(visible);
    let tab = settings.tab_size;
    let row_count = rows.count();
    let first_row = (((visible.top() - origin.y) / m.row_h).floor().max(0.0) as usize).min(row_count);
    let last_row = (((visible.bottom() - origin.y) / m.row_h).ceil() as usize + 1).min(row_count);
    // Document lines shown this frame.
    let (first, last) = if first_row < last_row {
        (rows.line_of_row(first_row), rows.line_of_row(last_row - 1) + 1)
    } else {
        (0, 0)
    };
    let shown = |l: usize| (first..last).contains(&l) && !rows.is_hidden(l);
    let text_x0 = origin.x + m.gutter_w + TEXT_PADDING;
    let g = Geo { rows, origin, text_x0, row_h: m.row_h, char_w: m.char_w, tab, vis: first_row..last_row };
    let wrapped = rows.wrap.is_some();
    // Without wrapping, long lines are laid out from the first visible column.
    let first_col = if wrapped {
        0
    } else {
        (((visible.left() + m.gutter_w - text_x0) / m.char_w).floor().max(0.0)) as usize
    };
    let col_count = (visible.width() / m.char_w).ceil() as usize + 2;
    let glyph_offset = (m.row_h - ui.fonts_mut(|f| f.row_height(&m.font))) / 2.0;

    let primary = doc.primary_selection();
    let cursors: Vec<usize> = doc.selections.iter().filter(|s| s.is_empty()).map(|s| s.head).collect();
    let cursor_lines: Vec<usize> = cursors.iter().map(|&p| doc.char_to_line(p)).collect();
    let cursor_rows: Vec<usize> = cursors.iter().map(|&p| rows.locate(doc, p, tab).0).collect();

    // Current line background.
    if settings.highlight_current_line {
        for (&l, &r) in cursor_lines.iter().zip(&cursor_rows) {
            if shown(l) {
                let r = Rect::from_min_size(pos2(visible.left(), g.row_y(r)), vec2(visible.width(), m.row_h));
                painter.rect_filled(r, 0.0, theme.current_line);
            }
        }
    }

    let paint_range = |range: Range<usize>, color: Color32, include_newline: bool| {
        for (r, x0, mut x1, newline) in g.spans(doc, range) {
            if include_newline && newline {
                x1 += m.char_w * 0.6;
            }
            if x1 > x0 {
                let y = g.row_y(r);
                painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + m.row_h)), 2.0, color);
            }
        }
    };

    // Find matches.
    let first_char = doc.line_to_char(first);
    let last_char = doc.line_to_char(last);
    if let Some(scope) = &doc.view.find_scope {
        paint_range(scope.clone(), theme.selection_inactive.gamma_multiply(0.6), true);
    }
    // Placeholders of the snippet being filled in, outlined.
    if let Some(session) = &doc.view.snippet {
        let stroke = Stroke::new(1.0, theme.accent.gamma_multiply(0.7));
        for (i, group) in session.stops.iter().enumerate() {
            for range in group {
                if range.end < first_char || range.start > last_char {
                    continue;
                }
                for (r, x0, x1, _) in g.spans(doc, range.clone()) {
                    let y = g.row_y(r);
                    let rect = Rect::from_min_max(pos2(x0 - 1.0, y + 1.0), pos2(x1.max(x0) + 1.0, y + m.row_h - 1.0));
                    if i == session.current {
                        painter.rect_filled(rect, 2.0, theme.accent.gamma_multiply(0.08));
                    }
                    painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
                }
            }
        }
    }
    let start_idx = matches.partition_point(|r| r.end < first_char);
    for (i, r) in matches.iter().enumerate().skip(start_idx) {
        if r.start > last_char {
            break;
        }
        let color = if Some(i) == current_match {
            theme.find_match.gamma_multiply(2.0)
        } else {
            theme.find_match
        };
        paint_range(r.clone(), color, false);
    }

    // Matching opening and closing tag.
    if let Some((_, Some((open, close)))) = &doc.view.tag_pair {
        paint_range(open.clone(), theme.bracket_match, false);
        paint_range(close.clone(), theme.bracket_match, false);
    }

    // Other occurrences of the word at the cursor (or of a selected word), in view.
    let in_tag = matches!(&doc.view.tag_pair, Some((_, Some(_))));
    if doc.selections.len() == 1 && !in_tag {
        let word = if primary.is_empty() {
            cmd::word_at(doc, primary.head)
        } else {
            let r = primary.range();
            let text = doc.slice(r.clone());
            (text.chars().all(cmd::is_word_char)).then_some(r)
        };
        if let Some(w) = word.filter(|w| (2..=100).contains(&w.len())) {
            let needle = doc.slice(w);
            let text = doc.slice(first_char..last_char);
            let (mut byte, mut chars) = (0usize, 0usize);
            for (i, _) in text.match_indices(&needle) {
                chars += text[byte..i].chars().count();
                byte = i;
                let before = text[..i].chars().next_back();
                let after = text[i + needle.len()..].chars().next();
                if before.is_some_and(cmd::is_word_char) || after.is_some_and(cmd::is_word_char) {
                    continue;
                }
                let start = first_char + chars;
                paint_range(start..start + needle.chars().count(), theme.word_highlight, false);
            }
        }
    }

    // Selections.
    let sel_color = if focused { theme.selection } else { theme.selection_inactive };
    for s in &doc.selections {
        if !s.is_empty() {
            paint_range(s.range(), sel_color, true);
        }
    }

    // Matching bracket.
    if primary.is_empty()
        && let Some((a, b)) = cmd::matching_bracket(doc, primary.head)
    {
        for p in [a, b] {
            if shown(doc.char_to_line(p)) {
                let at = g.at(doc, p);
                let r = Rect::from_min_size(at + vec2(0.0, 1.0), vec2(m.char_w, m.row_h - 2.0));
                painter.rect_filled(r, 2.0, theme.bracket_match);
            }
        }
    }

    let show_colors = matches!(
        doc.language,
        crate::lang::Language::Css
            | crate::lang::Language::Scss
            | crate::lang::Language::Less
            | crate::lang::Language::Html
            | crate::lang::Language::Php
    );
    // Text, row by row. Wrapped lines span several rows, so the line text
    // and its color values are kept while its rows are drawn.
    let spans = doc.highlights.clone();
    let mut prev_indent = 0usize;
    let mut current: Option<(usize, String, Vec<ColorValue>)> = None;
    for r in first_row..last_row {
        let row = rows.row(r);
        let line = row.line;
        if current.as_ref().is_none_or(|c| c.0 != line) {
            let text = doc.line_text(line);
            let mut colors = Vec::new();
            if show_colors {
                let line_start = doc.line_to_char(line);
                for c in COLOR_RE.find_iter(&text) {
                    if let Some(color) = crate::theme_import::parse_color(c.as_str()) {
                        let s = line_start + text[..c.start()].chars().count();
                        let e = line_start + text[..c.end()].chars().count();
                        colors.push((s, e, color.to_opaque()));
                    }
                }
            }
            current = Some((line, text, colors));
        }
        let (_, text, colors) = current.as_ref().unwrap();
        let line_spans: &[Span] = spans.get(line).map_or(&[], |v| v.as_slice());
        let y = g.row_y(r);
        let last_of_line = rows.continues(r).is_none();

        // Indent guides.
        if settings.show_indent_guides && row.start == 0 {
            let indent = if text.trim().is_empty() {
                prev_indent
            } else {
                indent_columns(text, tab)
            };
            prev_indent = indent;
            let mut c = tab;
            while c <= indent {
                if c >= first_col {
                    let x = g.col_x(c - tab) + 0.5;
                    painter.line_segment(
                        [pos2(x, y), pos2(x, y + m.row_h)],
                        Stroke::new(1.0, theme.indent_guide),
                    );
                }
                c += tab;
            }
        }

        if text.is_empty() {
            continue;
        }
        let (from_col, count) = if wrapped {
            (row.col, rows.row_width(doc, r, tab) - row.pad)
        } else {
            (first_col, col_count)
        };
        let (job, start_col) = layout_line(
            text,
            row.byte,
            row.col,
            line_spans,
            theme,
            &m.font,
            tab,
            from_col,
            count,
            settings.show_invisibles,
        );
        if !job.text.is_empty() {
            let galley = painter.layout_job(job);
            painter.galley(pos2(g.col_x(start_col - row.col + row.pad), y + glyph_offset), galley, theme.foreground);
        }
        if last_of_line && doc.view.folds.contains(&line) {
            let end = doc.column_of(doc.line_end(line), tab).saturating_sub(row.col) + row.pad;
            let x = g.col_x(end) + m.char_w;
            let pill = Rect::from_min_size(pos2(x, y + 4.0), vec2(m.char_w * 3.0, m.row_h - 8.0));
            painter.rect_filled(pill, 4.0, theme.hover.gamma_multiply(2.5));
            painter.text(pill.center(), egui::Align2::CENTER_CENTER, "⋯", FontId::proportional(m.font.size), theme.text_muted);
        }

        // Color previews under CSS color values starting on this row.
        for &(s, e, color) in colors {
            let (sr, _) = rows.locate(doc, s, tab);
            if sr != r {
                continue;
            }
            let x0 = g.at(doc, s).x;
            let (er, _) = rows.locate(doc, e, tab);
            let x1 = if er == r { g.at(doc, e).x } else { g.col_x(rows.row_width(doc, r, tab)) };
            let bar = Rect::from_min_max(pos2(x0, y + m.row_h - 4.0), pos2(x1, y + m.row_h - 1.0));
            painter.rect_filled(bar, 1.5, color);
            // A little taller than the bar so it is easy to hit.
            color_hits.push((bar.expand2(vec2(0.0, 3.0)).intersect(visible), s..e));
        }
    }

    // Problems reported by extensions: wavy underline in the severity color.
    if !doc.view.diagnostics.is_empty() {
        for d in &doc.view.diagnostics {
            if d.range.end < first_char || d.range.start > last_char {
                continue;
            }
            let color = severity_color(theme, d.severity);
            for (r, x0, x1, _) in g.spans(doc, d.range.clone()) {
                wavy(&painter, x0, x1.max(x0 + m.char_w), g.row_y(r) + m.row_h - 3.0, color);
            }
        }
    }

    // Cursors.
    if focused {
        let time = ui.input(|i| i.time);
        let blink_on = (time % 1.06) < 0.62;
        if blink_on {
            for s in &doc.selections {
                if shown(doc.char_to_line(s.head)) {
                    let at = g.at(doc, s.head);
                    let x = at.x.round() - 0.5;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(x, at.y + 2.0), vec2(2.0, m.row_h - 4.0)),
                        0.0,
                        theme.cursor,
                    );
                }
            }
        }
        let until_toggle = if blink_on { 0.62 - time % 1.06 } else { 1.06 - time % 1.06 };
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(until_toggle.max(0.01)));
    }

    // Gutter, drawn last so horizontally scrolled text slides under it.
    let gutter = Rect::from_min_size(visible.min, vec2(m.gutter_w, visible.height()));
    painter.rect_filled(gutter, 0.0, theme.gutter_bg);
    let num_font = FontId::monospace(m.font.size * 0.86);
    let num_offset = (m.row_h - ui.fonts_mut(|f| f.row_height(&num_font))) / 2.0;
    let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| gutter.contains(*p));
    let git_marks = doc.view.git_marks.as_ref().map(|(_, m)| m.as_slice());
    // Most severe problem per visible line.
    let mut line_problems: std::collections::HashMap<usize, u8> = std::collections::HashMap::new();
    if !doc.view.diagnostics.is_empty() {
        for d in doc.view.diagnostics.iter().filter(|d| d.range.start >= first_char && d.range.start <= last_char) {
            let l = doc.char_to_line(d.range.start);
            let e = line_problems.entry(l).or_insert(d.severity);
            *e = (*e).min(d.severity);
        }
    }
    for r in first_row..last_row {
        let row = rows.row(r);
        let line = row.line;
        let y = g.row_y(r);
        if cursor_rows.contains(&r) && settings.highlight_current_line {
            let rect = Rect::from_min_size(pos2(gutter.left(), y), vec2(gutter.width(), m.row_h));
            painter.rect_filled(rect, 0.0, theme.gutter_active_bg);
        }
        // Git change marker at the right edge of the gutter.
        match git_marks.and_then(|g| g.get(line)) {
            Some(&mark @ (1 | 2)) => {
                let color = if mark == 1 { theme.success } else { theme.modified };
                let bar = Rect::from_min_max(pos2(gutter.right() - 3.5, y), pos2(gutter.right() - 1.0, y + m.row_h));
                painter.rect_filled(bar, 0.0, color);
            }
            Some(3) if row.start == 0 => {
                let (x, y) = (gutter.right() - 1.0, y);
                painter.add(egui::Shape::convex_polygon(
                    vec![pos2(x - 5.0, y), pos2(x, y - 4.0), pos2(x, y + 4.0)],
                    theme.danger,
                    Stroke::NONE,
                ));
            }
            _ => {}
        }
        // Continuation rows of a wrapped line carry no number.
        if row.start != 0 {
            continue;
        }
        let cy = y + m.row_h / 2.0;
        if let Some(&sev) = line_problems.get(&line) {
            painter.circle_filled(pos2(gutter.right() - 6.0, cy), 2.5, severity_color(theme, sev));
        }
        // Fold marker: a small box with − (open) or + (folded).
        let folded = doc.view.folds.contains(&line);
        let range = fold_ranges.binary_search_by_key(&line, |r| r.0).ok().map(|i| fold_ranges[i]);
        if folded || range.is_some() {
            let center = pos2(gutter.left() + 10.0, cy);
            let hovered = pointer.is_some_and(|p| p.x < gutter.left() + 17.0 && (p.y - cy).abs() <= m.row_h / 2.0);
            let color = if hovered || folded { theme.gutter_fg_active } else { theme.gutter_fg };
            if hovered
                && !folded
                && let Some((_, end)) = range
            {
                // The block the marker folds.
                let bottom = g.row_y(rows.row_of_line(end) + 1).min(visible.bottom());
                painter.line_segment(
                    [pos2(center.x, cy + 5.0), pos2(center.x, bottom - 3.0)],
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                );
                painter.line_segment(
                    [pos2(center.x, bottom - 3.0), pos2(center.x + 4.0, bottom - 3.0)],
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                );
            }
            fold_marker(&painter, center, folded, color, theme.gutter_bg);
        }
        let active = cursor_lines.contains(&line)
            || doc.selections.iter().any(|s| {
                let r = s.range();
                doc.char_to_line(r.start) <= line && line <= doc.char_to_line(r.end)
            });
        let bookmarked = doc.view.bookmarks.contains(&line);
        let color = if active || bookmarked { theme.gutter_fg_active } else { theme.gutter_fg };
        let number = painter.text(
            pos2(gutter.right() - 14.0, y + num_offset),
            egui::Align2::RIGHT_TOP,
            (line + 1).to_string(),
            num_font.clone(),
            color,
        );
        if bookmarked {
            // A small flag right before the number, as high as the fold marker.
            let (w, h) = (7.0, 9.0);
            crate::ui::icons::bookmark_filled(&painter, pos2(number.left() - 3.0 - w / 2.0, cy), w, h, theme.bookmark());
        }
    }
}

/// The +/− box in the gutter for a foldable block.
fn fold_marker(painter: &egui::Painter, center: Pos2, folded: bool, color: Color32, bg: Color32) {
    let r = Rect::from_center_size(center, vec2(9.0, 9.0));
    painter.rect_filled(r, 2.0, bg);
    painter.rect_stroke(r, 2.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    let stroke = Stroke::new(1.2, color);
    painter.line_segment([pos2(center.x - 2.5, center.y), pos2(center.x + 2.5, center.y)], stroke);
    if folded {
        painter.line_segment([pos2(center.x, center.y - 2.5), pos2(center.x, center.y + 2.5)], stroke);
    }
}

fn indent_columns(text: &str, tab: usize) -> usize {
    let mut col = 0;
    for ch in text.chars() {
        match ch {
            ' ' => col += 1,
            '\t' => col = (col / tab + 1) * tab,
            _ => break,
        }
    }
    col
}

/// Builds a layout job for the visible part of a line, starting at byte
/// `from_byte` (display column `from_col`). Returns the job and the display
/// column it starts at.
#[allow(clippy::too_many_arguments)]
fn layout_line(
    text: &str,
    from_byte: usize,
    from_col: usize,
    spans: &[Span],
    theme: &Theme,
    font: &FontId,
    tab: usize,
    first_col: usize,
    col_count: usize,
    invisibles: bool,
) -> (LayoutJob, usize) {
    let mut job = LayoutJob::default();
    let faint = theme.invisible;
    let end_col = first_col + col_count;
    let mut col = from_col;
    let mut start_col = None;
    let from_byte = from_byte.min(text.len());
    let mut span_i = spans.partition_point(|s| (s.end as usize) <= from_byte);
    let mut segment = String::new();
    let mut segment_color = theme.foreground;

    let flush = |job: &mut LayoutJob, segment: &mut String, color: Color32| {
        if !segment.is_empty() {
            job.append(segment, 0.0, TextFormat::simple(font.clone(), color));
            segment.clear();
        }
    };

    for (offset, ch) in text[from_byte..].char_indices() {
        let byte = from_byte + offset;
        if col >= end_col {
            break;
        }
        let width = if ch == '\t' { tab - col % tab } else { 1 };
        if col + width <= first_col {
            col += width;
            continue;
        }
        start_col.get_or_insert(col);

        while span_i < spans.len() && spans[span_i].end as usize <= byte {
            span_i += 1;
        }
        let mut color = match spans.get(span_i) {
            Some(s) if (s.start as usize) <= byte => theme.syntax_color(s.style),
            _ => theme.foreground,
        };
        let shown: Option<char> = match ch {
            '\t' => None,
            ' ' if invisibles => {
                color = faint;
                Some('·')
            }
            c if c.is_control() => Some('\u{FFFD}'),
            c => Some(c),
        };
        if color != segment_color {
            flush(&mut job, &mut segment, segment_color);
            segment_color = color;
        }
        match shown {
            Some(c) => segment.push(c),
            None => {
                if invisibles {
                    flush(&mut job, &mut segment, segment_color);
                    segment.push('→');
                    segment.extend(std::iter::repeat_n(' ', width - 1));
                    flush(&mut job, &mut segment, faint);
                } else {
                    segment.extend(std::iter::repeat_n(' ', width));
                }
            }
        }
        col += width;
    }
    flush(&mut job, &mut segment, segment_color);
    (job, start_col.unwrap_or(first_col.max(from_col)))
}

/// A CSS color in the text: (start, end, color).
type ColorValue = (usize, usize, Color32);

static COLOR_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\b|(?:rgba?|hsla?)\([^()]*\)").unwrap()
});

fn severity_color(theme: &Theme, severity: u8) -> Color32 {
    match severity {
        1 => theme.danger,
        2 => theme.modified,
        _ => theme.accent,
    }
}

/// A wavy underline from `x0` to `x1` at height `y`.
fn wavy(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut pts = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        pts.push(pos2(x, if up { y - 1.2 } else { y + 1.2 }));
        up = !up;
        x += 2.5;
    }
    if pts.len() > 1 {
        painter.add(egui::Shape::line(pts, Stroke::new(1.1, color)));
    }
}

/// Tooltip for problems and extension hover information under the pointer.
#[allow(clippy::too_many_arguments)]
fn show_hover(
    ui: &Ui,
    response: &egui::Response,
    doc: &mut Document,
    theme: &Theme,
    id: Id,
    hit: &dyn Fn(&Document, Pos2) -> usize,
    visible: Rect,
    m: &Metrics,
) {
    let state_id = id.with("hover");
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let busy = ui.input(|i| i.pointer.any_down()) || doc.view.completion.is_some();
    let Some(p) = pointer.filter(|p| response.hovered() && !busy && p.x > visible.left() + m.gutter_w) else {
        ui.data_mut(|d| d.remove::<(usize, f64, bool)>(state_id));
        return;
    };
    let at = hit(doc, p);
    // Past the end of the line there is nothing to explain.
    let line = doc.char_to_line(at);
    if at >= doc.line_end(line) && !doc.view.diagnostics.iter().any(|d| d.range.start <= at && at <= d.range.end) {
        ui.data_mut(|d| d.remove::<(usize, f64, bool)>(state_id));
        return;
    }
    let now = ui.input(|i| i.time);
    let word = cmd::word_at(doc, at).unwrap_or(at..at + 1);
    let (last, since, requested) = ui.data(|d| d.get_temp::<(usize, f64, bool)>(state_id)).unwrap_or((usize::MAX, now, false));
    let same = last != usize::MAX && (word.start..word.end.max(word.start + 1)).contains(&last);
    let (since, mut requested) = if same { (since, requested) } else { (now, false) };
    let rested = now - since;
    const DELAY: f64 = 0.55;
    if rested < DELAY {
        ui.data_mut(|d| d.insert_temp(state_id, (at, since, requested)));
        ui.ctx().request_repaint_after(Duration::from_secs_f64(DELAY - rested + 0.01));
        return;
    }
    if !requested && word.end > word.start {
        doc.view.hover_request = Some(at);
        requested = true;
    }
    ui.data_mut(|d| d.insert_temp(state_id, (at, since, requested)));

    let mut parts: Vec<(Option<u8>, String)> = doc
        .view
        .diagnostics
        .iter()
        .filter(|d| d.range.start <= at && at <= d.range.end.max(d.range.start + 1))
        .map(|d| (Some(d.severity), d.message.clone()))
        .collect();
    if let Some((pos, text)) = &doc.view.hover
        && word.contains(pos)
    {
        parts.push((None, clean_markdown(text)));
    }
    if parts.is_empty() {
        return;
    }
    egui::Area::new(id.with("hover_area"))
        .fixed_pos(p + vec2(10.0, 18.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(10.0).show(ui, |ui| {
                ui.set_max_width(560.0);
                for (i, (sev, text)) in parts.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    match sev {
                        Some(s) => {
                            ui.horizontal_wrapped(|ui| {
                                let (label, color) = match s {
                                    1 => (t!("Fehler"), theme.danger),
                                    2 => (t!("Warnung"), theme.modified),
                                    _ => (t!("Hinweis"), theme.accent),
                                };
                                ui.label(egui::RichText::new(label).strong().color(color));
                                ui.label(text);
                            });
                        }
                        None => {
                            ui.add(egui::Label::new(egui::RichText::new(text).monospace().size(12.0)).wrap());
                        }
                    }
                }
            });
        });
}

/// Hover texts are Markdown; show code blocks without their fences.
fn clean_markdown(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim() == "---" {
            continue;
        }
        out.push(line.replace(r"\_", "_").replace("**", ""));
    }
    let s = out.join("\n");
    s.trim().chars().take(4000).collect()
}

/// Recomputes the Git change markers when the text changed.
fn update_git_marks(doc: &mut Document) {
    let Some(base) = doc.view.git_base.clone() else {
        doc.view.git_marks = None;
        return;
    };
    if doc.view.git_marks.as_ref().is_some_and(|(v, _)| *v == doc.version) {
        return;
    }
    let text = doc.text();
    let new = crate::diff::lines(&text);
    let mut marks = vec![0u8; doc.len_lines()];
    for h in crate::diff::hunks(&crate::diff::lines(&base), &new) {
        if h.new.is_empty() {
            let last = marks.len().saturating_sub(1);
            if let Some(m) = marks.get_mut(h.new.start.min(last)) {
                *m = 3;
            }
        } else {
            let mark = if h.old.is_empty() { 1 } else { 2 };
            for l in h.new.clone() {
                if let Some(m) = marks.get_mut(l) {
                    *m = mark;
                }
            }
        }
    }
    doc.view.git_marks = Some((doc.version, Arc::new(marks)));
}
