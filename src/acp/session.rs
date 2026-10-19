//! One conversation with an agent: protocol handshake, the transcript built
//! from `session/update` notifications, permission questions and file
//! requests that the app answers (it knows the open buffers).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::schema::{
    self, ContentBlock, InitializeResponse, NewSessionResponse, PermissionOption, PromptResponse, ReadTextFileRequest,
    RequestPermissionRequest, SessionModeState, SessionNotification, SessionUpdate, ToolCallContent, ToolCallStatus,
    ToolKind, WriteTextFileRequest,
};
use super::{AgentConfig, Connection, Event};

/// Where the session stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Starting,
    /// The agent wants a login first.
    NeedsAuth(Vec<AuthOption>),
    Ready,
    /// A prompt is running.
    Working,
    Failed(String),
    Exited,
}

#[derive(Clone, Debug)]
pub struct ToolEntry {
    pub id: String,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolCallStatus,
    pub content: Vec<ToolCallContent>,
    pub locations: Vec<(PathBuf, Option<u32>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanItem {
    pub text: String,
    pub done: bool,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub enum Entry {
    User(String),
    Agent(String),
    Thought(String),
    Tool(ToolEntry),
    Plan(Vec<PlanItem>),
    /// Something the user should know (stop reasons, errors).
    Notice { text: String, error: bool },
}

/// A way to log in offered by the agent.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Login runs interactively in a console window.
    pub terminal: Option<TerminalLogin>,
    /// The method needs an API key (entered in the settings, passed as environment variable).
    pub api_key: bool,
}

/// An interactive login: `command` (else the agent's own command and arguments) plus `args`.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalLogin {
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// A question of the agent before it runs a tool.
#[derive(Clone, Debug)]
pub struct Permission {
    rpc_id: Value,
    pub tool: ToolEntry,
    pub options: Vec<PermissionOption>,
}

/// File access the app has to carry out and answer with `reply`.
#[derive(Debug)]
pub enum FileRequest {
    Read { rpc_id: Value, path: PathBuf, line: Option<u32>, limit: Option<u32> },
    Write { rpc_id: Value, path: PathBuf, content: String },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    Initialize,
    Authenticate,
    Logout,
    NewSession,
    Prompt,
    SetMode,
    /// Hidden session for `/usage` and its prompt.
    UsageSession,
    UsagePrompt,
}

pub struct AgentSession {
    pub agent: AgentConfig,
    pub cwd: PathBuf,
    conn: Option<Connection>,
    pub phase: Phase,
    session_id: Option<String>,
    pub entries: Vec<Entry>,
    pub permissions: VecDeque<Permission>,
    pub modes: Option<SessionModeState>,
    /// Name and version reported by the agent.
    pub agent_info: Option<String>,
    /// The agent accepts file contents embedded in the prompt.
    embedded_context: bool,
    pub auth_methods: Vec<AuthOption>,
    /// The agent can log out (`logout` method).
    pub can_logout: bool,
    pending: HashMap<i64, Pending>,
    /// Last lines of the agent's log output (shown when starting fails).
    pub log: VecDeque<String>,
    /// Plan limits (5 hours, week) if the agent reports them.
    pub limits: Option<super::usage::Limits>,
    has_usage: bool,
    usage_session: Option<String>,
    usage_text: String,
}

impl AgentSession {
    /// Starts the agent process and the handshake.
    pub fn start(agent: AgentConfig, cwd: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let mut s = Self {
            agent,
            cwd,
            conn: None,
            phase: Phase::Starting,
            session_id: None,
            entries: Vec::new(),
            permissions: VecDeque::new(),
            modes: None,
            agent_info: None,
            embedded_context: false,
            auth_methods: Vec::new(),
            can_logout: false,
            pending: HashMap::new(),
            log: VecDeque::new(),
            limits: None,
            has_usage: false,
            usage_session: None,
            usage_text: String::new(),
        };
        match Connection::spawn(&s.agent, &s.cwd, wake) {
            Ok(conn) => {
                s.conn = Some(conn);
                s.send(
                    "initialize",
                    Pending::Initialize,
                    json!({
                        "protocolVersion": 1,
                        "clientCapabilities": {
                            "fs": { "readTextFile": true, "writeTextFile": true },
                            "terminal": false,
                            // Interactive logins (e.g. Claude) are offered only to clients that can run them.
                            "_meta": { "terminal-auth": true },
                        },
                        "clientInfo": { "name": "nebula", "title": "Nebula", "version": env!("CARGO_PKG_VERSION") },
                    }),
                );
            }
            Err(e) => s.phase = Phase::Failed(e),
        }
        s
    }

    fn send(&mut self, method: &str, kind: Pending, params: Value) {
        if let Some(conn) = &mut self.conn {
            let id = conn.request(method, params);
            self.pending.insert(id, kind);
        }
    }

    pub fn is_working(&self) -> bool {
        self.phase == Phase::Working
    }

    /// Handles everything the agent sent; returns file requests for the app.
    pub fn poll(&mut self) -> Vec<FileRequest> {
        let Some(conn) = &self.conn else { return Vec::new() };
        let events: Vec<Event> = conn.rx.try_iter().collect();
        let mut files = Vec::new();
        for event in events {
            match event {
                Event::Message(msg) => self.handle(msg, &mut files),
                Event::Stderr(line) => {
                    self.log.push_back(line);
                    if self.log.len() > 40 {
                        self.log.pop_front();
                    }
                }
                Event::Exit => {
                    self.permissions.clear();
                    if !matches!(self.phase, Phase::Failed(_)) {
                        let tail: Vec<&str> = self.log.iter().rev().take(6).map(String::as_str).collect::<Vec<_>>().into_iter().rev().collect();
                        let reason = if tail.is_empty() { String::new() } else { format!("\n{}", tail.join("\n")) };
                        if self.session_id.is_none() {
                            self.phase = Phase::Failed(format!("Der Agent wurde beendet.{reason}"));
                        } else {
                            self.phase = Phase::Exited;
                            self.notice(format!("Der Agent wurde beendet.{reason}"), true);
                        }
                    }
                    self.conn = None;
                    break;
                }
            }
        }
        files
    }

    fn handle(&mut self, msg: Value, files: &mut Vec<FileRequest>) {
        let method = msg.get("method").and_then(Value::as_str).map(str::to_string);
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            // Request from the agent.
            (Some(method), Some(id)) => match method.as_str() {
                "session/request_permission" => match serde_json::from_value::<RequestPermissionRequest>(params) {
                    Ok(req) => {
                        let mut tool = self.tool(&req.tool_call.tool_call_id.to_string()).cloned().unwrap_or_else(|| ToolEntry {
                            id: req.tool_call.tool_call_id.to_string(),
                            title: String::new(),
                            kind: ToolKind::Other,
                            status: ToolCallStatus::Pending,
                            content: Vec::new(),
                            locations: Vec::new(),
                        });
                        apply_fields(&mut tool, &req.tool_call.fields);
                        self.permissions.push_back(Permission { rpc_id: id, tool, options: req.options });
                    }
                    Err(e) => self.reply_error(&id, -32602, &e.to_string()),
                },
                "fs/read_text_file" => match serde_json::from_value::<ReadTextFileRequest>(params) {
                    Ok(r) => files.push(FileRequest::Read { rpc_id: id, path: r.path, line: r.line, limit: r.limit }),
                    Err(e) => self.reply_error(&id, -32602, &e.to_string()),
                },
                "fs/write_text_file" => match serde_json::from_value::<WriteTextFileRequest>(params) {
                    Ok(r) => files.push(FileRequest::Write { rpc_id: id, path: r.path, content: r.content }),
                    Err(e) => self.reply_error(&id, -32602, &e.to_string()),
                },
                _ => self.reply_error(&id, -32601, &format!("Methode nicht unterstützt: {method}")),
            },
            // Notification.
            (Some(method), None) => {
                if method == "session/update"
                    && let Ok(n) = serde_json::from_value::<SessionNotification>(params)
                {
                    // The hidden `/usage` session only feeds the limits.
                    if self.usage_session.as_deref() == Some(&*n.session_id.to_string()) {
                        if let SessionUpdate::AgentMessageChunk(c) = &n.update {
                            self.usage_text.push_str(&text_of(&c.content));
                        }
                        return;
                    }
                    self.apply_update(n.update);
                }
            }
            // Response to one of our requests.
            (None, Some(id)) => {
                let Some(kind) = id.as_i64().and_then(|n| self.pending.remove(&n)) else { return };
                match msg.get("error") {
                    Some(err) => self.request_failed(kind, err),
                    None => self.request_done(kind, msg.get("result").cloned().unwrap_or(Value::Null)),
                }
            }
            (None, None) => {}
        }
    }

    fn request_done(&mut self, kind: Pending, result: Value) {
        match kind {
            Pending::Initialize => {
                self.auth_methods =
                    result.get("authMethods").and_then(Value::as_array).map(|a| a.iter().filter_map(auth_option).collect()).unwrap_or_default();
                self.can_logout = result.pointer("/agentCapabilities/auth/logout").is_some();
                let resp: Option<InitializeResponse> = serde_json::from_value(result).ok();
                if let Some(info) = resp.as_ref().and_then(|r| r.agent_info.as_ref()) {
                    self.agent_info = Some(format!("{} {}", info.title.as_deref().unwrap_or(&info.name), info.version));
                    // Only Claude's adapter answers `/usage` locally; elsewhere it would be a real prompt.
                    self.has_usage = info.name.contains("claude-agent-acp");
                }
                self.embedded_context = resp.as_ref().is_some_and(|r| r.agent_capabilities.prompt_capabilities.embedded_context);
                self.new_session();
            }
            Pending::Authenticate => self.new_session(),
            Pending::NewSession => match serde_json::from_value::<NewSessionResponse>(result) {
                Ok(r) => {
                    self.session_id = Some(r.session_id.to_string());
                    self.modes = r.modes;
                    self.phase = Phase::Ready;
                    self.refresh_limits();
                }
                Err(e) => self.phase = Phase::Failed(format!("Ungültige Antwort auf session/new: {e}")),
            },
            Pending::Prompt => {
                self.phase = Phase::Ready;
                self.permissions.clear();
                if let Ok(r) = serde_json::from_value::<PromptResponse>(result) {
                    let note = match r.stop_reason {
                        schema::StopReason::EndTurn => None,
                        schema::StopReason::Cancelled => Some("Abgebrochen."),
                        schema::StopReason::MaxTokens => Some("Die Antwort wurde wegen der Längenbegrenzung abgeschnitten."),
                        schema::StopReason::MaxTurnRequests => Some("Der Agent hat die maximale Anzahl an Schritten erreicht."),
                        schema::StopReason::Refusal => Some("Der Agent hat die Anfrage abgelehnt."),
                        _ => None,
                    };
                    if let Some(note) = note {
                        self.notice(note.to_string(), false);
                    }
                }
                // A turn used up some of the limits; not more often than every 20 s.
                if self.limits.as_ref().is_none_or(|l| super::usage::now() - l.fetched >= 20) {
                    self.refresh_limits();
                }
            }
            Pending::UsageSession => {
                self.usage_session = result.get("sessionId").and_then(Value::as_str).map(str::to_string);
                self.ask_usage();
            }
            Pending::UsagePrompt => {
                if let Some(l) = super::usage::parse(&self.usage_text, super::usage::now()) {
                    self.limits = Some(l);
                }
                self.usage_text.clear();
            }
            Pending::SetMode => {}
            Pending::Logout => {
                self.phase = Phase::NeedsAuth(self.auth_methods.clone());
                self.session_id = None;
            }
        }
    }

    fn request_failed(&mut self, kind: Pending, err: &Value) {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = err.get("message").and_then(Value::as_str).unwrap_or("Unbekannter Fehler").to_string();
        match kind {
            // -32000: authentication required.
            Pending::NewSession if code == -32000 => self.phase = Phase::NeedsAuth(self.auth_methods.clone()),
            Pending::Initialize | Pending::NewSession | Pending::Authenticate => {
                self.phase = Phase::Failed(message);
            }
            Pending::Prompt => {
                self.phase = Phase::Ready;
                self.permissions.clear();
                if code == -32000 {
                    self.phase = Phase::NeedsAuth(self.auth_methods.clone());
                }
                self.notice(message, true);
            }
            Pending::SetMode | Pending::Logout => self.notice(message, true),
            // The limits are extra information: no error shown.
            Pending::UsageSession | Pending::UsagePrompt => self.usage_text.clear(),
        }
    }

    /// Reads the plan limits again (Claude only, costs no tokens).
    pub fn refresh_limits(&mut self) {
        let busy = self.pending.values().any(|p| matches!(p, Pending::UsageSession | Pending::UsagePrompt));
        if !self.has_usage || busy || self.session_id.is_none() {
            return;
        }
        match self.usage_session {
            Some(_) => self.ask_usage(),
            None => {
                let cwd = self.cwd.to_string_lossy().into_owned();
                self.send("session/new", Pending::UsageSession, json!({ "cwd": cwd, "mcpServers": [] }));
            }
        }
    }

    fn ask_usage(&mut self) {
        let Some(id) = self.usage_session.clone() else { return };
        self.usage_text.clear();
        self.send("session/prompt", Pending::UsagePrompt, json!({ "sessionId": id, "prompt": [{ "type": "text", "text": "/usage" }] }));
    }


    fn new_session(&mut self) {
        let cwd = self.cwd.to_string_lossy().into_owned();
        self.send("session/new", Pending::NewSession, json!({ "cwd": cwd, "mcpServers": [] }));
    }

    /// Logs in with one of the methods offered by the agent.
    pub fn authenticate(&mut self, method_id: &str) {
        self.phase = Phase::Starting;
        self.send("authenticate", Pending::Authenticate, json!({ "methodId": method_id }));
    }

    /// A login started with `authenticate` is still running (e.g. in the browser).
    pub fn is_authenticating(&self) -> bool {
        self.pending.values().any(|p| *p == Pending::Authenticate)
    }

    /// Logs out of the agent's account.
    pub fn logout(&mut self) {
        self.send("logout", Pending::Logout, json!({}));
    }

    /// Sends a message; `context` are files given along (path and current text).
    pub fn prompt(&mut self, text: &str, context: &[(PathBuf, String)]) {
        let Some(session_id) = self.session_id.clone() else { return };
        let mut blocks = vec![json!({ "type": "text", "text": text })];
        for (path, content) in context {
            let uri = file_uri(path);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if self.embedded_context {
                blocks.push(json!({ "type": "resource", "resource": { "uri": uri, "text": content } }));
            } else {
                blocks.push(json!({ "type": "resource_link", "uri": uri, "name": name }));
            }
        }
        let mut shown = text.to_string();
        for (path, _) in context {
            shown.push_str(&format!("\n📎 {}", path.display()));
        }
        self.entries.push(Entry::User(shown));
        self.phase = Phase::Working;
        self.send("session/prompt", Pending::Prompt, json!({ "sessionId": session_id, "prompt": blocks }));
    }

    /// Stops the running prompt; open permission questions count as cancelled.
    pub fn cancel(&mut self) {
        let Some(session_id) = self.session_id.clone() else { return };
        while let Some(p) = self.permissions.pop_front() {
            self.reply(&p.rpc_id, Ok(json!({ "outcome": { "outcome": "cancelled" } })));
        }
        if let Some(conn) = &self.conn {
            conn.notify("session/cancel", json!({ "sessionId": session_id }));
        }
    }

    /// Answers the oldest permission question (`None`: cancelled).
    pub fn answer_permission(&mut self, option_id: Option<&str>) {
        let Some(p) = self.permissions.pop_front() else { return };
        let outcome = match option_id {
            Some(id) => json!({ "outcome": "selected", "optionId": id }),
            None => json!({ "outcome": "cancelled" }),
        };
        self.reply(&p.rpc_id, Ok(json!({ "outcome": outcome })));
    }

    pub fn set_mode(&mut self, mode_id: &str) {
        let Some(session_id) = self.session_id.clone() else { return };
        if let Some(m) = &mut self.modes {
            m.current_mode_id = mode_id.to_string().into();
        }
        self.send("session/set_mode", Pending::SetMode, json!({ "sessionId": session_id, "modeId": mode_id }));
    }

    /// Answers a file request (read: the text, write: null).
    pub fn reply_file(&mut self, rpc_id: &Value, result: Result<Option<String>, String>) {
        let result = result.map(|content| match content {
            Some(content) => json!({ "content": content }),
            None => Value::Null,
        });
        self.reply(rpc_id, result);
    }

    fn reply(&self, id: &Value, result: Result<Value, String>) {
        if let Some(conn) = &self.conn {
            conn.respond(id, result);
        }
    }

    fn reply_error(&self, id: &Value, code: i64, message: &str) {
        if let Some(conn) = &self.conn {
            conn.respond_error(id, code, message);
        }
    }

    fn notice(&mut self, text: String, error: bool) {
        self.entries.push(Entry::Notice { text, error });
    }

    fn tool(&self, id: &str) -> Option<&ToolEntry> {
        self.entries.iter().rev().find_map(|e| match e {
            Entry::Tool(t) if t.id == id => Some(t),
            _ => None,
        })
    }

    fn tool_mut(&mut self, id: &str) -> Option<&mut ToolEntry> {
        self.entries.iter_mut().rev().find_map(|e| match e {
            Entry::Tool(t) if t.id == id => Some(t),
            _ => None,
        })
    }

    fn apply_update(&mut self, update: SessionUpdate) {
        match update {
            SessionUpdate::UserMessageChunk(c) => self.append_text(text_of(&c.content), |s| Entry::User(s), |e| match e {
                Entry::User(s) => Some(s),
                _ => None,
            }),
            SessionUpdate::AgentMessageChunk(c) => self.append_text(text_of(&c.content), Entry::Agent, |e| match e {
                Entry::Agent(s) => Some(s),
                _ => None,
            }),
            SessionUpdate::AgentThoughtChunk(c) => self.append_text(text_of(&c.content), Entry::Thought, |e| match e {
                Entry::Thought(s) => Some(s),
                _ => None,
            }),
            SessionUpdate::ToolCall(t) => {
                let entry = ToolEntry {
                    id: t.tool_call_id.to_string(),
                    title: t.title,
                    kind: t.kind,
                    status: t.status,
                    content: t.content,
                    locations: t.locations.into_iter().map(|l| (l.path, l.line)).collect(),
                };
                match self.tool_mut(&entry.id) {
                    Some(existing) => *existing = entry,
                    None => self.entries.push(Entry::Tool(entry)),
                }
            }
            SessionUpdate::ToolCallUpdate(u) => {
                let id = u.tool_call_id.to_string();
                if let Some(t) = self.tool_mut(&id) {
                    apply_fields(t, &u.fields);
                } else {
                    let mut t = ToolEntry {
                        id,
                        title: String::new(),
                        kind: ToolKind::Other,
                        status: ToolCallStatus::Pending,
                        content: Vec::new(),
                        locations: Vec::new(),
                    };
                    apply_fields(&mut t, &u.fields);
                    self.entries.push(Entry::Tool(t));
                }
            }
            SessionUpdate::Plan(p) => {
                let items: Vec<PlanItem> = p
                    .entries
                    .iter()
                    .map(|e| PlanItem {
                        text: e.content.clone(),
                        done: matches!(e.status, schema::PlanEntryStatus::Completed),
                        active: matches!(e.status, schema::PlanEntryStatus::InProgress),
                    })
                    .collect();
                // One plan per turn: update it in place.
                let last_user = self.entries.iter().rposition(|e| matches!(e, Entry::User(_))).unwrap_or(0);
                match self.entries[last_user..].iter_mut().find_map(|e| match e {
                    Entry::Plan(p) => Some(p),
                    _ => None,
                }) {
                    Some(plan) => *plan = items,
                    None => self.entries.push(Entry::Plan(items)),
                }
            }
            SessionUpdate::CurrentModeUpdate(m) => {
                if let Some(modes) = &mut self.modes {
                    modes.current_mode_id = m.current_mode_id;
                }
            }
            SessionUpdate::Notice(n) => {
                let text = match n.description {
                    Some(d) => format!("{}: {d}", n.title),
                    None => n.title,
                };
                self.notice(text, matches!(n.severity, schema::NoticeSeverity::Error));
            }
            _ => {}
        }
    }

    /// Streams text into the last entry of the same kind, or starts a new one.
    fn append_text(&mut self, text: String, make: impl FnOnce(String) -> Entry, get: impl Fn(&mut Entry) -> Option<&mut String>) {
        if text.is_empty() {
            return;
        }
        if let Some(s) = self.entries.last_mut().and_then(get) {
            s.push_str(&text);
        } else {
            self.entries.push(make(text));
        }
    }
}

/// A login method from `initialize` (read from JSON to keep the `_meta` details).
fn auth_option(m: &Value) -> Option<AuthOption> {
    let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let list = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    let id = s(m.get("id"))?;
    let meta = m.pointer("/_meta/terminal-auth");
    let is_terminal = m.get("type").and_then(Value::as_str) == Some("terminal") || meta.is_some();
    let terminal = is_terminal.then(|| {
        let env = m
            .get("env")
            .and_then(Value::as_object)
            .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        // The exact command from `_meta` if given, else the agent's own command with `args`.
        match meta.and_then(|t| s(t.get("command")).map(|c| (c, list(t.get("args"))))) {
            Some((command, args)) => TerminalLogin { command: Some(command), args, env },
            None => TerminalLogin { command: None, args: list(m.get("args")), env },
        }
    });
    let api_key = m.pointer("/_meta/api-key").is_some() || id.contains("api-key");
    Some(AuthOption { name: s(m.get("name")).unwrap_or_else(|| id.clone()), description: s(m.get("description")), id, terminal, api_key })
}

fn apply_fields(t: &mut ToolEntry, f: &schema::ToolCallUpdateFields) {
    if let Some(title) = &f.title {
        t.title = title.clone();
    }
    if let Some(kind) = &f.kind {
        t.kind = kind.clone();
    }
    if let Some(status) = &f.status {
        t.status = status.clone();
    }
    if let Some(content) = &f.content {
        t.content = content.clone();
    }
    if let Some(locations) = &f.locations {
        t.locations = locations.iter().map(|l| (l.path.clone(), l.line)).collect();
    }
}

/// Text of a content block (other kinds as a short placeholder).
pub fn text_of(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        ContentBlock::ResourceLink(l) => format!("[{}]", l.name),
        ContentBlock::Image(_) => "[Bild]".into(),
        ContentBlock::Audio(_) => "[Audio]".into(),
        ContentBlock::Resource(_) => "[Datei]".into(),
        #[allow(unreachable_patterns)]
        _ => String::new(),
    }
}

pub fn file_uri(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    format!("file:///{}", crate::preview::percent_encode(p.trim_start_matches('/')))
}

/// Applies `line` (1-based) and `limit` of a read request to a text.
pub fn slice_lines(text: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_string();
    }
    let start = line.map_or(0, |l| l.saturating_sub(1) as usize);
    let take = limit.map_or(usize::MAX, |l| l as usize);
    text.split_inclusive('\n').skip(start).take(take).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_login_methods() {
        // As sent by claude-agent-acp and codex-acp.
        let claude = json!({ "id": "claude-ai-login", "name": "Claude Subscription", "type": "terminal", "args": ["--cli", "auth", "login"],
            "_meta": { "terminal-auth": { "command": "C:\node.exe", "args": ["index.js", "--cli", "auth", "login"] } } });
        let m = auth_option(&claude).unwrap();
        let t = m.terminal.unwrap();
        assert_eq!(t.command.as_deref(), Some("C:\node.exe"));
        assert_eq!(t.args, vec!["index.js", "--cli", "auth", "login"]);
        assert!(!m.api_key);

        let key = auth_option(&json!({ "id": "api-key", "name": "API Key", "_meta": { "api-key": { "provider": "openai" } } })).unwrap();
        assert!(key.api_key && key.terminal.is_none());
        let browser = auth_option(&json!({ "id": "chat-gpt", "name": "ChatGPT" })).unwrap();
        assert!(!browser.api_key && browser.terminal.is_none());
        assert!(auth_option(&json!({ "name": "ohne id" })).is_none());
    }
}
