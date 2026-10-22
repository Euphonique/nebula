//! "Quick open" (Ctrl+P): project files and files in loaded server folders, with a `:line` suffix.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Key, Modifiers, text::LayoutJob};

use super::{Dialog, NebulaApp, modal};
use crate::ui::sidebar::FileRef;

pub(super) struct QuickOpenState {
    query: String,
    items: Arc<Vec<QuickItem>>,
    selected: usize,
    focus: bool,
}

struct QuickItem {
    /// Path relative to the project or the server's start folder; what the query matches.
    label: String,
    /// Server name for files on a server.
    site: Option<String>,
    file: FileRef,
}

/// Lists files below `root`, skipping VCS and dependency folders.
fn collect_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[".git", "node_modules", "vendor", ".svn", ".hg", "target", ".idea", ".vs"];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !SKIP.iter().any(|s| name == *s) {
                    stack.push(e.path());
                }
            } else {
                out.push(e.path());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// Splits `name.php:42` or `name.php:42:5` into the file query and a 1-based line/column.
fn split_position(query: &str) -> (&str, Option<(usize, Option<usize>)>) {
    let number = |s: &str| (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten();
    let q = query.trim();
    if let Some((rest, last)) = q.rsplit_once(':')
        && let Some(n) = number(last)
    {
        if let Some((file, line)) = rest.rsplit_once(':')
            && let Some(l) = number(line)
        {
            return (file, Some((l, Some(n))));
        }
        return (rest, Some((n, None)));
    }
    (q, None)
}

/// Subsequence match scored by contiguity and filename hits; returns item indices, best first.
fn fuzzy_filter(labels: &[&str], query: &str, max: usize) -> Vec<usize> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let mut scored: Vec<(i64, usize)> = labels
        .iter()
        .enumerate()
        .filter_map(|(idx, label)| {
            if q.is_empty() {
                return Some((0, idx));
            }
            let lower: Vec<char> = label.to_lowercase().chars().collect();
            let name_start = lower.iter().rposition(|&c| c == '/').map_or(0, |i| i + 1);
            // Greedy matching from the start can spend letters on folder names
            // ("css/style.css"), so the file name alone is tried as well.
            let score = [0, name_start].into_iter().filter_map(|from| greedy_score(&lower, &q, from, name_start)).max()?;
            Some((score * 100 - label.len() as i64, idx))
        })
        .collect();
    // Stable: on equal scores the earlier item (project before server) wins.
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(max).map(|(_, i)| i).collect()
}

/// Score of matching `q` as a subsequence of `text[from..]`; `None` if it doesn't match.
fn greedy_score(text: &[char], q: &[char], from: usize, name_start: usize) -> Option<i64> {
    let mut score = 0i64;
    let mut qi = 0;
    let mut last: Option<usize> = None;
    for (i, &c) in text.iter().enumerate().skip(from) {
        if qi < q.len() && c == q[qi] {
            score += 1;
            if last == Some(i.wrapping_sub(1)) {
                score += 5;
            }
            if i >= name_start {
                score += 3;
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

impl NebulaApp {
    pub(super) fn open_quick_open(&mut self) {
        let mut items = Vec::new();
        if let Some(root) = self.sidebar.root.clone() {
            let mut files: Vec<QuickItem> = collect_files(&root, 20_000)
                .into_iter()
                .map(|p| QuickItem {
                    label: p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('\\', "/"),
                    site: None,
                    file: FileRef::Local(p),
                })
                .collect();
            files.sort_by(|a, b| a.label.cmp(&b.label));
            items.extend(files);
        }
        // Files in server folders that have been listed in the sidebar.
        let mut remote: Vec<QuickItem> = Vec::new();
        for (site_id, tree) in &self.sidebar.remote {
            let Some(site) = self.sites.get(site_id) else { continue };
            let home = tree.home.clone().unwrap_or_default();
            let prefix = format!("{}/", home.trim_end_matches('/'));
            for entry in tree.dirs.values().flatten().filter(|e| !e.is_dir) {
                let label = entry.path.strip_prefix(&prefix).unwrap_or(&entry.path).to_string();
                remote.push(QuickItem {
                    label,
                    site: Some(site.display_name()),
                    file: FileRef::Remote { site_id: site_id.clone(), path: entry.path.clone() },
                });
            }
        }
        remote.sort_by(|a, b| a.site.cmp(&b.site).then_with(|| a.label.cmp(&b.label)));
        items.extend(remote);
        if items.is_empty() && self.active.is_none() {
            self.info("Öffne zuerst einen Ordner oder verbinde einen Server, um Dateien schnell zu finden.");
            return;
        }
        self.dialog = Some(Dialog::QuickOpen(QuickOpenState { query: String::new(), items: Arc::new(items), selected: 0, focus: true }));
    }

    /// Opens a local or server file and puts the cursor at a 0-based line/column
    /// (for server files once the download has arrived).
    pub(super) fn open_file_at(&mut self, file: FileRef, line: usize, column: usize) {
        match file {
            FileRef::Local(path) => {
                self.open_local(&path);
                // open_local activates the file (or the already open tab).
                if let Some(i) = self.active
                    && matches!(&self.docs[i].source, crate::editor::DocSource::Local(p) if p.file_name() == path.file_name())
                {
                    self.goto_line_column(i, line, column);
                }
            }
            FileRef::Remote { site_id, path } => {
                let source = crate::editor::DocSource::Remote { site_id: site_id.clone(), path: path.clone() };
                if let Some(i) = self.docs.iter().position(|d| d.source == source) {
                    self.activate(i);
                    self.goto_line_column(i, line, column);
                } else {
                    // Applied once the download arrives.
                    self.pending_lines.insert((site_id.clone(), path.clone()), (line, column));
                    self.open_remote(&site_id, &path);
                }
            }
        }
    }

    /// Returns whether the dialog stays open.
    pub(super) fn quick_open_dialog(&mut self, ctx: &egui::Context, state: &mut QuickOpenState) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        // (file, position) – `None` file: jump within the active tab.
        let mut chosen: Option<(Option<FileRef>, Option<(usize, Option<usize>)>)> = None;
        let has_active = self.active.is_some();
        modal(ctx, egui::Id::unique("quick-open")).show(ctx, |ui| {
            ui.set_width(560.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("Dateiname eingeben… (:Zeile springt zur Zeile)")
                    .desired_width(f32::INFINITY),
            );
            if state.focus || resp.changed() {
                resp.request_focus();
            }
            if resp.changed() {
                state.selected = 0;
            }
            let (file_query, position) = split_position(&state.query);
            let only_line = file_query.is_empty() && position.is_some();
            let results = if only_line {
                Vec::new()
            } else {
                let labels: Vec<&str> = state.items.iter().map(|i| i.label.as_str()).collect();
                fuzzy_filter(&labels, file_query, 14)
            };
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    state.selected = (state.selected + 1).min(results.len().saturating_sub(1));
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    state.selected = state.selected.saturating_sub(1);
                }
            });
            ui.add_space(4.0);
            if only_line {
                let (line, _) = position.unwrap_or_default();
                let text = if has_active { format!("Im aktuellen Tab zu Zeile {line} springen") } else { "Kein Tab geöffnet".into() };
                ui.label(egui::RichText::new(text).color(theme.gutter_fg));
            } else if results.is_empty() {
                ui.label(egui::RichText::new("Keine passende Datei").color(theme.gutter_fg));
            }
            let font = egui::TextStyle::Button.resolve(ui.style());
            for (row, &idx) in results.iter().enumerate() {
                let item = &state.items[idx];
                let mut job = LayoutJob::default();
                job.append(&item.label, 0.0, egui::TextFormat::simple(font.clone(), theme.foreground));
                if let Some(site) = &item.site {
                    job.append(&format!("   {site}"), 0.0, egui::TextFormat::simple(font.clone(), theme.gutter_fg));
                }
                if ui.selectable_label(row == state.selected, job).clicked() {
                    chosen = Some((Some(item.file.clone()), position));
                }
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                if only_line {
                    if has_active {
                        chosen = Some((None, position));
                    }
                } else if let Some(&idx) = results.get(state.selected) {
                    chosen = Some((Some(state.items[idx].file.clone()), position));
                }
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                keep = false;
            }
        });
        state.focus = false;
        if let Some((file, position)) = chosen {
            keep = false;
            // 1-based input, 0-based internally.
            let (line, column) = position.map_or((None, 0), |(l, c)| (Some(l.saturating_sub(1)), c.map_or(0, |c| c.saturating_sub(1))));
            match (file, line) {
                (Some(file), Some(line)) => self.open_file_at(file, line, column),
                (Some(FileRef::Local(p)), None) => self.open_local(&p),
                (Some(FileRef::Remote { site_id, path }), None) => self.open_remote(&site_id, &path),
                (None, Some(line)) => {
                    if let Some(i) = self.active {
                        self.goto_line_column(i, line, column);
                    }
                }
                (None, None) => {}
            }
            self.focus_editor = true;
        }
        keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_suffix() {
        assert_eq!(split_position("index.php"), ("index.php", None));
        assert_eq!(split_position("index.php:42"), ("index.php", Some((42, None))));
        assert_eq!(split_position("index.php:42:7"), ("index.php", Some((42, Some(7)))));
        assert_eq!(split_position(":12"), ("", Some((12, None))));
        assert_eq!(split_position("a:b"), ("a:b", None));
        assert_eq!(split_position("index.php:"), ("index.php:", None));
    }

    #[test]
    fn fuzzy_prefers_file_names_and_keeps_order_on_ties() {
        let labels = ["src/app.rs", "css/style.css", "app/style.css"];
        assert_eq!(fuzzy_filter(&labels, "style", 10), [1, 2]);
        assert_eq!(fuzzy_filter(&labels, "apprs", 10), [0]);
        assert_eq!(fuzzy_filter(&labels, "", 2), [0, 1]);
    }
}
