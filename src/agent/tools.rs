//! Tools of the built-in agent. Reading and writing go through Nebula (open
//! tabs, undo, upload on save); listing, searching and commands run here.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::llm::ToolDef;

pub fn definitions() -> Vec<ToolDef> {
    let path = json!({ "type": "string", "description": "File path, relative to the project folder or absolute" });
    vec![
        ToolDef {
            name: "read_file",
            description: "Read a text file (the open editor tab's current text if it is open). Use offset/limit for large files.",
            schema: json!({ "type": "object", "properties": {
                "path": path,
                "offset": { "type": "integer", "description": "First line to read (1-based)" },
                "limit": { "type": "integer", "description": "Number of lines to read" }
            }, "required": ["path"] }),
        },
        ToolDef {
            name: "list_files",
            description: "List files and folders (respects .gitignore). Without `recursive` only the folder itself.",
            schema: json!({ "type": "object", "properties": {
                "path": { "type": "string", "description": "Folder, default: project folder" },
                "recursive": { "type": "boolean" }
            } }),
        },
        ToolDef {
            name: "search",
            description: "Search file contents in the project with a regular expression (respects .gitignore). Returns path:line: text.",
            schema: json!({ "type": "object", "properties": {
                "pattern": { "type": "string", "description": "Regular expression (Rust syntax)" },
                "path": { "type": "string", "description": "Folder or file to search, default: project folder" },
                "glob": { "type": "string", "description": "Only files matching this pattern, e.g. *.php" },
                "ignore_case": { "type": "boolean" }
            }, "required": ["pattern"] }),
        },
        ToolDef {
            name: "edit_file",
            description: "Replace text in a file. `old_string` must match exactly (including indentation) and be unique unless replace_all is set. Read the file first. The user approves the change.",
            schema: json!({ "type": "object", "properties": {
                "path": path,
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "replace_all": { "type": "boolean" }
            }, "required": ["path", "old_string", "new_string"] }),
        },
        ToolDef {
            name: "write_file",
            description: "Create a file or replace its whole content. The user approves the change.",
            schema: json!({ "type": "object", "properties": {
                "path": path,
                "content": { "type": "string" }
            }, "required": ["path", "content"] }),
        },
        ToolDef {
            name: "run_command",
            description: "Run a command in cmd.exe in the project folder (timeout 2 minutes) and return its output. The user approves it first.",
            schema: json!({ "type": "object", "properties": {
                "command": { "type": "string" }
            }, "required": ["command"] }),
        },
    ]
}

/// A path given by the model, relative to the project folder or absolute.
pub fn resolve(cwd: &Path, path: &str) -> PathBuf {
    let p = Path::new(path.trim());
    let joined = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    // "a/./b/../c" → "a/c", with backslashes as on Windows.
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// The path as shown to the user: relative to the project if inside it.
pub fn display(cwd: &Path, path: &Path) -> String {
    // Server files (read and written through Nebula).
    if let Some((_, remote)) = crate::acp::session::parse_virtual(path) {
        return format!("{remote} (Server)");
    }
    path.strip_prefix(cwd).map(|p| p.display().to_string()).unwrap_or_else(|_| path.display().to_string())
}

const MAX_ENTRIES: usize = 500;
const MAX_MATCHES: usize = 200;

pub fn list_files(cwd: &Path, input: &Value) -> Result<String, String> {
    let dir = resolve(cwd, input["path"].as_str().unwrap_or("."));
    let recursive = input["recursive"].as_bool().unwrap_or(false);
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    let walk = ignore::WalkBuilder::new(&dir).max_depth(if recursive { None } else { Some(1) }).hidden(false).build();
    let mut out = Vec::new();
    for entry in walk.flatten() {
        if entry.depth() == 0 || entry.path().components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        let rel = display(&dir, entry.path()).replace('\\', "/");
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        out.push(if is_dir { format!("{rel}/") } else { rel });
        if out.len() >= MAX_ENTRIES {
            out.push(format!("… (more than {MAX_ENTRIES} entries, list a subfolder)"));
            break;
        }
    }
    out.sort();
    Ok(if out.is_empty() { "(empty)".into() } else { out.join("\n") })
}

pub fn search(cwd: &Path, input: &Value) -> Result<(String, usize), String> {
    let pattern = input["pattern"].as_str().unwrap_or("");
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(input["ignore_case"].as_bool().unwrap_or(false))
        .build()
        .map_err(|e| format!("Invalid pattern: {e}"))?;
    let root = resolve(cwd, input["path"].as_str().unwrap_or("."));
    let mut walk = ignore::WalkBuilder::new(&root);
    walk.hidden(false);
    if let Some(glob) = input["glob"].as_str().filter(|g| !g.is_empty()) {
        let mut ov = ignore::overrides::OverrideBuilder::new(&root);
        ov.add(glob).map_err(|e| e.to_string())?;
        walk.overrides(ov.build().map_err(|e| e.to_string())?);
    }
    let mut out = Vec::new();
    'files: for entry in walk.build().flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) || entry.path().components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
        for (n, line) in text.lines().enumerate() {
            if re.is_match(line) {
                let line: String = line.trim().chars().take(200).collect();
                out.push(format!("{}:{}: {line}", display(cwd, entry.path()).replace('\\', "/"), n + 1));
                if out.len() >= MAX_MATCHES {
                    out.push(format!("… (more than {MAX_MATCHES} matches, narrow the search)"));
                    break 'files;
                }
            }
        }
    }
    let count = out.len();
    Ok((if out.is_empty() { "No matches.".into() } else { out.join("\n") }, count))
}

/// Replaces `old` in `text` (exactly once unless `all`).
pub fn replace(text: &str, old: &str, new: &str, all: bool) -> Result<String, String> {
    if old.is_empty() {
        return Err("old_string is empty".into());
    }
    // Files with Windows line endings: the model usually writes \n.
    let (text_n, crlf) = if text.contains("\r\n") { (text.replace("\r\n", "\n"), true) } else { (text.to_string(), false) };
    let (old, new) = (old.replace("\r\n", "\n"), new.replace("\r\n", "\n"));
    let count = text_n.matches(&old).count();
    let result = match count {
        0 => return Err("old_string was not found in the file. Read the file again and copy the text exactly.".into()),
        1 => text_n.replacen(&old, &new, 1),
        n if all => {
            let _ = n;
            text_n.replace(&old, &new)
        }
        n => return Err(format!("old_string occurs {n} times; add more context to make it unique or set replace_all.")),
    };
    Ok(if crlf { result.replace('\n', "\r\n") } else { result })
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT: usize = 30_000;

/// Runs a command in cmd.exe; returns exit code and output (stdout and stderr).
pub fn run_command(cwd: &Path, command: &str, cancel: &AtomicBool) -> Result<(Option<i32>, String), String> {
    use std::os::windows::process::CommandExt;
    let mut child = std::process::Command::new("cmd")
        .raw_arg(format!("/D /S /C \"{command}\""))
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    let read = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    };
    let out = read(Box::new(child.stdout.take().unwrap()));
    let err = read(Box::new(child.stderr.take().unwrap()));
    let start = Instant::now();
    let mut note = None;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(status);
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() > COMMAND_TIMEOUT {
            note = Some(if cancel.load(Ordering::Relaxed) { "(cancelled)" } else { "(stopped after 2 minutes)" });
            crate::ext::process::kill(child.id());
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut text = crate::win32::console_text(&out.join().unwrap_or_default());
    let stderr = crate::win32::console_text(&err.join().unwrap_or_default());
    if !stderr.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&stderr);
    }
    if text.len() > MAX_OUTPUT {
        let cut = text.char_indices().nth(MAX_OUTPUT).map_or(text.len(), |(i, _)| i);
        text.truncate(cut);
        text.push_str("\n… (output cut)");
    }
    if let Some(n) = note {
        text.push_str(&format!("\n{n}"));
    }
    Ok((status.and_then(|s| s.code()), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_paths_in_the_project() {
        let cwd = Path::new(r"C:\proj");
        assert_eq!(resolve(cwd, "src/a.rs"), PathBuf::from(r"C:\proj\src\a.rs"));
        assert_eq!(resolve(cwd, "./src/../b.txt"), PathBuf::from(r"C:\proj\b.txt"));
        assert_eq!(resolve(cwd, r"D:\x.txt"), PathBuf::from(r"D:\x.txt"));
        assert_eq!(display(cwd, &resolve(cwd, "src/a.rs")), r"src\a.rs");
    }

    #[test]
    fn replaces_exactly_once() {
        assert_eq!(replace("a b a", "b", "c", false).unwrap(), "a c a");
        assert!(replace("a b a", "a", "c", false).unwrap_err().contains("2 times"));
        assert_eq!(replace("a b a", "a", "c", true).unwrap(), "c b c");
        assert!(replace("abc", "x", "y", false).is_err());
        // Windows line endings stay.
        assert_eq!(replace("x\r\ny\r\n", "x\ny", "x\nz", false).unwrap(), "x\r\nz\r\n");
    }

    #[test]
    fn lists_and_searches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.php"), "<?php\nfunction start() {}\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "start here\n").unwrap();
        assert_eq!(list_files(dir.path(), &json!({})).unwrap(), "b.txt\nsrc/");
        assert_eq!(list_files(dir.path(), &json!({ "recursive": true })).unwrap(), "b.txt\nsrc/\nsrc/a.php");
        let (hits, n) = search(dir.path(), &json!({ "pattern": "start", "glob": "*.php" })).unwrap();
        assert_eq!((hits.as_str(), n), ("src/a.php:2: function start() {}", 1));
    }

    #[test]
    fn runs_commands() {
        let dir = tempfile::tempdir().unwrap();
        let (code, out) = run_command(dir.path(), "echo grüße", &AtomicBool::new(false)).unwrap();
        assert_eq!(code, Some(0));
        assert_eq!(out.trim(), "grüße");
    }
}
