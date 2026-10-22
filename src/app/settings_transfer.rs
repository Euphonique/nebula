//! Exporting all settings into one file and importing them on another
//! computer (sections chosen in a dialog).

use eframe::egui::{self, RichText};

use super::{Dialog, NebulaApp, modal, primary_button};
use crate::ext::library;
use crate::ext::manifest::Extension;
use crate::settings::{Site, new_id};
use crate::settings_io::{Bundle, ExtensionRef, Section, Sources};

pub(super) struct ExportForm {
    sections: Vec<(Section, bool)>,
}

pub(super) struct ImportForm {
    source: String,
    bundle: Bundle,
    sections: Vec<(Section, bool)>,
}

/// Extensions being installed from the library after an import.
pub(super) type Reinstall = crossbeam_channel::Receiver<(String, anyhow::Result<Extension>)>;

impl NebulaApp {
    pub(super) fn open_settings_export(&mut self) {
        self.dialog = Some(Dialog::SettingsExport(Box::new(ExportForm { sections: Section::ALL.iter().map(|&s| (s, true)).collect() })));
    }

    fn export_sources(&self) -> (Vec<crate::theme::ThemeFile>, Vec<ExtensionRef>, serde_json::Value) {
        let themes = crate::theme::load_themes().into_iter().filter(|t| !t.is_builtin() && t.extension.is_none()).collect();
        let extensions = self
            .ext
            .installed
            .iter()
            .map(|e| ExtensionRef { identifier: e.identifier.clone(), name: e.name.clone() })
            .collect();
        let config = serde_json::Value::Object(self.ext.host.config.lock().unwrap().global_values().clone());
        (themes, extensions, config)
    }

    /// Returns whether the dialog stays open.
    pub(super) fn settings_export_dialog(&mut self, ctx: &egui::Context, form: &mut ExportForm) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        let mut export = false;
        modal(ctx, egui::Id::unique("settings-export")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Einstellungen exportieren");
            ui.add_space(4.0);
            ui.label(RichText::new("Für einen anderen Rechner oder als Sicherung. Passwörter und API-Schlüssel kommen nicht mit.").small().color(theme.gutter_fg));
            ui.add_space(8.0);
            for (section, on) in &mut form.sections {
                ui.checkbox(on, section.label());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let any = form.sections.iter().any(|s| s.1);
                if ui.add_enabled(any, primary_button("Exportieren…", theme.accent)).clicked() {
                    export = true;
                }
                if ui.button("Abbrechen").clicked() {
                    keep = false;
                }
            });
        });
        if export {
            keep = false;
            let chosen: Vec<Section> = form.sections.iter().filter(|s| s.1).map(|s| s.0).collect();
            let Some(path) = rfd::FileDialog::new()
                .set_title("Einstellungen exportieren")
                .set_file_name("nebula-einstellungen.json")
                .add_filter("Nebula-Einstellungen", &["json"])
                .save_file()
            else {
                return false;
            };
            let (user_themes, extensions, extension_config) = self.export_sources();
            let src = Sources { settings: &self.settings, sites: &self.sites.sites, user_themes, extensions, extension_config };
            match Bundle::build(&src, &chosen).to_json().and_then(|b| Ok(std::fs::write(&path, b)?)) {
                Ok(()) => self.info(format!("Einstellungen exportiert: {}", path.display())),
                Err(e) => self.error(format!("Export fehlgeschlagen: {e:#}")),
            }
        }
        keep
    }

    pub(super) fn open_settings_import(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Einstellungen importieren")
            .add_filter("Nebula-Einstellungen", &["json"])
            .pick_file()
        else {
            return;
        };
        let bundle = match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| Bundle::parse(&b)) {
            Ok(b) => b,
            Err(e) => {
                self.error(format!("Import fehlgeschlagen: {e:#}"));
                return;
            }
        };
        let sections: Vec<(Section, bool)> = Section::ALL.iter().filter(|s| bundle.has(**s)).map(|&s| (s, true)).collect();
        if sections.is_empty() {
            self.info("Die Datei enthält keine Einstellungen.");
            return;
        }
        let source = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        self.dialog = Some(Dialog::SettingsImport(Box::new(ImportForm { source, bundle, sections })));
    }

    /// Returns whether the dialog stays open.
    pub(super) fn settings_import_dialog(&mut self, ctx: &egui::Context, form: &mut ImportForm) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        let mut import = false;
        modal(ctx, egui::Id::unique("settings-import")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading(format!("Einstellungen importieren – {}", form.source));
            if !form.bundle.version.is_empty() {
                ui.label(RichText::new(format!("Exportiert mit Nebula {}", form.bundle.version)).small().color(theme.gutter_fg));
            }
            ui.add_space(8.0);
            for (section, on) in &mut form.sections {
                ui.checkbox(on, section.label());
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    ui.label(RichText::new(form.bundle.summary(*section)).small().color(theme.gutter_fg));
                });
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Editor, Darstellung und Tastenkürzel werden ersetzt. Server, Snippets, Themes und Agenten kommen dazu (vorhandene bleiben). \
                     Fehlende Erweiterungen werden aus der Bibliothek installiert.",
                )
                .small()
                .color(theme.gutter_fg),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let any = form.sections.iter().any(|s| s.1);
                if ui.add_enabled(any, primary_button("Importieren", theme.accent)).clicked() {
                    import = true;
                }
                if ui.button("Abbrechen").clicked() {
                    keep = false;
                }
            });
        });
        if import {
            let chosen: Vec<Section> = form.sections.iter().filter(|s| s.1).map(|s| s.0).collect();
            let bundle = std::mem::take(&mut form.bundle);
            self.apply_bundle(ctx, bundle, &chosen);
            keep = false;
        }
        keep
    }

    fn apply_bundle(&mut self, ctx: &egui::Context, b: Bundle, sections: &[Section]) {
        let mut notes = Vec::new();
        for section in sections {
            match section {
                Section::Editor => {
                    if let Some(e) = b.editor.clone() {
                        self.settings.editor = e.editor;
                        self.settings.appearance = e.appearance;
                        self.settings.theme_light = e.theme_light;
                        self.settings.theme_dark = e.theme_dark;
                        self.settings.mica = e.mica;
                        self.settings.preview_device = e.preview_device;
                        self.settings.terminal_shell = e.terminal_shell;
                    }
                }
                Section::Themes => {
                    let mut failed = 0;
                    for t in b.themes.iter().flatten() {
                        if crate::theme::save_theme(t).is_err() {
                            failed += 1;
                        }
                    }
                    crate::theme::themes_changed();
                    if failed > 0 {
                        notes.push(format!("{failed} Themes konnten nicht gespeichert werden."));
                    }
                }
                Section::Languages => {
                    if let Some(l) = &b.languages {
                        for (id, ls) in &l.languages {
                            self.settings.languages.insert(id.clone(), ls.clone());
                        }
                        crate::settings_io::merge_snippets(&mut self.settings.snippets, &l.snippets);
                    }
                }
                Section::Keys => {
                    if let Some(k) = &b.keybindings {
                        self.settings.keybindings = k.clone();
                    }
                }
                Section::Servers => {
                    let mut added = 0;
                    // Only servers that were here before count as duplicates
                    // (the file may hold deliberate copies with the same login).
                    let before = self.sites.sites.clone();
                    for site in b.servers.iter().flatten() {
                        if !crate::server_import::is_duplicate(&before, site) {
                            self.sites.sites.push(Site { id: new_id(), ..site.clone() });
                            added += 1;
                        }
                    }
                    if let Err(e) = self.sites.save() {
                        self.error(format!("Server konnten nicht gespeichert werden: {e:#}"));
                    }
                    if added > 0 {
                        notes.push(format!("{added} Server übernommen – Passwörter bitte neu eingeben."));
                    }
                }
                Section::Agents => {
                    if let Some(a) = &b.agents {
                        crate::settings_io::merge_agents(&mut self.settings.agents, &a.agents);
                        if !a.agent.is_empty() {
                            self.settings.agent = a.agent.clone();
                        }
                    }
                }
                Section::Extensions => {
                    if let Some(x) = &b.extensions {
                        if let serde_json::Value::Object(map) = &x.config {
                            let mut store = self.ext.host.config.lock().unwrap();
                            for (k, v) in map {
                                store.set(false, k, v.clone());
                            }
                        }
                        for d in &x.disabled {
                            if !self.settings.disabled_extensions.contains(d) {
                                self.settings.disabled_extensions.push(d.clone());
                            }
                        }
                        let missing: Vec<ExtensionRef> =
                            x.installed.iter().filter(|e| !self.ext.installed.iter().any(|i| i.identifier == e.identifier)).cloned().collect();
                        if !missing.is_empty() {
                            notes.push(match missing.len() {
                                1 => "1 Erweiterung wird aus der Bibliothek installiert…".to_string(),
                                n => format!("{n} Erweiterungen werden aus der Bibliothek installiert…"),
                            });
                            self.reinstall_extensions(ctx, missing);
                        }
                    }
                }
            }
        }
        self.settings.apply_languages();
        self.apply_theme(ctx);
        if let Err(e) = self.settings.save() {
            self.error(format!("Einstellungen konnten nicht gespeichert werden: {e:#}"));
        }
        let mut text = "Einstellungen importiert.".to_string();
        for n in notes {
            text.push(' ');
            text.push_str(&n);
        }
        self.info(text);
    }

    /// Installs extensions from the library in the background, one after another.
    fn reinstall_extensions(&mut self, ctx: &egui::Context, list: Vec<ExtensionRef>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for e in list {
                let result = library::details(&e.identifier).and_then(|entry| library::install(&entry));
                let _ = tx.send((e.name.clone(), result));
                ctx.request_repaint();
            }
        });
        self.reinstall = Some(rx);
    }

    /// Takes the results of `reinstall_extensions`.
    pub(super) fn poll_reinstall(&mut self) {
        let Some(rx) = &self.reinstall else { return };
        let mut results = Vec::new();
        // The thread has finished once its sender is gone.
        let mut done = false;
        loop {
            match rx.try_recv() {
                Ok(r) => results.push(r),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    done = true;
                    break;
                }
            }
        }
        if !results.is_empty() {
            let mut ok = Vec::new();
            for (name, r) in results {
                match r {
                    Ok(_) => ok.push(name),
                    Err(e) => self.error(format!("Erweiterung „{name}“ nicht installiert: {e:#} (nicht in der Bibliothek? Dann bitte von Hand installieren.)")),
                }
            }
            if !ok.is_empty() {
                self.ext.reload(&self.settings.disabled_extensions.clone());
                crate::theme::themes_changed();
                self.info(format!("Installiert: {}", ok.join(", ")));
            }
        }
        if done {
            self.reinstall = None;
        }
    }
}
