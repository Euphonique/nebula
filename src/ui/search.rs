//! Project-wide search (and replace): in the local project folder or, over
//! SSH, in a folder on a server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Receiver;
use eframe::egui::{self, Align2, FontId, RichText, Sense, Ui, pos2, vec2};
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use regex::{Regex, RegexBuilder};

use super::icons;
use super::sidebar::{DirRef, FileRef};
use crate::remote::HostKeyPrompt;
use crate::remote::search::{self as remote_search, SearchEvent};
use crate::settings::{Protocol, Sites};
use crate::theme::Theme;

const MAX_MATCHES: usize = 5000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const VCS_DIRS: &[&str] = &[".git", ".svn", ".hg"];
const SKIP_DIRS: &[&str] = &["node_modules", "vendor", "target", ".idea", ".vs", "dist", ".cache"];

/// `*.php, !vendor/` → `["*.php", "!vendor/"]` (commas or spaces separate).
fn split_patterns(s: &str) -> Vec<String> {
    s.split([',', ';', ' ']).map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect()
}

#[derive(Clone)]
pub struct LineMatch {
    /// Zero-based line number.
    pub line: usize,
    pub text: String,
    /// Byte ranges within `text`.
    pub ranges: Vec<std::ops::Range<usize>>,
}

#[derive(Clone)]
pub struct FileMatches {
    pub file: FileRef,
    /// Path shown in the list, relative to the searched folder.
    pub rel: String,
    pub lines: Vec<LineMatch>,
}

/// What the search panel asks the app to do.
pub enum SearchAction {
    Open { file: FileRef, line: usize, column: usize },
    ReplaceAll { files: Vec<PathBuf>, regex: Regex, replacement: String },
    /// The server's SSH host key is not trusted yet.
    HostKey { site_id: String, prompt: HostKeyPrompt },
}

/// Where the search runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    /// A folder in the local project, relative with `/` (empty = whole project).
    Local { dir: String },
    /// A folder on a server (empty = the login folder).
    Remote { site_id: String, path: String },
}

impl Default for Scope {
    fn default() -> Self {
        Scope::Local { dir: String::new() }
    }
}

impl Scope {
    /// Scope for a folder from the sidebar; local folders outside the project
    /// fall back to the whole project.
    pub fn for_dir(dir: &DirRef, project: Option<&Path>) -> Self {
        match dir {
            DirRef::Local(p) => {
                let rel = project.and_then(|root| p.strip_prefix(root).ok());
                Scope::Local { dir: rel.map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default() }
            }
            DirRef::Remote { site_id, path } => Scope::Remote { site_id: site_id.clone(), path: path.clone() },
        }
    }
}

struct RemoteRun {
    search: remote_search::RemoteSearch,
    site_id: String,
    root: String,
    lines: usize,
    /// The site is FTP; SSH goes to port 22, which may not be open.
    via_ftp: bool,
}

#[derive(Default)]
pub struct SearchPanel {
    pub scope: Scope,
    remote: Option<RemoteRun>,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    pub show_replace: bool,
    /// File patterns, e.g. `*.php, !vendor/`.
    pub files: String,
    /// Also search files hidden by `.gitignore` and folders like `node_modules`.
    pub include_ignored: bool,
    pub results: Vec<FileMatches>,
    pub collapsed: std::collections::HashSet<String>,
    pub error: Option<String>,
    pub focus_query: bool,
    running: Option<(Receiver<FileMatches>, Arc<AtomicBool>)>,
    truncated: bool,
    /// The user stopped the last search.
    cancelled: bool,
    /// Scope changed from outside; search again on the next frame.
    restart_pending: bool,
}

impl SearchPanel {
    pub fn build_regex(&self) -> Result<Regex, String> {
        let mut p = if self.use_regex { self.query.clone() } else { regex::escape(&self.query) };
        if self.whole_word {
            p = format!(r"\b(?:{p})\b");
        }
        RegexBuilder::new(&p)
            .case_insensitive(!self.case_sensitive)
            .build()
            .map_err(|e| e.to_string().lines().last().unwrap_or(t!("Ungültiger Ausdruck")).to_string())
    }

    /// Files to search: `.gitignore` and the usual build/dependency folders
    /// unless switched off, then the user's patterns (`*.php, !vendor/`).
    fn walker(&self, root: &Path) -> Result<WalkBuilder, String> {
        let mut overrides = OverrideBuilder::new(root);
        for pat in split_patterns(&self.files) {
            overrides.add(&pat).map_err(|e| t!("Muster „{pat}“: {e}", pat = pat, e = e))?;
        }
        let overrides = overrides.build().map_err(|e| e.to_string())?;
        let mut walk = WalkBuilder::new(root);
        let skip_ignored = !self.include_ignored;
        walk.hidden(false)
            .git_ignore(skip_ignored)
            .git_exclude(skip_ignored)
            .git_global(skip_ignored)
            .ignore(skip_ignored)
            .parents(skip_ignored)
            .require_git(false)
            .overrides(overrides)
            .sort_by_file_name(|a, b| a.cmp(b))
            .filter_entry(move |e| {
                let name = e.file_name();
                // VCS internals always, build/dependency folders unless wanted.
                if VCS_DIRS.iter().any(|s| name == *s) {
                    return false;
                }
                !(skip_ignored && e.depth() > 0 && e.file_type().is_some_and(|t| t.is_dir()) && SKIP_DIRS.iter().any(|s| name == *s))
            });
        Ok(walk)
    }

    /// The folder searched for `scope`, if there is one.
    pub fn target(&self, project: Option<&Path>) -> Option<DirRef> {
        match &self.scope {
            Scope::Local { dir } => {
                let root = project?;
                let dir = dir.trim().trim_matches('/');
                Some(DirRef::Local(if dir.is_empty() { root.to_path_buf() } else { root.join(dir) }))
            }
            Scope::Remote { site_id, path } => Some(DirRef::Remote { site_id: site_id.clone(), path: path.trim().to_string() }),
        }
    }

    /// Sets where to search; with a search term, searches again there.
    pub fn set_scope(&mut self, scope: Scope) {
        if scope != self.scope {
            self.scope = scope;
            self.restart_pending = !self.query.is_empty();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some() || self.remote.is_some()
    }

    /// Stops the running search; results found so far stay.
    pub fn cancel(&mut self) {
        if !self.is_running() {
            return;
        }
        if let Some((_, cancel)) = self.running.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        // Dropping the remote search stops the command on the server.
        self.remote = None;
        self.cancelled = true;
    }

    /// Runs the search in the current scope (the project folder by default).
    pub fn start(&mut self, project: Option<&Path>, sites: &Sites, ctx: &egui::Context) {
        match self.target(project) {
            Some(DirRef::Local(root)) => self.start_local(&root, ctx),
            Some(DirRef::Remote { site_id, path }) => self.start_remote(sites, &site_id, &path, ctx),
            None => {}
        }
    }

    fn reset(&mut self) {
        if let Some((_, cancel)) = self.running.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        // Dropping the remote search stops the command on the server.
        self.remote = None;
        self.results.clear();
        self.collapsed.clear();
        self.truncated = false;
        self.cancelled = false;
        self.error = None;
    }

    fn start_remote(&mut self, sites: &Sites, site_id: &str, root: &str, ctx: &egui::Context) {
        self.reset();
        if self.query.is_empty() {
            return;
        }
        // Checked here so a typo does not cost a server round trip.
        if let Err(e) = self.build_regex() {
            self.error = Some(e);
            return;
        }
        let Some(site) = sites.get(site_id).cloned() else {
            self.error = Some(t!("Server nicht gefunden").into());
            return;
        };
        let query = remote_search::Query {
            pattern: self.query.clone(),
            regex: self.use_regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            files: split_patterns(&self.files),
            include_ignored: self.include_ignored,
            skip_dirs: SKIP_DIRS.iter().map(|s| s.to_string()).collect(),
        };
        let via_ftp = site.protocol != Protocol::Sftp;
        let search = remote_search::start(site, root.to_string(), query, ctx.clone());
        self.remote = Some(RemoteRun { search, site_id: site_id.to_string(), root: root.to_string(), lines: 0, via_ftp });
    }

    fn start_local(&mut self, root: &Path, ctx: &egui::Context) {
        self.reset();
        if self.query.is_empty() {
            return;
        }
        let re = match self.build_regex() {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let walk = match self.walker(root) {
            Ok(w) => w,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let ctx = ctx.clone();
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let mut total = 0;
            for e in walk.build().flatten() {
                if flag.load(Ordering::Relaxed) || total >= MAX_MATCHES {
                    return;
                }
                if !e.file_type().is_some_and(|t| t.is_file()) {
                    continue;
                }
                if e.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(e.path()) else { continue };
                if bytes[..bytes.len().min(8192)].contains(&0) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                let mut lines = Vec::new();
                for (n, line) in text.lines().enumerate() {
                    let ranges: Vec<_> = re.find_iter(line).filter(|m| !m.is_empty()).map(|m| m.range()).collect();
                    if !ranges.is_empty() {
                        total += ranges.len();
                        lines.push(LineMatch { line: n, text: line.chars().take(400).collect(), ranges });
                    }
                }
                if !lines.is_empty() {
                    let rel = e.path().strip_prefix(&root).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
                    let file = FileRef::Local(e.path().to_path_buf());
                    if tx.send(FileMatches { file, rel, lines }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
            ctx.request_repaint();
        });
        self.running = Some((rx, cancel));
    }

    fn poll_remote(&mut self, actions: &mut Vec<SearchAction>) {
        if self.remote.is_none() {
            return;
        }
        let re = self.build_regex().ok();
        let Some(run) = &mut self.remote else { return };
        let mut done = None;
        for ev in run.search.events.try_iter() {
            match ev {
                SearchEvent::HostKey(prompt) => actions.push(SearchAction::HostKey { site_id: run.site_id.clone(), prompt }),
                SearchEvent::Root(root) => run.root = root,
                SearchEvent::Hits(hits) => {
                    run.lines += hits.len();
                    for h in hits {
                        // Highlight with Nebula's regex; the server's dialect may differ slightly.
                        let mut ranges: Vec<_> = re
                            .as_ref()
                            .map(|re| re.find_iter(&h.text).filter(|m| !m.is_empty()).map(|m| m.range()).collect())
                            .unwrap_or_default();
                        if ranges.is_empty() {
                            ranges.push(0..0);
                        }
                        let m = LineMatch { line: h.line, text: h.text, ranges };
                        match self.results.last_mut() {
                            Some(f) if f.rel == h.path => f.lines.push(m),
                            _ => {
                                let path = crate::remote::join(&run.root, &h.path);
                                let file = FileRef::Remote { site_id: run.site_id.clone(), path };
                                self.results.push(FileMatches { file, rel: h.path, lines: vec![m] });
                            }
                        }
                    }
                }
                SearchEvent::Done(err) => done = Some(err),
            }
        }
        if let Some(err) = done {
            self.truncated = run.lines >= remote_search::MAX_LINES;
            self.error = err.map(|e| if run.via_ftp { t!("Suche auf dem Server braucht SSH-Zugang (Port 22): {e}", e = e) } else { e });
            self.remote = None;
        }
    }

    fn poll(&mut self) {
        if let Some((rx, _)) = &self.running {
            loop {
                match rx.try_recv() {
                    Ok(m) => self.results.push(m),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        self.truncated = self.match_count() >= MAX_MATCHES;
                        self.running = None;
                        break;
                    }
                }
            }
        }
    }

    pub fn match_count(&self) -> usize {
        self.results.iter().flat_map(|f| &f.lines).map(|l| l.ranges.len()).sum()
    }

    /// Picker for where to search: the local project or a server, plus the
    /// folder inside it. Returns true if the scope changed.
    fn scope_picker(&mut self, ui: &mut Ui, theme: &Theme, project: Option<&Path>, sites: &Sites, homes: &HashMap<String, String>) -> bool {
        let label = match &self.scope {
            Scope::Local { .. } => t!("Projekt (lokal)").to_string(),
            Scope::Remote { site_id, .. } => sites.get(site_id).map(|s| s.display_name()).unwrap_or_default(),
        };
        let before = self.scope.clone();
        ui.horizontal(|ui| {
            ui.add_sized(vec2(52.0, 18.0), egui::Label::new(RichText::new(t!("Suchen in")).small().color(theme.text_faint)));
            egui::ComboBox::from_id_salt("search-scope").selected_text(label).width(ui.available_width()).show_ui(ui, |ui| {
                let local = matches!(before, Scope::Local { .. });
                if ui.add_enabled(project.is_some(), egui::Button::selectable(local, t!("Projekt (lokal)"))).clicked() && !local {
                    self.scope = Scope::default();
                }
                if !sites.sites.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(t!("Server (über SSH)")).small().color(theme.text_faint));
                }
                for site in &sites.sites {
                    let current = matches!(&before, Scope::Remote { site_id, .. } if *site_id == site.id);
                    if ui.selectable_label(current, site.display_name()).clicked() && !current {
                        let path = homes.get(&site.id).cloned().unwrap_or_else(|| site.root_path.trim().to_string());
                        self.scope = Scope::Remote { site_id: site.id.clone(), path };
                    }
                }
            });
        });
        let switched = match (&self.scope, &before) {
            (Scope::Local { .. }, Scope::Local { .. }) => false,
            (Scope::Remote { site_id: a, .. }, Scope::Remote { site_id: b, .. }) => a != b,
            _ => true,
        };
        // The folder inside the project or on the server; Enter searches again.
        let (text, hint) = match &mut self.scope {
            Scope::Local { dir } => (dir, t!("Ordner im Projekt (leer = ganzes Projekt)")),
            Scope::Remote { path, .. } => (path, t!("Ordner auf dem Server (leer = Startordner)")),
        };
        let resp = ui
            .horizontal(|ui| {
                ui.add_sized(vec2(52.0, 18.0), egui::Label::new(RichText::new(t!("Ordner")).small().color(theme.text_faint)));
                ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY))
            })
            .inner
            .on_hover_text(hint);
        switched || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        project: Option<&Path>,
        sites: &Sites,
        homes: &HashMap<String, String>,
        actions: &mut Vec<SearchAction>,
    ) {
        self.poll();
        self.poll_remote(actions);
        ui.add_space(8.0);
        let scope_changed = self.scope_picker(ui, theme, project, sites, homes);
        let Some(target) = self.target(project) else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(t!("Öffne einen Ordner oder wähle einen Server, um zu suchen.")).color(theme.text_faint)));
            return;
        };
        let remote = matches!(target, DirRef::Remote { .. });
        let hint = if remote { t!("Auf dem Server suchen") } else { t!("Im Projekt suchen") };
        let resp = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint).desired_width(f32::INFINITY));
        if std::mem::take(&mut self.focus_query) {
            resp.request_focus();
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut restart = enter || (scope_changed && !self.query.is_empty()) || std::mem::take(&mut self.restart_pending);
        ui.horizontal(|ui| {
            restart |= ui.toggle_value(&mut self.case_sensitive, t!("Aa")).on_hover_text(t!("Groß-/Kleinschreibung")).changed();
            restart |= ui.toggle_value(&mut self.whole_word, "W").on_hover_text(t!("Ganzes Wort")).changed();
            restart |= ui.toggle_value(&mut self.use_regex, ".*").on_hover_text(t!("Regulärer Ausdruck")).changed();
            if !remote {
                ui.toggle_value(&mut self.show_replace, t!("Ersetzen"));
            }
        });
        let files = ui.add(
            egui::TextEdit::singleline(&mut self.files)
                .hint_text(t!("Dateien, z. B. *.php, !vendor/"))
                .desired_width(f32::INFINITY),
        );
        let files = files.on_hover_text(t!("Muster wie in .gitignore, durch Komma getrennt. Ohne ! = nur diese Dateien durchsuchen, mit ! = ausschließen."));
        restart |= files.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        restart |= ui
            .checkbox(&mut self.include_ignored, RichText::new(t!("Ignorierte Dateien einbeziehen")).small())
            .on_hover_text(t!("Auch Dateien aus .gitignore und Ordner wie node_modules, vendor, dist durchsuchen"))
            .changed();
        if self.show_replace && !remote {
            ui.add(egui::TextEdit::singleline(&mut self.replacement).hint_text(t!("Ersetzen durch")).desired_width(f32::INFINITY));
            let files: Vec<PathBuf> = self
                .results
                .iter()
                .filter_map(|f| match &f.file {
                    FileRef::Local(p) => Some(p.clone()),
                    FileRef::Remote { .. } => None,
                })
                .collect();
            let label = t!("Alle ersetzen ({match_count} in {files_count} Dateien)", match_count = self.match_count(), files_count = files.len());
            if ui.add_enabled(!files.is_empty() && self.running.is_none(), egui::Button::new(label)).clicked()
                && let Ok(regex) = self.build_regex()
            {
                actions.push(SearchAction::ReplaceAll { files, regex, replacement: self.replacement.clone() });
            }
        }
        if restart {
            self.start(project, sites, ui.ctx());
        }
        ui.add_space(4.0);
        let running = self.is_running();
        let summary = match (&self.error, running) {
            (Some(e), _) => RichText::new(e).color(theme.danger),
            (None, true) => RichText::new(t!("Suche… {match_count} Treffer", match_count = self.match_count())).color(theme.text_faint),
            (None, false) if self.query.is_empty() => RichText::new(""),
            (None, false) => {
                let (matches, files) = (self.match_count(), self.results.len());
                RichText::new(if self.cancelled {
                    t!("{matches} Treffer in {files} Dateien (abgebrochen)", matches = matches, files = files)
                } else if self.truncated {
                    t!("{matches} Treffer in {files} Dateien (gekürzt)", matches = matches, files = files)
                } else {
                    t!("{matches} Treffer in {files} Dateien", matches = matches, files = files)
                })
                .color(theme.text_faint)
            }
        };
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(summary.small()).wrap());
            if running {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t!("Abbrechen")).clicked() {
                        self.cancel();
                    }
                });
            }
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical().id_salt("search-results").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for file in &self.results {
                let rel = &file.rel;
                let open = !self.collapsed.contains(rel);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, theme.hover);
                }
                let cy = rect.center().y;
                icons::chevron(ui.painter(), pos2(rect.left() + 8.0, cy), open, theme.text_faint);
                icons::file(ui.painter(), pos2(rect.left() + 24.0, cy), &rel);
                ui.painter().with_clip_rect(rect).text(
                    pos2(rect.left() + 36.0, cy),
                    Align2::LEFT_CENTER,
                    &rel,
                    FontId::proportional(12.5),
                    theme.foreground,
                );
                if resp.clicked() {
                    if open {
                        self.collapsed.insert(rel.clone());
                    } else {
                        self.collapsed.remove(rel);
                    }
                }
                if !open {
                    continue;
                }
                for m in &file.lines {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 4.0, theme.hover);
                    }
                    // Show the line around the first match.
                    let first = m.ranges[0].clone();
                    let skip = first.start.saturating_sub(24);
                    let skip = (0..=skip).rev().find(|i| m.text.is_char_boundary(*i)).unwrap_or(0);
                    let mut job = egui::text::LayoutJob::default();
                    let font = FontId::monospace(11.5);
                    let mut pos = skip;
                    for r in m.ranges.iter().filter(|r| r.start >= skip) {
                        if r.start > pos {
                            job.append(trim_if(&m.text[pos..r.start], pos == skip), 0.0, egui::TextFormat::simple(font.clone(), theme.text_muted));
                        }
                        let mut fmt = egui::TextFormat::simple(font.clone(), theme.foreground);
                        fmt.background = theme.find_match;
                        job.append(&m.text[r.start..r.end], 0.0, fmt);
                        pos = r.end;
                    }
                    if pos < m.text.len() {
                        job.append(&m.text[pos..], 0.0, egui::TextFormat::simple(font.clone(), theme.text_muted));
                    }
                    let galley = ui.painter().layout_job(job);
                    ui.painter().with_clip_rect(rect).galley(pos2(rect.left() + 36.0, rect.center().y - galley.size().y / 2.0), galley, theme.foreground);
                    ui.painter().text(
                        pos2(rect.left() + 30.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        (m.line + 1).to_string(),
                        FontId::monospace(10.5),
                        theme.text_faint,
                    );
                    if resp.clicked() {
                        let column = m.text[..first.start].chars().count();
                        actions.push(SearchAction::Open { file: file.file.clone(), line: m.line, column });
                    }
                }
            }
        });
    }
}

/// Trims leading whitespace of the first shown segment only.
fn trim_if(s: &str, cond: bool) -> &str {
    if cond { s.trim_start() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(panel: &SearchPanel, root: &Path) -> Vec<String> {
        let mut v: Vec<String> = panel
            .walker(root)
            .unwrap()
            .build()
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .map(|e| e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn scope_from_sidebar_folder() {
        let root = Path::new(r"C:\proj");
        let sub = DirRef::Local(PathBuf::from(r"C:\proj\src\app"));
        assert_eq!(Scope::for_dir(&sub, Some(root)), Scope::Local { dir: "src/app".into() });
        assert_eq!(Scope::for_dir(&DirRef::Local(PathBuf::from(r"D:\other")), Some(root)), Scope::default());

        let mut panel = SearchPanel { scope: Scope::Local { dir: "src/app".into() }, ..Default::default() };
        assert_eq!(panel.target(Some(root)), Some(DirRef::Local(PathBuf::from(r"C:\proj\src\app"))));
        assert_eq!(panel.target(None), None);

        // A new scope searches again only when there is something to search for.
        panel.set_scope(Scope::default());
        assert!(!panel.restart_pending);
        panel.query = "x".into();
        panel.set_scope(Scope::Remote { site_id: "s".into(), path: "/var/www".into() });
        assert!(panel.restart_pending);
    }

    #[test]
    fn patterns_and_gitignore() {
        let root = std::env::temp_dir().join(format!("nebula-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for f in ["a.php", "b.js", ".htaccess", "cache/x.php", "vendor/lib.php", "node_modules/m.js", ".git/config", ".gitignore"] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "x").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "cache/\n").unwrap();

        let mut panel = SearchPanel::default();
        assert_eq!(files(&panel, &root), [".gitignore", ".htaccess", "a.php", "b.js"]);

        panel.files = "*.php".into();
        assert_eq!(files(&panel, &root), ["a.php"]);

        panel.include_ignored = true;
        panel.files = "*.php, !vendor/".into();
        assert_eq!(files(&panel, &root), ["a.php", "cache/x.php"]);

        panel.files = "!*.js".into();
        assert_eq!(files(&panel, &root), [".gitignore", ".htaccess", "a.php", "cache/x.php", "vendor/lib.php"]);

        panel.files = "[".into();
        assert!(panel.walker(&root).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
