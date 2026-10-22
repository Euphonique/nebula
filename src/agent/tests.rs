//! The built-in agent end to end: a fake model server in the OpenAI format
//! asks for tools, the agent runs them through a real `AgentSession`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::acp::AgentConfig;
use crate::acp::session::{AgentSession, Entry, FileRequest, Phase};

/// Serves `/v1/models` and answers chat requests with the given streams in turn;
/// returns the port and the received chat request bodies.
fn fake_server(answers: Vec<String>) -> (u16, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        let mut answers = answers.into_iter();
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut stream = stream;
            // Several requests per connection (keep-alive).
            loop {
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                    if line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                if head.is_empty() {
                    break;
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let (kind, text) = if head.starts_with("GET /v1/models") {
                    ("application/json", json!({ "data": [{ "id": "test-modell" }, { "id": "text-embedding-3" }] }).to_string())
                } else {
                    log.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or(Value::Null));
                    ("text/event-stream", answers.next().unwrap_or_default())
                };
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\r\n{text}", text.len());
                if stream.write_all(reply.as_bytes()).is_err() {
                    break;
                }
            }
        }
    });
    (port, seen)
}

fn sse(chunks: &[Value]) -> String {
    let mut s: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    s.push_str("data: [DONE]\n\n");
    s
}

fn tool_call(id: &str, name: &str, args: Value) -> String {
    sse(&[
        json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": id, "function": { "name": name, "arguments": args.to_string() } }] } }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
    ])
}

/// Polls the session until `done`, answering file requests from `files` and permissions with `answer`.
fn drive(s: &mut AgentSession, files: &mut std::collections::HashMap<String, String>, answer: &str, done: impl Fn(&AgentSession) -> bool) {
    let start = Instant::now();
    while !done(s) {
        assert!(start.elapsed() < Duration::from_secs(20), "Zeitüberschreitung, Phase {:?}", s.phase);
        for req in s.poll() {
            match req {
                FileRequest::Read { rpc_id, path, .. } => {
                    let r = files.get(&path.to_string_lossy().into_owned()).cloned().ok_or_else(|| "fehlt".to_string());
                    s.reply_file(&rpc_id, r.map(Some));
                }
                FileRequest::Write { rpc_id, path, content } => {
                    files.insert(path.to_string_lossy().into_owned(), content);
                    s.reply_file(&rpc_id, Ok(None));
                }
            }
        }
        if !s.permissions.is_empty() {
            s.answer_permission(Some(answer));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn reads_edits_and_answers() {
    let (port, seen) = fake_server(vec![
        tool_call("c1", "read_file", json!({ "path": "a.txt" })),
        tool_call("c2", "edit_file", json!({ "path": "a.txt", "old_string": "alt", "new_string": "neu" })),
        sse(&[json!({ "choices": [{ "delta": { "content": "Erledigt." }, "finish_reason": "stop" }] })]),
    ]);
    let cwd = std::path::PathBuf::from(r"C:\projekt");
    let cfg = AgentConfig {
        name: "Test-Agent eingebaut".into(),
        command: super::COMMAND.into(),
        args: vec!["openai".into()],
        env: [(super::BASE_URL.to_string(), format!("http://127.0.0.1:{port}/v1"))].into(),
        ..Default::default()
    };
    let mut files = std::collections::HashMap::from([(r"C:\projekt\a.txt".to_string(), "Zeile alt\n".to_string())]);
    let mut s = AgentSession::start(cfg, cwd, || {});
    drive(&mut s, &mut files, "allow", |s| s.phase != Phase::Starting);
    assert_eq!(s.phase, Phase::Ready, "{:?}", s.phase);
    // Only chat models are offered.
    let models: Vec<String> = match &s.config[0].value {
        crate::acp::session::ConfigValue::Select { options, .. } => options.iter().map(|o| o.0.clone()).collect(),
        _ => vec![],
    };
    assert_eq!(models, ["test-modell"]);

    s.prompt("Ersetze alt durch neu", &[]);
    drive(&mut s, &mut files, "allow", |s| s.phase == Phase::Ready);
    assert_eq!(files[r"C:\projekt\a.txt"], "Zeile neu\n");
    let tools: Vec<String> = s.entries.iter().filter_map(|e| if let Entry::Tool(t) = e { Some(t.title.clone()) } else { None }).collect();
    assert_eq!(tools, ["a.txt", "a.txt"]);
    assert!(matches!(s.entries.last(), Some(Entry::Agent(t)) if t == "Erledigt."));
    // The model got the file content and then the result of the edit.
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[1]["messages"].as_array().unwrap().last().unwrap()["content"], "Zeile alt\n");
    assert_eq!(seen[2]["messages"].as_array().unwrap().last().unwrap()["content"], "a.txt saved.");
}

#[test]
fn rejected_edit_is_not_written() {
    let (port, seen) = fake_server(vec![
        tool_call("c1", "write_file", json!({ "path": "neu.txt", "content": "hallo" })),
        sse(&[json!({ "choices": [{ "delta": { "content": "Gut, dann nicht." }, "finish_reason": "stop" }] })]),
    ]);
    let cfg = AgentConfig {
        name: "Test-Agent eingebaut".into(),
        command: super::COMMAND.into(),
        args: vec!["openai".into()],
        env: [(super::BASE_URL.to_string(), format!("http://127.0.0.1:{port}/v1"))].into(),
        ..Default::default()
    };
    let mut files = std::collections::HashMap::new();
    let mut s = AgentSession::start(cfg, std::path::PathBuf::from(r"C:\projekt"), || {});
    drive(&mut s, &mut files, "reject", |s| s.phase != Phase::Starting);
    s.prompt("Lege neu.txt an", &[]);
    drive(&mut s, &mut files, "reject", |s| s.phase == Phase::Ready);
    assert!(files.is_empty());
    let seen = seen.lock().unwrap();
    let result = &seen[1]["messages"].as_array().unwrap().last().unwrap()["content"];
    assert!(result.as_str().unwrap().contains("rejected"));
}

#[test]
fn missing_key_is_reported() {
    let cfg = AgentConfig { name: "Test ohne Schlüssel".into(), command: super::COMMAND.into(), args: vec!["anthropic".into()], ..Default::default() };
    let mut s = AgentSession::start(cfg, std::env::temp_dir(), || {});
    drive(&mut s, &mut Default::default(), "allow", |s| s.phase != Phase::Starting);
    assert!(matches!(&s.phase, Phase::Failed(e) if e.contains("API-Schlüssel")), "{:?}", s.phase);
}

#[test]
fn reads_attached_files_from_the_prompt() {
    let blocks = json!([
        { "type": "text", "text": "Was macht das?" },
        { "type": "resource", "resource": { "uri": "file:///C:/p/a%20b.js", "text": "let x;" } },
    ]);
    assert_eq!(super::prompt_text(&blocks), "Was macht das?\n\nAttached file C:\\p\\a b.js:\n```\nlet x;\n```");
}
