//! Panel right of the editor, like Nova's inspector: live preview or the
//! outline of the active document.

use std::sync::Arc;

use eframe::egui::{self, Align2, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};

use super::NebulaApp;
use crate::editor::document::Selection;
use crate::editor::symbols::{self, Symbol};
use crate::settings::RightTab;
use crate::ui::icons;

/// Header with the tab buttons; same height as the sidebar's mode row.
const HEADER_H: f32 = 40.0;

#[derive(Default)]
pub(super) struct RightPanelState {
    /// Symbols for (document id, version).
    symbols: Option<((u64, u64), Arc<Vec<Symbol>>)>,
    filter: String,
    pub(super) snippets: super::snippets_panel::SnippetsState,
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
            .frame(egui::Frame::new().fill(self.theme.sidebar_bg))
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
        let body = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
        // The button row, with the same margins as in the sidebar.
        let row = Rect::from_min_size(header.min + vec2(10.0, 10.0), vec2(header.width() - 20.0, 26.0));

        // Tabs on the left, as icons like the mode switcher of the sidebar.
        let mut tabs = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        tabs.spacing_mut().item_spacing.x = 2.0;
        let tip = |name: &str, key: String| if key.is_empty() { name.to_string() } else { format!("{name} ({key})") };
        let preview_tip = tip(t!("Vorschau"), self.shortcut_label(super::palette::Command::TogglePreview));
        let entries: [(RightTab, String, fn(&egui::Painter, egui::Pos2, egui::Color32)); 4] = [
            (RightTab::Outline, t!("Gliederung").into(), icons::outline),
            (RightTab::Snippets, t!("Snippets").into(), icons::snippets),
            (RightTab::Agent, tip("KI-Agent", self.shortcut_label(super::palette::Command::ToggleAgent)), icons::sparkle),
            (RightTab::Preview, preview_tip, icons::eye),
        ];
        for (tab, hover, paint) in entries {
            let selected = self.settings.right_tab == tab;
            if icons::icon_button(&mut tabs, &theme, vec2(34.0, 26.0), selected, paint).on_hover_text(hover).clicked() {
                self.settings.right_tab = tab;
                self.sync_preview_flag();
            }
        }
        // Where the preview's address bar may start.
        let x = tabs.min_rect().right() + 4.0;

        match self.settings.right_tab {
            RightTab::Preview => self.preview_pane(ui, row, body, row.right() - 13.0, x + 6.0),
            RightTab::Outline => {
                self.outline(ui, body);
                None
            }
            RightTab::Snippets => {
                self.snippets_pane(ui, body);
                None
            }
            RightTab::Agent => {
                self.agent_pane(ui, row, body);
                None
            }
        }
    }

    /// Clickable list of the symbols in the active document.
    fn outline(&mut self, ui: &mut egui::Ui, body: Rect) {
        let theme = self.theme.clone();
        let Some(i) = self.active else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink2(vec2(8.0, 26.0))), |ui| {
                ui.vertical_centered(|ui| ui.label(RichText::new(t!("Keine Datei geöffnet")).color(theme.text_faint)));
            });
            return;
        };
        let list = self.doc_symbols(i);
        let doc = &self.docs[i];
        let cursor_line = doc.char_to_line(doc.primary_selection().head);
        let current = symbols::current(&list, cursor_line);

        let mut jump = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink2(vec2(10.0, 4.0))), |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.right_state.filter).hint_text(t!("Symbole filtern…")).desired_width(f32::INFINITY));
            ui.add_space(4.0);
            if list.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(t!("Keine Symbole in dieser Datei")).color(theme.text_faint));
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
            self.jump_to_line(i, line);
        }
    }

    /// Symbols of a document, cached per version.
    pub(super) fn doc_symbols(&mut self, i: usize) -> Arc<Vec<Symbol>> {
        let key = (self.docs[i].id, self.docs[i].version);
        if self.right_state.symbols.as_ref().is_none_or(|(k, _)| *k != key) {
            let doc = &mut self.docs[i];
            let text = doc.text();
            self.right_state.symbols = Some((key, Arc::new(symbols::symbols(&text, doc.language))));
        }
        self.right_state.symbols.as_ref().map(|(_, s)| s.clone()).unwrap_or_default()
    }

    /// Cursor to the first non-blank character of `line`, centered.
    pub(super) fn jump_to_line(&mut self, i: usize, line: usize) {
        let doc = &mut self.docs[i];
        let pos = doc.line_to_char(line);
        let indent = doc.line_text(line).chars().take_while(|c| c.is_whitespace() && *c != '\n').count();
        doc.set_selection(Selection::cursor(pos + indent));
        doc.view.center_line = Some(line);
        self.focus_editor = true;
    }
}
