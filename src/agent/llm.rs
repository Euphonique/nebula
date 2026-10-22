//! Language model APIs for the built-in agent: Anthropic's Messages API and
//! the OpenAI chat completions format (OpenAI, Ollama, LM Studio, OpenRouter
//! and other compatible servers). Answers are streamed; tools are offered in
//! each provider's own format.

use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    /// OpenAI or a compatible server.
    OpenAi,
}

/// Where and how to reach a model.
#[derive(Clone, Debug)]
pub struct Llm {
    pub provider: Provider,
    /// Base URL without trailing slash (Anthropic: without `/v1`).
    pub base: String,
    pub key: Option<String>,
}

/// One piece of a message.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    ToolUse { id: String, name: String, input: Value },
    ToolResult { id: String, content: String, error: bool },
}

/// A message of the conversation (`user`: from the user or tool results).
#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub user: bool,
    pub parts: Vec<Part>,
}

/// A tool the model may call.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    End,
    ToolUse,
    /// The answer hit the length limit.
    MaxTokens,
}

/// What the model answered in one request.
#[derive(Debug, Default)]
pub struct Turn {
    pub text: String,
    /// (id, name, input)
    pub calls: Vec<(String, String, Value)>,
    pub stop: Option<Stop>,
}

/// Why a request failed.
#[derive(Debug)]
pub enum Error {
    /// No answer from the server (the system's message).
    Unreachable(String),
    Other(String),
}

impl Error {
    /// For the user, naming the server that didn't answer.
    pub fn message(&self, base: &str) -> String {
        match self {
            Error::Unreachable(text) => t!("Keine Verbindung zu {base} ({text})", base = base, text = text),
            Error::Other(text) => text.clone(),
        }
    }
}

/// Returned when the user stopped the answer.
pub const CANCELLED: &str = "cancelled";

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 16_000;

fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .timeout_connect(Some(Duration::from_secs(20)))
        // Time to the first byte; thinking models take a while.
        .timeout_recv_response(Some(Duration::from_secs(300)))
        .build()
        .into()
}

impl Llm {
    fn url(&self, path: &str) -> String {
        match self.provider {
            Provider::Anthropic => format!("{}/v1/{path}", self.base),
            Provider::OpenAi => format!("{}/{path}", self.base),
        }
    }

    fn request(&self, method: &str, path: &str) -> ureq::http::request::Builder {
        let mut b = ureq::http::Request::builder().method(method).uri(self.url(path)).header("content-type", "application/json");
        match self.provider {
            Provider::Anthropic => {
                b = b.header("anthropic-version", ANTHROPIC_VERSION);
                if let Some(k) = &self.key {
                    b = b.header("x-api-key", k);
                }
            }
            Provider::OpenAi => {
                if let Some(k) = &self.key {
                    b = b.header("authorization", format!("Bearer {k}"));
                }
            }
        }
        b
    }

    /// Sends a request; errors carry the server's message.
    fn send(&self, req: ureq::http::Request<Vec<u8>>) -> Result<ureq::http::Response<ureq::Body>, Error> {
        // No answer at all (server down, no network, wrong address); the
        // system's message may be in any language.
        let mut resp = http().run(req).map_err(|e| {
            let text = e.to_string();
            Error::Unreachable(text.strip_prefix("io: ").unwrap_or(&text).to_string())
        })?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.pointer("/error/message").or_else(|| v.get("error")).and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| body.chars().take(300).collect());
        Err(Error::Other(match status.as_u16() {
            401 | 403 => t!("Zugriff verweigert – API-Schlüssel prüfen ({message})", message = message),
            404 => t!("Nicht gefunden: {url} ({message})", url = self.base, message = message),
            429 => t!("Zu viele Anfragen oder Kontingent erschöpft ({message})", message = message),
            code => format!("HTTP {code}: {message}"),
        }))
    }

    /// The models the server offers, sorted (chat models only, as far as recognizable).
    pub fn models(&self) -> Result<Vec<String>, Error> {
        let path = match self.provider {
            Provider::Anthropic => "models?limit=100",
            Provider::OpenAi => "models",
        };
        let req = self.request("GET", path).body(Vec::new()).map_err(|e| Error::Other(e.to_string()))?;
        let mut resp = self.send(req)?;
        let body: Value = resp.body_mut().read_json().map_err(|e| Error::Other(e.to_string()))?;
        let mut ids: Vec<String> = body
            .get("data")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string)).collect())
            .unwrap_or_default();
        // OpenAI lists embedding, speech and image models too.
        const NOT_CHAT: &[&str] = &["embed", "whisper", "tts", "dall-e", "moderation", "image", "audio", "realtime", "transcribe", "davinci", "babbage", "search"];
        ids.retain(|id| !NOT_CHAT.iter().any(|n| id.contains(n)));
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Sends the conversation and streams the answer; `on_text` gets text as it arrives.
    pub fn complete(
        &self,
        model: &str,
        system: &str,
        messages: &[Msg],
        tools: &[ToolDef],
        on_text: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<Turn, String> {
        let body = match self.provider {
            Provider::Anthropic => anthropic_body(model, system, messages, tools),
            Provider::OpenAi => openai_body(model, system, messages, tools),
        };
        let path = match self.provider {
            Provider::Anthropic => "messages",
            Provider::OpenAi => "chat/completions",
        };
        let req = self.request("POST", path).body(body.to_string().into_bytes()).map_err(|e| e.to_string())?;
        let resp = self.send(req).map_err(|e| e.message(&self.base))?;
        let reader = resp.into_body().into_reader();
        match self.provider {
            Provider::Anthropic => read_anthropic(reader, on_text, cancel),
            Provider::OpenAi => read_openai(reader, on_text, cancel),
        }
    }
}

fn anthropic_body(model: &str, system: &str, messages: &[Msg], tools: &[ToolDef]) -> Value {
    let messages: Vec<Value> = messages
        .iter()
        .map(|m| {
            let content: Vec<Value> = m
                .parts
                .iter()
                .map(|p| match p {
                    Part::Text(t) => json!({ "type": "text", "text": t }),
                    Part::ToolUse { id, name, input } => json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
                    Part::ToolResult { id, content, error } => json!({ "type": "tool_result", "tool_use_id": id, "content": content, "is_error": error }),
                })
                .collect();
            json!({ "role": if m.user { "user" } else { "assistant" }, "content": content })
        })
        .collect();
    let tools: Vec<Value> = tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.schema })).collect();
    json!({ "model": model, "max_tokens": MAX_TOKENS, "system": system, "messages": messages, "tools": tools, "stream": true })
}

fn openai_body(model: &str, system: &str, messages: &[Msg], tools: &[ToolDef]) -> Value {
    let mut out = vec![json!({ "role": "system", "content": system })];
    for m in messages {
        let text: String = m.parts.iter().filter_map(|p| if let Part::Text(t) = p { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join("\n\n");
        if m.user {
            // Tool results are messages of their own, before any text.
            for p in &m.parts {
                if let Part::ToolResult { id, content, .. } = p {
                    out.push(json!({ "role": "tool", "tool_call_id": id, "content": content }));
                }
            }
            if !text.is_empty() {
                out.push(json!({ "role": "user", "content": text }));
            }
        } else {
            let calls: Vec<Value> = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    Part::ToolUse { id, name, input } => {
                        Some(json!({ "id": id, "type": "function", "function": { "name": name, "arguments": input.to_string() } }))
                    }
                    _ => None,
                })
                .collect();
            let mut msg = json!({ "role": "assistant", "content": text });
            if !calls.is_empty() {
                msg["tool_calls"] = json!(calls);
            }
            out.push(msg);
        }
    }
    let tools: Vec<Value> =
        tools.iter().map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.schema } })).collect();
    json!({ "model": model, "messages": out, "tools": tools, "stream": true })
}

/// The `data:` payloads of a server-sent event stream.
fn sse_data(reader: impl Read, cancel: &AtomicBool, mut each: impl FnMut(Value) -> Result<bool, String>) -> Result<(), String> {
    for line in BufReader::new(reader).lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let line = line.map_err(|e| t!("Verbindung unterbrochen: {e}", e = e))?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
        if !each(v)? {
            break;
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    Ok(())
}

fn read_anthropic(reader: impl Read, on_text: &mut dyn FnMut(&str), cancel: &AtomicBool) -> Result<Turn, String> {
    let mut turn = Turn::default();
    // Tool calls by content block index: (id, name, json so far).
    let mut open: std::collections::BTreeMap<u64, (String, String, String)> = Default::default();
    sse_data(reader, cancel, |v| {
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "content_block_start" => {
                let block = &v["content_block"];
                if block["type"] == "tool_use" {
                    let index = v["index"].as_u64().unwrap_or(0);
                    open.insert(index, (block["id"].as_str().unwrap_or("").into(), block["name"].as_str().unwrap_or("").into(), String::new()));
                }
            }
            "content_block_delta" => {
                let delta = &v["delta"];
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or("");
                        turn.text.push_str(text);
                        on_text(text);
                    }
                    "input_json_delta" => {
                        if let Some(call) = open.get_mut(&v["index"].as_u64().unwrap_or(0)) {
                            call.2.push_str(delta["partial_json"].as_str().unwrap_or(""));
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                turn.stop = match v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    Some("tool_use") => Some(Stop::ToolUse),
                    Some("max_tokens") => Some(Stop::MaxTokens),
                    Some(_) => Some(Stop::End),
                    None => turn.stop,
                };
            }
            "message_stop" => return Ok(false),
            "error" => return Err(v.pointer("/error/message").and_then(Value::as_str).unwrap_or("Fehler").to_string()),
            _ => {}
        }
        Ok(true)
    })?;
    for (_, (id, name, args)) in open {
        let input = if args.trim().is_empty() { json!({}) } else { serde_json::from_str(&args).unwrap_or(json!({})) };
        turn.calls.push((id, name, input));
    }
    Ok(turn)
}

fn read_openai(reader: impl Read, on_text: &mut dyn FnMut(&str), cancel: &AtomicBool) -> Result<Turn, String> {
    let mut turn = Turn::default();
    // Tool calls by index: (id, name, arguments so far).
    let mut open: std::collections::BTreeMap<u64, (String, String, String)> = Default::default();
    sse_data(reader, cancel, |v| {
        if let Some(msg) = v.pointer("/error/message").and_then(Value::as_str) {
            return Err(msg.to_string());
        }
        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else { return Ok(true) };
        let delta = &choice["delta"];
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            turn.text.push_str(text);
            on_text(text);
        }
        for (n, call) in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(n as u64);
            let entry = open.entry(index).or_default();
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                entry.0 = id.to_string();
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                entry.1.push_str(name);
            }
            if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                entry.2.push_str(args);
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            turn.stop = Some(match reason {
                "tool_calls" | "function_call" => Stop::ToolUse,
                "length" => Stop::MaxTokens,
                _ => Stop::End,
            });
        }
        Ok(true)
    })?;
    for (n, (id, name, args)) in open {
        let input = if args.trim().is_empty() { json!({}) } else { serde_json::from_str(&args).unwrap_or(json!({})) };
        // Some servers (Ollama) send no id.
        let id = if id.is_empty() { format!("call_{n}") } else { id };
        turn.calls.push((id, name, input));
    }
    if !turn.calls.is_empty() {
        turn.stop = Some(Stop::ToolUse);
    }
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_anthropic_stream() {
        let stream = r#"event: message_start
data: {"type":"message_start","message":{"id":"m"}}

data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Ich lese "}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"die Datei."}}
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}}
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"a"}}
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":".txt\"}"}}
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}
data: {"type":"message_stop"}
"#;
        let mut shown = String::new();
        let turn = read_anthropic(stream.as_bytes(), &mut |t| shown.push_str(t), &AtomicBool::new(false)).unwrap();
        assert_eq!(shown, "Ich lese die Datei.");
        assert_eq!(turn.calls, vec![("toolu_1".to_string(), "read_file".to_string(), json!({ "path": "a.txt" }))]);
        assert_eq!(turn.stop, Some(Stop::ToolUse));
    }

    #[test]
    fn reads_openai_stream() {
        let stream = r#"data: {"choices":[{"delta":{"role":"assistant","content":"Hallo"}}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"search","arguments":"{\"pat"}}]}}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"tern\":\"x\"}"}}]}}]}
data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}
data: [DONE]
"#;
        let turn = read_openai(stream.as_bytes(), &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(turn.text, "Hallo");
        assert_eq!(turn.calls, vec![("c1".to_string(), "search".to_string(), json!({ "pattern": "x" }))]);
        assert_eq!(turn.stop, Some(Stop::ToolUse));
    }

    #[test]
    fn openai_format_keeps_tool_results_in_order() {
        let msgs = vec![
            Msg { user: true, parts: vec![Part::Text("Lies a.txt".into())] },
            Msg { user: false, parts: vec![Part::ToolUse { id: "c1".into(), name: "read_file".into(), input: json!({ "path": "a.txt" }) }] },
            Msg { user: true, parts: vec![Part::ToolResult { id: "c1".into(), content: "x".into(), error: false }] },
        ];
        let body = openai_body("m", "sys", &msgs, &[]);
        let roles: Vec<&str> = body["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool"]);
        assert_eq!(body["messages"][2]["tool_calls"][0]["function"]["arguments"], "{\"path\":\"a.txt\"}");
    }
}
