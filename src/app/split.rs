//! Split view: two editors side by side, showing two documents or the same
//! document at two places. The document of the focused half is the active
//! one (tabs, saving, find), the other half keeps its document in `other`.

use eframe::egui::{self, CursorIcon, FontId, Rect, Sense, Stroke, pos2, vec2};

use super::NebulaApp;
use crate::editor::{EditorOutput, EditorView, editor_id};
use crate::ui::icons;

const HEADER_H: f32 = 26.0;

pub(super) struct SplitView {
    /// Document shown in the half without focus.
    pub other: u64,
    /// The right half has the focus (shows the active document).
    pub right_focused: bool,
    /// Width share of the left half.
    pub ratio: f32,
}

impl NebulaApp {
    /// Pane of the active document: 0 left/single, 1 right.
    pub(super) fn active_pane(&self) -> u8 {
        u8::from(self.split.as_ref().is_some_and(|s| s.right_focused))
    }

    /// Splits the editor, showing the active document in both halves, or closes the split.
    pub(super) fn toggle_split(&mut self) {
        if self.split.take().is_some() {
            self.focus_editor = true;
            return;
        }
        let Some(i) = self.active else { return };
        self.split = Some(SplitView { other: self.docs[i].id, right_focused: true, ratio: 0.5 });
        self.focus_editor = true;
    }

    /// Shows a document in the right half (opening the split if needed).
    pub(super) fn open_in_split(&mut self, index: usize) {
        let Some(active) = self.active.map(|i| self.docs[i].id) else { return };
        match &mut self.split {
            None => {
                self.split = Some(SplitView { other: active, right_focused: true, ratio: 0.5 });
                self.activate(index);
            }
            Some(s) if s.right_focused => self.activate(index),
            Some(s) => {
                // Move focus to the right half with the document.
                s.right_focused = true;
                s.other = active;
                self.activate(index);
            }
        }
    }

    /// The editor area: one editor, or two with headers and a divider.
    pub(super) fn editor_area(&mut self, ui: &mut egui::Ui, rect: Rect, matches: &[std::ops::Range<usize>]) -> EditorOutput {
        let Some(active) = self.active else { return EditorOutput::default() };
        if let Some(s) = &self.split
            && self.doc_index(s.other).is_none()
        {
            self.split = None;
        }
        let Some(split) = &self.split else {
            return self.show_pane(ui, rect, active, 0, true, matches);
        };
        let (ratio, right_focused, other) = (split.ratio, split.right_focused, split.other);
        let Some(other_index) = self.doc_index(other) else { return EditorOutput::default() };
        let same = self.docs[active].id == other;
        let (left, right) = if right_focused { (other_index, active) } else { (active, other_index) };

        let divider_x = (rect.left() + rect.width() * ratio).round().clamp(rect.left() + 150.0, rect.right() - 150.0);
        let left_rect = Rect::from_min_max(rect.min, pos2(divider_x, rect.bottom()));
        let right_rect = Rect::from_min_max(pos2(divider_x + 1.0, rect.top()), rect.max);

        // Divider, draggable.
        let handle = Rect::from_min_max(pos2(divider_x - 3.0, rect.top()), pos2(divider_x + 4.0, rect.bottom()));
        let drag = ui.interact(handle, egui::Id::unique("split-divider"), Sense::drag());
        if drag.hovered() || drag.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if drag.dragged()
            && let Some(p) = drag.interact_pointer_pos()
            && let Some(s) = &mut self.split
        {
            s.ratio = ((p.x - rect.left()) / rect.width()).clamp(0.15, 0.85);
        }

        let mut close = false;
        let mut outputs = [EditorOutput::default(), EditorOutput::default()];
        for (pane, (index, area)) in [(left, left_rect), (right, right_rect)].into_iter().enumerate() {
            let pane = pane as u8;
            let focused = pane == self.active_pane();
            let header = Rect::from_min_size(area.min, vec2(area.width(), HEADER_H));
            if self.pane_header(ui, header, index, pane, focused) {
                close = true;
            }
            let body = Rect::from_min_max(pos2(area.left(), area.top() + HEADER_H), area.max);
            let pane_matches = if focused { matches } else { &[] };
            outputs[pane as usize] = self.show_pane(ui, body, index, pane, focused || !same, pane_matches);
        }
        ui.painter().line_segment(
            [pos2(divider_x + 0.5, rect.top()), pos2(divider_x + 0.5, rect.bottom())],
            Stroke::new(1.0, self.theme.separator),
        );
        if close {
            self.split = None;
            self.focus_editor = true;
            return EditorOutput::default();
        }

        // Focus moved to the other half: its document becomes the active one.
        let [left_out, right_out] = outputs;
        let clicked_other = if right_focused { left_out.gained_focus } else { right_out.gained_focus };
        if clicked_other {
            self.switch_pane();
        }
        EditorOutput {
            copied: left_out.copied.or(right_out.copied),
            ai: left_out.ai.or(right_out.ai),
            has_focus: left_out.has_focus || right_out.has_focus,
            gained_focus: false,
        }
    }

    /// Makes the document of the other half the active one.
    pub(super) fn switch_pane(&mut self) {
        let (Some(active), Some(split)) = (self.active, &mut self.split) else { return };
        let other = split.other;
        split.other = self.docs[active].id;
        split.right_focused = !split.right_focused;
        if let Some(i) = self.doc_index(other) {
            self.active = Some(i);
            self.reveal_active_tab = true;
        }
    }

    fn show_pane(&mut self, ui: &mut egui::Ui, rect: Rect, index: usize, pane: u8, follow: bool, matches: &[std::ops::Range<usize>]) -> EditorOutput {
        let settings = self.settings.editor_for_doc(&self.docs[index]);
        let current_match = if matches.is_empty() { None } else { self.find.current };
        self.docs[index].view.find_scope = if self.find.visible { self.find.scope_in(&self.docs[index]) } else { None };
        let doc = &mut self.docs[index];
        if let Some(image) = doc.image.clone() {
            let id = egui::Id::unique(("image-pane", doc.id, pane));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(("pane", pane)), |ui| {
                crate::image_view::show(ui, rect, &image, &self.theme, id);
            });
            return EditorOutput::default();
        }
        if let Some(hex) = doc.hex.clone() {
            let id = egui::Id::unique(("hex-pane", doc.id, pane));
            let can_text = hex.len() <= super::MAX_READ_ONLY_BYTES as u64;
            let action = ui
                .scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(("pane", pane)), |ui| {
                    crate::hex_view::show(ui, rect, &hex, &self.theme, id, can_text)
                })
                .inner;
            if let crate::hex_view::Action::OpenAsText = action {
                self.reopen_as_text(index);
            }
            return EditorOutput::default();
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(("pane", pane)), |ui| {
            EditorView { doc, settings: &settings, theme: &self.theme, matches, current_match, pane, follow }.show(ui)
        })
        .inner
    }

    /// File name above a half of the split view; returns true when it should close.
    fn pane_header(&mut self, ui: &mut egui::Ui, rect: Rect, index: usize, pane: u8, focused: bool) -> bool {
        let theme = &self.theme;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.jumpbar_bg);
        if focused {
            painter.line_segment(
                [pos2(rect.left(), rect.bottom() - 1.0), pos2(rect.right(), rect.bottom() - 1.0)],
                Stroke::new(2.0, theme.accent),
            );
        }
        let doc = &self.docs[index];
        let name = doc.title();
        icons::file(&painter, pos2(rect.left() + 16.0, rect.center().y), &name);
        let color = if focused { ui.visuals().text_color() } else { theme.text_muted };
        let label = if doc.is_dirty() { format!("{name} ●") } else { name };
        painter.text(pos2(rect.left() + 30.0, rect.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(12.5), color);

        let resp = ui.interact(rect, egui::Id::unique(("pane-header", pane)), Sense::click());
        if resp.clicked() && !focused {
            self.switch_pane();
            self.focus_editor = true;
        }
        // Close button on the right.
        let close = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(18.0, 18.0));
        let close_resp = ui.interact(close, egui::Id::unique(("pane-close", pane)), Sense::click()).on_hover_text(t!("Geteilte Ansicht schließen"));
        if close_resp.hovered() {
            ui.painter().rect_filled(close, 4.0, self.theme.hover);
        }
        let c = close.center();
        let s = 4.0;
        let stroke = Stroke::new(1.3, if close_resp.hovered() { ui.visuals().text_color() } else { self.theme.text_faint });
        ui.painter().line_segment([pos2(c.x - s, c.y - s), pos2(c.x + s, c.y + s)], stroke);
        ui.painter().line_segment([pos2(c.x - s, c.y + s), pos2(c.x + s, c.y - s)], stroke);
        close_resp.clicked()
    }

    /// Id of the editor that has (or should get) the keyboard focus.
    pub(super) fn active_editor_id(&self) -> Option<egui::Id> {
        let i = self.active?;
        Some(editor_id(self.docs[i].id, self.active_pane()))
    }
}
