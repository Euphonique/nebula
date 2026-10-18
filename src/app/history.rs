//! Autosave and the "Lokaler Verlauf" window (earlier versions of a file,
//! compared with the current text and restorable).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, RichText};

use super::NebulaApp;
use crate::diff::{DiffLine, unified};
use crate::editor::DocSource;
use crate::history::{self, Entry};
use crate::settings::AutoSave;

#[derive(Default)]
pub(super) struct AutosaveState {
    /// Per document: version last seen and when it changed.
    edits: HashMap<u64, (u64, Instant)>,
    /// Per document: version autosave last tried, so a failed or refused
    /// save (conflict) is not retried until the text changes again.
    attempted: HashMap<u64, u64>,
    was_focused: bool,
    last_active: Option<u64>,
    /// The current save comes from autosave (no whitespace trimming).
    pub running: bool,
}

pub(super) struct HistoryView {
    doc_id: u64,
    title: String,
    entries: Vec<Entry>,
    selected: usize,
    /// Diff of the selected entry against the document at a version.
    diff: Option<(usize, u64, Result<Arc<Vec<DiffLine>>, String>)>,
}

/// "vor 5 Min." for recent times, else date and time.
fn when(ms: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(ms);
    let ago = now.saturating_sub(ms) / 1000;
    match ago {
        0..60 => "gerade eben".into(),
        60..3600 => format!("vor {} Min.", ago / 60),
        3600..86_400 => format!("vor {} Std.", ago / 3600),
        _ => {
            let t = crate::win32::local_time(ms);
            format!("{:02}.{:02}.{} {:02}:{:02}", t.day, t.month, t.year, t.hour, t.minute)
        }
    }
}

impl NebulaApp {
    /// Stores a version in the local history if it is switched on.
    pub(super) fn record_history(&self, source: &DocSource, bytes: &[u8], kind: history::Kind) {
        let e = &self.settings.editor;
        if e.local_history {
            history::record(source, bytes, kind, e.history_max);
        }
    }

    /// Saves documents according to the autosave setting; called every frame.
    pub(super) fn autosave_tick(&mut self, ctx: &egui::Context) {
        let mode = self.settings.editor.autosave;
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let active = self.active.and_then(|i| self.docs.get(i)).map(|d| d.id);
        let lost_focus = std::mem::replace(&mut self.autosave.was_focused, focused) && !focused;
        let left_doc = std::mem::replace(&mut self.autosave.last_active, active).filter(|id| Some(*id) != active);
        if mode == AutoSave::Off || self.dialog.is_some() {
            return;
        }
        let delay = Duration::from_secs_f32(self.settings.editor.autosave_delay.max(0.5));
        let now = Instant::now();
        let mut due = Vec::new();
        for (i, doc) in self.docs.iter().enumerate() {
            let eligible = doc.is_dirty()
                && !doc.saving
                && match doc.source {
                    DocSource::Local(_) => true,
                    DocSource::Remote { .. } => self.settings.editor.autosave_remote,
                    DocSource::Untitled(_) => false,
                }
                && self.autosave.attempted.get(&doc.id) != Some(&doc.version);
            if !eligible {
                continue;
            }
            match mode {
                AutoSave::Delay => {
                    let entry = self.autosave.edits.entry(doc.id).or_insert((doc.version, now));
                    if entry.0 != doc.version {
                        *entry = (doc.version, now);
                    }
                    let waited = now.duration_since(entry.1);
                    if waited >= delay {
                        due.push(i);
                    } else {
                        ctx.request_repaint_after(delay - waited);
                    }
                }
                AutoSave::FocusChange => {
                    if lost_focus || left_doc == Some(doc.id) {
                        due.push(i);
                    }
                }
                AutoSave::Off => {}
            }
        }
        for i in due {
            let id = self.docs[i].id;
            self.autosave.attempted.insert(id, self.docs[i].version);
            // A file changed by another program needs a decision; leave it to Strg+S.
            if matches!(self.docs[i].source, DocSource::Local(_)) && self.changed_on_disk(i) {
                continue;
            }
            self.autosave.running = true;
            self.save_doc_with(i, false, false);
            self.autosave.running = false;
            if self.dialog.is_some() {
                break;
            }
        }
        let ids: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.autosave.edits.retain(|id, _| ids.contains(id));
        self.autosave.attempted.retain(|id, _| ids.contains(id));
    }

    pub(super) fn open_history(&mut self, i: usize) {
        let Some(doc) = self.docs.get(i) else { return };
        if !self.settings.editor.local_history {
            self.info("Der lokale Verlauf ist ausgeschaltet (Einstellungen → Editor).");
            return;
        }
        let entries = history::list(&doc.source);
        if entries.is_empty() {
            self.info(format!("Für „{}“ gibt es noch keinen lokalen Verlauf.", doc.title()));
            return;
        }
        // Start at the newest version that differs from the current text.
        let current = doc.to_bytes();
        let selected = entries.iter().position(|e| e.size != current.len() as u64 || e.read().is_ok_and(|b| b != current)).unwrap_or(0);
        self.history_view = Some(HistoryView { doc_id: doc.id, title: doc.title(), entries, selected, diff: None });
    }

    /// The history window; returns false when it should close.
    pub(super) fn history_window(&mut self, ctx: &egui::Context) {
        let Some(mut view) = self.history_view.take() else { return };
        let Some(i) = self.doc_index(view.doc_id) else { return };
        let version = self.docs[i].version;
        if view.diff.as_ref().is_none_or(|(sel, v, _)| *sel != view.selected || *v != version) {
            let doc = &mut self.docs[i];
            let rows = view.entries[view.selected]
                .read()
                .map(|bytes| Arc::new(unified(&doc.encoding.decode(&bytes).replace("\r\n", "\n"), &doc.text(), 3)))
                .map_err(|e| format!("Diese Fassung konnte nicht gelesen werden: {e}"));
            view.diff = Some((view.selected, version, rows));
        }
        let theme = self.theme.clone();
        let autosave = self.settings.editor.autosave != AutoSave::Off;
        let mut keep = true;
        let mut restore = false;
        super::modal(ctx, egui::Id::unique("local-history")).show(ctx, |ui| {
            ui.set_width(860.0);
            ui.heading(format!("Lokaler Verlauf – {}", view.title));
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(220.0);
                    egui::ScrollArea::vertical().id_salt("history-list").max_height(460.0).show(ui, |ui| {
                        for (n, e) in view.entries.iter().enumerate() {
                            let mut job = egui::text::LayoutJob::default();
                            job.append(&when(e.time), 0.0, egui::TextFormat::simple(egui::FontId::proportional(13.5), ui.visuals().text_color()));
                            job.append(
                                &format!("\n{} · {}", e.kind.label(), super::human_bytes(e.size as usize)),
                                0.0,
                                egui::TextFormat::simple(egui::FontId::proportional(11.0), theme.text_faint),
                            );
                            if ui.add(egui::Button::selectable(n == view.selected, job).min_size(egui::vec2(210.0, 36.0))).clicked() {
                                view.selected = n;
                            }
                        }
                    });
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_max_height(460.0);
                    match view.diff.as_ref().map(|(_, _, d)| d) {
                        Some(Ok(rows)) => super::compare::diff_view(ui, &theme, rows, "diese Fassung", "aktueller Stand", 430.0),
                        Some(Err(e)) => {
                            ui.colored_label(theme.danger, e);
                        }
                        None => {}
                    }
                });
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let has_diff = matches!(view.diff.as_ref().map(|(_, _, d)| d), Some(Ok(rows)) if !rows.is_empty());
                if ui
                    .add_enabled(has_diff, egui::Button::new("Wiederherstellen"))
                    .on_hover_text("Ersetzt den Text im Editor durch diese Fassung (rückgängig mit Strg+Z)")
                    .clicked()
                {
                    restore = true;
                }
                if ui.button("Schließen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    keep = false;
                }
                let hint = if autosave { "Wird danach automatisch gespeichert." } else { "Gespeichert wird erst mit Strg+S." };
                ui.label(RichText::new(hint).small().color(theme.text_faint));
            });
        });
        if restore
            && let Ok(bytes) = view.entries[view.selected].read()
        {
            let doc = &mut self.docs[i];
            let text = doc.encoding.decode(&bytes);
            doc.replace_text(&text);
            self.info(format!("Fassung von {} wiederhergestellt", when(view.entries[view.selected].time)));
            keep = false;
        }
        if keep {
            self.history_view = Some(view);
        }
    }
}
