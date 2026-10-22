//! Nebula as an MCP server (Model Context Protocol, "Streamable HTTP" with
//! plain JSON answers): agents in the panel, Claude Code in a terminal or
//! Claude Desktop can ask which files are open, what is selected, which
//! problems the language servers report, and open files in the editor.
//!
//! Only reachable from this computer (127.0.0.1) and only with the token in
//! the address. Tool calls are answered by the app thread (it owns the
//! documents): the server hands them over and waits for the result.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Settings: on/off, the port tried first and the access token.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: true, port: 47_316, token: String::new() }
    }
}

/// A tool call for the app; `reply` takes the text result (or an error).
pub struct Call {
    pub tool: String,
    pub args: Value,
    pub reply: Sender<Result<String, String>>,
}

pub struct McpServer {
    pub port: u16,
    pub token: String,
    pub calls: Receiver<Call>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for McpServer {
    /// Ends the server: the accept loop checks the flag after the next connection.
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&([127, 0, 0, 1], self.port).into(), Duration::from_millis(300));
        if let Ok(mut url) = URL.lock() {
            *url = None;
        }
    }
}

impl McpServer {
    /// The address clients connect to.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp/{}", self.port, self.token)
    }
}

/// The running server's address, for agent sessions started in Nebula.
static URL: Mutex<Option<String>> = Mutex::new(None);

/// `mcpServers` entry for ACP sessions (if the server runs).
pub fn acp_entry() -> Option<Value> {
    let url = URL.lock().ok()?.clone()?;
    Some(json!({ "type": "http", "name": "nebula", "url": url, "headers": [] }))
}

/// A random token (no extra crate: the std hasher is seeded randomly).
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Starts the server on `port` (another free one if it is taken).
pub fn start(port: u16, token: String, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<McpServer> {
    // A restarted server may wait a moment for its old port to be free.
    let mut listener = TcpListener::bind(("127.0.0.1", port));
    for _ in 0..10 {
        if listener.is_ok() || port == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        listener = TcpListener::bind(("127.0.0.1", port));
    }
    let listener = listener.or_else(|_| TcpListener::bind(("127.0.0.1", 0)))?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = crossbeam_channel::unbounded();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
    let path = format!("/mcp/{token}");
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let (tx, wake, path) = (tx.clone(), wake.clone(), path.clone());
            std::thread::spawn(move || {
                let _ = serve(stream, &path, &tx, &*wake);
            });
        }
    });
    let server = McpServer { port, token, calls: rx, stop };
    *URL.lock().unwrap() = Some(server.url());
    Ok(server)
}

/// Requests of one connection (keep-alive).
fn serve(stream: TcpStream, path: &str, calls: &Sender<Call>, wake: &(dyn Fn() + Send + Sync)) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line)? == 0 {
            return Ok(());
        }
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; length.min(16 * 1024 * 1024)];
        reader.read_exact(&mut body)?;
        let mut parts = request_line.split_whitespace();
        let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let (status, reply) = if target.split('?').next() != Some(path) {
            ("404 Not Found", None)
        } else {
            match method {
                "POST" => match serde_json::from_slice::<Value>(&body) {
                    Ok(msg) => match handle(msg, calls, wake) {
                        Some(answer) => ("200 OK", Some(answer)),
                        None => ("202 Accepted", None),
                    },
                    Err(e) => ("400 Bad Request", Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }))),
                },
                // No server-sent events: the client then just doesn't listen.
                "GET" => ("405 Method Not Allowed", None),
                "DELETE" => ("200 OK", None),
                _ => ("405 Method Not Allowed", None),
            }
        };
        let body = reply.map(|v| v.to_string()).unwrap_or_default();
        let head = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n", body.len());
        out.write_all(head.as_bytes())?;
        out.write_all(body.as_bytes())?;
        out.flush()?;
    }
}

/// One message or a batch; `None` if nothing needs an answer.
fn handle(msg: Value, calls: &Sender<Call>, wake: &(dyn Fn() + Send + Sync)) -> Option<Value> {
    if let Value::Array(batch) = msg {
        let answers: Vec<Value> = batch.into_iter().filter_map(|m| handle(m, calls, wake)).collect();
        return (!answers.is_empty()).then(|| Value::Array(answers));
    }
    let id = msg.get("id").cloned()?; // notifications and responses: nothing to say
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-03-26"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "nebula", "title": "Nebula", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Nebula is the code editor the user works in. Use these tools to see the open files, the active file with cursor and selection, and reported problems, and to open files for the user.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let tool = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (tx, rx) = crossbeam_channel::bounded(1);
            let sent = calls.send(Call { tool, args, reply: tx }).is_ok();
            wake();
            let outcome = if sent { rx.recv_timeout(Duration::from_secs(15)).unwrap_or_else(|_| Err("Nebula did not answer in time.".into())) } else { Err("Nebula is closing.".into()) };
            Ok(match outcome {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
                Err(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
            })
        }
        other => Err(json!({ "code": -32601, "message": format!("Method not found: {other}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

fn tools() -> Value {
    let path = json!({ "type": "string", "description": "Absolute path of a local file" });
    json!([
        {
            "name": "get_open_files",
            "description": "List the files open in Nebula's tabs: path (or server and path), unsaved changes, which one is active.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_active_editor",
            "description": "The file the user is looking at: path, language, cursor line/column (1-based) and the selected text with its line range.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_diagnostics",
            "description": "Problems reported by the language servers (errors, warnings) for the open files, or for one file.",
            "inputSchema": { "type": "object", "properties": { "path": path } }
        },
        {
            "name": "get_file_text",
            "description": "The current text of an open file, including unsaved changes.",
            "inputSchema": { "type": "object", "properties": { "path": path }, "required": ["path"] }
        },
        {
            "name": "open_file",
            "description": "Open a local file in Nebula's editor for the user, optionally at a line (1-based).",
            "inputSchema": { "type": "object", "properties": { "path": path, "line": { "type": "integer" } }, "required": ["path"] }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speaks MCP over HTTP like a client would.
    fn post(port: u16, path: &str, body: &Value) -> (String, Value) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let body = body.to_string();
        write!(s, "POST {path} HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut r = BufReader::new(s);
        let mut status = String::new();
        r.read_line(&mut status).unwrap();
        let mut length = 0;
        loop {
            let mut l = String::new();
            r.read_line(&mut l).unwrap();
            if l.trim().is_empty() {
                break;
            }
            if let Some(v) = l.to_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0; length];
        r.read_exact(&mut buf).unwrap();
        (status.trim().to_string(), serde_json::from_slice(&buf).unwrap_or(Value::Null))
    }

    #[test]
    fn answers_mcp_requests() {
        let server = start(0, "geheim".into(), || {}).unwrap();
        let calls = server.calls.clone();
        // The "app": answers tool calls.
        std::thread::spawn(move || {
            for c in calls {
                let _ = c.reply.send(Ok(format!("{} {}", c.tool, c.args["path"].as_str().unwrap_or(""))));
            }
        });
        let path = "/mcp/geheim";
        let (status, init) = post(server.port, path, &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }));
        assert!(status.contains("200"));
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let (status, _) = post(server.port, path, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        assert!(status.contains("202"));
        let (_, list) = post(server.port, path, &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 5);
        let (_, call) = post(server.port, path, &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "open_file", "arguments": { "path": "C:\\a.txt" } } }));
        assert_eq!(call["result"]["content"][0]["text"], "open_file C:\\a.txt");
        // Without the token: nothing.
        let (status, _) = post(server.port, "/mcp/falsch", &json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" }));
        assert!(status.contains("404"));
    }
}
