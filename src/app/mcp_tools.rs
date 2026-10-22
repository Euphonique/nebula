//! The tools of Nebula's MCP server (src/mcp.rs), answered on the app thread.

use std::path::PathBuf;

use eframe::egui;
use serde_json::{Value, json};

use super::NebulaApp;
use crate::editor::DocSource;
use crate::ui::sidebar::FileRef;

/// Most characters of a selection or file returned at once.
const MAX_TEXT: usize = 200_000;

impl NebulaApp {
    /// Starts the server if it is turned on (a token is created on first use).
    pub(super) fn start_mcp(&mut self, ctx: &egui::Context) {
        self.mcp = None;
        if !self.settings.mcp.enabled {
            return;
        }
        if self.settings.mcp.token.is_empty() {
            self.settings.mcp.token = crate::mcp::new_token();
            let _ = self.settings.save();
        }
        let ctx = ctx.clone();
        match crate::mcp::start(self.settings.mcp.port, self.settings.mcp.token.clone(), move || ctx.request_repaint()) {
            Ok(server) => self.mcp = Some(server),
            Err(e) => self.error(t!("MCP-Server konnte nicht starten: {e}", e = e)),
        }
    }

    /// The address of the running server.
    pub(super) fn mcp_url(&self) -> Option<String> {
        self.mcp.as_ref().map(|s| s.url())
    }

    /// Answers tool calls; called every frame.
    pub(super) fn poll_mcp(&mut self) {
        let Some(server) = &self.mcp else { return };
        let calls: Vec<crate::mcp::Call> = server.calls.try_iter().collect();
        for call in calls {
            let result = self.mcp_tool(&call.tool, &call.args);
            let _ = call.reply.send(result);
        }
    }

    fn mcp_tool(&mut self, tool: &str, args: &Value) -> Result<String, String> {
        let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
        match tool {
            "get_open_files" => {
                let files: Vec<Value> = self
                    .docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| json!({ "file": self.mcp_name(&d.source, &d.title()), "unsaved_changes": d.is_dirty(), "active": Some(i) == self.active }))
                    .collect();
                Ok(pretty(json!({ "project": self.sidebar.root, "files": files })))
            }
            "get_active_editor" => {
                let i = self.active.ok_or("No file is open.")?;
                let name = self.mcp_name(&self.docs[i].source, &self.docs[i].title());
                let d = &mut self.docs[i];
                let sel = d.primary_selection();
                let head = sel.head;
                let line = d.char_to_line(head);
                let column = head - d.line_to_char(line);
                let range = sel.range();
                let mut out = json!({
                    "file": name,
                    "language": d.language.id(),
                    "lines": d.len_lines(),
                    "cursor": { "line": line + 1, "column": column + 1 },
                    "unsaved_changes": d.is_dirty(),
                });
                if !range.is_empty() {
                    let text = d.text();
                    let selected: String = text.chars().skip(range.start).take((range.end - range.start).min(MAX_TEXT)).collect();
                    out["selection"] = json!({ "from_line": d.char_to_line(range.start) + 1, "to_line": d.char_to_line(range.end) + 1, "text": selected });
                }
                Ok(pretty(out))
            }
            "get_diagnostics" => {
                let only = args.get("path").and_then(Value::as_str).map(PathBuf::from);
                let severity = |s: u8| match s {
                    1 => "error",
                    2 => "warning",
                    3 => "info",
                    _ => "hint",
                };
                let mut out = Vec::new();
                for d in &self.docs {
                    if let Some(p) = &only
                        && !matches!(&d.source, DocSource::Local(dp) if same_path(dp, p))
                    {
                        continue;
                    }
                    for diag in &d.view.diagnostics {
                        let line = d.char_to_line(diag.range.start);
                        out.push(json!({
                            "file": self.mcp_name(&d.source, &d.title()),
                            "line": line + 1,
                            "column": diag.range.start - d.line_to_char(line) + 1,
                            "severity": severity(diag.severity),
                            "message": diag.message,
                        }));
                    }
                }
                Ok(if out.is_empty() { "No problems reported.".into() } else { pretty(Value::Array(out)) })
            }
            "get_file_text" => {
                let path = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("path is missing")?);
                let i = self.local_doc(&path).ok_or("This file is not open in Nebula; read it from disk instead.")?;
                let text = self.docs[i].text();
                Ok(text.chars().take(MAX_TEXT).collect())
            }
            "open_file" => {
                let path = PathBuf::from(args.get("path").and_then(Value::as_str).ok_or("path is missing")?);
                if !path.is_file() {
                    return Err(format!("{} does not exist.", path.display()));
                }
                let line = args.get("line").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
                self.open_file_at(FileRef::Local(path.clone()), line - 1, 0);
                Ok(format!("Opened {} at line {line}.", path.display()))
            }
            other => Err(format!("Unknown tool {other}")),
        }
    }

    /// A document as agents see it: local path, "server: path" or the tab title.
    fn mcp_name(&self, source: &DocSource, title: &str) -> String {
        match source {
            DocSource::Local(p) => p.display().to_string(),
            DocSource::Remote { site_id, path } => {
                format!("{} (server {}): {path}", title, self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default())
            }
            DocSource::Untitled(_) => format!("{title} (not saved yet)"),
        }
    }
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    let norm = |p: &std::path::Path| crate::ui::sidebar::normalize_path(p).to_string_lossy().to_lowercase();
    norm(a) == norm(b)
}
