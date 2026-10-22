//! Settings page "Windows": installing Nebula for the current user, the
//! Explorer context menu and "Open with" for text file types.

use eframe::egui::{self, RichText, Ui};

use super::{NebulaApp, primary_button};
use crate::integration;

impl NebulaApp {
    pub(super) fn system_page(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let faint = |t: &str| RichText::new(t.to_string()).small().color(theme.text_faint);

        ui.heading("Windows-Integration");
        ui.label(faint("Alles gilt nur für dein Benutzerkonto, braucht keine Administratorrechte und kein zusätzliches Programm."));
        ui.add_space(14.0);

        ui.label(RichText::new("Installation").strong());
        ui.add_space(4.0);
        let current = std::env::current_exe().ok();
        let installed = integration::installed_exe();
        let running_installed = match (&current, &installed) {
            (Some(c), Some(i)) => c.canonicalize().ok() == i.canonicalize().ok(),
            _ => false,
        };
        if integration::is_installed() {
            ui.label(format!("Installiert in {}", integration::install_dir().display()));
            ui.label(faint("Mit Startmenü-Eintrag; entfernen unter Windows-Einstellungen › Apps oder hier."));
        } else {
            ui.label("Nebula läuft ohne Installation (z. B. direkt aus dem Build-Ordner).");
            ui.label(faint(&format!(
                "„Installieren“ kopiert Nebula nach {}, legt einen Startmenü-Eintrag an und trägt es unter „Apps“ ein.",
                integration::install_dir().display()
            )));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = if integration::is_installed() && !running_installed { "Installation aktualisieren" } else { "Installieren" };
            if (!running_installed || !integration::is_installed()) && ui.add(primary_button(label, theme.accent)).clicked() {
                match integration::install() {
                    Ok(exe) => self.info(format!("Nebula wurde installiert: {}", exe.display())),
                    Err(e) => self.error(format!("Installation fehlgeschlagen: {e:#}")),
                }
            }
            if integration::is_installed() && ui.button("Deinstallieren…").clicked() {
                self.confirm_uninstall = true;
            }
        });
        if self.confirm_uninstall {
            ui.add_space(6.0);
            ui.label(RichText::new("Nebula aus dem Programmordner entfernen? Die Einstellungen bleiben erhalten.").color(theme.modified));
            ui.horizontal(|ui| {
                if ui.button("Ja, entfernen").clicked() {
                    self.confirm_uninstall = false;
                    match integration::uninstall() {
                        Ok(()) if running_installed => self.info("Nebula wurde entfernt; der Programmordner wird beim Beenden gelöscht."),
                        Ok(()) => self.info("Nebula wurde entfernt."),
                        Err(e) => self.error(format!("Entfernen fehlgeschlagen: {e:#}")),
                    }
                }
                if ui.button("Abbrechen").clicked() {
                    self.confirm_uninstall = false;
                }
            });
        }

        ui.add_space(18.0);
        ui.label(RichText::new("Explorer").strong());
        ui.add_space(4.0);
        let mut menu = integration::context_menu_enabled();
        if ui.checkbox(&mut menu, "„Mit Nebula öffnen“ im Kontextmenü von Dateien und Ordnern").changed()
            && let Err(e) = integration::set_context_menu(menu)
        {
            self.error(format!("Kontextmenü konnte nicht geändert werden: {e:#}"));
        }
        ui.label(faint("Unter Windows 11 steht der Eintrag unter „Weitere Optionen anzeigen“. Ordner öffnen sich als Projekt."));
        ui.add_space(8.0);
        let mut types = integration::file_types_registered();
        if ui.checkbox(&mut types, "Nebula bei „Öffnen mit“ für Text- und Code-Dateien anbieten").changed()
            && let Err(e) = integration::set_file_types(types)
        {
            self.error(format!("Dateitypen konnten nicht geändert werden: {e:#}"));
        }
        ui.label(faint(&format!("{} Dateitypen, z. B. .html, .css, .js, .php, .json, .md.", integration::FILE_TYPES.len())));
        ui.label(faint("Als Standardprogramm festlegen kann man Nebula aus Sicherheitsgründen nur in den Windows-Einstellungen:"));
        if ui.add_enabled(types, egui::Button::new("Standard-Apps öffnen…")).clicked() {
            crate::win32::open_url("ms-settings:defaultapps");
        }
        if !integration::is_installed() && (menu || types) {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Hinweis: Die Einträge zeigen auf dieses Programm hier. Nach „Installieren“ zeigen sie auf die installierte Kopie.")
                    .small()
                    .color(theme.modified),
            );
        }

        ui.add_space(18.0);
        ui.label(RichText::new("Fenster").strong());
        ui.add_space(4.0);
        ui.label(faint("Läuft Nebula schon, öffnen sich Dateien aus dem Explorer im vorhandenen Fenster. Ein zweites Fenster erzwingt der Start mit --new-instance."));
    }
}
