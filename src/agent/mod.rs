//! Nebula's own coding agent for API keys and local models (Anthropic,
//! OpenAI and compatible servers such as Ollama). It speaks the Agent Client
//! Protocol like the external agents, but runs in a thread instead of a Node
//! process, so the agent panel, diffs, permission questions and file access
//! through the open tabs work the same way.

pub mod llm;
pub mod tools;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender};
use serde_json::{Value, json};

use crate::acp::{AgentConfig, Event};
use llm::{Llm, Msg, Part, Provider, Stop};

/// `command` of an agent that runs inside Nebula; `args[0]` is the provider.
pub const COMMAND: &str = "nebula";

/// Providers: id, name, default base URL, needs a key.
pub const PROVIDERS: &[(&str, &str, &str, bool)] = &[
    ("anthropic", tk!("Anthropic (Claude)"), "https://api.anthropic.com", true),
    ("openai", tk!("OpenAI-kompatibel"), "https://api.openai.com/v1", true),
    ("ollama", tk!("Ollama (lokal)"), "http://localhost:11434/v1", false),
];

/// Ready-made built-in agents: name (shown translated), provider.
pub const PRESETS: &[(&str, &str)] =
    &[(tk!("Claude (API-Schlüssel)"), "anthropic"), (tk!("OpenAI (API-Schlüssel)"), "openai"), (tk!("Ollama (lokal)"), "ollama")];

/// Environment entry for a different server address.
pub const BASE_URL: &str = "BASE_URL";

pub fn is_builtin(cfg: &AgentConfig) -> bool {
    cfg.command == COMMAND
}

pub fn provider_of(cfg: &AgentConfig) -> &'static (&'static str, &'static str, &'static str, bool) {
    let id = cfg.args.first().map(String::as_str).unwrap_or("anthropic");
    PROVIDERS.iter().find(|p| p.0 == id).unwrap_or(&PROVIDERS[0])
}

pub fn preset(name: &str, provider: &str) -> AgentConfig {
    AgentConfig { name: name.into(), command: COMMAND.into(), args: vec![provider.into()], ..Default::default() }
}

/// The server address in use (the configured one or the provider's default).
pub fn base_url(cfg: &AgentConfig) -> String {
    let custom = cfg.env.get(BASE_URL).map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty());
    custom.unwrap_or_else(|| provider_of(cfg).2.to_string())
}

/// Starts the agent; it reads JSON-RPC messages from the returned sender and
/// answers into `events`.
/// The model API of a built-in agent.
pub fn llm(cfg: &AgentConfig) -> Llm {
    let (id, _, _, _) = *provider_of(cfg);
    Llm { provider: if id == "anthropic" { Provider::Anthropic } else { Provider::OpenAi }, base: base_url(cfg), key: cfg.api_key() }
}

pub fn start(cfg: AgentConfig, events: Sender<Event>, wake: Arc<dyn Fn() + Send + Sync>) -> Sender<Value> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (id, _, _, _) = *provider_of(&cfg);
    let llm = llm(&cfg);
    // The providers' own servers need a key; other addresses may not.
    let needs_key = provider_of(&cfg).3 && llm.base == provider_of(&cfg).2 && llm.key.is_none();
    let out = Out { events, wake };
    std::thread::spawn(move || Server::new(llm, id == "ollama", needs_key, out).run(rx));
    tx
}

/// Messages to the client (Nebula's agent panel).
#[derive(Clone)]
struct Out {
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Out {
    fn send(&self, msg: Value) {
        let _ = self.events.send(Event::Message(msg));
        (self.wake)();
    }

    fn respond(&self, id: &Value, result: Result<Value, String>) {
        self.send(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": message } }),
        });
    }
}

/// Requests from the agent to the client, answered asynchronously.
#[derive(Clone)]
struct Calls {
    out: Out,
    next: Arc<AtomicI64>,
    waiting: Arc<Mutex<HashMap<i64, Sender<Result<Value, String>>>>>,
}

impl Calls {
    /// Sends a request and waits for the answer.
    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.waiting.lock().unwrap().insert(id, tx);
        self.out.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        rx.recv().unwrap_or_else(|_| Err(t!("Keine Antwort").into()))
    }

    fn answer(&self, id: i64, result: Result<Value, String>) {
        if let Some(tx) = self.waiting.lock().unwrap().remove(&id) {
            let _ = tx.send(result);
        }
    }
}

struct Session {
    id: String,
    cwd: PathBuf,
    model: Mutex<String>,
    history: Mutex<Vec<Msg>>,
    cancel: AtomicBool,
    /// Tool kinds the user allowed for the rest of the session ("edit", "execute").
    always: Mutex<HashSet<&'static str>>,
}

struct Server {
    llm: Arc<Llm>,
    local: bool,
    needs_key: bool,
    out: Out,
    calls: Calls,
    sessions: HashMap<String, Arc<Session>>,
    models: Vec<String>,
}

impl Server {
    fn new(llm: Llm, local: bool, needs_key: bool, out: Out) -> Self {
        let calls = Calls { out: out.clone(), next: Arc::new(AtomicI64::new(1)), waiting: Default::default() };
        Self { llm: Arc::new(llm), local, needs_key, out, calls, sessions: HashMap::new(), models: Vec::new() }
    }

    fn run(mut self, rx: Receiver<Value>) {
        for msg in rx {
            let method = msg.get("method").and_then(Value::as_str).map(str::to_string);
            let id = msg.get("id").cloned();
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            match (method, id) {
                (Some(method), Some(id)) => self.request(&method, &id, params),
                (Some(method), None) => {
                    if method == "session/cancel"
                        && let Some(s) = params["sessionId"].as_str().and_then(|s| self.sessions.get(s))
                    {
                        s.cancel.store(true, Ordering::Relaxed);
                    }
                }
                (None, Some(id)) => {
                    let result = match msg.get("error") {
                        Some(e) => Err(e["message"].as_str().unwrap_or("Fehler").to_string()),
                        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    if let Some(id) = id.as_i64() {
                        self.calls.answer(id, result);
                    }
                }
                (None, None) => {}
            }
        }
        // The client is gone: release waiting tool calls.
        self.calls.waiting.lock().unwrap().clear();
    }

    fn request(&mut self, method: &str, id: &Value, params: Value) {
        match method {
            "initialize" => self.out.respond(
                id,
                Ok(json!({
                    "protocolVersion": 1,
                    "agentCapabilities": { "promptCapabilities": { "embeddedContext": true } },
                    "agentInfo": { "name": "nebula-agent", "title": t!("Nebula-Agent"), "version": env!("CARGO_PKG_VERSION") },
                    "authMethods": [],
                })),
            ),
            "session/new" => {
                let result = self.new_session(&params);
                self.out.respond(id, result);
            }
            "session/set_config_option" => {
                let s = params["sessionId"].as_str().and_then(|s| self.sessions.get(s)).cloned();
                match (s, params["configId"].as_str(), params["value"].as_str()) {
                    (Some(s), Some("model"), Some(value)) => {
                        *s.model.lock().unwrap() = value.to_string();
                        self.out.respond(id, Ok(json!({ "configOptions": self.config_options(value) })));
                    }
                    _ => self.out.respond(id, Err(t!("Unbekannte Einstellung").into())),
                }
            }
            "session/prompt" => match params["sessionId"].as_str().and_then(|s| self.sessions.get(s)).cloned() {
                Some(s) => {
                    s.cancel.store(false, Ordering::Relaxed);
                    let (llm, calls, out, id) = (self.llm.clone(), self.calls.clone(), self.out.clone(), id.clone());
                    let text = prompt_text(&params["prompt"]);
                    // The turn waits for the model and for answers of the user: own thread.
                    std::thread::spawn(move || {
                        let result = Turn { llm: &llm, calls: &calls, session: &s }.run(text);
                        out.respond(&id, result.map(|stop| json!({ "stopReason": stop })));
                    });
                }
                None => self.out.respond(id, Err(t!("Unbekannte Sitzung").into())),
            },
            _ => self.out.send(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {method}") } })),
        }
    }

    fn new_session(&mut self, params: &Value) -> Result<Value, String> {
        if self.needs_key {
            return Err(t!("Kein API-Schlüssel hinterlegt – unter Einstellungen › KI-Agenten bei diesem Agenten eintragen.").into());
        }
        if self.models.is_empty() {
            self.models = self.llm.models().map_err(|e| match e {
                llm::Error::Unreachable(_) if self.local => t!("Ollama antwortet nicht unter {base} – läuft es?", base = self.llm.base),
                e => e.message(&self.llm.base),
            })?;
            if self.models.is_empty() {
                return Err(if self.local { t!("Ollama hat noch keine Modelle – z. B. „ollama pull qwen2.5-coder“ ausführen.").into() } else { t!("Der Server bietet keine Modelle an.").into() });
            }
        }
        let model = default_model(&self.models);
        let id = format!("s{}", self.sessions.len() + 1);
        let cwd = PathBuf::from(params["cwd"].as_str().unwrap_or("."));
        let session = Session {
            id: id.clone(),
            cwd,
            model: Mutex::new(model.clone()),
            history: Mutex::new(Vec::new()),
            cancel: AtomicBool::new(false),
            always: Mutex::new(HashSet::new()),
        };
        self.sessions.insert(id.clone(), Arc::new(session));
        Ok(json!({ "sessionId": id, "configOptions": self.config_options(&model) }))
    }

    fn config_options(&self, current: &str) -> Value {
        let options: Vec<Value> = self.models.iter().map(|m| json!({ "value": m, "name": m })).collect();
        json!([{ "id": "model", "name": t!("Modell"), "category": "model", "type": "select", "currentValue": current, "options": options }])
    }
}

/// A sensible first choice among the offered models.
fn default_model(models: &[String]) -> String {
    const PREFERRED: &[&str] = &["claude-sonnet-4", "claude-sonnet", "claude-opus", "gpt-5", "gpt-4.1", "gpt-4o", "coder", "qwen", "llama"];
    for p in PREFERRED {
        // Newest first: the ids carry dates or version numbers.
        if let Some(m) = models.iter().rev().find(|m| m.contains(p) && !m.contains("mini") && !m.contains("nano")) {
            return m.clone();
        }
    }
    models.first().cloned().unwrap_or_default()
}

/// The user's message: text plus attached files.
fn prompt_text(blocks: &Value) -> String {
    let mut parts = Vec::new();
    for b in blocks.as_array().into_iter().flatten() {
        match b["type"].as_str() {
            Some("text") => parts.push(b["text"].as_str().unwrap_or("").to_string()),
            Some("resource") => {
                let r = &b["resource"];
                let name = uri_path(r["uri"].as_str().unwrap_or(""));
                parts.push(format!("Attached file {name}:\n```\n{}\n```", r["text"].as_str().unwrap_or("")));
            }
            Some("resource_link") => parts.push(format!("Attached file: {}", uri_path(b["uri"].as_str().unwrap_or("")))),
            _ => {}
        }
    }
    parts.join("\n\n")
}

/// `file:///C:/a%20b.txt` → `C:\a b.txt` (other URIs unchanged).
fn uri_path(uri: &str) -> String {
    match uri.strip_prefix("file:///") {
        Some(p) => percent_decode(p).replace('/', "\\"),
        None => percent_decode(uri),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok())
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Most model requests per prompt (tool calls in between).
const MAX_STEPS: usize = 50;
/// Lines of a file sent to the model without `limit`.
const READ_LINES: usize = 2000;

fn system_prompt(cwd: &Path) -> String {
    format!(
        "You are the coding assistant built into Nebula, a code editor for web developers on Windows. \
         The project folder is {cwd}; relative paths refer to it. Look at files with the tools before changing them. \
         For changes prefer edit_file with an exact, unique old_string; use write_file for new files or complete rewrites. \
         Commands run in cmd.exe in the project folder. The user sees every change as a diff and every command, and approves them. \
         Work step by step, keep explanations short, and answer in the language the user writes in.",
        cwd = cwd.display()
    )
}

/// One prompt: model requests and tool calls until the model is done.
struct Turn<'a> {
    llm: &'a Llm,
    calls: &'a Calls,
    session: &'a Session,
}

impl Turn<'_> {
    fn cancelled(&self) -> bool {
        self.session.cancel.load(Ordering::Relaxed)
    }

    fn update(&self, update: Value) {
        self.calls.out.send(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": self.session.id, "update": update } }));
    }

    fn run(&self, text: String) -> Result<&'static str, String> {
        let mut history = self.session.history.lock().unwrap().clone();
        history.push(Msg { user: true, parts: vec![Part::Text(text)] });
        let result = self.steps(&mut history);
        // Keep user and assistant alternating for the next prompt.
        if history.last().is_some_and(|m| m.user) {
            let note = match &result {
                Ok(_) => "[stopped]".to_string(),
                Err(e) => format!("[error: {e}]"),
            };
            history.push(Msg { user: false, parts: vec![Part::Text(note)] });
        }
        *self.session.history.lock().unwrap() = history;
        result
    }

    fn steps(&self, history: &mut Vec<Msg>) -> Result<&'static str, String> {
        let tools = tools::definitions();
        let system = system_prompt(&self.session.cwd);
        for _ in 0..MAX_STEPS {
            if self.cancelled() {
                return Ok("cancelled");
            }
            let model = self.session.model.lock().unwrap().clone();
            let mut on_text = |t: &str| self.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": t } }));
            let turn = match self.llm.complete(&model, &system, history, &tools, &mut on_text, &self.session.cancel) {
                Ok(turn) => turn,
                Err(e) if e == llm::CANCELLED => return Ok("cancelled"),
                Err(e) => return Err(e),
            };
            let mut parts = Vec::new();
            if !turn.text.is_empty() {
                parts.push(Part::Text(turn.text.clone()));
            }
            for (id, name, input) in &turn.calls {
                parts.push(Part::ToolUse { id: id.clone(), name: name.clone(), input: input.clone() });
            }
            if parts.is_empty() {
                parts.push(Part::Text(String::new()));
            }
            history.push(Msg { user: false, parts });
            if turn.calls.is_empty() {
                return Ok(if turn.stop == Some(Stop::MaxTokens) { "max_tokens" } else { "end_turn" });
            }
            // Separate the next text from this one in the panel.
            if !turn.text.is_empty() && !turn.text.ends_with('\n') {
                self.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "\n\n" } }));
            }
            let mut results = Vec::new();
            for (id, name, input) in &turn.calls {
                let (content, error) = if self.cancelled() {
                    ("Cancelled by the user.".to_string(), true)
                } else {
                    match self.tool(id, name, input) {
                        Ok(text) => (text, false),
                        Err(text) => (text, true),
                    }
                };
                results.push(Part::ToolResult { id: id.clone(), content, error });
            }
            history.push(Msg { user: true, parts: results });
        }
        Ok("max_turn_requests")
    }

    /// Reports a tool call to the panel.
    /// (The kind is shown as a label, so the title is just the path, pattern or command.)
    fn tool_call(&self, id: &str, title: String, kind: &str, status: &str, content: Vec<Value>) {
        self.update(json!({ "sessionUpdate": "tool_call", "toolCallId": id, "title": title, "kind": kind, "status": status, "content": content }));
    }

    fn tool_done(&self, id: &str, ok: bool, content: Option<Vec<Value>>) {
        let mut update = json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": if ok { "completed" } else { "failed" } });
        if let Some(c) = content {
            update["content"] = json!(c);
        }
        self.update(update);
    }

    fn tool(&self, id: &str, name: &str, input: &Value) -> Result<String, String> {
        let cwd = &self.session.cwd;
        let path = input["path"].as_str().map(|p| tools::resolve(cwd, p));
        let shown = path.as_ref().map(|p| tools::display(cwd, p)).unwrap_or_default();
        let result = match name {
            "read_file" => {
                let path = path.ok_or("path is missing")?;
                self.tool_call(id, shown.clone(), "read", "in_progress", vec![]);
                let (offset, limit) = (input["offset"].as_u64(), input["limit"].as_u64());
                self.read(&path).map(|text| {
                    let total = text.lines().count();
                    let start = offset.map_or(0, |o| o.saturating_sub(1) as usize);
                    let take = limit.map_or(READ_LINES, |l| l as usize);
                    let mut out: String = text.split_inclusive('\n').skip(start).take(take).collect();
                    if start + take < total {
                        out.push_str(&format!("\n… ({total} lines in total; read further with offset {})", start + take + 1));
                    }
                    if out.is_empty() { "(empty file)".into() } else { out }
                })
            }
            "list_files" => {
                let dir = input["path"].as_str().unwrap_or(".");
                self.tool_call(id, dir.to_string(), "search", "in_progress", vec![]);
                tools::list_files(cwd, input)
            }
            "search" => {
                let pattern = input["pattern"].as_str().unwrap_or("");
                self.tool_call(id, pattern.to_string(), "search", "in_progress", vec![]);
                tools::search(cwd, input).map(|(text, n)| {
                    self.tool_done(id, true, Some(vec![text_content(&t!("{n} Treffer", n = n))]));
                    text
                })
            }
            "edit_file" | "write_file" => {
                let path = path.ok_or("path is missing")?;
                let old = self.read(&path).ok();
                let new = if name == "edit_file" {
                    let old = old.as_deref().ok_or_else(|| format!("{} does not exist; use write_file to create it.", path.display()))?;
                    let replaced = tools::replace(old, input["old_string"].as_str().unwrap_or(""), input["new_string"].as_str().unwrap_or(""), input["replace_all"].as_bool().unwrap_or(false));
                    match replaced {
                        Ok(new) => new,
                        Err(e) => {
                            self.tool_call(id, shown.clone(), "edit", "failed", vec![text_content(&e)]);
                            return Err(e);
                        }
                    }
                } else {
                    input["content"].as_str().ok_or("content is missing")?.to_string()
                };
                let title = if old.is_none() { t!("{shown} (neu)", shown = shown) } else { shown.clone() };
                let diff = json!({ "type": "diff", "path": path, "oldText": old, "newText": new });
                self.tool_call(id, title, "edit", "pending", vec![diff]);
                if !self.allowed(id, "edit")? {
                    self.tool_done(id, false, None);
                    return Err("The user rejected this change. Ask what they want instead.".into());
                }
                match self.calls.request("fs/write_text_file", json!({ "sessionId": self.session.id, "path": path, "content": new })) {
                    Ok(_) => Ok(format!("{shown} saved.")),
                    Err(e) => {
                        self.tool_done(id, false, Some(vec![text_content(&e)]));
                        Err(e)
                    }
                }
            }
            "run_command" => {
                let command = input["command"].as_str().unwrap_or("").to_string();
                self.tool_call(id, command.clone(), "execute", "pending", vec![text_content(&format!("> {command}"))]);
                if !self.allowed(id, "execute")? {
                    self.tool_done(id, false, None);
                    return Err("The user rejected this command.".into());
                }
                self.update(json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": "in_progress" }));
                match tools::run_command(cwd, &command, &self.session.cancel) {
                    Ok((code, output)) => {
                        let shown = format!("> {command}\n{}", output.chars().take(4000).collect::<String>());
                        self.tool_done(id, code == Some(0), Some(vec![text_content(&shown)]));
                        let code = code.map_or("none".to_string(), |c| c.to_string());
                        Ok(format!("Exit code {code}\n{output}"))
                    }
                    Err(e) => {
                        self.tool_done(id, false, Some(vec![text_content(&e)]));
                        Err(e)
                    }
                }
            }
            other => Err(format!("Unknown tool {other}")),
        };
        // Edits and commands report their outcome themselves (the diff stays visible).
        match &result {
            Ok(_) if matches!(name, "read_file" | "list_files" | "edit_file" | "write_file") => self.tool_done(id, true, None),
            Err(e) if matches!(name, "read_file" | "list_files" | "search") => self.tool_done(id, false, Some(vec![text_content(e)])),
            _ => {}
        }
        result
    }

    /// Text of a file through Nebula (open tabs have their unsaved text).
    fn read(&self, path: &Path) -> Result<String, String> {
        let r = self.calls.request("fs/read_text_file", json!({ "sessionId": self.session.id, "path": path }))?;
        Ok(r["content"].as_str().unwrap_or("").to_string())
    }

    /// Asks the user (unless allowed for the session already).
    fn allowed(&self, id: &str, kind: &'static str) -> Result<bool, String> {
        if self.session.always.lock().unwrap().contains(kind) {
            return Ok(true);
        }
        let (once, always) = if kind == "edit" { (t!("Übernehmen"), t!("Immer übernehmen")) } else { (t!("Ausführen"), t!("Immer ausführen")) };
        let options = json!([
            { "optionId": "allow", "name": once, "kind": "allow_once" },
            { "optionId": "always", "name": always, "kind": "allow_always" },
            { "optionId": "reject", "name": t!("Ablehnen"), "kind": "reject_once" },
        ]);
        let answer = self.calls.request(
            "session/request_permission",
            json!({ "sessionId": self.session.id, "toolCall": { "toolCallId": id }, "options": options }),
        )?;
        match answer.pointer("/outcome/optionId").and_then(Value::as_str) {
            Some("always") => {
                self.session.always.lock().unwrap().insert(kind);
                Ok(true)
            }
            Some("allow") => Ok(true),
            _ => {
                if answer.pointer("/outcome/outcome").and_then(Value::as_str) == Some("cancelled") {
                    self.session.cancel.store(true, Ordering::Relaxed);
                }
                Ok(false)
            }
        }
    }
}

fn text_content(text: &str) -> Value {
    json!({ "type": "content", "content": { "type": "text", "text": text } })
}

#[cfg(test)]
mod tests;
