//! Loads the committed version of open files for the change markers in the gutter.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use eframe::egui;

use super::NebulaApp;
use crate::editor::DocSource;

#[derive(Default)]
pub(super) struct GitBases {
    /// Commit the base of each document (by id) was loaded for.
    loaded: HashMap<u64, Option<String>>,
    pending: Vec<(u64, Receiver<Option<Vec<u8>>>)>,
}

impl NebulaApp {
    pub(super) fn poll_git_bases(&mut self, ctx: &egui::Context) {
        let mut i = 0;
        while i < self.git_bases.pending.len() {
            let Ok(result) = self.git_bases.pending[i].1.try_recv() else {
                i += 1;
                continue;
            };
            let (id, _) = self.git_bases.pending.remove(i);
            if let Some(doc) = self.doc_index(id).map(|i| &mut self.docs[i]) {
                doc.view.git_base = result.map(|bytes| {
                    let text = doc.encoding.decode(&bytes);
                    Arc::new(text.replace("\r\n", "\n"))
                });
                doc.view.git_marks = None;
            }
        }

        let ids: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.git_bases.loaded.retain(|id, _| ids.contains(id));
        let Some(top) = self.sidebar.git.top.clone() else {
            if !self.git_bases.loaded.is_empty() {
                self.git_bases.loaded.clear();
                for doc in &mut self.docs {
                    doc.view.git_base = None;
                }
            }
            return;
        };
        let head = self.sidebar.git.head.clone();
        for doc in &self.docs {
            let DocSource::Local(path) = &doc.source else { continue };
            if self.git_bases.loaded.get(&doc.id) == Some(&head) || self.git_bases.pending.iter().any(|(id, _)| *id == doc.id) {
                continue;
            }
            self.git_bases.loaded.insert(doc.id, head.clone());
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (top, path, ctx) = (top.clone(), path.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(crate::ui::git::head_file(&top, &path));
                ctx.request_repaint();
            });
            self.git_bases.pending.push((doc.id, rx));
        }
    }
}
