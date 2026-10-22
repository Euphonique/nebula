//! Sidebar: local folder tree and remote servers, with selection, clipboard
//! and drag and drop.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2,
};

use super::icons;
use crate::remote::{RemoteEntry, Request};
use crate::settings::{Site, Sites};
use crate::theme::{Theme, bold_family};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab {
    Files,
    Servers,
    Search,
    Git,
    /// Sidebars contributed by extensions.
    Extensions,
}

/// A file or folder, local or on a server.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileRef {
    Local(PathBuf),
    Remote { site_id: String, path: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Item {
    pub file: FileRef,
    pub is_dir: bool,
}

impl Item {
    pub fn local(path: PathBuf, is_dir: bool) -> Self {
        Self { file: FileRef::Local(path), is_dir }
    }

    pub fn remote(site_id: &str, path: &str, is_dir: bool) -> Self {
        Self {
            file: FileRef::Remote { site_id: site_id.to_string(), path: path.to_string() },
            is_dir,
        }
    }

    pub fn name(&self) -> String {
        match &self.file {
            FileRef::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            FileRef::Remote { path, .. } => path.rsplit('/').next().unwrap_or(path).to_string(),
        }
    }

    pub fn parent(&self) -> Option<DirRef> {
        Some(match &self.file {
            FileRef::Local(p) => DirRef::Local(p.parent()?.to_path_buf()),
            FileRef::Remote { site_id, path } => DirRef::Remote {
                site_id: site_id.clone(),
                path: crate::remote::parent(path),
            },
        })
    }
}

/// A folder that items can be pasted or dropped into.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DirRef {
    Local(PathBuf),
    Remote { site_id: String, path: String },
}

impl DirRef {
    /// True if `item` is this folder or one of its ancestors (moving would recurse).
    pub fn is_inside(&self, item: &Item) -> bool {
        match (self, &item.file) {
            (DirRef::Local(d), FileRef::Local(p)) => item.is_dir && d.starts_with(p),
            (DirRef::Remote { site_id: a, path: d }, FileRef::Remote { site_id: b, path: p }) => {
                a == b && item.is_dir && (d == p || d.starts_with(&format!("{p}/")))
            }
            _ => false,
        }
    }
}

/// Whether dropping copies (like Explorer): between different places
/// (another server, local and server) copy, within the same place move;
/// Shift forces moving, Ctrl copying.
pub fn drop_copies(items: &[Item], target: &DirRef, mods: Modifiers) -> bool {
    if mods.shift {
        return false;
    }
    if mods.command {
        return true;
    }
    items.iter().any(|i| match (&i.file, target) {
        (FileRef::Local(_), DirRef::Local(_)) => false,
        (FileRef::Remote { site_id: a, .. }, DirRef::Remote { site_id: b, .. }) => a != b,
        _ => true,
    })
}

/// "Kopieren"/"Verschieben" next to the pointer while dragging over a target.
fn drop_hint(ui: &Ui, theme: &Theme, copy: bool) {
    let Some(pos) = ui.ctx().pointer_latest_pos() else { return };
    let text = if copy { "+ Kopieren (Umschalt: verschieben)" } else { "→ Verschieben (Strg: kopieren)" };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("drop-hint")));
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(11.5), theme.foreground);
    let rect = Rect::from_min_size(pos + vec2(16.0, 18.0), galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(rect, 5.0, theme.sidebar_bg);
    painter.rect_stroke(rect, 5.0, Stroke::new(1.0, theme.accent), egui::StrokeKind::Inside);
    painter.galley(rect.min + vec2(6.0, 3.0), galley, theme.foreground);
}

/// Payload while dragging items inside the app.
#[derive(Clone)]
pub struct DragItems(pub Vec<Item>);

#[derive(Clone, Debug)]
pub enum NameAction {
    NewRemoteFile { site_id: String, dir: String },
    NewRemoteFolder { site_id: String, dir: String },
    RenameRemote { site_id: String, path: String },
    NewLocalFile { dir: PathBuf },
    NewLocalFolder { dir: PathBuf },
    RenameLocal { path: PathBuf },
    /// Put a server into a new group.
    NewGroup { site_id: String },
    RenameGroup { old: String },
}

/// Something the user asked for in the sidebar; handled by the app.
pub enum SidebarAction {
    OpenLocal(PathBuf),
    OpenFolder,
    /// Switch to a recently used project folder.
    SwitchFolder(PathBuf),
    OpenRemote { site_id: String, path: String },
    Connect(String),
    Disconnect(String),
    Remote { site_id: String, request: Request },
    AskName { title: String, initial: String, action: NameAction },
    ConfirmDelete(Vec<Item>),
    EditSite(Option<String>),
    /// New server, preset to a group.
    AddSiteToGroup(String),
    DuplicateSite(String),
    RemoveSite(String),
    /// Move a server into a group (empty = none).
    SetSiteGroup { site_id: String, group: String },
    /// Fold or unfold a group of servers.
    ToggleGroup(String),
    /// Remove the group, keeping its servers.
    DissolveGroup(String),
    ImportSites,
    ImportFileZilla,
    ExportSites,
    OpenUrl(String),
    CopyText(String),
    /// Put items on the clipboard (`cut` moves them on paste).
    Clipboard { items: Vec<Item>, cut: bool },
    Paste(DirRef),
    /// Copy or move items into a folder (drag and drop).
    Transfer { items: Vec<Item>, target: DirRef, copy: bool },
    Duplicate(Vec<Item>),
    UploadFiles { site_id: String, dir: String },
    DownloadTo(Item),
    /// Change permissions of server files (chmod).
    Permissions(Vec<Item>),
    /// Open an SSH terminal on a server.
    OpenTerminal(String),
    Publish,
    PublishSetup,
    /// Two-way synchronization of the project with its server folder.
    Sync,
    Search(super::search::SearchAction),
    /// Project search limited to a folder (local or on a server).
    SearchIn(DirRef),
    /// Compare two files, or one file with the active tab.
    Compare(Vec<FileRef>),
    Git(super::git_panel::GitAction),
    /// A saved command in the server's SSH terminal.
    RunCommand { site_id: String, command: String },
    /// Send files along with the next message to the AI agent.
    AttachToAgent(Vec<FileRef>),
}

#[derive(Clone, PartialEq)]
pub enum RemoteStatus {
    Connecting,
    Connected,
    Failed(String),
}

pub struct RemoteTree {
    pub status: RemoteStatus,
    pub home: Option<String>,
    pub dirs: HashMap<String, Vec<RemoteEntry>>,
    pub expanded: HashSet<String>,
    pub loading: HashSet<String>,
}

impl RemoteTree {
    pub fn new() -> Self {
        Self {
            status: RemoteStatus::Connecting,
            home: None,
            dirs: HashMap::new(),
            expanded: HashSet::new(),
            loading: HashSet::new(),
        }
    }
}

#[derive(Clone)]
pub struct LocalEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Per-frame context for drawing tree rows.
struct RowCtx<'a> {
    theme: &'a Theme,
    active_path: Option<&'a str>,
    /// Folder under an OS drag from Explorer, in screen coordinates.
    external_hover: Option<Pos2>,
    modifiers: Modifiers,
    /// Connected servers (id, name, start folder) as targets for "copy to server".
    servers: Vec<(String, String, String)>,
}

pub struct Sidebar {
    pub tab: SidebarTab,
    pub root: Option<PathBuf>,
    local_dirs: HashMap<PathBuf, Vec<LocalEntry>>,
    local_expanded: HashSet<PathBuf>,
    pub remote: HashMap<String, RemoteTree>,
    pub expanded_sites: HashSet<String>,
    pub filter: String,
    local_filter: super::file_filter::LocalFilter,
    pub selection: Vec<Item>,
    /// Internal clipboard and the system clipboard sequence when it was set.
    pub clipboard: Option<(Vec<Item>, bool, u32)>,
    /// The sidebar has keyboard focus (last click was in it).
    pub focused: bool,
    /// Pointer position during an OS drag (set by the app).
    pub external_hover: Option<Pos2>,
    /// Show the tab for extension sidebars.
    pub has_extension_sidebars: bool,
    /// Area the app fills with extension sidebars this frame.
    pub extension_rect: Option<Rect>,
    /// Rows that accept drops this frame, for drops from Explorer.
    drop_targets: Vec<(Rect, DirRef)>,
    list_rect: Rect,
    /// Folder hovered while dragging, for spring-loaded expansion.
    spring: Option<(String, Instant)>,
    pub search: super::search::SearchPanel,
    pub git: super::git::GitStatus,
    pub git_panel: super::git_panel::GitPanel,
    /// Recent project folders for the switcher in the header.
    pub recent_folders: Vec<PathBuf>,
}

impl Sidebar {
    pub fn new(root: Option<PathBuf>) -> Self {
        let root = root.map(|r| normalize_path(&r));
        Self {
            tab: if root.is_some() { SidebarTab::Files } else { SidebarTab::Servers },
            root,
            local_dirs: HashMap::new(),
            local_expanded: HashSet::new(),
            remote: HashMap::new(),
            expanded_sites: HashSet::new(),
            filter: String::new(),
            local_filter: Default::default(),
            selection: Vec::new(),
            clipboard: None,
            focused: false,
            external_hover: None,
            has_extension_sidebars: false,
            extension_rect: None,
            drop_targets: Vec::new(),
            list_rect: Rect::NOTHING,
            spring: None,
            search: Default::default(),
            git: Default::default(),
            git_panel: Default::default(),
            recent_folders: Vec::new(),
        }
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.root = Some(normalize_path(&root));
        self.local_dirs.clear();
        self.local_expanded.clear();
        self.selection.clear();
        self.tab = SidebarTab::Files;
    }

    pub fn refresh_local(&mut self, dir: Option<&Path>) {
        self.local_filter.invalidate();
        match dir {
            Some(d) => {
                self.local_dirs.remove(d);
            }
            None => self.local_dirs.clear(),
        }
    }

    /// Shows the search tab. Coming from the files or servers view, it
    /// searches where the user just was: the selected folder (or the folder
    /// of the selected file), else the project or the connected server.
    pub fn open_search(&mut self) {
        if self.tab != SidebarTab::Search {
            if let Some(scope) = self.context_scope() {
                self.search.set_scope(scope);
            }
            self.tab = SidebarTab::Search;
        }
        self.search.focus_query = true;
    }

    fn context_scope(&self) -> Option<super::search::Scope> {
        use super::search::Scope;
        let root = self.root.as_deref();
        let remote_view = match self.tab {
            SidebarTab::Files => false,
            SidebarTab::Servers => true,
            _ => return None,
        };
        let selected = self
            .selection
            .iter()
            .rev()
            .find(|i| matches!(i.file, FileRef::Remote { .. }) == remote_view)
            .and_then(|item| {
                if !item.is_dir {
                    return item.parent();
                }
                Some(match &item.file {
                    FileRef::Local(p) => DirRef::Local(p.clone()),
                    FileRef::Remote { site_id, path } => DirRef::Remote { site_id: site_id.clone(), path: path.clone() },
                })
            });
        if let Some(dir) = selected {
            return Some(Scope::for_dir(&dir, root));
        }
        if !remote_view {
            return Some(Scope::default());
        }
        // No selection: the start folder of a connected server.
        let mut connected: Vec<_> = self
            .expanded_sites
            .iter()
            .filter_map(|id| Some((id.clone(), self.remote.get(id)?.home.clone()?)))
            .collect();
        connected.sort();
        connected.into_iter().next().map(|(site_id, path)| Scope::Remote { site_id, path })
    }

    /// Names in a remote folder, if it has been listed.
    pub fn remote_names(&self, site_id: &str, dir: &str) -> Option<Vec<String>> {
        let tree = self.remote.get(site_id)?;
        Some(tree.dirs.get(dir)?.iter().map(|e| e.name.clone()).collect())
    }

    /// Permission bits of a server file, if its folder was listed with them.
    pub fn remote_mode(&self, site_id: &str, path: &str) -> Option<u32> {
        let tree = self.remote.get(site_id)?;
        tree.dirs.get(&crate::remote::parent(path))?.iter().find(|e| e.path == path)?.mode
    }

    /// Folder under `pos` (screen coordinates) for drops from Explorer.
    pub fn drop_target_at(&self, pos: Pos2) -> Option<DirRef> {
        if !self.list_rect.contains(pos) {
            return None;
        }
        self.drop_targets
            .iter()
            .find(|(r, _)| r.contains(pos))
            .map(|(_, d)| d.clone())
            .or_else(|| self.default_target())
    }

    pub fn contains(&self, pos: Pos2) -> bool {
        self.list_rect.contains(pos)
    }

    /// Where pasting goes when nothing specific is selected.
    pub fn default_target(&self) -> Option<DirRef> {
        if let Some(item) = self.selection.last() {
            return if item.is_dir {
                Some(match &item.file {
                    FileRef::Local(p) => DirRef::Local(p.clone()),
                    FileRef::Remote { site_id, path } => DirRef::Remote { site_id: site_id.clone(), path: path.clone() },
                })
            } else {
                item.parent()
            };
        }
        match self.tab {
            SidebarTab::Files | SidebarTab::Search | SidebarTab::Git | SidebarTab::Extensions => self.root.clone().map(DirRef::Local),
            SidebarTab::Servers => self
                .expanded_sites
                .iter()
                .find_map(|id| {
                    let home = self.remote.get(id)?.home.clone()?;
                    Some(DirRef::Remote { site_id: id.clone(), path: home })
                }),
        }
    }

    pub fn local_children(&mut self, dir: &Path) -> Vec<LocalEntry> {
        if let Some(v) = self.local_dirs.get(dir) {
            return v.clone();
        }
        let mut entries: Vec<LocalEntry> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    // Hidden like in Nova: VCS internals and OS clutter.
                    .filter(|e| !matches!(e.file_name().to_str(), Some(".git" | ".svn" | ".hg" | ".DS_Store" | "Thumbs.db" | "desktop.ini")))
                    .map(|e| LocalEntry {
                        name: e.file_name().to_string_lossy().into_owned(),
                        is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                        path: e.path(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.local_dirs.insert(dir.to_path_buf(), entries.clone());
        entries
    }

    /// Keyboard shortcuts while the sidebar has focus.
    pub fn handle_keys(&mut self, ctx: &egui::Context, actions: &mut Vec<SidebarAction>) {
        if !self.focused || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Copy | egui::Event::Cut if !self.selection.is_empty() => {
                    let cut = matches!(event, egui::Event::Cut);
                    actions.push(SidebarAction::Clipboard { items: self.selection.clone(), cut });
                }
                // egui turns Ctrl+V into a text Paste event and drops it when the
                // clipboard holds files, so react to the key release instead.
                egui::Event::Key { key: Key::V, pressed: false, modifiers, .. } if modifiers.command => {
                    if let Some(t) = self.default_target() {
                        actions.push(SidebarAction::Paste(t));
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::D if modifiers.command && !self.selection.is_empty() => {
                        actions.push(SidebarAction::Duplicate(self.selection.clone()));
                    }
                    Key::Delete if !self.selection.is_empty() => {
                        actions.push(SidebarAction::ConfirmDelete(self.selection.clone()));
                    }
                    Key::F2 => {
                        if let Some(item) = self.selection.last() {
                            actions.push(rename_action(item));
                        }
                    }
                    Key::Escape => self.selection.clear(),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        sites: &Sites,
        theme: &Theme,
        active_path: Option<&str>,
        actions: &mut Vec<SidebarAction>,
    ) {
        let full = ui.max_rect();
        self.git.poll(self.root.as_deref(), ui.ctx());
        let dragging = egui::DragAndDrop::has_payload_of_type::<DragItems>(ui.ctx());
        // Mode switcher and actions, with the same margin above as at the sides.
        let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(header.min + vec2(10.0, 10.0), vec2(header.width() - 20.0, 26.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        {
            let ui = &mut child;
            ui.spacing_mut().item_spacing.x = 2.0;
            let seg = vec2(34.0, 26.0);
            let files = icons::icon_button(ui, theme, seg, self.tab == SidebarTab::Files, |p, c, col| {
                icons::folder(p, c, col, false)
            })
            .on_hover_text("Dateien");
            // Hovering a tab while dragging switches to it (spring loading).
            if files.clicked() || (dragging && files.hovered()) {
                self.tab = SidebarTab::Files;
            }
            let servers = icons::icon_button(ui, theme, seg, self.tab == SidebarTab::Servers, |p, c, col| {
                icons::server(p, c, col)
            })
            .on_hover_text("Server");
            if servers.clicked() || (dragging && servers.hovered()) {
                self.tab = SidebarTab::Servers;
            }
            let search = icons::icon_button(ui, theme, seg, self.tab == SidebarTab::Search, |p, c, col| {
                icons::search(p, c, col)
            })
            .on_hover_text("Im Projekt suchen (Strg+Umschalt+F)");
            if search.clicked() {
                self.open_search();
            }
            let changes = self.git.changes.len();
            let git_tip = if changes > 0 { format!("Git ({changes} Änderungen)") } else { "Git".to_string() };
            let git = icons::icon_button(ui, theme, seg, self.tab == SidebarTab::Git, |p, c, col| icons::branch(p, c, col)).on_hover_text(git_tip);
            if changes > 0 {
                // Small dot: there is something to commit.
                ui.painter().circle_filled(git.rect.right_top() + vec2(-7.0, 6.0), 3.0, theme.modified);
            }
            if git.clicked() {
                self.tab = SidebarTab::Git;
            }
            if self.has_extension_sidebars {
                let ext = icons::icon_button(ui, theme, seg, self.tab == SidebarTab::Extensions, |p, c, col| icons::blocks(p, c, col))
                    .on_hover_text("Erweiterungen");
                if ext.clicked() {
                    self.tab = SidebarTab::Extensions;
                }
            } else if self.tab == SidebarTab::Extensions {
                self.tab = SidebarTab::Files;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let small = vec2(26.0, 26.0);
                match self.tab {
                    SidebarTab::Files => {
                        if self.root.is_some() {
                            let resp = icons::icon_button(ui, theme, small, false, |p, c, col| icons::upload(p, c, col))
                                .on_hover_text("Veröffentlichen (geänderte Dateien hochladen)");
                            if resp.clicked() {
                                actions.push(SidebarAction::Publish);
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Synchronisieren…").on_hover_text("In beide Richtungen abgleichen, mit Vorschau").clicked() {
                                    actions.push(SidebarAction::Sync);
                                    ui.close();
                                }
                                if ui.button("Veröffentlichen einrichten…").clicked() {
                                    actions.push(SidebarAction::PublishSetup);
                                    ui.close();
                                }
                            });
                        }
                        if icons::icon_button(ui, theme, small, false, |p, c, col| icons::refresh(p, c, col))
                            .on_hover_text("Aktualisieren")
                            .clicked()
                        {
                            self.refresh_local(None);
                        }
                        if icons::icon_button(ui, theme, small, false, |p, c, col| icons::plus(p, c, col))
                            .on_hover_text("Ordner öffnen…")
                            .clicked()
                        {
                            actions.push(SidebarAction::OpenFolder);
                        }
                    }
                    SidebarTab::Servers => {
                        let more = icons::icon_button(ui, theme, small, false, |p, c, col| icons::more(p, c, col))
                            .on_hover_text("Importieren und exportieren");
                        egui::Popup::menu(&more).show(|ui| server_list_menu(ui, actions));
                        if icons::icon_button(ui, theme, small, false, |p, c, col| icons::plus(p, c, col))
                            .on_hover_text("Server hinzufügen…")
                            .clicked()
                        {
                            actions.push(SidebarAction::EditSite(None));
                        }
                    }
                    SidebarTab::Search | SidebarTab::Git | SidebarTab::Extensions => {}
                }
            });
        }

        // Filter field pinned to the bottom, as in Nova.
        let filter_h = 40.0;
        let list_rect = Rect::from_min_max(pos2(full.left(), header.bottom()), pos2(full.right(), full.bottom() - filter_h));
        self.list_rect = list_rect;
        let filter_rect = Rect::from_min_max(pos2(full.left(), full.bottom() - filter_h), full.max);
        ui.painter().line_segment(
            [filter_rect.left_top(), filter_rect.right_top()],
            Stroke::new(1.0, theme.separator.gamma_multiply(0.6)),
        );

        self.drop_targets.clear();
        let mut servers: Vec<(String, String, String)> = self
            .remote
            .iter()
            .filter_map(|(id, t)| Some((id.clone(), sites.get(id)?.display_name(), t.home.clone()?)))
            .collect();
        servers.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
        let rc = RowCtx {
            theme,
            active_path,
            external_hover: self.external_hover,
            modifiers: ui.input(|i| i.modifiers),
            servers,
        };
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect.shrink2(vec2(8.0, 0.0))));
        if self.tab == SidebarTab::Extensions {
            self.extension_rect = Some(Rect::from_min_max(list_rect.min, full.max));
            ui.advance_cursor_after_rect(full);
            return;
        }
        if self.tab == SidebarTab::Git {
            let mut git_actions = Vec::new();
            self.git_panel.show(&mut list_ui, &mut self.git, theme, &mut git_actions);
            actions.extend(git_actions.into_iter().map(SidebarAction::Git));
            ui.advance_cursor_after_rect(full);
            return;
        }
        if self.tab == SidebarTab::Search {
            let mut search_actions = Vec::new();
            let root = self.root.clone();
            let homes: HashMap<String, String> =
                self.remote.iter().filter_map(|(id, t)| Some((id.clone(), t.home.clone()?))).collect();
            self.search.show(&mut list_ui, theme, root.as_deref(), sites, &homes, &mut search_actions);
            actions.extend(search_actions.into_iter().map(SidebarAction::Search));
            ui.advance_cursor_after_rect(full);
            return;
        }
        let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut list_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            match self.tab {
                SidebarTab::Files => self.show_local(ui, &rc, actions),
                SidebarTab::Servers => self.show_servers(ui, sites, &rc, actions),
                SidebarTab::Search | SidebarTab::Git | SidebarTab::Extensions => {}
            }
            ui.add_space(8.0);
            // Empty space below the tree: click clears selection, drops go to the root.
            let rest = ui.available_rect_before_wrap();
            if rest.height() > 0.0 {
                let resp = ui.allocate_rect(rest, Sense::click());
                if resp.clicked() {
                    self.selection.clear();
                    self.focused = true;
                }
                if let Some(target) = self.default_target() {
                    if let Some(payload) = resp.dnd_hover_payload::<DragItems>() {
                        drop_hint(ui, rc.theme, drop_copies(&payload.0, &target, rc.modifiers));
                    }
                    if let Some(payload) = resp.dnd_release_payload::<DragItems>() {
                        let copy = drop_copies(&payload.0, &target, rc.modifiers);
                        actions.push(SidebarAction::Transfer { items: payload.0.clone(), target, copy });
                    }
                    resp.context_menu(|ui| {
                        if self.clipboard.is_some() && ui.button("Einfügen").clicked() {
                            actions.push(SidebarAction::Paste(self.default_target().unwrap()));
                            ui.close();
                        }
                        if ui.button("Aktualisieren").clicked() {
                            self.refresh_local(None);
                            ui.close();
                        }
                    });
                }
            }
        });
        // Auto-scroll while dragging near the edges.
        if dragging
            && let Some(p) = ui.input(|i| i.pointer.hover_pos())
            && list_rect.contains(p)
        {
            let edge = 28.0;
            let dy = if p.y < list_rect.top() + edge {
                8.0
            } else if p.y > list_rect.bottom() - edge {
                -8.0
            } else {
                0.0
            };
            if dy != 0.0 {
                list_ui.scroll_with_delta(vec2(0.0, dy));
                ui.ctx().request_repaint();
            }
        }
        let _ = scroll;

        let mut filter_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(filter_rect.shrink2(vec2(10.0, 7.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let field = filter_ui.max_rect();
        filter_ui.painter().rect_filled(field, 7.0, theme.hover.gamma_multiply(1.4));
        icons::search(filter_ui.painter(), pos2(field.left() + 14.0, field.center().y), theme.text_faint);
        // The text field fills the box and centers its text vertically.
        let text_rect = Rect::from_min_max(pos2(field.left() + 26.0, field.top()), pos2(field.right() - 8.0, field.bottom()));
        filter_ui.put(
            text_rect,
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filtern")
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .align(egui::Align2::LEFT_CENTER)
                .min_size(text_rect.size())
                .desired_width(text_rect.width()),
        );
        ui.advance_cursor_after_rect(full);
    }

    /// One tree row with selection, drag and drop and its context menu.
    /// Returns the response for click handling by the caller.
    #[allow(clippy::too_many_arguments)]
    fn item_row(
        &mut self,
        ui: &mut Ui,
        rc: &RowCtx,
        item: &Item,
        depth: usize,
        expanded: Option<bool>,
        label: &str,
        url: Option<&str>,
        actions: &mut Vec<SidebarAction>,
    ) -> egui::Response {
        let selected = self.selection.contains(item);
        let path_str = match &item.file {
            FileRef::Local(p) => p.to_string_lossy().into_owned(),
            FileRef::Remote { path, .. } => path.clone(),
        };
        let active = rc.active_path == Some(path_str.as_str());
        // Folders accept drops into themselves, files into their parent.
        let target = if item.is_dir {
            match &item.file {
                FileRef::Local(p) => Some(DirRef::Local(p.clone())),
                FileRef::Remote { site_id, path } => Some(DirRef::Remote { site_id: site_id.clone(), path: path.clone() }),
            }
        } else {
            item.parent()
        };

        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
        if let Some(t) = &target {
            self.drop_targets.push((rect, t.clone()));
        }

        // Drag source.
        if resp.drag_started() {
            let items = if selected { self.selection.clone() } else { vec![item.clone()] };
            resp.dnd_set_drag_payload(DragItems(items));
        }
        // Drop target highlighting.
        let internal_hover = resp.dnd_hover_payload::<DragItems>().is_some();
        let external_hover = rc.external_hover.is_some_and(|p| rect.contains(p));
        if (internal_hover || external_hover) && item.is_dir {
            // Spring-loaded folders open after a short hover.
            let key = path_str.clone();
            match &self.spring {
                Some((k, t)) if *k == key => {
                    if expanded == Some(false) && t.elapsed().as_millis() > 700 {
                        self.expand(item);
                    }
                    ui.ctx().request_repaint();
                }
                _ => self.spring = Some((key, Instant::now())),
            }
        }
        if let Some(payload) = resp.dnd_hover_payload::<DragItems>()
            && let Some(t) = &target
        {
            drop_hint(ui, rc.theme, drop_copies(&payload.0, t, rc.modifiers));
        }
        if let Some(payload) = resp.dnd_release_payload::<DragItems>()
            && let Some(t) = target.clone()
        {
            let copy = drop_copies(&payload.0, &t, rc.modifiers);
            actions.push(SidebarAction::Transfer { items: payload.0.clone(), target: t, copy });
        }

        let painter = ui.painter();
        let theme = rc.theme;
        if (internal_hover || external_hover) && item.is_dir {
            painter.rect_filled(rect, 6.0, theme.accent.gamma_multiply(0.25));
            painter.rect_stroke(rect, 6.0, Stroke::new(1.5, theme.accent), egui::StrokeKind::Inside);
        } else if selected {
            painter.rect_filled(rect, 6.0, theme.row_selected);
        } else if active {
            painter.rect_filled(rect, 6.0, theme.row_selected.gamma_multiply(0.6));
        } else if resp.hovered() {
            painter.rect_filled(rect, 6.0, theme.hover);
        }
        let cut = self
            .clipboard
            .as_ref()
            .is_some_and(|(items, cut, _)| *cut && items.contains(item));
        let x = rect.left() + 8.0 + depth as f32 * INDENT;
        let cy = rect.center().y;
        match expanded {
            Some(open) => {
                icons::chevron(painter, pos2(x + 3.0, cy), open, theme.text_faint);
                icons::folder(painter, pos2(x + 20.0, cy), folder_color(theme), open);
            }
            None => icons::file(painter, pos2(x + 20.0, cy), label),
        }
        let mut text_color = if selected && theme.dark {
            Color32::WHITE
        } else {
            theme.foreground.gamma_multiply(0.92)
        };
        if cut {
            text_color = text_color.gamma_multiply(0.45);
        }
        let galley = painter.layout_no_wrap(label.to_string(), FontId::proportional(13.0), text_color);
        let text_pos = pos2(x + 33.0, cy - galley.size().y / 2.0);
        painter.with_clip_rect(rect.shrink2(vec2(4.0, 0.0))).galley(text_pos, galley, text_color);
        if let FileRef::Local(p) = &item.file {
            if let Some(state) = self.git.file(p) {
                painter.text(
                    pos2(rect.right() - 10.0, cy),
                    Align2::RIGHT_CENTER,
                    state.letter(),
                    FontId::new(11.0, bold_family()),
                    state.color(theme),
                );
            } else if item.is_dir && self.git.dir_changed(p) {
                painter.circle_filled(pos2(rect.right() - 13.0, cy), 3.0, theme.modified.gamma_multiply(0.8));
            }
        }

        // Selection on click (Ctrl toggles).
        if resp.clicked() || resp.secondary_clicked() {
            self.focused = true;
            // Take keyboard focus away from the editor so Ctrl+C etc. act on files.
            ui.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            if rc.modifiers.command && resp.clicked() {
                if let Some(i) = self.selection.iter().position(|s| s == item) {
                    self.selection.remove(i);
                } else {
                    self.selection.push(item.clone());
                }
            } else if !selected || resp.clicked() {
                self.selection = vec![item.clone()];
            }
        }

        let clipboard_ready = self.clipboard.is_some();
        let selection = if selected { self.selection.clone() } else { vec![item.clone()] };
        resp.context_menu(|ui| {
            item_menu(ui, item, &selection, target.as_ref(), clipboard_ready, url, &rc.servers, actions);
        });
        resp
    }

    fn expand(&mut self, item: &Item) {
        match &item.file {
            FileRef::Local(p) => {
                self.local_expanded.insert(p.clone());
            }
            FileRef::Remote { site_id, path } => {
                if let Some(t) = self.remote.get_mut(site_id) {
                    t.expanded.insert(path.clone());
                }
            }
        }
    }

    fn show_local(&mut self, ui: &mut Ui, rc: &RowCtx, actions: &mut Vec<SidebarAction>) {
        let Some(root) = self.root.clone() else {
            empty_state(ui, rc.theme, "Kein Ordner geöffnet", "Ordner öffnen…", || {
                actions.push(SidebarAction::OpenFolder)
            });
            return;
        };
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let resp = section_header(ui, rc.theme, &format!("{name} ▾"));
        if let Some(branch) = &self.git.branch {
            ui.painter().text(
                pos2(resp.rect.right() - 8.0, resp.rect.center().y),
                Align2::RIGHT_CENTER,
                format!("⎇ {branch}"),
                FontId::proportional(11.0),
                rc.theme.text_faint,
            );
        }
        let target = DirRef::Local(root.clone());
        self.drop_targets.push((resp.rect, target.clone()));
        if let Some(payload) = resp.dnd_hover_payload::<DragItems>() {
            drop_hint(ui, rc.theme, drop_copies(&payload.0, &target, rc.modifiers));
        }
        if let Some(payload) = resp.dnd_release_payload::<DragItems>() {
            let copy = drop_copies(&payload.0, &target, rc.modifiers);
            actions.push(SidebarAction::Transfer { items: payload.0.clone(), target: target.clone(), copy });
        }
        let clipboard_ready = self.clipboard.is_some();
        resp.context_menu(|ui| dir_menu(ui, &target, clipboard_ready, actions));
        let resp = resp.on_hover_text("Projekt wechseln");
        // Own id: the context menu above already uses the default popup id of this response.
        egui::Popup::menu(&resp).id(resp.id.with("projects")).show(|ui| {
            ui.set_min_width(240.0);
            for p in self.recent_folders.iter().filter(|p| **p != root) {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
                let chars: Vec<char> = dir.chars().collect();
                let dir = if chars.len() > 40 { format!("…{}", chars[chars.len() - 39..].iter().collect::<String>()) } else { dir };
                let mut job = egui::text::LayoutJob::simple_singleline(name, FontId::proportional(13.0), ui.visuals().text_color());
                job.append(&format!("   {dir}"), 0.0, egui::TextFormat::simple(FontId::proportional(11.0), rc.theme.text_faint));
                if ui.button(job).on_hover_text(p.display().to_string()).clicked() {
                    actions.push(SidebarAction::SwitchFolder(p.clone()));
                }
            }
            if self.recent_folders.len() > 1 {
                ui.separator();
            }
            if ui.button("Ordner öffnen…").clicked() {
                actions.push(SidebarAction::OpenFolder);
            }
        });
        let filter = self.filter.trim().to_lowercase();
        let matches = self.local_filter.update(ui.ctx(), &root, &filter);
        if !filter.is_empty() {
            let note = match &matches {
                _ if self.local_filter.busy() => Some("Suche…"),
                Some(m) if m.files.is_empty() && m.dirs.is_empty() => Some("Keine Treffer"),
                Some(m) if m.truncated => Some("Nur die ersten Treffer – Filter genauer fassen"),
                _ => None,
            };
            if let Some(note) = note {
                indent_label(ui, 0, note, rc.theme.text_faint);
            }
        }
        match matches {
            Some(m) => self.show_filtered_dir(ui, rc, &root, 0, &m, actions),
            None if filter.is_empty() => self.show_local_dir(ui, rc, &root, 0, actions),
            None => {}
        }
    }

    /// The tree reduced to filter matches (all folders with matches open).
    fn show_filtered_dir(&mut self, ui: &mut Ui, rc: &RowCtx, dir: &Path, depth: usize, m: &super::file_filter::Matches, actions: &mut Vec<SidebarAction>) {
        for entry in self.local_children(dir) {
            let item = Item::local(entry.path.clone(), entry.is_dir);
            if entry.is_dir {
                if !m.dirs.contains(&entry.path) {
                    continue;
                }
                let resp = self.item_row(ui, rc, &item, depth, Some(true), &entry.name, None, actions);
                if resp.clicked() && !rc.modifiers.command {
                    self.local_expanded.insert(entry.path.clone());
                }
                self.show_filtered_dir(ui, rc, &entry.path, depth + 1, m, actions);
            } else if m.files.contains(&entry.path) {
                let resp = self.item_row(ui, rc, &item, depth, None, &entry.name, None, actions);
                if resp.clicked() && !rc.modifiers.command {
                    actions.push(SidebarAction::OpenLocal(entry.path.clone()));
                }
            }
        }
    }

    fn show_local_dir(&mut self, ui: &mut Ui, rc: &RowCtx, dir: &Path, depth: usize, actions: &mut Vec<SidebarAction>) {
        for entry in self.local_children(dir) {
            let item = Item::local(entry.path.clone(), entry.is_dir);
            if entry.is_dir {
                let open = self.local_expanded.contains(&entry.path);
                let resp = self.item_row(ui, rc, &item, depth, Some(open), &entry.name, None, actions);
                if resp.clicked() && !rc.modifiers.command {
                    if !self.local_expanded.insert(entry.path.clone()) {
                        self.local_expanded.remove(&entry.path);
                    }
                }
                if open {
                    self.show_local_dir(ui, rc, &entry.path, depth + 1, actions);
                }
            } else {
                let resp = self.item_row(ui, rc, &item, depth, None, &entry.name, None, actions);
                if resp.clicked() && !rc.modifiers.command {
                    actions.push(SidebarAction::OpenLocal(entry.path.clone()));
                }
            }
        }
    }

    fn show_servers(&mut self, ui: &mut Ui, sites: &Sites, rc: &RowCtx, actions: &mut Vec<SidebarAction>) {
        let theme = rc.theme;
        if sites.sites.is_empty() {
            ui.add_space(36.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Noch keine Server").color(theme.text_faint));
                ui.add_space(10.0);
                if ui.button("Server hinzufügen…").clicked() {
                    actions.push(SidebarAction::EditSite(None));
                }
                if crate::server_import::filezilla_path().is_some() && ui.button("Aus FileZilla importieren…").clicked() {
                    actions.push(SidebarAction::ImportFileZilla);
                }
                if ui.link("Aus Datei importieren…").clicked() {
                    actions.push(SidebarAction::ImportSites);
                }
            });
            return;
        }
        let groups = sites.groups();
        let filter = self.filter.to_lowercase();
        let ungrouped: Vec<&Site> = sites.sites.iter().filter(|s| s.group.is_empty()).collect();
        if !ungrouped.is_empty() {
            section_header(ui, theme, "Server").context_menu(|ui| server_list_menu(ui, actions));
            for site in ungrouped {
                self.show_site(ui, site, rc, &groups, &filter, actions);
            }
        }
        for group in &groups {
            let open = !sites.collapsed_groups.contains(group);
            let members: Vec<&Site> = sites.sites.iter().filter(|s| &s.group == group).collect();
            let resp = group_header(ui, theme, group, open, members.len());
            if resp.clicked() {
                actions.push(SidebarAction::ToggleGroup(group.clone()));
            }
            resp.context_menu(|ui| {
                if ui.button("Server hinzufügen…").clicked() {
                    actions.push(SidebarAction::AddSiteToGroup(group.clone()));
                    ui.close();
                }
                if ui.button("Umbenennen…").clicked() {
                    actions.push(SidebarAction::AskName {
                        title: "Gruppe umbenennen".into(),
                        initial: group.clone(),
                        action: NameAction::RenameGroup { old: group.clone() },
                    });
                    ui.close();
                }
                if ui.button("Gruppe auflösen").on_hover_text("Die Server bleiben erhalten").clicked() {
                    actions.push(SidebarAction::DissolveGroup(group.clone()));
                    ui.close();
                }
            });
            if open {
                for site in members {
                    self.show_site(ui, site, rc, &groups, &filter, actions);
                }
            }
        }
    }

    fn show_site(&mut self, ui: &mut Ui, site: &Site, rc: &RowCtx, groups: &[String], filter: &str, actions: &mut Vec<SidebarAction>) {
        let theme = rc.theme;
        let open = self.expanded_sites.contains(&site.id);
        let tree = self.remote.get(&site.id);
        let status_color = match tree.map(|t| &t.status) {
            Some(RemoteStatus::Connected) => theme.success,
            Some(RemoteStatus::Connecting) => theme.modified,
            Some(RemoteStatus::Failed(_)) => theme.danger,
            None => theme.gutter_fg,
        };
        let resp = site_row(ui, theme, open, &site.display_name(), site.protocol.label(), status_color);
        // Dropping onto a server uploads into its start folder.
        if let Some(home) = tree.and_then(|t| t.home.clone()) {
            let target = DirRef::Remote { site_id: site.id.clone(), path: home };
            self.drop_targets.push((resp.rect, target.clone()));
            if let Some(payload) = resp.dnd_hover_payload::<DragItems>() {
                drop_hint(ui, theme, drop_copies(&payload.0, &target, rc.modifiers));
            }
            if let Some(payload) = resp.dnd_release_payload::<DragItems>() {
                let copy = drop_copies(&payload.0, &target, rc.modifiers);
                actions.push(SidebarAction::Transfer { items: payload.0.clone(), target, copy });
            }
        }
        if resp.clicked() {
            if open {
                self.expanded_sites.remove(&site.id);
            } else {
                self.expanded_sites.insert(site.id.clone());
                actions.push(SidebarAction::Connect(site.id.clone()));
            }
        }
        let connected = tree.is_some();
        let home = tree.and_then(|t| t.home.clone());
        resp.context_menu(|ui| {
            if connected {
                if ui.button("Trennen").clicked() {
                    actions.push(SidebarAction::Disconnect(site.id.clone()));
                    ui.close();
                }
                if let Some(home) = &home
                    && ui.button("Aktualisieren").clicked()
                {
                    actions.push(SidebarAction::Remote { site_id: site.id.clone(), request: Request::List(home.clone()) });
                    ui.close();
                }
            } else if ui.button("Verbinden").clicked() {
                self.expanded_sites.insert(site.id.clone());
                actions.push(SidebarAction::Connect(site.id.clone()));
                ui.close();
            }
            if ui.button("SSH-Terminal öffnen").clicked() {
                actions.push(SidebarAction::OpenTerminal(site.id.clone()));
                ui.close();
            }
            if !site.commands.is_empty() {
                ui.menu_button("Befehl ausführen", |ui| {
                    for c in &site.commands {
                        if ui.button(&c.name).on_hover_text(&c.command).clicked() {
                            actions.push(SidebarAction::RunCommand { site_id: site.id.clone(), command: c.command.clone() });
                            ui.close();
                        }
                    }
                });
            }
            if ui.button("Auf dem Server suchen…").clicked() {
                let path = home.clone().unwrap_or_else(|| site.root_path.trim().to_string());
                actions.push(SidebarAction::SearchIn(DirRef::Remote { site_id: site.id.clone(), path }));
                ui.close();
            }
            ui.separator();
            if ui.button("Bearbeiten…").clicked() {
                actions.push(SidebarAction::EditSite(Some(site.id.clone())));
                ui.close();
            }
            if ui.button("Duplizieren…").clicked() {
                actions.push(SidebarAction::DuplicateSite(site.id.clone()));
                ui.close();
            }
            ui.menu_button("Gruppe", |ui| {
                let set = |group: &str| SidebarAction::SetSiteGroup { site_id: site.id.clone(), group: group.to_string() };
                if menu_radio(ui, site.group.is_empty(), "Ohne Gruppe") {
                    actions.push(set(""));
                    ui.close();
                }
                for g in groups {
                    if menu_radio(ui, &site.group == g, g) {
                        actions.push(set(g));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Neue Gruppe…").clicked() {
                    actions.push(SidebarAction::AskName {
                        title: "Neue Gruppe".into(),
                        initial: String::new(),
                        action: NameAction::NewGroup { site_id: site.id.clone() },
                    });
                    ui.close();
                }
            });
            if ui.button("Entfernen…").clicked() {
                actions.push(SidebarAction::RemoveSite(site.id.clone()));
                ui.close();
            }
        });
        if !open {
            return;
        }
        let Some(tree) = self.remote.get(&site.id) else { return };
        match &tree.status {
            RemoteStatus::Connecting if tree.home.is_none() => {
                indent_label(ui, 1, "Verbinde…", theme.gutter_fg);
            }
            RemoteStatus::Failed(msg) => {
                let msg = msg.clone();
                indent_label(ui, 1, &msg, theme.danger);
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    if ui.small_button("Erneut versuchen").clicked() {
                        actions.push(SidebarAction::Connect(site.id.clone()));
                    }
                });
            }
            _ => {}
        }
        if let Some(home) = tree.home.clone() {
            let web_url = site.web_url.trim().trim_end_matches('/').to_string();
            let root = Item::remote(&site.id, &home, true);
            self.item_row(ui, rc, &root, 1, Some(true), &home, None, actions);
            self.show_remote_dir(ui, rc, &site.id, &home, &home, &web_url, 2, filter, actions);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_remote_dir(
        &mut self,
        ui: &mut Ui,
        rc: &RowCtx,
        site_id: &str,
        dir: &str,
        home: &str,
        web_url: &str,
        depth: usize,
        filter: &str,
        actions: &mut Vec<SidebarAction>,
    ) {
        let Some(tree) = self.remote.get_mut(site_id) else { return };
        let Some(entries) = tree.dirs.get(dir).cloned() else {
            if !tree.loading.contains(dir) {
                tree.loading.insert(dir.to_string());
                actions.push(SidebarAction::Remote { site_id: site_id.to_string(), request: Request::List(dir.to_string()) });
            }
            indent_label(ui, depth, "Lade…", rc.theme.gutter_fg);
            return;
        };
        if entries.is_empty() {
            indent_label(ui, depth, "(leer)", rc.theme.gutter_fg);
        }
        for entry in entries {
            if !filter.is_empty() && !entry.is_dir && !entry.name.to_lowercase().contains(filter) {
                continue;
            }
            let item = Item::remote(site_id, &entry.path, entry.is_dir);
            if entry.is_dir {
                let open = self.remote.get(site_id).is_some_and(|t| t.expanded.contains(&entry.path));
                let resp = self.item_row(ui, rc, &item, depth, Some(open), &entry.name, None, actions);
                if resp.clicked() && !rc.modifiers.command
                    && let Some(tree) = self.remote.get_mut(site_id)
                {
                    if open {
                        tree.expanded.remove(&entry.path);
                    } else {
                        tree.expanded.insert(entry.path.clone());
                    }
                }
                if open {
                    self.show_remote_dir(ui, rc, site_id, &entry.path, home, web_url, depth + 1, filter, actions);
                }
            } else {
                let url = (!web_url.is_empty())
                    .then(|| entry.path.strip_prefix(home).map(|rel| format!("{web_url}{rel}")))
                    .flatten();
                let resp = self
                    .item_row(ui, rc, &item, depth, None, &entry.name, url.as_deref(), actions)
                    .on_hover_text(match entry.mode {
                        Some(mode) => format!("{}\n{}\n{}", entry.path, format_size(entry.size), crate::remote::mode_string(mode)),
                        None => format!("{}\n{}", entry.path, format_size(entry.size)),
                    });
                if resp.clicked() && !rc.modifiers.command {
                    actions.push(SidebarAction::OpenRemote { site_id: site_id.to_string(), path: entry.path.clone() });
                }
            }
        }
    }
}

fn rename_action(item: &Item) -> SidebarAction {
    let initial = item.name();
    let action = match &item.file {
        FileRef::Local(p) => NameAction::RenameLocal { path: p.clone() },
        FileRef::Remote { site_id, path } => NameAction::RenameRemote { site_id: site_id.clone(), path: path.clone() },
    };
    SidebarAction::AskName { title: "Umbenennen".into(), initial, action }
}

fn menu_button(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked()
}

/// Context menu for a file or folder row.
fn item_menu(
    ui: &mut Ui,
    item: &Item,
    selection: &[Item],
    target: Option<&DirRef>,
    clipboard_ready: bool,
    url: Option<&str>,
    servers: &[(String, String, String)],
    actions: &mut Vec<SidebarAction>,
) {
    let mut close = false;
    if let Some(url) = url
        && menu_button(ui, "Im Browser öffnen", "")
    {
        actions.push(SidebarAction::OpenUrl(url.to_string()));
        close = true;
    }
    if item.is_dir {
        if let Some(dir) = target {
            new_entries(ui, dir, actions, &mut close);
            if menu_button(ui, "In diesem Ordner suchen…", "") {
                actions.push(SidebarAction::SearchIn(dir.clone()));
                close = true;
            }
            ui.separator();
        }
    } else {
        if menu_button(ui, "Öffnen", "") {
            actions.push(match &item.file {
                FileRef::Local(p) => SidebarAction::OpenLocal(p.clone()),
                FileRef::Remote { site_id, path } => SidebarAction::OpenRemote { site_id: site_id.clone(), path: path.clone() },
            });
            close = true;
        }
        // Two selected files compare with each other, one with the active tab.
        let files: Vec<&Item> = selection.iter().filter(|i| !i.is_dir).collect();
        if files.len() == 2 {
            if menu_button(ui, "Miteinander vergleichen", "") {
                actions.push(SidebarAction::Compare(files.iter().map(|i| i.file.clone()).collect()));
                close = true;
            }
        } else if menu_button(ui, "Mit aktivem Tab vergleichen", "") {
            actions.push(SidebarAction::Compare(vec![item.file.clone()]));
            close = true;
        }
        let label = if files.len() > 1 { format!("{} Dateien an KI-Agent anhängen", files.len()) } else { "An KI-Agent anhängen".to_string() };
        if menu_button(ui, &label, "") {
            let files = if files.len() > 1 { files.iter().map(|i| i.file.clone()).collect() } else { vec![item.file.clone()] };
            actions.push(SidebarAction::AttachToAgent(files));
            close = true;
        }
    }
    if menu_button(ui, "Ausschneiden", "Strg+X") {
        actions.push(SidebarAction::Clipboard { items: selection.to_vec(), cut: true });
        close = true;
    }
    if menu_button(ui, "Kopieren", "Strg+C") {
        actions.push(SidebarAction::Clipboard { items: selection.to_vec(), cut: false });
        close = true;
    }
    if let Some(t) = target
        && ui.add_enabled(clipboard_ready, egui::Button::new("Einfügen").shortcut_text("Strg+V")).clicked()
    {
        actions.push(SidebarAction::Paste(t.clone()));
        close = true;
    }
    if menu_button(ui, "Duplizieren", "Strg+D") {
        actions.push(SidebarAction::Duplicate(selection.to_vec()));
        close = true;
    }
    // To the start folder of another connected server (or the same one from local).
    let source_site = match &item.file {
        FileRef::Remote { site_id, .. } => Some(site_id.as_str()),
        FileRef::Local(_) => None,
    };
    let others: Vec<&(String, String, String)> = servers.iter().filter(|(id, _, _)| Some(id.as_str()) != source_site).collect();
    if !others.is_empty() {
        for (label, copy) in [("Auf Server kopieren", true), ("Auf Server verschieben", false)] {
            ui.menu_button(label, |ui| {
                for (id, name, home) in &others {
                    if ui.button(name).on_hover_text(format!("In den Startordner {home}")).clicked() {
                        let target = DirRef::Remote { site_id: id.clone(), path: home.clone() };
                        actions.push(SidebarAction::Transfer { items: selection.to_vec(), target, copy });
                        ui.close();
                    }
                }
            });
        }
    }
    ui.separator();
    if menu_button(ui, "Umbenennen…", "F2") {
        actions.push(rename_action(item));
        close = true;
    }
    match &item.file {
        FileRef::Local(p) => {
            if menu_button(ui, "Pfad kopieren", "") {
                actions.push(SidebarAction::CopyText(p.display().to_string()));
                close = true;
            }
            if menu_button(ui, "Im Explorer zeigen", "") {
                let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
                close = true;
            }
        }
        FileRef::Remote { site_id, path } => {
            if menu_button(ui, "Pfad kopieren", "") {
                actions.push(SidebarAction::CopyText(path.clone()));
                close = true;
            }
            if item.is_dir && menu_button(ui, "Dateien hochladen…", "") {
                actions.push(SidebarAction::UploadFiles { site_id: site_id.clone(), dir: path.clone() });
                close = true;
            }
            if menu_button(ui, "Herunterladen nach…", "") {
                actions.push(SidebarAction::DownloadTo(item.clone()));
                close = true;
            }
            if menu_button(ui, "Berechtigungen…", "") {
                actions.push(SidebarAction::Permissions(selection.to_vec()));
                close = true;
            }
            if item.is_dir && menu_button(ui, "Aktualisieren", "") {
                actions.push(SidebarAction::Remote { site_id: site_id.clone(), request: Request::List(path.clone()) });
                close = true;
            }
        }
    }
    ui.separator();
    let label = if selection.len() > 1 { format!("{} Objekte löschen…", selection.len()) } else { "Löschen…".into() };
    if menu_button(ui, &label, "Entf") {
        actions.push(SidebarAction::ConfirmDelete(selection.to_vec()));
        close = true;
    }
    if close {
        ui.close();
    }
}

fn new_entries(ui: &mut Ui, dir: &DirRef, actions: &mut Vec<SidebarAction>, close: &mut bool) {
    if menu_button(ui, "Neue Datei…", "") {
        actions.push(SidebarAction::AskName {
            title: "Neue Datei".into(),
            initial: "neu.html".into(),
            action: match dir {
                DirRef::Local(d) => NameAction::NewLocalFile { dir: d.clone() },
                DirRef::Remote { site_id, path } => NameAction::NewRemoteFile { site_id: site_id.clone(), dir: path.clone() },
            },
        });
        *close = true;
    }
    if menu_button(ui, "Neuer Ordner…", "") {
        actions.push(SidebarAction::AskName {
            title: "Neuer Ordner".into(),
            initial: "neuer-ordner".into(),
            action: match dir {
                DirRef::Local(d) => NameAction::NewLocalFolder { dir: d.clone() },
                DirRef::Remote { site_id, path } => NameAction::NewRemoteFolder { site_id: site_id.clone(), dir: path.clone() },
            },
        });
        *close = true;
    }
}

/// Context menu for a folder header (project root).
fn dir_menu(ui: &mut Ui, dir: &DirRef, clipboard_ready: bool, actions: &mut Vec<SidebarAction>) {
    let mut close = false;
    new_entries(ui, dir, actions, &mut close);
    if ui.add_enabled(clipboard_ready, egui::Button::new("Einfügen").shortcut_text("Strg+V")).clicked() {
        actions.push(SidebarAction::Paste(dir.clone()));
        close = true;
    }
    if let DirRef::Local(d) = dir
        && menu_button(ui, "Im Explorer öffnen", "")
    {
        let _ = std::process::Command::new("explorer").arg(d).spawn();
        close = true;
    }
    if close {
        ui.close();
    }
}

fn format_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / 1024.0 / 1024.0),
    }
}

/// Height of the mode switcher row: 10 pt margin above the buttons, like at the sides.
const HEADER_H: f32 = 40.0;
const ROW_H: f32 = 24.0;
const INDENT: f32 = 14.0;

fn folder_color(theme: &Theme) -> Color32 {
    if theme.dark {
        Color32::from_rgb(98, 160, 236)
    } else {
        Color32::from_rgb(76, 140, 222)
    }
}

fn site_row(ui: &mut Ui, theme: &Theme, open: bool, name: &str, proto: &str, status: Color32) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H + 4.0), Sense::click());
    let painter = ui.painter();
    if resp.dnd_hover_payload::<DragItems>().is_some() {
        painter.rect_stroke(rect, 6.0, Stroke::new(1.5, theme.accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        painter.rect_filled(rect, 6.0, theme.hover);
    }
    let cy = rect.center().y;
    let x = rect.left() + 8.0;
    icons::chevron(painter, pos2(x + 3.0, cy), open, theme.text_faint);
    icons::server(painter, pos2(x + 20.0, cy), theme.text_muted);
    painter.circle_filled(pos2(x + 26.5, cy + 4.5), 3.6, theme.sidebar_bg);
    painter.circle_filled(pos2(x + 26.5, cy + 4.5), 2.6, status);
    painter.text(pos2(x + 34.0, cy), Align2::LEFT_CENTER, name, FontId::new(13.0, bold_family()), theme.foreground);
    let label = proto.split(' ').next().unwrap_or(proto);
    let galley = painter.layout_no_wrap(label.to_string(), FontId::new(9.5, bold_family()), theme.text_muted);
    let badge = Rect::from_center_size(
        pos2(rect.right() - 8.0 - galley.size().x / 2.0 - 5.0, cy),
        vec2(galley.size().x + 10.0, 16.0),
    );
    painter.rect_filled(badge, 4.0, theme.hover.gamma_multiply(1.6));
    painter.galley(badge.center() - galley.size() / 2.0, galley, theme.text_muted);
    resp
}

/// Small caps section title, like Nova's sidebar headers.
fn section_header(ui: &mut Ui, theme: &Theme, title: &str) -> egui::Response {
    ui.add_space(6.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if resp.dnd_hover_payload::<DragItems>().is_some() {
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.5, theme.accent), egui::StrokeKind::Inside);
    }
    ui.painter().text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        FontId::new(10.5, bold_family()),
        theme.text_faint,
    );
    resp
}

/// Foldable header of a server group.
fn group_header(ui: &mut Ui, theme: &Theme, title: &str, open: bool, count: usize) -> egui::Response {
    ui.add_space(6.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, 6.0, theme.hover);
    }
    let cy = rect.center().y;
    icons::chevron(painter, pos2(rect.left() + 11.0, cy), open, theme.text_faint);
    let galley = painter.layout_no_wrap(title.to_uppercase(), FontId::new(10.5, bold_family()), theme.text_faint);
    let width = galley.size().x;
    painter.galley(pos2(rect.left() + 22.0, cy - galley.size().y / 2.0), galley, theme.text_faint);
    painter.text(
        pos2(rect.left() + 28.0 + width, cy),
        Align2::LEFT_CENTER,
        count.to_string(),
        FontId::new(10.0, bold_family()),
        theme.gutter_fg,
    );
    resp
}

/// Radio button in a menu, with a frame so the empty circle stays visible.
fn menu_radio(ui: &mut Ui, selected: bool, label: &str) -> bool {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        let border = w.noninteractive.fg_stroke.color.gamma_multiply(0.6);
        w.inactive.bg_stroke = Stroke::new(1.0, border);
        w.hovered.bg_stroke = Stroke::new(1.0, border);
        ui.radio(selected, label).clicked()
    })
    .inner
}

/// Import and export entries for the server list.
fn server_list_menu(ui: &mut Ui, actions: &mut Vec<SidebarAction>) {
    if crate::server_import::filezilla_path().is_some() && ui.button("Aus FileZilla importieren…").clicked() {
        actions.push(SidebarAction::ImportFileZilla);
        ui.close();
    }
    if ui.button("Aus Datei importieren…").on_hover_text("Nebula-Export (.json) oder FileZilla (sitemanager.xml)").clicked() {
        actions.push(SidebarAction::ImportSites);
        ui.close();
    }
    if ui.button("Exportieren…").on_hover_text("Ohne Passwörter").clicked() {
        actions.push(SidebarAction::ExportSites);
        ui.close();
    }
}

fn empty_state(ui: &mut Ui, theme: &Theme, text: &str, button: &str, on_click: impl FnOnce()) {
    ui.add_space(36.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).color(theme.text_faint));
        ui.add_space(10.0);
        if ui.button(button).clicked() {
            on_click();
        }
    });
}

fn indent_label(ui: &mut Ui, depth: usize, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.add_space(8.0 + depth as f32 * INDENT + 33.0);
        ui.add(egui::Label::new(RichText::new(text).color(color).small()).wrap());
    });
}

/// Canonical form of a local path (no `\?` prefix, consistent separators).
/// Mapped network drives keep their letter instead of becoming `\\server\share`.
pub fn normalize_path(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let c = match p.canonicalize() {
        Ok(c) if c.to_string_lossy().starts_with(r"\\?\UNC\") && !abs.to_string_lossy().starts_with(r"\\") => abs,
        Ok(c) => c,
        Err(_) => abs,
    };
    strip_verbatim(&c)
}

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\server\x` → `\\server\x`.
pub fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => PathBuf::from(format!(r"\\{rest}")),
        None => PathBuf::from(s.trim_start_matches(r"\\?\")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_copies_between_places_and_moves_within() {
        let none = Modifiers::NONE;
        let a = [Item::remote("a", "/www/x.css", false)];
        let to_a = DirRef::Remote { site_id: "a".into(), path: "/www/css".into() };
        let to_b = DirRef::Remote { site_id: "b".into(), path: "/".into() };
        let local = DirRef::Local(PathBuf::from(r"C:\proj"));
        assert!(!drop_copies(&a, &to_a, none));
        assert!(drop_copies(&a, &to_b, none));
        assert!(drop_copies(&a, &local, none));
        assert!(!drop_copies(&a, &to_b, Modifiers::SHIFT));
        assert!(drop_copies(&a, &to_a, Modifiers::COMMAND));
        let l = [Item::local(PathBuf::from(r"C:\proj\a.txt"), false)];
        assert!(!drop_copies(&l, &local, none));
        assert!(drop_copies(&l, &to_a, none));
    }
}
