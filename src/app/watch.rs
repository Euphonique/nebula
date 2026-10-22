//! Noticing changes that other programs make to open files and to the
//! project folder (Git checkouts, build tools, other editors).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use notify::Watcher as _;

use super::{Dialog, MAX_OPEN_BYTES, NebulaApp, is_binary};
use crate::editor::DocSource;
use crate::editor::document::DiskStamp;

/// How often open files are compared with the disk while the window is focused.
const CHECK_EVERY: Duration = Duration::from_secs(2);

pub(super) struct DiskWatch {
    /// Recursive watcher on the project folder.
    watcher: Option<(PathBuf, notify::RecommendedWatcher)>,
    tx: Sender<PathBuf>,
    rx: Receiver<PathBuf>,
    last_check: Option<Instant>,
    was_focused: bool,
}

impl Default for DiskWatch {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { watcher: None, tx, rx, last_check: None, was_focused: true }
    }
}

impl DiskWatch {
    /// Watches `root` (or nothing), replacing the previous watcher.
    fn watch(&mut self, root: Option<&Path>, ctx: &egui::Context) {
        if self.watcher.as_ref().map(|(p, _)| p.as_path()) == root {
            return;
        }
        self.watcher = None;
        let Some(root) = root else { return };
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res
                && !matches!(event.kind, notify::EventKind::Access(_))
            {
                for p in event.paths {
                    let _ = tx.send(p);
                }
                ctx.request_repaint_after(Duration::from_millis(150));
            }
        });
        if let Ok(mut w) = watcher
            && w.watch(root, notify::RecursiveMode::Recursive).is_ok()
        {
            self.watcher = Some((root.to_path_buf(), w));
        }
    }
}

/// Inside a `.git` folder: only Git's own bookkeeping.
fn in_git_dir(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}

impl NebulaApp {
    /// Refreshes changed folders in the sidebar and compares open files with the disk.
    pub(super) fn poll_disk(&mut self, ctx: &egui::Context) {
        self.disk.watch(self.sidebar.root.as_deref(), ctx);
        let mut changed_docs = false;
        let mut config_changed = false;
        let mut dirs = Vec::new();
        while let Ok(path) = self.disk.rx.try_recv() {
            if in_git_dir(&path) {
                continue;
            }
            if let Some(parent) = path.parent()
                && !dirs.iter().any(|d: &PathBuf| d == parent)
            {
                dirs.push(parent.to_path_buf());
            }
            dirs.push(path.clone());
            config_changed |= path.file_name().is_some_and(|n| n == ".editorconfig");
            changed_docs |= self.docs.iter().any(|d| d.source == DocSource::Local(path.clone()));
        }
        for d in &dirs {
            self.sidebar.refresh_local(Some(d));
        }
        if config_changed {
            for doc in &mut self.docs {
                if let DocSource::Local(p) = &doc.source {
                    doc.config = crate::editorconfig::for_file(p);
                }
            }
        }

        let focused = ctx.input(|i| i.focused);
        let regained = focused && !self.disk.was_focused;
        self.disk.was_focused = focused;
        let due = focused && self.disk.last_check.is_none_or(|t| t.elapsed() >= CHECK_EVERY);
        if changed_docs || regained || due {
            self.disk.last_check = Some(Instant::now());
            self.check_open_files();
        }
        if focused {
            ctx.request_repaint_after(CHECK_EVERY);
        }
    }

    /// Reloads open files that changed on disk, or asks when they have unsaved edits.
    fn check_open_files(&mut self) {
        for i in 0..self.docs.len() {
            let DocSource::Local(path) = self.docs[i].source.clone() else { continue };
            let doc = &mut self.docs[i];
            if doc.saving {
                continue;
            }
            let Some(now) = DiskStamp::of(&path) else {
                // Deleting and recreating (Git, some editors) briefly leaves no file.
                doc.disk_missing = doc.disk_missing.saturating_add(1);
                if doc.disk_missing == 2 {
                    doc.mark_unsaved();
                    let name = doc.title();
                    self.error(format!("„{name}“ wurde gelöscht oder umbenannt. Speichern legt die Datei neu an."));
                }
                continue;
            };
            let came_back = doc.disk_missing >= 2;
            doc.disk_missing = 0;
            let Some(old) = doc.disk_stamp else {
                doc.disk_stamp = Some(now);
                continue;
            };
            if old == now && !came_back {
                continue;
            }
            // Hex view: show the new content without reading a possibly huge file here.
            if doc.hex.is_some() {
                doc.disk_stamp = Some(now);
                let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                doc.hex = Some(std::sync::Arc::new(match len > MAX_OPEN_BYTES as u64 {
                    true => crate::hex_view::HexData::file(path.clone(), len),
                    false => crate::hex_view::HexData::memory(std::fs::read(&path).unwrap_or_default()),
                }));
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let doc = &mut self.docs[i];
            // An image changed on disk: show the new picture.
            if doc.image.is_some() {
                doc.disk_stamp = Some(now);
                if let Ok(image) = crate::image_view::decode(&bytes) {
                    doc.image = Some(std::sync::Arc::new(image));
                }
                continue;
            }
            if bytes.len() > MAX_OPEN_BYTES || is_binary(&bytes) {
                doc.disk_stamp = Some(now);
                continue;
            }
            if bytes == doc.to_bytes() {
                doc.disk_stamp = Some(now);
                if came_back {
                    doc.mark_saved();
                }
                continue;
            }
            if !doc.is_dirty() {
                let source = doc.source.clone();
                self.record_history(&source, &bytes, crate::history::Kind::Opened);
                let doc = &mut self.docs[i];
                doc.reload_bytes(&bytes);
                doc.disk_stamp = Some(now);
                let name = doc.title();
                self.info(format!("„{name}“ wurde außerhalb geändert und neu geladen."));
            } else if self.dialog.is_none() {
                // Asked once per change: keeping the own version takes over the stamp.
                doc.disk_stamp = Some(now);
                self.dialog = Some(Dialog::Conflict { doc_id: doc.id, close_after: false, saving: false });
            }
        }
    }

    /// Reads a local document again from disk, keeping cursors and undo history.
    pub(super) fn reload_local(&mut self, doc_id: u64) {
        let Some(i) = self.doc_index(doc_id) else { return };
        let DocSource::Local(path) = self.docs[i].source.clone() else { return };
        match std::fs::read(&path) {
            Ok(bytes) => {
                // Keep the own changes that the reload throws away.
                if self.docs[i].is_dirty() {
                    let mine = self.docs[i].to_bytes();
                    self.record_history(&DocSource::Local(path.clone()), &mine, crate::history::Kind::Discarded);
                }
                self.record_history(&DocSource::Local(path.clone()), &bytes, crate::history::Kind::Opened);
                let doc = &mut self.docs[i];
                doc.reload_bytes(&bytes);
                doc.disk_stamp = DiskStamp::of(&path);
                doc.disk_missing = 0;
            }
            Err(e) => self.error(format!("{} konnte nicht gelesen werden: {e}", path.display())),
        }
    }

    /// The local file changed since it was loaded or saved (by another program).
    pub(super) fn changed_on_disk(&self, index: usize) -> bool {
        let doc = &self.docs[index];
        let DocSource::Local(path) = &doc.source else { return false };
        match (doc.disk_stamp, DiskStamp::of(path)) {
            (Some(old), Some(now)) => old != now,
            _ => false,
        }
    }
}
