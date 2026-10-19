//! Coding agents over the Agent Client Protocol (ACP, as in Zed): the agent
//! runs as a subprocess and talks JSON-RPC over stdin/stdout, one message per
//! line. Nebula is the client: it shows the conversation, answers permission
//! requests and serves file reads/writes (open buffers included).

pub mod session;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use agent_client_protocol_schema::v1 as schema;

/// An agent the user can start, e.g. Claude Code through its ACP adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { name: String::new(), command: String::new(), args: Vec::new(), env: HashMap::new() }
    }
}

/// Agents offered out of the box (all started through npx, installed on first use).
pub fn default_agents() -> Vec<AgentConfig> {
    let npx = |name: &str, args: &[&str]| AgentConfig {
        name: name.into(),
        command: "npx".into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: HashMap::new(),
    };
    vec![
        npx("Claude Code", &["-y", "@agentclientprotocol/claude-agent-acp@latest"]),
        npx("Gemini CLI", &["-y", "@google/gemini-cli@latest", "--experimental-acp"]),
        npx("Codex", &["-y", "@zed-industries/codex-acp@latest"]),
    ]
}

/// Something that came from the agent process.
#[derive(Debug)]
pub enum Event {
    /// A JSON-RPC message (request, response or notification).
    Message(Value),
    /// A line on stderr (log output).
    Stderr(String),
    /// The process ended.
    Exit,
}

/// The running agent process.
pub struct Connection {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pub rx: Receiver<Event>,
    next_id: i64,
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

impl Connection {
    /// Starts the agent in `cwd`; `wake` is called whenever something arrives.
    pub fn spawn(cfg: &AgentConfig, cwd: &Path, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        let program = crate::ext::process::which(&cfg.command, &cfg.env)
            .ok_or_else(|| format!("„{}“ wurde nicht gefunden. Ist Node.js installiert?", cfg.command))?;
        let mut child = Command::new(&program)
            .args(&cfg.args)
            .envs(&cfg.env)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("{} konnte nicht gestartet werden: {e}", cfg.name))?;
        let stdin = child.stdin.take().ok_or("stdin fehlt")?;
        let stdout = child.stdout.take().ok_or("stdout fehlt")?;
        let stderr = child.stderr.take().ok_or("stderr fehlt")?;
        let (tx, rx) = crossbeam_channel::unbounded();
        let wake = Arc::new(wake);
        spawn_reader(stdout, tx.clone(), wake.clone(), false);
        spawn_reader(stderr, tx, wake, true);
        Ok(Self { child, stdin: Arc::new(Mutex::new(stdin)), rx, next_id: 1 })
    }

    fn send(&self, msg: &Value) {
        let mut line = msg.to_string();
        line.push('\n');
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = stdin.write_all(line.as_bytes());
            let _ = stdin.flush();
        }
    }

    /// Sends a request and returns its id.
    pub fn request(&mut self, method: &str, params: impl Serialize) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": to_value(params) }));
        id
    }

    pub fn notify(&self, method: &str, params: impl Serialize) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": to_value(params) }));
    }

    /// Answers a request of the agent.
    pub fn respond(&self, id: &Value, result: Result<Value, String>) {
        let msg = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": message } }),
        };
        self.send(&msg);
    }

    pub fn respond_error(&self, id: &Value, code: i64, message: &str) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }));
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // npx starts node as a child: end the whole tree.
        crate::ext::process::kill(self.child.id());
        let _ = self.child.wait();
    }
}

fn to_value(params: impl Serialize) -> Value {
    serde_json::to_value(params).unwrap_or(Value::Null)
}

fn spawn_reader(pipe: impl std::io::Read + Send + 'static, tx: Sender<Event>, wake: Arc<impl Fn() + Send + Sync + 'static>, stderr: bool) {
    std::thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let event = if stderr {
                Event::Stderr(line)
            } else {
                match serde_json::from_str::<Value>(line.trim()) {
                    Ok(v) if v.is_object() => Event::Message(v),
                    // Agents sometimes print plain text to stdout: treat it as log output.
                    _ if line.trim().is_empty() => continue,
                    _ => Event::Stderr(line),
                }
            };
            if tx.send(event).is_err() {
                return;
            }
            wake();
        }
        if !stderr {
            let _ = tx.send(Event::Exit);
            wake();
        }
    });
}

#[cfg(test)]
mod tests;
