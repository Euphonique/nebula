//! The change watcher window and its checks: on connecting and every hour
//! the watched folder of a server is scanned and compared with its state.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, RichText, Sense};

use super::NebulaApp;
use crate::remote::{self, Request, ScannedFile};
use crate::server_watch::{self, Found, Kind};
use crate::ui::sidebar::{FileRef, RemoteStatus};

/// Checks while connected.
const INTERVAL: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// Take the state as reference.
    Take,
    Check,
}

/// Outcome of the last check of a server.
pub(super) struct Report {
    found: Vec<Found>,
    /// The scan, to take accepted changes from.
    files: Vec<ScannedFile>,
}

#[derive(Default)]
pub(super) struct WatchState {
    /// Running scans: tag → (site, purpose).
    scans: HashMap<u64, (String, Purpose)>,
    reports: HashMap<String, Report>,
    last_check: HashMap<String, Instant>,
    /// Server whose window is open, and the fields for taking the state.
    view: Option<String>,
    root: String,
    exclude: String,
}

/// `01.11.2026 14:05` for seconds since 1970.
fn date(secs: i64) -> String {
    let t = crate::platform::local_time(secs.max(0) as u64 * 1000);
    format!("{:02}.{:02}.{} {:02}:{:02}", t.day, t.month, t.year, t.hour, t.minute)
}

impl NebulaApp {
    fn watch_scan(&mut self, site_id: &str, root: &str, exclude: &str, purpose: Purpose) {
        // Plain names are skipped on the server already; patterns afterwards.
        let names: Vec<String> = exclude.split(',').map(str::trim).filter(|p| !p.is_empty() && !p.contains('*')).map(str::to_string).collect();
        if let Some(tag) = self.send_remote(site_id, Request::Scan { root: remote::normalize(root), exclude: names }) {
            self.server_watch.scans.insert(tag, (site_id.to_string(), purpose));
            self.server_watch.last_check.insert(site_id.to_string(), Instant::now());
        }
    }

    /// Checks watched servers after connecting and then every hour.
    pub(super) fn server_watch_tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let due: Vec<(String, String, String)> = self
            .sites
            .sites
            .iter()
            .filter(|s| self.sidebar.remote.get(&s.id).is_some_and(|t| t.status == RemoteStatus::Connected && t.offline.is_none()))
            .filter(|s| !self.server_watch.scans.values().any(|(id, _)| *id == s.id))
            .filter(|s| self.server_watch.last_check.get(&s.id).is_none_or(|t| now.duration_since(*t) >= INTERVAL))
            .filter_map(|s| server_watch::load(&s.id).map(|b| (s.id.clone(), b.root, b.exclude)))
            .collect();
        for (site, root, exclude) in due {
            self.watch_scan(&site, &root, &exclude, Purpose::Check);
        }
        if !self.server_watch.last_check.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(60));
        }
    }

    /// A scan arrived: handled if it is the watcher's, else handed back.
    pub(super) fn watch_take_scan(&mut self, tag: u64, files: Vec<ScannedFile>, truncated: bool) -> Option<(Vec<ScannedFile>, bool)> {
        if !self.server_watch.scans.contains_key(&tag) {
            return Some((files, truncated));
        }
        self.watch_scanned(tag, files, truncated);
        None
    }

    fn watch_scanned(&mut self, tag: u64, files: Vec<ScannedFile>, truncated: bool) -> bool {
        let Some((site_id, purpose)) = self.server_watch.scans.remove(&tag) else { return false };
        let site_name = self.sites.get(&site_id).map(|s| s.display_name()).unwrap_or_default();
        if truncated {
            self.error(t!("Änderungswächter für {site}: Der Ordner enthält zu viele Dateien – große Ordner (z. B. vendor, uploads) in die Ausschlussliste aufnehmen.", site = site_name));
            return true;
        }
        match purpose {
            Purpose::Take => {
                let (root, exclude) = (self.server_watch.root.clone(), self.server_watch.exclude.clone());
                match server_watch::take(&site_id, &root, &exclude, &files) {
                    Ok(b) => {
                        self.server_watch.reports.remove(&site_id);
                        self.info(t!("Änderungswächter für {site} eingerichtet ({n} Dateien).", site = site_name, n = b.files.len()));
                    }
                    Err(e) => self.error(t!("Der Stand konnte nicht gespeichert werden: {e}", e = e)),
                }
            }
            Purpose::Check => {
                let Some(found) = server_watch::check(&site_id, &files) else { return true };
                if found.is_empty() {
                    self.server_watch.reports.remove(&site_id);
                    return true;
                }
                let code = found.iter().filter(|f| f.is_code()).count();
                let mut msg = t!("Änderungswächter: {n} Datei(en) auf {site} wurden außerhalb von Nebula geändert", n = found.len(), site = site_name);
                if code > 0 {
                    msg += &t!(", darunter {code} mit Code (PHP, JS, .htaccess …)", code = code);
                }
                self.error(msg + ".");
                self.server_watch.reports.insert(site_id, Report { found, files });
            }
        }
        true
    }

    pub(super) fn watch_failed(&mut self, tag: u64) -> bool {
        self.server_watch.scans.remove(&tag).is_some()
    }

    pub(super) fn open_server_watch(&mut self, site_id: &str) {
        let Some(site) = self.sites.get(site_id).cloned() else { return };
        self.ensure_tree(&site);
        let w = &mut self.server_watch;
        w.view = Some(site_id.to_string());
        match server_watch::load(site_id) {
            Some(b) => (w.root, w.exclude) = (b.root, b.exclude),
            None => {
                w.root = String::new();
                w.exclude = server_watch::DEFAULT_EXCLUDE.to_string();
            }
        }
    }

    /// Status bar field while there are unexplained changes.
    pub(super) fn server_watch_status(&mut self, ui: &mut egui::Ui) {
        let n: usize = self.server_watch.reports.values().map(|r| r.found.len()).sum();
        if n == 0 {
            return;
        }
        let text = RichText::new(t!("{n} fremde Änderungen", n = n)).size(11.5).color(self.theme.danger);
        if ui.add(egui::Label::new(text).sense(Sense::click())).on_hover_text(t!("Änderungswächter: Dateien, die außerhalb von Nebula geändert wurden")).clicked()
            && let Some(site) = self.server_watch.reports.keys().next().cloned()
        {
            self.open_server_watch(&site);
        }
    }

    pub(super) fn server_watch_window(&mut self, ctx: &egui::Context) {
        let Some(site_id) = self.server_watch.view.clone() else { return };
        let Some(site) = self.sites.get(&site_id).cloned() else {
            self.server_watch.view = None;
            return;
        };
        let theme = self.theme.clone();
        let baseline = server_watch::load(&site_id);
        let home = self.sidebar.remote.get(&site_id).and_then(|t| t.home.clone());
        let connected = self.sidebar.remote.get(&site_id).is_some_and(|t| t.status == RemoteStatus::Connected);
        let busy = self.server_watch.scans.values().any(|(id, _)| *id == site_id);
        if self.server_watch.root.is_empty()
            && let Some(h) = &home
        {
            self.server_watch.root = h.clone();
        }
        let mut close = false;
        let mut take = false;
        let mut check = false;
        let mut off = false;
        let mut accept: Vec<String> = Vec::new();
        let mut open: Option<String> = None;
        super::modal(ctx, egui::Id::unique("server-watch")).show(ctx, |ui| {
            ui.set_width(760.0);
            ui.heading(t!("Änderungswächter – {site}", site = site.display_name()));
            ui.label(
                RichText::new(t!("Nebula merkt sich Größe und Änderungszeit aller Dateien eines Serverordners und meldet beim Verbinden und stündlich, was sich ohne Nebula geändert hat – etwa durch FileZilla, ein Update oder eingeschleusten Schadcode. Eigene Änderungen trägt Nebula selbst nach."))
                    .small()
                    .color(theme.text_faint),
            );
            ui.add_space(8.0);
            let w = &mut self.server_watch;
            match &baseline {
                None => {
                    egui::Grid::new("watch-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label(t!("Ordner"));
                        ui.add(egui::TextEdit::singleline(&mut w.root).hint_text("/httpdocs").desired_width(460.0));
                        ui.end_row();
                        ui.label(t!("Ausschließen"));
                        ui.add(egui::TextEdit::singleline(&mut w.exclude).desired_width(460.0))
                            .on_hover_text(t!("Namen von Dateien und Ordnern, durch Komma getrennt; *.log und tmp* gehen auch"));
                        ui.end_row();
                    });
                }
                Some(b) => {
                    let taken = date((b.taken / 1000) as i64);
                    let checked = date((b.checked / 1000) as i64);
                    ui.label(t!("Ordner {root} · {n} Dateien · Stand vom {taken} · zuletzt geprüft {checked}", root = b.root, n = b.files.len(), taken = taken, checked = checked));
                    ui.label(RichText::new(t!("Ausgeschlossen: {list}", list = b.exclude)).small().color(theme.text_faint));
                }
            }
            ui.add_space(8.0);
            if let Some(report) = w.reports.get(&site_id) {
                ui.label(RichText::new(t!("Außerhalb von Nebula geändert:")).strong());
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for f in &report.found {
                        ui.horizontal(|ui| {
                            let (what, color) = match f.kind {
                                Kind::Added => (t!("neu"), theme.accent),
                                Kind::Modified => (t!("geändert"), theme.modified),
                                Kind::Removed => (t!("gelöscht"), theme.text_muted),
                            };
                            ui.label(RichText::new(what.to_string()).small().color(color));
                            let path = RichText::new(&f.rel).monospace().size(12.0);
                            ui.add(egui::Label::new(if f.is_code() { path.color(theme.danger) } else { path }).truncate());
                            if f.is_code() {
                                ui.label(RichText::new(t!("Code")).small().color(theme.danger)).on_hover_text(t!("Läuft auf dem Server oder im Browser – genau ansehen, ob die Änderung gewollt ist."));
                            }
                            if let Some(m) = f.mtime {
                                ui.label(RichText::new(date(m)).small().color(theme.text_faint));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button(t!("In Ordnung")).on_hover_text(t!("Als bekannt übernehmen")).clicked() {
                                    accept.push(f.rel.clone());
                                }
                                if f.kind != Kind::Removed && ui.small_button(t!("Öffnen")).clicked() {
                                    open = Some(f.rel.clone());
                                }
                            });
                        });
                    }
                });
                if ui.button(t!("Alle als bekannt übernehmen")).clicked() {
                    accept = report.found.iter().map(|f| f.rel.clone()).collect();
                }
            } else if baseline.is_some() && !busy {
                ui.label(RichText::new(t!("Keine fremden Änderungen seit der letzten Prüfung.")).color(theme.success));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = connected && !busy;
                match baseline {
                    None => {
                        if ui.add_enabled(ready && !w.root.trim().is_empty(), egui::Button::new(t!("Stand aufnehmen"))).clicked() {
                            take = true;
                        }
                    }
                    Some(_) => {
                        if ui.add_enabled(ready, egui::Button::new(t!("Jetzt prüfen"))).clicked() {
                            check = true;
                        }
                        if ui.add_enabled(ready, egui::Button::new(t!("Stand neu aufnehmen"))).on_hover_text(t!("Den aktuellen Serverstand als bekannt übernehmen")).clicked() {
                            take = true;
                        }
                        if ui.button(t!("Ausschalten")).clicked() {
                            off = true;
                        }
                    }
                }
                if busy {
                    ui.spinner();
                    ui.label(RichText::new(t!("Ordner wird eingelesen …")).small().color(theme.text_faint));
                } else if !connected {
                    ui.label(RichText::new(t!("Verbinde …")).small().color(theme.text_faint));
                }
                if ui.button(t!("Schließen")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    close = true;
                }
            });
        });
        if take {
            let (root, exclude) = (self.server_watch.root.clone(), self.server_watch.exclude.clone());
            self.watch_scan(&site_id, &root, &exclude, Purpose::Take);
        }
        if check && let Some(b) = &baseline {
            self.watch_scan(&site_id, &b.root, &b.exclude, Purpose::Check);
        }
        if off {
            server_watch::remove(&site_id);
            self.server_watch.reports.remove(&site_id);
            self.info(t!("Änderungswächter für {site} ausgeschaltet.", site = site.display_name()));
        }
        if !accept.is_empty()
            && let Some(report) = self.server_watch.reports.get_mut(&site_id)
        {
            server_watch::accept(&site_id, &accept, &report.files);
            report.found.retain(|f| !accept.contains(&f.rel));
            if report.found.is_empty() {
                self.server_watch.reports.remove(&site_id);
            }
        }
        if let (Some(rel), Some(b)) = (open, &baseline) {
            self.open_file_at(FileRef::Remote { site_id: site_id.clone(), path: remote::join(&b.root, &rel) }, 0, 0);
            close = true;
        }
        if close {
            self.server_watch.view = None;
        }
    }
}
