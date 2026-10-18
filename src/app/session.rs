//! Reopening the documents of the previous session.

use super::NebulaApp;
use crate::editor::DocSource;
use crate::editor::document::Selection;
use crate::settings::SessionEntry;

impl NebulaApp {
    pub(super) fn store_session(&mut self) {
        let mut active = None;
        let mut entries = Vec::new();
        for (i, d) in self.docs.iter().enumerate() {
            let cursor = d.primary_selection().head;
            let entry = match &d.source {
                DocSource::Local(p) => SessionEntry { local: Some(p.clone()), cursor, ..Default::default() },
                DocSource::Remote { site_id, path } => SessionEntry {
                    site_id: Some(site_id.clone()),
                    remote_path: Some(path.clone()),
                    cursor,
                    ..Default::default()
                },
                DocSource::Untitled(_) => continue,
            };
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
                    let pos = e.cursor.min(doc.len_chars());
                    doc.set_selection(Selection::cursor(pos));
                    doc.view.center_line = Some(doc.char_to_line(pos));
                    if active == Some(n) {
                        active_id = Some(doc.id);
                    }
                }
            } else if let (Some(site_id), Some(path)) = (&e.site_id, &e.remote_path)
                && self.sites.get(site_id).is_some()
            {
                // Remote files open once the connection delivers them.
                self.pending_cursors.insert((site_id.clone(), path.clone()), e.cursor);
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
        if let Some(cursor) = self.pending_cursors.remove(&(site_id, path)) {
            let doc = &mut self.docs[index];
            let pos = cursor.min(doc.len_chars());
            doc.set_selection(Selection::cursor(pos));
            doc.view.center_line = Some(doc.char_to_line(pos));
        }
    }
}
