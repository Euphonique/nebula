//! Managing the server list: groups, duplicating, import and export.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use super::{Dialog, NebulaApp, SiteForm, modal, primary_button};
use crate::server_import::{self, Imported};
use crate::settings::{Site, new_id};

pub(super) struct ImportForm {
    /// Where the servers come from, for the heading.
    source: String,
    entries: Vec<ImportEntry>,
    with_passwords: bool,
}

struct ImportEntry {
    item: Imported,
    selected: bool,
    duplicate: bool,
}

impl NebulaApp {
    fn save_sites(&mut self) {
        if let Err(e) = self.sites.save() {
            self.error(t!("Server konnten nicht gespeichert werden: {e}", e = format!("{:#}", e)));
        }
    }

    /// New server form, preset to a group.
    pub(super) fn add_site_to_group(&mut self, group: String) {
        self.open_site_form(None);
        if let Some(Dialog::EditSite(form)) = &mut self.dialog {
            form.site.group = group;
        }
    }

    /// Opens the form with a copy; nothing is saved until the user confirms.
    pub(super) fn duplicate_site(&mut self, id: &str) {
        let Some(original) = self.sites.get(id).cloned() else { return };
        let password = original.password().unwrap_or_default();
        let site = Site { id: new_id(), name: t!("{display_name} (Kopie)", display_name = original.display_name()), ..original };
        self.dialog = Some(Dialog::EditSite(Box::new(SiteForm { site, password, password_loaded: true, is_new: true })));
    }

    pub(super) fn set_site_group(&mut self, id: &str, group: &str) {
        if let Some(site) = self.sites.sites.iter_mut().find(|s| s.id == id) {
            site.group = group.trim().to_string();
        }
        // A new group starts unfolded.
        self.sites.collapsed_groups.retain(|g| g != group);
        self.save_sites();
    }

    pub(super) fn rename_group(&mut self, old: &str, new: &str) {
        for site in self.sites.sites.iter_mut().filter(|s| s.group == old) {
            site.group = new.to_string();
        }
        for g in self.sites.collapsed_groups.iter_mut().filter(|g| *g == old) {
            *g = new.to_string();
        }
        self.save_sites();
    }

    pub(super) fn dissolve_group(&mut self, group: &str) {
        self.rename_group(group, "");
        self.sites.collapsed_groups.retain(|g| !g.is_empty());
        self.save_sites();
    }

    pub(super) fn toggle_group(&mut self, group: String) {
        if let Some(i) = self.sites.collapsed_groups.iter().position(|g| *g == group) {
            self.sites.collapsed_groups.remove(i);
        } else {
            self.sites.collapsed_groups.push(group);
        }
        self.save_sites();
    }

    /// Reads a file (asks for one when `path` is `None`) and shows what would be imported.
    pub(super) fn import_sites(&mut self, path: Option<PathBuf>) {
        let path = match path {
            Some(p) => p,
            None => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title(t!("Server importieren"))
                    .add_filter(t!("Serverliste"), &["json", "xml"])
                    .add_filter(t!("Alle Dateien"), &["*"]);
                if let Some(dir) = server_import::filezilla_path().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.pick_file() {
                    Some(p) => p,
                    None => return,
                }
            }
        };
        let list = std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|b| server_import::parse(&b));
        let list = match list {
            Ok(l) => l,
            Err(e) => {
                self.error(t!("Import fehlgeschlagen: {e}", e = format!("{:#}", e)));
                return;
            }
        };
        if list.is_empty() {
            self.info(t!("In der Datei wurden keine Server gefunden."));
            return;
        }
        let source = if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("sitemanager.xml")) {
            "FileZilla".to_string()
        } else {
            path.file_name().unwrap_or_default().to_string_lossy().into_owned()
        };
        let entries = list
            .into_iter()
            .map(|item| {
                let duplicate = server_import::is_duplicate(&self.sites.sites, &item.site);
                ImportEntry { item, selected: !duplicate, duplicate }
            })
            .collect();
        self.dialog = Some(Dialog::ImportSites(Box::new(ImportForm { source, entries, with_passwords: true })));
    }

    pub(super) fn export_sites(&mut self) {
        if self.sites.sites.is_empty() {
            self.info(t!("Es gibt noch keine Server zum Exportieren."));
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title(t!("Server exportieren"))
            .set_file_name("nebula-server.json")
            .add_filter(t!("Nebula-Serverliste"), &["json"])
            .save_file()
        else {
            return;
        };
        match server_import::export(&self.sites).and_then(|b| Ok(std::fs::write(&path, b)?)) {
            Ok(()) => self.info(t!("{sites_count} Server exportiert (ohne Passwörter).", sites_count = self.sites.sites.len())),
            Err(e) => self.error(t!("Export fehlgeschlagen: {e}", e = format!("{:#}", e))),
        }
    }

    /// Returns whether the dialog stays open.
    pub(super) fn import_dialog(&mut self, ctx: &egui::Context, form: &mut ImportForm) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        let mut import = false;
        let has_passwords = form.entries.iter().any(|e| e.item.password.is_some());
        modal(ctx, egui::Id::unique("import-sites")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(t!("Server importieren – {source}", source = form.source));
            ui.add_space(6.0);
            let dupes = form.entries.iter().filter(|e| e.duplicate).count();
            if dupes > 0 {
                let text = if dupes == 1 {
                    t!("Einen der Server gibt es schon, er ist abgewählt.").to_string()
                } else {
                    t!("{dupes} der Server gibt es schon, sie sind abgewählt.", dupes = dupes)
                };
                ui.label(RichText::new(text).small().color(theme.gutter_fg));
            }
            ui.horizontal(|ui| {
                if ui.small_button(t!("Alle")).clicked() {
                    form.entries.iter_mut().for_each(|e| e.selected = true);
                }
                if ui.small_button(t!("Keine")).clicked() {
                    form.entries.iter_mut().for_each(|e| e.selected = false);
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut last_group = None;
                for e in &mut form.entries {
                    let site = &e.item.site;
                    if last_group != Some(&site.group) && !site.group.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(site.group.to_uppercase()).small().strong().color(theme.text_faint));
                    }
                    last_group = Some(&site.group);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut e.selected, RichText::new(site.display_name()).strong());
                        let detail = format!("{} · {}@{}:{}", site.protocol.label().split(' ').next().unwrap_or(""), site.user, site.host, site.port);
                        ui.label(RichText::new(detail).small().color(theme.gutter_fg));
                        if e.duplicate {
                            ui.label(RichText::new("vorhanden").small().color(theme.modified));
                        }
                    });
                    if let Some(note) = &e.item.note {
                        ui.horizontal(|ui| {
                            ui.add_space(26.0);
                            ui.add(egui::Label::new(RichText::new(note).small().color(theme.modified)).wrap());
                        });
                    }
                }
            });
            ui.add_space(8.0);
            if has_passwords {
                ui.checkbox(&mut form.with_passwords, t!("Gespeicherte Passwörter übernehmen"))
                    .on_hover_text(t!("Sie landen in der Windows-Anmeldeinformationsverwaltung"));
            }
            ui.add_space(8.0);
            let count = form.entries.iter().filter(|e| e.selected).count();
            ui.horizontal(|ui| {
                let label = if count == 1 { t!("1 Server importieren").to_string() } else { t!("{count} Server importieren", count = count) };
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
            for e in form.entries.iter().filter(|e| e.selected) {
                let site = Site { id: new_id(), ..e.item.site.clone() };
                if form.with_passwords
                    && let Some(p) = &e.item.password
                    && let Err(err) = site.set_password(p)
                {
                    self.error(format!("{}: {err:#}", site.display_name()));
                }
                self.sites.sites.push(site);
                count += 1;
            }
            self.save_sites();
            self.show_sidebar = true;
            self.sidebar.tab = crate::ui::sidebar::SidebarTab::Servers;
            self.info(if count == 1 { t!("1 Server importiert.").to_string() } else { t!("{count} Server importiert.", count = count) });
            keep = false;
        }
        keep
    }
}
