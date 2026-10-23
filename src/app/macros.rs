//! Recording and playing back keyboard macros (see `editor::macros`).

use eframe::egui;

use super::NebulaApp;
use super::palette::RECORDABLE;
use crate::editor::macros::{self, Step};

/// Most repetitions in one go.
const MAX_TIMES: usize = 10_000;

impl NebulaApp {
    pub(super) fn toggle_macro_recording(&mut self) {
        if let Some(steps) = macros::stop() {
            if steps.is_empty() {
                self.info(t!("Makro-Aufnahme beendet – nichts aufgenommen."));
            } else {
                self.last_macro = steps;
                self.info(t!("Makro aufgenommen – abspielen mit Strg+Umschalt+M."));
            }
        } else {
            self.macro_queue.clear();
            macros::start();
        }
        self.focus_editor = true;
    }

    /// Plays the last macro `times` times on the active document.
    pub(super) fn play_macro(&mut self, times: usize) {
        if macros::recording() {
            self.info(t!("Erst die Aufnahme beenden (Strg+Umschalt+R)."));
            return;
        }
        if self.last_macro.is_empty() {
            self.info(t!("Noch kein Makro aufgenommen (Strg+Umschalt+R)."));
            return;
        }
        if self.active.is_none() {
            return;
        }
        for _ in 0..times.clamp(1, MAX_TIMES) {
            self.macro_queue.extend(self.last_macro.iter().cloned());
        }
        self.focus_editor = true;
    }

    /// Feeds the playback to the active editor: input in batches the editor
    /// handles next frame, commands in between.
    pub(super) fn drive_macro(&mut self, ctx: &egui::Context) {
        if self.macro_queue.is_empty() {
            return;
        }
        let Some(i) = self.active else {
            self.macro_queue.clear();
            return;
        };
        // Escape stops a long playback.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !self.docs[i].view.macro_events.is_empty() {
            self.macro_queue.clear();
            self.docs[i].view.macro_events.clear();
            return;
        }
        // The editor has not taken the previous batch yet.
        if !self.docs[i].view.macro_events.is_empty() {
            ctx.request_repaint();
            return;
        }
        let mut events = Vec::new();
        while let Some(step) = self.macro_queue.front() {
            match step {
                Step::Event(_) => {
                    let Some(Step::Event(e)) = self.macro_queue.pop_front() else { unreachable!() };
                    events.push(e);
                }
                Step::Command(n) if events.is_empty() => {
                    let c = RECORDABLE[*n];
                    self.macro_queue.pop_front();
                    self.run_command(ctx, c);
                }
                Step::Command(_) => break,
            }
        }
        if let Some(doc) = self.active.and_then(|i| self.docs.get_mut(i)) {
            doc.view.macro_events = events;
        }
        self.focus_editor = true;
        ctx.request_repaint();
    }
}
