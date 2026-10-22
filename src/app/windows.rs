//! Several windows: each one is a Nebula process of its own with its own
//! project. Settings and servers are shared through their files (merged on
//! saving, read again when another window changed them).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

use super::NebulaApp;

/// How often a window looks for changes made by the others.
const CHECK_EVERY: Duration = Duration::from_secs(1);

impl NebulaApp {
    /// Opens a further Nebula window, empty or with `folder`.
    pub(super) fn open_new_window(&mut self, folder: Option<PathBuf>) {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return self.error(t!("Neues Fenster konnte nicht geöffnet werden: {e}", e = e)),
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--window");
        if let Some(f) = folder {
            cmd.arg(f);
        }
        if let Err(e) = cmd.spawn() {
            self.error(t!("Neues Fenster konnte nicht geöffnet werden: {e}", e = e));
        }
    }

    /// "Ordner in neuem Fenster öffnen…"
    pub(super) fn open_folder_in_new_window(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().set_title(t!("Ordner in neuem Fenster öffnen")).pick_folder() {
            self.open_new_window(Some(dir));
        }
    }

    /// Takes over settings and servers other windows saved; called every frame.
    pub(super) fn poll_shared_files(&mut self, ctx: &egui::Context) {
        if self.shared_check.elapsed() < CHECK_EVERY {
            return;
        }
        self.shared_check = Instant::now();
        let language = self.settings.ui_language.clone();
        if self.settings.reload_if_changed() {
            if self.settings.ui_language != language {
                crate::i18n::set_language(&self.settings.ui_language);
            }
            self.settings.apply_languages();
            self.apply_theme(ctx);
            self.sidebar.recent_folders = self.settings.recent_folders.clone();
        }
        self.sites.reload_if_changed();
        ctx.request_repaint_after(CHECK_EVERY);
    }
}
