//! Built-in "go to definition" (Strg+Klick, F12) when no extension knows one:
//! searches the document, the open files and the project in the background.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use eframe::egui::{self, Key, Modifiers, RichText};

use super::NebulaApp;
use crate::editor::DocSource;
use crate::editor::definition::{self, Found, Target};
use crate::editor::document::Selection;
use crate::lang::Language;

/// Folders never searched (besides what `.gitignore` excludes).
const SKIP_DIRS: &[&str] = &[".git", ".svn", ".hg", "node_modules"];
const MAX_FILES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 2 << 20;
const MAX_RESULTS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Place {
    /// An open document (also server files).
    Doc(u64),
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub(super) struct Hit {
    place: Place,
    /// File name or path relative to the project.
    label: String,
    found: Found,
}

pub(super) struct DefinitionList {
    name: String,
    hits: Vec<Hit>,
    selected: usize,
}

/// A running search: what is searched for, and the results to come.
pub(super) struct DefinitionJob {
    name: String,
    rx: Receiver<Vec<Hit>>,
}

impl NebulaApp {
    /// Looks up the definition of what is at `pos` in document `doc_id`.
    pub(super) fn builtin_definition(&mut self, doc_id: u64, pos: usize) {
        let Some(i) = self.doc_index(doc_id) else { return };
        let doc = &self.docs[i];
        let line = doc.char_to_line(pos);
        let col = pos - doc.line_to_char(line);
        let Some(target) = definition::target_at(&doc.line_text(line), col, doc.language) else {
            self.info(t!("Keine Definition gefunden."));
            return;
        };
        let root = self.sidebar.root.clone();
        let doc_path = match &doc.source {
            DocSource::Local(p) => Some(p.clone()),
            _ => None,
        };
        if let Target::Path(path) = &target {
            match definition::resolve_path(path, doc_path.as_deref().and_then(Path::parent), root.as_deref()) {
                Some(file) => self.open_local(&file),
                None => self.info(t!("{path} wurde im Projekt nicht gefunden.", path = path)),
            }
            return;
        }

        // Open documents are searched with their current text.
        let language = doc.language;
        let mut texts: Vec<(Place, String, Language, std::sync::Arc<String>)> = Vec::new();
        for d in &mut self.docs {
            if !definition::searched(&target, language, d.language) && d.id != doc_id {
                continue;
            }
            let place = match &d.source {
                DocSource::Local(p) => Place::File(p.clone()),
                _ if d.id == doc_id => Place::Doc(d.id),
                _ => continue,
            };
            let label = match &place {
                Place::File(p) => relative(p, root.as_deref()),
                Place::Doc(_) => d.title(),
            };
            // The current document first.
            let entry = (place, label, d.language, d.text());
            if d.id == doc_id { texts.insert(0, entry) } else { texts.push(entry) }
        }
        let name = match &target {
            Target::Selector { prefix, name } => format!("{prefix}{name}"),
            Target::Symbol(name) | Target::Path(name) => name.clone(),
        };
        let here = (doc_path.map_or(Place::Doc(doc_id), Place::File), line);
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(search(&target, language, texts, root.as_deref(), &here));
        });
        self.definition_job = Some(DefinitionJob { name, rx });
    }

    pub(super) fn poll_definition(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.definition_job else { return };
        let Ok(hits) = job.rx.try_recv() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
            return;
        };
        let name = self.definition_job.take().unwrap().name;
        match hits.len() {
            0 => self.info(t!("Keine Definition von „{name}“ gefunden.", name = name)),
            1 => self.goto_hit(&hits[0]),
            _ => self.dialog = Some(super::Dialog::Definitions(Box::new(DefinitionList { name, hits, selected: 0 }))),
        }
    }

    fn goto_hit(&mut self, hit: &Hit) {
        match &hit.place {
            Place::Doc(id) => {
                let Some(i) = self.doc_index(*id) else { return };
                self.activate(i);
            }
            Place::File(path) => self.open_local(path),
        }
        let Some(i) = self.active else { return };
        let doc = &mut self.docs[i];
        let line = hit.found.line.min(doc.len_lines().saturating_sub(1));
        let pos = (doc.line_to_char(line) + hit.found.column).min(doc.line_end(line));
        doc.set_selection(Selection::cursor(pos));
        doc.view.center_line = Some(line);
        self.focus_editor = true;
    }

    /// The list of definitions to choose from; returns false when closed.
    pub(super) fn definitions_dialog(&mut self, ctx: &egui::Context, list: &mut DefinitionList) -> bool {
        let mut keep = true;
        let mut chosen = None;
        let theme = self.theme.clone();
        super::modal(ctx, egui::Id::unique("definitions")).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.label(RichText::new(t!("{n} Definitionen von „{name}“", n = list.hits.len(), name = list.name)).strong());
            ui.add_space(6.0);
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    list.selected = (list.selected + 1).min(list.hits.len() - 1);
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    list.selected = list.selected.saturating_sub(1);
                }
            });
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (n, hit) in list.hits.iter().enumerate() {
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&hit.found.preview, 0.0, egui::TextFormat::simple(egui::FontId::monospace(13.0), ui.visuals().text_color()));
                    job.append(
                        &format!("   {}:{}", hit.label, hit.found.line + 1),
                        0.0,
                        egui::TextFormat::simple(egui::FontId::proportional(11.5), theme.text_faint),
                    );
                    let r = ui.add(egui::Button::selectable(n == list.selected, job).truncate().min_size(egui::vec2(620.0, 26.0)));
                    if n == list.selected {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        chosen = Some(n);
                    }
                }
            });
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                chosen = Some(list.selected);
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                keep = false;
            }
        });
        if let Some(n) = chosen {
            let hit = list.hits[n].clone();
            self.dialog = None;
            self.goto_hit(&hit);
            return false;
        }
        keep
    }
}

/// `path` relative to `root` with `/`, else the file name.
fn relative(path: &Path, root: Option<&Path>) -> String {
    match root.and_then(|r| path.strip_prefix(r).ok()) {
        Some(rel) => rel.to_string_lossy().replace('\\', "/"),
        None => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

/// Definitions of `target` in the given texts and then in the project's files;
/// the place the request came from (`here`: place and line) is left out.
fn search(target: &Target, language: Language, texts: Vec<(Place, String, Language, std::sync::Arc<String>)>, root: Option<&Path>, here: &(Place, usize)) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let add = |place: Place, label: String, file_language: Language, text: &str, hits: &mut Vec<Hit>| {
        for found in definition::find(text, file_language, target) {
            if place == here.0 && found.line == here.1 {
                continue;
            }
            hits.push(Hit { place: place.clone(), label: label.clone(), found });
        }
    };
    for (place, label, file_language, text) in texts {
        if let Place::File(p) = &place {
            seen.push(p.clone());
        }
        add(place, label, file_language, &text, &mut hits);
    }
    let Some(root) = root else { return hits };
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .filter_entry(|e| !(e.depth() > 0 && e.file_type().is_some_and(|t| t.is_dir()) && SKIP_DIRS.iter().any(|s| e.file_name() == *s)))
        .build();
    for entry in walk.flatten().filter(|e| e.file_type().is_some_and(|t| t.is_file())).take(MAX_FILES) {
        if hits.len() >= MAX_RESULTS {
            break;
        }
        let path = crate::ui::sidebar::normalize_path(entry.path());
        let file_language = Language::from_path(&path.to_string_lossy());
        if !definition::searched(target, language, file_language)
            || seen.iter().any(|s| s == &path)
            || entry.metadata().is_ok_and(|m| m.len() > MAX_FILE_BYTES)
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let label = relative(&path, Some(root));
        add(Place::File(path), label, file_language, &text, &mut hits);
    }
    hits.truncate(MAX_RESULTS);
    hits
}
