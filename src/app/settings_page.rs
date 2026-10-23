//! The settings window: editor options, themes and languages.

use eframe::egui::{self, Color32, RichText, Ui};

use super::{NebulaApp, primary_button};
use crate::lang::Language;
use crate::settings::{Appearance, LanguageSettings};
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
    Agents,
    Extensions,
    /// Installing, Explorer context menu, file types.
    System,
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
    pub agents: super::agents_page::AgentsPage,
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
            ui.set_width(880.0);
            ui.set_height(560.0);
            ui.horizontal(|ui| {
                ui.heading(t!("Einstellungen"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary_button(t!("Fertig"), self.theme.accent)).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                // Navigation.
                ui.vertical(|ui| {
                    ui.set_width(150.0);
                    for (page, label) in [(Page::Editor, t!("Editor")), (Page::Design, t!("Design")), (Page::Languages, t!("Sprachen")), (Page::Keys, t!("Tastenkürzel")), (Page::Agents, t!("KI-Agenten")), (Page::Extensions, t!("Erweiterungen")), (Page::System, t!("Windows"))] {
                        let sel = state.page == page;
                        if ui
                            .add_sized([140.0, 30.0], egui::Button::selectable(sel, RichText::new(label).size(14.0)))
                            .clicked()
                        {
                            state.page = page;
                            state.font_picker_open = false;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt("settings-page").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(680.0);
                        match state.page {
                            Page::Editor => self.editor_page(ui, &mut state),
                            Page::Design => self.design_page(ui, &mut state),
                            Page::Languages => self.languages_page(ui, &mut state),
                            Page::Keys => self.keys_page(ui, &mut state.keys),
                            Page::Agents => self.agents_page(ui, &mut state.agents),
                            Page::System => self.system_page(ui),
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
                // Esc closes an open font list first.
                if state.font_picker_open {
                    state.font_picker_open = false;
                } else {
                    close = true;
                }
            }
        });
        self.settings_ui = state;
        if close {
            self.settings_ui.agents.stop();
            open = false;
            let _ = self.settings.save();
        }
        open
    }

    fn editor_page(&mut self, ui: &mut Ui, state: &mut SettingsUi) {
        self.language_section(ui);
        let e = &mut self.settings.editor;
        section(ui, t!("Darstellung"));
        egui::Grid::new("editor-grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label(t!("Schriftart"));
            let current = if e.font_family.is_empty() { t!("Standard (Cascadia Mono)").to_string() } else { e.font_family.clone() };
            let button = ui.add(
                egui::Button::new(format!("{current}  {}", if state.font_picker_open { "▴" } else { "▾" }))
                    .min_size(egui::vec2(280.0, 28.0)),
            );
            if button.clicked() {
                state.font_picker_open = !state.font_picker_open;
                state.font_filter.clear();
            }
            if state.font_picker_open {
                // Floats above the settings window like a dropdown. Tooltip order puts it
                // above the modal, so the search field gets keyboard input.
                let area = egui::Area::new(egui::Id::unique("font-picker"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(button.rect.left_bottom() + egui::vec2(0.0, 4.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_width(280.0);
                            let filter = ui.add(
                                egui::TextEdit::singleline(&mut state.font_filter)
                                    .hint_text(t!("Schrift suchen…"))
                                    .desired_width(f32::INFINITY),
                            );
                            if !filter.has_focus() && state.font_filter.is_empty() {
                                filter.request_focus();
                            }
                            let q = state.font_filter.to_lowercase();
                            let mut chosen = None;
                            egui::ScrollArea::vertical().id_salt("font-list").max_height(320.0).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                if q.is_empty() && ui.selectable_label(e.font_family.is_empty(), t!("Standard (Cascadia Mono)")).clicked() {
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
                    });
                // Clicking elsewhere closes it (Esc is handled by the window).
                let clicked_outside = ui.input(|i| i.pointer.any_click())
                    && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p) && !button.rect.contains(p));
                if clicked_outside {
                    state.font_picker_open = false;
                }
            }
            ui.end_row();
            ui.label(t!("Schriftgröße"));
            ui.add(egui::Slider::new(&mut e.font_size, 8.0..=32.0).step_by(0.5));
            ui.end_row();
            ui.label(t!("Zeilenhöhe"));
            ui.add(egui::Slider::new(&mut e.line_height, 1.0..=2.5).step_by(0.05).fixed_decimals(2));
            ui.end_row();
            ui.label("");
            ui.vertical(|ui| {
                ui.checkbox(&mut e.ligatures, t!("Ligaturen (z. B. => als Pfeil, != als ≠)"));
                // Only some fonts have them; name the installed ones.
                const LIGATURE_FONTS: [&str; 7] = ["Cascadia Code", "Fira Code", "JetBrains Mono", "Iosevka", "Victor Mono", "Hasklig", "Monaspace Neon"];
                let installed: Vec<&str> = LIGATURE_FONTS.iter().copied().filter(|f| crate::fonts::installed().contains_key(*f)).collect();
                if e.ligatures && !LIGATURE_FONTS.contains(&e.font_family.as_str()) {
                    let hint = if installed.is_empty() {
                        t!("Diese Schrift hat keine Ligaturen – z. B. Fira Code oder Cascadia Code installieren.").to_string()
                    } else {
                        t!("Nur Schriften mit Ligaturen zeigen sie, installiert: {fonts}.", fonts = installed.join(", "))
                    };
                    ui.label(RichText::new(hint).small().color(self.theme.text_faint));
                }
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.highlight_current_line, t!("Aktuelle Zeile hervorheben"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.show_indent_guides, t!("Einrückungshilfslinien"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.show_invisibles, t!("Unsichtbare Zeichen anzeigen"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.word_wrap, t!("Automatischer Zeilenumbruch"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.minimap, t!("Minimap am rechten Rand"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.git_blame, t!("Autor und Datum der letzten Änderung am Ende der aktuellen Zeile (Git)"));
            ui.end_row();
        });
        section(ui, t!("Einrückung (Standard)"));
        egui::Grid::new("indent-grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label(t!("Tabulatorbreite"));
            ui.add(egui::DragValue::new(&mut e.tab_size).range(1..=8));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut e.use_spaces, t!("Leerzeichen statt Tabs"));
            ui.end_row();
        });
        ui.label(RichText::new(t!("Pro Sprache lässt sich das unter „Sprachen“ abweichend einstellen.")).small().color(self.theme.text_faint));
        section(ui, t!("Beim Speichern"));
        ui.checkbox(&mut e.trim_trailing_whitespace, t!("Leerzeichen am Zeilenende entfernen (nicht in Markdown)"));
        ui.checkbox(&mut e.insert_final_newline, t!("Zeilenumbruch am Dateiende sicherstellen"));
        ui.label(RichText::new(t!("Eine .editorconfig im Projekt hat Vorrang vor diesen Einstellungen (Einrückung, Leerzeichen, Dateiende).")).small().color(self.theme.text_faint));
        section(ui, t!("Automatisch speichern"));
        {
            use crate::settings::AutoSave;
            let label = |a: AutoSave| match a {
                AutoSave::Off => t!("Aus"),
                AutoSave::Delay => t!("Nach einer Tipppause"),
                AutoSave::FocusChange => t!("Beim Wechsel von Tab oder Fenster"),
            };
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("autosave").selected_text(label(e.autosave)).width(240.0).show_ui(ui, |ui| {
                    for a in [AutoSave::Off, AutoSave::Delay, AutoSave::FocusChange] {
                        ui.selectable_value(&mut e.autosave, a, label(a));
                    }
                });
                if e.autosave == AutoSave::Delay {
                    ui.add(egui::DragValue::new(&mut e.autosave_delay).range(0.5..=60.0).speed(0.1).suffix(t!(" s")));
                }
            });
            ui.add_enabled(e.autosave != AutoSave::Off, egui::Checkbox::new(&mut e.autosave_remote, t!("Auch Serverdateien (jedes Speichern lädt hoch)")));
            ui.label(
                RichText::new(t!("Automatisches Speichern entfernt keine Leerzeichen am Zeilenende und lässt Formatierer aus; das passiert beim Speichern mit Strg+S."))
                    .small()
                    .color(self.theme.text_faint),
            );
        }
        section(ui, t!("Lokaler Verlauf"));
        ui.horizontal(|ui| {
            ui.checkbox(&mut e.local_history, t!("Frühere Fassungen von Dateien aufbewahren"));
            ui.add_enabled(e.local_history, egui::DragValue::new(&mut e.history_max).range(1..=500).suffix(t!(" je Datei")));
        });
        ui.label(
            RichText::new(t!("Beim Öffnen und bei jedem Speichern, auch für Serverdateien. Ansehen und wiederherstellen über das Tab-Menü „Lokaler Verlauf…“."))
                .small()
                .color(self.theme.text_faint),
        );
        section(ui, t!("Offline"));
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.settings.offline_cache, t!("Serverdateien und -ordner für die Offline-Ansicht zwischenspeichern"));
            if ui.small_button(t!("Leeren")).clicked() {
                let mb = crate::offline::total_size() as f64 / 1024.0 / 1024.0;
                match crate::offline::clear() {
                    Ok(()) => self.info(t!("Zwischenspeicher geleert ({mb} MB).", mb = format!("{mb:.1}"))),
                    Err(e) => self.error(t!("Zwischenspeicher konnte nicht geleert werden: {e}", e = e)),
                }
            }
        });
        ui.label(
            RichText::new(t!("Ist ein Server nicht erreichbar, zeigt die Seitenleiste den zuletzt gesehenen Stand, und Dateien öffnen sich schreibgeschützt aus der Kopie. Nach dem Wiederverbinden werden sie neu geladen."))
                .small()
                .color(self.theme.text_faint),
        );
        let e = &mut self.settings.editor;
        section(ui, t!("Rechtschreibprüfung"));
        ui.checkbox(&mut e.spell_check, t!("Kommentare, Markdown und Text prüfen (Windows-Rechtschreibprüfung)"));
        if e.spell_check {
            let (available, active) = crate::spell::languages();
            ui.horizontal_wrapped(|ui| {
                ui.label(t!("Sprachen:"));
                if ui.radio(e.spell_languages.is_empty(), t!("Automatisch")).clicked() {
                    e.spell_languages.clear();
                }
                for tag in &available {
                    let mut on = e.spell_languages.contains(tag);
                    if ui.checkbox(&mut on, tag.as_str()).changed() {
                        if e.spell_languages.is_empty() {
                            // Start from what "automatic" uses.
                            e.spell_languages = active.clone();
                        }
                        e.spell_languages.retain(|t| t != tag);
                        if on {
                            e.spell_languages.push(tag.clone());
                        }
                    }
                }
            });
            let note = if available.is_empty() {
                t!("Keine Wörterbücher gefunden – Sprachen lassen sich unter Windows › Einstellungen › Zeit und Sprache hinzufügen.").to_string()
            } else {
                t!("Aktiv: {list}. Ein Wort gilt als falsch, wenn es in keiner der Sprachen vorkommt. Weitere Wörterbücher kommen mit den Sprachen von Windows.", list = active.join(", "))
            };
            ui.label(RichText::new(note).small().color(self.theme.text_faint));
        }
        section(ui, t!("Eingabehilfen"));
        ui.checkbox(&mut e.auto_pair, t!("Klammern und Anführungszeichen automatisch schließen"));
        ui.checkbox(&mut e.auto_close_tags, t!("HTML-Tags automatisch schließen"));
        ui.checkbox(&mut e.emmet, t!("Emmet-Kürzel mit Tab expandieren (z. B. ul>li*3)"));
        section(ui, t!("Terminal"));
        let shells = crate::remote::terminal::local_shells();
        let current = shells
            .iter()
            .find(|s| s.name == self.settings.terminal_shell)
            .unwrap_or(&shells[0])
            .name
            .clone();
        ui.horizontal(|ui| {
            ui.label(t!("Lokale Shell"));
            egui::ComboBox::from_id_salt("terminal-shell").selected_text(&current).width(220.0).show_ui(ui, |ui| {
                for s in &shells {
                    if ui.selectable_label(s.name == current, &s.name).on_hover_text(&s.program).clicked() {
                        self.settings.terminal_shell = s.name.clone();
                    }
                }
            });
        });
        ui.label(RichText::new(t!("Strg+Ö öffnet bei lokalen Dateien diese Shell im Projektordner, bei Serverdateien ein SSH-Terminal.")).small().color(self.theme.text_faint));
    }

    pub(super) fn apply_theme(&mut self, ctx: &egui::Context) {
        self.theme = Theme::for_settings(&self.settings);
        self.theme.apply(ctx);
        self.backdrop_state = None;
    }

    pub(super) fn set_appearance(&mut self, ctx: &egui::Context, appearance: Appearance) {
        self.settings.appearance = appearance;
        if appearance == Appearance::System {
            crate::win32::refresh_system_dark();
        }
        self.apply_theme(ctx);
        let _ = self.settings.save();
    }

    /// In automatic mode, switches the theme when Windows changes between light and dark.
    pub(super) fn follow_system_theme(&mut self, ctx: &egui::Context) {
        if self.settings.appearance != Appearance::System {
            return;
        }
        let now = ctx.input(|i| i.time);
        let reported = ctx.system_theme();
        let (last, last_reported) = self.system_theme_check;
        if now - last < 2.0 && reported == last_reported {
            return;
        }
        self.system_theme_check = (now, reported);
        if crate::win32::refresh_system_dark() {
            self.apply_theme(ctx);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(3));
    }

    fn design_page(&mut self, ui: &mut Ui, state: &mut SettingsUi) {
        let ctx = ui.ctx().clone();
        // Extensions installed meanwhile bring their themes.
        if state.themes_gen != theme::themes_generation() {
            state.reload_themes();
        }
        section(ui, t!("Erscheinungsbild"));
        ui.horizontal(|ui| {
            for a in [Appearance::Light, Appearance::Dark, Appearance::System] {
                if ui.selectable_label(self.settings.appearance == a, a.label()).clicked() && self.settings.appearance != a {
                    self.set_appearance(&ctx, a);
                }
            }
        });
        ui.add_space(6.0);
        let dark = self.settings.is_dark();
        egui::Grid::new("theme-slots").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            for (slot_dark, label) in [(false, t!("Theme für helles Design")), (true, t!("Theme für dunkles Design"))] {
                ui.label(label);
                let slot = self.settings.theme_slot_mut(slot_dark);
                egui::ComboBox::from_id_salt(("theme-select", slot_dark))
                    .selected_text(slot.as_str())
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        // Themes of the matching kind first.
                        let themes = state.themes.iter().filter(|t| t.dark == slot_dark).chain(state.themes.iter().filter(|t| t.dark != slot_dark));
                        for t in themes {
                            let label = match &t.extension {
                                _ if t.is_builtin() => t!("{name} (mitgeliefert)", name = t.name),
                                Some(ext) => format!("{} ({ext})", t.name),
                                None => t.name.clone(),
                            };
                            if ui.selectable_label(t.name == *slot, label).clicked() {
                                *slot = t.name.clone();
                            }
                        }
                    });
                ui.end_row();
            }
        });
        let current = self.settings.active_theme().to_string();
        section(ui, &t!("Aktives Theme: {current}", current = current));
        ui.horizontal(|ui| {
            if ui.button(t!("Duplizieren")).on_hover_text(t!("Eigene, bearbeitbare Kopie anlegen")).clicked() {
                let mut name = t!("{current} Kopie", current = current);
                let mut n = 2;
                while state.themes.iter().any(|t| t.name == name) {
                    name = t!("{current} Kopie {n}", current = current, n = n);
                    n += 1;
                }
                let file = self.theme_without_mica().to_file(&name);
                match theme::save_theme(&file) {
                    Ok(()) => {
                        *self.settings.theme_slot_mut(dark) = name;
                        state.themes = theme::load_themes();
                    }
                    Err(e) => self.error(t!("Theme konnte nicht gespeichert werden: {e}", e = format!("{:#}", e))),
                }
            }
            if ui.button(t!("Importieren…")).on_hover_text(t!("Nebula-Theme (.json) oder Nova-Theme (.css)")).clicked() {
                self.import_theme_dialog(state);
            }
            if ui.button(t!("Exportieren…")).clicked() {
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
        if ui.checkbox(&mut mica, t!("Mica-Hintergrund in der Seitenleiste (Windows 11)")).changed() {
            self.settings.mica = mica;
            self.apply_theme(&ctx);
        }

        if builtin {
            ui.add_space(10.0);
            ui.label(
                RichText::new(t!("Mitgelieferte Themes sind schreibgeschützt. „Duplizieren“ legt eine bearbeitbare Kopie an."))
                    .color(self.theme.text_faint),
            );
        } else if let Some(ext) = &from_extension {
            ui.add_space(10.0);
            ui.label(
                RichText::new(t!("Stammt aus der Erweiterung „{ext}“. Änderungen werden als eigene Fassung gespeichert.", ext = ext))
                    .color(self.theme.text_faint),
            );
        } else if overrides_extension {
            ui.horizontal(|ui| {
                if ui.button(t!("Auf Original der Erweiterung zurücksetzen")).clicked() {
                    theme::delete_theme(&current);
                    state.themes = theme::load_themes();
                    self.apply_theme(&ctx);
                }
            });
        } else {
            ui.horizontal(|ui| {
                if ui.button(t!("Theme löschen")).clicked() {
                    theme::delete_theme(&current);
                    *self.settings.theme_slot_mut(dark) = if dark { BUILTIN_DARK.into() } else { theme::BUILTIN_LIGHT.into() };
                    state.themes = theme::load_themes();
                    self.apply_theme(&ctx);
                }
            });
        }

        // Color editor (read-only preview for built-in themes).
        let Some(idx) = state.themes.iter().position(|t| t.name == current) else { return };
        let mut changed = false;
        section(ui, t!("Oberfläche"));
        ui.add_enabled_ui(!builtin, |ui| {
            egui::Grid::new("ui-colors").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                for (n, (key, label)) in UI_COLORS.iter().enumerate() {
                    let file = &mut state.themes[idx];
                    let mut c = file.ui.get(*key).and_then(|h| color_from_hex(h)).unwrap_or(Color32::GRAY);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        file.ui.insert(key.to_string(), color_to_hex(c));
                        changed = true;
                    }
                    ui.label(crate::i18n::tr(label));
                    if n % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        });
        section(ui, t!("Syntax"));
        ui.add(egui::TextEdit::singleline(&mut state.syntax_filter).hint_text(t!("Kategorie filtern…")).desired_width(240.0));
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
                self.error(t!("Theme konnte nicht gespeichert werden: {e}", e = format!("{:#}", e)));
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
            .add_filter(t!("Themes"), &["json", "css"])
            .set_title(t!("Theme importieren"))
            .pick_file()
        else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.error(t!("Datei konnte nicht gelesen werden: {e}", e = e));
                return;
            }
        };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let parsed = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("css")) {
            crate::theme_import::import_nova_css(&text, &stem)
        } else {
            serde_json::from_str::<ThemeFile>(&text).map_err(|e| anyhow::anyhow!(t!("Ungültige Theme-Datei: {e}", e = e)))
        };
        match parsed {
            Ok(file) => {
                // Fill colors the source doesn't define from the base theme.
                let mut file = Theme::from_file(&file).to_file(&file.name);
                if file.is_builtin() {
                    file.name = t!("{name} (importiert)", name = file.name);
                }
                if let Err(e) = theme::save_theme(&file) {
                    self.error(format!("{e:#}"));
                    return;
                }
                self.info(t!("Theme „{name}“ importiert", name = file.name));
                // Use it for its kind of design and show it right away.
                *self.settings.theme_slot_mut(file.dark) = file.name.clone();
                if self.settings.is_dark() != file.dark {
                    self.settings.appearance = if file.dark { Appearance::Dark } else { Appearance::Light };
                }
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
            Ok(()) => self.info(t!("Theme exportiert nach {path}", path = path.display())),
            Err(e) => self.error(t!("Export fehlgeschlagen: {e}", e = format!("{:#}", e))),
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
                section(ui, t!("Dateiendungen"));
                ui.label(RichText::new(t!("Eingebaut: {default_extensions}", default_extensions = lang.default_extensions().join(", "))).color(self.theme.text_faint));
                ui.horizontal(|ui| {
                    ui.label(t!("Zusätzlich:"));
                    if ui
                        .add(egui::TextEdit::singleline(&mut state.ext_text).hint_text(t!("z. B. tpl, inc")).desired_width(260.0))
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

                section(ui, t!("Einrückung"));
                let mut custom = ls.tab_size.is_some() || ls.use_spaces.is_some();
                if ui.checkbox(&mut custom, t!("Eigene Einrückung für diese Sprache")).changed() {
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
                        ui.label(t!("Breite"));
                        if ui.add(egui::DragValue::new(&mut t).range(1..=8)).changed() {
                            ls.tab_size = Some(t);
                        }
                        let mut s = ls.use_spaces.unwrap_or(true);
                        if ui.checkbox(&mut s, t!("Leerzeichen")).changed() {
                            ls.use_spaces = Some(s);
                        }
                    });
                }

                section(ui, t!("Eigene Vervollständigungen"));
                ui.label(
                    RichText::new(t!(
                        "Eine Zeile pro Eintrag: ein Wort, oder „Kürzel => Text“. $0 setzt den Cursor, \\n bricht die Zeile um.\nBeispiel:  card => <div class=\"card\">\\n    $0\\n</div>\nMehrzeilige Snippets mit Platzhaltern legst du bequemer im rechten Panel unter „{ }“ an."
                    ))
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

impl NebulaApp {
    /// Language of the user interface, with export and import of language files.
    fn language_section(&mut self, ui: &mut Ui) {
        section(ui, t!("Sprache"));
        let languages = crate::i18n::available();
        let name_of = |code: &str| languages.iter().find(|(c, _)| c == code).map_or(code.to_string(), |(_, n)| n.clone());
        let auto = t!("Automatisch ({name})", name = name_of(&crate::i18n::resolve("")));
        let selected = if self.settings.ui_language.is_empty() { auto.clone() } else { name_of(&self.settings.ui_language) };
        let mut choice = None;
        ui.horizontal(|ui| {
            ui.label(t!("Oberfläche"));
            egui::ComboBox::from_id_salt("ui-language").selected_text(selected).width(240.0).height(480.0).show_ui(ui, |ui| {
                if ui.selectable_label(self.settings.ui_language.is_empty(), &auto).on_hover_text(t!("Folgt der Windows-Anzeigesprache; ohne Übersetzung Englisch")).clicked() {
                    choice = Some(String::new());
                }
                for (code, name) in &languages {
                    if ui.selectable_label(&self.settings.ui_language == code, name).clicked() {
                        choice = Some(code.clone());
                    }
                }
            });
        });
        if let Some(code) = choice {
            self.settings.ui_language = code;
            crate::i18n::set_language(&self.settings.ui_language);
            let _ = self.settings.save();
        }
        ui.horizontal(|ui| {
            if ui.button(t!("Sprachdatei exportieren…")).on_hover_text(t!("Alle Texte als Vorlage für eine Übersetzung")).clicked() {
                self.export_language_file();
            }
            if ui.button(t!("Sprachdatei importieren…")).clicked() {
                self.import_language_file();
            }
        });
        ui.label(
            RichText::new(t!("Eigene Sprachdateien liegen in {dir}.", dir = crate::i18n::user_dir().display()))
                .small()
                .color(self.theme.text_faint),
        );
    }

    fn export_language_file(&mut self) {
        let code = crate::i18n::current();
        // German is the source: export an English-based template then.
        let code = if code == crate::i18n::SOURCE { crate::i18n::FALLBACK.to_string() } else { code };
        let Some(path) = rfd::FileDialog::new()
            .set_title(t!("Sprachdatei exportieren"))
            .set_file_name(format!("nebula-{code}.json"))
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        match crate::i18n::export(&code, &crate::i18n::all_keys()).and_then(|b| Ok(std::fs::write(&path, b)?)) {
            Ok(()) => self.info(t!("Sprachdatei gespeichert: {path}", path = path.display())),
            Err(e) => self.error(t!("Export fehlgeschlagen: {e}", e = format!("{e:#}"))),
        }
    }

    fn import_language_file(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title(t!("Sprachdatei importieren")).add_filter("JSON", &["json"]).pick_file() else {
            return;
        };
        match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| crate::i18n::import(&b)) {
            Ok((code, name)) => {
                self.settings.ui_language = code;
                crate::i18n::set_language(&self.settings.ui_language);
                let _ = self.settings.save();
                self.info(t!("Sprache „{name}“ importiert und eingestellt.", name = name));
            }
            Err(e) => self.error(t!("Import fehlgeschlagen: {e}", e = format!("{e:#}"))),
        }
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title.to_uppercase()).font(egui::FontId::new(11.0, theme::bold_family())).weak());
    ui.add_space(4.0);

}
