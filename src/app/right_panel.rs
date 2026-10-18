//! Panel right of the editor, like Nova's inspector: live preview or the
//! outline of the active document.

use std::sync::Arc;

use eframe::egui::{self, Align2, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};

use super::NebulaApp;
use crate::editor::document::Selection;
use crate::editor::symbols::{self, Symbol};
use crate::settings::RightTab;
use crate::ui::icons;

const HEADER_H: f32 = 30.0;

#[derive(Default)]
pub(super) struct RightPanelState {
    /// Symbols for (document id, version).
    symbols: Option<((u64, u64), Arc<Vec<Symbol>>)>,
    filter: String,
}

impl NebulaApp {
    pub(super) fn toggle_right_panel(&mut self) {
        self.settings.right_panel = !self.settings.right_panel;
        self.sync_preview_flag();
    }

    /// Draws the panel if it is open; returns the area for the preview's web view.
    pub(super) fn right_panel(&mut self, ui: &mut egui::Ui) -> Option<Rect> {
        self.sync_preview_flag();
        if !self.settings.right_panel {
            return None;
        }
        // The width comes from the settings every frame (egui would remember
        // a width squeezed in an early, tiny frame); only dragging changes it.
        let available = ui.available_width();
        let width = self.settings.right_width.clamp(320.0, (available - 320.0).max(320.0));
        let resp = egui::Panel::right("right-panel")
            .resizable(false)
            .exact_size(width)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(self.theme.background))
            .show(ui, |ui| self.right_panel_contents(ui));
        let r = resp.response.rect;
        let handle = Rect::from_min_max(pos2(r.left() - 3.0, r.top()), pos2(r.left() + 3.0, r.bottom()));
        let drag = ui.interact(handle, egui::Id::unique("right-panel-resize"), Sense::drag());
        if drag.hovered() || drag.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if drag.dragged() {
            self.settings.right_width = (width - drag.drag_delta().x).clamp(320.0, (available - 320.0).max(320.0));
        }
        ui.painter().line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0, self.theme.separator));
        resp.inner
    }

    fn right_panel_contents(&mut self, ui: &mut egui::Ui) -> Option<Rect> {
        let theme = self.theme.clone();
        let full = ui.available_rect_before_wrap();
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
        let body = Rect::from_min_max(pos2(full.left(), header.bottom() + 1.0), full.max);
        ui.painter().rect_filled(header, 0.0, theme.jumpbar_bg);
        ui.painter().line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, theme.separator));
        let cy = header.center().y;

        // Tabs on the left.
        let mut x = header.left() + 8.0;
        for (tab, label) in [(RightTab::Preview, "Vorschau"), (RightTab::Outline, "Gliederung")] {
            let selected = self.settings.right_tab == tab;
            let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), theme.foreground);
            let r = Rect::from_min_size(pos2(x, cy - 11.0), vec2(g.size().x + 16.0, 22.0));
            let resp = ui.interact(r, egui::Id::unique(("right-tab", label)), Sense::click());
            if selected {
                ui.painter().rect_filled(r, 6.0, theme.hover.gamma_multiply(2.2));
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, theme.hover);
            }
            let color = if selected || resp.hovered() { theme.foreground } else { theme.text_muted };
            ui.painter().galley(pos2(r.left() + 8.0, cy - g.size().y / 2.0), g, color);
            if resp.clicked() {
                self.settings.right_tab = tab;
                self.sync_preview_flag();
            }
            x = r.right() + 4.0;
        }

        // Close button on the right.
        let close = Rect::from_center_size(pos2(header.right() - 18.0, cy), vec2(26.0, 24.0));
        let close_resp = ui.interact(close, egui::Id::unique("right-close"), Sense::click()).on_hover_text("Panel schließen");
        if close_resp.hovered() {
            ui.painter().rect_filled(close, 6.0, theme.hover);
        }
        icons::close(ui.painter(), close.center(), 8.0, if close_resp.hovered() { theme.foreground } else { theme.text_muted });
        if close_resp.clicked() {
            self.toggle_right_panel();
            return None;
        }

        match self.settings.right_tab {
            RightTab::Preview => self.preview_pane(ui, header, body, header.right() - 46.0, x + 6.0),
            RightTab::Outline => {
                self.outline(ui, body);
                None
            }
        }
    }

    /// Clickable list of the symbols in the active document.
    fn outline(&mut self, ui: &mut egui::Ui, body: Rect) {
        let theme = self.theme.clone();
        let Some(i) = self.active else { return };
        let key = (self.docs[i].id, self.docs[i].version);
        if self.right_state.symbols.as_ref().is_none_or(|(k, _)| *k != key) {
            let doc = &mut self.docs[i];
            let text = doc.text();
            self.right_state.symbols = Some((key, Arc::new(symbols::symbols(&text, doc.language))));
        }
        let list = self.right_state.symbols.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
        let doc = &self.docs[i];
        let cursor_line = doc.char_to_line(doc.primary_selection().head);
        let current = symbols::current(&list, cursor_line);

        let mut jump = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink2(vec2(8.0, 6.0))), |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.right_state.filter).hint_text("Symbole filtern…").desired_width(f32::INFINITY));
            ui.add_space(4.0);
            if list.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Keine Symbole in dieser Datei").color(theme.text_faint));
                });
                return;
            }
            let filter = self.right_state.filter.to_lowercase();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (n, s) in list.iter().enumerate() {
                    if !filter.is_empty() && !s.name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    let depth = if filter.is_empty() { s.depth.min(8) } else { 0 };
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                    if Some(n) == current {
                        ui.painter().rect_filled(r, 5.0, theme.hover.gamma_multiply(2.2));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 5.0, theme.hover);
                    }
                    let x = r.left() + 6.0 + depth as f32 * 14.0;
                    let badge_color = match s.kind {
                        symbols::Kind::Function => theme.accent,
                        symbols::Kind::Class => theme.modified,
                        symbols::Kind::Heading | symbols::Kind::Section => theme.success,
                        _ => theme.text_muted,
                    };
                    ui.painter().text(pos2(x + 8.0, r.center().y), Align2::CENTER_CENTER, s.kind.badge(), FontId::monospace(11.0), badge_color);
                    let painter = ui.painter().with_clip_rect(r);
                    painter.text(pos2(x + 22.0, r.center().y), Align2::LEFT_CENTER, &s.name, FontId::proportional(13.0), theme.foreground);
                    painter.text(pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER, (s.line + 1).to_string(), FontId::proportional(11.0), theme.text_faint);
                    if resp.clicked() {
                        jump = Some(s.line);
                    }
                }
            });
        });
        if let Some(line) = jump {
            let doc = &mut self.docs[i];
            let pos = doc.line_to_char(line);
            // Cursor at the first non-blank character of the line.
            let indent = doc.line_text(line).chars().take_while(|c| c.is_whitespace() && *c != '\n').count();
            doc.set_selection(Selection::cursor(pos + indent));
            doc.view.center_line = Some(line);
            self.focus_editor = true;
        }
    }
}
