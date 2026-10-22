//! Tab actions (context menu of a tab) and printing.

use eframe::egui::{self, Color32};

use super::{Dialog, NebulaApp};
use crate::editor::{DocSource, Document};

/// Something chosen in a tab's context menu.
#[derive(Clone, Copy)]
pub(super) enum TabCmd {
    Save,
    SaveAs,
    Duplicate,
    Split,
    Pin,
    Print,
    Close,
    CloseOthers,
    CloseRight,
    CloseAll,
    CopyPath,
    Reveal,
    History,
    CompareHead,
    CompareFile,
}

/// Context menu of a tab.
/// `keys`: shortcuts of Save, Save As, Print and Close.
pub(super) fn tab_menu(ui: &mut egui::Ui, doc: &Document, i: usize, count: usize, keys: &[String; 4], out: &mut Option<(usize, TabCmd)>) {
    let mut item = |ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, cmd: TabCmd| {
        if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked() {
            *out = Some((i, cmd));
            ui.close();
        }
    };
    item(ui, "Speichern", &keys[0], true, TabCmd::Save);
    item(ui, "Speichern unter…", &keys[1], true, TabCmd::SaveAs);
    item(ui, "Duplizieren", "", true, TabCmd::Duplicate);
    item(ui, "In geteilter Ansicht öffnen", "", true, TabCmd::Split);
    item(ui, if doc.pinned { "Lösen" } else { "Anheften" }, "", true, TabCmd::Pin);
    item(ui, "Drucken…", &keys[2], true, TabCmd::Print);
    ui.separator();
    item(ui, "Schließen", &keys[3], true, TabCmd::Close);
    item(ui, "Andere schließen", "", count > 1, TabCmd::CloseOthers);
    item(ui, "Rechts davon schließen", "", i + 1 < count, TabCmd::CloseRight);
    item(ui, "Alle schließen", "", true, TabCmd::CloseAll);
    if !matches!(doc.source, DocSource::Untitled(_)) {
        ui.separator();
        item(ui, "Lokaler Verlauf…", "", true, TabCmd::History);
        item(ui, "Mit letztem Commit vergleichen", "", matches!(doc.source, DocSource::Local(_)), TabCmd::CompareHead);
        item(ui, "Vergleichen mit Datei…", "", true, TabCmd::CompareFile);
        item(ui, "Pfad kopieren", "", true, TabCmd::CopyPath);
        if matches!(doc.source, DocSource::Local(_)) {
            item(ui, "Im Explorer zeigen", "", true, TabCmd::Reveal);
        }
    }
}

impl NebulaApp {
    pub(super) fn run_tab_cmd(&mut self, ctx: &egui::Context, i: usize, cmd: TabCmd) {
        if i >= self.docs.len() {
            return;
        }
        match cmd {
            TabCmd::Save => {
                self.save_doc(i, false);
            }
            TabCmd::SaveAs => {
                self.save_doc_as(i);
            }
            TabCmd::Duplicate => self.duplicate_doc(i),
            TabCmd::Split => self.open_in_split(i),
            TabCmd::Pin => self.toggle_pin(i),
            TabCmd::Print => {
                self.activate(i);
                self.print_active(ctx);
            }
            TabCmd::Close => self.close_doc(i, false),
            TabCmd::CloseOthers => self.close_others(i),
            TabCmd::CloseRight => self.close_many((i + 1..self.docs.len()).filter(|&n| !self.docs[n].pinned).collect()),
            TabCmd::CloseAll => self.close_many((0..self.docs.len()).filter(|&n| !self.docs[n].pinned).collect()),
            TabCmd::CopyPath => ctx.copy_text(self.docs[i].source.path_string()),
            TabCmd::Reveal => {
                if let DocSource::Local(p) = &self.docs[i].source {
                    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
                }
            }
            TabCmd::History => self.open_history(i),
            TabCmd::CompareHead => self.compare_with_head(i),
            TabCmd::CompareFile => self.compare_with_file_dialog(i),
        }
    }

    /// Opens an unsaved copy of a document next to it.
    pub(super) fn duplicate_doc(&mut self, i: usize) {
        let Some(src) = self.docs.get(i) else { return };
        self.untitled += 1;
        let mut doc = Document::new(DocSource::Untitled(self.untitled), &src.rope.to_string());
        doc.language = src.language;
        doc.line_ending = src.line_ending;
        doc.encoding = src.encoding;
        let name = src.title();
        let (stem, ext) = match name.rfind('.').filter(|&p| p > 0) {
            Some(p) => name.split_at(p),
            None => (name.as_str(), ""),
        };
        doc.suggested_name = Some(format!("{stem} Kopie{ext}"));
        doc.set_selection(src.primary_selection());
        // Untitled with content: closing asks to save.
        doc.mark_unsaved();
        self.docs.insert(i + 1, doc);
        self.activate(i + 1);
    }

    pub(super) fn toggle_pin(&mut self, i: usize) {
        let Some(doc) = self.docs.get_mut(i) else { return };
        doc.pinned = !doc.pinned;
        self.sort_pinned();
        self.reveal_active_tab = true;
    }

    /// Keeps pinned tabs at the left (in their order), the others after them.
    pub(super) fn sort_pinned(&mut self) {
        if self.docs.windows(2).all(|w| w[0].pinned || !w[1].pinned) {
            return;
        }
        let active_id = self.active.and_then(|a| self.docs.get(a)).map(|d| d.id);
        self.docs.sort_by_key(|d| !d.pinned);
        self.active = active_id.and_then(|id| self.doc_index(id));
    }

    pub(super) fn close_others(&mut self, keep: usize) {
        self.close_many((0..self.docs.len()).filter(|&i| i != keep && !self.docs[i].pinned).collect());
        if let Some(i) = self.active {
            self.activate(i);
        }
    }

    /// Closes several tabs; the ones with unsaved changes ask first.
    pub(super) fn close_many(&mut self, indices: Vec<usize>) {
        let active_id = self.active.and_then(|a| self.docs.get(a)).map(|d| d.id);
        let ids: Vec<u64> = indices.iter().filter_map(|&i| self.docs.get(i)).map(|d| d.id).collect();
        let mut dirty = Vec::new();
        for id in ids {
            let Some(i) = self.doc_index(id) else { continue };
            if self.docs[i].is_dirty() {
                dirty.push(id);
            } else {
                self.docs.remove(i);
            }
        }
        let old = self.active.unwrap_or(0);
        self.active = active_id
            .and_then(|id| self.doc_index(id))
            .or_else(|| (!self.docs.is_empty()).then(|| old.min(self.docs.len() - 1)));
        self.reveal_active_tab = true;
        self.focus_editor = true;
        if !dirty.is_empty() {
            self.dialog = Some(Dialog::UnsavedChanges { doc_ids: dirty, quit: false });
        }
    }

    /// Prints the active document with syntax colors.
    pub(super) fn print_active(&mut self, ctx: &egui::Context) {
        let Some(i) = self.active else { return };
        if self.printing.is_some() {
            self.info("Es wird bereits gedruckt …");
            return;
        }
        let tab = self.settings.editor_for_doc(&self.docs[i]).tab_size;
        let doc = &self.docs[i];
        // Colors of the light theme read well on paper.
        let paper = crate::theme::Theme::light();
        let colored = doc.highlight_version == doc.version;
        let mut lines: Vec<Vec<(String, Color32)>> = (0..doc.len_lines())
            .map(|l| {
                let text = doc.line_text(l);
                let spans = if colored { doc.highlights.get(l).map(|v| v.as_slice()) } else { None };
                color_runs(&text, spans.unwrap_or(&[]), &paper)
            })
            .collect();
        // The empty line after a final line break is not printed.
        if lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let title = match &doc.source {
            DocSource::Untitled(_) => doc.title(),
            s => s.path_string(),
        };
        let job = crate::print::PrintJob { title, lines, tab };
        self.printing = crate::print::print(job, ctx);
    }

    /// Reports a finished print job.
    pub(super) fn poll_printing(&mut self) {
        if !self.printing.as_ref().is_some_and(|h| h.is_finished()) {
            return;
        }
        match self.printing.take().unwrap().join() {
            // Cancelled in the dialog.
            Ok(Ok(0)) => {}
            Ok(Ok(pages)) => self.info(format!("{pages} Seite(n) an den Drucker gesendet")),
            Ok(Err(e)) => self.error(e),
            Err(_) => self.error("Drucken fehlgeschlagen."),
        }
    }
}

/// Splits a line into runs by highlight span.
fn color_runs(text: &str, spans: &[crate::highlight::Span], theme: &crate::theme::Theme) -> Vec<(String, Color32)> {
    let mut runs = Vec::new();
    let mut pos = 0;
    let push = |a: usize, b: usize, color: Color32, runs: &mut Vec<(String, Color32)>| {
        if let Some(s) = text.get(a..b).filter(|s| !s.is_empty()) {
            runs.push((s.to_string(), color));
        }
    };
    for s in spans {
        let (a, b) = ((s.start as usize).min(text.len()), (s.end as usize).min(text.len()));
        if a > pos {
            push(pos, a, theme.foreground, &mut runs);
        }
        let a = a.max(pos);
        if b > a {
            push(a, b, theme.syntax_color(s.style), &mut runs);
            pos = b;
        }
    }
    push(pos, text.len(), theme.foreground, &mut runs);
    runs
}
