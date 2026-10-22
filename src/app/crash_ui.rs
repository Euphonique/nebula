//! Offers the report of the last crash on start.

use eframe::egui::{self, RichText};

use super::{NebulaApp, modal, primary_button};
use crate::crash::Report;

impl NebulaApp {
    /// Returns whether the dialog stays open.
    pub(super) fn crash_dialog(&mut self, ctx: &egui::Context, report: &Report) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        let mut open_report = false;
        modal(ctx, egui::Id::unique("crash")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(t!("Nebula wurde unerwartet beendet"));
            ui.add_space(6.0);
            ui.label(t!("Beim letzten Mal ist ein Fehler aufgetreten. Ungespeicherte Änderungen wurden – soweit vorhanden – wiederhergestellt."));
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(theme.hover)
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&report.message).monospace());
                    if !report.location.is_empty() {
                        ui.label(RichText::new(&report.location).small().color(theme.gutter_fg));
                    }
                });
            ui.add_space(6.0);
            ui.label(
                RichText::new(t!("Der vollständige Bericht enthält keine Dateiinhalte und keine Zugangsdaten."))
                    .small()
                    .color(theme.gutter_fg),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(primary_button(t!("Bericht öffnen"), theme.accent)).clicked() {
                    open_report = true;
                }
                if ui.button(t!("Kopieren")).on_hover_text(t!("Bericht in die Zwischenablage")).clicked() {
                    ui.ctx().copy_text(report.text.clone());
                }
                if ui.button(t!("Im Explorer zeigen")).clicked() {
                    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", report.path.display())).spawn();
                }
                if ui.button(t!("Schließen")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        });
        if open_report {
            keep = false;
            self.open_local(&report.path);
        }
        keep
    }
}
