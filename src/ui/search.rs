//! Project-wide search (and replace) in the local project folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Receiver;
use eframe::egui::{self, Align2, FontId, RichText, Sense, Ui, pos2, vec2};
use regex::{Regex, RegexBuilder};

use super::icons;
use crate::theme::Theme;

const MAX_MATCHES: usize = 5000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const SKIP_DIRS: &[&str] = &[".git", "node_modules", "vendor", ".svn", ".hg", "target", ".idea", ".vs", "dist", ".cache"];

#[derive(Clone)]
pub struct LineMatch {
    /// Zero-based line number.
    pub line: usize,
    pub text: String,
    /// Byte ranges within `text`.
    pub ranges: Vec<std::ops::Range<usize>>,
}

#[derive(Clone)]
pub struct FileMatches {
    pub path: PathBuf,
    pub lines: Vec<LineMatch>,
}

/// What the search panel asks the app to do.
pub enum SearchAction {
    Open { path: PathBuf, line: usize, column: usize },
    ReplaceAll { files: Vec<PathBuf>, regex: Regex, replacement: String },
}

#[derive(Default)]
pub struct SearchPanel {
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    pub show_replace: bool,
    pub results: Vec<FileMatches>,
    pub collapsed: std::collections::HashSet<PathBuf>,
    pub error: Option<String>,
    pub focus_query: bool,
    running: Option<(Receiver<FileMatches>, Arc<AtomicBool>)>,
    truncated: bool,
}

impl SearchPanel {
    pub fn build_regex(&self) -> Result<Regex, String> {
        let mut p = if self.use_regex { self.query.clone() } else { regex::escape(&self.query) };
        if self.whole_word {
            p = format!(r"\b(?:{p})\b");
        }
        RegexBuilder::new(&p)
            .case_insensitive(!self.case_sensitive)
            .build()
            .map_err(|e| e.to_string().lines().last().unwrap_or("Ungültiger Ausdruck").to_string())
    }

    pub fn start(&mut self, root: &Path, ctx: &egui::Context) {
        if let Some((_, cancel)) = self.running.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.results.clear();
        self.collapsed.clear();
        self.truncated = false;
        self.error = None;
        if self.query.is_empty() {
            return;
        }
        let re = match self.build_regex() {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let root = root.to_path_buf();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut stack = vec![root];
            let mut total = 0;
            while let Some(dir) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                let mut entries: Vec<_> = rd.flatten().collect();
                entries.sort_by_key(|e| e.file_name());
                for e in entries {
                    if flag.load(Ordering::Relaxed) || total >= MAX_MATCHES {
                        return;
                    }
                    let Ok(ft) = e.file_type() else { continue };
                    let name = e.file_name();
                    if ft.is_dir() {
                        if !SKIP_DIRS.iter().any(|s| name == *s) {
                            stack.push(e.path());
                        }
                        continue;
                    }
                    if e.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                        continue;
                    }
                    let Ok(bytes) = std::fs::read(e.path()) else { continue };
                    if bytes[..bytes.len().min(8192)].contains(&0) {
                        continue;
                    }
                    let text = String::from_utf8_lossy(&bytes);
                    let mut lines = Vec::new();
                    for (n, line) in text.lines().enumerate() {
                        let ranges: Vec<_> = re.find_iter(line).filter(|m| !m.is_empty()).map(|m| m.range()).collect();
                        if !ranges.is_empty() {
                            total += ranges.len();
                            lines.push(LineMatch { line: n, text: line.chars().take(400).collect(), ranges });
                        }
                    }
                    if !lines.is_empty() {
                        if tx.send(FileMatches { path: e.path(), lines }).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                }
            }
            ctx.request_repaint();
        });
        self.running = Some((rx, cancel));
    }

    fn poll(&mut self) {
        if let Some((rx, _)) = &self.running {
            loop {
                match rx.try_recv() {
                    Ok(m) => self.results.push(m),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        self.truncated = self.match_count() >= MAX_MATCHES;
                        self.running = None;
                        break;
                    }
                }
            }
        }
    }

    pub fn match_count(&self) -> usize {
        self.results.iter().flat_map(|f| &f.lines).map(|l| l.ranges.len()).sum()
    }

    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, root: Option<&Path>, actions: &mut Vec<SearchAction>) {
        self.poll();
        let Some(root) = root else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Öffne einen Ordner, um im Projekt zu suchen.").color(theme.text_faint)));
            return;
        };
        ui.add_space(8.0);
        let resp = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Im Projekt suchen").desired_width(f32::INFINITY));
        if std::mem::take(&mut self.focus_query) {
            resp.request_focus();
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut restart = enter;
        ui.horizontal(|ui| {
            restart |= ui.toggle_value(&mut self.case_sensitive, "Aa").on_hover_text("Groß-/Kleinschreibung").changed();
            restart |= ui.toggle_value(&mut self.whole_word, "W").on_hover_text("Ganzes Wort").changed();
            restart |= ui.toggle_value(&mut self.use_regex, ".*").on_hover_text("Regulärer Ausdruck").changed();
            ui.toggle_value(&mut self.show_replace, "Ersetzen");
        });
        if self.show_replace {
            ui.add(egui::TextEdit::singleline(&mut self.replacement).hint_text("Ersetzen durch").desired_width(f32::INFINITY));
            let files: Vec<PathBuf> = self.results.iter().map(|f| f.path.clone()).collect();
            let label = format!("Alle ersetzen ({} in {} Dateien)", self.match_count(), files.len());
            if ui.add_enabled(!files.is_empty() && self.running.is_none(), egui::Button::new(label)).clicked()
                && let Ok(regex) = self.build_regex()
            {
                actions.push(SearchAction::ReplaceAll { files, regex, replacement: self.replacement.clone() });
            }
        }
        if restart {
            self.start(root, ui.ctx());
        }
        ui.add_space(4.0);
        let summary = match (&self.error, self.running.is_some()) {
            (Some(e), _) => RichText::new(e).color(theme.danger),
            (None, true) => RichText::new(format!("Suche… {} Treffer", self.match_count())).color(theme.text_faint),
            (None, false) if self.query.is_empty() => RichText::new(""),
            (None, false) => RichText::new(format!(
                "{} Treffer in {} Dateien{}",
                self.match_count(),
                self.results.len(),
                if self.truncated { " (gekürzt)" } else { "" }
            ))
            .color(theme.text_faint),
        };
        ui.label(summary.small());
        ui.add_space(4.0);

        egui::ScrollArea::vertical().id_salt("search-results").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for file in &self.results {
                let rel = file.path.strip_prefix(root).unwrap_or(&file.path).to_string_lossy().replace('\\', "/");
                let open = !self.collapsed.contains(&file.path);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, theme.hover);
                }
                let cy = rect.center().y;
                icons::chevron(ui.painter(), pos2(rect.left() + 8.0, cy), open, theme.text_faint);
                icons::file(ui.painter(), pos2(rect.left() + 24.0, cy), &rel);
                ui.painter().with_clip_rect(rect).text(
                    pos2(rect.left() + 36.0, cy),
                    Align2::LEFT_CENTER,
                    &rel,
                    FontId::proportional(12.5),
                    theme.foreground,
                );
                if resp.clicked() {
                    if open {
                        self.collapsed.insert(file.path.clone());
                    } else {
                        self.collapsed.remove(&file.path);
                    }
                }
                if !open {
                    continue;
                }
                for m in &file.lines {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 4.0, theme.hover);
                    }
                    // Show the line around the first match.
                    let first = m.ranges[0].clone();
                    let skip = first.start.saturating_sub(24);
                    let skip = (0..=skip).rev().find(|i| m.text.is_char_boundary(*i)).unwrap_or(0);
                    let mut job = egui::text::LayoutJob::default();
                    let font = FontId::monospace(11.5);
                    let mut pos = skip;
                    for r in m.ranges.iter().filter(|r| r.start >= skip) {
                        if r.start > pos {
                            job.append(trim_if(&m.text[pos..r.start], pos == skip), 0.0, egui::TextFormat::simple(font.clone(), theme.text_muted));
                        }
                        let mut fmt = egui::TextFormat::simple(font.clone(), theme.foreground);
                        fmt.background = theme.find_match;
                        job.append(&m.text[r.start..r.end], 0.0, fmt);
                        pos = r.end;
                    }
                    if pos < m.text.len() {
                        job.append(&m.text[pos..], 0.0, egui::TextFormat::simple(font.clone(), theme.text_muted));
                    }
                    let galley = ui.painter().layout_job(job);
                    ui.painter().with_clip_rect(rect).galley(pos2(rect.left() + 36.0, rect.center().y - galley.size().y / 2.0), galley, theme.foreground);
                    ui.painter().text(
                        pos2(rect.left() + 30.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        (m.line + 1).to_string(),
                        FontId::monospace(10.5),
                        theme.text_faint,
                    );
                    if resp.clicked() {
                        let column = m.text[..first.start].chars().count();
                        actions.push(SearchAction::Open { path: file.path.clone(), line: m.line, column });
                    }
                }
            }
        });
    }
}

/// Trims leading whitespace of the first shown segment only.
fn trim_if(s: &str, cond: bool) -> &str {
    if cond { s.trim_start() } else { s }
}
