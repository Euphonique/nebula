//! Reopening the documents of the previous session.

use super::NebulaApp;
use crate::editor::DocSource;
use crate::editor::document::Selection;
use std::path::PathBuf;

use crate::settings::{ProjectTabs, SessionEntry};

impl NebulaApp {
    pub(super) fn store_session(&mut self) {
        let mut active = None;
        let mut entries = Vec::new();
        for (i, d) in self.docs.iter().enumerate() {
            let Some(entry) = session_entry(d) else { continue };
            if self.active == Some(i) {
                active = Some(entries.len());
            }
            entries.push(entry);
        }
        self.settings.session = entries;
        self.settings.session_active = active;
    }

    pub(super) fn restore_session(&mut self) {
        let entries = std::mem::take(&mut self.settings.session);
        let active = self.settings.session_active;
        let mut active_id = None;
        for (n, e) in entries.iter().enumerate() {
            if let Some(path) = &e.local {
                if !path.is_file() {
                    continue;
                }
                self.open_local(path);
                if let Some(i) = self.active {
                    let doc = &mut self.docs[i];
                    apply_entry(doc, e);
                    if active == Some(n) {
                        active_id = Some(doc.id);
                    }
                }
            } else if let (Some(site_id), Some(path)) = (&e.site_id, &e.remote_path)
                && self.sites.get(site_id).is_some()
            {
                // Remote files open once the connection delivers them.
                self.pending_cursors.insert((site_id.clone(), path.clone()), e.cursor);
                if e.pinned {
                    self.pending_pins.insert((site_id.clone(), path.clone()));
                }
                if !e.bookmarks.is_empty() {
                    self.pending_bookmarks.insert((site_id.clone(), path.clone()), e.bookmarks.clone());
                }
                self.open_remote(site_id, path);
            }
        }
        if let Some(i) = active_id.and_then(|id| self.doc_index(id)) {
            self.activate(i);
        }
        self.settings.session = entries;
    }

    /// Applies a cursor remembered from the last session to a freshly opened remote file.
    pub(super) fn apply_pending_cursor(&mut self, index: usize) {
        let DocSource::Remote { site_id, path } = self.docs[index].source.clone() else { return };
        if self.pending_pins.remove(&(site_id.clone(), path.clone())) {
            self.docs[index].pinned = true;
        }
        if let Some(marks) = self.pending_bookmarks.remove(&(site_id.clone(), path.clone())) {
            let doc = &mut self.docs[index];
            doc.view.bookmarks = marks.into_iter().filter(|&l| l < doc.len_lines()).collect();
        }
        if let Some((line, column)) = self.pending_lines.remove(&(site_id.clone(), path.clone())) {
            self.goto_line_column(index, line, column);
        } else if let Some(cursor) = self.pending_cursors.remove(&(site_id, path)) {
            let doc = &mut self.docs[index];
            let pos = cursor.min(doc.len_chars());
            doc.set_selection(Selection::cursor(pos));
            doc.view.center_line = Some(doc.char_to_line(pos));
        }
    }

    /// Cursor to `line`/`column` (clamped), centered, editor focused.
    pub(super) fn goto_line_column(&mut self, index: usize, line: usize, column: usize) {
        let doc = &mut self.docs[index];
        let line = line.min(doc.len_lines() - 1);
        let pos = doc.line_to_char(line) + column.min(doc.line_len(line));
        doc.set_selection(Selection::cursor(pos));
        doc.view.center_line = Some(line);
        self.focus_editor = true;
    }

    /// Opens a project folder. Switching projects closes the unchanged files
    /// of the previous one and reopens the files last open in the new one.
    pub(super) fn open_folder(&mut self, path: PathBuf) {
        let path = crate::ui::sidebar::normalize_path(&path);
        if self.sidebar.root.as_ref() == Some(&path) {
            return;
        }
        if let Some(old) = self.sidebar.root.clone() {
            let in_old = |d: &crate::editor::Document| matches!(&d.source, DocSource::Local(p) if p.starts_with(&old));
            let mut tabs = ProjectTabs::default();
            for (i, d) in self.docs.iter().enumerate().filter(|(_, d)| in_old(d)) {
                if self.active == Some(i) {
                    tabs.active = Some(tabs.entries.len());
                }
                if let Some(entry) = session_entry(d) {
                    tabs.entries.push(entry);
                }
            }
            self.settings.project_tabs.insert(old.to_string_lossy().into_owned(), tabs);
            let active_id = self.active.map(|i| self.docs[i].id);
            self.docs.retain(|d| !in_old(d) || d.is_dirty());
            self.active = active_id.and_then(|id| self.doc_index(id)).or((!self.docs.is_empty()).then_some(0));
        }
        self.settings.last_folder = Some(path.clone());
        self.remember_folder(&path);
        self.sidebar.set_root(path.clone());
        if let Some(tabs) = self.settings.project_tabs.get(&*path.to_string_lossy()).cloned() {
            let mut active_id = None;
            for (n, e) in tabs.entries.iter().enumerate() {
                let Some(p) = e.local.as_ref().filter(|p| p.is_file()) else { continue };
                self.open_local(p);
                if let Some(i) = self.active {
                    let doc = &mut self.docs[i];
                    apply_entry(doc, e);
                    if tabs.active == Some(n) {
                        active_id = Some(doc.id);
                    }
                }
            }
            if let Some(i) = active_id.and_then(|id| self.doc_index(id)) {
                self.activate(i);
            }
        }
        let _ = self.settings.save();
    }

    /// Puts a folder at the top of the recent projects.
    pub(super) fn remember_folder(&mut self, path: &std::path::Path) {
        let folders = &mut self.settings.recent_folders;
        folders.retain(|p| p != path);
        folders.insert(0, path.to_path_buf());
        folders.truncate(12);
        let keep: Vec<String> = folders.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        self.settings.project_tabs.retain(|k, _| keep.contains(k));
        self.sidebar.recent_folders = self.settings.recent_folders.clone();
    }
}

/// What the session remembers of a tab (untitled ones are not kept).
fn session_entry(d: &crate::editor::Document) -> Option<SessionEntry> {
    let mut entry = SessionEntry {
        cursor: d.primary_selection().head,
        pinned: d.pinned,
        bookmarks: d.view.bookmarks.iter().copied().collect(),
        ..Default::default()
    };
    match &d.source {
        DocSource::Local(p) => entry.local = Some(p.clone()),
        DocSource::Remote { site_id, path } => {
            entry.site_id = Some(site_id.clone());
            entry.remote_path = Some(path.clone());
        }
        DocSource::Untitled(_) => return None,
    }
    Some(entry)
}

/// Cursor, pin and bookmarks of a reopened local tab.
fn apply_entry(doc: &mut crate::editor::Document, e: &SessionEntry) {
    let pos = e.cursor.min(doc.len_chars());
    doc.set_selection(Selection::cursor(pos));
    doc.view.center_line = Some(doc.char_to_line(pos));
    doc.pinned = e.pinned;
    doc.view.bookmarks = e.bookmarks.iter().copied().filter(|&l| l < doc.len_lines()).collect();
}
