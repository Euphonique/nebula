//! The transfer list: a field in the status bar that opens the list of
//! uploads, downloads, copies and saves with cancel and retry.

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, pos2, vec2};

use super::NebulaApp;
use crate::remote::transfers::{Direction, TransferState};

enum Act {
    Cancel(u64),
    Retry(u64),
    Remove(u64),
    ClearFinished,
}

/// "vor 12 s" / "vor 3 Min." for the list.
fn ago(t: std::time::Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..60 => t!("vor {s} s", s = s),
        60..3600 => t!("vor {value} Min.", value = s / 60),
        _ => t!("vor {value} Std.", value = s / 3600),
    }
}

impl NebulaApp {
    /// Status bar field; returns true if it drew something.
    pub(super) fn transfers_status(&mut self, ui: &mut egui::Ui) -> bool {
        let theme = self.theme.clone();
        let list = &self.remote.transfers;
        if list.items.is_empty() {
            return false;
        }
        let (running, failed) = (list.running(), list.failed());
        let (text, color) = match (running, failed) {
            (0, 0) => (t!("Übertragungen").to_string(), theme.text_faint),
            (0, f) => (if f == 1 { t!("1 Übertragung fehlgeschlagen").into() } else { t!("{f} Übertragungen fehlgeschlagen", f = f) }, theme.danger),
            (1, _) => (t!("1 Übertragung läuft").into(), theme.text_muted),
            (r, _) => (t!("{r} Übertragungen laufen", r = r), theme.text_muted),
        };
        let resp = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if running > 0 {
                    ui.add(egui::Spinner::new().size(11.0).color(theme.accent));
                } else {
                    ui.label(RichText::new("⇅").size(12.0).color(color));
                }
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(color)).sense(Sense::click()))
            })
            .inner;
        if resp.hovered() {
            ui.painter().rect_filled(resp.rect.expand2(vec2(5.0, 2.0)), 4.0, theme.hover);
        }
        let resp = resp.on_hover_text(t!("Übertragungsliste"));
        let mut acts = Vec::new();
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.transfer_list(ui, &mut acts));
        for act in acts {
            match act {
                Act::Cancel(tag) => self.remote.transfers.cancel(tag),
                Act::Remove(tag) => self.remote.transfers.remove(tag),
                Act::ClearFinished => self.remote.transfers.clear_finished(),
                Act::Retry(tag) => {
                    let site_id = self.remote.transfers.items.iter().find(|t| t.tag == tag).map(|t| t.site_id.clone());
                    match site_id.and_then(|id| self.sites.get(&id).cloned()) {
                        Some(site) => {
                            self.ensure_tree(&site);
                            self.remote.retry(&site, tag);
                        }
                        None => self.error(t!("Der Server dieser Übertragung existiert nicht mehr.")),
                    }
                }
            }
        }
        true
    }

    fn transfer_list(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let theme = &self.theme;
        let list = &self.remote.transfers;
        // While something runs the list must keep updating.
        if list.running() > 0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(t!("Übertragungen")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let finished = list.items.iter().any(|t| t.state.finished());
                if ui.add_enabled(finished, egui::Button::new(t!("Erledigte entfernen")).small()).clicked() {
                    acts.push(Act::ClearFinished);
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
            for t in &list.items {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
                let p = ui.painter();
                let arrow = match t.direction() {
                    Direction::Up => "↑",
                    Direction::Down => "↓",
                    Direction::Within => "⇄",
                };
                let (status, color): (String, Color32) = match &t.state {
                    TransferState::Queued => ("wartet".into(), theme.text_faint),
                    TransferState::Running { done, total: Some(total) } if *total > 1 => (t!("läuft – {done} von {total} Dateien", done = done, total = total), theme.accent),
                    TransferState::Running { done, total: None } if *done > 0 => (t!("läuft – {done} Dateien", done = done), theme.accent),
                    TransferState::Running { .. } => (t!("läuft …").into(), theme.accent),
                    TransferState::Done(msg) => (t!("fertig {ago} – {msg}", ago = ago(t.finished.unwrap_or(t.started)), msg = msg), theme.success),
                    TransferState::Failed(msg) => (msg.clone(), theme.danger),
                };
                p.text(pos2(rect.left() + 10.0, rect.top() + 13.0), Align2::CENTER_CENTER, arrow, FontId::proportional(14.0), color);
                let text_right = rect.right() - 100.0;
                let clip = p.with_clip_rect(egui::Rect::from_min_max(rect.min, pos2(text_right, rect.bottom())));
                clip.text(pos2(rect.left() + 24.0, rect.top() + 6.0), Align2::LEFT_TOP, t.name(), FontId::proportional(13.0), theme.foreground);
                clip.text(pos2(rect.left() + 24.0, rect.top() + 25.0), Align2::LEFT_TOP, &status, FontId::proportional(11.0), color);
                // Progress bar for multi-file transfers.
                if let TransferState::Running { done, total: Some(total) } = t.state
                    && total > 1
                {
                    let bar = egui::Rect::from_min_size(pos2(rect.left() + 24.0, rect.bottom() - 4.0), vec2(text_right - rect.left() - 30.0, 3.0));
                    p.rect_filled(bar, 1.5, theme.hover);
                    let w = bar.width() * (done as f32 / total as f32).min(1.0);
                    p.rect_filled(egui::Rect::from_min_size(bar.min, vec2(w, bar.height())), 1.5, theme.accent);
                }
                let mut buttons = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_max(pos2(text_right, rect.top()), rect.max))
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                match &t.state {
                    TransferState::Queued | TransferState::Running { .. } => {
                        let cancelling = t.cancel.load(std::sync::atomic::Ordering::Relaxed);
                        if buttons.add_enabled(!cancelling, egui::Button::new(t!("Abbrechen")).small()).clicked() {
                            acts.push(Act::Cancel(t.tag));
                        }
                    }
                    TransferState::Failed(_) => {
                        if buttons.small_button("✕").on_hover_text(t!("Aus der Liste entfernen")).clicked() {
                            acts.push(Act::Remove(t.tag));
                        }
                        if t.can_retry() {
                            if buttons.button(t!("Wiederholen")).clicked() {
                                acts.push(Act::Retry(t.tag));
                            }
                        } else {
                            buttons.label(RichText::new("Strg+S").small().color(theme.text_faint)).on_hover_text(t!("Zum Wiederholen die Datei erneut speichern"));
                        }
                    }
                    TransferState::Done(_) => {
                        if buttons.small_button("✕").on_hover_text(t!("Aus der Liste entfernen")).clicked() {
                            acts.push(Act::Remove(t.tag));
                        }
                    }
                }
                ui.add(egui::Label::new(RichText::new(t.detail()).size(10.5).color(theme.text_faint)).truncate());
                ui.add_space(4.0);
            }
        });
    }
}
