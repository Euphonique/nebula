//! End-to-end tests of the extension host with small test extensions.

use std::time::{Duration, Instant};

use ropey::Rope;
use serde_json::json;

use super::host::ExtHost;
use super::manifest::Extension;
use super::protocol::{DocInfo, FromHost, ToHost};

fn make_ext(dir: &std::path::Path, id: &str, main: &str, extra: serde_json::Value) -> Extension {
    std::fs::create_dir_all(dir.join("Scripts")).unwrap();
    let mut manifest = json!({
        "identifier": id,
        "name": "Test",
        "main": "main.js",
        "activationEvents": ["onLanguage:html"],
        "commands": {"editor": [{"title": "Upper", "command": "test.upper"}], "extensions": [{"title": "Info", "command": "test.info"}]},
    });
    for (k, v) in extra.as_object().unwrap() {
        manifest[k] = v.clone();
    }
    std::fs::write(dir.join("extension.json"), manifest.to_string()).unwrap();
    std::fs::write(dir.join("Scripts").join("main.js"), main).unwrap();
    Extension::load(dir).unwrap()
}

fn doc_info(id: u64) -> DocInfo {
    DocInfo {
        id,
        uri: "file:///C:/p/index.html".into(),
        path: Some("C:/p/index.html".into()),
        syntax: Some("html".into()),
        remote: false,
        dirty: false,
        eol: "\n".into(),
        tab_length: 4,
        soft_tabs: true,
    }
}

/// Collects host events until `pred` matches one (or times out).
thread_local! {
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<super::protocol::HostEvent>> = Default::default();
}

fn wait_for(host: &ExtHost, mut pred: impl FnMut(&FromHost) -> bool) -> Vec<FromHost> {
    let start = Instant::now();
    let mut seen = Vec::new();
    while start.elapsed() < Duration::from_secs(15) {
        BACKLOG.with(|b| b.borrow_mut().extend(host.poll()));
        while let Some(ev) = BACKLOG.with(|b| b.borrow_mut().pop_front()) {
            if let FromHost::Log { level, text } = &ev.msg {
                eprintln!("[{}] {level}: {text}", ev.ext);
            }
            let hit = pred(&ev.msg);
            seen.push(ev.msg);
            if hit {
                return seen;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out; saw {seen:#?}");
}

#[test]
fn extension_host_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: tests in this module are the only ones reading this variable.
    unsafe { std::env::set_var("NEBULA_CONFIG_DIR", tmp.path().join("cfg")) };
    let main = r#"
        const helper = require("helper.js");
        exports.activate = function () {
            console.log("activated " + helper.name + " " + nova.extension.identifier);
        };
        nova.commands.register("test.upper", (editor) => {
            const r = new Range(0, editor.document.length);
            const text = editor.getTextInRange(r);
            editor.edit((e) => e.replace(r, text.toUpperCase())).then(() => {
                console.log("after edit: " + editor.getTextInRange(new Range(0, 5)));
                nova.workspace.showInformativeMessage("fertig " + editor.document.syntax);
            });
        });
        nova.commands.register("test.info", (workspace) => {
            const p = new Process("/usr/bin/env", { args: ["cmd", "/C", "echo", "prozess-ausgabe"] });
            let out = "";
            p.onStdout((l) => { out += l; });
            p.onDidExit((code) => {
                setTimeout(() => nova.workspace.showWarningMessage(out.trim() + " " + code), 10);
            });
            p.start();
        });
        nova.assistants.registerCompletionAssistant("html", {
            provideCompletionItems(editor, ctx) {
                const item = new CompletionItem("hallo-" + ctx.text, CompletionItemKind.Tag);
                item.insertText = "<hallo>$0</hallo>";
                item.insertTextFormat = InsertTextFormat.Snippet;
                return [item];
            }
        });
        nova.assistants.registerIssueAssistant("html", {
            provideIssues(editor) {
                const i = new Issue();
                i.message = "Testproblem";
                i.severity = IssueSeverity.Error;
                i.line = 1; i.column = 2;
                return [i];
            }
        });
        nova.config.observe("test.flag", (v) => console.log("flag=" + v));
    "#;
    let dir = tmp.path().join("ext");
    let ext = make_ext(&dir, "test.ext", main, json!({}));
    std::fs::write(dir.join("Scripts").join("helper.js"), "exports.name = 'helper';").unwrap();

    let host = ExtHost::spawn(eframe::egui::Context::default());
    host.send(ToHost::Load(vec![ext]));
    host.send(ToHost::DocOpened(doc_info(7), Rope::from_str("<p>grüße 😀</p>\n")));
    host.send(ToHost::ActiveDoc(Some(7)));
    wait_for(&host, |m| matches!(m, FromHost::Activated));
    wait_for(&host, |m| matches!(m, FromHost::Log { text, .. } if text == "activated helper test.ext"));

    // Issue assistant runs for the opened document.
    let seen = wait_for(&host, |m| matches!(m, FromHost::Issues { issues, .. } if !issues.is_empty()));
    let FromHost::Issues { uri, issues, .. } = seen.last().unwrap() else { unreachable!() };
    assert_eq!(uri, "file:///C:/p/index.html");
    assert_eq!(issues[0].message, "Testproblem");
    assert_eq!(issues[0].severity, 1);

    // Editor command: the edit covers the whole document in UTF-16 units.
    host.send(ToHost::Command { ext: "test.ext".into(), name: "test.upper".into(), editor: Some(7), args: vec![] });
    let seen = wait_for(&host, |m| matches!(m, FromHost::Edit { .. }));
    let FromHost::Edit { doc, edits, cb } = seen.last().unwrap() else { unreachable!() };
    assert_eq!(*doc, 7);
    assert_eq!(edits[0].start, 0);
    assert_eq!(edits[0].end, "<p>grüße 😀</p>\n".encode_utf16().count());
    assert_eq!(edits[0].text, "<P>GRÜSSE 😀</P>\n");
    // The UI applies the edit, sends the new text, then confirms.
    host.send(ToHost::DocChanged { id: 7, text: Rope::from_str(&edits[0].text), dirty: true });
    host.send(ToHost::Reply { ext: "test.ext".into(), cb: cb.unwrap(), value: json!(true) });
    let seen = wait_for(&host, |m| matches!(m, FromHost::Message { .. }));
    let FromHost::Message { text, .. } = seen.last().unwrap() else { unreachable!() };
    assert_eq!(text, "fertig html");

    // Completion: aggregated answer with snippet item.
    host.send(ToHost::Completion { req: 99, doc: 7, pos: 2, trigger: None });
    let seen = wait_for(&host, |m| matches!(m, FromHost::Completions { .. }));
    let FromHost::Completions { req, items } = seen.last().unwrap() else { unreachable!() };
    assert_eq!(*req, 99);
    assert_eq!(items[0].label, "hallo-P");
    assert!(items[0].snippet);

    // Process output and exit, delivered through a timer.
    host.send(ToHost::Command { ext: "test.ext".into(), name: "test.info".into(), editor: None, args: vec![] });
    let seen = wait_for(&host, |m| matches!(m, FromHost::Message { level, .. } if level == "warning"));
    let FromHost::Message { text, .. } = seen.last().unwrap() else { unreachable!() };
    assert_eq!(text, "prozess-ausgabe 0");

    // Configuration changes reach observers.
    host.config.lock().unwrap().set(false, "test.flag", json!(true));
    host.send(ToHost::ConfigChanged { workspace: false, key: "test.flag".into() });
    wait_for(&host, |m| matches!(m, FromHost::Log { text, .. } if text == "flag=true"));

    // Will-save round trip completes even without listeners.
    host.send(ToHost::WillSave { req: 5, doc: 7 });
    wait_for(&host, |m| matches!(m, FromHost::WillSaveDone { req: 5 }));
}

/// Loads every extension below $NEBULA_TEST_EXTS and reports errors.
#[test]
#[ignore]
fn real_extensions_activate() {
    let root = std::path::PathBuf::from(std::env::var("NEBULA_TEST_EXTS").expect("NEBULA_TEST_EXTS"));
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: only this test reads the variable in this process.
    unsafe { std::env::set_var("NEBULA_CONFIG_DIR", tmp.path().join("cfg")) };
    let mut exts = Vec::new();
    for e in walk_manifests(&root) {
        match Extension::load(&e) {
            Ok(x) => exts.push(x),
            Err(err) => eprintln!("LOAD FAIL {}: {err}", e.display()),
        }
    }
    let host = ExtHost::spawn(eframe::egui::Context::default());
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    host.send(ToHost::Workspace(Some(project.to_string_lossy().into_owned())));
    host.send(ToHost::Load(exts));
    for (i, syntax) in ["html", "css", "javascript", "typescript", "php", "json", "python", "yaml", "xml", "markdown", "scss", "go", "rust"].iter().enumerate() {
        let mut info = doc_info(i as u64 + 1);
        info.syntax = Some(syntax.to_string());
        info.uri = format!("file:///C:/p/f{i}.{syntax}");
        info.path = Some(format!("C:/p/f{i}.{syntax}"));
        host.send(ToHost::DocOpened(info, Rope::from_str("x\n")));
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(12) {
        for ev in host.poll() {
            match &ev.msg {
                FromHost::Log { level, text } if level == "error" || level == "warning" => eprintln!("[{}] {level}: {}", ev.ext, text.lines().take(4).collect::<Vec<_>>().join(" | ")),
                FromHost::Failed(e) => eprintln!("FAILED [{}]: {}", ev.ext, e.lines().next().unwrap_or("")),
                FromHost::Activated => eprintln!("activated {}", ev.ext),
                FromHost::Message { level, text } => eprintln!("[{}] message {level}: {text}", ev.ext),
                FromHost::Notify { title, body, .. } => eprintln!("[{}] notify: {title} {body}", ev.ext),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn walk_manifests(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.join("extension.json").is_file() {
                out.push(p);
            } else {
                out.extend(walk_manifests(&p));
            }
        }
    }
    out
}

/// Runs one extension ($NEBULA_TEST_EXT) on a document and prints what it does.
#[test]
#[ignore]
fn single_extension_trace() {
    let dir = std::path::PathBuf::from(std::env::var("NEBULA_TEST_EXT").expect("NEBULA_TEST_EXT"));
    let syntax = std::env::var("NEBULA_TEST_SYNTAX").unwrap_or_else(|_| "css".into());
    let text = std::env::var("NEBULA_TEST_TEXT").unwrap_or_else(|_| "body { colr: red; }\n".into());
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: only this test reads the variable in this process.
    unsafe { std::env::set_var("NEBULA_CONFIG_DIR", tmp.path().join("cfg")) };
    let ext = Extension::load(&dir).unwrap();
    let host = ExtHost::spawn(eframe::egui::Context::default());
    host.send(ToHost::Load(vec![ext]));
    let mut info = doc_info(1);
    info.syntax = Some(syntax.clone());
    info.uri = format!("file:///C:/p/test.{syntax}");
    host.send(ToHost::DocOpened(info, Rope::from_str(&text)));
    host.send(ToHost::ActiveDoc(Some(1)));
    host.send(ToHost::DocSaved(1));
    let start = Instant::now();
    let secs: u64 = std::env::var("NEBULA_TEST_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut asked = false;
    while start.elapsed() < Duration::from_secs(secs) {
        if !asked && start.elapsed() > Duration::from_secs(secs * 2 / 3) {
            asked = true;
            host.send(ToHost::Completion { req: 77, doc: 1, pos: text.chars().count().saturating_sub(1), trigger: Some(".".into()) });
            host.send(ToHost::Hover { req: 78, doc: 1, pos: 7 });
        }
        for ev in host.poll() {
            let s = format!("{:?}", ev.msg);
            eprintln!("{}", &s[..s.len().min(600)]);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
