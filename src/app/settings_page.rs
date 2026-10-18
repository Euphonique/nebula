//! The settings window: editor options, themes and languages.

use eframe::egui::{self, Color32, RichText, Ui};

use super::{NebulaApp, primary_button};
use crate::lang::Language;
use crate::settings::LanguageSettings;
use crate::theme::{
    self, BUILTIN_DARK, HIGHLIGHT_NAMES, Theme, ThemeFile, UI_COLORS, color_from_hex, color_to_hex,
};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Editor,
    Design,
    Languages,
    Keys,
    Extensions,
}

/// Transient state of the settings window.
#[derive(Default)]
pub struct SettingsUi {
    pub page: Page,
    themes: Vec<ThemeFile>,
    language: Option<Language>,
    /// Text fields for the selected language (extensions, completions).
    ext_text: String,
    completion_text: String,
    syntax_filter: String,
    /// Search text in the font picker.
    font_filter: String,
    font_picker_open: bool,
    /// `theme::themes_generation()` when `themes` was loaded.
    themes_gen: u64,
    /// Extension shown on the extensions page.
    pub selected_extension: Option<String>,
    pub ext: super::extensions_page::ExtPageState,
    pub keys: super::keys::KeysPage,
}

impl SettingsUi {
    pub fn open(page: Page) -> Self {
        Self { page, themes: theme::load_themes(), themes_gen: theme::themes_generation(), ..Default::default() }
    }

    pub fn reload_themes(&mut self) {
        self.themes = theme::load_themes();
        self.themes_gen = theme::themes_generation();
    }
}

impl NebulaApp {
    /// Draws the settings window. Returns false when it was closed.
    pub(super) fn settings_window(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut close = false;
        let mut state = std::mem::take(&mut self.settings_ui);
        super::modal(ctx, egui::Id::unique("settings-window")).show(ctx, |ui| {
            ui.set_width(820.0);
            ui.set_height(560.0);
            ui.horizontal(|ui| {
                ui.heading("Einstellungen");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary_button("Fertig", self.theme.accent)).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                // Navigation.
                ui.vertical(|ui| {
                    ui.set_width(150.0);
                    for (page, label) in [(Page::Editor, "Editor"), (Page::Design, "Design"), (Page::Languages, "Sprachen"), (Page::Keys, "Tastenkürzel"), (Page::Extensions, "Erweiterungen")] {
                        let sel = state.page == page;
                        if ui
                            .add_sized([140.0, 30.0], egui::Button::selectable(sel, RichText::new(label).size(14.0)))
                            .clicked()
                        {
                            state.page = page;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt("settings-page").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(620.0);
                        match state.page {
                            Page::Editor => self.editor_page(ui, &mut state),
                            Page::Design => self.design_page(ui, &mut state),
                            Page::Languages => self.languages_page(ui, &mut state),
                            Page::Keys => self.keys_page(ui, &mut state.keys),
                            Page::Extensions => {
                                self.settings_ui = std::mem::take(&mut state);
                                self.extensions_page(ui);
                                state = std::mem::take(&mut self.settings_ui);
                            }
                        }
                    });
                });
            });
            if state.keys.recording.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
        self.settings_ui = state;
        if close {
            open = false;
            let _ = self.settings.save();
        }
        open
    }

    fn editor_page(&mut self, ui: &mut Ui, state: &mut SettingsUi) {
        let e = &mut self.settings.editor;
        section(ui, "Darstellung");
        egui::Grid::new("editor-grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Schriftart");
            let current = if e.font_family.is_empty() { "Standard (Cascadia Mono)".to_string() } else { e.font_family.clone() };
            // Opens in place: a popup outside the modal settings window gets no keyboard input.
            ui.vertical(|ui| {
                let open = &mut state.font_picker_open;
                if ui.add(egui::Button::new(format!("{current}  {}", if *open { "▴" } else { "▾" })).min_size(egui::vec2(260.0, 0.0))).clicked() {
                    *open = !*open;
                    state.font_filter.clear();
                }
                if !*open {
                    return;
                }
                let filter = ui.add(egui::TextEdit::singleline(&mut state.font_filter).hint_text("Schrift suchen…").desired_width(260.0));
                if !filter.has_focus() && state.font_filter.is_empty() {
                    filter.request_focus();
                }
                let q = state.font_filter.to_lowercase();
                let mut chosen = None;
                egui::ScrollArea::vertical().id_salt("font-list").max_height(220.0).show(ui, |ui| {
                    ui.set_width(260.0);
                    if q.is_empty() && ui.selectable_label(e.font_family.is_empty(), "Standard (Cascadia Mono)").clicked() {
                        chosen = Some(String::new());
                    }
                    for family in crate::fonts::installed().keys().filter(|f| f.to_lowercase().contains(&q)) {
                        if ui.selectable_label(&e.font_family == family, family).clicked() {
                            chosen = Some(family.clone());
                        }
                    }
                });
                if let Some(family) = chosen {
                    e.font_family = family;
                    state.font_picker_open = false;
                }
            });
            ui.end_row();
            ui.label("Schriftgröße");
            ui.add(egui::Slider::new(&mut e.font_size, 8.0..=32.0).step_by(0.5));
            ui.end_row();
            ui.label("Zeilenhöhe");
            ui.add(egui::Slider::new(&mut e.line_height, 1.0..=2.5).step_by(0.05).fixed_decimals(2));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.highlight_current_line, "Aktuelle Zeile hervorheben");
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.show_indent_guides, "Einrückungshilfslinien");
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.show_invisibles, "Unsichtbare Zeichen anzeigen");
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.word_wrap, "Automatischer Zeilenumbruch");
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.minimap, "Minimap am rechten Rand");
            ui.end_row();
        });
        section(ui, "Einrückung (Standard)");
        egui::Grid::new("indent-grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Tabulatorbreite");
            ui.add(egui::DragValue::new(&mut e.tab_size).range(1..=8));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.use_spaces, "Leerzeichen statt Tabs");
            ui.end_row();
        });
        ui.label(RichText::new("Pro Sprache lässt sich das unter „Sprachen“ abweichend einstellen.").small().color(self.theme.text_faint));
        section(ui, "Beim Speichern");
        ui.checkbox(&mut e.trim_trailing_whitespace, "Leerzeichen am Zeilenende entfernen (nicht in Markdown)");
        ui.checkbox(&mut e.insert_final_newline, "Zeilenumbruch am Dateiende sicherstellen");
        ui.label(RichText::new("Eine .editorconfig im Projekt hat Vorrang vor diesen Einstellungen (Einrückung, Leerzeichen, Dateiende).").small().color(self.theme.text_faint));
        section(ui, "Automatisch speichern");
        {
            use crate::settings::AutoSave;
            let label = |a: AutoSave| match a {
                AutoSave::Off => "Aus",
                AutoSave::Delay => "Nach einer Tipppause",
                AutoSave::FocusChange => "Beim Wechsel von Tab oder Fenster",
            };
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("autosave").selected_text(label(e.autosave)).width(240.0).show_ui(ui, |ui| {
                    for a in [AutoSave::Off, AutoSave::Delay, AutoSave::FocusChange] {
                        ui.selectable_value(&mut e.autosave, a, label(a));
                    }
                });
                if e.autosave == AutoSave::Delay {
                    ui.add(egui::DragValue::new(&mut e.autosave_delay).range(0.5..=60.0).speed(0.1).suffix(" s"));
                }
            });
            ui.add_enabled(e.autosave != AutoSave::Off, egui::Checkbox::new(&mut e.autosave_remote, "Auch Serverdateien (jedes Speichern lädt hoch)"));
            ui.label(
                RichText::new("Automatisches Speichern entfernt keine Leerzeichen am Zeilenende und lässt Formatierer aus; das passiert beim Speichern mit Strg+S.")
                    .small()
                    .color(self.theme.text_faint),
            );
        }
        section(ui, "Lokaler Verlauf");
        ui.horizontal(|ui| {
            ui.checkbox(&mut e.local_history, "Frühere Fassungen von Dateien aufbewahren");
            ui.add_enabled(e.local_history, egui::DragValue::new(&mut e.history_max).range(1..=500).suffix(" je Datei"));
        });
        ui.label(
            RichText::new("Beim Öffnen und bei jedem Speichern, auch für Serverdateien. Ansehen und wiederherstellen über das Tab-Menü „Lokaler Verlauf…“.")
                .small()
                .color(self.theme.text_faint),
        );
        section(ui, "Eingabehilfen");
        ui.checkbox(&mut e.auto_pair, "Klammern und Anführungszeichen automatisch schließen");
        ui.checkbox(&mut e.auto_close_tags, "HTML-Tags automatisch schließen");
        ui.checkbox(&mut e.emmet, "Emmet-Kürzel mit Tab expandieren (z. B. ul>li*3)");
        section(ui, "Terminal");
        let shells = crate::remote::terminal::local_shells();
        let current = shells
            .iter()
            .find(|s| s.name == self.settings.terminal_shell)
            .unwrap_or(&shells[0])
            .name
            .clone();
        ui.horizontal(|ui| {
            ui.label("Lokale Shell");
            egui::ComboBox::from_id_salt("terminal-shell").selected_text(&current).width(220.0).show_ui(ui, |ui| {
                for s in &shells {
                    if ui.selectable_label(s.name == current, &s.name).on_hover_text(&s.program).clicked() {
                        self.settings.terminal_shell = s.name.clone();
                    }
                }
            });
        });
        ui.label(RichText::new("Strg+Ö öffnet bei lokalen Dateien diese Shell im Projektordner, bei Serverdateien ein SSH-Terminal.").small().color(self.theme.text_faint));
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        self.theme = Theme::for_settings(&self.settings);
        self.theme.apply(ctx);
        self.backdrop_state = None;
    }

    fn design_page(&mut self, ui: &mut Ui, state: &mut SettingsUi) {
        let ctx = ui.ctx().clone();
        // Extensions installed meanwhile bring their themes.
        if state.themes_gen != theme::themes_generation() {
            state.reload_themes();
        }
        section(ui, "Theme");
        let current = if self.settings.theme.is_empty() { self.theme.name.clone() } else { self.settings.theme.clone() };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("theme-select")
                .selected_text(&current)
                .width(260.0)
                .show_ui(ui, |ui| {
                    for t in &state.themes {
                        let label = match &t.extension {
                            _ if t.is_builtin() => format!("{} (mitgeliefert)", t.name),
                            Some(ext) => format!("{} ({ext})", t.name),
                            None => t.name.clone(),
                        };
                        if ui.selectable_label(t.name == current, label).clicked() {
                            self.settings.theme = t.name.clone();
                            self.settings.dark_theme = t.dark;
                        }
                    }
                });
            if ui.button("Duplizieren").on_hover_text("Eigene, bearbeitbare Kopie anlegen").clicked() {
                let mut name = format!("{current} Kopie");
                let mut n = 2;
                while state.themes.iter().any(|t| t.name == name) {
                    name = format!("{current} Kopie {n}");
                    n += 1;
                }
                let file = self.theme_without_mica().to_file(&name);
                match theme::save_theme(&file) {
                    Ok(()) => {
                        self.settings.theme = name;
                        state.themes = theme::load_themes();
                    }
                    Err(e) => self.error(format!("Theme konnte nicht gespeichert werden: {e:#}")),
                }
            }
            if ui.button("Importieren…").on_hover_text("Nebula-Theme (.json) oder Nova-Theme (.css)").clicked() {
                self.import_theme_dialog(state);
            }
            if ui.button("Exportieren…").clicked() {
                self.export_theme_dialog(&current);
            }
        });
        if current != self.theme.name {
            self.apply_theme(&ctx);
        }
        let builtin = state.themes.iter().find(|t| t.name == current).is_none_or(|t| t.is_builtin());
        let selected = state.themes.iter().find(|t| t.name == current);
        let from_extension = selected.and_then(|t| t.extension.clone());
        let overrides_extension = selected.is_some_and(|t| t.overrides_extension);
        ui.add_space(4.0);
        let mut mica = self.settings.mica;
        if ui.checkbox(&mut mica, "Mica-Hintergrund in der Seitenleiste (Windows 11)").changed() {
            self.settings.mica = mica;
            self.apply_theme(&ctx);
        }

        if builtin {
            ui.add_space(10.0);
            ui.label(
                RichText::new("Mitgelieferte Themes sind schreibgeschützt. „Duplizieren“ legt eine bearbeitbare Kopie an.")
                    .color(self.theme.text_faint),
            );
        } else if let Some(ext) = &from_extension {
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("Stammt aus der Erweiterung „{ext}“. Änderungen werden als eigene Fassung gespeichert."))
                    .color(self.theme.text_faint),
            );
        } else if overrides_extension {
            ui.horizontal(|ui| {
                if ui.button("Auf Original der Erweiterung zurücksetzen").clicked() {
                    theme::delete_theme(&current);
                    state.themes = theme::load_themes();
                    self.apply_theme(&ctx);
                }
            });
        } else {
            ui.horizontal(|ui| {
                if ui.button("Theme löschen").clicked() {
                    theme::delete_theme(&current);
                    self.settings.theme = if self.theme.dark { BUILTIN_DARK.into() } else { theme::BUILTIN_LIGHT.into() };
                    state.themes = theme::load_themes();
                    self.apply_theme(&ctx);
                }
            });
        }

        // Color editor (read-only preview for built-in themes).
        let Some(idx) = state.themes.iter().position(|t| t.name == current) else { return };
        let mut changed = false;
        section(ui, "Oberfläche");
        ui.add_enabled_ui(!builtin, |ui| {
            egui::Grid::new("ui-colors").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                for (n, (key, label)) in UI_COLORS.iter().enumerate() {
                    let file = &mut state.themes[idx];
                    let mut c = file.ui.get(*key).and_then(|h| color_from_hex(h)).unwrap_or(Color32::GRAY);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        file.ui.insert(key.to_string(), color_to_hex(c));
                        changed = true;
                    }
                    ui.label(*label);
                    if n % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        });
        section(ui, "Syntax");
        ui.add(egui::TextEdit::singleline(&mut state.syntax_filter).hint_text("Kategorie filtern…").desired_width(240.0));
        ui.add_space(4.0);
        ui.add_enabled_ui(!builtin, |ui| {
            let filter = state.syntax_filter.to_lowercase();
            egui::Grid::new("syntax-colors").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for name in HIGHLIGHT_NAMES.iter().filter(|n| n.contains(&filter)) {
                    let file = &mut state.themes[idx];
                    let mut c = file.syntax.get(*name).and_then(|h| color_from_hex(h)).unwrap_or(Color32::GRAY);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        file.syntax.insert(name.to_string(), color_to_hex(c));
                        changed = true;
                    }
                    ui.label(RichText::new(*name).monospace().color(c));
                    ui.end_row();
                }
            });
        });
        if changed {
            if let Err(e) = theme::save_theme(&state.themes[idx]) {
                self.error(format!("Theme konnte nicht gespeichert werden: {e:#}"));
            }
            self.apply_theme(&ctx);
        }
    }

    /// The current theme with its original (opaque) sidebar color.
    fn theme_without_mica(&self) -> Theme {
        let mut s = self.settings.clone();
        s.mica = false;
        Theme::for_settings(&s)
    }

    fn import_theme_dialog(&mut self, state: &mut SettingsUi) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Themes", &["json", "css"])
            .set_title("Theme importieren")
            .pick_file()
        else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.error(format!("Datei konnte nicht gelesen werden: {e}"));
                return;
            }
        };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let parsed = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("css")) {
            crate::theme_import::import_nova_css(&text, &stem)
        } else {
            serde_json::from_str::<ThemeFile>(&text).map_err(|e| anyhow::anyhow!("Ungültige Theme-Datei: {e}"))
        };
        match parsed {
            Ok(file) => {
                // Fill colors the source doesn't define from the base theme.
                let mut file = Theme::from_file(&file).to_file(&file.name);
                if file.is_builtin() {
                    file.name = format!("{} (importiert)", file.name);
                }
                if let Err(e) = theme::save_theme(&file) {
                    self.error(format!("{e:#}"));
                    return;
                }
                self.info(format!("Theme „{}“ importiert", file.name));
                self.settings.theme = file.name.clone();
                self.settings.dark_theme = file.dark;
                state.themes = theme::load_themes();
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    fn export_theme_dialog(&mut self, name: &str) {
        let file = self.theme_without_mica().to_file(name);
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.json"))
            .add_filter("Nebula-Theme", &["json"])
            .save_file()
        else {
            return;
        };
        match serde_json::to_vec_pretty(&file).map_err(anyhow::Error::from).and_then(|b| Ok(std::fs::write(&path, b)?)) {
            Ok(()) => self.info(format!("Theme exportiert nach {}", path.display())),
            Err(e) => self.error(format!("Export fehlgeschlagen: {e:#}")),
        }
    }

    fn languages_page(&mut self, ui: &mut Ui, state: &mut SettingsUi) {
        let lang = *state.language.get_or_insert_with(|| {
            self.active.and_then(|i| self.docs.get(i)).map_or(Language::Html, |d| d.language)
        });
        // Load text fields when the selection changes.
        let select = |state: &mut SettingsUi, l: Language, settings: &crate::settings::AppSettings| {
            let ls = settings.languages.get(l.id()).cloned().unwrap_or_default();
            state.language = Some(l);
            state.ext_text = ls.extensions.join(", ");
            state.completion_text = ls.completions.join("\n");
        };
        if state.ext_text.is_empty() && state.completion_text.is_empty() {
            select(state, lang, &self.settings);
        }

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                egui::ScrollArea::vertical().id_salt("lang-list").max_height(480.0).show(ui, |ui| {
                    for l in Language::ALL {
                        let customized = self.settings.languages.get(l.id()).is_some_and(|s| *s != LanguageSettings::default());
                        let label = if customized { format!("{} •", l.name()) } else { l.name().to_string() };
                        if crate::ui::icons::language_item(ui, l == lang, l, &label).clicked() && l != lang {
                            select(state, l, &self.settings);
                        }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                let mut ls = self.settings.languages.get(lang.id()).cloned().unwrap_or_default();
                let before = ls.clone();
                ui.heading(lang.name());
                ui.add_space(6.0);
                section(ui, "Dateiendungen");
                ui.label(RichText::new(format!("Eingebaut: {}", lang.default_extensions().join(", "))).color(self.theme.text_faint));
                ui.horizontal(|ui| {
                    ui.label("Zusätzlich:");
                    if ui
                        .add(egui::TextEdit::singleline(&mut state.ext_text).hint_text("z. B. tpl, inc").desired_width(260.0))
                        .changed()
                    {
                        ls.extensions = state
                            .ext_text
                            .split([',', ' ', ';'])
                            .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                });

                section(ui, "Einrückung");
                let mut custom = ls.tab_size.is_some() || ls.use_spaces.is_some();
                if ui.checkbox(&mut custom, "Eigene Einrückung für diese Sprache").changed() {
                    if custom {
                        ls.tab_size = Some(self.settings.editor.tab_size);
                        ls.use_spaces = Some(self.settings.editor.use_spaces);
                    } else {
                        ls.tab_size = None;
                        ls.use_spaces = None;
                    }
                }
                if custom {
                    ui.horizontal(|ui| {
                        let mut t = ls.tab_size.unwrap_or(4);
                        ui.label("Breite");
                        if ui.add(egui::DragValue::new(&mut t).range(1..=8)).changed() {
                            ls.tab_size = Some(t);
                        }
                        let mut s = ls.use_spaces.unwrap_or(true);
                        if ui.checkbox(&mut s, "Leerzeichen").changed() {
                            ls.use_spaces = Some(s);
                        }
                    });
                }

                section(ui, "Eigene Vervollständigungen");
                ui.label(
                    RichText::new(
                        "Eine Zeile pro Eintrag: ein Wort, oder „Kürzel => Text“. $0 setzt den Cursor, \\n bricht die Zeile um.\nBeispiel:  card => <div class=\"card\">\\n    $0\\n</div>",
                    )
                    .small()
                    .color(self.theme.text_faint),
                );
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut state.completion_text)
                            .code_editor()
                            .desired_rows(10)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    ls.completions = state
                        .completion_text
                        .lines()
                        .map(str::to_string)
                        .filter(|l| !l.trim().is_empty())
                        .collect();
                }

                if ls != before {
                    if ls == LanguageSettings::default() {
                        self.settings.languages.remove(lang.id());
                    } else {
                        self.settings.languages.insert(lang.id().to_string(), ls);
                    }
                    self.settings.apply_languages();
                    // Re-detect languages of open files with changed extensions.
                    for d in &mut self.docs {
                        let l = Language::from_path(&d.source.path_string());
                        if l != Language::PlainText && l != d.language {
                            d.language = l;
                            d.highlight_version = u64::MAX;
                        }
                    }
                }
            });
        });
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title.to_uppercase()).font(egui::FontId::new(11.0, theme::bold_family())).weak());
    ui.add_space(4.0);

}
