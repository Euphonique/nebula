//! Nova extensions inside the app: keeps the extension host's view of the
//! open documents current and carries out what extensions ask for (edits,
//! prompts, notifications, problems, completions, hover).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Key, RichText, Sense, vec2};
use serde_json::{Value, json};

use super::{NebulaApp, Toast, ToastKind};
use crate::editor::document::{Change, Diagnostic, LineEnding, Selection};
use crate::editor::{DocSource, Document, complete, snippet};
use crate::ext::protocol::{CompletionMsg, IssueMsg};
use crate::ext::{DocInfo, ExtHost, Extension, FromHost, HostEvent, ToHost, manifest};

pub struct LogLine {
    pub ext: String,
    pub level: String,
    pub text: String,
}

#[derive(Clone, PartialEq)]
pub enum ExtStatus {
    Active,
    Failed(String),
}

pub enum Prompt {
    Action { ext: String, cb: Option<u64>, message: String, buttons: Vec<String> },
    Input { ext: String, cb: Option<u64>, message: String, label: String, placeholder: String, value: String, prompt: String, secure: bool, focus: bool },
    Palette { ext: String, cb: Option<u64>, message: String, placeholder: String, value: String, focus: bool },
    Choice { ext: String, cb: Option<u64>, choices: Vec<String>, placeholder: String, query: String, selected: usize, focus: bool },
}

pub struct Notice {
    pub ext: String,
    pub id: String,
    pub cb: Option<u64>,
    pub title: String,
    pub body: String,
    pub input: Option<(String, String, bool)>,
    pub actions: Vec<String>,
}

/// A command contributed by an extension, as shown in menus and the palette.
#[derive(Clone)]
pub struct ExtCommand {
    pub ext: String,
    pub ext_name: String,
    pub command: String,
    pub title: String,
    pub section: String,
    pub syntaxes: Vec<String>,
    pub shortcut: Option<egui::KeyboardShortcut>,
}

#[derive(Default)]
pub struct TreeState {
    pub alive: bool,
    /// Children per element (`None` = root); `None` value = loading.
    pub children: HashMap<Option<u64>, Option<Vec<crate::ext::protocol::TreeItemMsg>>>,
    pub expanded: HashSet<u64>,
    pub selected: Option<u64>,
}

#[derive(Default)]
struct Synced {
    version: u64,
    sels: Vec<(usize, usize)>,
    meta: String,
    dirty: bool,
}

pub struct ExtUi {
    pub host: ExtHost,
    pub installed: Vec<Extension>,
    pub load_errors: Vec<(PathBuf, String)>,
    pub status: HashMap<String, ExtStatus>,
    pub log: VecDeque<LogLine>,
    pub commands: Vec<ExtCommand>,
    pub prompts: VecDeque<Prompt>,
    pub notices: Vec<Notice>,
    pub trees: HashMap<(String, String), TreeState>,
    /// Problems: collection -> (extension, uri key -> issues).
    issues: HashMap<u64, (String, HashMap<String, Vec<IssueMsg>>)>,
    issues_gen: u64,
    applied: HashMap<u64, (u64, u64)>,
    synced: HashMap<u64, Synced>,
    workspace: Option<Option<String>>,
    active: Option<Option<u64>>,
    next_req: u64,
    completion: Option<(u64, u64, usize)>,
    hover: Option<(u64, u64, usize)>,
    /// Definition request: (req, doc, position); without an answer the built-in search runs.
    definition: Option<(u64, u64, usize)>,
    /// Saves waiting for extensions to finish `onWillSave`: req -> (doc, close after).
    will_save: HashMap<u64, (u64, bool)>,
    replies: Vec<(String, u64, Value)>,
    tree_reqs: HashMap<u64, (String, String, Option<u64>)>,
}

impl ExtUi {
    pub fn new(ctx: &egui::Context, disabled: &[String]) -> Self {
        let mut ui = Self {
            host: ExtHost::spawn(ctx.clone()),
            installed: Vec::new(),
            load_errors: Vec::new(),
            status: HashMap::new(),
            log: VecDeque::new(),
            commands: Vec::new(),
            prompts: VecDeque::new(),
            notices: Vec::new(),
            trees: HashMap::new(),
            issues: HashMap::new(),
            issues_gen: 0,
            applied: HashMap::new(),
            synced: HashMap::new(),
            workspace: None,
            active: None,
            next_req: 1,
            completion: None,
            hover: None,
            definition: None,
            will_save: HashMap::new(),
            replies: Vec::new(),
            tree_reqs: HashMap::new(),
        };
        ui.reload(disabled);
        ui
    }

    /// Re-reads the extensions folder and (re)starts the enabled ones.
    pub fn reload(&mut self, disabled: &[String]) {
        let (exts, errors) = manifest::load_all();
        self.installed = exts;
        self.load_errors = errors;
        let enabled: Vec<Extension> = self.installed.iter().filter(|e| !disabled.contains(&e.identifier)).cloned().collect();
        self.status.retain(|id, _| enabled.iter().any(|e| &e.identifier == id));
        self.commands = enabled
            .iter()
            .flat_map(|e| {
                e.commands.iter().filter(|c| c.section != "text").map(|c| ExtCommand {
                    ext: e.identifier.clone(),
                    ext_name: e.name.clone(),
                    command: c.command.clone(),
                    title: c.title.clone(),
                    section: c.section.clone(),
                    syntaxes: c.syntaxes.clone(),
                    shortcut: c.shortcut.as_deref().and_then(manifest::parse_shortcut),
                })
            })
            .collect();
        // Issues of extensions that went away disappear.
        let ids: HashSet<String> = enabled.iter().map(|e| e.identifier.clone()).collect();
        self.issues.retain(|_, (ext, _)| ids.contains(ext));
        self.issues_gen += 1;
        self.trees.retain(|(ext, _), _| ids.contains(ext));
        self.host.send(ToHost::Load(enabled));
        // Their themes appear in (or leave) the theme list.
        crate::theme::themes_changed();
    }

    pub fn name_of(&self, ext: &str) -> String {
        self.installed.iter().find(|e| e.identifier == ext).map_or_else(|| ext.to_string(), |e| e.name.clone())
    }

    pub fn running(&self) -> bool {
        self.status.values().any(|s| *s == ExtStatus::Active)
    }

    fn req(&mut self) -> u64 {
        self.next_req += 1;
        self.next_req
    }

    fn reply(&mut self, ext: &str, cb: Option<u64>, value: Value) {
        if let Some(cb) = cb {
            self.replies.push((ext.to_string(), cb, value));
        }
    }

    fn log(&mut self, ext: &str, level: &str, text: String) {
        self.log.push_back(LogLine { ext: ext.to_string(), level: level.to_string(), text });
        while self.log.len() > 3000 {
            self.log.pop_front();
        }
    }

}

/// Comparable form of a document URI (case and encoding insensitive).
pub fn uri_key(uri: &str) -> String {
    let decoded = percent_decode(uri);
    let s = decoded.strip_prefix("file://").map(|p| p.trim_start_matches('/').to_string()).unwrap_or(decoded);
    s.replace('\\', "/").to_lowercase()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn file_uri(path: &std::path::Path) -> String {
    let p = crate::ext::host::js_path(path);
    let mut out = String::from("file:///");
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:!$&'()*+,;=@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn u16_to_char(rope: &ropey::Rope, u: usize) -> usize {
    rope.utf16_cu_to_char(u.min(rope.len_utf16_cu()))
}

impl NebulaApp {
    pub(super) fn doc_info(&self, doc: &Document) -> DocInfo {
        let (uri, path, remote) = match &doc.source {
            DocSource::Local(p) => (file_uri(p), Some(crate::ext::host::js_path(p)), false),
            DocSource::Remote { site_id, path } => {
                let uri = match self.sites.get(site_id) {
                    Some(s) => format!("{}://{}@{}{}", s.protocol.scheme(), s.user, s.host, path),
                    None => format!("remote://{site_id}{path}"),
                };
                (uri, Some(path.clone()), true)
            }
            DocSource::Untitled(n) => (format!("unsaved://{}/Ohne%20Titel%20{n}", doc.id), None, false),
        };
        let editor = self.settings.editor_for_doc(doc);
        DocInfo {
            id: doc.id,
            uri,
            path,
            syntax: manifest::syntax_name(doc.language).map(str::to_string),
            remote,
            dirty: doc.is_dirty(),
            eol: if doc.line_ending == LineEnding::CrLf { "\r\n".into() } else { "\n".into() },
            tab_length: editor.tab_size,
            soft_tabs: editor.use_spaces,
        }
    }

    /// Runs every frame: mirrors documents to the host and handles its messages.
    pub(super) fn sync_extensions(&mut self, ctx: &egui::Context) {
        let events = self.ext.host.poll();
        for ev in events {
            self.handle_host_event(ctx, ev);
        }

        let ws = self.sidebar.root.as_ref().map(|p| p.to_string_lossy().into_owned());
        if self.ext.workspace.as_ref() != Some(&ws) {
            self.ext.workspace = Some(ws.clone());
            self.ext.host.send(ToHost::Workspace(ws));
        }

        // Documents: opened, changed, selection, renamed, closed.
        let mut seen = HashSet::new();
        for i in 0..self.docs.len() {
            let id = self.docs[i].id;
            seen.insert(id);
            let info = self.doc_info(&self.docs[i]);
            let doc = &self.docs[i];
            let meta = format!("{}|{:?}", info.uri, info.syntax);
            let primary = doc.primary_selection();
            let mut sels: Vec<(usize, usize)> = vec![(primary.anchor.min(primary.head), primary.anchor.max(primary.head))];
            sels.extend(doc.selections.iter().filter(|s| **s != primary).map(|s| (s.range().start, s.range().end)));
            match self.ext.synced.get_mut(&id) {
                None => {
                    self.ext.host.send(ToHost::DocOpened(info.clone(), doc.rope.clone()));
                    self.ext.host.send(ToHost::Selection { id, ranges: sels.clone() });
                    self.ext.synced.insert(id, Synced { version: doc.version, sels, meta, dirty: info.dirty });
                }
                Some(s) => {
                    if s.meta != meta {
                        s.meta = meta;
                        self.ext.host.send(ToHost::DocMeta(info.clone()));
                    }
                    if s.version != doc.version || s.dirty != info.dirty {
                        s.version = doc.version;
                        s.dirty = info.dirty;
                        self.ext.host.send(ToHost::DocChanged { id, text: doc.rope.clone(), dirty: info.dirty });
                    }
                    if s.sels != sels {
                        s.sels = sels.clone();
                        self.ext.host.send(ToHost::Selection { id, ranges: sels });
                    }
                }
            }
        }
        let closed: Vec<u64> = self.ext.synced.keys().filter(|id| !seen.contains(id)).copied().collect();
        for id in closed {
            self.ext.synced.remove(&id);
            self.ext.applied.remove(&id);
            self.ext.host.send(ToHost::DocClosed(id));
        }
        let active = self.active.and_then(|i| self.docs.get(i)).map(|d| d.id);
        if self.ext.active != Some(active) {
            self.ext.active = Some(active);
            self.ext.host.send(ToHost::ActiveDoc(active));
        }

        // Answers wait until the documents they depend on were mirrored.
        for (ext, cb, value) in std::mem::take(&mut self.ext.replies) {
            self.ext.host.send(ToHost::Reply { ext, cb, value });
        }

        self.forward_editor_requests();
        self.apply_diagnostics();
    }

    /// Completion, hover and definition requests from the active editor.
    fn forward_editor_requests(&mut self) {
        let Some(i) = self.active else { return };
        let running = self.ext.running();
        let doc = &mut self.docs[i];
        let id = doc.id;
        if let Some((pos, trigger)) = doc.view.completion_request.take()
            && running
        {
            let req = self.ext.req();
            self.ext.completion = Some((req, id, pos));
            self.ext.host.send(ToHost::Completion { req, doc: id, pos, trigger: trigger.map(String::from) });
        }
        let doc = &mut self.docs[i];
        if let Some(pos) = doc.view.hover_request.take()
            && running
        {
            let req = self.ext.req();
            self.ext.hover = Some((req, id, pos));
            self.ext.host.send(ToHost::Hover { req, doc: id, pos });
        }
        let doc = &mut self.docs[i];
        if let Some(pos) = doc.view.definition_request.take() {
            if running {
                let req = self.ext.req();
                self.ext.definition = Some((req, id, pos));
                self.ext.host.send(ToHost::Definition { req, doc: id, pos });
            } else {
                self.builtin_definition(id, pos);
            }
        }
    }

    /// Converts extension problems into editor diagnostics for open documents.
    fn apply_diagnostics(&mut self) {
        let generation = self.ext.issues_gen;
        for i in 0..self.docs.len() {
            let (id, version) = (self.docs[i].id, self.docs[i].version);
            if self.ext.applied.get(&id) == Some(&(generation, version)) {
                continue;
            }
            self.ext.applied.insert(id, (generation, version));
            let key = uri_key(&self.doc_info(&self.docs[i]).uri);
            let doc = &mut self.docs[i];
            let mut diags = Vec::new();
            for (_, files) in self.ext.issues.values() {
                let Some(list) = files.get(&key) else { continue };
                for issue in list {
                    if let Some(d) = diagnostic(doc, issue) {
                        diags.push(d);
                    }
                }
            }
            diags.sort_by_key(|d| d.range.start);
            doc.view.diagnostics = diags;
        }
    }

    pub(super) fn ext_doc_saved(&mut self, doc_id: u64) {
        self.ext.host.send(ToHost::DocSaved(doc_id));
    }

    /// Saving asks extensions first (formatters run in `onWillSave`).
    pub(super) fn save_doc_checked(&mut self, index: usize, close_after: bool) -> bool {
        let Some(doc) = self.docs.get(index) else { return false };
        if !self.ext.running() || matches!(doc.source, DocSource::Untitled(_)) {
            return self.save_doc_with(index, close_after, false);
        }
        let id = doc.id;
        if self.ext.will_save.values().any(|(d, _)| *d == id) {
            return true;
        }
        let req = self.ext.req();
        self.ext.will_save.insert(req, (id, close_after));
        self.ext.host.send(ToHost::WillSave { req, doc: id });
        true
    }

    pub(super) fn run_ext_command(&mut self, cmd: &ExtCommand) {
        let editor = self.active.and_then(|i| self.docs.get(i)).map(|d| d.id);
        self.ext.host.send(ToHost::Command { ext: cmd.ext.clone(), name: cmd.command.clone(), editor, args: Vec::new() });
    }

    /// Commands usable for the active document.
    pub(super) fn available_ext_commands(&self) -> Vec<ExtCommand> {
        let syntax = self.active.and_then(|i| self.docs.get(i)).and_then(|d| manifest::syntax_name(d.language));
        self.ext
            .commands
            .iter()
            .filter(|c| c.syntaxes.is_empty() || syntax.is_some_and(|s| c.syntaxes.iter().any(|x| x == s)))
            .filter(|c| c.section != "editor" || self.active.is_some())
            .cloned()
            .collect()
    }

    /// Keyboard shortcuts of extension commands.
    pub(super) fn ext_shortcuts(&mut self, ctx: &egui::Context) {
        let cmds: Vec<ExtCommand> = self.available_ext_commands().into_iter().filter(|c| c.shortcut.is_some()).collect();
        for c in cmds {
            let sc = c.shortcut.unwrap();
            if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
                self.run_ext_command(&c);
            }
        }
    }

    fn handle_host_event(&mut self, ctx: &egui::Context, ev: HostEvent) {
        let HostEvent { ext, msg } = ev;
        let ext_name = self.ext.name_of(&ext);
        match msg {
            FromHost::Activated => {
                self.ext.status.insert(ext, ExtStatus::Active);
            }
            FromHost::Failed(e) => {
                self.error(t!("Erweiterung „{ext_name}“ konnte nicht gestartet werden.", ext_name = ext_name));
                self.ext.status.insert(ext, ExtStatus::Failed(e));
            }
            FromHost::Log { level, text } => self.ext.log(&ext, &level, text),
            FromHost::Message { level, text } => {
                let kind = if level == "error" { ToastKind::Error } else { ToastKind::Info };
                let secs = if level == "info" { 5 } else { 10 };
                self.toasts.push(Toast { text: format!("{ext_name}: {text}"), kind, until: Instant::now() + Duration::from_secs(secs) });
            }
            FromHost::Edit { doc, edits, cb } => {
                let ok = self.apply_ext_edits(doc, &edits);
                self.ext.reply(&ext, cb, json!(ok));
            }
            FromHost::Insert { doc, text, snippet: is_snippet, cb } => {
                let ok = self.ext_insert(doc, &text, is_snippet);
                self.ext.reply(&ext, cb, json!(ok));
            }
            FromHost::Select { doc, ranges } => {
                if let Some(i) = self.doc_index(doc) {
                    let d = &mut self.docs[i];
                    let sels: Vec<Selection> = ranges
                        .iter()
                        .map(|r| Selection::new(u16_to_char(&d.rope, r[0]), u16_to_char(&d.rope, r[1])))
                        .collect();
                    if !sels.is_empty() {
                        d.selections = sels;
                        d.primary = 0;
                        d.normalize_selections();
                    }
                }
            }
            FromHost::Scroll { doc, pos } => {
                if let Some(i) = self.doc_index(doc) {
                    let d = &mut self.docs[i];
                    let c = u16_to_char(&d.rope, pos);
                    d.view.center_line = Some(d.char_to_line(c));
                }
            }
            FromHost::Save { doc, cb } => {
                let ok = self.doc_index(doc).is_some_and(|i| self.save_doc_with(i, false, false));
                self.ext.reply(&ext, cb, json!(ok));
            }
            FromHost::ActionPanel { cb, message, buttons } => {
                self.ext.prompts.push_back(Prompt::Action { ext, cb, message, buttons });
            }
            FromHost::InputPanel { cb, message, label, placeholder, value, prompt, secure } => {
                self.ext.prompts.push_back(Prompt::Input { ext, cb, message, label, placeholder, value, prompt, secure, focus: true });
            }
            FromHost::InputPalette { cb, message, placeholder, value } => {
                self.ext.prompts.push_back(Prompt::Palette { ext, cb, message, placeholder, value, focus: true });
            }
            FromHost::ChoicePalette { cb, choices, placeholder } => {
                self.ext.prompts.push_back(Prompt::Choice { ext, cb, choices, placeholder, query: String::new(), selected: 0, focus: true });
            }
            FromHost::FileChooser { cb, message, allow_files, allow_folders, allow_multiple } => {
                let mut dialog = rfd::FileDialog::new().set_title(if message.is_empty() { ext_name.clone() } else { message });
                if let Some(root) = &self.sidebar.root {
                    dialog = dialog.set_directory(root);
                }
                let folders = allow_folders.unwrap_or(false) && !allow_files.unwrap_or(true);
                let picked: Option<Vec<PathBuf>> = match (folders, allow_multiple.unwrap_or(false)) {
                    (true, true) => dialog.pick_folders(),
                    (true, false) => dialog.pick_folder().map(|p| vec![p]),
                    (false, true) => dialog.pick_files(),
                    (false, false) => dialog.pick_file().map(|p| vec![p]),
                };
                let value = picked.map_or(Value::Null, |v| json!(v.iter().map(|p| crate::ext::host::js_path(p)).collect::<Vec<_>>()));
                self.ext.reply(&ext, cb, value);
            }
            FromHost::Notify { cb, id, title, body, kind, value, placeholder, actions } => {
                // A new notification with the same id replaces the old one.
                if let Some(pos) = self.ext.notices.iter().position(|n| n.ext == ext && n.id == id) {
                    let old = self.ext.notices.remove(pos);
                    self.ext.reply(&old.ext, old.cb, json!({"actionIdx": null}));
                }
                let input = matches!(kind.as_str(), "input" | "secure-input").then(|| (value, placeholder, kind == "secure-input"));
                self.ext.notices.push(Notice { ext, id, cb, title, body, input, actions });
            }
            FromHost::CancelNotification { id } => {
                if let Some(pos) = self.ext.notices.iter().position(|n| n.ext == ext && n.id == id) {
                    let old = self.ext.notices.remove(pos);
                    self.ext.reply(&old.ext, old.cb, json!({"actionIdx": null}));
                }
            }
            FromHost::Issues { collection, uri, issues, .. } => {
                let entry = self.ext.issues.entry(collection).or_insert_with(|| (ext.clone(), HashMap::new()));
                let key = uri_key(&uri);
                if issues.is_empty() {
                    entry.1.remove(&key);
                } else {
                    entry.1.insert(key, issues);
                }
                self.ext.issues_gen += 1;
            }
            FromHost::ClearIssues { collection } => {
                self.ext.issues.remove(&collection);
                self.ext.issues_gen += 1;
            }
            FromHost::Completions { req, items } => {
                self.ext_completions(req, items);
                // The popup sizes itself from the previous frame.
                ctx.request_repaint();
            }
            FromHost::Hover { req, text } => {
                if let Some((r, doc, pos)) = self.ext.hover
                    && r == req
                    && let Some(i) = self.doc_index(doc)
                {
                    self.docs[i].view.hover = text.map(|t| (pos, t));
                    ctx.request_repaint();
                }
            }
            FromHost::Definition { req, locations } => {
                if let Some((r, doc, pos)) = self.ext.definition
                    && r == req
                {
                    self.ext.definition = None;
                    match locations.first() {
                        Some(l) => {
                            self.goto_location(&l.uri, l.line, l.column);
                        }
                        None => self.builtin_definition(doc, pos),
                    }
                }
            }
            FromHost::WillSaveDone { req } => {
                if let Some((doc, close_after)) = self.ext.will_save.remove(&req)
                    && let Some(i) = self.doc_index(doc)
                {
                    self.save_doc_with(i, close_after, false);
                }
            }
            FromHost::OpenFile { cb, uri, line, column } => {
                let id = self.goto_location(&uri, line.unwrap_or(0), column.unwrap_or(1));
                self.ext.reply(&ext, cb, json!(id));
            }
            FromHost::NewDocument { cb, content, syntax } => {
                self.new_document();
                let id = self.active.map(|i| {
                    let d = &mut self.docs[i];
                    if let Some(l) = syntax.as_deref().and_then(crate::lang::Language::from_id) {
                        d.language = l;
                        d.highlight_version = u64::MAX;
                    }
                    if !content.is_empty() {
                        d.apply(vec![Change::new(0..0, content.clone())], Some(vec![Selection::cursor(0)]), crate::editor::document::EditKind::Other);
                    }
                    d.id
                });
                self.ext.reply(&ext, cb, json!(id));
            }
            FromHost::OpenUrl { url } => {
                crate::win32::open_url(&url);
            }
            FromHost::OpenConfig { identifier } => {
                self.settings_ui.selected_extension = identifier.or(Some(ext));
                self.open_settings(super::settings_page::Page::Extensions);
            }
            FromHost::TreeView { view, alive } => {
                let t = self.ext.trees.entry((ext, view)).or_default();
                t.alive = alive;
                t.children.clear();
            }
            FromHost::TreeReload { view } => {
                if let Some(t) = self.ext.trees.get_mut(&(ext, view)) {
                    t.children.clear();
                }
            }
            FromHost::TreeChildren { view, req, items } => {
                if let Some((e, v, element)) = self.ext.tree_reqs.remove(&req)
                    && e == ext
                    && v == view
                {
                    let t = self.ext.trees.entry((e, v)).or_default();
                    for item in &items {
                        if item.collapsible == 2 {
                            t.expanded.insert(item.id);
                        }
                    }
                    t.children.insert(element, Some(items));
                }
            }
            FromHost::ContextChanged { .. } => {}
        }
    }

    /// Children of a tree element, requested from the extension on first use.
    pub(super) fn tree_children(&mut self, ext: &str, view: &str, element: Option<u64>) -> Option<Vec<crate::ext::protocol::TreeItemMsg>> {
        let key = (ext.to_string(), view.to_string());
        let t = self.ext.trees.entry(key).or_default();
        match t.children.get(&element) {
            Some(Some(items)) => Some(items.clone()),
            Some(None) => None,
            None => {
                t.children.insert(element, None);
                let req = self.ext.req();
                self.ext.tree_reqs.insert(req, (ext.to_string(), view.to_string(), element));
                self.ext.host.send(ToHost::TreeChildren { ext: ext.to_string(), view: view.to_string(), req, element });
                None
            }
        }
    }

    pub(super) fn tree_event(&mut self, ext: &str, view: &str, kind: &str, element: Option<u64>) {
        self.ext.host.send(ToHost::TreeEvent { ext: ext.to_string(), view: view.to_string(), kind: kind.to_string(), element });
    }

    /// Applies an extension's edit operations as one undo step.
    fn apply_ext_edits(&mut self, doc_id: u64, edits: &[crate::ext::protocol::EditOp]) -> bool {
        let Some(i) = self.doc_index(doc_id) else { return false };
        let vars = self.snippet_vars(i);
        let doc = &mut self.docs[i];
        let mut cursor: Option<std::ops::Range<usize>> = None;
        let texts: Vec<(String, Option<std::ops::Range<usize>>)> = edits
            .iter()
            .map(|e| {
                if e.snippet {
                    let x = snippet::expand(&e.text, &vars);
                    (x.text, Some(x.select))
                } else {
                    (e.text.clone(), None)
                }
            })
            .collect();
        doc.apply_sequential(edits.len(), |rope, n| {
            let e = &edits[n];
            let s = u16_to_char(rope, e.start.min(e.end));
            let end = u16_to_char(rope, e.end.max(e.start));
            let (text, select) = &texts[n];
            if e.insert || select.is_some() {
                let r = select.clone().unwrap_or_else(|| {
                    let n = text.chars().count();
                    n..n
                });
                cursor = Some(s + r.start..s + r.end);
            }
            Change::new(s..end, text.clone())
        });
        if let Some(r) = cursor {
            doc.set_selection(Selection::new(r.start, r.end));
        }
        doc.view.completion = None;
        true
    }

    fn ext_insert(&mut self, doc_id: u64, text: &str, is_snippet: bool) -> bool {
        let Some(i) = self.doc_index(doc_id) else { return false };
        let mut vars = self.snippet_vars(i);
        let doc = &mut self.docs[i];
        let primary = doc.primary_selection().range();
        vars.selected_text = doc.slice(primary.clone());
        let (text, select) = if is_snippet {
            let x = snippet::expand(text, &vars);
            (x.text, Some(x.select))
        } else {
            (text.to_string(), None)
        };
        let single = doc.selections.len() == 1;
        doc.replace_selections(crate::editor::document::EditKind::Other, |_| text.clone());
        if single && let Some(r) = select {
            doc.set_selection(Selection::new(primary.start + r.start, primary.start + r.end));
        }
        true
    }

    pub(super) fn snippet_vars(&self, i: usize) -> snippet::Vars {
        let doc = &self.docs[i];
        let path = match &doc.source {
            DocSource::Local(p) => p.display().to_string(),
            DocSource::Remote { path, .. } => path.clone(),
            DocSource::Untitled(_) => String::new(),
        };
        let sel = doc.primary_selection().range();
        snippet::Vars {
            selected_text: doc.slice(sel),
            filename: doc.title(),
            parent_folder: std::path::Path::new(&path).parent().map(|p| p.display().to_string()).unwrap_or_default(),
            file_path: path,
            workspace_name: self.sidebar.root.as_ref().and_then(|r| r.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }

    fn ext_completions(&mut self, req: u64, items: Vec<CompletionMsg>) {
        let Some((r, doc_id, pos)) = self.ext.completion else { return };
        if r != req || items.is_empty() {
            return;
        }
        let Some(i) = self.doc_index(doc_id) else { return };
        if self.active != Some(i) {
            return;
        }
        let doc = &mut self.docs[i];
        let converted: Vec<complete::Item> = items
            .into_iter()
            .map(|m| {
                let raw = m.insert_text.clone().unwrap_or_else(|| m.label.clone());
                let insert = if m.snippet {
                    let x = snippet::expand(&raw, &snippet::Vars::default());
                    let mut t: Vec<char> = x.text.chars().collect();
                    // Our completion format marks only the cursor ($0).
                    let at = x.select.start.min(t.len());
                    let sel: String = t.drain(at..x.select.end.min(t.len())).collect();
                    let mut s: String = t.iter().collect();
                    let byte = s.char_indices().nth(at).map_or(s.len(), |(b, _)| b);
                    s.insert_str(byte, &format!("{sel}$0"));
                    if sel.is_empty() { s } else { s.replacen(&format!("{sel}$0"), &format!("$0{sel}"), 1) }
                } else {
                    raw.replace("$0", "")
                };
                complete::Item {
                    label: m.label.clone(),
                    insert,
                    kind: complete::Kind::Ext,
                    detail: m.detail.clone().or(m.kind.clone()).unwrap_or_default().chars().take(48).collect(),
                    start: m.start.map(|s| u16_to_char(&doc.rope, s)),
                    end: m.end.map(|e| u16_to_char(&doc.rope, e)),
                    filter: m.filter_text.clone(),
                }
            })
            .collect();
        doc.view.ext_items = Some((pos, converted));
        doc.view.completion = complete::compute(doc);
    }

    /// Opens a file at a 1-based line and column; returns the document id.
    fn goto_location(&mut self, uri: &str, line: usize, column: usize) -> Option<u64> {
        let path = if let Some(rest) = uri.strip_prefix("file://") {
            let p = percent_decode(rest);
            PathBuf::from(p.trim_start_matches('/').replace('/', "\\"))
        } else {
            PathBuf::from(uri.replace('/', "\\"))
        };
        if !path.is_file() {
            self.error(t!("{path} kann nicht geöffnet werden.", path = path.display()));
            return None;
        }
        self.open_local(&path);
        let i = self.active?;
        let doc = &mut self.docs[i];
        if line > 0 {
            let l = (line - 1).min(doc.len_lines().saturating_sub(1));
            let start = doc.line_to_char(l);
            let u = doc.rope.char_to_utf16_cu(start) + column.saturating_sub(1);
            let pos = u16_to_char(&doc.rope, u).min(doc.line_end(l));
            doc.set_selection(Selection::cursor(pos));
            doc.view.center_line = Some(l);
        }
        self.focus_editor = true;
        Some(doc.id)
    }

    // ---- prompts and notifications ------------------------------------------------

    /// Draws the front extension prompt as a modal; returns true if one is shown.
    pub(super) fn ext_prompts(&mut self, ctx: &egui::Context) -> bool {
        let Some(mut prompt) = self.ext.prompts.pop_front() else { return false };
        let mut done: Option<(String, Option<u64>, Value)> = None;
        let accent = self.theme.accent;
        let modal = super::modal(ctx, egui::Id::unique("ext_prompt"));
        match &mut prompt {
            Prompt::Action { ext, cb, message, buttons } => {
                let name = self.ext.name_of(ext);
                modal.show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.heading(&name);
                    ui.add(egui::Label::new(message.as_str()).wrap());
                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        for (i, b) in buttons.iter().enumerate() {
                            let clicked = if i == 0 { ui.add(super::primary_button(b, accent)).clicked() } else { ui.button(b).clicked() };
                            if clicked {
                                done = Some((ext.clone(), *cb, json!(i)));
                            }
                        }
                    });
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        done = Some((ext.clone(), *cb, Value::Null));
                    }
                });
            }
            Prompt::Input { ext, cb, message, label, placeholder, value, prompt: ok, secure, focus } => {
                let name = self.ext.name_of(ext);
                modal.show(ctx, |ui| {
                    ui.set_width(420.0);
                    ui.heading(&name);
                    ui.add(egui::Label::new(message.as_str()).wrap());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if !label.is_empty() {
                            ui.label(label.as_str());
                        }
                        let r = ui.add(egui::TextEdit::singleline(value).hint_text(placeholder.as_str()).password(*secure).desired_width(f32::INFINITY));
                        if std::mem::take(focus) {
                            r.request_focus();
                        }
                        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            done = Some((ext.clone(), *cb, json!(value.clone())));
                        }
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let ok = if ok.is_empty() { "OK" } else { ok.as_str() };
                        if ui.add(super::primary_button(ok, accent)).clicked() {
                            done = Some((ext.clone(), *cb, json!(value.clone())));
                        }
                        if ui.button(t!("Abbrechen")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            done = Some((ext.clone(), *cb, Value::Null));
                        }
                    });
                });
            }
            Prompt::Palette { ext, cb, message, placeholder, value, focus } => {
                modal.show(ctx, |ui| {
                    ui.set_width(560.0);
                    if !message.is_empty() {
                        ui.label(RichText::new(message.as_str()).color(self.theme.text_muted));
                    }
                    let r = ui.add(egui::TextEdit::singleline(value).hint_text(placeholder.as_str()).desired_width(f32::INFINITY).font(egui::TextStyle::Heading));
                    if std::mem::take(focus) {
                        r.request_focus();
                    }
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        done = Some((ext.clone(), *cb, json!(value.clone())));
                    } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                        done = Some((ext.clone(), *cb, Value::Null));
                    }
                });
            }
            Prompt::Choice { ext, cb, choices, placeholder, query, selected, focus } => {
                let q = query.to_lowercase();
                let shown: Vec<(usize, &String)> = choices.iter().enumerate().filter(|(_, c)| q.is_empty() || c.to_lowercase().contains(&q)).take(200).collect();
                *selected = (*selected).min(shown.len().saturating_sub(1));
                modal.show(ctx, |ui| {
                    ui.set_width(560.0);
                    let r = ui.add(egui::TextEdit::singleline(query).hint_text(placeholder.as_str()).desired_width(f32::INFINITY).font(egui::TextStyle::Heading));
                    if std::mem::take(focus) {
                        r.request_focus();
                    }
                    if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
                        *selected = (*selected + 1).min(shown.len().saturating_sub(1));
                    }
                    if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
                        *selected = selected.saturating_sub(1);
                    }
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for (n, (idx, c)) in shown.iter().enumerate() {
                            let resp = ui.add(egui::Button::selectable(n == *selected, c.as_str()).min_size(vec2(ui.available_width(), 26.0)));
                            if n == *selected {
                                resp.scroll_to_me(None);
                            }
                            if resp.clicked() {
                                done = Some((ext.clone(), *cb, json!(idx)));
                            }
                        }
                    });
                    if ui.input(|i| i.key_pressed(Key::Enter))
                        && let Some((idx, _)) = shown.get(*selected)
                    {
                        done = Some((ext.clone(), *cb, json!(idx)));
                    } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                        done = Some((ext.clone(), *cb, Value::Null));
                    }
                });
            }
        }
        match done {
            Some((ext, cb, value)) => {
                self.ext.reply(&ext, cb, value);
                self.focus_editor = true;
            }
            None => self.ext.prompts.push_front(prompt),
        }
        true
    }

    /// Extension notifications, stacked above the toasts.
    pub(super) fn ext_notices(&mut self, ctx: &egui::Context) {
        if self.ext.notices.is_empty() {
            return;
        }
        let theme = self.theme.clone();
        let mut answer: Option<(usize, Value)> = None;
        let offset = self.preview_state.covered_width;
        egui::Area::new(egui::Id::unique("ext_notices"))
            .anchor(Align2::RIGHT_TOP, vec2(-18.0 - offset, 56.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for (n, notice) in self.ext.notices.iter_mut().enumerate().take(4) {
                    let name = self.ext.installed.iter().find(|e| e.identifier == notice.ext).map_or(notice.ext.clone(), |e| e.name.clone());
                    egui::Frame::new()
                        .fill(ui.visuals().window_fill)
                        .stroke(ui.visuals().window_stroke)
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::same(12))
                        .shadow(ui.visuals().popup_shadow)
                        .show(ui, |ui| {
                            ui.set_width(356.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&name).small().color(theme.text_muted));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new("×").frame(false)).clicked() {
                                        answer = Some((n, json!({"actionIdx": null})));
                                    }
                                });
                            });
                            if !notice.title.is_empty() {
                                ui.label(RichText::new(&notice.title).strong().color(theme.foreground));
                            }
                            if !notice.body.is_empty() {
                                ui.add(egui::Label::new(RichText::new(&notice.body).color(theme.foreground)).wrap());
                            }
                            if let Some((value, placeholder, secure)) = &mut notice.input {
                                ui.add(egui::TextEdit::singleline(value).hint_text(placeholder.as_str()).password(*secure).desired_width(f32::INFINITY));
                            }
                            let actions = if notice.actions.is_empty() { vec!["OK".to_string()] } else { notice.actions.clone() };
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for (i, a) in actions.iter().enumerate() {
                                    if ui.button(a).clicked() {
                                        let text = notice.input.as_ref().map(|(v, _, _)| v.clone());
                                        answer = Some((n, json!({"actionIdx": i, "textInputValue": text})));
                                    }
                                }
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some((n, value)) = answer {
            let notice = self.ext.notices.remove(n);
            self.ext.reply(&notice.ext, notice.cb, value);
        }
        let _ = Sense::hover();
        let _ = Color32::TRANSPARENT;
    }
}

/// Converts an extension issue into a diagnostic for a document.
fn diagnostic(doc: &Document, issue: &IssueMsg) -> Option<Diagnostic> {
    let rope = &doc.rope;
    let len = rope.len_chars();
    let (start, end) = match (issue.start, issue.end, issue.line) {
        (Some(s), e, _) => {
            let s = u16_to_char(rope, s);
            (s, e.map_or(s, |e| u16_to_char(rope, e)))
        }
        (None, _, Some(line)) => {
            let at = |line: usize, col: Option<usize>| -> usize {
                let l = line.saturating_sub(1).min(rope.len_lines().saturating_sub(1));
                let ls = rope.line_to_char(l);
                let le = doc.line_end(l);
                match col {
                    Some(c) => u16_to_char(rope, rope.char_to_utf16_cu(ls) + c.saturating_sub(1)).min(le),
                    None => ls,
                }
            };
            let s = at(line, issue.column);
            let e = match issue.end_line {
                Some(el) => at(el, issue.end_column),
                None if issue.column.is_none() => doc.line_end(line.saturating_sub(1).min(rope.len_lines().saturating_sub(1))),
                None => s,
            };
            (s, e)
        }
        _ => return None,
    };
    let start = start.min(len);
    let mut end = end.clamp(start, len);
    if end == start {
        // Mark the word at the position, or one character.
        end = crate::editor::commands::word_at(doc, start).filter(|r| r.start <= start).map_or((start + 1).min(len), |r| r.end.max(start + 1).min(len));
    }
    let mut message = issue.message.clone();
    if let Some(src) = &issue.source {
        message = format!("{message} ({src})");
    }
    Some(Diagnostic { range: start..end, severity: if issue.severity == 0 { 2 } else { issue.severity }, message })
}

impl NebulaApp {
    /// Clips from enabled extensions that fit the active document.
    pub(super) fn available_clips(&self) -> Vec<(String, manifest::Clip)> {
        let syntax = self.active.and_then(|i| self.docs.get(i)).and_then(|d| manifest::syntax_name(d.language));
        self.ext
            .installed
            .iter()
            .filter(|e| !self.settings.disabled_extensions.contains(&e.identifier))
            .flat_map(|e| e.clips.iter().map(move |c| (e.name.clone(), c.clone())))
            .filter(|(_, c)| c.syntax.as_deref().is_none_or(|s| Some(s) == syntax))
            .collect()
    }

    pub(super) fn insert_clip(&mut self, clip: &manifest::Clip) {
        let Some(i) = self.active else { return };
        let id = self.docs[i].id;
        self.ext_insert(id, &clip.content, true);
        self.focus_editor = true;
    }
}

impl NebulaApp {
    /// Sidebars contributed by extensions, drawn into the sidebar's list area.
    pub(super) fn extension_sidebars(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let theme = self.theme.clone();
        let sections: Vec<(String, String, manifest::SidebarSection)> = self
            .ext
            .installed
            .iter()
            .filter(|e| !self.settings.disabled_extensions.contains(&e.identifier))
            .flat_map(|e| {
                e.sidebars.iter().flat_map(move |s| {
                    s.sections.iter().map(move |sec| (e.identifier.clone(), if sec.name.is_empty() { s.name.clone() } else { sec.name.clone() }, sec.clone()))
                })
            })
            .collect();
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 4.0))));
        egui::ScrollArea::vertical().id_salt("ext-sidebars").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (ext, title, sec) in sections {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(theme.text_faint));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (label, cmd) in sec.header_commands.iter().rev() {
                            let text = if label.is_empty() { cmd.rsplit('.').next().unwrap_or(cmd).to_string() } else { label.clone() };
                            if ui.small_button(text).clicked() {
                                self.ext.host.send(ToHost::Command { ext: ext.clone(), name: cmd.clone(), editor: None, args: Vec::new() });
                            }
                        }
                    });
                });
                let alive = self.ext.trees.get(&(ext.clone(), sec.id.clone())).is_some_and(|t| t.alive);
                if !alive {
                    let status = self.ext.status.get(&ext);
                    let text = match status {
                        Some(ExtStatus::Failed(_)) => t!("Die Erweiterung konnte nicht gestartet werden."),
                        _ if sec.placeholder.is_empty() => t!("Wird geladen, sobald die Erweiterung läuft…"),
                        _ => sec.placeholder.as_str(),
                    };
                    ui.label(RichText::new(text).small().color(theme.text_muted));
                    continue;
                }
                self.tree_level(ui, &ext, &sec, None, 0);
            }
        });
    }

    fn tree_level(&mut self, ui: &mut egui::Ui, ext: &str, sec: &manifest::SidebarSection, element: Option<u64>, depth: usize) {
        let theme = self.theme.clone();
        let Some(items) = self.tree_children(ext, &sec.id, element) else {
            if element.is_none() {
                ui.spinner();
            }
            return;
        };
        if items.is_empty() && element.is_none() && !sec.placeholder.is_empty() {
            ui.label(RichText::new(&sec.placeholder).small().color(theme.text_muted));
        }
        let key = (ext.to_string(), sec.id.clone());
        for item in items {
            let (expanded, selected) = {
                let t = self.ext.trees.entry(key.clone()).or_default();
                (t.expanded.contains(&item.id), t.selected == Some(item.id))
            };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
            let painter = ui.painter();
            if selected {
                painter.rect_filled(rect, 5.0, theme.selection);
            } else if resp.hovered() {
                painter.rect_filled(rect, 5.0, theme.hover);
            }
            let x = rect.left() + 6.0 + depth as f32 * 14.0;
            let cy = rect.center().y;
            if item.collapsible > 0 {
                crate::ui::icons::chevron(painter, egui::pos2(x + 4.0, cy), expanded, theme.text_muted);
            }
            let mut tx = x + 14.0;
            if let Some(c) = item.color {
                let col = Color32::from_rgba_unmultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8);
                painter.circle_filled(egui::pos2(tx + 5.0, cy), 5.0, col);
                tx += 16.0;
            } else if let Some(p) = &item.path {
                let name = p.rsplit('/').next().unwrap_or(p);
                crate::ui::icons::file(painter, egui::pos2(tx + 5.0, cy), name);
                tx += 16.0;
            }
            let galley = painter.layout_no_wrap(item.name.clone(), egui::FontId::proportional(13.0), theme.foreground);
            let w = galley.size().x;
            painter.galley(egui::pos2(tx, cy - galley.size().y / 2.0), galley, theme.foreground);
            if let Some(d) = &item.description {
                painter.text(egui::pos2(tx + w + 8.0, cy), Align2::LEFT_CENTER, d, egui::FontId::proportional(11.5), theme.text_faint);
            }
            let resp = match &item.tooltip {
                Some(t) => resp.on_hover_text(t),
                None => resp,
            };
            if resp.clicked() {
                let t = self.ext.trees.entry(key.clone()).or_default();
                t.selected = Some(item.id);
                if item.collapsible > 0 {
                    if expanded {
                        t.expanded.remove(&item.id);
                    } else {
                        t.expanded.insert(item.id);
                    }
                    self.tree_event(ext, &sec.id, if expanded { "collapse" } else { "expand" }, Some(item.id));
                }
                self.tree_event(ext, &sec.id, "select", Some(item.id));
            }
            if resp.double_clicked() {
                self.tree_event(ext, &sec.id, "activate", Some(item.id));
            }
            if !sec.context_commands.is_empty() {
                resp.context_menu(|ui| {
                    for (label, cmd) in &sec.context_commands {
                        if ui.button(label).clicked() {
                            self.ext.trees.entry(key.clone()).or_default().selected = Some(item.id);
                            self.tree_event(ext, &sec.id, "select", Some(item.id));
                            self.ext.host.send(ToHost::Command { ext: ext.to_string(), name: cmd.clone(), editor: None, args: Vec::new() });
                            ui.close();
                        }
                    }
                });
            }
            if item.collapsible > 0 && expanded {
                self.tree_level(ui, ext, sec, Some(item.id), depth + 1);
            }
        }
    }
}
