//! "CSS-Klasse umbenennen": renames a class in all HTML, PHP, CSS and JS
//! files of the project, with a preview of every place; also lists classes
//! that are defined but used nowhere.

use std::path::PathBuf;

use eframe::egui::{self, Key, RichText};

use super::NebulaApp;
use crate::css_rename::{self, Kind};
use crate::editor::DocSource;
use crate::editor::document::{Change, EditKind};
use crate::ui::sidebar::FileRef;

/// Folders that hold no own code.
const SKIP_DIRS: &[&str] = &[".git", "node_modules", "vendor", ".svn", ".hg", ".idea", ".vs", "target", "dist", "build"];
const MAX_FILES: usize = 5000;
const MAX_SIZE: u64 = 1024 * 1024;

struct FileHits {
    path: PathBuf,
    rel: String,
    /// (line, 0-based, and the line's text).
    lines: Vec<(usize, String)>,
    count: usize,
}

#[derive(Default)]
pub(super) struct RenameClass {
    old: String,
    new: String,
    searched: String,
    hits: Vec<FileHits>,
    /// Defined but used nowhere: (class, file, line).
    unused: Option<Vec<(String, PathBuf, String, usize)>>,
    focus: bool,
}

/// Project files that can contain classes, with their text (open tabs win).
fn project_files(app: &NebulaApp) -> Vec<(PathBuf, Kind, String)> {
    let Some(root) = app.sidebar.root.clone() else { return Vec::new() };
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(e.path());
                }
                continue;
            }
            let Some(kind) = css_rename::kind_of(&name) else { continue };
            // Minified bundles are generated: not renamed in.
            if name.contains(".min.") || e.metadata().is_ok_and(|m| m.len() > MAX_SIZE) {
                continue;
            }
            let path = e.path();
            let open = app.docs.iter().find(|d| d.source == DocSource::Local(path.clone()));
            let text = match open {
                Some(d) => d.rope.to_string(),
                None => match std::fs::read(&path).ok().and_then(|b| String::from_utf8(b).ok()) {
                    Some(t) => t,
                    None => continue,
                },
            };
            out.push((path, kind, text));
            if out.len() >= MAX_FILES {
                return out;
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn line_of(text: &str, byte: usize) -> (usize, String) {
    let line = text[..byte].matches('\n').count();
    let start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
    let end = text[byte..].find('\n').map_or(text.len(), |i| byte + i);
    (line, text[start..end].trim().chars().take(140).collect())
}

impl NebulaApp {
    /// Opens the dialog, with the class at the cursor if there is one.
    pub(super) fn open_rename_class(&mut self) {
        if self.sidebar.root.is_none() {
            self.info(t!("Öffne zuerst einen Projektordner."));
            return;
        }
        let mut state = RenameClass { focus: true, ..Default::default() };
        if let Some(i) = self.active {
            let doc = &mut self.docs[i];
            if let Some(kind) = css_rename::kind_of(&doc.source.file_name()) {
                let text = doc.text();
                let head = doc.primary_selection().head;
                let byte = text.char_indices().nth(head).map_or(text.len(), |(b, _)| b);
                if let Some(name) = css_rename::name_at(&text, kind, byte) {
                    state.new = name.clone();
                    state.old = name;
                }
            }
        }
        self.rename_class = Some(state);
    }

    fn search_class(&self, state: &mut RenameClass) {
        state.searched = state.old.clone();
        state.hits.clear();
        if !css_rename::valid(&state.old) {
            return;
        }
        let root = self.sidebar.root.clone().unwrap_or_default();
        for (path, kind, text) in project_files(self) {
            let found = css_rename::occurrences(&text, kind, &state.old);
            if found.is_empty() {
                continue;
            }
            let mut lines: Vec<(usize, String)> = found.iter().map(|r| line_of(&text, r.start)).collect();
            lines.dedup_by_key(|(l, _)| *l);
            let rel = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            state.hits.push(FileHits { path, rel, lines, count: found.len() });
        }
    }

    fn find_unused(&self) -> Vec<(String, PathBuf, String, usize)> {
        let files = project_files(self);
        let root = self.sidebar.root.clone().unwrap_or_default();
        let mut used = std::collections::BTreeSet::new();
        for (_, kind, text) in &files {
            used.extend(css_rename::used(text, *kind));
        }
        // Names that also appear as a word in markup or scripts (built by PHP,
        // read from data) count as used: better a class too few in the list.
        let words: std::collections::HashSet<&str> = files
            .iter()
            .filter(|(_, k, _)| *k != Kind::Css)
            .flat_map(|(_, _, t)| t.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-')))
            .collect();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (path, kind, text) in &files {
            for (name, range) in css_rename::defined(text, *kind) {
                if used.contains(&name) || seen.contains(&name) {
                    continue;
                }
                seen.insert(name.clone());
                let in_words = words.contains(name.as_str())
                    // A <style> block's own file mentions the name in the selector itself.
                    && files.iter().any(|(p, k, t)| *k != Kind::Css && (p != path || t.matches(name.as_str()).count() > 1));
                if !in_words {
                    let rel = path.strip_prefix(&root).unwrap_or(path).to_string_lossy().replace('\\', "/");
                    out.push((name, path.clone(), rel, line_of(text, range.start).0));
                }
            }
        }
        out.sort();
        out
    }

    /// Renames in all files; open tabs get an undoable change, other files are written.
    fn apply_rename(&mut self, old: &str, new: &str) {
        let (mut places, mut files, mut failed) = (0, 0, Vec::new());
        for (path, kind, text) in project_files(self) {
            let found = css_rename::occurrences(&text, kind, old);
            if found.is_empty() {
                continue;
            }
            files += 1;
            places += found.len();
            if let Some(i) = self.docs.iter().position(|d| d.source == DocSource::Local(path.clone())) {
                let doc = &mut self.docs[i];
                let changes: Vec<Change> = found
                    .iter()
                    .map(|r| {
                        let start = text[..r.start].chars().count();
                        Change::new(start..start + old.chars().count(), new)
                    })
                    .collect();
                doc.apply(changes, None, EditKind::Other);
                continue;
            }
            let mut out = text.clone();
            for r in found.iter().rev() {
                out.replace_range(r.clone(), new);
            }
            if let Err(e) = crate::safe_write::write(&path, out.as_bytes()) {
                failed.push(format!("{}: {e}", path.display()));
            }
        }
        if failed.is_empty() {
            self.info(t!("„{old}“ heißt jetzt „{new}“: {n} Stellen in {f} Dateien. Geöffnete Dateien sind geändert, aber noch nicht gespeichert.", old = old, new = new, n = places, f = files));
        } else {
            self.error(format!("{}\n{}", t!("Nicht alle Dateien konnten geschrieben werden:"), failed.join("\n")));
        }
    }

    pub(super) fn rename_class_window(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.rename_class.take() else { return };
        if s.old != s.searched {
            self.search_class(&mut s);
        }
        let theme = self.theme.clone();
        let mut keep = true;
        let mut apply = false;
        let mut unused = false;
        let mut open: Option<(PathBuf, usize)> = None;
        super::modal(ctx, egui::Id::unique("rename-class")).show(ctx, |ui| {
            ui.set_width(760.0);
            ui.heading(t!("CSS-Klasse umbenennen"));
            ui.add_space(6.0);
            egui::Grid::new("rename-class-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label(t!("Klasse"));
                let r = ui.add(egui::TextEdit::singleline(&mut s.old).hint_text("btn-primary").desired_width(300.0));
                if s.focus {
                    r.request_focus();
                    s.focus = false;
                }
                ui.end_row();
                ui.label(t!("Neuer Name"));
                ui.add(egui::TextEdit::singleline(&mut s.new).desired_width(300.0));
                ui.end_row();
            });
            ui.add_space(6.0);
            let total: usize = s.hits.iter().map(|h| h.count).sum();
            if css_rename::valid(&s.old) {
                ui.label(RichText::new(t!("{n} Stellen in {f} Dateien", n = total, f = s.hits.len())).small().color(theme.text_faint));
            }
            egui::ScrollArea::vertical().id_salt("rename-class-hits").max_height(300.0).show(ui, |ui| {
                for h in &s.hits {
                    ui.label(RichText::new(format!("{}  ({})", h.rel, h.count)).strong());
                    for (line, text) in &h.lines {
                        let label = RichText::new(format!("{:>5}  {text}", line + 1)).monospace().size(12.0);
                        if ui.add(egui::Label::new(label).sense(egui::Sense::click()).truncate()).on_hover_text(t!("Öffnen")).clicked() {
                            open = Some((h.path.clone(), *line));
                        }
                    }
                    ui.add_space(4.0);
                }
            });
            if let Some(list) = &s.unused {
                ui.separator();
                ui.label(RichText::new(t!("Definiert, aber nirgends verwendet ({n}):", n = list.len())).strong());
                ui.label(RichText::new(t!("Klassen, die PHP oder JavaScript zur Laufzeit zusammensetzen, erkennt Nebula nicht – vor dem Löschen prüfen.")).small().color(theme.text_faint));
                egui::ScrollArea::vertical().id_salt("rename-class-unused").max_height(200.0).show(ui, |ui| {
                    for (name, path, rel, line) in list {
                        ui.horizontal(|ui| {
                            if ui.link(RichText::new(format!(".{name}")).monospace()).clicked() {
                                open = Some((path.clone(), *line));
                            }
                            ui.label(RichText::new(format!("{rel}:{}", line + 1)).small().color(theme.text_faint));
                        });
                    }
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = total > 0 && css_rename::valid(&s.new) && s.new != s.old;
                let enter = ok && ui.input(|i| i.key_pressed(Key::Enter));
                if ui.add_enabled(ok, egui::Button::new(t!("Umbenennen"))).clicked() || enter {
                    apply = true;
                }
                if ui.button(t!("Ungenutzte Klassen")).clicked() {
                    unused = true;
                }
                if ui.button(t!("Schließen")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    keep = false;
                }
                if !s.new.is_empty() && !css_rename::valid(&s.new) {
                    ui.label(RichText::new(t!("Kein gültiger Klassenname")).small().color(theme.danger));
                }
            });
        });
        if unused {
            s.unused = Some(self.find_unused());
        }
        if apply {
            let (old, new) = (s.old.clone(), s.new.clone());
            self.apply_rename(&old, &new);
            keep = false;
        }
        if let Some((path, line)) = open {
            self.open_file_at(FileRef::Local(path), line, 0);
        }
        if keep {
            self.rename_class = Some(s);
        }
    }
}
