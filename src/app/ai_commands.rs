//! AI commands on the selection (editor context menu "KI", command palette):
//! explain, comment, rewrite and fix. They become a message to the agent of
//! the panel; changes come back as a diff to approve like any other edit.

use eframe::egui::{self, RichText};

use super::{Dialog, NebulaApp, modal, primary_button};
use crate::editor::DocSource;
use crate::editor::view::AiAction;

/// The question "how should it be rewritten?".
pub(super) struct RewriteForm {
    doc_id: u64,
    how: String,
    focus: bool,
}

impl NebulaApp {
    /// An AI command for a document (from the context menu or the palette).
    pub(super) fn ai_action(&mut self, doc_id: u64, action: AiAction, ctx: &egui::Context) {
        if action == AiAction::Rewrite {
            self.dialog = Some(Dialog::AiRewrite(Box::new(RewriteForm { doc_id, how: String::new(), focus: true })));
            return;
        }
        self.run_ai(doc_id, action, "", ctx);
    }

    /// The same for the active tab.
    pub(super) fn ai_action_active(&mut self, action: AiAction, ctx: &egui::Context) {
        let Some(i) = self.active else { return };
        if action != AiAction::Explain && !matches!(self.docs[i].source, DocSource::Local(_)) {
            self.info(t!("Nur für lokale Dateien – der Agent bearbeitet Dateien auf diesem Rechner."));
            return;
        }
        self.ai_action(self.docs[i].id, action, ctx);
    }

    fn run_ai(&mut self, doc_id: u64, action: AiAction, how: &str, ctx: &egui::Context) {
        let Some(i) = self.doc_index(doc_id) else { return };
        let doc = &self.docs[i];
        // Whole lines of the selection (the cursor's line without one).
        let sel = doc.primary_selection().range();
        let first = doc.char_to_line(sel.start);
        let mut last = doc.char_to_line(sel.end);
        if last > first && doc.line_to_char(last) == sel.end {
            last -= 1; // a selection ending at a line start doesn't include that line
        }
        let code = (first..=last).map(|l| doc.line_text(l).trim_end_matches(['\n', '\r']).to_string()).collect::<Vec<_>>().join("\n");
        let cwd = self.agent_cwd();
        let file = match &doc.source {
            // Relative to the project (where the agent works): shorter, and every agent understands it.
            DocSource::Local(p) => p.strip_prefix(&cwd).unwrap_or(p).display().to_string(),
            DocSource::Remote { site_id, path } => {
                format!("{}: {path}", self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default())
            }
            DocSource::Untitled(_) => doc.title(),
        };
        let (from, to) = (first + 1, last + 1);
        let mut text = match action {
            AiAction::Explain => t!("Erkläre mir diesen Code aus {file} (Zeilen {from}–{to}). Ändere dabei keine Dateien.", file = file, from = from, to = to),
            AiAction::Comment => t!(
                "Füge dem Code in {file}, Zeilen {from}–{to}, passende Kommentare hinzu: Doc-Kommentare für Funktionen und Klassen, kurze Erklärungen an schwierigen Stellen. Ändere nur Kommentare, nicht den Code, und nur in diesem Bereich.",
                file = file,
                from = from,
                to = to
            ),
            AiAction::Rewrite => t!(
                "Schreibe den Code in {file}, Zeilen {from}–{to}, um: {how}. Ändere nur diesen Bereich und erkläre kurz, was du geändert hast.",
                file = file,
                from = from,
                to = to,
                how = how
            ),
            AiAction::Fix => t!(
                "Finde und behebe Fehler im Code in {file}, Zeilen {from}–{to}. Ändere nur, was dafür nötig ist, und erkläre kurz, was falsch war.",
                file = file,
                from = from,
                to = to
            ),
        };
        // Problems the language server reports in that range help with fixing.
        if action == AiAction::Fix {
            let problems: Vec<String> = doc
                .view
                .diagnostics
                .iter()
                .filter(|d| (first..=last).contains(&doc.char_to_line(d.range.start)))
                .map(|d| t!("Zeile {line}: {message}", line = doc.char_to_line(d.range.start) + 1, message = d.message))
                .collect();
            if !problems.is_empty() {
                text.push_str(&format!("\n\n{}\n{}", t!("Gemeldete Probleme:"), problems.join("\n")));
            }
        }
        text.push_str(&format!("\n\n```{}\n{code}\n```", doc.language.id()));
        self.show_agent();
        self.send_agent_prompt(text, Vec::new(), ctx);
    }

    /// Returns whether the dialog stays open.
    pub(super) fn ai_rewrite_dialog(&mut self, ctx: &egui::Context, form: &mut RewriteForm) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        let mut go = false;
        modal(ctx, egui::Id::unique("ai-rewrite")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(t!("Umschreiben"));
            ui.label(RichText::new(t!("Wie soll der Code umgeschrieben werden?")).color(theme.text_muted));
            ui.add_space(6.0);
            let edit = ui.add(egui::TextEdit::singleline(&mut form.how).hint_text(t!("z. B. lesbarer, als async/await, ohne jQuery")).desired_width(f32::INFINITY));
            if std::mem::take(&mut form.focus) {
                edit.request_focus();
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for idea in [t!("lesbarer"), t!("kürzer"), t!("moderner"), t!("mit Fehlerbehandlung"), t!("schneller")] {
                    if ui.small_button(idea).clicked() {
                        form.how = idea.to_string();
                        go = true;
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!form.how.trim().is_empty(), primary_button(t!("Umschreiben"), theme.accent)).clicked() {
                    go = true;
                }
                if ui.button(t!("Abbrechen")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        });
        if go && !form.how.trim().is_empty() {
            let how = form.how.trim().to_string();
            self.run_ai(form.doc_id, AiAction::Rewrite, &how, ctx);
            return false;
        }
        keep
    }
}
