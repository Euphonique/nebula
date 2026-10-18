//! Application state and top-level UI.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Key, KeyboardShortcut, Layout, Modifiers, RichText, Sense,
    Rect, ViewportCommand, pos2, vec2,
};

use crate::editor::{DocSource, Document, EditorView, FindState, editor_id};
use crate::editor::document::DiskStamp;
use crate::highlight::HighlightService;
use crate::lang::Language;
use crate::remote::{self, HostKeyPrompt, RemoteManager, Reply, Request};
use crate::settings::{APP_NAME, AppSettings, Protocol, Site, Sites};
use crate::theme::Theme;
use crate::ui::sidebar::{FileRef, Item, NameAction, RemoteStatus, RemoteTree, Sidebar, SidebarAction};

mod extensions;
mod extensions_page;
mod files;
mod palette;
mod preview;
mod publish;
mod session;
mod settings_page;
mod tabs;
mod terminal;
mod watch;
mod backup;
use files::{ConflictPolicy, TransferQueue};
use settings_page::{Page, SettingsUi};
use crate::ui::{chrome, icons};

const MAX_OPEN_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum ToastKind {
    Info,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    until: Instant,
}

struct SiteForm {
    site: Site,
    password: String,
    password_loaded: bool,
    is_new: bool,
}

enum Dialog {
    EditSite(Box<SiteForm>),
    AskName { title: String, value: String, action: NameAction, focus: bool },
    ConfirmDelete(Vec<Item>),
    /// A pasted/dropped item exists in the target folder.
    TransferConflict { name: String, queue: TransferQueue, apply_all: bool },
    ConfirmRemoveSite(String),
    /// Unsaved changes when closing documents; `quit` closes the window afterwards.
    UnsavedChanges { doc_ids: Vec<u64>, quit: bool },
    /// The server file changed since it was opened.
    /// The file changed on the server or disk since it was opened; `saving`: while saving.
    Conflict { doc_id: u64, close_after: bool, saving: bool },
    CommandPalette(palette::PaletteState),
    /// Linking the local project to a server folder.
    PublishSetup { root: PathBuf, target: crate::settings::PublishTarget },
    GotoLine { value: String, focus: bool },
    /// chmod for server files; `recurse` applies to folder contents too.
    Permissions { items: Vec<Item>, mode: u32, octal: String, recurse: Option<remote::Recurse> },
    QuickOpen { query: String, files: Arc<Vec<PathBuf>>, selected: usize, focus: bool },
    Settings,
}

/// Work that depends on a transfer finishing first.
enum FollowUp {
    DeleteRemote { site_id: String, path: String, is_dir: bool },
    DeleteLocal(PathBuf),
    Upload { site_id: String, local: PathBuf, remote: String, then: Option<Box<FollowUp>> },
}

struct PendingSave {
    doc_id: u64,
    point: u64,
    close_after: bool,
}

pub struct NebulaApp {
    settings: AppSettings,
    sites: Sites,
    theme: Theme,
    docs: Vec<Document>,
    active: Option<usize>,
    untitled: u32,
    highlighter: HighlightService,
    remote: RemoteManager,
    find: FindState,
    sidebar: Sidebar,
    show_sidebar: bool,
    dialog: Option<Dialog>,
    toasts: Vec<Toast>,
    host_keys: VecDeque<(String, HostKeyPrompt)>,
    pending_opens: HashMap<u64, (String, String)>,
    pending_saves: HashMap<u64, PendingSave>,
    /// Remote reads that replace an open document (tag -> doc id, forced encoding).
    pending_reloads: HashMap<u64, (u64, Option<crate::editor::document::Encoding>)>,
    /// Actions to run after a transfer succeeded (e.g. deleting a moved source).
    pending_followups: HashMap<u64, FollowUp>,
    allow_close: bool,
    focus_editor: bool,
    last_title: String,
    icon_texture: Option<egui::TextureHandle>,
    backdrop_state: Option<(bool, bool)>,
    settings_ui: SettingsUi,
    /// Cursor positions for remote files reopened from the last session.
    pending_cursors: HashMap<(String, String), usize>,
    /// Publish uploads in flight: tag -> (project, relative path, mtime).
    pending_publish: HashMap<u64, (String, String, u64)>,
    /// Scroll the tab strip so the active tab is visible.
    reveal_active_tab: bool,
    /// Jump bar symbols for (doc id, version, cursor).
    outline_cache: Option<((u64, u64, usize), Vec<String>)>,
    preview: crate::preview::Preview,
    preview_state: preview::PreviewState,
    ext: extensions::ExtUi,
    /// Print job running in the background; yields the number of pages.
    printing: Option<std::thread::JoinHandle<Result<usize, String>>>,
    /// SSH terminals in the bottom panel.
    terminals: terminal::Terminals,
    /// Changes to open files and the project folder made by other programs.
    disk: watch::DiskWatch,
    /// Copies of unsaved contents for crash recovery.
    backups: backup::Backups,
}

impl NebulaApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open_paths: Vec<PathBuf>) -> Self {
        let settings = AppSettings::load();
        settings.apply_languages();
        let theme = Theme::for_settings(&settings);
        theme.apply(&cc.egui_ctx);
        let root = settings.last_folder.clone().filter(|p| p.is_dir());
        let mut app = Self {
            sidebar: Sidebar::new(root),
            sites: Sites::load(),
            theme,
            docs: Vec::new(),
            active: None,
            untitled: 0,
            highlighter: HighlightService::spawn(cc.egui_ctx.clone()),
            remote: RemoteManager::new(cc.egui_ctx.clone()),
            find: FindState::default(),
            show_sidebar: true,
            dialog: None,
            toasts: Vec::new(),
            host_keys: VecDeque::new(),
            pending_opens: HashMap::new(),
            pending_saves: HashMap::new(),
            pending_reloads: HashMap::new(),
            pending_followups: HashMap::new(),
            allow_close: false,
            focus_editor: true,
            last_title: String::new(),
            icon_texture: None,
            backdrop_state: None,
            settings_ui: SettingsUi::default(),
            pending_cursors: HashMap::new(),
            pending_publish: HashMap::new(),
            reveal_active_tab: true,
            outline_cache: None,
            preview: Default::default(),
            preview_state: Default::default(),
            ext: extensions::ExtUi::new(&cc.egui_ctx, &settings.disabled_extensions),
            printing: None,
            terminals: Default::default(),
            disk: Default::default(),
            backups: Default::default(),
            settings,
        };
        app.restore_session();
        if let Some(root) = app.sidebar.root.clone() {
            app.remember_folder(&root);
        }
        app.restore_backups();
        for path in open_paths {
            if path.is_dir() {
                app.open_folder(path);
            } else {
                app.open_local(&path);
            }
        }
        app
    }

    // ---- notifications -------------------------------------------------------

    fn info(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            kind: ToastKind::Info,
            until: Instant::now() + Duration::from_secs(3),
        });
    }

    fn error(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            kind: ToastKind::Error,
            until: Instant::now() + Duration::from_secs(8),
        });
    }

    // ---- documents -----------------------------------------------------------

    fn active_doc(&mut self) -> Option<&mut Document> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }

    fn doc_index(&self, id: u64) -> Option<usize> {
        self.docs.iter().position(|d| d.id == id)
    }

    fn activate(&mut self, index: usize) {
        if index < self.docs.len() {
            self.active = Some(index);
            self.reveal_active_tab = true;
            self.focus_editor = !self.sidebar.focused;
        }
    }

    fn add_doc(&mut self, doc: Document) {
        // Replace an untouched, empty untitled document.
        if let Some(i) = self.active
            && let Some(d) = self.docs.get(i)
            && matches!(d.source, DocSource::Untitled(_))
            && !d.is_dirty()
            && d.len_chars() == 0
        {
            self.docs[i] = doc;
            self.activate(i);
            return;
        }
        let at = self.active.map_or(self.docs.len(), |i| i + 1);
        self.docs.insert(at, doc);
        self.activate(at);
    }

    fn new_document(&mut self) {
        self.untitled += 1;
        let mut doc = Document::new(DocSource::Untitled(self.untitled), "");
        doc.language = Language::Html;
        let at = self.active.map_or(self.docs.len(), |i| i + 1);
        self.docs.insert(at, doc);
        self.activate(at);
    }

    fn open_local(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path = PathBuf::from(path.to_string_lossy().trim_start_matches(r"\\?\"));
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.source == DocSource::Local(path.clone()))
        {
            self.activate(i);
            return;
        }
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() > MAX_OPEN_BYTES => self.error(format!("{} ist zu groß.", path.display())),
            Ok(bytes) if is_binary(&bytes) => {
                self.error(format!("{} ist keine Textdatei.", path.display()));
            }
            Ok(bytes) => {
                let mut doc = Document::from_bytes(DocSource::Local(path.clone()), &bytes);
                doc.disk_stamp = DiskStamp::of(&path);
                doc.config = crate::editorconfig::for_file(&path);
                self.add_doc(doc);
                self.settings.recent_files.retain(|p| p != &path);
                self.settings.recent_files.insert(0, path);
                self.settings.recent_files.truncate(15);
            }
            Err(e) => self.error(format!("{} konnte nicht geöffnet werden: {e}", path.display())),
        }
    }

    fn open_remote(&mut self, site_id: &str, path: &str) {
        let source = DocSource::Remote {
            site_id: site_id.to_string(),
            path: path.to_string(),
        };
        if let Some(i) = self.docs.iter().position(|d| d.source == source) {
            self.activate(i);
            return;
        }
        if self
            .pending_opens
            .values()
            .any(|(s, p)| s == site_id && p == path)
        {
            return;
        }
        let Some(site) = self.sites.get(site_id).cloned() else { return };
        self.ensure_tree(&site);
        let tag = self.remote.send(&site, Request::Read(path.to_string()));
        self.pending_opens.insert(tag, (site_id.to_string(), path.to_string()));
    }

    fn pick_and_open_file(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        if let Some(root) = &self.sidebar.root {
            dialog = dialog.set_directory(root);
        }
        if let Some(files) = dialog.pick_files() {
            for f in files {
                self.open_local(&f);
            }
        }
    }

    fn pick_and_open_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.open_folder(dir);
        }
    }

    /// Saves a document. Remote saves complete asynchronously.
    fn save_doc(&mut self, index: usize, close_after: bool) -> bool {
        self.save_doc_checked(index, close_after)
    }

    /// `force` overwrites the server file even if it changed in the meantime.
    fn save_doc_with(&mut self, index: usize, close_after: bool, force: bool) -> bool {
        if let Some(doc) = self.docs.get(index)
            && !matches!(doc.source, DocSource::Untitled(_))
        {
            let e = self.settings.editor_for_doc(doc);
            crate::editor::commands::tidy_whitespace(&mut self.docs[index], e.trim_trailing_whitespace, e.insert_final_newline);
        }
        let Some(doc) = self.docs.get_mut(index) else { return false };
        match doc.source.clone() {
            DocSource::Untitled(_) => self.save_doc_as(index),
            DocSource::Local(path) => {
                if !force && self.changed_on_disk(index) {
                    let doc_id = self.docs[index].id;
                    self.dialog = Some(Dialog::Conflict { doc_id, close_after, saving: true });
                    return false;
                }
                let doc = &mut self.docs[index];
                let doc_id = doc.id;
                let bytes = doc.to_bytes();
                match std::fs::write(&path, bytes) {
                    Ok(()) => {
                        doc.mark_saved();
                        doc.disk_stamp = DiskStamp::of(&path);
                        doc.disk_missing = 0;
                        if close_after {
                            self.docs.remove(index);
                            self.fix_active_after_remove(index);
                        }
                        self.upload_on_save(&path);
                        self.preview_saved(&DocSource::Local(path.clone()));
                        self.ext_doc_saved(doc_id);
                        true
                    }
                    Err(e) => {
                        self.error(format!("Speichern fehlgeschlagen: {e}"));
                        false
                    }
                }
            }
            DocSource::Remote { site_id, path } => {
                let Some(site) = self.sites.get(&site_id).cloned() else {
                    self.error("Der Server dieser Datei existiert nicht mehr.");
                    return false;
                };
                let data = Arc::new(doc.to_bytes());
                let point = doc.save_point();
                doc.saving = true;
                let doc_id = doc.id;
                let expected = if force { None } else { doc.remote_stamp };
                self.ensure_tree(&site);
                let tag = self.remote.send(&site, Request::Write { path, data, expected });
                self.pending_saves.insert(tag, PendingSave { doc_id, point, close_after });
                true
            }
        }
    }

    fn save_doc_as(&mut self, index: usize) -> bool {
        let Some(doc) = self.docs.get(index) else { return false };
        let mut dialog = rfd::FileDialog::new().set_file_name(doc.title());
        if let DocSource::Untitled(_) = doc.source
            && doc.suggested_name.is_none()
        {
            dialog = dialog.set_file_name(format!("{}.html", doc.title()));
        }
        if let Some(root) = &self.sidebar.root {
            dialog = dialog.set_directory(root);
        }
        let Some(path) = dialog.save_file() else { return false };
        let doc = &mut self.docs[index];
        let was_untitled = matches!(doc.source, DocSource::Untitled(_));
        doc.source = DocSource::Local(path.clone());
        doc.disk_stamp = None;
        doc.config = crate::editorconfig::for_file(&path);
        if was_untitled {
            doc.line_ending = doc.config.end_of_line.unwrap_or(doc.line_ending);
            doc.encoding = doc.config.charset.unwrap_or(doc.encoding);
        }
        doc.suggested_name = None;
        doc.language = Language::from_path(&path.to_string_lossy());
        doc.highlight_version = u64::MAX;
        let ok = self.save_doc(index, false);
        if ok {
            self.sidebar.refresh_local(path.parent());
        }
        ok
    }

    fn save_all(&mut self) {
        for i in 0..self.docs.len() {
            if self.docs[i].is_dirty() && !matches!(self.docs[i].source, DocSource::Untitled(_)) {
                self.save_doc(i, false);
            }
        }
    }

    fn fix_active_after_remove(&mut self, removed: usize) {
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(a) if a > removed => Some(a - 1),
            Some(a) if a >= self.docs.len() => Some(self.docs.len() - 1),
            other => other,
        };
        self.focus_editor = true;
    }

    fn close_doc(&mut self, index: usize, force: bool) {
        let Some(doc) = self.docs.get(index) else { return };
        if doc.is_dirty() && !force {
            self.dialog = Some(Dialog::UnsavedChanges {
                doc_ids: vec![doc.id],
                quit: false,
            });
            return;
        }
        self.docs.remove(index);
        self.fix_active_after_remove(index);
    }

    // ---- remote --------------------------------------------------------------

    fn ensure_tree(&mut self, site: &Site) {
        if !self.remote.is_connected(&site.id) {
            self.sidebar.remote.insert(site.id.clone(), RemoteTree::new());
            self.remote.connect(site);
        }
    }

    fn remote_request(&mut self, site_id: &str, request: Request) {
        let Some(site) = self.sites.get(site_id).cloned() else { return };
        self.ensure_tree(&site);
        if let Request::List(dir) = &request
            && let Some(tree) = self.sidebar.remote.get_mut(site_id)
        {
            tree.loading.insert(dir.clone());
        }
        self.remote.send(&site, request);
    }

    fn process_remote_events(&mut self) {
        let events: Vec<_> = self.remote.poll().collect();
        for ev in events {
            let site_name = self
                .sites
                .get(&ev.site_id)
                .map(|s| s.display_name())
                .unwrap_or_default();
            match ev.reply {
                Reply::Connected { home } => {
                    let tree = self
                        .sidebar
                        .remote
                        .entry(ev.site_id.clone())
                        .or_insert_with(RemoteTree::new);
                    tree.status = RemoteStatus::Connected;
                    if tree.home.is_none() {
                        tree.home = Some(home);
                    }
                }
                Reply::ConnectFailed { message } => {
                    if let Some(tree) = self.sidebar.remote.get_mut(&ev.site_id) {
                        tree.status = RemoteStatus::Failed(message.clone());
                        tree.loading.clear();
                    }
                    self.remote.forget(&ev.site_id);
                    self.error(format!("{site_name}: {message}"));
                    self.fail_pending_for_site(&ev.site_id);
                    self.preview_site_failed(&ev.site_id);
                }
                Reply::Disconnected => {
                    self.sidebar.remote.remove(&ev.site_id);
                }
                Reply::Listed { path, entries } => {
                    if let Some(tree) = self.sidebar.remote.get_mut(&ev.site_id) {
                        tree.loading.remove(&path);
                        tree.dirs.insert(path, entries);
                    }
                }
                Reply::Read { path, data, stamp } => {
                    if self.preview_state.pending.contains_key(&ev.tag) {
                        self.preview_read_done(ev.tag, &path, Some(data));
                        continue;
                    }
                    if let Some((doc_id, encoding)) = self.pending_reloads.remove(&ev.tag) {
                        if let Some(i) = self.doc_index(doc_id) {
                            let mut doc = Document::from_bytes_as(self.docs[i].source.clone(), &data, encoding);
                            doc.id = doc_id;
                            doc.remote_stamp = stamp;
                            self.docs[i] = doc;
                            self.info(format!("{} neu vom Server geladen", self.docs[i].title()));
                        }
                        continue;
                    }
                    let Some((site_id, _)) = self.pending_opens.remove(&ev.tag) else { continue };
                    if is_binary(&data) {
                        self.error(format!("{path} ist keine Textdatei."));
                        continue;
                    }
                    let mut doc = Document::from_bytes(DocSource::Remote { site_id, path }, &data);
                    doc.remote_stamp = stamp;
                    self.add_doc(doc);
                    if let Some(i) = self.active {
                        self.apply_pending_cursor(i);
                        self.apply_pending_backup(i);
                    }
                }
                Reply::Conflict { path } => {
                    if let Some(save) = self.pending_saves.remove(&ev.tag)
                        && let Some(i) = self.doc_index(save.doc_id)
                    {
                        self.docs[i].saving = false;
                        self.dialog = Some(Dialog::Conflict { doc_id: save.doc_id, close_after: save.close_after, saving: true });
                    } else {
                        self.error(format!("{path} wurde auf dem Server geändert."));
                    }
                }
                Reply::Transferred { message, remote_dir, local_dir } => {
                    if self.pending_publish.contains_key(&ev.tag) {
                        self.mark_published(ev.tag);
                        if self.pending_publish.is_empty() {
                            self.info(format!("Veröffentlicht auf {site_name}"));
                        }
                    } else {
                        self.info(format!("{site_name}: {message}"));
                    }
                    if let Some(dir) = remote_dir {
                        self.remote_request(&ev.site_id, Request::List(dir));
                    }
                    if let Some(dir) = local_dir {
                        self.sidebar.refresh_local(Some(&dir));
                    }
                    if let Some(follow) = self.pending_followups.remove(&ev.tag) {
                        self.run_followup(follow);
                    }
                }
                Reply::Written { path, bytes, stamp } => {
                    if let Some(save) = self.pending_saves.remove(&ev.tag)
                        && let Some(i) = self.doc_index(save.doc_id)
                    {
                        let doc = &mut self.docs[i];
                        doc.saving = false;
                        doc.remote_stamp = stamp;
                        doc.mark_saved_at(save.point);
                        let saved_id = doc.id;
                        self.ext_doc_saved(saved_id);
                        let doc = &mut self.docs[i];
                        if save.close_after && !doc.is_dirty() {
                            self.docs.remove(i);
                            self.fix_active_after_remove(i);
                        }
                    }
                    self.preview_saved(&DocSource::Remote { site_id: ev.site_id.clone(), path: path.clone() });
                    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                    self.info(format!("{name} auf {site_name} gespeichert ({})", human_bytes(bytes)));
                }
                Reply::Changed { dir, message } => {
                    self.info(message);
                    self.remote_request(&ev.site_id, Request::List(dir));
                }
                Reply::Failed { request, message } => {
                    if self.preview_state.pending.contains_key(&ev.tag) {
                        let p = if let Request::Read(p) = &request { p.clone() } else { String::new() };
                        self.preview_read_done(ev.tag, &p, None);
                        continue;
                    }
                    match &request {
                        Request::List(path) => {
                            if let Some(tree) = self.sidebar.remote.get_mut(&ev.site_id) {
                                tree.loading.remove(path);
                                tree.dirs.insert(path.clone(), Vec::new());
                            }
                        }
                        Request::Read(path) => {
                            if let Some((site_id, _)) = self.pending_opens.remove(&ev.tag) {
                                self.backup_open_failed(&site_id, Some(path));
                            }
                            self.pending_reloads.remove(&ev.tag);
                        }
                        Request::Upload { .. } | Request::Download { .. } | Request::Copy { .. } => {
                            // Keep the source of a failed move.
                            self.pending_followups.remove(&ev.tag);
                            self.pending_publish.remove(&ev.tag);
                        }
                        Request::Write { .. } => {
                            if let Some(save) = self.pending_saves.remove(&ev.tag)
                                && let Some(i) = self.doc_index(save.doc_id)
                            {
                                self.docs[i].saving = false;
                            }
                        }
                        _ => {}
                    }
                    self.error(format!("{site_name}: {message}"));
                }
                Reply::HostKeyUnknown(prompt) => {
                    self.host_keys.push_back((ev.site_id, prompt));
                }
            }
        }
    }

    fn run_followup(&mut self, follow: FollowUp) {
        match follow {
            FollowUp::DeleteRemote { site_id, path, is_dir } => {
                self.remote_request(&site_id, Request::Delete { path, is_dir });
            }
            FollowUp::Upload { site_id, local, remote, then } => {
                if let Some(tag) = self.send_remote(&site_id, Request::Upload { local, remote })
                    && let Some(then) = then
                {
                    self.pending_followups.insert(tag, *then);
                }
            }
            FollowUp::DeleteLocal(path) => {
                let result = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                match result {
                    Ok(()) => self.sidebar.refresh_local(path.parent()),
                    Err(e) => self.error(format!("{} konnte nicht entfernt werden: {e}", path.display())),
                }
            }
        }
    }

    /// Reloads a remote document from the server, discarding local changes.
    fn reload_remote(&mut self, doc_id: u64) {
        self.reload_remote_as(doc_id, None);
    }

    /// Reads the active document again from disk or server with an encoding.
    fn reload_with_encoding(&mut self, encoding: crate::editor::document::Encoding) {
        let Some(i) = self.active else { return };
        match self.docs[i].source.clone() {
            DocSource::Local(path) => match std::fs::read(&path) {
                Ok(bytes) => {
                    let old = &self.docs[i];
                    let mut doc = Document::from_bytes_as(old.source.clone(), &bytes, Some(encoding));
                    doc.id = old.id;
                    doc.language = old.language;
                    doc.disk_stamp = DiskStamp::of(&path);
                    let head = old.primary_selection().head.min(doc.len_chars());
                    doc.set_selection(crate::editor::document::Selection::cursor(head));
                    self.docs[i] = doc;
                }
                Err(e) => self.error(format!("{} konnte nicht gelesen werden: {e}", path.display())),
            },
            DocSource::Remote { .. } => {
                let id = self.docs[i].id;
                self.reload_remote_as(id, Some(encoding));
            }
            DocSource::Untitled(_) => {}
        }
    }

    /// Reloads a remote document, decoding it with `encoding` (or detecting it).
    fn reload_remote_as(&mut self, doc_id: u64, encoding: Option<crate::editor::document::Encoding>) {
        let Some(i) = self.doc_index(doc_id) else { return };
        let DocSource::Remote { site_id, path } = self.docs[i].source.clone() else { return };
        let Some(site) = self.sites.get(&site_id).cloned() else { return };
        self.ensure_tree(&site);
        let tag = self.remote.send(&site, Request::Read(path));
        self.pending_reloads.insert(tag, (doc_id, encoding));
    }

    fn fail_pending_for_site(&mut self, site_id: &str) {
        self.pending_opens.retain(|_, (s, _)| s != site_id);
        self.backup_open_failed(site_id, None);
        let ids: Vec<u64> = self.pending_saves.values().map(|s| s.doc_id).collect();
        for id in ids {
            if let Some(i) = self.doc_index(id)
                && matches!(&self.docs[i].source, DocSource::Remote { site_id: s, .. } if s == site_id)
            {
                self.docs[i].saving = false;
            }
        }
    }

    fn handle_sidebar_action(&mut self, ctx: &egui::Context, action: SidebarAction) {
        match action {
            SidebarAction::OpenLocal(p) => self.open_local(&p),
            SidebarAction::OpenFolder => self.pick_and_open_folder(),
            SidebarAction::SwitchFolder(path) => {
                if path.is_dir() {
                    self.open_folder(path);
                } else {
                    self.settings.recent_folders.retain(|p| p != &path);
                    self.sidebar.recent_folders = self.settings.recent_folders.clone();
                    self.error(format!("{} existiert nicht mehr.", path.display()));
                }
            }
            SidebarAction::OpenRemote { site_id, path } => self.open_remote(&site_id, &path),
            SidebarAction::Connect(site_id) => {
                if let Some(site) = self.sites.get(&site_id).cloned() {
                    if let Some(tree) = self.sidebar.remote.get(&site_id)
                        && matches!(tree.status, RemoteStatus::Failed(_))
                    {
                        self.remote.forget(&site_id);
                    }
                    self.ensure_tree(&site);
                }
            }
            SidebarAction::Disconnect(site_id) => {
                self.remote.disconnect(&site_id);
                self.sidebar.remote.remove(&site_id);
                self.sidebar.expanded_sites.remove(&site_id);
            }
            SidebarAction::Remote { site_id, request } => self.remote_request(&site_id, request),
            SidebarAction::AskName { title, initial, action } => {
                self.dialog = Some(Dialog::AskName { title, value: initial, action, focus: true });
            }
            SidebarAction::ConfirmDelete(items) => self.dialog = Some(Dialog::ConfirmDelete(items)),
            SidebarAction::Clipboard { items, cut } => self.set_clipboard(items, cut),
            SidebarAction::Paste(target) => self.paste(target),
            SidebarAction::Transfer { items, target, copy } => self.transfer(items, target, copy),
            SidebarAction::Duplicate(items) => self.duplicate(items),
            SidebarAction::UploadFiles { site_id, dir } => self.upload_files_dialog(site_id, dir),
            SidebarAction::DownloadTo(item) => self.download_dialog(item),
            SidebarAction::OpenTerminal(site_id) => self.open_terminal(ctx, &site_id),
            SidebarAction::Permissions(items) => {
                let items: Vec<Item> = items.into_iter().filter(|i| matches!(i.file, FileRef::Remote { .. })).collect();
                let Some(first) = items.first() else { return };
                let FileRef::Remote { site_id, path } = &first.file else { return };
                let mode = self
                    .sidebar
                    .remote_mode(site_id, path)
                    .map(|m| m & 0o777)
                    .unwrap_or(if first.is_dir { 0o755 } else { 0o644 });
                self.dialog = Some(Dialog::Permissions { items, mode, octal: format!("{mode:03o}"), recurse: None });
            }
            SidebarAction::Publish => self.publish(),
            SidebarAction::PublishSetup => self.open_publish_setup(),
            SidebarAction::Search(crate::ui::search::SearchAction::Open { path, line, column }) => {
                self.open_local(&path);
                // open_local activates the file (or the already open tab).
                if let Some(i) = self.active
                    && matches!(&self.docs[i].source, DocSource::Local(p) if p.file_name() == path.file_name())
                {
                    let doc = &mut self.docs[i];
                    let line = line.min(doc.len_lines() - 1);
                    let pos = doc.line_to_char(line) + column.min(doc.line_len(line));
                    doc.set_selection(crate::editor::document::Selection::cursor(pos));
                    doc.view.center_line = Some(line);
                    self.focus_editor = true;
                    self.sidebar.focused = false;
                }
            }
            SidebarAction::Search(crate::ui::search::SearchAction::ReplaceAll { files, regex, replacement }) => {
                self.replace_in_files(files, regex, replacement, ctx);
            }
            SidebarAction::EditSite(id) => self.open_site_form(id),
            SidebarAction::RemoveSite(id) => self.dialog = Some(Dialog::ConfirmRemoveSite(id)),
            SidebarAction::OpenUrl(url) => {
                if !crate::win32::open_url(&url) {
                    self.error(format!("{url} konnte nicht geöffnet werden."));
                }
            }
            SidebarAction::CopyText(text) => ctx.copy_text(text),
        }
    }

    fn open_settings(&mut self, page: Page) {
        let ext = std::mem::take(&mut self.settings_ui.ext);
        let selected = self.settings_ui.selected_extension.take();
        self.settings_ui = SettingsUi::open(page);
        self.settings_ui.ext = ext;
        self.settings_ui.selected_extension = selected;
        self.dialog = Some(Dialog::Settings);
    }

    fn open_site_form(&mut self, id: Option<String>) {
        let (site, is_new) = match id.and_then(|id| self.sites.get(&id).cloned()) {
            Some(site) => (site, false),
            None => (Site::default(), true),
        };
        self.dialog = Some(Dialog::EditSite(Box::new(SiteForm {
            site,
            password: String::new(),
            password_loaded: is_new,
            is_new,
        })));
    }

    fn apply_name_action(&mut self, action: NameAction, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() || name.contains(['/', '\\']) {
            self.error("Ungültiger Name.");
            return;
        }
        match action {
            NameAction::NewRemoteFile { site_id, dir } => {
                let path = remote::join(&dir, &name);
                self.remote_request(&site_id, Request::CreateFile(path.clone()));
                // Open it right away; the empty file exists once the request ran.
                self.open_remote(&site_id, &path);
            }
            NameAction::NewRemoteFolder { site_id, dir } => {
                self.remote_request(&site_id, Request::Mkdir(remote::join(&dir, &name)));
            }
            NameAction::RenameRemote { site_id, path } => {
                let to = remote::join(&remote::parent(&path), &name);
                for d in &mut self.docs {
                    if let DocSource::Remote { site_id: s, path: p } = &mut d.source
                        && *s == site_id
                        && *p == path
                    {
                        *p = to.clone();
                    }
                }
                self.remote_request(&site_id, Request::Rename { from: path, to });
            }
            NameAction::NewLocalFile { dir } => {
                let path = dir.join(&name);
                if path.exists() {
                    self.error(format!("{} existiert bereits.", path.display()));
                    return;
                }
                match std::fs::write(&path, b"") {
                    Ok(()) => {
                        self.sidebar.refresh_local(Some(&dir));
                        self.open_local(&path);
                    }
                    Err(e) => self.error(format!("Datei konnte nicht angelegt werden: {e}")),
                }
            }
            NameAction::NewLocalFolder { dir } => match std::fs::create_dir(dir.join(&name)) {
                Ok(()) => self.sidebar.refresh_local(Some(&dir)),
                Err(e) => self.error(format!("Ordner konnte nicht angelegt werden: {e}")),
            },
            NameAction::RenameLocal { path } => {
                let to = path.with_file_name(&name);
                match std::fs::rename(&path, &to) {
                    Ok(()) => {
                        for d in &mut self.docs {
                            if d.source == DocSource::Local(path.clone()) {
                                d.source = DocSource::Local(to.clone());
                            }
                        }
                        self.sidebar.refresh_local(path.parent());
                    }
                    Err(e) => self.error(format!("Umbenennen fehlgeschlagen: {e}")),
                }
            }
        }
    }

    // ---- shortcuts -----------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));

        // Strg+Ö: German keyboards report the key by its US position (`;`),
        // US keyboards use Strg+` as in other editors.
        if pressed(ctrl, Key::Semicolon) || pressed(ctrl, Key::Backtick) {
            self.toggle_terminal(ctx);
            return;
        }
        // In the terminal, keys like Strg+W or Strg+S belong to the shell.
        if self.terminal_focused(ctx) {
            if pressed(ctrl_shift, Key::P) {
                self.dialog = Some(Dialog::CommandPalette(palette::PaletteState { query: String::new(), selected: 0, focus: true }));
            }
            return;
        }

        if pressed(ctrl_shift, Key::S) {
            if let Some(i) = self.active {
                self.save_doc_as(i);
            }
        } else if pressed(ctrl | Modifiers::ALT, Key::S) {
            self.save_all();
        } else if pressed(ctrl, Key::S) {
            if let Some(i) = self.active {
                self.save_doc(i, false);
            }
        }
        if pressed(ctrl, Key::N) {
            self.new_document();
        }
        if pressed(ctrl_shift, Key::O) {
            self.pick_and_open_folder();
        } else if pressed(ctrl, Key::O) {
            self.pick_and_open_file();
        }
        if pressed(ctrl, Key::W) || pressed(ctrl, Key::F4) {
            if let Some(i) = self.active {
                self.close_doc(i, false);
            }
        }
        if pressed(ctrl_shift, Key::Tab) || pressed(ctrl, Key::PageUp) {
            if let Some(i) = self.active
                && !self.docs.is_empty()
            {
                self.activate((i + self.docs.len() - 1) % self.docs.len());
            }
        } else if (pressed(ctrl, Key::Tab) || pressed(ctrl, Key::PageDown))
            && let Some(i) = self.active
            && !self.docs.is_empty()
        {
            self.activate((i + 1) % self.docs.len());
        }
        if pressed(ctrl_shift, Key::F) {
            self.open_project_search();
        } else if pressed(ctrl, Key::F) {
            self.open_find(false);
        }
        if pressed(ctrl, Key::H) {
            self.open_find(true);
        }
        if pressed(Modifiers::SHIFT, Key::F3) {
            self.find_step(false);
        } else if pressed(Modifiers::NONE, Key::F3) {
            self.find_step(true);
        }
        if pressed(Modifiers::NONE, Key::F5) {
            self.open_in_browser(ctx);
        }
        if pressed(ctrl_shift, Key::V) {
            self.toggle_preview();
        }
        if pressed(ctrl, Key::G) {
            self.dialog = Some(Dialog::GotoLine { value: String::new(), focus: true });
        }
        // Extra modifiers don't keep a shortcut from matching, so the longer
        // combinations come first.
        if pressed(ctrl | Modifiers::ALT, Key::P) {
            self.print_active(ctx);
        } else if pressed(ctrl_shift, Key::P) {
            self.dialog = Some(Dialog::CommandPalette(palette::PaletteState { query: String::new(), selected: 0, focus: true }));
        } else if pressed(ctrl, Key::P) {
            self.open_quick_open();
        }
        if pressed(ctrl, Key::Comma) {
            self.open_settings(Page::Editor);
        }
        if pressed(ctrl, Key::B) {
            self.show_sidebar = !self.show_sidebar;
        }
        if pressed(ctrl, Key::Plus) || pressed(ctrl, Key::Equals) {
            self.settings.editor.font_size = (self.settings.editor.font_size + 1.0).min(40.0);
        }
        if pressed(ctrl, Key::Minus) {
            self.settings.editor.font_size = (self.settings.editor.font_size - 1.0).max(8.0);
        }
        if pressed(ctrl, Key::Num0) {
            self.settings.editor.font_size = 14.0;
        }
        if pressed(Modifiers::ALT, Key::Z) {
            self.settings.editor.word_wrap = !self.settings.editor.word_wrap;
            // Alt+letter may also arrive as typed text.
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case("z"))));
        }
        // Undo/redo also work while the editor doesn't have keyboard focus,
        // e.g. after an extension command changed the document.
        if let Some(i) = self.active
            && !ctx.memory(|m| m.has_focus(editor_id(self.docs[i].id)))
            && !ctx.egui_wants_keyboard_input()
        {
            if pressed(ctrl_shift, Key::Z) || pressed(ctrl, Key::Y) {
                self.docs[i].redo();
                self.focus_editor = true;
            } else if pressed(ctrl, Key::Z) {
                self.docs[i].undo();
                self.focus_editor = true;
            }
        }
        if self.find.visible
            && self.dialog.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.find.visible = false;
            self.focus_editor = true;
        }
    }

    /// Items dragged from the tree out of the window become an OS drag, so
    /// they can be dropped into Explorer (or any other program).
    fn drag_out_of_window(&mut self, ctx: &egui::Context) {
        let Some(items) = egui::DragAndDrop::payload::<crate::ui::sidebar::DragItems>(ctx) else { return };
        if !ctx.input(|i| i.pointer.primary_down()) {
            return;
        }
        // The cursor position from Windows also works outside the window.
        let Some(p) = crate::win32::cursor_pos(ctx.pixels_per_point()) else { return };
        if ctx.viewport_rect().shrink(2.0).contains(p) {
            return;
        }
        egui::DragAndDrop::clear_payload(ctx);
        let paths: Vec<PathBuf> = items
            .0
            .iter()
            .filter_map(|i| match &i.file {
                FileRef::Local(p) => Some(p.clone()),
                FileRef::Remote { .. } => None,
            })
            .collect();
        if paths.is_empty() {
            self.info("Dateien vom Server lassen sich nicht direkt hinausziehen – nutze „Herunterladen nach…“.");
            return;
        }
        if crate::win32::drag_files_out(&paths) {
            self.sidebar.refresh_local(None);
        }
    }

    fn open_project_search(&mut self) {
        self.show_sidebar = true;
        self.sidebar.tab = crate::ui::sidebar::SidebarTab::Search;
        self.sidebar.search.focus_query = true;
        if let Some(doc) = self.active.and_then(|i| self.docs.get(i)) {
            let sel = doc.primary_selection();
            if !sel.is_empty() && doc.char_to_line(sel.anchor) == doc.char_to_line(sel.head) {
                self.sidebar.search.query = doc.slice(sel.range());
            }
        }
    }

    fn open_find(&mut self, replace: bool) {
        self.find.visible = true;
        self.find.show_replace = replace;
        self.find.focus_query = true;
        if let Some(doc) = self.active.and_then(|i| self.docs.get(i)) {
            let sel = doc.primary_selection();
            if !sel.is_empty() && doc.char_to_line(sel.anchor) == doc.char_to_line(sel.head) {
                self.find.query = doc.slice(sel.range());
            }
        }
    }

    fn find_step(&mut self, forward: bool) {
        if let Some(i) = self.active {
            let doc = &mut self.docs[i];
            self.find.find(doc, forward);
        }
    }

    fn open_quick_open(&mut self) {
        let Some(root) = self.sidebar.root.clone() else {
            self.info("Öffne zuerst einen Ordner, um Dateien schnell zu finden.");
            return;
        };
        let files = Arc::new(collect_files(&root, 20_000));
        self.dialog = Some(Dialog::QuickOpen { query: String::new(), files, selected: 0, focus: true });
    }

    // ---- UI ------------------------------------------------------------------

    fn menu_contents(&mut self, ui: &mut egui::Ui) {
        {
            ui.menu_button("Datei", |ui| {
                if ui.add(egui::Button::new("Neu").shortcut_text("Strg+N")).clicked() {
                    self.new_document();
                    ui.close();
                }
                if ui.add(egui::Button::new("Öffnen…").shortcut_text("Strg+O")).clicked() {
                    ui.close();
                    self.pick_and_open_file();
                }
                if ui
                    .add(egui::Button::new("Ordner öffnen…").shortcut_text("Strg+Umschalt+O"))
                    .clicked()
                {
                    ui.close();
                    self.pick_and_open_folder();
                }
                if ui.add(egui::Button::new("Schnell öffnen…").shortcut_text("Strg+P")).clicked() {
                    ui.close();
                    self.open_quick_open();
                }
                let folders = self.settings.recent_folders.clone();
                ui.add_enabled_ui(!folders.is_empty(), |ui| {
                    ui.menu_button("Zuletzt geöffnete Projekte", |ui| {
                        for p in folders {
                            if ui.button(p.display().to_string()).clicked() {
                                ui.close();
                                self.handle_sidebar_action(ui.ctx(), SidebarAction::SwitchFolder(p));
                            }
                        }
                    });
                });
                let recent = self.settings.recent_files.clone();
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    ui.menu_button("Zuletzt geöffnet", |ui| {
                        for p in recent {
                            if ui.button(p.display().to_string()).clicked() {
                                self.open_local(&p);
                                ui.close();
                            }
                        }
                    });
                });
                ui.separator();
                let has_doc = self.active.is_some();
                if ui
                    .add_enabled(has_doc, egui::Button::new("Speichern").shortcut_text("Strg+S"))
                    .clicked()
                {
                    ui.close();
                    if let Some(i) = self.active {
                        self.save_doc(i, false);
                    }
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new("Speichern unter…").shortcut_text("Strg+Umschalt+S"),
                    )
                    .clicked()
                {
                    ui.close();
                    if let Some(i) = self.active {
                        self.save_doc_as(i);
                    }
                }
                if ui.add(egui::Button::new("Alle speichern").shortcut_text("Strg+Alt+S")).clicked() {
                    ui.close();
                    self.save_all();
                }
                if ui.add_enabled(has_doc, egui::Button::new("Duplizieren")).clicked() {
                    ui.close();
                    if let Some(i) = self.active {
                        self.duplicate_doc(i);
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(has_doc && self.printing.is_none(), egui::Button::new("Drucken…").shortcut_text("Strg+Alt+P"))
                    .clicked()
                {
                    ui.close();
                    self.print_active(&ui.ctx().clone());
                }
                ui.separator();
                if ui
                    .add_enabled(has_doc, egui::Button::new("Tab schließen").shortcut_text("Strg+W"))
                    .clicked()
                {
                    ui.close();
                    if let Some(i) = self.active {
                        self.close_doc(i, false);
                    }
                }
                if ui.button("Beenden").clicked() {
                    ui.close();
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
            ui.menu_button("Bearbeiten", |ui| {
                let has_doc = self.active.is_some();
                ui.add_enabled_ui(has_doc, |ui| {
                    if ui.add(egui::Button::new("Rückgängig").shortcut_text("Strg+Z")).clicked() {
                        if let Some(d) = self.active_doc() {
                            d.undo();
                        }
                        ui.close();
                    }
                    if ui.add(egui::Button::new("Wiederholen").shortcut_text("Strg+Y")).clicked() {
                        if let Some(d) = self.active_doc() {
                            d.redo();
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.add(egui::Button::new("Suchen…").shortcut_text("Strg+F")).clicked() {
                        self.open_find(false);
                        ui.close();
                    }
                    if ui.add(egui::Button::new("Ersetzen…").shortcut_text("Strg+H")).clicked() {
                        self.open_find(true);
                        ui.close();
                    }
                    if ui.add(egui::Button::new("Gehe zu Zeile…").shortcut_text("Strg+G")).clicked() {
                        self.dialog = Some(Dialog::GotoLine { value: String::new(), focus: true });
                        ui.close();
                    }
                    ui.separator();
                    use crate::editor::commands as cmd;
                    let entries: [(&str, &str, fn(&mut Document)); 7] = [
                        ("Nächstes Vorkommen auswählen", "Strg+D", cmd::select_next_occurrence),
                        ("Alle Vorkommen auswählen", "Strg+Umschalt+L", cmd::select_all_occurrences),
                        ("Zeile auswählen", "Strg+L", cmd::select_line),
                        ("Zeile duplizieren", "Strg+Umschalt+D", cmd::duplicate_lines),
                        ("Zeile löschen", "Strg+Umschalt+K", cmd::delete_lines),
                        ("Kommentar umschalten", "Strg+/", cmd::toggle_comment),
                        ("Alles auswählen", "Strg+A", cmd::select_all),
                    ];
                    for (label, shortcut, f) in entries {
                        if ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked() {
                            if let Some(d) = self.active_doc() {
                                f(d);
                            }
                            self.focus_editor = true;
                            ui.close();
                        }
                    }
                });
            });
            ui.menu_button("Ansicht", |ui| {
                if ui
                    .add(egui::Button::new("Seitenleiste").shortcut_text("Strg+B").selected(self.show_sidebar))
                    .clicked()
                {
                    self.show_sidebar = !self.show_sidebar;
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("SSH-Terminal").shortcut_text("Strg+Ö").selected(self.terminals.visible))
                    .clicked()
                {
                    self.toggle_terminal(&ui.ctx().clone());
                    ui.close();
                }
                let e = &mut self.settings.editor;
                if ui
                    .add(egui::Button::new("Automatischer Zeilenumbruch").shortcut_text("Alt+Z").selected(e.word_wrap))
                    .clicked()
                {
                    e.word_wrap = !e.word_wrap;
                    ui.close();
                }
                ui.checkbox(&mut e.show_invisibles, "Unsichtbare Zeichen");
                ui.checkbox(&mut e.show_indent_guides, "Einrückungshilfslinien");
                ui.checkbox(&mut e.highlight_current_line, "Aktuelle Zeile hervorheben");
                ui.separator();
                if ui.button(if self.settings.dark_theme { "Helles Design" } else { "Dunkles Design" }).clicked() {
                    self.settings.dark_theme = !self.settings.dark_theme;
                    self.theme = Theme::for_settings(&self.settings);
                    self.theme.apply(ui.ctx());
                    ui.close();
                }
                ui.separator();
                if ui.add(egui::Button::new("Schrift größer").shortcut_text("Strg++")).clicked() {
                    self.settings.editor.font_size = (self.settings.editor.font_size + 1.0).min(40.0);
                }
                if ui.add(egui::Button::new("Schrift kleiner").shortcut_text("Strg+-")).clicked() {
                    self.settings.editor.font_size = (self.settings.editor.font_size - 1.0).max(8.0);
                }
            });
            ui.menu_button("Server", |ui| {
                if ui.button("Server hinzufügen…").clicked() {
                    self.open_site_form(None);
                    ui.close();
                }
                ui.separator();
                let sites: Vec<(String, String)> =
                    self.sites.sites.iter().map(|s| (s.id.clone(), s.display_name())).collect();
                for (id, name) in sites {
                    let connected = self.remote.is_connected(&id);
                    ui.menu_button(name, |ui| {
                        if connected {
                            if ui.button("Trennen").clicked() {
                                self.remote.disconnect(&id);
                                self.sidebar.remote.remove(&id);
                                ui.close();
                            }
                        } else if ui.button("Verbinden").clicked() {
                            self.show_sidebar = true;
                            self.sidebar.tab = crate::ui::sidebar::SidebarTab::Servers;
                            self.sidebar.expanded_sites.insert(id.clone());
                            if let Some(site) = self.sites.get(&id).cloned() {
                                self.ensure_tree(&site);
                            }
                            ui.close();
                        }
                        if ui.button("SSH-Terminal öffnen").clicked() {
                            self.open_terminal(&ui.ctx().clone(), &id);
                            ui.close();
                        }
                        if ui.button("Bearbeiten…").clicked() {
                            self.open_site_form(Some(id.clone()));
                            ui.close();
                        }
                    });
                }
            });
            ui.menu_button("Einstellungen", |ui| {
                for (label, page) in [("Editor…", Page::Editor), ("Design und Themes…", Page::Design), ("Sprachen…", Page::Languages), ("Erweiterungen…", Page::Extensions)] {
                    if ui.add(egui::Button::new(label).shortcut_text(if page == Page::Editor { "Strg+," } else { "" })).clicked() {
                        self.open_settings(page);
                        ui.close();
                    }
                }
            });
            ui.menu_button("Erweiterungen", |ui| {
                let cmds = self.available_ext_commands();
                let mut last_ext = String::new();
                for c in cmds.iter().filter(|c| c.section != "command-palette") {
                    if c.ext != last_ext {
                        if !last_ext.is_empty() {
                            ui.separator();
                        }
                        ui.label(RichText::new(&c.ext_name).small().color(self.theme.text_faint));
                        last_ext = c.ext.clone();
                    }
                    let shortcut = c.shortcut.map(|s| ui.ctx().format_shortcut(&s)).unwrap_or_default();
                    if ui.add(egui::Button::new(&c.title).shortcut_text(shortcut)).clicked() {
                        self.run_ext_command(c);
                        // Edits the command makes can be undone right away.
                        self.focus_editor = true;
                        ui.close();
                    }
                }
                if !last_ext.is_empty() {
                    ui.separator();
                }
                if ui.button("Erweiterungsbibliothek…").clicked() {
                    self.settings_ui.ext.tab = extensions_page::Tab::Library;
                    self.open_settings(Page::Extensions);
                    ui.close();
                }
                if ui.button("Erweiterungen verwalten…").clicked() {
                    self.settings_ui.ext.tab = extensions_page::Tab::Installed;
                    self.open_settings(Page::Extensions);
                    ui.close();
                }
                if ui.button("Erweiterungskonsole").clicked() {
                    self.settings_ui.ext.tab = extensions_page::Tab::Console;
                    self.open_settings(Page::Extensions);
                    ui.close();
                }
            });
        }
    }

    /// Title bar row: sidebar toggle, menu, tabs and caption buttons.
    fn title_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let (bar, bar_resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), chrome::TITLEBAR_H), Sense::click_and_drag());
        ui.painter().rect_filled(bar, 0.0, theme.titlebar_bg);
        chrome::window_drag(ui, &bar_resp);

        let content = Rect::from_min_max(
            pos2(bar.left() + 8.0, bar.top() + 7.0),
            pos2(bar.right() - chrome::caption_buttons_width() - 8.0, bar.bottom() - 7.0),
        );
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content)
                .layout(Layout::left_to_right(Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 4.0;
        let btn = vec2(30.0, 30.0);
        if icons::icon_button(&mut row, &theme, btn, false, |p, c, col| icons::sidebar_toggle(p, c, col))
            .on_hover_text("Seitenleiste ein/aus (Strg+B)")
            .clicked()
        {
            self.show_sidebar = !self.show_sidebar;
        }
        let menu = icons::icon_button(&mut row, &theme, btn, false, |p, c, col| icons::menu(p, c, col))
            .on_hover_text("Menü");
        egui::Popup::menu(&menu).show(|ui| {
            ui.set_min_width(190.0);
            self.menu_contents(ui);
        });
        row.add_space(6.0);

        // Tabs as rounded pills.
        let count = self.docs.len();
        let available = row.available_width() - 60.0;
        let font = FontId::proportional(13.0);
        let natural: Vec<f32> = self
            .docs
            .iter()
            .map(|d| {
                let w = row.fonts_mut(|f| f.layout_no_wrap(d.title(), font.clone(), theme.foreground).size().x);
                let icon = if matches!(d.source, DocSource::Remote { .. }) { 15.0 } else { 0.0 };
                (w + 62.0 + icon).clamp(110.0, 240.0)
            })
            .collect();
        let total: f32 = natural.iter().sum::<f32>() + count as f32 * 4.0;
        // Shrink a little first; beyond that the tab strip scrolls.
        let scale = if total > available && total > 0.0 { (available / total).max(0.8) } else { 1.0 };
        let strip_w = available.min(total * scale + 4.0).max(0.0);

        #[derive(Clone, Copy)]
        struct TabDrag(usize);
        let mut close = None;
        let mut activate = None;
        let mut tab_cmd: Option<(usize, tabs::TabCmd)> = None;
        let mut reorder: Option<(usize, usize)> = None;
        let reveal = std::mem::take(&mut self.reveal_active_tab);
        row.allocate_ui(vec2(strip_w, 30.0), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("tab-strip")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    // Mouse wheel scrolls the strip sideways.
                    if ui.ui_contains_pointer() {
                        let dy = ui.input(|i| i.smooth_scroll_delta.y);
                        if dy != 0.0 {
                            ui.scroll_with_delta(vec2(dy, 0.0));
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (i, doc) in self.docs.iter().enumerate() {
                            let w = (natural[i] * scale).max(90.0);
                            let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click_and_drag());
                            resp.dnd_set_drag_payload(TabDrag(i));
                            let active = self.active == Some(i);
                            if active && reveal {
                                resp.scroll_to_me(None);
                            }
                            let painter = ui.painter().clone();
                            if active {
                                painter.rect_filled(rect, 8.0, theme.tab_active_bg);
                            } else if resp.hovered() {
                                painter.rect_filled(rect, 8.0, theme.tab_hover_bg);
                            }
                            if let Some(from) = resp.dnd_hover_payload::<TabDrag>()
                                && from.0 != i
                            {
                                let x = if from.0 < i { rect.right() + 2.0 } else { rect.left() - 2.0 };
                                painter.line_segment(
                                    [pos2(x, rect.top() + 4.0), pos2(x, rect.bottom() - 4.0)],
                                    egui::Stroke::new(2.0, theme.accent),
                                );
                            }
                            if let Some(from) = resp.dnd_release_payload::<TabDrag>() {
                                reorder = Some((from.0, i));
                            }
                            let cy = rect.center().y;
                            let mut x = rect.left() + 18.0;
                            icons::file(&painter, pos2(x, cy), &doc.title());
                            x += 14.0;
                            if matches!(doc.source, DocSource::Remote { .. }) {
                                icons::cloud(&painter, pos2(x + 4.0, cy), theme.accent);
                                x += 15.0;
                            }
                            let color = if active { theme.foreground } else { theme.text_muted };
                            let galley = painter.layout_no_wrap(doc.title(), font.clone(), color);
                            let text_clip =
                                Rect::from_min_max(pos2(x, rect.top()), pos2(rect.right() - 26.0, rect.bottom()));
                            painter
                                .with_clip_rect(text_clip.intersect(ui.clip_rect()))
                                .galley(pos2(x, cy - galley.size().y / 2.0), galley, color);

                            let close_rect = Rect::from_center_size(pos2(rect.right() - 15.0, cy), vec2(18.0, 18.0));
                            let close_resp =
                                ui.interact(close_rect, egui::Id::unique(("tab-close", doc.id)), Sense::click());
                            if doc.saving {
                                ui.put(close_rect.shrink(2.0), egui::Spinner::new().size(12.0).color(theme.accent));
                            } else if doc.is_dirty() && !close_resp.hovered() {
                                painter.circle_filled(close_rect.center(), 3.8, theme.modified);
                            } else if resp.hovered() || active || close_resp.hovered() {
                                if close_resp.hovered() {
                                    painter.rect_filled(close_rect, 5.0, theme.hover.gamma_multiply(2.0));
                                }
                                icons::close(&painter, close_rect.center(), 7.0, color);
                            }
                            if close_resp.clicked() || resp.middle_clicked() {
                                close = Some(i);
                            } else if resp.clicked() {
                                activate = Some(i);
                            }
                            let count = self.docs.len();
                            resp.context_menu(|ui| {
                                ui.set_min_width(220.0);
                                tabs::tab_menu(ui, doc, i, count, &mut tab_cmd);
                            });
                            resp.on_hover_text(match &doc.source {
                                DocSource::Remote { site_id, path } => format!(
                                    "{} – {}",
                                    self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default(),
                                    path
                                ),
                                s => s.path_string(),
                            });
                        }
                    });
                });
        });
        if let Some((from, to)) = reorder
            && from != to
            && from < self.docs.len()
            && to < self.docs.len()
        {
            let active_id = self.active.map(|a| self.docs[a].id);
            let doc = self.docs.remove(from);
            self.docs.insert(to, doc);
            self.active = active_id.and_then(|id| self.doc_index(id));
        }
        if icons::icon_button(&mut row, &theme, vec2(26.0, 26.0), false, |p, c, col| icons::plus(p, c, col))
            .on_hover_text("Neue Datei (Strg+N)")
            .clicked()
        {
            self.new_document();
        }
        if let Some(i) = activate {
            self.sidebar.focused = false;
            self.activate(i);
        }
        if let Some(i) = close {
            self.close_doc(i, false);
        }
        if let Some((i, cmd)) = tab_cmd {
            self.run_tab_cmd(&ui.ctx().clone(), i, cmd);
        }

        chrome::caption_buttons(ui, &theme, bar);
        ui.painter().line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            egui::Stroke::new(1.0, theme.separator),
        );
    }

    /// Breadcrumbs for the active document, like Nova's jump bar.
    fn jump_bar(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.active else { return };
        let theme = self.theme.clone();
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let painter = ui.painter().with_clip_rect(bar);
        painter.rect_filled(bar, 0.0, theme.jumpbar_bg);
        painter.line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            egui::Stroke::new(1.0, theme.separator.gamma_multiply(if theme.dark { 0.8 } else { 0.5 })),
        );

        let doc = &self.docs[i];
        let head = doc.primary_selection().head;
        let key = (doc.id, doc.version, head);
        if self.outline_cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let path = crate::editor::outline::symbol_path(doc, head);
            self.outline_cache = Some((key, path));
        }
        let symbols = self.outline_cache.as_ref().map(|(_, p)| p.clone()).unwrap_or_default();

        // (label, kind): 0 = location root, 1 = folder, 2 = file, 3 = symbol
        let mut parts: Vec<(String, u8)> = Vec::new();
        match &doc.source {
            DocSource::Remote { site_id, path } => {
                let site = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default();
                parts.push((site, 0));
                let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                for (n, s) in segs.iter().enumerate() {
                    parts.push((s.to_string(), if n + 1 == segs.len() { 2 } else { 1 }));
                }
            }
            DocSource::Local(p) => {
                let rel = self
                    .sidebar
                    .root
                    .as_ref()
                    .and_then(|root| p.strip_prefix(root).ok().map(|r| (root, r)));
                match rel {
                    Some((root, r)) => {
                        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        parts.push((name, 0));
                        let comps: Vec<String> =
                            r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                        for (n, c) in comps.iter().enumerate() {
                            parts.push((c.clone(), if n + 1 == comps.len() { 2 } else { 1 }));
                        }
                    }
                    None => {
                        if let Some(parent) = p.parent().and_then(|p| p.file_name()) {
                            parts.push((parent.to_string_lossy().into_owned(), 0));
                        }
                        parts.push((doc.title(), 2));
                    }
                }
            }
            DocSource::Untitled(_) => parts.push((doc.title(), 2)),
        }
        parts.extend(symbols.into_iter().map(|s| (s, 3)));

        let mut x = bar.left() + 14.0;
        let cy = bar.center().y;
        let font = FontId::proportional(12.5);
        for (n, (label, kind)) in parts.iter().enumerate() {
            if n > 0 {
                icons::chevron_thin(&painter, pos2(x + 9.0, cy), theme.text_faint);
                x += 21.0;
            }
            match kind {
                0 => {
                    if matches!(doc.source, DocSource::Remote { .. }) {
                        icons::server(&painter, pos2(x + 7.0, cy), theme.text_muted);
                    } else {
                        icons::folder(&painter, pos2(x + 8.0, cy), theme.text_muted, false);
                    }
                    x += 20.0;
                }
                2 => {
                    icons::file(&painter, pos2(x + 6.0, cy), label);
                    x += 17.0;
                }
                3 => {
                    painter.rect_stroke(
                        Rect::from_center_size(pos2(x + 5.0, cy), vec2(8.0, 8.0)),
                        2.0,
                        egui::Stroke::new(1.2, theme.color_of("tag.name")),
                        egui::StrokeKind::Inside,
                    );
                    x += 15.0;
                }
                _ => {}
            }
            let color = if *kind == 2 { theme.foreground } else { theme.text_muted };
            let g = painter.layout_no_wrap(label.clone(), font.clone(), color);
            let w = g.size().x;
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
            x += w;
            if x > bar.right() - 44.0 {
                break;
            }
        }

        // Live preview toggle at the right end.
        let r = Rect::from_center_size(pos2(bar.right() - 20.0, cy), vec2(28.0, 24.0));
        let resp = ui
            .interact(r, egui::Id::unique("jump_preview"), Sense::click())
            .on_hover_text("Live-Vorschau (Strg+Umschalt+V)");
        if self.settings.preview {
            ui.painter().rect_filled(r, 6.0, theme.hover.gamma_multiply(2.2));
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 6.0, theme.hover);
        }
        let color = if self.settings.preview || resp.hovered() { theme.foreground } else { theme.text_muted };
        icons::preview(ui.painter(), r.center(), color);
        if resp.clicked() {
            self.toggle_preview();
        }
    }

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.active else { return };
        let doc = &mut self.docs[i];
        self.find.update(doc);
        let f = &mut self.find;
        let mut action: Option<&str> = None;
        egui::Frame::new()
            .fill(self.theme.sidebar_bg)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut f.query)
                            .hint_text("Suchen")
                            .desired_width(260.0),
                    );
                    if std::mem::take(&mut f.focus_query) {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        action = Some(if ui.input(|i| i.modifiers.shift) { "prev" } else { "next" });
                        resp.request_focus();
                    }
                    ui.toggle_value(&mut f.case_sensitive, "Aa").on_hover_text("Groß-/Kleinschreibung");
                    ui.toggle_value(&mut f.whole_word, "W").on_hover_text("Ganzes Wort");
                    ui.toggle_value(&mut f.use_regex, ".*").on_hover_text("Regulärer Ausdruck");
                    let label = match (&f.error, f.matches.len(), f.current) {
                        (Some(e), _, _) => RichText::new(e).color(Color32::from_rgb(230, 100, 100)),
                        (None, 0, _) if !f.query.is_empty() => RichText::new("Keine Treffer"),
                        (None, 0, _) => RichText::new(""),
                        (None, n, Some(c)) => RichText::new(format!("{} von {n}", c + 1)),
                        (None, n, None) => RichText::new(format!("{n} Treffer")),
                    };
                    if ui.button("▲").on_hover_text("Vorheriger (Umschalt+F3)").clicked() {
                        action = Some("prev");
                    }
                    if ui.button("▼").on_hover_text("Nächster (F3)").clicked() {
                        action = Some("next");
                    }
                    ui.label(label.small());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("×").clicked() {
                            action = Some("close");
                        }
                        ui.toggle_value(&mut f.show_replace, "Ersetzen");
                    });
                });
                if f.show_replace {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut f.replacement)
                                .hint_text("Ersetzen durch")
                                .desired_width(260.0),
                        );
                        if ui.button("Ersetzen").clicked() {
                            action = Some("replace");
                        }
                        if ui.button("Alle ersetzen").clicked() {
                            action = Some("replace_all");
                        }
                    });
                }
            });
        match action {
            Some("next") => self.find.find(doc, true),
            Some("prev") => self.find.find(doc, false),
            Some("replace") => self.find.replace_current(doc),
            Some("replace_all") => {
                let n = self.find.replace_all(doc);
                self.info(format!("{n} Ersetzungen"));
            }
            Some("close") => {
                self.find.visible = false;
                self.focus_editor = true;
            }
            _ => {}
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let pending = self.remote.total_pending();
        let theme = self.theme.clone();
        let rect = ui.max_rect();
        ui.painter().line_segment(
            [rect.left_top(), rect.right_top()],
            egui::Stroke::new(1.0, theme.separator.gamma_multiply(if theme.dark { 0.8 } else { 0.5 })),
        );
        // Done after drawing, as they need the whole app.
        let mut goto_line = false;
        let mut reload_as: Option<crate::editor::document::Encoding> = None;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let small = |s: String| RichText::new(s).size(11.5).color(theme.text_muted);
            let clickable = |ui: &mut egui::Ui, s: String| {
                let r = ui.add(egui::Label::new(small(s)).sense(Sense::click()));
                if r.hovered() {
                    ui.painter().rect_filled(r.rect.expand2(vec2(5.0, 2.0)), 4.0, theme.hover);
                }
                r
            };
            if pending > 0 {
                ui.add(egui::Spinner::new().size(11.0).color(theme.accent));
                ui.label(small(format!("{pending} Übertragung(en)…")));
            } else if let Some(DocSource::Remote { site_id, .. }) = self.active.and_then(|i| self.docs.get(i)).map(|d| &d.source) {
                let connected = self.remote.is_connected(site_id);
                let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.5, if connected { theme.success } else { theme.text_faint });
                let site = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default();
                ui.label(small(if connected { format!("Verbunden mit {site}") } else { site }));
            }
            let Some(i) = self.active else { return };
            let e = self.settings.editor_for_doc(&self.docs[i]);
            let tab_size = e.tab_size;
            let doc = &mut self.docs[i];
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                // Icon and name form one click target.
                let galley = ui.painter().layout_no_wrap(doc.language.name().to_string(), FontId::proportional(11.5), theme.text_muted);
                let (rect, lang) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, galley.size().y), Sense::click());
                if lang.hovered() {
                    ui.painter().rect_filled(rect.expand2(vec2(5.0, 2.0)), 4.0, theme.hover);
                }
                icons::language(ui.painter(), pos2(rect.left() + 6.0, rect.center().y), doc.language);
                ui.painter().galley(pos2(rect.right() - galley.size().x, rect.top()), galley, theme.text_muted);
                let lang = lang.on_hover_text("Sprache");
                egui::Popup::menu(&lang).show(|ui| {
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        ui.set_min_width(170.0);
                        for l in Language::ALL {
                            if icons::language_item(ui, doc.language == l, l, l.name()).clicked() {
                                doc.language = l;
                                doc.highlight_version = u64::MAX;
                                ui.close();
                            }
                        }
                    });
                });
                let enc = clickable(ui, doc.encoding.label().to_string()).on_hover_text("Kodierung");
                egui::Popup::menu(&enc).show(|ui| {
                    use crate::editor::document::Encoding;
                    ui.set_min_width(280.0);
                    ui.label(RichText::new("Speichern mit Kodierung").small().color(theme.text_faint));
                    let text = doc.text();
                    for e in Encoding::ALL {
                        let fits = e.can_encode(&text);
                        let resp = ui.add_enabled(fits, egui::Button::selectable(doc.encoding == e, e.description()));
                        if resp.clicked() && doc.encoding != e {
                            doc.encoding = e;
                            doc.mark_unsaved();
                            ui.close();
                        }
                        if !fits {
                            resp.on_disabled_hover_text("Der Text enthält Zeichen, die diese Kodierung nicht darstellen kann.");
                        }
                    }
                    let reloadable = !matches!(doc.source, DocSource::Untitled(_));
                    if reloadable {
                        ui.separator();
                        ui.label(RichText::new("Neu laden mit Kodierung").small().color(theme.text_faint));
                        let dirty = doc.is_dirty();
                        for e in Encoding::ALL {
                            let resp = ui.add_enabled(!dirty, egui::Button::new(e.description()));
                            if resp.clicked() {
                                reload_as = Some(e);
                                ui.close();
                            }
                            if dirty {
                                resp.on_disabled_hover_text("Erst speichern oder Änderungen verwerfen");
                            }
                        }
                    }
                });
                if clickable(ui, doc.line_ending.label().to_string())
                    .on_hover_text("Zeilenenden umschalten")
                    .clicked()
                {
                    use crate::editor::document::LineEnding;
                    doc.line_ending = match doc.line_ending {
                        LineEnding::Lf => LineEnding::CrLf,
                        LineEnding::CrLf => LineEnding::Lf,
                    };
                    doc.mark_unsaved();
                }
                ui.label(small(format!(
                    "{} {}",
                    if e.use_spaces { "Leerzeichen:" } else { "Tabs:" },
                    e.tab_size
                )));
                let sel = doc.primary_selection();
                let line = doc.char_to_line(sel.head);
                let col = doc.column_of(sel.head, tab_size);
                let mut info = format!("Zeile {}, Spalte {}", line + 1, col + 1);
                if doc.selections.len() > 1 {
                    info = format!("{} Cursor  ·  {info}", doc.selections.len());
                } else if !sel.is_empty() {
                    info = format!("{} ausgewählt  ·  {info}", sel.range().len());
                }
                if clickable(ui, info).on_hover_text("Gehe zu Zeile (Strg+G)").clicked() {
                    goto_line = true;
                }

                // Problems reported by extensions for this document.
                if !doc.view.diagnostics.is_empty() {
                    let errors = doc.view.diagnostics.iter().filter(|d| d.severity == 1).count();
                    let warnings = doc.view.diagnostics.len() - errors;
                    let text = RichText::new(format!("✕ {errors}   ⚠ {warnings}")).size(11.5).color(if errors > 0 { theme.danger } else { theme.modified });
                    let r = ui.add(egui::Label::new(text).sense(Sense::click())).on_hover_text("Probleme");
                    egui::Popup::menu(&r).show(|ui| {
                        ui.set_max_width(520.0);
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for d in doc.view.diagnostics.clone() {
                                let line = doc.char_to_line(d.range.start);
                                let color = match d.severity {
                                    1 => theme.danger,
                                    2 => theme.modified,
                                    _ => theme.accent,
                                };
                                let label = RichText::new(format!("{}  {}", line + 1, d.message)).color(color);
                                if ui.add(egui::Button::new(label).frame(false).truncate()).clicked() {
                                    doc.set_selection(crate::editor::document::Selection::cursor(d.range.start));
                                    doc.view.center_line = Some(line);
                                    ui.close();
                                }
                            }
                        });
                    });
                }
            });
        });
        if goto_line {
            self.dialog = Some(Dialog::GotoLine { value: String::new(), focus: true });
        }
        if let Some(e) = reload_as {
            self.reload_with_encoding(e);
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let area = ui.available_rect_before_wrap();
        ui.painter().rect_filled(area, 0.0, theme.background);
        let icon = self
            .icon_texture
            .get_or_insert_with(|| {
                let size = 160;
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [size, size],
                    &crate::icon::render_icon(size as u32),
                );
                ui.ctx().load_texture("app-icon", image, egui::TextureOptions::LINEAR)
            })
            .clone();

        let card_size = vec2(680.0_f32.min(area.width() - 40.0), 400.0_f32.min(area.height() - 40.0));
        let card = Rect::from_center_size(area.center(), card_size);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(card));
        let painter = ui.painter().clone();
        painter.rect_filled(card, 14.0, theme.sidebar_bg);
        painter.rect_stroke(card, 14.0, egui::Stroke::new(1.0, theme.hover.gamma_multiply(1.5)), egui::StrokeKind::Inside);

        // Left: icon and name.
        let left = Rect::from_min_size(card.min, vec2(card.width() * 0.42, card.height()));
        painter.image(
            icon.id(),
            Rect::from_center_size(left.center() - vec2(0.0, 40.0), vec2(112.0, 112.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        painter.text(
            left.center() + vec2(0.0, 44.0),
            Align2::CENTER_CENTER,
            APP_NAME,
            FontId::new(28.0, crate::theme::bold_family()),
            theme.foreground,
        );
        painter.text(
            left.center() + vec2(0.0, 74.0),
            Align2::CENTER_CENTER,
            format!("Version {}", env!("CARGO_PKG_VERSION")),
            FontId::proportional(12.5),
            theme.text_faint,
        );
        painter.line_segment(
            [pos2(left.right(), card.top() + 24.0), pos2(left.right(), card.bottom() - 24.0)],
            egui::Stroke::new(1.0, theme.hover.gamma_multiply(1.5)),
        );

        // Right: actions, recent files and servers.
        let right = Rect::from_min_max(pos2(left.right() + 18.0, card.top() + 22.0), card.max - vec2(18.0, 18.0));
        let mut col = ui.new_child(egui::UiBuilder::new().max_rect(right));
        col.spacing_mut().item_spacing.y = 2.0;
        let heading = |ui: &mut egui::Ui, t: &str| {
            ui.add_space(6.0);
            ui.label(RichText::new(t.to_uppercase()).font(FontId::new(10.5, crate::theme::bold_family())).color(theme.text_faint));
            ui.add_space(2.0);
        };
        heading(&mut col, "Start");
        type IconFn = fn(&egui::Painter, egui::Pos2, Color32);
        let actions: [(IconFn, &str, &str); 4] = [
            (icons::new_file, "Neue Datei", "Strg+N"),
            (|p, c, col| icons::folder(p, c, col, true), "Ordner öffnen…", "Strg+Umschalt+O"),
            (|p, c, col| icons::search(p, c, col), "Datei öffnen…", "Strg+O"),
            (icons::server, "Server hinzufügen…", ""),
        ];
        for (n, (icon, label, hint)) in actions.into_iter().enumerate() {
            if welcome_row(&mut col, &theme, |p, c| icon(p, c, theme.accent), label, hint).clicked() {
                match n {
                    0 => self.new_document(),
                    1 => self.pick_and_open_folder(),
                    2 => self.pick_and_open_file(),
                    _ => self.open_site_form(None),
                }
            }
        }

        let servers: Vec<(String, String)> =
            self.sites.sites.iter().take(4).map(|s| (s.id.clone(), s.display_name())).collect();
        if !servers.is_empty() {
            heading(&mut col, "Server");
            for (id, name) in servers {
                if welcome_row(&mut col, &theme, |p, c| icons::server(p, c, theme.text_muted), &name, "Verbinden").clicked() {
                    self.show_sidebar = true;
                    self.sidebar.tab = crate::ui::sidebar::SidebarTab::Servers;
                    self.sidebar.expanded_sites.insert(id.clone());
                    if let Some(site) = self.sites.get(&id).cloned() {
                        self.ensure_tree(&site);
                    }
                }
            }
        }
        let current = self.sidebar.root.clone();
        let folders: Vec<PathBuf> = self.settings.recent_folders.iter().filter(|p| Some(*p) != current.as_ref()).take(5).cloned().collect();
        if !folders.is_empty() {
            heading(&mut col, "Projekte");
            for p in folders {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                let dir = p.parent().and_then(|d| d.file_name()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                if welcome_row(&mut col, &theme, |painter, c| icons::folder(painter, c, theme.text_muted, false), &name, &dir)
                    .on_hover_text(p.display().to_string())
                    .clicked()
                {
                    let ctx = col.ctx().clone();
                    self.handle_sidebar_action(&ctx, SidebarAction::SwitchFolder(p));
                }
            }
        }
        let recent: Vec<PathBuf> = self.settings.recent_files.iter().take(5).cloned().collect();
        if !recent.is_empty() {
            heading(&mut col, "Zuletzt geöffnet");
            for p in recent {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let dir = p.parent().and_then(|d| d.file_name()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                if welcome_row(&mut col, &theme, |painter, c| icons::file(painter, c, &name), &name, &dir)
                    .on_hover_text(p.display().to_string())
                    .clicked()
                {
                    self.open_local(&p);
                }
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let accent = self.theme.accent;
        // Host key confirmation takes priority over everything else.
        if let Some((site_id, prompt)) = self.host_keys.front() {
            let site_name = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default();
            let mut answer = None;
            modal(ctx, egui::Id::unique("hostkey")).show(ctx, |ui| {
                ui.set_width(460.0);
                if prompt.changed {
                    ui.heading(RichText::new("⚠ Serverschlüssel hat sich geändert!").color(Color32::from_rgb(240, 90, 90)));
                    ui.label("Der Server meldet einen anderen Schlüssel als beim letzten Mal. Das kann auf einen Angriff hindeuten – oder der Server wurde neu installiert.");
                } else {
                    ui.heading("Unbekannter Server");
                    ui.label(format!("Die Echtheit von {}:{} ({site_name}) kann nicht bestätigt werden.", prompt.host, prompt.port));
                }
                ui.add_space(8.0);
                ui.label(format!("{}-Fingerabdruck:", prompt.algorithm));
                ui.code(&prompt.fingerprint);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(primary_button("Vertrauen und verbinden", accent)).clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Abbrechen").clicked() {
                        answer = Some(false);
                    }
                });
            });
            if let Some(a) = answer {
                let (_, prompt) = self.host_keys.pop_front().unwrap();
                prompt.answer(a);
            }
            return;
        }

        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let dialog = match dialog {
            Dialog::EditSite(mut form) => {
                if !form.password_loaded {
                    form.password = form.site.password().unwrap_or_default();
                    form.password_loaded = true;
                }
                let mut save = false;
                modal(ctx, egui::Id::unique("site")).show(ctx, |ui| {
                    ui.set_width(440.0);
                    ui.heading(if form.is_new { "Server hinzufügen" } else { "Server bearbeiten" });
                    ui.add_space(8.0);
                    egui::Grid::new("site-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        let s = &mut form.site;
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut s.name).hint_text("Meine Website").desired_width(280.0));
                        ui.end_row();
                        ui.label("Protokoll");
                        let before = s.protocol;
                        egui::ComboBox::from_id_salt("proto")
                            .selected_text(s.protocol.label())
                            .width(280.0)
                            .show_ui(ui, |ui| {
                                for p in Protocol::ALL {
                                    ui.selectable_value(&mut s.protocol, p, p.label());
                                }
                            });
                        if before != s.protocol && s.port == before.default_port() {
                            s.port = s.protocol.default_port();
                        }
                        ui.end_row();
                        ui.label("Server");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut s.host).hint_text("example.com").desired_width(200.0));
                            ui.label("Port");
                            ui.add(egui::DragValue::new(&mut s.port).range(1..=65535));
                        });
                        ui.end_row();
                        ui.label("Benutzer");
                        ui.add(egui::TextEdit::singleline(&mut s.user).desired_width(280.0));
                        ui.end_row();
                        ui.label(if s.protocol == Protocol::Sftp && s.key_file.is_some() { "Passphrase" } else { "Passwort" });
                        ui.add(egui::TextEdit::singleline(&mut form.password).password(true).desired_width(280.0));
                        ui.end_row();
                        ui.label("Startpfad");
                        ui.add(egui::TextEdit::singleline(&mut s.root_path).hint_text("/httpdocs (leer = Home)").desired_width(280.0));
                        ui.end_row();
                        ui.label("Web-URL");
                        ui.add(egui::TextEdit::singleline(&mut s.web_url).hint_text("https://example.com").desired_width(280.0));
                        ui.end_row();
                        match s.protocol {
                            Protocol::Sftp => {
                                ui.label("Schlüssel");
                                ui.horizontal(|ui| {
                                    let label = s
                                        .key_file
                                        .as_ref()
                                        .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "automatisch (~/.ssh)".into());
                                    ui.label(label);
                                    if ui.button("Wählen…").clicked()
                                        && let Some(p) = rfd::FileDialog::new().pick_file()
                                    {
                                        s.key_file = Some(p);
                                    }
                                    if s.key_file.is_some() && ui.button("×").clicked() {
                                        s.key_file = None;
                                    }
                                });
                                ui.end_row();
                            }
                            Protocol::Ftp | Protocol::Ftps => {
                                ui.label("");
                                ui.checkbox(&mut s.passive, "Passiver Modus");
                                ui.end_row();
                                if s.protocol == Protocol::Ftps {
                                    ui.label("");
                                    ui.checkbox(&mut s.accept_invalid_certs, "Ungültige Zertifikate akzeptieren");
                                    ui.end_row();
                                }
                            }
                        }
                    });
                    if form.site.protocol == Protocol::Ftp {
                        ui.add_space(4.0);
                        ui.label(RichText::new("Hinweis: FTP überträgt Passwörter unverschlüsselt. Wenn möglich SFTP oder FTPS verwenden.").small().color(self.theme.modified));
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("Passwörter werden in der Windows-Anmeldeinformationsverwaltung gespeichert.").small().color(self.theme.gutter_fg));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let valid = !form.site.host.trim().is_empty();
                        if ui.add_enabled(valid, primary_button("Sichern", accent)).clicked() {
                            save = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            keep = false;
                        }
                    });
                });
                if save {
                    keep = false;
                    let mut site = form.site.clone();
                    site.host = site.host.trim().to_string();
                    site.user = site.user.trim().to_string();
                    if let Err(e) = site.set_password(&form.password) {
                        self.error(format!("{e:#}"));
                    }
                    // Reconnect with the new settings.
                    self.remote.disconnect(&site.id);
                    self.sidebar.remote.remove(&site.id);
                    match self.sites.sites.iter_mut().find(|s| s.id == site.id) {
                        Some(existing) => *existing = site,
                        None => self.sites.sites.push(site),
                    }
                    if let Err(e) = self.sites.save() {
                        self.error(format!("Server konnten nicht gespeichert werden: {e:#}"));
                    }
                    self.show_sidebar = true;
                    self.sidebar.tab = crate::ui::sidebar::SidebarTab::Servers;
                }
                Dialog::EditSite(form)
            }
            Dialog::AskName { title, mut value, action, focus } => {
                let mut ok = false;
                modal(ctx, egui::Id::unique("ask-name")).show(ctx, |ui| {
                    ui.set_width(360.0);
                    ui.heading(&title);
                    let resp = ui.add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY));
                    if focus {
                        resp.request_focus();
                        // Preselect the name without extension.
                        let stem = value.rfind('.').filter(|&i| i > 0).unwrap_or(value.len());
                        let mut state = egui::TextEdit::load_state(ui.ctx(), resp.id).unwrap_or_default();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(value[..stem].chars().count()),
                        )));
                        state.store(ui.ctx(), resp.id);
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        ok = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_button("OK", accent)).clicked() {
                            ok = true;
                        }
                        if ui.button("Abbrechen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            keep = false;
                        }
                    });
                });
                if ok {
                    keep = false;
                    self.apply_name_action(action.clone(), value.clone());
                }
                Dialog::AskName { title, value, action, focus: false }
            }
            Dialog::ConfirmDelete(items) => {
                let mut yes = false;
                let remote = items.iter().any(|i| matches!(i.file, FileRef::Remote { .. }));
                let what = if items.len() == 1 { format!("„{}“", items[0].name()) } else { format!("{} Objekte", items.len()) };
                let folders = if items.iter().any(|i| i.is_dir) { "\nOrdner werden mit allen Inhalten gelöscht." } else { "" };
                let text = format!("{what} wirklich{} löschen?{folders}", if remote { " vom Server" } else { "" });
                confirm(ctx, "Löschen?", &text, "Löschen", &mut yes, &mut keep);
                if yes {
                    self.delete_items(items.clone());
                }
                Dialog::ConfirmDelete(items)
            }
            Dialog::TransferConflict { name, queue, mut apply_all } => {
                let mut choice = None;
                let more = queue.items.len() > 1;
                modal(ctx, egui::Id::unique("transfer-conflict")).show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.heading("Name bereits vorhanden");
                    ui.add(egui::Label::new(format!("Im Zielordner gibt es bereits „{name}“.")).wrap());
                    if more {
                        ui.add_space(6.0);
                        ui.checkbox(&mut apply_all, format!("Für alle {} Objekte übernehmen", queue.items.len()));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_button("Ersetzen", accent)).clicked() {
                            choice = Some(ConflictPolicy::Replace);
                        }
                        if ui.button("Beide behalten").clicked() {
                            choice = Some(ConflictPolicy::KeepBoth);
                        }
                        if ui.button("Überspringen").clicked() {
                            choice = Some(ConflictPolicy::Skip);
                        }
                        if ui.button("Abbrechen").clicked() {
                            keep = false;
                        }
                    });
                });
                if let Some(policy) = choice {
                    // The queue moves into the resolver; nothing to keep.
                    self.resolve_conflict(queue, policy, apply_all);
                    return;
                }
                Dialog::TransferConflict { name, queue, apply_all }
            }
            Dialog::ConfirmRemoveSite(id) => {
                let name = self.sites.get(&id).map(|s| s.display_name()).unwrap_or_default();
                let mut yes = false;
                confirm(ctx, "Server entfernen?", &format!("„{name}“ aus der Liste entfernen? Dateien auf dem Server bleiben unberührt."), "Entfernen", &mut yes, &mut keep);
                if yes {
                    self.remote.disconnect(&id);
                    self.sidebar.remote.remove(&id);
                    if let Some(site) = self.sites.get(&id) {
                        site.delete_password();
                    }
                    self.sites.sites.retain(|s| s.id != id);
                    let _ = self.sites.save();
                }
                Dialog::ConfirmRemoveSite(id)
            }
            Dialog::UnsavedChanges { doc_ids, quit } => {
                let names: Vec<String> = doc_ids
                    .iter()
                    .filter_map(|id| self.doc_index(*id))
                    .map(|i| self.docs[i].title())
                    .collect();
                let mut choice = None;
                modal(ctx, egui::Id::unique("unsaved")).show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.heading("Ungesicherte Änderungen");
                    if names.len() == 1 {
                        ui.label(format!("Änderungen an „{}“ speichern?", names[0]));
                    } else {
                        ui.label("Folgende Dateien haben ungesicherte Änderungen:");
                        for n in &names {
                            ui.label(format!("  • {n}"));
                        }
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_button("Speichern", accent)).clicked() {
                            choice = Some(0);
                        }
                        if ui.button("Nicht speichern").clicked() {
                            choice = Some(1);
                        }
                        if ui.button("Abbrechen").clicked() {
                            choice = Some(2);
                        }
                    });
                });
                match choice {
                    Some(0) => {
                        keep = false;
                        for id in &doc_ids {
                            if let Some(i) = self.doc_index(*id) {
                                if quit {
                                    self.save_doc_with(i, false, false);
                                } else {
                                    self.save_doc(i, true);
                                }
                            }
                        }
                        if quit && self.pending_saves.is_empty() && !self.docs.iter().any(|d| d.is_dirty()) {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        } else if quit {
                            self.info("Wird gespeichert… Danach erneut schließen.");
                        }
                    }
                    Some(1) => {
                        keep = false;
                        if quit {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        } else {
                            for id in &doc_ids {
                                if let Some(i) = self.doc_index(*id) {
                                    self.close_doc(i, true);
                                }
                            }
                        }
                    }
                    Some(_) => keep = false,
                    None => {}
                }
                Dialog::UnsavedChanges { doc_ids, quit }
            }
            Dialog::Conflict { doc_id, close_after, saving } => {
                let index = self.doc_index(doc_id);
                let name = index.map(|i| self.docs[i].title()).unwrap_or_default();
                let local = index.is_some_and(|i| matches!(self.docs[i].source, DocSource::Local(_)));
                let (place, theirs) = if local { ("auf der Festplatte", "Version von der Festplatte laden") } else { ("auf dem Server", "Server-Version laden") };
                let mut choice = None;
                modal(ctx, egui::Id::unique("conflict")).show(ctx, |ui| {
                    ui.set_width(440.0);
                    ui.heading(format!("Datei {place} geändert"));
                    let text = if saving {
                        format!("„{name}“ wurde {place} verändert, seit du sie geöffnet hast. Beim Speichern würden diese Änderungen überschrieben.")
                    } else {
                        format!("„{name}“ wurde von einem anderen Programm verändert. Du hast hier ungespeicherte Änderungen.")
                    };
                    ui.add(egui::Label::new(text).wrap());
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if saving {
                            if ui.add(primary_button("Überschreiben", Color32::from_rgb(214, 58, 58))).clicked() {
                                choice = Some(0);
                            }
                        } else if ui.add(primary_button("Meine Version behalten", self.theme.accent)).on_hover_text("Beim Speichern wird die Datei überschrieben").clicked() {
                            choice = Some(2);
                        }
                        if ui.button(theirs).on_hover_text("Eigene Änderungen verwerfen (rückgängig machbar)").clicked() {
                            choice = Some(1);
                        }
                        if saving && ui.button("Abbrechen").clicked() {
                            choice = Some(2);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    match c {
                        0 => {
                            if let Some(i) = self.doc_index(doc_id) {
                                self.save_doc_with(i, close_after, true);
                            }
                        }
                        1 if local => self.reload_local(doc_id),
                        1 => self.reload_remote(doc_id),
                        _ => {}
                    }
                }
                Dialog::Conflict { doc_id, close_after, saving }
            }
            Dialog::PublishSetup { root, mut target } => {
                let mut save = false;
                let mut unlink = false;
                let linked = self.settings.publish.contains_key(&root.to_string_lossy().into_owned());
                let sites: Vec<(String, String)> = self.sites.sites.iter().map(|s| (s.id.clone(), s.display_name())).collect();
                modal(ctx, egui::Id::unique("publish-setup")).show(ctx, |ui| {
                    ui.set_width(460.0);
                    ui.heading("Veröffentlichen einrichten");
                    ui.label(RichText::new(format!("Projekt: {}", root.display())).color(self.theme.text_faint));
                    ui.add_space(10.0);
                    if sites.is_empty() {
                        ui.label("Lege zuerst einen Server an.");
                    }
                    egui::Grid::new("publish-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label("Server");
                        let current = sites.iter().find(|(id, _)| *id == target.site_id).map(|(_, n)| n.clone()).unwrap_or_default();
                        egui::ComboBox::from_id_salt("publish-site").selected_text(current).width(300.0).show_ui(ui, |ui| {
                            for (id, name) in &sites {
                                ui.selectable_value(&mut target.site_id, id.clone(), name);
                            }
                        });
                        ui.end_row();
                        ui.label("Zielordner");
                        ui.add(egui::TextEdit::singleline(&mut target.remote_path).hint_text("/httpdocs").desired_width(300.0));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut target.upload_on_save, "Beim Speichern automatisch hochladen");
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("„Veröffentlichen“ lädt alle seit dem letzten Mal geänderten Dateien hoch.").small().color(self.theme.text_faint));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!target.site_id.is_empty(), primary_button("Sichern", accent)).clicked() {
                            save = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            keep = false;
                        }
                        if linked && ui.button("Verknüpfung entfernen").clicked() {
                            unlink = true;
                        }
                    });
                });
                let key = root.to_string_lossy().into_owned();
                if save {
                    keep = false;
                    // Keep the upload history when only options changed.
                    if let Some(old) = self.settings.publish.get(&key)
                        && old.site_id == target.site_id
                        && old.remote_path == target.remote_path
                    {
                        target.uploaded = old.uploaded.clone();
                    }
                    self.settings.publish.insert(key, target.clone());
                    let _ = self.settings.save();
                    self.info("Veröffentlichen eingerichtet");
                } else if unlink {
                    keep = false;
                    self.settings.publish.remove(&key);
                    let _ = self.settings.save();
                }
                Dialog::PublishSetup { root, target }
            }
            Dialog::GotoLine { mut value, focus } => {
                let mut go = false;
                modal(ctx, egui::Id::unique("goto")).show(ctx, |ui| {
                    ui.set_width(260.0);
                    ui.label("Gehe zu Zeile");
                    let resp = ui.add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY));
                    if focus {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        go = true;
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        keep = false;
                    }
                });
                if go {
                    keep = false;
                    if let (Ok(n), Some(doc)) = (value.trim().parse::<usize>(), self.active_doc()) {
                        let line = n.saturating_sub(1).min(doc.len_lines() - 1);
                        let pos = doc.line_to_char(line);
                        doc.set_selection(crate::editor::document::Selection::cursor(pos));
                        doc.view.center_line = Some(line);
                    }
                    self.focus_editor = true;
                }
                Dialog::GotoLine { value, focus: false }
            }
            Dialog::Permissions { items, mut mode, mut octal, mut recurse } => {
                let mut ok = false;
                let any_dir = items.iter().any(|i| i.is_dir);
                let what = if items.len() == 1 { format!("„{}“", items[0].name()) } else { format!("{} Objekte", items.len()) };
                modal(ctx, egui::Id::unique("permissions")).show(ctx, |ui| {
                    ui.set_width(400.0);
                    ui.heading("Berechtigungen");
                    ui.label(RichText::new(what).color(self.theme.text_muted));
                    ui.add_space(10.0);
                    let before = mode;
                    egui::Grid::new("perm-grid").num_columns(4).spacing([18.0, 8.0]).show(ui, |ui| {
                        ui.label("");
                        for h in ["Lesen", "Schreiben", "Ausführen"] {
                            ui.label(RichText::new(h).color(self.theme.text_muted));
                        }
                        ui.end_row();
                        for (label, shift) in [("Besitzer", 6), ("Gruppe", 3), ("Andere", 0)] {
                            ui.label(label);
                            for bit in [4, 2, 1] {
                                let mask = bit << shift;
                                let mut on = mode & mask != 0;
                                if ui.checkbox(&mut on, "").changed() {
                                    mode = if on { mode | mask } else { mode & !mask };
                                }
                            }
                            ui.end_row();
                        }
                    });
                    if mode != before {
                        octal = format!("{mode:03o}");
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Numerisch");
                        let resp = ui.add(egui::TextEdit::singleline(&mut octal).char_limit(3).desired_width(48.0).font(egui::TextStyle::Monospace));
                        if resp.changed()
                            && let Ok(m) = u32::from_str_radix(octal.trim(), 8)
                            && m <= 0o777
                        {
                            mode = m;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            ok = true;
                        }
                        ui.label(RichText::new(remote::mode_string(mode)).monospace().color(self.theme.text_faint));
                    });
                    if any_dir {
                        ui.add_space(8.0);
                        let mut on = recurse.is_some();
                        if ui.checkbox(&mut on, "Auch auf den Inhalt von Ordnern anwenden").changed() {
                            recurse = on.then_some(remote::Recurse::All);
                        }
                        if let Some(r) = &mut recurse {
                            ui.horizontal(|ui| {
                                ui.add_space(24.0);
                                ui.radio_value(r, remote::Recurse::All, "Alles");
                                ui.radio_value(r, remote::Recurse::Files, "Nur Dateien");
                                ui.radio_value(r, remote::Recurse::Folders, "Nur Ordner");
                            });
                        }
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_button("Übernehmen", accent)).clicked() {
                            ok = true;
                        }
                        if ui.button("Abbrechen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            keep = false;
                        }
                    });
                });
                if ok {
                    keep = false;
                    for item in &items {
                        if let FileRef::Remote { site_id, path } = &item.file {
                            let request = Request::Chmod {
                                path: path.clone(),
                                mode,
                                is_dir: item.is_dir,
                                recurse: if item.is_dir { recurse } else { None },
                            };
                            self.remote_request(site_id, request);
                        }
                    }
                }
                Dialog::Permissions { items, mode, octal, recurse }
            }
            Dialog::QuickOpen { mut query, files, mut selected, focus } => {
                let mut open: Option<PathBuf> = None;
                let root = self.sidebar.root.clone().unwrap_or_default();
                modal(ctx, egui::Id::unique("quick-open")).show(ctx, |ui| {
                    ui.set_width(520.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text("Dateiname eingeben…")
                            .desired_width(f32::INFINITY),
                    );
                    if focus || resp.changed() {
                        resp.request_focus();
                    }
                    if resp.changed() {
                        selected = 0;
                    }
                    let results = fuzzy_filter(&files, &root, &query, 14);
                    ui.input_mut(|i| {
                        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                            selected = (selected + 1).min(results.len().saturating_sub(1));
                        }
                        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                            selected = selected.saturating_sub(1);
                        }
                    });
                    ui.add_space(4.0);
                    for (i, (rel, path)) in results.iter().enumerate() {
                        if ui.selectable_label(i == selected, rel.as_str()).clicked() {
                            open = Some(path.clone());
                        }
                    }
                    if ui.input(|i| i.key_pressed(Key::Enter))
                        && let Some((_, p)) = results.get(selected)
                    {
                        open = Some(p.clone());
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        keep = false;
                    }
                });
                if let Some(p) = open {
                    keep = false;
                    self.open_local(&p);
                }
                Dialog::QuickOpen { query, files, selected, focus: false }
            }
            Dialog::Settings => {
                keep = self.settings_window(ctx);
                Dialog::Settings
            }
            Dialog::CommandPalette(mut state) => {
                keep = self.command_palette(ctx, &mut state);
                Dialog::CommandPalette(state)
            }
        };
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        let theme = &self.theme;
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-18.0 - self.preview_state.covered_width, -40.0))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                let mut dismiss = None;
                for (i, t) in self.toasts.iter().enumerate() {
                    let bar = match t.kind {
                        ToastKind::Info => theme.success,
                        ToastKind::Error => theme.danger,
                    };
                    let resp = egui::Frame::new()
                        .fill(ui.visuals().window_fill)
                        .stroke(ui.visuals().window_stroke)
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin { left: 18, right: 14, top: 10, bottom: 10 })
                        .shadow(ui.visuals().popup_shadow)
                        .show(ui, |ui| ui.add(egui::Label::new(RichText::new(&t.text).color(theme.foreground)).wrap()))
                        .response;
                    let r = resp.rect;
                    ui.painter().rect_filled(
                        Rect::from_min_size(r.min + vec2(7.0, 9.0), vec2(3.0, r.height() - 18.0)),
                        2.0,
                        bar,
                    );
                    if resp.interact(Sense::click()).clicked() {
                        dismiss = Some(i);
                    }
                    ui.add_space(6.0);
                }
                if let Some(i) = dismiss {
                    self.toasts.remove(i);
                }
            });
        let next = self.toasts.iter().map(|t| t.until).min().unwrap();
        ctx.request_repaint_after(next.saturating_duration_since(now));
    }

    fn update_highlights(&mut self) {
        for res in self.highlighter.poll() {
            if let Some(doc) = self.docs.iter_mut().find(|d| d.id == res.doc_id)
                && (doc.highlight_version == u64::MAX || res.version >= doc.highlight_version || doc.highlight_version > doc.version)
            {
                doc.highlights = res.lines;
                doc.highlight_version = res.version;
                if doc.highlight_pending == Some(res.version) {
                    doc.highlight_pending = None;
                }
            }
        }
        // Only the visible document needs fresh colors.
        if let Some(i) = self.active {
            let doc = &mut self.docs[i];
            if doc.highlight_version != doc.version && doc.highlight_pending != Some(doc.version) {
                let text = doc.text();
                self.highlighter.request(doc.id, doc.version, doc.language, text);
                doc.highlight_pending = Some(doc.version);
                if doc.highlight_version == u64::MAX {
                    // Language changed: drop stale colors.
                    doc.highlights = Arc::new(Vec::new());
                }
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.active.and_then(|i| self.docs.get(i)) {
            Some(doc) => {
                let dirty = if doc.is_dirty() { "● " } else { "" };
                match &doc.source {
                    DocSource::Remote { site_id, .. } => format!(
                        "{dirty}{} – {} – {APP_NAME}",
                        doc.title(),
                        self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default()
                    ),
                    _ => format!("{dirty}{} – {APP_NAME}", doc.title()),
                }
            }
            None => APP_NAME.to_string(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}

impl eframe::App for NebulaApp {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ui = root;
        crate::win32::init(frame, &ctx);
        let backdrop = (self.theme.dark, self.settings.mica);
        if self.backdrop_state != Some(backdrop) {
            crate::win32::set_backdrop(backdrop.0, backdrop.1);
            self.backdrop_state = Some(backdrop);
        }
        self.process_remote_events();
        self.poll_disk(&ctx);
        self.backup_unsaved(&ctx);
        self.poll_terminals();
        self.poll_printing();
        self.serve_preview();
        self.sync_extensions(&ctx);

        // Files dropped onto the window.
        // Files dragged in from Explorer: highlight the folder under the
        // cursor while hovering, then copy/upload there or open them.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        self.sidebar.external_hover = if hovering {
            ctx.request_repaint();
            crate::win32::cursor_pos(ctx.pixels_per_point())
        } else {
            None
        };
        if !dropped.is_empty() {
            self.handle_external_drop(&ctx, dropped);
        }

        // Closing the window with unsaved changes.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            let dirty: Vec<u64> = self.docs.iter().filter(|d| d.is_dirty()).map(|d| d.id).collect();
            if !dirty.is_empty() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.dialog = Some(Dialog::UnsavedChanges { doc_ids: dirty, quit: true });
            }
        }

        if self.dialog.is_none() && self.host_keys.is_empty() {
            self.handle_shortcuts(&ctx);
            if !self.terminal_focused(&ctx) {
                self.ext_shortcuts(&ctx);
            }
        }
        self.update_highlights();

        if self.show_sidebar {
            let mut actions = Vec::new();
            if self.dialog.is_none() {
                self.sidebar.handle_keys(&ctx, &mut actions);
            }
            let active_path = self.active.and_then(|i| self.docs.get(i)).map(|d| d.source.path_string());
            let resp = egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(self.settings.sidebar_width)
                .size_range(160.0..=600.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(self.theme.sidebar_bg))
                .show(ui, |ui| {
                    self.sidebar.has_extension_sidebars = self.ext.installed.iter().any(|e| !e.sidebars.is_empty() && !self.settings.disabled_extensions.contains(&e.identifier));
                    self.sidebar.show(ui, &self.sites, &self.theme, active_path.as_deref(), &mut actions);
                    if let Some(rect) = self.sidebar.extension_rect.take() {
                        self.extension_sidebars(ui, rect);
                    }
                });
            let r = resp.response.rect;
            ui.painter().line_segment(
                [r.right_top(), r.right_bottom()],
                egui::Stroke::new(1.0, self.theme.separator),
            );
            self.settings.sidebar_width = r.width();
            for a in actions {
                self.handle_sidebar_action(&ctx, a);
            }
            self.drag_out_of_window(&ctx);
        }

        let mut preview_rect = None;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            self.title_bar(ui);
            // Below the (translucent) title bar everything sits on the document color.
            ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, self.theme.background);
            if self.docs.is_empty() {
                self.terminal_panel(ui);
                self.welcome(ui);
                return;
            }
            egui::Panel::bottom("status")
                .exact_size(26.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(self.theme.jumpbar_bg).inner_margin(egui::Margin::symmetric(12, 0)))
                .show(ui, |ui| self.status_bar(ui));
            self.terminal_panel(ui);
            self.jump_bar(ui);
            if self.find.visible {
                self.find_bar(ui);
            }
            let Some(i) = self.active else { return };
            // Editor and, optionally, the live preview side by side.
            let full = ui.available_rect_before_wrap();
            let mut editor_rect = full;
            if self.settings.preview && full.width() > 420.0 {
                let w = (full.width() * self.settings.preview_split).clamp(200.0, full.width() - 200.0);
                let split = (full.right() - w).round();
                editor_rect.max.x = split;
                let pane = Rect::from_min_max(pos2(split, full.top()), full.max);
                preview_rect = self.preview_pane(ui, pane, full);
            }
            let matches = if self.find.visible { std::mem::take(&mut self.find.matches) } else { Vec::new() };
            let editor_settings = self.settings.editor_for_doc(&self.docs[i]);
            let doc = &mut self.docs[i];
            let doc_id = doc.id;
            let out = ui
                .scope_builder(egui::UiBuilder::new().max_rect(editor_rect), |ui| {
                    EditorView {
                        doc,
                        settings: &editor_settings,
                        theme: &self.theme,
                        matches: &matches,
                        current_match: self.find.current,
                    }
                    .show(ui)
                })
                .inner;
            if self.find.visible {
                self.find.matches = matches;
            }
            if let Some(text) = out.copied {
                ctx.copy_text(text);
            }
            if out.has_focus {
                self.sidebar.focused = false;
            }
            if std::mem::take(&mut self.focus_editor) && self.dialog.is_none() && !self.find.focus_query {
                ctx.memory_mut(|m| m.request_focus(editor_id(doc_id)));
            }
        });

        if self.dialog.is_none() && self.host_keys.is_empty() {
            self.ext_prompts(&ctx);
        }
        self.dialogs(&ctx);
        self.toasts(&ctx);
        self.ext_notices(&ctx);
        self.place_preview(&ctx, frame, preview_rect);
        self.update_title(&ctx);
        chrome::resize_borders(&ctx);
    }

    // Scroll offsets are keyed by per-session document ids, so restoring
    // them would scroll the wrong files.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent so the Mica backdrop shows through translucent panels.
        [0.0, 0.0, 0.0, 0.0]
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.store_session();
        let _ = self.settings.save();
    }

    fn on_exit(&mut self) {
        self.discard_backups();
        self.store_session();
        let _ = self.settings.save();
    }
}

fn confirm(ctx: &egui::Context, title: &str, text: &str, action: &str, yes: &mut bool, keep: &mut bool) {
    modal(ctx, egui::Id::unique(("confirm", title))).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading(title);
        ui.add(egui::Label::new(text).wrap());
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add(primary_button(action, Color32::from_rgb(214, 58, 58))).clicked() {
                *yes = true;
                *keep = false;
            }
            if ui.button("Abbrechen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                *keep = false;
            }
        });
    });
}

fn is_binary(bytes: &[u8]) -> bool {
    // UTF-16 text is full of zero bytes.
    if bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
        return false;
    }
    bytes[..bytes.len().min(8192)].contains(&0)
}

fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}

/// Lists files below `root`, skipping VCS and dependency folders.
fn collect_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[".git", "node_modules", "vendor", ".svn", ".hg", "target", ".idea", ".vs"];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !SKIP.iter().any(|s| name == *s) {
                    stack.push(e.path());
                }
            } else {
                out.push(e.path());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// Subsequence match scored by contiguity and filename hits.
fn fuzzy_filter(files: &[PathBuf], root: &Path, query: &str, max: usize) -> Vec<(String, PathBuf)> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let mut scored: Vec<(i64, String, &PathBuf)> = files
        .iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
            if q.is_empty() {
                return Some((0, rel, p));
            }
            let lower = rel.to_lowercase();
            let name_start = lower.rfind('/').map_or(0, |i| i + 1);
            let mut score = 0i64;
            let mut qi = 0;
            let mut last: Option<usize> = None;
            for (i, c) in lower.chars().enumerate() {
                if qi < q.len() && c == q[qi] {
                    score += 1;
                    if last == Some(i.wrapping_sub(1)) {
                        score += 5;
                    }
                    if i >= name_start {
                        score += 3;
                    }
                    last = Some(i);
                    qi += 1;
                }
            }
            (qi == q.len()).then(|| (score * 100 - rel.len() as i64, rel, p))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(max).map(|(_, r, p)| (r, p.clone())).collect()
}

/// A clickable row on the welcome screen: icon, label and a muted hint.
fn welcome_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    icon: impl FnOnce(&egui::Painter, egui::Pos2),
    label: &str,
    hint: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, 6.0, theme.hover.gamma_multiply(1.4));
    }
    let cy = rect.center().y;
    icon(painter, pos2(rect.left() + 16.0, cy));
    painter.text(pos2(rect.left() + 34.0, cy), Align2::LEFT_CENTER, label, FontId::proportional(13.5), theme.foreground);
    painter.text(pos2(rect.right() - 8.0, cy), Align2::RIGHT_CENTER, hint, FontId::proportional(11.5), theme.text_faint);
    resp
}

/// Filled button for the main action of a dialog.
fn primary_button(text: &str, fill: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(Color32::WHITE))
        .fill(fill)
        .stroke(egui::Stroke::NONE)
        .min_size(vec2(84.0, 28.0))
}

/// A modal dialog with window styling and generous padding.
fn modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    let style = ctx.global_style();
    egui::Modal::new(id).frame(
        egui::Frame::window(&style)
            .inner_margin(egui::Margin::same(22))
            .corner_radius(12.0),
    )
}
