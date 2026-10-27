//! "Über Nebula": logo, name, version, the GitHub page and the update check.

use eframe::egui::{self, RichText, Sense, pos2, vec2};

use super::{NebulaApp, modal, primary_button};
use crate::update::REPO;

impl NebulaApp {
    /// Returns whether the dialog stays open.
    pub(super) fn about_dialog(&mut self, ctx: &egui::Context) -> bool {
        let theme = self.theme.clone();
        let icon = self.app_icon(ctx);
        let mut keep = true;
        let mut check = false;
        modal(ctx, egui::Id::unique("about")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                let (r, _) = ui.allocate_exact_size(vec2(112.0, 112.0), Sense::hover());
                ui.painter().image(icon.id(), r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
                ui.add_space(6.0);
                let (r, _) = ui.allocate_exact_size(vec2(200.0, 30.0), Sense::hover());
                crate::ui::wordmark::paint(ui.painter(), &mut self.wordmark_tex, r.center(), r.left(), 28.0, theme.foreground);
                ui.add_space(6.0);
                ui.label(RichText::new(t!("Version {value}", value = env!("CARGO_PKG_VERSION"))).color(theme.text_muted));
                ui.add_space(8.0);
                ui.label(t!("Schneller Code-Editor für Windows mit FTP/SFTP"));
                ui.add_space(4.0);
                let url = format!("https://github.com/{REPO}");
                if ui.link(format!("github.com/{REPO}")).on_hover_text(&url).clicked() {
                    crate::win32::open_url(&url);
                }
                ui.label(RichText::new(t!("MIT-Lizenz")).small().color(theme.text_faint));
            });

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(10.0);
            ui.label(RichText::new(t!("Updates")).strong());
            ui.add_space(4.0);
            ui.checkbox(&mut self.settings.check_updates, t!("Einmal am Tag nach einer neuen Version suchen"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let searching = self.updates.is_checking();
                if ui.add_enabled(!searching, primary_button(t!("Nach Updates suchen"), theme.accent)).clicked() {
                    check = true;
                }
                if searching {
                    ui.spinner();
                }
                if let Some(r) = self.updates.available.clone()
                    && ui.button(t!("Version {version} herunterladen…", version = r.version)).on_hover_text(&r.url).clicked()
                {
                    crate::win32::open_url(&r.url);
                }
            });
            if let Some(status) = &self.updates.status {
                ui.add_space(4.0);
                ui.label(RichText::new(status).color(theme.foreground));
            }
            ui.label(
                RichText::new(t!("Gesucht wird in den Releases auf GitHub; eine neue Version erscheint auch als Hinweis in der Statusleiste."))
                    .small()
                    .color(theme.text_faint),
            );

            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("Schließen")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        });
        if check {
            self.check_for_update(ctx, true);
        }
        keep
    }
}

