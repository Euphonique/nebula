//! Snippets tab of the right panel: own and built-in snippets of the current
//! language; click inserts, own ones are created and edited right here.

use eframe::egui::{self, Align2, FontId, Rect, RichText, Sense, pos2, vec2};

use super::{NebulaApp, primary_button};
use crate::editor::complete;
use crate::editor::snippet;
use crate::lang::Language;
use crate::settings::{ALL_LANGUAGES, UserSnippet};
use crate::snippet_io;
use crate::ui::icons;

#[derive(Default)]
pub(super) struct SnippetsState {
    query: String,
    edit: Option<Edit>,
}

/// A snippet being created or changed.
struct Edit {
    /// Where the snippet came from (language key, index); `None` for a new one.
    original: Option<(String, usize)>,
    /// Language id or `*`.
    key: String,
    snippet: UserSnippet,
    focus: bool,
}

enum Action {
    Insert(String),
    New,
    Edit(String, usize),
    /// Built-in snippet copied into an own one.
    Customize(UserSnippet),
    Delete(String, usize),
    Import,
    /// Fold or unfold a snippet group.
    ToggleGroup(String),
    /// Move a snippet (language key, index) into a group (empty: none).
    SetGroup(String, usize, String),
    Export,
}

/// Snippets found in import files, shown for selection.
pub(super) struct SnippetImport {
    source: String,
    entries: Vec<(snippet_io::Imported, bool, bool)>,
}

const HELP: &str = tk!("$1, ${2:Name} – Platzhalter, Tab springt weiter · $0 – Cursor am Ende · \
                    $SELECTED_TEXT – Auswahl · $FILENAME, $DATE");

impl NebulaApp {
    pub(super) fn snippets_pane(&mut self, ui: &mut egui::Ui, body: Rect) {
        let theme = self.theme.clone();
        let Some(i) = self.active else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink2(vec2(8.0, 26.0))), |ui| {
                ui.vertical_centered(|ui| ui.label(RichText::new(t!("Keine Datei geöffnet")).color(theme.text_faint)));
            });
            return;
        };
        let language = self.docs[i].language;
        if self.right_state.snippets.edit.is_some() {
            self.snippet_editor(ui, body, language);
            return;
        }

        let own: Vec<(String, usize, UserSnippet)> = [language.id(), ALL_LANGUAGES]
            .iter()
            .flat_map(|k| {
                self.settings.snippets.get(*k).into_iter().flatten().enumerate().map(move |(n, s)| (k.to_string(), n, s.clone()))
            })
            .collect();
        let builtin = complete::builtin_snippets(language);
        let own_all_empty = self.settings.snippets.values().all(|l| l.is_empty());
        let collapsed = self.settings.collapsed_snippet_groups.clone();
        let state = &mut self.right_state.snippets;
        let mut action = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink2(vec2(10.0, 4.0))), |ui| {
            ui.horizontal(|ui| {
                let add = icons::icon_button(ui, &theme, vec2(26.0, 26.0), false, |p, c, col| icons::plus(p, c, col))
                    .on_hover_text(t!("Neues Snippet (mit der Auswahl als Inhalt)"));
                if add.clicked() {
                    action = Some(Action::New);
                }
                let more = icons::icon_button(ui, &theme, vec2(26.0, 26.0), false, |p, c, col| icons::more(p, c, col))
                    .on_hover_text(t!("Importieren und exportieren"));
                egui::Popup::menu(&more).show(|ui| {
                    if ui.button(t!("Importieren…")).on_hover_text(t!("Nebula-Export oder VS-Code-Snippets (.json, .code-snippets)")).clicked() {
                        action = Some(Action::Import);
                        ui.close();
                    }
                    if ui.add_enabled(!own_all_empty, egui::Button::new(t!("Alle eigenen exportieren…"))).clicked() {
                        action = Some(Action::Export);
                        ui.close();
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut state.query)
                        .hint_text(t!("Snippets für {name} filtern…", name = language.name()))
                        .desired_width(f32::INFINITY),
                );
            });
            ui.add_space(4.0);
            let q = state.query.to_lowercase();
            let matches = |trigger: &str, desc: &str, body: &str| {
                q.is_empty() || trigger.to_lowercase().contains(&q) || desc.to_lowercase().contains(&q) || body.to_lowercase().contains(&q)
            };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let own_shown: Vec<_> = own.iter().filter(|(_, _, s)| matches(&s.trigger, &s.description, &s.body)).collect();
                // "Eigene" also takes snippets dragged out of their group.
                let own_heading = heading(ui, &theme, t!("Eigene"));
                snippet_drop_target(ui, &theme, &own_heading, "", &mut action);
                if own_shown.is_empty() {
                    ui.label(
                        RichText::new(if own.is_empty() { t!("Noch keine – mit + anlegen oder ein eingebautes anpassen.") } else { t!("Keine Treffer") })
                            .small()
                            .color(theme.text_faint),
                    );
                }
                let mut groups: Vec<&str> = own_shown.iter().map(|(_, _, s)| s.group.as_str()).filter(|g| !g.is_empty()).collect();
                groups.sort_by_key(|g| g.to_lowercase());
                groups.dedup();
                // Ungrouped first, then each group under a foldable heading.
                for group in std::iter::once("").chain(groups.iter().copied()) {
                    let members: Vec<_> = own_shown.iter().filter(|(_, _, s)| s.group == group).collect();
                    if !group.is_empty() {
                        let open = !collapsed.iter().any(|c| c == group);
                        let resp = group_heading(ui, &theme, group, open, members.len());
                        snippet_drop_target(ui, &theme, &resp, group, &mut action);
                        if resp.clicked() {
                            action = Some(Action::ToggleGroup(group.to_string()));
                        }
                        if !open {
                            continue;
                        }
                    }
                    for (key, n, s) in members {
                        let tag = (key == ALL_LANGUAGES).then_some(t!("alle Sprachen"));
                        let resp = row(ui, &theme, &s.trigger, &s.description, &s.body, tag);
                        resp.dnd_set_drag_payload(SnippetDrag(key.clone(), *n));
                        snippet_drop_target(ui, &theme, &resp, &s.group, &mut action);
                        if resp.clicked() {
                            action = Some(Action::Insert(s.body.clone()));
                        }
                        resp.context_menu(|ui| {
                            if ui.button(t!("Einfügen")).clicked() {
                                action = Some(Action::Insert(s.body.clone()));
                                ui.close();
                            }
                            if ui.button(t!("Bearbeiten…")).clicked() {
                                action = Some(Action::Edit(key.clone(), *n));
                                ui.close();
                            }
                            if ui.button(t!("Löschen")).clicked() {
                                action = Some(Action::Delete(key.clone(), *n));
                                ui.close();
                            }
                        });
                    }
                }
                if !builtin.is_empty() {
                    ui.add_space(8.0);
                    heading(ui, &theme, t!("Eingebaut"));
                    for (trigger, body, desc) in builtin.iter().filter(|(t, b, d)| matches(t, crate::i18n::tr(d), b)) {
                        let resp = row(ui, &theme, trigger, crate::i18n::tr(desc), body, None);
                        if resp.clicked() {
                            action = Some(Action::Insert(body.to_string()));
                        }
                        resp.context_menu(|ui| {
                            if ui.button(t!("Einfügen")).clicked() {
                                action = Some(Action::Insert(body.to_string()));
                                ui.close();
                            }
                            if ui.button(t!("Als eigenes Snippet anpassen…")).clicked() {
                                action = Some(Action::Customize(UserSnippet {
                                    group: String::new(),
                                    trigger: trigger.to_string(),
                                    description: desc.to_string(),
                                    body: body.to_string(),
                                }));
                                ui.close();
                            }
                        });
                    }
                }
            });
        });

        match action {
            Some(Action::Insert(body)) => self.insert_snippet_body(&body),
            Some(Action::New) => {
                let doc = &self.docs[i];
                let selected = doc.slice(doc.primary_selection().range());
                self.right_state.snippets.edit = Some(Edit {
                    original: None,
                    key: language.id().to_string(),
                    snippet: UserSnippet { body: selected, ..Default::default() },
                    focus: true,
                });
            }
            Some(Action::Edit(key, n)) => {
                if let Some(s) = self.settings.snippets.get(&key).and_then(|l| l.get(n)).cloned() {
                    self.right_state.snippets.edit = Some(Edit { original: Some((key.clone(), n)), key, snippet: s, focus: true });
                }
            }
            Some(Action::Customize(s)) => {
                self.right_state.snippets.edit = Some(Edit { original: None, key: language.id().to_string(), snippet: s, focus: true });
            }
            Some(Action::Delete(key, n)) => {
                if let Some(list) = self.settings.snippets.get_mut(&key)
                    && n < list.len()
                {
                    list.remove(n);
                    if list.is_empty() {
                        self.settings.snippets.remove(&key);
                    }
                }
                self.save_snippets();
            }
            Some(Action::Import) => self.import_snippets(),
            Some(Action::ToggleGroup(group)) => {
                let c = &mut self.settings.collapsed_snippet_groups;
                match c.iter().position(|g| *g == group) {
                    Some(i) => {
                        c.remove(i);
                    }
                    None => c.push(group),
                }
                let _ = self.settings.save();
            }
            Some(Action::SetGroup(key, n, group)) => {
                if let Some(s) = self.settings.snippets.get_mut(&key).and_then(|l| l.get_mut(n))
                    && s.group != group
                {
                    s.group = group;
                    self.save_snippets();
                }
            }
            Some(Action::Export) => self.export_snippets(),
            None => {}
        }
    }

    pub(super) fn export_snippets(&mut self) {
        if self.settings.snippets.values().all(|l| l.is_empty()) {
            self.info(t!("Es gibt noch keine eigenen Snippets zum Exportieren."));
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title(t!("Snippets exportieren"))
            .set_file_name("nebula-snippets.json")
            .add_filter("Nebula-Snippets", &["json"])
            .save_file()
        else {
            return;
        };
        let count: usize = self.settings.snippets.values().map(Vec::len).sum();
        match snippet_io::export(&self.settings.snippets).and_then(|b| Ok(std::fs::write(&path, b)?)) {
            Ok(()) => self.info(if count == 1 { t!("1 Snippet exportiert.").to_string() } else { t!("{count} Snippets exportiert.", count = count) }),
            Err(e) => self.error(t!("Export fehlgeschlagen: {e}", e = format!("{:#}", e))),
        }
    }

    /// Asks for files and shows what they contain.
    pub(super) fn import_snippets(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title(t!("Snippets importieren"))
            .add_filter(t!("Snippets"), &["json", "code-snippets"])
            .add_filter(t!("Alle Dateien"), &["*"]);
        // VS Code keeps the user's snippets here.
        if let Some(dir) = std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Code").join("User").join("snippets"))
            && dir.is_dir()
        {
            dialog = dialog.set_directory(dir);
        }
        let Some(files) = dialog.pick_files() else { return };
        let mut found = Vec::new();
        let mut errors = Vec::new();
        for path in &files {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            match std::fs::read(path).map_err(anyhow::Error::from).and_then(|b| snippet_io::parse(&b, &name)) {
                Ok(list) => found.extend(list),
                Err(e) => errors.push(format!("{name}: {e:#}")),
            }
        }
        if !errors.is_empty() {
            self.error(errors.join("\n"));
        }
        if found.is_empty() {
            return;
        }
        found.sort_by(|a, b| a.key.cmp(&b.key));
        let source = match files.as_slice() {
            [one] => one.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            many => t!("{many_count} Dateien", many_count = many.len()),
        };
        let entries = found
            .into_iter()
            .map(|item| {
                let dup = snippet_io::is_duplicate(&self.settings.snippets, &item);
                (item, !dup, dup)
            })
            .collect();
        self.dialog = Some(super::Dialog::ImportSnippets(Box::new(SnippetImport { source, entries })));
    }

    /// Returns whether the dialog stays open.
    pub(super) fn snippet_import_dialog(&mut self, ctx: &egui::Context, form: &mut SnippetImport) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        let mut import = false;
        super::modal(ctx, egui::Id::unique("import-snippets")).show(ctx, |ui| {
            ui.set_width(540.0);
            ui.heading(t!("Snippets importieren – {source}", source = form.source));
            ui.add_space(6.0);
            let dupes = form.entries.iter().filter(|e| e.2).count();
            if dupes > 0 {
                let text = if dupes == 1 { t!("Eines gibt es schon, es ist abgewählt.").to_string() } else { t!("{dupes} gibt es schon, sie sind abgewählt.", dupes = dupes) };
                ui.label(RichText::new(text).small().color(theme.gutter_fg));
            }
            ui.horizontal(|ui| {
                if ui.small_button(t!("Alle")).clicked() {
                    form.entries.iter_mut().for_each(|e| e.1 = true);
                }
                if ui.small_button(t!("Keine")).clicked() {
                    form.entries.iter_mut().for_each(|e| e.1 = false);
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut last_key = None;
                for (item, selected, dup) in &mut form.entries {
                    if last_key != Some(&item.key) {
                        ui.add_space(4.0);
                        let name = if item.key == ALL_LANGUAGES {
                            t!("Alle Sprachen")
                        } else {
                            Language::from_id(&item.key).map_or(item.key.as_str(), |l| l.name())
                        };
                        ui.label(RichText::new(name.to_uppercase()).small().strong().color(theme.text_faint));
                    }
                    last_key = Some(&item.key);
                    ui.horizontal(|ui| {
                        let trigger = if item.snippet.trigger.is_empty() { t!("(ohne Kürzel)") } else { item.snippet.trigger.as_str() };
                        ui.checkbox(selected, RichText::new(trigger).monospace().color(theme.accent));
                        ui.label(RichText::new(&item.snippet.description).small().color(theme.gutter_fg));
                        if !item.snippet.group.is_empty() {
                            ui.label(RichText::new(t!("Gruppe: {group}", group = item.snippet.group)).small().color(theme.text_faint));
                        }
                        if *dup {
                            ui.label(RichText::new("vorhanden").small().color(theme.modified));
                        }
                    })
                    .response
                    .on_hover_ui(|ui| {
                        ui.label(RichText::new(&item.snippet.body).monospace());
                    });
                }
            });
            ui.add_space(10.0);
            let count = form.entries.iter().filter(|e| e.1).count();
            ui.horizontal(|ui| {
                let label = if count == 1 { t!("1 Snippet importieren").to_string() } else { t!("{count} Snippets importieren", count = count) };
                if ui.add_enabled(count > 0, primary_button(&label, theme.accent)).clicked() {
                    import = true;
                }
                if ui.button(t!("Abbrechen")).clicked() {
                    keep = false;
                }
            });
        });
        if import {
            let mut count = 0;
            for (item, _, _) in form.entries.iter().filter(|e| e.1) {
                self.settings.snippets.entry(item.key.clone()).or_default().push(item.snippet.clone());
                count += 1;
            }
            self.save_snippets();
            self.settings.right_panel = true;
            self.settings.right_tab = crate::settings::RightTab::Snippets;
            self.info(if count == 1 { t!("1 Snippet importiert.").to_string() } else { t!("{count} Snippets importiert.", count = count) });
            keep = false;
        }
        keep
    }

    fn snippet_editor(&mut self, ui: &mut egui::Ui, body: Rect, language: Language) {
        let theme = self.theme.clone();
        let mut all_groups: Vec<String> = self.settings.snippets.values().flatten().map(|s| s.group.clone()).filter(|g| !g.is_empty()).collect();
        all_groups.sort_by_key(|g| g.to_lowercase());
        all_groups.dedup();
        let Some(edit) = &mut self.right_state.snippets.edit else { return };
        let (mut save, mut cancel, mut delete) = (false, false, false);
        ui.scope_builder(egui::UiBuilder::new().max_rect(body.shrink2(vec2(12.0, 6.0))), |ui| {
            ui.label(RichText::new(if edit.original.is_some() { t!("Snippet bearbeiten") } else { t!("Neues Snippet") }).strong());
            ui.add_space(6.0);
            egui::Grid::new("snippet-edit").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(t!("Kürzel"));
                let t = ui.add(egui::TextEdit::singleline(&mut edit.snippet.trigger).hint_text(t!("z. B. card")).desired_width(f32::INFINITY));
                if edit.focus {
                    t.request_focus();
                    edit.focus = false;
                }
                ui.end_row();
                ui.label(t!("Beschreibung"));
                ui.add(egui::TextEdit::singleline(&mut edit.snippet.description).hint_text("optional").desired_width(f32::INFINITY));
                ui.end_row();
                ui.label(t!("Gruppe"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut edit.snippet.group).hint_text("keine").desired_width(ui.available_width() - 34.0));
                    if !all_groups.is_empty() {
                        let pick = ui.button("▾").on_hover_text(t!("Vorhandene Gruppe wählen"));
                        egui::Popup::menu(&pick).show(|ui| {
                            for g in &all_groups {
                                if ui.selectable_label(&edit.snippet.group == g, g).clicked() {
                                    edit.snippet.group = g.clone();
                                    ui.close();
                                }
                            }
                        });
                    }
                });
                ui.end_row();
                ui.label(t!("Gilt für"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut edit.key, language.id().to_string(), language.name());
                    ui.radio_value(&mut edit.key, ALL_LANGUAGES.to_string(), t!("alle Sprachen"));
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(t!("Inhalt"));
            let rows = ((ui.available_height() - 110.0) / 18.0).clamp(4.0, 24.0) as usize;
            ui.add(
                egui::TextEdit::multiline(&mut edit.snippet.body)
                    .code_editor()
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY)
                    .hint_text("<div class=\"${1:name}\">\n    $0\n</div>"),
            );
            ui.add_space(4.0);
            ui.label(RichText::new(crate::i18n::tr(HELP)).small().color(theme.text_faint));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let valid = !edit.snippet.body.trim().is_empty();
                if ui.add_enabled(valid, primary_button(t!("Sichern"), theme.accent)).clicked() {
                    save = true;
                }
                if ui.button(t!("Abbrechen")).clicked() {
                    cancel = true;
                }
                if edit.original.is_some() && ui.button(t!("Löschen")).clicked() {
                    delete = true;
                }
            });
        });
        if !(save || cancel || delete) {
            return;
        }
        let edit = self.right_state.snippets.edit.take().unwrap();
        if cancel {
            return;
        }
        // Take the old version out, then put the new one in (same place if the language stayed).
        let mut index = None;
        if let Some((key, n)) = &edit.original
            && let Some(list) = self.settings.snippets.get_mut(key)
            && *n < list.len()
        {
            list.remove(*n);
            if *key == edit.key {
                index = Some(*n);
            }
        }
        if save {
            let mut s = edit.snippet;
            s.trigger = s.trigger.trim().to_string();
            s.group = s.group.trim().to_string();
            s.description = s.description.trim().to_string();
            let list = self.settings.snippets.entry(edit.key).or_default();
            let at = index.unwrap_or(list.len()).min(list.len());
            list.insert(at, s);
        }
        self.settings.snippets.retain(|_, l| !l.is_empty());
        self.save_snippets();
    }

    fn save_snippets(&mut self) {
        self.settings.apply_languages();
        if let Err(e) = self.settings.save() {
            self.error(t!("Einstellungen konnten nicht gespeichert werden: {e}", e = format!("{:#}", e)));
        }
    }

    /// Inserts a snippet at the cursor (replacing the selection) and selects its first placeholder.
    pub(super) fn insert_snippet_body(&mut self, body: &str) {
        let Some(i) = self.active else { return };
        let vars = self.snippet_vars(i);
        let doc = &mut self.docs[i];
        let range = doc.primary_selection().range();
        let line = doc.line_text(doc.char_to_line(range.start));
        let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let text = body.replace('\n', &format!("\n{indent}"));
        doc.insert_snippet(range, snippet::expand(&text, &vars));
        doc.view.scroll_to_cursor = true;
        self.focus_editor = true;
    }
}

fn heading(ui: &mut egui::Ui, theme: &crate::theme::Theme, title: &str) -> egui::Response {
    let resp = ui.label(RichText::new(title.to_uppercase()).size(10.5).strong().color(theme.text_faint));
    ui.add_space(2.0);
    resp
}

/// Foldable heading of a snippet group with the number of snippets.
fn group_heading(ui: &mut egui::Ui, theme: &crate::theme::Theme, title: &str, open: bool, count: usize) -> egui::Response {
    ui.add_space(4.0);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 5.0, theme.hover);
    }
    let cy = r.center().y;
    icons::chevron(ui.painter(), pos2(r.left() + 9.0, cy), open, theme.text_faint);
    let galley = ui.painter().layout_no_wrap(title.to_uppercase(), FontId::proportional(10.5), theme.text_faint);
    let w = galley.size().x;
    ui.painter().galley(pos2(r.left() + 20.0, cy - galley.size().y / 2.0), galley, theme.text_faint);
    ui.painter().text(pos2(r.left() + 26.0 + w, cy), Align2::LEFT_CENTER, count.to_string(), FontId::proportional(10.0), theme.gutter_fg);
    resp
}

/// Payload while dragging an own snippet: (language key, index).
#[derive(Clone)]
struct SnippetDrag(String, usize);

/// `resp` accepts a dragged snippet and moves it into `group` (empty: none).
fn snippet_drop_target(ui: &egui::Ui, theme: &crate::theme::Theme, resp: &egui::Response, group: &str, action: &mut Option<Action>) {
    if resp.dnd_hover_payload::<SnippetDrag>().is_some() {
        ui.painter().rect_stroke(resp.rect.expand(2.0), 5.0, egui::Stroke::new(1.5, theme.accent), egui::StrokeKind::Inside);
    }
    if let Some(p) = resp.dnd_release_payload::<SnippetDrag>() {
        *action = Some(Action::SetGroup(p.0.clone(), p.1, group.to_string()));
    }
}

/// Two-line row: trigger and description, then the first line of the body.
fn row(ui: &mut egui::Ui, theme: &crate::theme::Theme, trigger: &str, desc: &str, body: &str, tag: Option<&str>) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click_and_drag());
    if resp.hovered() {
        ui.painter().rect_filled(r, 5.0, theme.hover);
    }
    let painter = ui.painter().with_clip_rect(r);
    let x = r.left() + 8.0;
    let top = r.top() + 12.0;
    let trigger = if trigger.is_empty() { t!("(ohne Kürzel)") } else { trigger };
    let galley = painter.layout_no_wrap(trigger.to_string(), FontId::monospace(12.5), theme.accent);
    let w = galley.size().x;
    painter.galley(pos2(x, top - galley.size().y / 2.0), galley, theme.accent);
    painter.text(pos2(x + w + 10.0, top), Align2::LEFT_CENTER, desc, FontId::proportional(12.5), theme.foreground);
    if let Some(tag) = tag {
        painter.text(pos2(r.right() - 8.0, top), Align2::RIGHT_CENTER, tag, FontId::proportional(10.5), theme.text_faint);
    }
    let first = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    painter.text(pos2(x, r.top() + 29.0), Align2::LEFT_CENTER, first, FontId::monospace(11.0), theme.text_faint);
    resp.on_hover_ui(|ui| {
        ui.label(RichText::new(body).monospace());
    })
}
