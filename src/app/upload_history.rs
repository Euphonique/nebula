//! The "Upload-Verlauf" window: operations on a server (saves, uploads,
//! publishes, syncs, deletions) with what they changed, each undoable.

use eframe::egui::{self, Key, RichText};

use super::NebulaApp;
use crate::editor::DocSource;
use crate::remote::Request;
use crate::remote::journal::{self, Change, Entry, Kind, Prior};
use crate::ui::sidebar::FileRef;

pub(super) struct UploadHistoryView {
    site_id: String,
    site_name: String,
    entries: Vec<Entry>,
    selected: usize,
    /// Tag of the running undo; the buttons wait for it.
    busy: Option<u64>,
    /// Files changed on the server since the operation, waiting for a decision.
    conflict: Option<(u64, Vec<String>)>,
}

fn kind_label(kind: Kind) -> String {
    match kind {
        Kind::Save => t!("Gespeichert").into(),
        Kind::Upload => t!("Hochgeladen").into(),
        Kind::Publish => t!("Veröffentlicht").into(),
        Kind::Sync => t!("Synchronisiert").into(),
        Kind::Copy => t!("Kopiert").into(),
        Kind::Delete => t!("Gelöscht").into(),
        Kind::Rename => t!("Umbenannt").into(),
        Kind::Create => t!("Angelegt").into(),
        Kind::Undo => t!("Rückgängig gemacht").into(),
    }
}

/// "Gespeichert · index.php", "Rückgängig gemacht: Veröffentlicht".
fn title(e: &Entry) -> String {
    let what = match (e.kind, e.reverts) {
        (Kind::Undo, Some((_, of))) => t!("Rückgängig gemacht: {what}", what = kind_label(of)),
        (kind, _) => kind_label(kind),
    };
    let name = e.name.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if name.is_empty() { what } else { format!("{what} · {name}") }
}

fn prior_note(prior: &Prior) -> Option<String> {
    match prior {
        Prior::TooLarge(n) => Some(t!("zu groß zum Sichern ({size})", size = super::human_bytes(*n as usize))),
        Prior::Unreadable => Some(t!("nicht gesichert").into()),
        _ => None,
    }
}

/// One line per changed path: what happened and whether it can be restored.
fn describe(a: &Change) -> (String, Option<String>) {
    match a {
        Change::Wrote { prior: Prior::Absent, .. } => (t!("neu").into(), None),
        Change::Wrote { prior, .. } => (t!("geändert").into(), prior_note(prior)),
        Change::Removed { prior } => (t!("gelöscht").into(), prior_note(prior)),
        Change::Mkdir => (t!("Ordner angelegt").into(), None),
        Change::Rmdir => (t!("Ordner gelöscht").into(), None),
        Change::Renamed { to } => (t!("umbenannt in {to}", to = to), None),
    }
}

/// Actions to show: a file written several times appears once, with its first
/// earlier state (the one undo goes back to).
fn shown_actions(e: &Entry) -> Vec<&journal::Action> {
    let mut seen = std::collections::HashSet::new();
    e.actions
        .iter()
        .filter(|a| !matches!(a.change, Change::Wrote { .. } | Change::Removed { .. }) || seen.insert(a.path.as_str()))
        .collect()
}

impl NebulaApp {
    /// Opens the upload history of a server, optionally at an operation.
    pub(super) fn open_upload_history(&mut self, site_id: &str, select: Option<u64>) {
        let Some(site) = self.sites.get(site_id) else { return };
        let entries = journal::list(site_id);
        if entries.is_empty() {
            let hint = if self.settings.upload_history { String::new() } else { t!(" Er ist ausgeschaltet (Einstellungen › Editor).").into() };
            self.info(t!("Für „{site}“ gibt es noch keinen Upload-Verlauf.", site = site.display_name()) + &hint);
            return;
        }
        let selected = match select {
            Some(id) => match entries.iter().position(|e| e.has_id(id)) {
                Some(i) => i,
                None => {
                    self.info(t!("Diese Übertragung hat auf dem Server nichts verändert."));
                    return;
                }
            },
            None => 0,
        };
        self.upload_history = Some(UploadHistoryView { site_id: site_id.to_string(), site_name: site.display_name(), entries, selected, busy: None, conflict: None });
    }

    /// Upload history of the active server document, else of the project's server.
    pub(super) fn open_upload_history_for_context(&mut self) {
        let site = match self.active.and_then(|i| self.docs.get(i)).map(|d| &d.source) {
            Some(DocSource::Remote { site_id, .. }) => Some(site_id.clone()),
            _ => self.publish_target().map(|(_, t)| t.site_id),
        };
        match site.or_else(|| (self.sites.sites.len() == 1).then(|| self.sites.sites[0].id.clone())) {
            Some(id) => self.open_upload_history(&id, None),
            None => self.info(t!("Öffne eine Serverdatei oder rechtsklicke auf einen Server, um seinen Upload-Verlauf zu sehen.")),
        }
    }

    fn send_undo(&mut self, view: &mut UploadHistoryView, entry: u64, force: bool) {
        view.conflict = None;
        view.busy = self.send_remote(&view.site_id.clone(), Request::Undo { entry, force });
    }

    pub(super) fn upload_undo_conflict(&mut self, site_id: &str, entry: u64, paths: Vec<String>) {
        if let Some(view) = self.upload_history.as_mut().filter(|v| v.site_id == site_id) {
            view.busy = None;
            view.conflict = Some((entry, paths));
        }
    }

    /// An undo failed before it could start (e.g. no connection).
    pub(super) fn upload_undo_failed(&mut self, site_id: &str) {
        if let Some(view) = self.upload_history.as_mut().filter(|v| v.site_id == site_id) {
            view.busy = None;
        }
    }

    pub(super) fn upload_undone(&mut self, site_id: &str, restored: usize, skipped: Vec<String>, failed: Vec<String>, paths: Vec<String>) {
        let site_name = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default();
        if let Some(view) = self.upload_history.as_mut().filter(|v| v.site_id == site_id) {
            view.busy = None;
            let id = view.entries.get(view.selected).map(|e| e.id);
            view.entries = journal::list(site_id);
            view.selected = id.and_then(|id| view.entries.iter().position(|e| e.id == id)).unwrap_or(0);
        }
        // Folders shown in the sidebar list their new content.
        let mut dirs: Vec<String> = paths.iter().map(|p| crate::remote::parent(p)).collect();
        dirs.sort();
        dirs.dedup();
        let shown: Vec<String> = match self.sidebar.remote.get(site_id) {
            Some(tree) => dirs.into_iter().filter(|d| tree.dirs.contains_key(d)).collect(),
            None => Vec::new(),
        };
        for dir in shown {
            self.remote_request(site_id, Request::List(dir));
        }
        // Open tabs without own changes show the restored content.
        let reload: Vec<u64> = self
            .docs
            .iter()
            .filter(|d| !d.is_dirty() && matches!(&d.source, DocSource::Remote { site_id: s, path } if s == site_id && paths.contains(path)))
            .map(|d| d.id)
            .collect();
        for id in reload {
            self.reload_remote(id);
        }
        self.forget_published(site_id, &paths);
        let mut msg = t!("{n} Datei(en) auf {site_name} zurückgesetzt", n = restored, site_name = site_name);
        if !skipped.is_empty() {
            msg += &t!(" – {n} ließen sich nicht wiederherstellen (nicht gesichert)", n = skipped.len());
        }
        if failed.is_empty() {
            self.info(msg);
        } else {
            self.error(format!("{msg}\n{}", failed.join("\n")));
        }
    }

    /// Shows a kept earlier version next to the current server file.
    fn compare_prior(&mut self, site_id: &str, path: &str, blob: &str) {
        let name = path.rsplit('/').next().unwrap_or("datei");
        let dir = std::env::temp_dir().join("nebula-upload-history").join(&blob[..blob.len().min(16)]);
        let local = dir.join(name);
        let result = journal::read_blob(site_id, blob).and_then(|data| {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&local, data)
        });
        match result {
            Ok(()) => self.compare_files(FileRef::Local(local), FileRef::Remote { site_id: site_id.to_string(), path: path.to_string() }),
            Err(e) => self.error(t!("Die gesicherte Fassung konnte nicht gelesen werden: {e}", e = e)),
        }
    }

    pub(super) fn upload_history_window(&mut self, ctx: &egui::Context) {
        let Some(mut view) = self.upload_history.take() else { return };
        let theme = self.theme.clone();
        let mut keep = true;
        let mut undo: Option<(u64, bool)> = None;
        let mut compare: Option<(String, String)> = None;
        super::modal(ctx, egui::Id::unique("upload-history")).show(ctx, |ui| {
            ui.set_width(880.0);
            ui.heading(t!("Upload-Verlauf – {site}", site = view.site_name));
            ui.label(
                RichText::new(t!("Vor jedem Überschreiben, Löschen oder Umbenennen auf dem Server merkt sich Nebula den alten Stand. „Rückgängig machen“ stellt ihn für den ganzen Vorgang wieder her."))
                    .small()
                    .color(theme.text_faint),
            );
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    egui::ScrollArea::vertical().id_salt("upload-history-list").max_height(440.0).show(ui, |ui| {
                        for (n, e) in view.entries.iter().enumerate() {
                            let mut job = egui::text::LayoutJob::default();
                            let color = if e.undone { theme.text_faint } else { ui.visuals().text_color() };
                            job.append(&title(e), 0.0, egui::TextFormat::simple(egui::FontId::proportional(13.5), color));
                            let mut sub = format!("\n{} · {}", super::history::when(e.time), t!("{n} Datei(en)", n = e.file_count()));
                            if e.undone {
                                sub += &format!(" · {}", t!("rückgängig gemacht"));
                            }
                            job.append(&sub, 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), theme.text_faint));
                            if ui.add(egui::Button::selectable(n == view.selected, job).min_size(egui::vec2(290.0, 36.0))).clicked() {
                                view.selected = n;
                                view.conflict = None;
                            }
                        }
                    });
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    let Some(e) = view.entries.get(view.selected) else { return };
                    ui.label(RichText::new(title(e)).strong());
                    if !e.name.is_empty() {
                        ui.label(RichText::new(&e.name).small().color(theme.text_faint));
                    }
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("upload-history-actions").max_height(330.0).show(ui, |ui| {
                        for a in shown_actions(e) {
                            let (what, note) = describe(&a.change);
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new(RichText::new(&a.path).monospace().size(12.0)).truncate());
                                ui.label(RichText::new(what).small().color(theme.text_muted));
                                if let Some(note) = note {
                                    ui.label(RichText::new(format!("⚠ {note}")).small().color(theme.danger));
                                }
                                if let Change::Wrote { prior: Prior::Kept(blob), .. } = &a.change
                                    && ui.small_button(t!("Vergleichen")).on_hover_text(t!("Frühere Fassung mit dem aktuellen Stand auf dem Server vergleichen")).clicked()
                                {
                                    compare = Some((a.path.clone(), blob.clone()));
                                }
                            });
                        }
                    });
                    if e.incomplete() > 0 {
                        ui.add_space(4.0);
                        ui.label(RichText::new(t!("Nicht alle Dateien wurden gesichert; diese bleiben beim Rückgängigmachen, wie sie sind.")).small().color(theme.danger));
                    }
                });
            });
            ui.add_space(10.0);
            if let Some((entry, paths)) = view.conflict.clone() {
                egui::Frame::new().fill(theme.hover).corner_radius(6.0).inner_margin(10.0).show(ui, |ui| {
                    ui.label(RichText::new(t!("Diese Dateien wurden seitdem auf dem Server geändert:")).strong());
                    for p in paths.iter().take(8) {
                        ui.label(RichText::new(p).monospace().size(12.0));
                    }
                    if paths.len() > 8 {
                        ui.label(t!("… und {n} weitere", n = paths.len() - 8));
                    }
                    ui.label(RichText::new(t!("Beim Rückgängigmachen gehen diese späteren Änderungen verloren (sie landen aber wieder im Upload-Verlauf).")).small().color(theme.text_faint));
                    ui.horizontal(|ui| {
                        if ui.button(t!("Trotzdem rückgängig machen")).clicked() {
                            undo = Some((entry, true));
                        }
                        if ui.button(t!("Abbrechen")).clicked() {
                            view.conflict = None;
                        }
                    });
                });
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                let entry = view.entries.get(view.selected);
                let can = entry.is_some_and(|e| !e.undone) && view.busy.is_none() && view.conflict.is_none();
                if ui
                    .add_enabled(can, egui::Button::new(t!("Rückgängig machen")))
                    .on_hover_text(t!("Stellt den Stand vor diesem Vorgang auf dem Server wieder her"))
                    .on_disabled_hover_text(t!("Dieser Vorgang wurde schon rückgängig gemacht."))
                    .clicked()
                    && let Some(e) = entry
                {
                    undo = Some((e.id, false));
                }
                if view.busy.is_some() {
                    ui.spinner();
                }
                if ui.button(t!("Schließen")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    keep = false;
                }
            });
        });
        if let Some((entry, force)) = undo {
            self.send_undo(&mut view, entry, force);
        }
        if let Some((path, blob)) = compare {
            let site_id = view.site_id.clone();
            self.compare_prior(&site_id, &path, &blob);
            keep = false;
        }
        if keep {
            self.upload_history = Some(view);
        }
    }
}
