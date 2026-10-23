//! Large local files are read and prepared in the background; their tab
//! shows "Lade …" until then, so the window keeps responding.

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use eframe::egui;

use super::{MAX_OPEN_BYTES, NebulaApp, is_binary};
use crate::editor::{DocSource, Document};

/// Text files from this size on are loaded in the background.
pub(super) const BACKGROUND_BYTES: u64 = 16 * 1024 * 1024;

pub(super) enum Loaded {
    Text(Box<Document>),
    Binary(Vec<u8>),
    Failed(String),
}

/// A file being loaded for the placeholder tab with this id.
pub(super) struct Loading {
    doc_id: u64,
    rx: Receiver<Loaded>,
}

impl NebulaApp {
    /// Opens a placeholder tab and loads `path` into it in the background.
    pub(super) fn open_local_in_background(&mut self, path: PathBuf, size: u64) {
        let mut placeholder = Document::new(DocSource::Local(path.clone()), "");
        placeholder.read_only = true;
        placeholder.loading = Some(size);
        let doc_id = placeholder.id;
        self.add_doc(placeholder);
        let history = self.settings.editor.local_history.then_some(self.settings.editor.history_max);
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let loaded = match std::fs::read(&path) {
                Err(e) => Loaded::Failed(e.to_string()),
                Ok(bytes) if is_binary(&bytes[..bytes.len().min(8192)]) => Loaded::Binary(bytes),
                Ok(bytes) => {
                    let source = DocSource::Local(path.clone());
                    if let Some(max) = history.filter(|_| bytes.len() <= MAX_OPEN_BYTES) {
                        crate::history::record(&source, &bytes, crate::history::Kind::Opened, max);
                    }
                    let mut doc = Document::from_bytes(source, &bytes);
                    doc.disk_stamp = crate::editor::document::DiskStamp::of(&path);
                    doc.config = crate::editorconfig::for_file(&path);
                    // Large text: viewable, but editing would be too slow.
                    doc.read_only = bytes.len() > MAX_OPEN_BYTES;
                    Loaded::Text(Box::new(doc))
                }
            };
            let _ = tx.send(loaded);
        });
        self.loadings.push(Loading { doc_id, rx });
    }

    /// Puts finished loads into their tabs (closed tabs drop theirs).
    pub(super) fn poll_loadings(&mut self, ctx: &egui::Context) {
        if !self.loadings.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let mut i = 0;
        while i < self.loadings.len() {
            let Ok(loaded) = self.loadings[i].rx.try_recv() else {
                i += 1;
                continue;
            };
            let doc_id = self.loadings.remove(i).doc_id;
            let Some(index) = self.doc_index(doc_id) else { continue };
            let old = &self.docs[index];
            let (source, pinned) = (old.source.clone(), old.pinned);
            match loaded {
                Loaded::Text(doc) => {
                    let mut doc = *doc;
                    doc.id = doc_id;
                    doc.pinned = pinned;
                    let (read_only, len) = (doc.read_only, doc.len_chars());
                    self.docs[index] = doc;
                    if let DocSource::Local(path) = source {
                        self.settings.recent_files.retain(|p| p != &path);
                        self.settings.recent_files.insert(0, path);
                        self.settings.recent_files.truncate(15);
                    }
                    if read_only {
                        self.info(t!("{name} ist sehr groß ({value} MB) und wurde schreibgeschützt geöffnet.", name = self.docs[index].title(), value = len / 1024 / 1024));
                    }
                }
                Loaded::Binary(bytes) => {
                    let mut doc = Document::new(source, "");
                    doc.id = doc_id;
                    doc.pinned = pinned;
                    doc.read_only = true;
                    doc.hex = Some(Arc::new(crate::hex_view::HexData::memory(bytes)));
                    self.docs[index] = doc;
                }
                Loaded::Failed(e) => {
                    let name = self.docs[index].title();
                    self.docs.remove(index);
                    self.fix_active_after_remove(index);
                    self.error(t!("{path} konnte nicht geöffnet werden: {e}", path = name, e = e));
                }
            }
        }
    }
}

/// The placeholder shown while a file loads.
pub(super) fn show_loading(ui: &mut egui::Ui, rect: egui::Rect, size: u64, theme: &crate::theme::Theme) {
    let painter = ui.painter_at(rect);
    let text = t!("Lade … ({value} MB)", value = size / 1024 / 1024);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(14.0), theme.text_faint);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}
