//! Error logs of connected servers: new lines are fetched every few seconds,
//! shown as markers at the line they name (in server files and in local
//! files of a project published there) and listed in the status bar.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, Sense};

use super::NebulaApp;
use crate::editor::DocSource;
use crate::editor::document::{Diagnostic, Origin};
use crate::remote::{self, Request};
use crate::server_log::{self, LogEntry};
use crate::ui::sidebar::{FileRef, RemoteStatus};

/// How often a log is asked for new lines.
const POLL: Duration = Duration::from_secs(5);
/// After a failure (log missing, no permission).
const RETRY: Duration = Duration::from_secs(60);
/// Entries kept at most (oldest go first).
const MAX_ENTRIES: usize = 300;

#[derive(Default)]
struct Watch {
    /// Where to read on; `None` until the first answer (starts at the end).
    offset: Option<u64>,
    /// Tag of the request on its way.
    pending: Option<u64>,
    next: Option<Instant>,
    /// Last failure, reported once.
    error: Option<String>,
}

pub(super) struct Entry {
    site_id: String,
    log: LogEntry,
    /// The file in Nebula's view of the server, once known.
    site_path: Option<String>,
    /// The file was saved since: the marker is gone, the entry stays listed.
    stale: bool,
}

#[derive(Default)]
pub(super) struct ServerLogs {
    /// Per (site, log path).
    watches: HashMap<(String, String), Watch>,
    entries: Vec<Entry>,
    /// Bumped when entries change, so markers are recomputed.
    generation: u64,
    /// Per document: (generation, version) the markers were made for.
    applied: HashMap<u64, (u64, u64)>,
    /// Per site: what the log puts before Nebula's paths (`/home/user`).
    prefixes: HashMap<String, String>,
    /// Errors and warnings since the list was last opened.
    unseen: usize,
}

impl NebulaApp {
    /// Asks the logs of connected servers for new lines; called every frame.
    pub(super) fn server_log_tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut wanted = Vec::new();
        for site in &self.sites.sites {
            let Some(tree) = self.sidebar.remote.get(&site.id) else { continue };
            if tree.status != RemoteStatus::Connected || tree.offline.is_some() || !self.remote.is_connected(&site.id) {
                continue;
            }
            let home = tree.home.clone().unwrap_or_else(|| "/".into());
            for path in site.error_logs.split([',', '\n']).map(str::trim).filter(|p| !p.is_empty()) {
                let path = if path.starts_with('/') { remote::normalize(path) } else { remote::normalize(&remote::join(&home, path)) };
                wanted.push((site.id.clone(), path));
            }
        }
        // Servers that went away or logs taken out of the settings.
        self.server_logs.watches.retain(|k, _| wanted.contains(k));
        for key in wanted {
            let w = self.server_logs.watches.entry(key.clone()).or_default();
            if w.pending.is_some() {
                continue;
            }
            match w.next {
                Some(next) if next > now => {
                    ctx.request_repaint_after(next - now);
                    continue;
                }
                _ => {}
            }
            let offset = w.offset;
            let Some(site) = self.sites.get(&key.0).cloned() else { continue };
            let tag = self.remote.send(&site, Request::TailLog { path: key.1.clone(), offset });
            if let Some(w) = self.server_logs.watches.get_mut(&key) {
                w.pending = Some(tag);
            }
        }
        self.apply_server_markers();
    }

    fn watch_for_tag(&mut self, tag: u64) -> Option<(&(String, String), &mut Watch)> {
        self.server_logs.watches.iter_mut().find(|(_, w)| w.pending == Some(tag))
    }

    /// New lines of a log arrived.
    pub(super) fn server_log_read(&mut self, site_id: &str, tag: u64, offset: u64, text: &str) {
        let Some((_, w)) = self.watch_for_tag(tag) else { return };
        let first = w.offset.is_none();
        w.pending = None;
        w.offset = Some(offset);
        w.next = Some(Instant::now() + POLL);
        w.error = None;
        if first || text.is_empty() {
            return;
        }
        let logs = &mut self.server_logs;
        for log in server_log::parse(text) {
            if log.severity <= 2 {
                logs.unseen += 1;
            }
            logs.entries.push(Entry { site_id: site_id.to_string(), log, site_path: None, stale: false });
        }
        if logs.entries.len() > MAX_ENTRIES {
            let n = logs.entries.len() - MAX_ENTRIES;
            logs.entries.drain(..n);
        }
        logs.generation += 1;
    }

    /// A log request failed; true if it was one (no general error message).
    pub(super) fn server_log_failed(&mut self, site_id: &str, tag: u64, message: &str) -> bool {
        let Some(((_, path), w)) = self.watch_for_tag(tag) else { return false };
        let path = path.clone();
        w.pending = None;
        w.next = Some(Instant::now() + RETRY);
        let first = w.error.is_none();
        w.error = Some(message.to_string());
        if first {
            let site = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default();
            self.error(t!("Fehlerprotokoll {path} auf {site} kann nicht gelesen werden: {message}", path = path, site = site, message = message));
        }
        true
    }

    /// After saving a file its markers go (the error may be fixed now).
    pub(super) fn server_log_saved(&mut self, doc_id: u64) {
        let Some(i) = self.doc_index(doc_id) else { return };
        let targets = self.site_paths_of(&self.docs[i].source);
        let logs = &mut self.server_logs;
        let mut changed = false;
        for e in logs.entries.iter_mut().filter(|e| !e.stale) {
            if let Some(p) = &e.site_path
                && targets.iter().any(|(s, t)| *s == e.site_id && t == p)
            {
                e.stale = true;
                changed = true;
            }
        }
        if changed {
            logs.generation += 1;
        }
    }

    /// Server paths a document stands for: its own, or for a local file of a
    /// published project the path it is published to.
    fn site_paths_of(&self, source: &DocSource) -> Vec<(String, String)> {
        match source {
            DocSource::Remote { site_id, path } => vec![(site_id.clone(), path.clone())],
            DocSource::Local(file) => self
                .settings
                .publish
                .iter()
                .filter_map(|(root, t)| {
                    let rel = file.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
                    Some((t.site_id.clone(), remote::join(&remote::normalize(&t.remote_path), &rel)))
                })
                .collect(),
            DocSource::Untitled(_) => Vec::new(),
        }
    }

    /// Finds the files of entries in Nebula's view of their server.
    fn resolve_server_paths(&mut self) {
        let logs = &mut self.server_logs;
        for e in logs.entries.iter_mut().filter(|e| e.site_path.is_none()) {
            let Some((log_path, _)) = &e.log.file else { continue };
            if let Some(prefix) = logs.prefixes.get(&e.site_id) {
                if let Some(p) = log_path.strip_prefix(prefix.as_str()) {
                    e.site_path = Some(p.to_string());
                }
                continue;
            }
            // Known: listed in the sidebar or open in a tab.
            let tree = self.sidebar.remote.get(&e.site_id);
            let docs = &self.docs;
            let site_id = e.site_id.clone();
            let known = |p: &str| {
                tree.is_some_and(|t| t.dirs.get(&remote::parent(p)).is_some_and(|l| l.iter().any(|x| x.path == p)))
                    || docs.iter().any(|d| matches!(&d.source, DocSource::Remote { site_id: s, path } if *s == site_id && path == p))
            };
            if let Some((path, prefix)) = server_log::to_site_path(log_path, known) {
                e.site_path = Some(path);
                logs.prefixes.insert(e.site_id.clone(), prefix);
            }
        }
    }

    /// Puts entries as markers into the documents they belong to.
    fn apply_server_markers(&mut self) {
        let generation = self.server_logs.generation;
        if self.docs.iter().all(|d| self.server_logs.applied.get(&d.id) == Some(&(generation, d.version))) {
            return;
        }
        self.resolve_server_paths();
        for i in 0..self.docs.len() {
            let (id, version) = (self.docs[i].id, self.docs[i].version);
            if self.server_logs.applied.get(&id) == Some(&(generation, version)) {
                continue;
            }
            self.server_logs.applied.insert(id, (generation, version));
            let targets = self.site_paths_of(&self.docs[i].source);
            let doc = &mut self.docs[i];
            let mut marks = Vec::new();
            for e in self.server_logs.entries.iter().filter(|e| !e.stale) {
                let (Some(p), Some((_, line))) = (&e.site_path, &e.log.file) else { continue };
                if !targets.iter().any(|(s, t)| *s == e.site_id && t == p) {
                    continue;
                }
                let line = line.saturating_sub(1).min(doc.len_lines().saturating_sub(1));
                let start = doc.line_to_char(line);
                let end = doc.line_end(line).max(start + 1);
                let when = e.log.time.as_deref().map(|t| format!(" ({t})")).unwrap_or_default();
                marks.push(Diagnostic { range: start..end, severity: e.log.severity, message: format!("{}{when}: {}", t!("Server"), e.log.message), origin: Origin::ServerLog });
            }
            let had = doc.view.diagnostics.iter().any(|d| d.origin == Origin::ServerLog);
            if marks.is_empty() && !had {
                continue;
            }
            doc.view.diagnostics.retain(|d| d.origin != Origin::ServerLog);
            doc.view.diagnostics.extend(marks);
            doc.view.diagnostics.sort_by_key(|d| d.range.start);
        }
        let ids: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.server_logs.applied.retain(|id, _| ids.contains(id));
    }

    /// Opens the file of an entry at its line: the local project file if the
    /// project is published there, else the server file.
    fn open_log_entry(&mut self, site_id: &str, site_path: &str, line: usize) {
        let local = self.settings.publish.iter().find_map(|(root, t)| {
            let base = remote::normalize(&t.remote_path);
            let rel = site_path.strip_prefix(&base)?.strip_prefix('/')?;
            let file = std::path::Path::new(root).join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            (t.site_id == site_id && file.is_file()).then_some(file)
        });
        let file = match local {
            Some(f) => FileRef::Local(f),
            None => FileRef::Remote { site_id: site_id.to_string(), path: site_path.to_string() },
        };
        self.open_file_at(file, line, 0);
    }

    /// The newest entries for agents (MCP), optionally of one server (by name).
    pub(super) fn server_log_json(&self, site: Option<&str>) -> serde_json::Value {
        let names: HashMap<String, String> = self.sites.sites.iter().map(|s| (s.id.clone(), s.display_name())).collect();
        let entries: Vec<serde_json::Value> = self
            .server_logs
            .entries
            .iter()
            .rev()
            .filter(|e| site.is_none_or(|s| names.get(&e.site_id).is_some_and(|n| n.eq_ignore_ascii_case(s))))
            .take(50)
            .map(|e| {
                let (file, line) = match &e.log.file {
                    Some((p, l)) => (Some(e.site_path.clone().unwrap_or_else(|| p.clone())), Some(*l)),
                    None => (None, None),
                };
                let level = match e.log.severity {
                    1 => "error",
                    2 => "warning",
                    _ => "notice",
                };
                serde_json::json!({ "server": names.get(&e.site_id), "time": e.log.time, "level": level, "message": e.log.message, "file": file, "line": line, "saved_since": e.stale })
            })
            .collect();
        let watched: Vec<String> = self.server_logs_watched();
        serde_json::json!({ "watched_logs": watched, "entries": entries, "note": if watched.is_empty() { "No error log is watched: set one in the server settings (field Fehlerprotokoll / error log) and connect to the server." } else { "Newest first; only lines written since Nebula connected." } })
    }

    fn server_logs_watched(&self) -> Vec<String> {
        self.server_logs.watches.keys().map(|(site, path)| format!("{}:{path}", self.sites.get(site).map(|s| s.display_name()).unwrap_or_default())).collect()
    }

    /// Status bar field with the entries; nothing while there are none.
    pub(super) fn server_log_status(&mut self, ui: &mut egui::Ui) {
        if self.server_logs.entries.is_empty() {
            return;
        }
        let theme = self.theme.clone();
        let unseen = self.server_logs.unseen;
        let errors = self.server_logs.entries.iter().filter(|e| e.log.severity == 1).count();
        let color = if unseen == 0 { theme.text_faint } else if errors > 0 { theme.danger } else { theme.modified };
        let text = if unseen > 0 { t!("Server: {n} neu", n = unseen) } else { t!("Serverprotokoll").into() };
        let resp = ui.add(egui::Label::new(RichText::new(format!("⚠ {text}")).size(11.5).color(color)).sense(Sense::click())).on_hover_text(t!("Meldungen aus den Fehlerprotokollen der Server"));
        let mut open: Option<(String, String, usize)> = None;
        let mut clear = false;
        let names: HashMap<String, String> = self.sites.sites.iter().map(|s| (s.id.clone(), s.display_name())).collect();
        let popup = egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(640.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(t!("Serverprotokoll")).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t!("Leeren")).clicked() {
                        clear = true;
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                for e in self.server_logs.entries.iter().rev() {
                    let color = match e.log.severity {
                        1 => theme.danger,
                        2 => theme.modified,
                        _ => theme.text_muted,
                    };
                    let color = if e.stale { theme.text_faint } else { color };
                    ui.add(egui::Label::new(RichText::new(&e.log.message).size(12.5).color(color)).wrap());
                    ui.horizontal(|ui| {
                        let mut meta = names.get(&e.site_id).cloned().unwrap_or_default();
                        if let Some(t) = &e.log.time {
                            meta = format!("{meta} · {t}");
                        }
                        ui.label(RichText::new(meta).small().color(theme.text_faint));
                        if let Some((log_path, line)) = &e.log.file {
                            let path = e.site_path.clone().unwrap_or_else(|| log_path.clone());
                            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                            if ui.link(RichText::new(format!("{name}:{line}")).small()).on_hover_text(&path).clicked() {
                                open = Some((e.site_id.clone(), path, *line));
                            }
                        }
                        if e.stale {
                            ui.label(RichText::new(t!("seitdem gespeichert")).small().color(theme.text_faint));
                        }
                    });
                    ui.add_space(4.0);
                }
            });
        });
        if popup.is_some() {
            self.server_logs.unseen = 0;
        }
        if clear {
            self.server_logs.entries.clear();
            self.server_logs.unseen = 0;
            self.server_logs.generation += 1;
        }
        if let Some((site_id, path, line)) = open {
            self.open_log_entry(&site_id, &path, line.saturating_sub(1));
        }
    }
}
