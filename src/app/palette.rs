//! Command palette (Ctrl+Shift+P): every command, searchable.

use eframe::egui::{self, Key, Modifiers, RichText};

use super::{NebulaApp, settings_page::Page};
use crate::editor::commands as cmd;
use crate::editor::document::{Document, LineEnding};
use crate::lang::Language;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    NewFile,
    OpenFile,
    OpenFolder,
    QuickOpen,
    Save,
    SaveAs,
    SaveAll,
    CloseTab,
    Find,
    Replace,
    FindInProject,
    GotoLine,
    ToggleSidebar,
    Settings,
    Themes,
    Languages,
    AddServer,
    Publish,
    PublishSetup,
    OpenInBrowser,
    TogglePreview,
    ExtLibrary,
    ExtManage,
    ExtConsole,
    ToggleTheme,
    FontBigger,
    FontSmaller,
    ToggleInvisibles,
    ToggleWordWrap,
    Print,
    DuplicateDoc,
    CloseOthers,
    CloseAll,
    ToggleComment,
    DuplicateLine,
    DeleteLine,
    SelectAllOccurrences,
    LineEndingLf,
    LineEndingCrLf,
    TrimWhitespace,
    IndentToSpaces,
    IndentToTabs,
    FoldToggle,
    FoldAll,
    UnfoldAll,
    Language(Language),
    /// Index into `available_ext_commands()`.
    Ext(usize),
    /// Index into `available_clips()`.
    Clip(usize),
    ToggleTerminal,
    /// SSH terminal for the server at this index.
    Terminal(usize),
    SwitchProject(usize),
}

impl Command {
    fn all() -> Vec<(Command, String, &'static str)> {
        use Command::*;
        let mut v: Vec<(Command, String, &'static str)> = [
            (NewFile, "Datei: Neu", "Strg+N"),
            (OpenFile, "Datei: Öffnen…", "Strg+O"),
            (OpenFolder, "Datei: Ordner öffnen…", "Strg+Umschalt+O"),
            (QuickOpen, "Datei: Schnell öffnen…", "Strg+P"),
            (Save, "Datei: Speichern", "Strg+S"),
            (SaveAs, "Datei: Speichern unter…", "Strg+Umschalt+S"),
            (SaveAll, "Datei: Alle speichern", "Strg+Alt+S"),
            (CloseTab, "Datei: Tab schließen", "Strg+W"),
            (CloseOthers, "Datei: Andere Tabs schließen", ""),
            (CloseAll, "Datei: Alle Tabs schließen", ""),
            (DuplicateDoc, "Datei: Duplizieren", ""),
            (Print, "Datei: Drucken…", "Strg+Alt+P"),
            (Find, "Suchen: In Datei suchen", "Strg+F"),
            (Replace, "Suchen: Ersetzen", "Strg+H"),
            (FindInProject, "Suchen: Im Projekt suchen", "Strg+Umschalt+F"),
            (GotoLine, "Gehe zu Zeile…", "Strg+G"),
            (ToggleSidebar, "Ansicht: Seitenleiste ein/aus", "Strg+B"),
            (ToggleTheme, "Ansicht: Hell/Dunkel umschalten", ""),
            (FontBigger, "Ansicht: Schrift größer", "Strg++"),
            (FontSmaller, "Ansicht: Schrift kleiner", "Strg+-"),
            (ToggleInvisibles, "Ansicht: Unsichtbare Zeichen ein/aus", ""),
            (ToggleWordWrap, "Ansicht: Automatischer Zeilenumbruch ein/aus", "Alt+Z"),
            (ToggleTerminal, "Terminal: SSH-Terminal ein/aus", "Strg+Ö"),
            (Settings, "Einstellungen: Editor", "Strg+,"),
            (Themes, "Einstellungen: Design und Themes", ""),
            (Languages, "Einstellungen: Sprachen", ""),
            (AddServer, "Server: Hinzufügen…", ""),
            (Publish, "Projekt: Veröffentlichen", ""),
            (PublishSetup, "Projekt: Veröffentlichen einrichten…", ""),
            (ExtLibrary, "Erweiterungen: Bibliothek durchsuchen…", ""),
            (ExtManage, "Erweiterungen: Verwalten…", ""),
            (ExtConsole, "Erweiterungen: Konsole", ""),
            (TogglePreview, "Vorschau: Live-Vorschau ein/aus", "Strg+Umschalt+V"),
            (OpenInBrowser, "Vorschau: Im Browser öffnen", "F5"),
            (ToggleComment, "Bearbeiten: Kommentar umschalten", "Strg+/"),
            (DuplicateLine, "Bearbeiten: Zeile duplizieren", "Strg+Umschalt+D"),
            (DeleteLine, "Bearbeiten: Zeile löschen", "Strg+Umschalt+K"),
            (SelectAllOccurrences, "Bearbeiten: Alle Vorkommen auswählen", "Strg+Umschalt+L"),
            (FoldToggle, "Faltung: Block ein-/ausklappen", ""),
            (FoldAll, "Faltung: Alles einklappen", ""),
            (UnfoldAll, "Faltung: Alles ausklappen", ""),
            (LineEndingLf, "Zeilenenden: LF (Unix)", ""),
            (LineEndingCrLf, "Zeilenenden: CRLF (Windows)", ""),
            (TrimWhitespace, "Text: Leerzeichen am Zeilenende entfernen", ""),
            (IndentToSpaces, "Text: Einrückung in Leerzeichen umwandeln", ""),
            (IndentToTabs, "Text: Einrückung in Tabs umwandeln", ""),
        ]
        .into_iter()
        .map(|(c, l, s)| (c, l.to_string(), s))
        .collect();
        for l in crate::lang::Language::ALL {
            v.push((Command::Language(l), format!("Sprache festlegen: {}", l.name()), ""));
        }
        v
    }
}

/// Subsequence match: all query chars in order; contiguous runs score higher.
fn score(label: &str, query: &str) -> Option<i32> {
    let label = label.to_lowercase();
    let q = query.trim().to_lowercase();
    // Whole query as a substring beats scattered letters; at a word start even more.
    if let Some(pos) = label.find(&q) {
        let word_start = pos == 0 || !label.as_bytes()[pos - 1].is_ascii_alphanumeric();
        return Some(10_000 + if word_start { 5_000 } else { 0 } - pos as i32);
    }
    let mut s = 0;
    let mut last = None;
    let mut chars = label.char_indices();
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let (i, _) = chars.find(|(_, c)| *c == q)?;
        s += if last == Some(i.wrapping_sub(1)) { 5 } else { 1 };
        last = Some(i);
    }
    Some(s * 10 - label.len() as i32 / 4)
}

pub struct PaletteState {
    pub query: String,
    pub selected: usize,
    pub focus: bool,
}

impl NebulaApp {
    /// Draws the palette; returns false when it closed.
    pub(super) fn command_palette(&mut self, ctx: &egui::Context, state: &mut PaletteState) -> bool {
        let mut run = None;
        let mut keep = true;
        let mut all = Command::all();
        for (n, c) in self.available_ext_commands().iter().enumerate() {
            all.push((Command::Ext(n), format!("{}: {}", c.ext_name, c.title), ""));
        }
        for (n, (ext, c)) in self.available_clips().iter().enumerate() {
            all.push((Command::Clip(n), format!("Clip: {} ({ext})", c.name), ""));
        }
        for (n, s) in self.sites.sites.iter().enumerate() {
            all.push((Command::Terminal(n), format!("Terminal: SSH-Terminal für {}", s.display_name()), ""));
        }
        for (n, p) in self.settings.recent_folders.iter().enumerate().skip(1) {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            all.push((Command::SwitchProject(n), format!("Projekt wechseln: {name}  ({})", p.display()), ""));
        }
        let mut items: Vec<(Command, String, &'static str, i32)> = all
            .into_iter()
            .filter_map(|(c, l, s)| Some((c, l.clone(), s, if state.query.is_empty() { 0 } else { score(&l, &state.query)? })))
            .collect();
        items.sort_by(|a, b| b.3.cmp(&a.3));
        items.truncate(14);
        super::modal(ctx, egui::Id::unique("palette")).show(ctx, |ui| {
            ui.set_width(560.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("Befehl eingeben…")
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Heading),
            );
            if std::mem::take(&mut state.focus) || resp.changed() {
                resp.request_focus();
            }
            if resp.changed() {
                state.selected = 0;
            }
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    state.selected = (state.selected + 1).min(items.len().saturating_sub(1));
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    state.selected = state.selected.saturating_sub(1);
                }
            });
            ui.add_space(6.0);
            for (n, (c, label, shortcut, _)) in items.iter().enumerate() {
                let r = ui.add(
                    egui::Button::selectable(n == state.selected, RichText::new(label).size(14.0))
                        .shortcut_text(RichText::new(*shortcut).color(self.theme.text_faint))
                        .min_size(egui::vec2(540.0, 28.0)),
                );
                if r.clicked() {
                    run = Some(*c);
                }
            }
            if items.is_empty() {
                ui.label(RichText::new("Kein passender Befehl").color(self.theme.text_faint));
            }
            if ui.input(|i| i.key_pressed(Key::Enter))
                && let Some((c, ..)) = items.get(state.selected)
            {
                run = Some(*c);
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                keep = false;
            }
        });
        if let Some(c) = run {
            // Close first: some commands open their own dialog.
            self.dialog = None;
            self.run_command(ctx, c);
            return false;
        }
        keep
    }

    pub(super) fn run_command(&mut self, ctx: &egui::Context, c: Command) {
        let with_doc = |app: &mut NebulaApp, f: fn(&mut Document)| {
            if let Some(i) = app.active {
                f(&mut app.docs[i]);
                app.focus_editor = true;
            }
        };
        match c {
            Command::NewFile => self.new_document(),
            Command::OpenFile => self.pick_and_open_file(),
            Command::OpenFolder => self.pick_and_open_folder(),
            Command::QuickOpen => self.open_quick_open(),
            Command::Save => {
                if let Some(i) = self.active {
                    self.save_doc(i, false);
                }
            }
            Command::SaveAs => {
                if let Some(i) = self.active {
                    self.save_doc_as(i);
                }
            }
            Command::SaveAll => self.save_all(),
            Command::CloseTab => {
                if let Some(i) = self.active {
                    self.close_doc(i, false);
                }
            }
            Command::Find => self.open_find(false),
            Command::Replace => self.open_find(true),
            Command::FindInProject => self.open_project_search(),
            Command::GotoLine => self.dialog = Some(super::Dialog::GotoLine { value: String::new(), focus: true }),
            Command::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Command::Settings => self.open_settings(Page::Editor),
            Command::Themes => self.open_settings(Page::Design),
            Command::Languages => self.open_settings(Page::Languages),
            Command::AddServer => self.open_site_form(None),
            Command::Publish => self.publish(),
            Command::PublishSetup => self.open_publish_setup(),
            Command::OpenInBrowser => self.open_in_browser(ctx),
            Command::TogglePreview => self.toggle_preview(),
            Command::ExtLibrary | Command::ExtManage | Command::ExtConsole => {
                self.settings_ui.ext.tab = match c {
                    Command::ExtLibrary => super::extensions_page::Tab::Library,
                    Command::ExtConsole => super::extensions_page::Tab::Console,
                    _ => super::extensions_page::Tab::Installed,
                };
                self.open_settings(super::settings_page::Page::Extensions);
            }
            Command::ToggleTheme => {
                self.settings.dark_theme = !self.theme.dark;
                self.settings.theme = if self.settings.dark_theme { crate::theme::BUILTIN_DARK } else { crate::theme::BUILTIN_LIGHT }.into();
                self.theme = crate::theme::Theme::for_settings(&self.settings);
                self.theme.apply(ctx);
                self.backdrop_state = None;
            }
            Command::FontBigger => self.settings.editor.font_size = (self.settings.editor.font_size + 1.0).min(40.0),
            Command::FontSmaller => self.settings.editor.font_size = (self.settings.editor.font_size - 1.0).max(8.0),
            Command::ToggleInvisibles => self.settings.editor.show_invisibles = !self.settings.editor.show_invisibles,
            Command::ToggleWordWrap => self.settings.editor.word_wrap = !self.settings.editor.word_wrap,
            Command::ToggleTerminal => self.toggle_terminal(ctx),
            Command::Terminal(n) => {
                if let Some(id) = self.sites.sites.get(n).map(|s| s.id.clone()) {
                    self.open_terminal(ctx, &id);
                }
            }
            Command::SwitchProject(n) => {
                if let Some(p) = self.settings.recent_folders.get(n).cloned() {
                    self.handle_sidebar_action(ctx, crate::ui::sidebar::SidebarAction::SwitchFolder(p));
                }
            }
            Command::Print => self.print_active(ctx),
            Command::DuplicateDoc => {
                if let Some(i) = self.active {
                    self.duplicate_doc(i);
                }
            }
            Command::CloseOthers => {
                if let Some(i) = self.active {
                    self.close_others(i);
                }
            }
            Command::CloseAll => self.close_many((0..self.docs.len()).collect()),
            Command::ToggleComment => with_doc(self, cmd::toggle_comment),
            Command::DuplicateLine => with_doc(self, cmd::duplicate_lines),
            Command::DeleteLine => with_doc(self, cmd::delete_lines),
            Command::SelectAllOccurrences => with_doc(self, cmd::select_all_occurrences),
            Command::TrimWhitespace => with_doc(self, |d| cmd::tidy_whitespace(d, true, false)),
            Command::IndentToSpaces | Command::IndentToTabs => {
                if let Some(i) = self.active {
                    let tab = self.settings.editor_for_doc(&self.docs[i]).tab_size;
                    cmd::convert_indentation(&mut self.docs[i], c == Command::IndentToSpaces, tab);
                }
            }
            Command::LineEndingLf | Command::LineEndingCrLf => {
                if let Some(i) = self.active {
                    let doc = &mut self.docs[i];
                    doc.line_ending = if c == Command::LineEndingLf { LineEnding::Lf } else { LineEnding::CrLf };
                    doc.mark_unsaved();
                }
            }
            Command::FoldToggle | Command::FoldAll | Command::UnfoldAll => {
                if let Some(i) = self.active {
                    let tab = self.settings.editor_for_doc(&self.docs[i]).tab_size;
                    let doc = &mut self.docs[i];
                    match c {
                        Command::FoldToggle => crate::editor::folding::toggle_at_cursor(doc, tab),
                        Command::FoldAll => crate::editor::folding::fold_all(doc, tab),
                        _ => crate::editor::folding::unfold_all(doc),
                    }
                    self.focus_editor = true;
                }
            }
            Command::Clip(n) => {
                if let Some((_, c)) = self.available_clips().get(n).cloned() {
                    self.insert_clip(&c);
                }
            }
            Command::Ext(n) => {
                if let Some(c) = self.available_ext_commands().get(n).cloned() {
                    self.run_ext_command(&c);
                    // Edits the command makes can be undone right away.
                    self.focus_editor = true;
                }
            }
            Command::Language(l) => {
                if let Some(i) = self.active {
                    self.docs[i].language = l;
                    self.docs[i].highlight_version = u64::MAX;
                }
            }
        }
    }
}
