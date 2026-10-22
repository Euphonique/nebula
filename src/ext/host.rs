//! The extension host: runs extension JavaScript (QuickJS) on its own
//! thread, like Nova's separate extension service. Every extension gets its
//! own context; the Nova API itself is implemented in `runtime.js` on top of
//! a few native operations defined here.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use ropey::Rope;
use rquickjs::context::EvalOptions;
use rquickjs::{Context, Ctx, Exception, Function, Object, Runtime, Value as JsValue};
use serde_json::{Value, json};

use super::config::ConfigStore;
use super::manifest::Extension;
use super::process::{self, ProcEvent, Running, Spawn};
use super::protocol::{DocInfo, FromHost, HostEvent, ToHost};

const RUNTIME_JS: &str = include_str!("runtime.js");
/// Longest a single call into an extension may run before it is interrupted.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the IDE waits for all extensions to answer a request.
const ANSWER_TIMEOUT: Duration = Duration::from_millis(1500);
const WILL_SAVE_TIMEOUT: Duration = Duration::from_secs(5);

enum HostMsg {
    Ui(ToHost),
    Proc(ProcEvent),
    Http { id: u64, result: Value },
    Shutdown,
}

/// UI-side handle of the extension host thread.
pub struct ExtHost {
    tx: Sender<HostMsg>,
    rx: Receiver<HostEvent>,
    pub config: Arc<Mutex<ConfigStore>>,
}

impl ExtHost {
    pub fn spawn(repaint: egui::Context) -> Self {
        let (tx, host_rx) = crossbeam_channel::unbounded();
        let (out_tx, rx) = crossbeam_channel::unbounded();
        let config = Arc::new(Mutex::new(ConfigStore::load()));
        let cfg = config.clone();
        let inbox = tx.clone();
        std::thread::Builder::new()
            .name("extension-host".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || Host::new(out_tx, inbox, repaint, cfg).run(host_rx))
            .expect("extension host thread");
        Self { tx, rx, config }
    }

    pub fn send(&self, msg: ToHost) {
        let _ = self.tx.send(HostMsg::Ui(msg));
    }

    pub fn poll(&self) -> Vec<HostEvent> {
        self.rx.try_iter().collect()
    }
}

impl Drop for ExtHost {
    fn drop(&mut self) {
        let _ = self.tx.send(HostMsg::Shutdown);
    }
}

/// Host-side mirror of an open document.
struct Doc {
    info: DocInfo,
    text: Rope,
    sels: Vec<(usize, usize)>,
}

impl Doc {
    fn to16(&self, c: usize) -> usize {
        self.text.char_to_utf16_cu(c.min(self.text.len_chars()))
    }
    fn from16(&self, u: usize) -> usize {
        self.text.utf16_cu_to_char(u.min(self.text.len_utf16_cu()))
    }
}

enum AnswerKind {
    Completion,
    Hover,
    Definition,
    WillSave,
}

struct Pending {
    kind: AnswerKind,
    waiting: usize,
    values: Vec<Value>,
    deadline: Instant,
}

/// State shared with the native functions of all contexts.
struct Shared {
    out: Sender<HostEvent>,
    inbox: Sender<HostMsg>,
    proc_tx: Sender<ProcEvent>,
    repaint: egui::Context,
    docs: HashMap<u64, Doc>,
    active: Option<u64>,
    workspace: Option<String>,
    config: Arc<Mutex<ConfigStore>>,
    timers: Vec<(Instant, usize, u64)>,
    procs: HashMap<u64, (usize, Running)>,
    http: HashMap<u64, usize>,
    next_id: u64,
    /// Events produced inside native calls, delivered afterwards (ext, event); `None` = all.
    deferred: Vec<(Option<usize>, Value)>,
    pending: HashMap<u64, Pending>,
    /// Extension identifiers by index.
    names: Vec<String>,
}

impl Shared {
    fn emit(&self, ext: usize, msg: FromHost) {
        let _ = self.out.send(HostEvent { ext: self.names.get(ext).cloned().unwrap_or_default(), msg });
        self.repaint.request_repaint();
    }
}

struct ExtState {
    ext: Extension,
    ctx: Option<Context>,
    failed: bool,
}

struct Host {
    rt: Runtime,
    exts: Vec<ExtState>,
    shared: Rc<RefCell<Shared>>,
    deadline: Rc<Cell<Option<Instant>>>,
    proc_rx: Receiver<ProcEvent>,
}

/// Forward-slash path for JavaScript.
pub fn js_path(p: &Path) -> String {
    crate::ui::sidebar::strip_verbatim(p).to_string_lossy().replace('\\', "/")
}

fn native_path(p: &str) -> PathBuf {
    let p = p.strip_prefix("file://").unwrap_or(p);
    // file:///C:/x -> C:/x
    let p = if p.len() > 3 && p.starts_with('/') && p.as_bytes()[2] == b':' { &p[1..] } else { p };
    PathBuf::from(p.replace('/', "\\"))
}

impl Host {
    fn new(out: Sender<HostEvent>, inbox: Sender<HostMsg>, repaint: egui::Context, config: Arc<Mutex<ConfigStore>>) -> Self {
        let rt = Runtime::new().expect("QuickJS runtime");
        rt.set_memory_limit(1024 * 1024 * 1024);
        rt.set_max_stack_size(8 * 1024 * 1024);
        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let d = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || d.get().is_some_and(|t| Instant::now() > t))));
        let (proc_tx, proc_rx) = crossbeam_channel::unbounded();
        let shared = Rc::new(RefCell::new(Shared {
            out,
            inbox,
            proc_tx,
            repaint,
            docs: HashMap::new(),
            active: None,
            workspace: None,
            config,
            timers: Vec::new(),
            procs: HashMap::new(),
            http: HashMap::new(),
            next_id: 1,
            deferred: Vec::new(),
            pending: HashMap::new(),
            names: Vec::new(),
        }));
        Self { rt, exts: Vec::new(), shared, deadline, proc_rx }
    }

    fn run(mut self, rx: Receiver<HostMsg>) {
        loop {
            let next = {
                let sh = self.shared.borrow();
                let t = sh.timers.iter().map(|t| t.0).min();
                let p = sh.pending.values().map(|p| p.deadline).min();
                match (t, p) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            };
            let msg = match next {
                Some(at) => {
                    crossbeam_channel::select! {
                        recv(rx) -> m => m.ok(),
                        recv(self.proc_rx) -> p => p.ok().map(HostMsg::Proc),
                        default(at.saturating_duration_since(Instant::now())) => None,
                    }
                }
                None => {
                    crossbeam_channel::select! {
                        recv(rx) -> m => match m { Ok(m) => Some(m), Err(_) => return },
                        recv(self.proc_rx) -> p => p.ok().map(HostMsg::Proc),
                    }
                }
            };
            match msg {
                Some(HostMsg::Shutdown) => {
                    self.shutdown();
                    return;
                }
                Some(m) => self.handle(m),
                None => {}
            }
            self.fire_timers();
            self.expire_pending();
            self.flush_deferred();
        }
    }

    fn shutdown(&mut self) {
        for i in 0..self.exts.len() {
            if self.exts[i].ctx.is_some() {
                self.dispatch(i, &json!({"type": "deactivate"}));
            }
        }
        let procs: Vec<u32> = self.shared.borrow().procs.values().map(|(_, r)| r.pid).collect();
        for pid in procs {
            process::kill(pid);
        }
    }

    // ---- messages -------------------------------------------------------------

    fn handle(&mut self, msg: HostMsg) {
        match msg {
            HostMsg::Ui(m) => self.handle_ui(m),
            HostMsg::Proc(ev) => {
                let (id, event) = match ev {
                    ProcEvent::Line { id, stderr, line } => {
                        (id, json!({"type": "proc", "id": id, "stderr": stderr, "line": line}))
                    }
                    ProcEvent::Json { id, text } => (id, json!({"type": "procJson", "id": id, "text": text})),
                    ProcEvent::Exit { id, code } => (id, json!({"type": "procExit", "id": id, "code": code})),
                };
                let ext = {
                    let mut sh = self.shared.borrow_mut();
                    let ext = sh.procs.get(&id).map(|p| p.0);
                    if event["type"] == "procExit" {
                        sh.procs.remove(&id);
                    }
                    ext
                };
                if let Some(ext) = ext {
                    self.dispatch(ext, &event);
                }
            }
            HostMsg::Http { id, result } => {
                let ext = self.shared.borrow_mut().http.remove(&id);
                if let Some(ext) = ext {
                    let mut ev = result;
                    ev["type"] = json!("http");
                    ev["id"] = json!(id);
                    self.dispatch(ext, &ev);
                }
            }
            HostMsg::Shutdown => {}
        }
    }

    fn active_indices(&self) -> Vec<usize> {
        (0..self.exts.len()).filter(|&i| self.exts[i].ctx.is_some()).collect()
    }

    fn broadcast(&mut self, event: &Value) {
        for i in self.active_indices() {
            self.dispatch(i, event);
        }
    }

    fn handle_ui(&mut self, msg: ToHost) {
        match msg {
            ToHost::Load(exts) => self.load(exts),
            ToHost::Workspace(path) => {
                {
                    let mut sh = self.shared.borrow_mut();
                    sh.config.lock().unwrap().set_workspace(path.as_ref().map(PathBuf::from));
                    sh.workspace = path.as_ref().map(|p| js_path(Path::new(p)));
                }
                let p = self.shared.borrow().workspace.clone();
                self.broadcast(&json!({"type": "workspace", "path": p}));
                self.activate_matching(|ev| ev.starts_with("onWorkspaceContains:"));
            }
            ToHost::DocOpened(info, text) => {
                let id = info.id;
                let syntax = info.syntax.clone();
                self.shared.borrow_mut().docs.insert(id, Doc { info, text, sels: vec![(0, 0)] });
                if let Some(s) = syntax {
                    let ev = format!("onLanguage:{s}");
                    self.activate_matching(|e| e == ev);
                }
                self.broadcast(&json!({"type": "docOpened", "id": id}));
            }
            ToHost::DocChanged { id, text, dirty } => {
                if let Some(d) = self.shared.borrow_mut().docs.get_mut(&id) {
                    d.text = text;
                    d.info.dirty = dirty;
                }
                self.broadcast(&json!({"type": "docChanged", "id": id}));
            }
            ToHost::Selection { id, ranges } => {
                if let Some(d) = self.shared.borrow_mut().docs.get_mut(&id) {
                    d.sels = ranges;
                }
                self.broadcast(&json!({"type": "selection", "id": id}));
            }
            ToHost::DocMeta(info) => {
                let id = info.id;
                let syntax = info.syntax.clone();
                if let Some(d) = self.shared.borrow_mut().docs.get_mut(&id) {
                    d.info = info;
                }
                if let Some(s) = syntax {
                    let ev = format!("onLanguage:{s}");
                    self.activate_matching(|e| e == ev);
                }
                self.broadcast(&json!({"type": "docMeta", "id": id}));
            }
            ToHost::DocSaved(id) => {
                if let Some(d) = self.shared.borrow_mut().docs.get_mut(&id) {
                    d.info.dirty = false;
                }
                self.broadcast(&json!({"type": "docSaved", "id": id}));
            }
            ToHost::DocClosed(id) => {
                self.broadcast(&json!({"type": "docClosed", "id": id}));
                let mut sh = self.shared.borrow_mut();
                sh.docs.remove(&id);
                if sh.active == Some(id) {
                    sh.active = None;
                }
            }
            ToHost::ActiveDoc(id) => {
                self.shared.borrow_mut().active = id;
                self.broadcast(&json!({"type": "active", "id": id}));
            }
            ToHost::Command { ext, name, editor, args } => {
                let Some(i) = self.exts.iter().position(|e| e.ext.identifier == ext) else { return };
                self.activate(i);
                self.dispatch(i, &json!({"type": "command", "name": name, "editor": editor, "args": args}));
            }
            ToHost::ConfigChanged { workspace, key } => {
                let value = self.shared.borrow().config.lock().unwrap().get(workspace, &key);
                self.broadcast(&json!({"type": "config", "workspace": workspace, "key": key, "value": value, "old": Value::Null}));
            }
            ToHost::Completion { req, doc, pos, trigger } => {
                let pos16 = self.shared.borrow().docs.get(&doc).map(|d| d.to16(pos));
                let Some(pos16) = pos16 else { return };
                self.ask(req, AnswerKind::Completion, ANSWER_TIMEOUT, &json!({"type": "completion", "req": req, "id": doc, "pos": pos16, "trigger": trigger}));
            }
            ToHost::Hover { req, doc, pos } => {
                let pos16 = self.shared.borrow().docs.get(&doc).map(|d| d.to16(pos));
                let Some(pos16) = pos16 else { return };
                self.ask(req, AnswerKind::Hover, ANSWER_TIMEOUT * 2, &json!({"type": "hover", "req": req, "id": doc, "pos": pos16}));
            }
            ToHost::Definition { req, doc, pos } => {
                let pos16 = self.shared.borrow().docs.get(&doc).map(|d| d.to16(pos));
                let Some(pos16) = pos16 else { return };
                self.ask(req, AnswerKind::Definition, ANSWER_TIMEOUT * 2, &json!({"type": "definition", "req": req, "id": doc, "pos": pos16}));
            }
            ToHost::WillSave { req, doc } => {
                self.ask(req, AnswerKind::WillSave, WILL_SAVE_TIMEOUT, &json!({"type": "willSave", "req": req, "id": doc}));
            }
            ToHost::Reply { ext, cb, value } => {
                if let Some(i) = self.exts.iter().position(|e| e.ext.identifier == ext) {
                    self.dispatch(i, &json!({"type": "reply", "cb": cb, "value": value}));
                }
            }
            ToHost::TreeChildren { ext, view, req, element } => {
                if let Some(i) = self.exts.iter().position(|e| e.ext.identifier == ext) {
                    self.dispatch(i, &json!({"type": "treeChildren", "view": view, "req": req, "element": element}));
                }
            }
            ToHost::TreeEvent { ext, view, kind, element } => {
                if let Some(i) = self.exts.iter().position(|e| e.ext.identifier == ext) {
                    self.dispatch(i, &json!({"type": "treeEvent", "view": view, "kind": kind, "element": element}));
                }
            }
        }
    }

    /// Sends a request to all running extensions and collects their answers.
    fn ask(&mut self, req: u64, kind: AnswerKind, timeout: Duration, event: &Value) {
        let targets = self.active_indices();
        self.shared.borrow_mut().pending.insert(
            req,
            Pending { kind, waiting: targets.len(), values: Vec::new(), deadline: Instant::now() + timeout },
        );
        for i in targets {
            self.dispatch(i, event);
        }
        self.finish_if_done(req, false);
    }

    fn answer(&mut self, req: u64, value: Value) {
        if let Some(p) = self.shared.borrow_mut().pending.get_mut(&req) {
            p.waiting = p.waiting.saturating_sub(1);
            p.values.push(value);
        }
        self.finish_if_done(req, false);
    }

    fn finish_if_done(&mut self, req: u64, force: bool) {
        let mut sh = self.shared.borrow_mut();
        let done = sh.pending.get(&req).is_some_and(|p| force || p.waiting == 0);
        if !done {
            return;
        }
        let p = sh.pending.remove(&req).unwrap();
        let msg = match p.kind {
            AnswerKind::Completion => {
                let items = p
                    .values
                    .into_iter()
                    .flat_map(|v| v.as_array().cloned().unwrap_or_default())
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                FromHost::Completions { req, items }
            }
            AnswerKind::Hover => {
                let parts: Vec<String> = p.values.iter().filter_map(|v| v.as_str().map(str::to_string)).filter(|s| !s.trim().is_empty()).collect();
                FromHost::Hover { req, text: (!parts.is_empty()).then(|| parts.join("\n\n---\n\n")) }
            }
            AnswerKind::Definition => {
                let locations = p
                    .values
                    .into_iter()
                    .flat_map(|v| v.as_array().cloned().unwrap_or_default())
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                FromHost::Definition { req, locations }
            }
            AnswerKind::WillSave => FromHost::WillSaveDone { req },
        };
        let _ = sh.out.send(HostEvent { ext: String::new(), msg });
        sh.repaint.request_repaint();
    }

    fn expire_pending(&mut self) {
        let now = Instant::now();
        let expired: Vec<u64> = self.shared.borrow().pending.iter().filter(|(_, p)| p.deadline <= now).map(|(k, _)| *k).collect();
        for req in expired {
            self.finish_if_done(req, true);
        }
    }

    fn fire_timers(&mut self) {
        let now = Instant::now();
        let due: Vec<(usize, u64)> = {
            let mut sh = self.shared.borrow_mut();
            let mut due = Vec::new();
            sh.timers.retain(|&(at, ext, id)| {
                if at <= now {
                    due.push((ext, id));
                    false
                } else {
                    true
                }
            });
            due.sort_by_key(|d| d.1);
            due
        };
        for (ext, id) in due {
            self.dispatch(ext, &json!({"type": "timer", "id": id}));
        }
    }

    fn flush_deferred(&mut self) {
        for _ in 0..8 {
            let events = std::mem::take(&mut self.shared.borrow_mut().deferred);
            if events.is_empty() {
                return;
            }
            for (ext, ev) in events {
                if ev["type"] == "answer" {
                    let req = ev["req"].as_u64().unwrap_or(0);
                    self.answer(req, ev["value"].clone());
                    continue;
                }
                match ext {
                    Some(i) => self.dispatch(i, &ev),
                    None => self.broadcast(&ev),
                }
            }
        }
    }

    // ---- extensions -----------------------------------------------------------

    fn load(&mut self, exts: Vec<Extension>) {
        // Unload extensions that went away or changed.
        let keep: Vec<String> = exts.iter().map(|e| format!("{}@{}", e.identifier, e.version)).collect();
        for i in (0..self.exts.len()).rev() {
            let key = format!("{}@{}", self.exts[i].ext.identifier, self.exts[i].ext.version);
            if !keep.contains(&key) {
                self.deactivate(i);
            }
        }
        let old: Vec<ExtState> = std::mem::take(&mut self.exts);
        let mut old: HashMap<String, ExtState> = old.into_iter().map(|e| (e.ext.identifier.clone(), e)).collect();
        for ext in exts {
            let state = match old.remove(&ext.identifier) {
                Some(s) if s.ext.version == ext.version => ExtState { ext, ..s },
                _ => ExtState { ext, ctx: None, failed: false },
            };
            self.exts.push(state);
        }
        // Contexts are bound to indices; rebuild names and re-register if the order changed.
        let names: Vec<String> = self.exts.iter().map(|e| e.ext.identifier.clone()).collect();
        let reorder = self.shared.borrow().names != names;
        if reorder {
            for i in 0..self.exts.len() {
                if self.exts[i].ctx.is_some() {
                    self.deactivate(i);
                }
            }
        }
        self.shared.borrow_mut().names = names;
        {
            let defaults: Vec<(String, Value)> = self.exts.iter().flat_map(|e| e.ext.config_defaults()).collect();
            self.shared.borrow().config.lock().unwrap().set_defaults(defaults);
        }
        let langs: Vec<String> = self
            .shared
            .borrow()
            .docs
            .values()
            .filter_map(|d| d.info.syntax.clone())
            .map(|s| format!("onLanguage:{s}"))
            .collect();
        self.activate_matching(|e| e == "*" || langs.iter().any(|l| l == e) || e.starts_with("onWorkspaceContains:"));
    }

    fn activate_matching(&mut self, pred: impl Fn(&str) -> bool) {
        for i in 0..self.exts.len() {
            if self.exts[i].ctx.is_some() || self.exts[i].failed || !self.exts[i].ext.has_scripts() {
                continue;
            }
            let events = &self.exts[i].ext.activation;
            let hit = if events.is_empty() {
                true
            } else {
                events.iter().any(|e| {
                    if !pred(e) {
                        return false;
                    }
                    match e.strip_prefix("onWorkspaceContains:") {
                        Some(glob) => self.workspace_contains(glob),
                        None => true,
                    }
                })
            };
            if hit {
                self.activate(i);
            }
        }
    }

    fn workspace_contains(&self, glob: &str) -> bool {
        let Some(root) = self.shared.borrow().workspace.clone() else { return false };
        let root = native_path(&root);
        let Ok(re) = regex::Regex::new(&glob_to_regex(glob)) else { return false };
        let mut stack = vec![root.clone()];
        let mut seen = 0;
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                seen += 1;
                if seen > 5000 {
                    return false;
                }
                let p = e.path();
                let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                if re.is_match(&rel) {
                    return true;
                }
                let name = e.file_name();
                if p.is_dir() && glob.contains("**") && !matches!(name.to_str(), Some("node_modules" | ".git" | "vendor")) {
                    stack.push(p);
                }
            }
        }
        false
    }

    fn activate(&mut self, i: usize) {
        if self.exts[i].ctx.is_some() || self.exts[i].failed {
            return;
        }
        let Some(main) = self.exts[i].ext.main.clone() else { return };
        let ctx = match Context::full(&self.rt) {
            Ok(c) => c,
            Err(e) => {
                self.fail(i, t!("Kontext konnte nicht erstellt werden: {e}", e = e));
                return;
            }
        };
        let ext = &self.exts[i].ext;
        let storage = crate::settings::config_dir().join("extension-storage").join(&ext.identifier);
        let _ = std::fs::create_dir_all(storage.parent().unwrap());
        let ws_storage = crate::settings::config_dir().join("extension-workspace-storage").join(&ext.identifier);
        let _ = std::fs::create_dir_all(ws_storage.parent().unwrap());
        let temp = std::env::temp_dir().join("nebula-extensions").join(&ext.identifier);
        let _ = std::fs::create_dir_all(&temp);
        let info = json!({
            "identifier": ext.identifier,
            "name": ext.name,
            "vendor": ext.organization,
            "version": ext.version,
            "path": js_path(&ext.path),
            "main": js_path(&main),
            "globalStoragePath": js_path(&storage),
            "workspaceStoragePath": js_path(&ws_storage),
            "tempdir": js_path(&temp),
            "strings": ext.strings,
            "entitlements": ext.entitlements,
            "commands": ext.commands.iter().map(|c| json!({"command": c.command, "section": c.section})).collect::<Vec<_>>(),
            "appVersion": env!("CARGO_PKG_VERSION"),
        });
        let shared = self.shared.clone();
        self.deadline.set(Some(Instant::now() + SCRIPT_TIMEOUT));
        let result: Result<(), String> = ctx.with(|ctx| install(ctx.clone(), shared, i, &info).map_err(|e| describe(&ctx, e)));
        self.deadline.set(None);
        match result {
            Ok(()) => {
                self.exts[i].ctx = Some(ctx);
                self.shared.borrow().emit(i, FromHost::Activated);
                self.dispatch(i, &json!({"type": "activate"}));
            }
            Err(e) => self.fail(i, e),
        }
    }

    fn deactivate(&mut self, i: usize) {
        if self.exts[i].ctx.is_some() {
            self.dispatch(i, &json!({"type": "deactivate"}));
        }
        self.exts[i].ctx = None;
        let mut sh = self.shared.borrow_mut();
        let pids: Vec<u64> = sh.procs.iter().filter(|(_, p)| p.0 == i).map(|(k, _)| *k).collect();
        for id in pids {
            if let Some((_, r)) = sh.procs.remove(&id) {
                process::kill(r.pid);
            }
        }
        sh.timers.retain(|t| t.1 != i);
    }

    fn fail(&mut self, i: usize, error: String) {
        self.exts[i].failed = true;
        self.exts[i].ctx = None;
        let sh = self.shared.borrow();
        sh.emit(i, FromHost::Log { level: "error".into(), text: error.clone() });
        sh.emit(i, FromHost::Failed(error));
    }

    /// Delivers an event to an extension's runtime and runs pending jobs.
    fn dispatch(&mut self, i: usize, event: &Value) {
        let Some(ctx) = self.exts[i].ctx.clone() else { return };
        let text = event.to_string();
        self.deadline.set(Some(Instant::now() + SCRIPT_TIMEOUT));
        let result: Result<(), String> = ctx.with(|ctx| {
            let f: Function = ctx.globals().get("__nebula_dispatch").map_err(|e| describe(&ctx, e))?;
            f.call::<_, ()>((text,)).map_err(|e| describe(&ctx, e))
        });
        if let Err(e) = result {
            self.shared.borrow().emit(i, FromHost::Log { level: "error".into(), text: e });
        }
        self.run_jobs();
        self.deadline.set(None);
    }

    fn run_jobs(&mut self) {
        for _ in 0..100_000 {
            match self.rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    let text = e.0.with(|ctx| describe(&ctx, rquickjs::Error::Exception));
                    let _ = self.shared.borrow().out.send(HostEvent { ext: String::new(), msg: FromHost::Log { level: "error".into(), text } });
                }
            }
        }
    }
}

fn glob_to_regex(glob: &str) -> String {
    let mut re = String::from("^");
    let mut chars = glob.trim_start_matches("./").chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    re.push_str("(?:.*/)?");
                } else {
                    re.push_str(".*");
                }
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '{' => re.push_str("(?:"),
            '}' => re.push(')'),
            ',' => re.push('|'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    re
}

/// Text of a pending JavaScript exception.
fn describe(ctx: &Ctx<'_>, err: rquickjs::Error) -> String {
    if !matches!(err, rquickjs::Error::Exception) {
        return err.to_string();
    }
    let v = ctx.catch();
    if let Some(ex) = v.as_exception() {
        let msg = ex.message().unwrap_or_default();
        let stack = ex.stack().unwrap_or_default();
        return if stack.is_empty() { msg } else { format!("{msg}\n{stack}") };
    }
    if let Some(s) = v.as_string() {
        return s.to_string().unwrap_or_default();
    }
    format!("{v:?}")
}

/// Installs the native bridge and runtime into a fresh context and loads
/// the extension's main script.
fn install<'js>(ctx: Ctx<'js>, shared: Rc<RefCell<Shared>>, ext: usize, info: &Value) -> rquickjs::Result<()> {
    let native = Object::new(ctx.clone())?;
    let sh = shared.clone();
    native.set(
        "call",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, args: String| -> rquickjs::Result<String> {
            let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
            match native_call(&sh, ext, &op, &args) {
                Ok(v) => Ok(v.to_string()),
                Err(e) => Err(Exception::throw_message(&ctx, &e)),
            }
        })?,
    )?;
    let sh = shared.clone();
    native.set(
        "post",
        Function::new(ctx.clone(), move |msg: String| {
            let Ok(v) = serde_json::from_str::<Value>(&msg) else { return };
            let mut s = sh.borrow_mut();
            if v["type"] == "answer" {
                s.deferred.push((None, v));
                return;
            }
            match serde_json::from_value::<FromHost>(v) {
                Ok(m) => s.emit(ext, m),
                Err(e) => s.emit(ext, FromHost::Log { level: "warning".into(), text: t!("Unbekannte Nachricht: {e}", e = e) }),
            }
        })?,
    )?;
    native.set(
        "compile",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, src: String, filename: String| -> rquickjs::Result<JsValue<'js>> {
            let wrapped = format!("(function (exports, require, module, __filename, __dirname) {{{}\n}})", src.trim_start_matches("#!"));
            let mut opts = EvalOptions::default();
            opts.strict = false;
            opts.filename = Some(filename);
            ctx.eval_with_options(wrapped, opts)
        })?,
    )?;
    ctx.globals().set("__native", native)?;
    ctx.globals().set("__nebula_info", info.to_string())?;
    let mut opts = EvalOptions::default();
    opts.strict = false;
    opts.filename = Some("nebula-runtime.js".into());
    ctx.eval_with_options::<(), _>(RUNTIME_JS, opts)?;
    let start: Function = ctx.globals().get("__nebula_start")?;
    start.call::<_, ()>(())?;
    Ok(())
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key].as_str().ok_or_else(|| t!("Argument {key} fehlt", key = key))
}

fn arg_u64(args: &Value, key: &str) -> u64 {
    args[key].as_u64().unwrap_or(0)
}

fn system_time(t: std::io::Result<std::time::SystemTime>) -> f64 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn b64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        buf = buf << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

/// Synchronous operations available to the runtime.
fn native_call(shared: &Rc<RefCell<Shared>>, ext: usize, op: &str, args: &Value) -> Result<Value, String> {
    let mut sh = shared.borrow_mut();
    let doc_id = arg_u64(args, "id");
    match op {
        // ---- documents and editors (positions are UTF-16) -----------------------
        "docs" => Ok(json!(sh.docs.keys().collect::<Vec<_>>())),
        "active" => Ok(json!(sh.active)),
        "workspace" => Ok(json!(sh.workspace)),
        "doc.info" => {
            let d = sh.docs.get(&doc_id).ok_or(t!("Dokument ist geschlossen"))?;
            let i = &d.info;
            Ok(json!({
                "uri": i.uri, "path": i.path, "syntax": i.syntax, "remote": i.remote, "dirty": i.dirty,
                "eol": i.eol, "length": d.text.len_utf16_cu(), "tabLength": i.tab_length, "softTabs": i.soft_tabs,
                "selections": d.sels.iter().map(|&(s, e)| [d.to16(s), d.to16(e)]).collect::<Vec<_>>(),
            }))
        }
        "doc.text" => {
            let d = sh.docs.get(&doc_id).ok_or(t!("Dokument ist geschlossen"))?;
            let len = d.text.len_utf16_cu();
            let (s, e) = (arg_u64(args, "start") as usize, args["end"].as_u64().map_or(len, |v| v as usize));
            if s > e || e > len {
                return Err(t!("Bereich {s}–{e} liegt außerhalb des Dokuments ({len})", s = s, e = e, len = len));
            }
            Ok(json!(d.text.slice(d.from16(s)..d.from16(e)).to_string()))
        }
        "doc.lineRange" => {
            let d = sh.docs.get(&doc_id).ok_or(t!("Dokument ist geschlossen"))?;
            let len = d.text.len_utf16_cu();
            let (s, e) = (arg_u64(args, "start") as usize, arg_u64(args, "end") as usize);
            if s > e || e > len {
                return Err(t!("Bereich liegt außerhalb des Dokuments").into());
            }
            let first = d.text.char_to_line(d.from16(s));
            let mut last = d.text.char_to_line(d.from16(e));
            // A range ending at a line start doesn't include that line.
            if e > s && last > first && d.text.line_to_char(last) == d.from16(e) {
                last -= 1;
            }
            let end_char = if last + 1 < d.text.len_lines() { d.text.line_to_char(last + 1) } else { d.text.len_chars() };
            Ok(json!([d.to16(d.text.line_to_char(first)), d.to16(end_char)]))
        }
        "doc.position" => {
            let d = sh.docs.get(&doc_id).ok_or(t!("Dokument ist geschlossen"))?;
            let c = d.from16(arg_u64(args, "offset") as usize);
            let line = d.text.char_to_line(c);
            Ok(json!([line, d.to16(c) - d.to16(d.text.line_to_char(line))]))
        }
        "doc.offset" => {
            let d = sh.docs.get(&doc_id).ok_or(t!("Dokument ist geschlossen"))?;
            let line = (arg_u64(args, "line") as usize).min(d.text.len_lines().saturating_sub(1));
            let start = d.to16(d.text.line_to_char(line));
            let line_end = if line + 1 < d.text.len_lines() { d.to16(d.text.line_to_char(line + 1)) } else { d.text.len_utf16_cu() };
            Ok(json!((start + arg_u64(args, "column") as usize).min(line_end)))
        }
        "editor.select" => {
            let d = sh.docs.get_mut(&doc_id).ok_or(t!("Dokument ist geschlossen"))?;
            let ranges: Vec<[usize; 2]> = serde_json::from_value(args["ranges"].clone()).map_err(|e| e.to_string())?;
            d.sels = ranges.iter().map(|r| (d.from16(r[0]), d.from16(r[1]))).collect();
            sh.emit(ext, FromHost::Select { doc: doc_id, ranges });
            Ok(Value::Null)
        }

        // ---- configuration ----------------------------------------------------------
        "config.get" => {
            let ws = args["workspace"].as_bool().unwrap_or(false);
            Ok(sh.config.lock().unwrap().get(ws, arg_str(args, "key")?))
        }
        "config.section" => Ok(sh.config.lock().unwrap().section(arg_str(args, "section")?)),
        "config.set" => {
            let ws = args["workspace"].as_bool().unwrap_or(false);
            let key = arg_str(args, "key")?.to_string();
            let value = args["value"].clone();
            let old = sh.config.lock().unwrap().set(ws, &key, value);
            let new = sh.config.lock().unwrap().get(ws, &key);
            if new != old {
                sh.deferred.push((None, json!({"type": "config", "workspace": ws, "key": key, "value": new, "old": old})));
            }
            Ok(Value::Null)
        }

        // ---- timers -------------------------------------------------------------------
        "timer.set" => {
            let ms = args["ms"].as_f64().unwrap_or(0.0).max(0.0);
            let at = Instant::now() + Duration::from_micros((ms * 1000.0) as u64);
            sh.timers.push((at, ext, arg_u64(args, "id")));
            Ok(Value::Null)
        }
        "timer.clear" => {
            let id = arg_u64(args, "id");
            sh.timers.retain(|t| !(t.1 == ext && t.2 == id));
            Ok(Value::Null)
        }

        // ---- file system ------------------------------------------------------------
        "fs.stat" => {
            let p = native_path(arg_str(args, "path")?);
            let Ok(m) = std::fs::symlink_metadata(&p) else { return Ok(Value::Null) };
            let link = m.file_type().is_symlink();
            let m = if link { std::fs::metadata(&p).unwrap_or(m) } else { m };
            Ok(json!({
                "size": m.len(), "mtime": system_time(m.modified()), "atime": system_time(m.accessed()),
                "ctime": system_time(m.modified()), "birthtime": system_time(m.created()),
                "mode": if m.permissions().readonly() { 0o444 } else { 0o644 } | if m.is_dir() { 0o40111 } else { 0o100000 },
                "isDirectory": m.is_dir(), "isFile": m.is_file(), "isSymbolicLink": link,
            }))
        }
        "fs.access" => {
            let p = native_path(arg_str(args, "path")?);
            let mode = arg_u64(args, "mode");
            let Ok(m) = std::fs::metadata(&p) else { return Ok(json!(false)) };
            // W_OK = 2, X_OK = 1.
            let writable = mode & 2 == 0 || !m.permissions().readonly();
            let exec = mode & 1 == 0 || m.is_dir() || {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                matches!(ext.as_str(), "exe" | "cmd" | "bat" | "com" | "js" | "") || process::which(&p.to_string_lossy(), &HashMap::new()).is_some()
            };
            Ok(json!(writable && exec))
        }
        "fs.listdir" => {
            let p = native_path(arg_str(args, "path")?);
            let rd = std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            Ok(json!(rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>()))
        }
        "fs.mkdir" => {
            let p = native_path(arg_str(args, "path")?);
            std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "fs.remove" => {
            let p = native_path(arg_str(args, "path")?);
            if p.is_dir() {
                return Err(t!("{p} ist ein Ordner", p = p.display()));
            }
            let _ = std::fs::remove_file(&p);
            Ok(Value::Null)
        }
        "fs.rmdir" => {
            let p = native_path(arg_str(args, "path")?);
            if p.exists() {
                std::fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
            }
            Ok(Value::Null)
        }
        "fs.copy" | "fs.move" => {
            let src = native_path(arg_str(args, "src")?);
            let dest = native_path(arg_str(args, "dest")?);
            if dest.exists() {
                return Err(t!("{dest} existiert bereits", dest = dest.display()));
            }
            if op == "fs.move" {
                std::fs::rename(&src, &dest).map_err(|e| e.to_string())?;
            } else if src.is_dir() {
                copy_dir(&src, &dest).map_err(|e| e.to_string())?;
            } else {
                std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
            }
            Ok(Value::Null)
        }
        "fs.read" => {
            let p = native_path(arg_str(args, "path")?);
            let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            if args["binary"].as_bool().unwrap_or(false) {
                Ok(json!(b64_encode(&bytes)))
            } else {
                Ok(json!(String::from_utf8_lossy(&bytes)))
            }
        }
        "fs.write" => {
            use std::io::Write;
            let p = native_path(arg_str(args, "path")?);
            let data = match args["base64"].as_str() {
                Some(b) => b64_decode(b),
                None => args["text"].as_str().unwrap_or("").as_bytes().to_vec(),
            };
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(args["append"].as_bool().unwrap_or(false))
                .truncate(!args["append"].as_bool().unwrap_or(false))
                .open(&p)
                .map_err(|e| format!("{}: {e}", p.display()))?;
            f.write_all(&data).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "fs.reveal" => {
            let p = native_path(arg_str(args, "path")?);
            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
            Ok(Value::Null)
        }
        "env" => Ok(json!(std::env::vars().collect::<HashMap<_, _>>())),
        "json.lenient" => super::manifest::parse_lenient(arg_str(args, "text")?).ok_or_else(|| t!("Ungültiges JSON").to_string()),

        // ---- processes ----------------------------------------------------------------
        "proc.spawn" => {
            let id = sh.next_id;
            sh.next_id += 1;
            let strings = |v: &Value| -> Vec<String> {
                v.as_array().map(|a| a.iter().map(|s| s.as_str().map(str::to_string).unwrap_or_else(|| s.to_string())).collect()).unwrap_or_default()
            };
            let stdio: Vec<String> = match &args["stdio"] {
                Value::String(s) if s == "jsonrpc" => vec!["pipe".into(); 3],
                Value::String(s) => vec![s.clone(); 3],
                Value::Array(a) => a.iter().map(|v| v.as_str().unwrap_or("pipe").to_string()).collect(),
                _ => vec!["pipe".into(); 3],
            };
            let env: HashMap<String, String> = args["env"]
                .as_object()
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect())
                .unwrap_or_default();
            let cwd = args["cwd"].as_str().map(|c| native_path(c).to_string_lossy().into_owned()).or_else(|| sh.workspace.as_ref().map(|w| native_path(w).to_string_lossy().into_owned()));
            let shell = match &args["shell"] {
                Value::Bool(true) => Some("sh".to_string()),
                Value::String(s) => Some(s.clone()),
                _ => None,
            };
            let spec = Spawn {
                command: arg_str(args, "command")?.to_string(),
                args: strings(&args["args"]),
                env,
                cwd,
                stdio: [stdio.first().cloned().unwrap_or_default(), stdio.get(1).cloned().unwrap_or_default(), stdio.get(2).cloned().unwrap_or_default()],
                jsonrpc: args["stdio"] == "jsonrpc",
                shell,
            };
            // Process events arrive on their own channel, which the host loop selects on.
            let running = process::spawn(id, spec, sh.proc_tx.clone(), || {})?;
            let pid = running.pid;
            sh.procs.insert(id, (ext, running));
            Ok(json!({"id": id, "pid": pid}))
        }
        "proc.write" => {
            use std::io::Write;
            let id = arg_u64(args, "id");
            let (_, p) = sh.procs.get_mut(&id).ok_or(t!("Prozess läuft nicht"))?;
            let stdin = p.stdin.as_mut().ok_or(t!("Standardeingabe ist geschlossen"))?;
            let data = match args["base64"].as_str() {
                Some(b) => b64_decode(b),
                None => args["text"].as_str().unwrap_or("").as_bytes().to_vec(),
            };
            stdin.write_all(&data).and_then(|_| stdin.flush()).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "proc.send" => {
            let id = arg_u64(args, "id");
            let (_, p) = sh.procs.get_mut(&id).ok_or(t!("Prozess läuft nicht"))?;
            let stdin = p.stdin.as_mut().ok_or(t!("Standardeingabe ist geschlossen"))?;
            process::write_jsonrpc(stdin, arg_str(args, "body")?).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "proc.closeStdin" => {
            if let Some((_, p)) = sh.procs.get_mut(&arg_u64(args, "id")) {
                p.stdin = None;
            }
            Ok(Value::Null)
        }
        "proc.kill" => {
            if let Some((_, p)) = sh.procs.get(&arg_u64(args, "id")) {
                process::kill(p.pid);
            }
            Ok(Value::Null)
        }

        // ---- network ------------------------------------------------------------------
        "http.request" => {
            let id = sh.next_id;
            sh.next_id += 1;
            sh.http.insert(id, ext);
            let inbox = sh.inbox.clone();
            let repaint = sh.repaint.clone();
            let req = args.clone();
            std::thread::spawn(move || {
                let result = http_request(&req);
                let _ = inbox.send(HostMsg::Http { id, result });
                repaint.request_repaint();
            });
            Ok(json!(id))
        }

        // ---- system ----------------------------------------------------------------------
        "clipboard.read" => Ok(json!(crate::win32::clipboard_text().unwrap_or_default())),
        "clipboard.write" => {
            crate::win32::set_clipboard_text(arg_str(args, "text")?);
            Ok(Value::Null)
        }
        "credentials.get" => {
            let entry = keyring::Entry::new(arg_str(args, "service")?, arg_str(args, "user")?).map_err(|e| e.to_string())?;
            Ok(entry.get_password().map(Value::String).unwrap_or(Value::Null))
        }
        "credentials.set" => {
            let entry = keyring::Entry::new(arg_str(args, "service")?, arg_str(args, "user")?).map_err(|e| e.to_string())?;
            entry.set_password(arg_str(args, "password")?).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "credentials.remove" => {
            if let Ok(entry) = keyring::Entry::new(arg_str(args, "service")?, arg_str(args, "user")?) {
                let _ = entry.delete_credential();
            }
            Ok(Value::Null)
        }
        "beep" => {
            crate::win32::beep();
            Ok(Value::Null)
        }
        _ => Err(t!("Unbekannte Operation: {op}", op = op)),
    }
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let to = dest.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

/// Performs a fetch() request; the body travels base64-encoded.
fn http_request(req: &Value) -> Value {
    let url = req["url"].as_str().unwrap_or("");
    let method = req["method"].as_str().unwrap_or("GET").to_uppercase();
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .timeout_global(Some(Duration::from_secs(60)))
        .build();
    let agent: ureq::Agent = config.into();
    let mut builder = ureq::http::Request::builder().method(method.as_str()).uri(url);
    if let Some(h) = req["headers"].as_object() {
        for (k, v) in h {
            builder = builder.header(k.as_str(), v.as_str().unwrap_or(""));
        }
    }
    let body = req["body"].as_str().map(b64_decode).unwrap_or_default();
    let request = match builder.body(body) {
        Ok(r) => r,
        Err(e) => return json!({"error": e.to_string()}),
    };
    match agent.run(request) {
        Ok(mut resp) => {
            let status = resp.status();
            let headers: serde_json::Map<String, Value> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), json!(v.to_str().unwrap_or(""))))
                .collect();
            let bytes = resp.body_mut().with_config().limit(256 * 1024 * 1024).read_to_vec().unwrap_or_default();
            json!({
                "status": status.as_u16(),
                "statusText": status.canonical_reason().unwrap_or(""),
                "headers": headers,
                "body": b64_encode(&bytes),
                "url": url,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for s in ["", "a", "ab", "abc", "Grüße 😀"] {
            assert_eq!(b64_decode(&b64_encode(s.as_bytes())), s.as_bytes());
        }
        assert_eq!(b64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn globs() {
        let re = regex::Regex::new(&glob_to_regex("**/*.php")).unwrap();
        assert!(re.is_match("index.php") && re.is_match("a/b/c.php") && !re.is_match("a.phpx"));
        let re = regex::Regex::new(&glob_to_regex("tsconfig.json")).unwrap();
        assert!(re.is_match("tsconfig.json") && !re.is_match("x/tsconfig.json"));
    }
}
