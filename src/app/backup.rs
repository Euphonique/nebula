//! Crash recovery ("hot exit"): unsaved contents are copied to a backup
//! folder every few seconds and brought back on the next start if Nebula
//! did not exit normally.
//!
//! Every running instance owns `backup/<id>/`, locked by an open `lock` file.
//! A folder whose lock can be taken belongs to an instance that is gone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::NebulaApp;
use crate::editor::document::{DiskStamp, Selection};
use crate::editor::{DocSource, Document};
use crate::lang::Language;

const BACKUP_EVERY: Duration = Duration::from_secs(3);

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    entries: Vec<Entry>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    /// Content file in the backup folder (UTF-8, LF line endings).
    file: String,
    local: Option<PathBuf>,
    site_id: Option<String>,
    remote_path: Option<String>,
    /// Title of an untitled document.
    title: Option<String>,
    language: String,
    cursor: usize,
}

pub(super) struct Backups {
    dir: PathBuf,
    /// Keeps the folder marked as in use.
    _lock: Option<std::fs::File>,
    /// Document version last written, by document id.
    written: HashMap<u64, u64>,
    last: Instant,
    /// Contents restored from a crashed session, waiting for the server file to open.
    pending_remote: HashMap<(String, String), (String, usize)>,
}

fn root() -> PathBuf {
    crate::settings::config_dir().join("backup")
}

fn lock(dir: &Path) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().create(true).truncate(false).write(true).share_mode(0).open(dir.join("lock")).ok()
}

impl Default for Backups {
    fn default() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let dir = root().join(format!("{}-{stamp}", std::process::id()));
        let lock = std::fs::create_dir_all(&dir).ok().and_then(|_| lock(&dir));
        Self { dir, _lock: lock, written: HashMap::new(), last: Instant::now(), pending_remote: HashMap::new() }
    }
}

impl NebulaApp {
    /// Writes the contents of documents with unsaved changes to the backup folder.
    pub(super) fn backup_unsaved(&mut self, ctx: &eframe::egui::Context) {
        if self.backups.last.elapsed() < BACKUP_EVERY {
            ctx.request_repaint_after(BACKUP_EVERY);
            return;
        }
        self.backups.last = Instant::now();
        let unsaved: Vec<usize> = (0..self.docs.len())
            .filter(|&i| {
                let d = &self.docs[i];
                d.is_dirty() || (matches!(d.source, DocSource::Untitled(_)) && d.len_chars() > 0)
            })
            .collect();
        let current: HashMap<u64, u64> = unsaved.iter().map(|&i| (self.docs[i].id, self.docs[i].version)).collect();
        if current == self.backups.written {
            return;
        }
        let mut entries = Vec::new();
        let mut writes: Vec<(PathBuf, Arc<String>)> = Vec::new();
        for &i in &unsaved {
            let changed = self.backups.written.get(&self.docs[i].id) != Some(&self.docs[i].version);
            let doc = &mut self.docs[i];
            let file = format!("{}.txt", doc.id);
            if changed {
                writes.push((self.backups.dir.join(&file), doc.text()));
            }
            let mut entry = Entry {
                file,
                language: doc.language.id().to_string(),
                cursor: doc.primary_selection().head,
                ..Default::default()
            };
            match &doc.source {
                DocSource::Local(p) => entry.local = Some(p.clone()),
                DocSource::Remote { site_id, path } => {
                    entry.site_id = Some(site_id.clone());
                    entry.remote_path = Some(path.clone());
                }
                DocSource::Untitled(_) => entry.title = Some(doc.title()),
            }
            entries.push(entry);
        }
        let stale: Vec<PathBuf> = self
            .backups
            .written
            .keys()
            .filter(|id| !current.contains_key(id))
            .map(|id| self.backups.dir.join(format!("{id}.txt")))
            .collect();
        self.backups.written = current;
        let manifest = self.backups.dir.join("manifest.json");
        let json = serde_json::to_vec_pretty(&Manifest { entries }).unwrap_or_default();
        std::thread::spawn(move || {
            for (path, text) in writes {
                let _ = std::fs::write(path, text.as_bytes());
            }
            for path in stale {
                let _ = std::fs::remove_file(path);
            }
            let tmp = manifest.with_extension("tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(tmp, manifest);
            }
        });
    }

    /// Normal exit: unsaved changes were dealt with, the backup is not needed.
    pub(super) fn discard_backups(&mut self) {
        self.backups._lock = None;
        let _ = std::fs::remove_dir_all(&self.backups.dir);
    }

    /// Brings back unsaved contents of instances that did not exit normally.
    pub(super) fn restore_backups(&mut self) {
        let Ok(dirs) = std::fs::read_dir(root()) else { return };
        let mut restored = 0;
        for dir in dirs.flatten().map(|e| e.path()) {
            if dir == self.backups.dir || !dir.is_dir() {
                continue;
            }
            let Some(lock) = lock(&dir) else { continue };
            let manifest: Manifest = std::fs::read(dir.join("manifest.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            for entry in manifest.entries {
                let Ok(text) = std::fs::read_to_string(dir.join(&entry.file)) else { continue };
                self.restore_entry(entry, text);
                restored += 1;
            }
            drop(lock);
            let _ = std::fs::remove_dir_all(&dir);
        }
        if restored > 0 {
            self.info(if restored == 1 {
                "Ungespeicherte Änderungen aus der letzten Sitzung wiederhergestellt.".to_string()
            } else {
                format!("{restored} Dateien mit ungespeicherten Änderungen wiederhergestellt.")
            });
        }
    }

    fn restore_entry(&mut self, entry: Entry, text: String) {
        let language = Language::from_id(&entry.language);
        if let Some(path) = &entry.local {
            let open = self.docs.iter().position(|d| d.source == DocSource::Local(path.clone()));
            let index = match open {
                Some(i) => Some(i),
                None if path.is_file() => {
                    self.open_local(path);
                    self.docs.iter().position(|d| d.source == DocSource::Local(path.clone()))
                }
                None => None,
            };
            match index {
                Some(i) => self.put_restored(i, &text, entry.cursor),
                None => {
                    // The file is gone: keep the content under its old path, unsaved.
                    let mut doc = Document::new(DocSource::Local(path.clone()), &text);
                    doc.mark_unsaved();
                    doc.disk_stamp = DiskStamp::of(path);
                    let pos = entry.cursor.min(doc.len_chars());
                    doc.set_selection(Selection::cursor(pos));
                    self.docs.push(doc);
                }
            }
        } else if let (Some(site_id), Some(path)) = (&entry.site_id, &entry.remote_path)
            && self.sites.get(site_id).is_some()
        {
            let source = DocSource::Remote { site_id: site_id.clone(), path: path.clone() };
            match self.docs.iter().position(|d| d.source == source) {
                Some(i) => self.put_restored(i, &text, entry.cursor),
                None => {
                    self.backups.pending_remote.insert((site_id.clone(), path.clone()), (text, entry.cursor));
                    self.open_remote(site_id, path);
                }
            }
        } else {
            self.untitled += 1;
            let mut doc = Document::new(DocSource::Untitled(self.untitled), &text);
            doc.language = language.or_else(|| entry.remote_path.as_deref().map(Language::from_path)).unwrap_or(Language::Html);
            doc.mark_unsaved();
            doc.suggested_name = entry.title.filter(|t| !t.starts_with("Ohne Titel"));
            if let Some(name) = entry.remote_path.as_deref().and_then(|p| p.rsplit('/').next()) {
                doc.suggested_name = Some(name.to_string());
            }
            let pos = entry.cursor.min(doc.len_chars());
            doc.set_selection(Selection::cursor(pos));
            self.docs.push(doc);
        }
        if self.active.is_none() && !self.docs.is_empty() {
            self.activate(self.docs.len() - 1);
        }
    }

    /// Puts restored content into an open document as an undoable change.
    fn put_restored(&mut self, index: usize, text: &str, cursor: usize) {
        let doc = &mut self.docs[index];
        doc.replace_text(text);
        let pos = cursor.min(doc.len_chars());
        doc.set_selection(Selection::cursor(pos));
        doc.view.center_line = Some(doc.char_to_line(pos));
    }

    /// A server file with restored content could not be opened (`path`, or
    /// any on the site): keep the content in an untitled document instead.
    pub(super) fn backup_open_failed(&mut self, site_id: &str, path: Option<&str>) {
        let keys: Vec<(String, String)> = self
            .backups
            .pending_remote
            .keys()
            .filter(|(s, p)| s == site_id && path.is_none_or(|path| path == p))
            .cloned()
            .collect();
        for key in keys {
            let (text, cursor) = self.backups.pending_remote.remove(&key).unwrap();
            let entry = Entry { remote_path: Some(key.1), cursor, ..Default::default() };
            self.restore_entry(entry, text);
        }
    }

    /// A server file opened: apply content restored from a crashed session.
    pub(super) fn apply_pending_backup(&mut self, index: usize) {
        let DocSource::Remote { site_id, path } = self.docs[index].source.clone() else { return };
        if let Some((text, cursor)) = self.backups.pending_remote.remove(&(site_id, path)) {
            self.put_restored(index, &text, cursor);
        }
    }
}
