//! "Show differences" in the conflict dialog: the own version compared with
//! the file on disk or on the server.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, RichText};

use super::NebulaApp;
use crate::diff::{DiffLine, unified};
use crate::editor::DocSource;
use crate::remote::Request;

pub(super) enum Compare {
    /// Waiting for the server file (request tag).
    Loading(u64),
    Ready(Arc<Vec<DiffLine>>),
    Failed(String),
}

impl NebulaApp {
    /// Starts comparing the document with its file on disk or server.
    pub(super) fn start_compare(&mut self, doc_id: u64) {
        let Some(i) = self.doc_index(doc_id) else { return };
        match self.docs[i].source.clone() {
            DocSource::Local(path) => {
                let state = match std::fs::read(&path) {
                    Ok(bytes) => self.compare_with(i, &bytes),
                    Err(e) => Compare::Failed(format!("Die Datei konnte nicht gelesen werden: {e}")),
                };
                self.compare = Some((doc_id, state));
            }
            DocSource::Remote { site_id, path } => {
                let Some(site) = self.sites.get(&site_id).cloned() else { return };
                self.ensure_tree(&site);
                let tag = self.remote.send(&site, Request::Read(path));
                self.compare = Some((doc_id, Compare::Loading(tag)));
            }
            DocSource::Untitled(_) => {}
        }
    }

    fn compare_with(&mut self, index: usize, theirs: &[u8]) -> Compare {
        let doc = &mut self.docs[index];
        let theirs = doc.encoding.decode(theirs).replace("\r\n", "\n");
        let mine = doc.text();
        Compare::Ready(Arc::new(unified(&theirs, &mine, 3)))
    }

    /// A server read finished; returns true if it was for the comparison.
    pub(super) fn compare_read_done(&mut self, tag: u64, data: Result<&[u8], String>) -> bool {
        let Some((doc_id, Compare::Loading(t))) = &self.compare else { return false };
        if *t != tag {
            return false;
        }
        let doc_id = *doc_id;
        let state = match (data, self.doc_index(doc_id)) {
            (Ok(bytes), Some(i)) => self.compare_with(i, bytes),
            (Err(e), _) => Compare::Failed(e),
            _ => return true,
        };
        self.compare = Some((doc_id, state));
        true
    }

    /// The differences inside the conflict dialog (`place`: where the other version is).
    pub(super) fn compare_view(&self, ui: &mut egui::Ui, doc_id: u64, place: &str) {
        let Some((id, state)) = &self.compare else { return };
        if *id != doc_id {
            return;
        }
        ui.add_space(10.0);
        let rows = match state {
            Compare::Loading(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Server-Version wird geladen…");
                });
                return;
            }
            Compare::Failed(e) => {
                ui.colored_label(self.theme.danger, e);
                return;
            }
            Compare::Ready(rows) => rows.clone(),
        };
        if rows.is_empty() {
            ui.label("Keine Unterschiede im Inhalt.");
            return;
        }
        let red = Color32::from_rgba_unmultiplied(230, 80, 80, 40);
        let green = Color32::from_rgba_unmultiplied(80, 190, 110, 40);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("− Version {place}")).color(self.theme.danger).small());
            ui.add_space(12.0);
            ui.label(RichText::new("+ deine Version").color(self.theme.success).small());
        });
        let font = FontId::monospace(12.0);
        egui::Frame::new().fill(self.theme.background).corner_radius(6.0).inner_margin(6.0).show(ui, |ui| {
            let row_h = ui.fonts_mut(|f| f.row_height(&font));
            let height = (rows.len() as f32 * row_h).min(380.0);
            egui::ScrollArea::both().max_height(380.0).min_scrolled_height(height).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows.iter() {
                    let (num, sign, text, bg, color) = match row {
                        DiffLine::Context(_, n, t) => (n + 1, " ", t.as_str(), Color32::TRANSPARENT, self.theme.text_muted),
                        DiffLine::Removed(n, t) => (n + 1, "−", t.as_str(), red, self.theme.danger),
                        DiffLine::Added(n, t) => (n + 1, "+", t.as_str(), green, self.theme.success),
                        DiffLine::Skip(n) => {
                            ui.label(RichText::new(format!("    ⋯ {n} unveränderte Zeilen")).font(font.clone()).color(self.theme.text_faint));
                            continue;
                        }
                    };
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&format!("{num:>5} {sign} "), 0.0, egui::TextFormat::simple(font.clone(), color));
                    job.append(&text.replace('\t', "    "), 0.0, egui::TextFormat::simple(font.clone(), ui.visuals().text_color()));
                    egui::Frame::new().fill(bg).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(egui::Label::new(job).extend());
                    });
                }
            });
        });
    }
}
