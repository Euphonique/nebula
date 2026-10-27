//! Looking for a newer Nebula on GitHub (at most once a day, in the background).

use crossbeam_channel::Receiver;
use eframe::egui;

use super::NebulaApp;
use crate::update::{self, Release};

/// Seconds between automatic checks.
const EVERY: i64 = 24 * 60 * 60;

#[derive(Default)]
pub(super) struct Updates {
    /// A newer version, shown in the status bar.
    pub available: Option<Release>,
    job: Option<(Receiver<Result<Option<Release>, String>>, bool)>,
    /// Result of the last check by hand, for the settings page.
    pub status: Option<String>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Updates {
    pub fn is_checking(&self) -> bool {
        self.job.is_some()
    }
}

impl NebulaApp {
    /// Starts a check; `manual` reports the result also when there is nothing new.
    pub(super) fn check_for_update(&mut self, ctx: &egui::Context, manual: bool) {
        if self.updates.job.is_some() {
            return;
        }
        self.settings.update_checked = now();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::latest());
            ctx.request_repaint();
        });
        self.updates.job = Some((rx, manual));
        if manual {
            self.updates.status = Some(t!("Suche …").into());
        }
    }

    pub(super) fn poll_updates(&mut self, ctx: &egui::Context) {
        if self.settings.check_updates
            && self.updates.job.is_none()
            && self.updates.available.is_none()
            && now() - self.settings.update_checked > EVERY
            && !crate::settings::is_secondary()
        {
            self.check_for_update(ctx, false);
        }
        let Some((rx, manual)) = &self.updates.job else { return };
        let Ok(result) = rx.try_recv() else { return };
        let manual = *manual;
        self.updates.job = None;
        let current = env!("CARGO_PKG_VERSION");
        match result {
            Ok(Some(r)) if update::is_newer(&r.version, current) && (manual || r.version != self.settings.update_skipped) => {
                let text = t!("Nebula {version} ist verfügbar (Statusleiste).", version = r.version);
                if manual {
                    self.updates.status = Some(text.clone());
                }
                self.info(text);
                self.updates.available = Some(r);
            }
            Ok(_) if manual => self.updates.status = Some(t!("Nebula {version} ist aktuell.", version = current)),
            Err(e) if manual => self.updates.status = Some(t!("Suche nach Updates fehlgeschlagen: {e}", e = e)),
            _ => {}
        }
    }

    /// The notice in the status bar.
    pub(super) fn update_badge(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.updates.available.clone() else { return };
        let label = egui::RichText::new(t!("⬆ Version {version} verfügbar", version = r.version)).size(11.5).color(self.theme.accent);
        let resp = ui.add(egui::Label::new(label).sense(egui::Sense::click())).on_hover_text(t!("Release-Seite öffnen"));
        if resp.clicked() {
            crate::win32::open_url(&r.url);
        }
        resp.context_menu(|ui| {
            if ui.button(t!("Diese Version überspringen")).clicked() {
                self.settings.update_skipped = r.version.clone();
                self.updates.available = None;
                ui.close();
            }
        });
    }
}
