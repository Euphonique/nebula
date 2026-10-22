//! Publishing a local project folder to a server, and browser preview.

use std::path::{Path, PathBuf};

use eframe::egui;

use super::{Dialog, NebulaApp};
use crate::editor::DocSource;
use crate::remote::{self, Request};
use crate::settings::PublishTarget;

const SKIP_DIRS: &[&str] = &[".git", "node_modules", ".svn", ".hg", ".idea", ".vs", ".DS_Store", "target"];

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn root_key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

impl NebulaApp {
    pub(super) fn publish_target(&self) -> Option<(PathBuf, PublishTarget)> {
        let root = self.sidebar.root.clone()?;
        let target = self.settings.publish.get(&root_key(&root))?.clone();
        Some((root, target))
    }

    /// Local files changed since they were last published: (absolute, relative, mtime).
    pub(super) fn changed_files(&self) -> Vec<(PathBuf, String, u64)> {
        let Some((root, target)) = self.publish_target() else { return Vec::new() };
        // The project's exclusion list (as for synchronizing), e.g. `.env`.
        let exclude = target.exclude_list();
        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name();
                if exclude.iter().any(|x| name == x.as_str()) {
                    continue;
                }
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if !SKIP_DIRS.iter().any(|s| name == *s) {
                        stack.push(e.path());
                    }
                    continue;
                }
                let path = e.path();
                let rel = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                let m = mtime(&path);
                if target.uploaded.get(&rel).is_none_or(|old| *old != m) {
                    out.push((path, rel, m));
                }
            }
        }
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    pub(super) fn open_publish_setup(&mut self) {
        let Some(root) = self.sidebar.root.clone() else {
            self.info(t!("Öffne zuerst einen lokalen Projektordner."));
            return;
        };
        let target = self.settings.publish.get(&root_key(&root)).cloned().unwrap_or_else(|| PublishTarget {
            site_id: self.sites.sites.first().map(|s| s.id.clone()).unwrap_or_default(),
            remote_path: self.sites.sites.first().map(|s| s.root_path.clone()).unwrap_or_default(),
            ..Default::default()
        });
        self.dialog = Some(Dialog::PublishSetup { root, target });
    }

    /// Uploads every changed file of the project.
    pub(super) fn publish(&mut self) {
        let Some((root, target)) = self.publish_target() else {
            self.open_publish_setup();
            return;
        };
        let files = self.changed_files();
        if files.is_empty() {
            self.info(t!("Alles ist veröffentlicht – keine Änderungen."));
            return;
        }
        let n = files.len();
        for (path, rel, m) in files {
            self.publish_file(&root, &target, &path, &rel, m);
        }
        self.info(t!("Veröffentliche {n} Datei(en)…", n = n));
    }

    fn publish_file(&mut self, root: &Path, target: &PublishTarget, path: &Path, rel: &str, m: u64) {
        let remote = remote::join(&remote::normalize(&target.remote_path), rel);
        if let Some(tag) = self.send_remote(&target.site_id, Request::Upload { local: path.to_path_buf(), remote }) {
            self.pending_publish.insert(tag, (root_key(root), rel.to_string(), m));
        }
    }

    /// After a local save: upload the file if the project publishes on save.
    pub(super) fn upload_on_save(&mut self, path: &Path) {
        let Some((root, target)) = self.publish_target() else { return };
        if !target.upload_on_save {
            return;
        }
        let Ok(rel) = path.strip_prefix(&root) else { return };
        let rel = rel.to_string_lossy().replace('\\', "/");
        // Excluded files (or files in excluded folders) stay local.
        let exclude = target.exclude_list();
        if rel.split('/').any(|part| exclude.iter().any(|x| x == part)) {
            return;
        }
        self.publish_file(&root, &target, path, &rel, mtime(path));
    }

    /// Records a successful publish upload.
    pub(super) fn mark_published(&mut self, tag: u64) {
        if let Some((root, rel, m)) = self.pending_publish.remove(&tag)
            && let Some(t) = self.settings.publish.get_mut(&root)
        {
            t.uploaded.insert(rel, m);
        }
    }

    /// Opens the active document in the browser: via the site's web URL if
    /// known, otherwise the local file. Documents without either (untitled,
    /// or on a server without web URL) open as a temporary copy.
    pub(super) fn open_in_browser(&mut self, _ctx: &egui::Context) {
        let Some(i) = self.active else { return };
        let doc = &self.docs[i];
        let dirty = doc.is_dirty();
        let url = match &doc.source {
            DocSource::Remote { .. } => match self.live_url(&doc.source) {
                Some(url) => url,
                None => {
                    let Some(url) = self.temp_copy(i) else { return };
                    self.info(t!("Für diesen Server ist keine Web-URL hinterlegt (Server bearbeiten → Web-URL). Geöffnet wird eine lokale Kopie, Bilder und CSS fehlen daher eventuell."));
                    url
                }
            },
            // Published projects open on the live site.
            DocSource::Local(p) => self.live_url(&doc.source).unwrap_or_else(|| file_url(p)),
            DocSource::Untitled(_) => {
                let Some(url) = self.temp_copy(i) else { return };
                url
            }
        };
        if !crate::win32::open_url(&url) {
            self.error(t!("{url} konnte nicht geöffnet werden.", url = url));
        } else if dirty && matches!(self.docs[i].source, DocSource::Local(_) | DocSource::Remote { .. }) {
            self.info(t!("Der Browser zeigt die gespeicherte Fassung – ungesicherte Änderungen fehlen noch."));
        }
    }

    /// Writes the document into the temp folder; returns its file URL.
    fn temp_copy(&mut self, i: usize) -> Option<String> {
        let title = self.docs[i].title();
        let dir = std::env::temp_dir().join("nebula-browser");
        let mut path = dir.join(&title);
        let bytes = if crate::preview::is_markdown(&title) {
            path.set_extension("html");
            let text = self.docs[i].text();
            crate::preview::markdown_page(&title, &text, false).into_bytes()
        } else {
            if path.extension().is_none() {
                path.set_extension("html");
            }
            self.docs[i].to_bytes()
        };
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, bytes)) {
            Ok(()) => Some(file_url(&path)),
            Err(e) => {
                self.error(t!("Temporäre Kopie konnte nicht angelegt werden: {e}", e = e));
                None
            }
        }
    }
}

/// `file:///` URL of a local path.
fn file_url(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    format!("file:///{}", crate::preview::percent_encode(p.trim_start_matches('/')))
}
