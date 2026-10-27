//! Settings page "Windows": installing Nebula for the current user, the
//! Explorer context menu and the file types Nebula opens.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui::{self, RichText, Sense, Ui, vec2};

use super::{NebulaApp, primary_button};
use crate::integration::{self, Handler};

/// The registered file types as Windows sees them; read again every few
/// seconds, because the default program is changed in Windows' settings.
pub(super) struct FileTypes {
    registered: BTreeSet<String>,
    handlers: HashMap<&'static str, Handler>,
    /// Checked types Windows opens with another program (and its name).
    conflicts: Vec<(&'static str, String)>,
    read: Instant,
    /// Registering runs in the background (it draws the icons).
    job: Option<Receiver<Result<(), String>>>,
}

impl FileTypes {
    fn read() -> Self {
        let registered = integration::registered_types();
        let handlers: HashMap<&'static str, Handler> =
            integration::FILE_TYPES.iter().map(|ext| (*ext, integration::handler_of(ext))).collect();
        let conflicts = integration::FILE_TYPES
            .iter()
            .filter(|ext| registered.contains(**ext))
            .filter_map(|ext| match &handlers[ext] {
                Handler::Nebula => None,
                Handler::NebulaLegacy => Some((*ext, t!("Nebula (altes Symbol)").to_string())),
                Handler::Other(name) => Some((*ext, name.clone())),
                Handler::Ask => Some((*ext, t!("keins – Windows fragt nach").to_string())),
            })
            .collect();
        Self { registered, handlers, conflicts, read: Instant::now(), job: None }
    }
}

impl NebulaApp {
    pub(super) fn system_page(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let faint = |t: &str| RichText::new(t.to_string()).small().color(theme.text_faint);

        ui.heading(t!("Windows-Integration"));
        ui.label(faint(t!("Alles gilt nur für dein Benutzerkonto, braucht keine Administratorrechte und kein zusätzliches Programm.")));
        ui.add_space(14.0);

        ui.label(RichText::new(t!("Installation")).strong());
        ui.add_space(4.0);
        let current = std::env::current_exe().ok();
        let installed = integration::installed_exe();
        let running_installed = match (&current, &installed) {
            (Some(c), Some(i)) => c.canonicalize().ok() == i.canonicalize().ok(),
            _ => false,
        };
        if integration::is_installed() {
            ui.label(t!("Installiert in {install_dir}", install_dir = integration::install_dir().display()));
            ui.label(faint(t!("Mit Startmenü-Eintrag; entfernen unter Windows-Einstellungen › Apps oder hier.")));
        } else {
            ui.label(t!("Nebula läuft ohne Installation (z. B. direkt aus dem Build-Ordner)."));
            ui.label(faint(&t!("„Installieren“ kopiert Nebula nach {install_dir}, legt einen Startmenü-Eintrag an und trägt es unter „Apps“ ein.", install_dir = integration::install_dir().display())));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = if integration::is_installed() && !running_installed { t!("Installation aktualisieren") } else { t!("Installieren") };
            if (!running_installed || !integration::is_installed()) && ui.add(primary_button(label, theme.accent)).clicked() {
                match integration::install() {
                    Ok(exe) => self.info(t!("Nebula wurde installiert: {exe}", exe = exe.display())),
                    Err(e) => self.error(t!("Installation fehlgeschlagen: {e}", e = format!("{:#}", e))),
                }
                self.file_types = None;
            }
            if integration::is_installed() && ui.button(t!("Deinstallieren…")).clicked() {
                self.confirm_uninstall = true;
            }
        });
        if self.confirm_uninstall {
            ui.add_space(6.0);
            ui.label(RichText::new(t!("Nebula aus dem Programmordner entfernen? Die Einstellungen bleiben erhalten.")).color(theme.modified));
            ui.horizontal(|ui| {
                if ui.button(t!("Ja, entfernen")).clicked() {
                    self.confirm_uninstall = false;
                    match integration::uninstall() {
                        Ok(()) if running_installed => self.info(t!("Nebula wurde entfernt; der Programmordner wird beim Beenden gelöscht.")),
                        Ok(()) => self.info(t!("Nebula wurde entfernt.")),
                        Err(e) => self.error(t!("Entfernen fehlgeschlagen: {e}", e = format!("{:#}", e))),
                    }
                    self.file_types = None;
                }
                if ui.button(t!("Abbrechen")).clicked() {
                    self.confirm_uninstall = false;
                }
            });
        }

        ui.add_space(18.0);
        ui.label(RichText::new(t!("Explorer")).strong());
        ui.add_space(4.0);
        let mut menu = integration::context_menu_enabled();
        if ui.checkbox(&mut menu, t!("„Mit Nebula öffnen“ im Kontextmenü von Dateien und Ordnern")).changed()
            && let Err(e) = integration::set_context_menu(menu)
        {
            self.error(t!("Kontextmenü konnte nicht geändert werden: {e}", e = format!("{:#}", e)));
        }
        ui.label(faint(t!("Unter Windows 11 steht der Eintrag unter „Weitere Optionen anzeigen“. Ordner öffnen sich als Projekt.")));

        ui.add_space(18.0);
        let any_types = self.file_types_section(ui);
        if !integration::is_installed() && (menu || any_types) {
            ui.add_space(8.0);
            ui.label(
                RichText::new(t!("Hinweis: Die Einträge zeigen auf dieses Programm hier. Nach „Installieren“ zeigen sie auf die installierte Kopie."))
                    .small()
                    .color(theme.modified),
            );
        }

        ui.add_space(18.0);
        ui.label(RichText::new(t!("Fenster")).strong());
        ui.add_space(4.0);
        ui.label(faint(t!("Läuft Nebula schon, öffnen sich Dateien aus dem Explorer im vorhandenen Fenster. Ein zweites Fenster erzwingt der Start mit --new-instance.")));
    }

    /// The list of file types with checkboxes; returns whether any is registered.
    fn file_types_section(&mut self, ui: &mut Ui) -> bool {
        let theme = self.theme.clone();
        let faint = |t: &str| RichText::new(t.to_string()).small().color(theme.text_faint);
        // Read again now and then (and right after registering).
        let finished = match self.file_types.as_ref().and_then(|f| f.job.as_ref()).map(|rx| rx.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => Some(Ok(())),
            _ => None,
        };
        if let Some(result) = finished {
            if let Err(e) = result {
                self.error(t!("Dateitypen konnten nicht geändert werden: {e}", e = e));
            }
            self.file_types = None;
        }
        let stale = self.file_types.as_ref().is_none_or(|f| f.job.is_none() && f.read.elapsed() > Duration::from_secs(3));
        if stale {
            self.file_types = Some(FileTypes::read());
        }
        ui.ctx().request_repaint_after(Duration::from_secs(3));
        let types = self.file_types.as_mut().expect("read above");
        let busy = types.job.is_some();

        ui.label(RichText::new(t!("Dateitypen")).strong());
        ui.add_space(4.0);
        ui.label(faint(t!("Angehakte Typen öffnet Nebula per Doppelklick und zeigt sie im Explorer mit demselben Symbol wie in der Dateiliste.")));
        ui.add_space(6.0);

        let mut wanted: Option<BTreeSet<String>> = None;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if ui.button(t!("Alle")).clicked() {
                    wanted = Some(integration::FILE_TYPES.iter().map(|e| e.to_string()).collect());
                }
                if ui.button(t!("Keine")).clicked() {
                    wanted = Some(BTreeSet::new());
                }
            });
            if ui.add_enabled(!types.registered.is_empty(), egui::Button::new(t!("Standard-Apps öffnen…"))).clicked() {
                crate::win32::open_url(integration::DEFAULT_APPS_PAGE);
            }
            if busy {
                ui.spinner();
                ui.label(faint(t!("Wird eingetragen …")));
            }
        });
        ui.add_space(6.0);

        let cell = 118.0;
        let columns = ((ui.available_width() / cell).floor() as usize).max(2);
        ui.add_enabled_ui(!busy, |ui| {
            egui::Grid::new("file-types").spacing(vec2(6.0, 2.0)).min_col_width(cell - 6.0).show(ui, |ui| {
                for (i, ext) in integration::FILE_TYPES.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let mut on = types.registered.contains(*ext);
                        if ui.checkbox(&mut on, "").changed() {
                            let mut set = types.registered.clone();
                            if on {
                                set.insert(ext.to_string());
                            } else {
                                set.remove(*ext);
                            }
                            wanted = Some(set);
                        }
                        let (r, _) = ui.allocate_exact_size(vec2(15.0, 18.0), Sense::hover());
                        crate::ui::icons::file(ui.painter(), r.center(), &format!("x.{ext}"));
                        ui.label(format!(".{ext}"));
                        if types.handlers.get(ext) == Some(&Handler::Nebula) {
                            ui.label(RichText::new("✓").small().color(theme.accent))
                                .on_hover_text(t!("Windows öffnet .{ext} mit Nebula.", ext = ext));
                        }
                    });
                    if (i + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
        });

        // Checked types Windows still opens with another program: only the
        // user may change that, on Nebula's page in Windows' settings.
        if !busy && !types.conflicts.is_empty() {
            ui.add_space(12.0);
            egui::Frame::new().fill(theme.hover).corner_radius(6.0).inner_margin(12.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(t!("⚠ Diese Typen öffnet Windows noch mit einem anderen Programm:")).color(theme.modified));
                ui.add_space(6.0);
                egui::Grid::new("file-type-conflicts").spacing(vec2(18.0, 4.0)).show(ui, |ui| {
                    for (ext, program) in &types.conflicts {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let (r, _) = ui.allocate_exact_size(vec2(15.0, 18.0), Sense::hover());
                            crate::ui::icons::file(ui.painter(), r.center(), &format!("x.{ext}"));
                            ui.label(format!(".{ext}"));
                        });
                        ui.label(RichText::new(program).color(theme.text_muted));
                        ui.end_row();
                    }
                });
                ui.add_space(10.0);
                if ui.add(primary_button(t!("In den Windows-Einstellungen umstellen…"), theme.accent)).clicked() {
                    crate::win32::open_url(integration::DEFAULT_APPS_PAGE);
                }
                ui.add_space(4.0);
                ui.label(faint(t!("Dort öffnet sich die Seite von Nebula unter Standard-Apps: beim jeweiligen Typ auf das Programm klicken und Nebula wählen. Programme dürfen diese Wahl aus Sicherheitsgründen nicht selbst ändern.")));
            });
        }

        let any = !types.registered.is_empty();
        // Registrations of an older version left other programs' defaults
        // in place: apply the current rules once per session (only from the
        // program the registrations point to).
        static REAPPLIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if wanted.is_none()
            && !busy
            && any
            && types.conflicts.iter().any(|(ext, _)| integration::has_no_user_choice(ext))
            && integration::runs_registered_program()
            && !REAPPLIED.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            wanted = Some(types.registered.clone());
        }
        if let Some(set) = wanted {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let _ = tx.send(integration::set_file_types(&set).map_err(|e| format!("{e:#}")));
                ctx.request_repaint();
            });
            types.job = Some(rx);
        }
        any
    }
}
