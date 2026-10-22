//! Choosing the target folder for "copy/move to server" and "download into
//! the project": a folder browser for servers, the Windows dialog locally.

use std::path::PathBuf;

use eframe::egui::{self, RichText, Sense, vec2};

use super::{NebulaApp, modal, primary_button};
use crate::remote::{self, Request};
use crate::ui::icons;
use crate::ui::sidebar::{DirRef, Item};

pub(super) struct RemoteFolderPick {
    pub items: Vec<Item>,
    pub site_id: String,
    /// The folder being shown; the items go here on confirm.
    pub current: String,
    pub copy: bool,
}

impl NebulaApp {
    /// Windows folder dialog, starting in `start`, then downloads the items there.
    pub(super) fn pick_local_folder(&mut self, items: Vec<Item>, start: PathBuf) {
        let Some(dir) = rfd::FileDialog::new().set_title(t!("Herunterladen nach")).set_directory(&start).pick_folder() else { return };
        self.transfer(items, DirRef::Local(dir), true);
    }

    /// Returns whether the dialog stays open.
    pub(super) fn remote_folder_dialog(&mut self, ctx: &egui::Context, pick: &mut RemoteFolderPick) -> bool {
        let theme = self.theme.clone();
        let site = self.sites.get(&pick.site_id).map(|s| s.display_name()).unwrap_or_default();
        let tree = self.sidebar.remote.get(&pick.site_id);
        let listing = tree.and_then(|t| t.dirs.get(&pick.current)).map(|entries| {
            entries.iter().filter(|e| e.is_dir).map(|e| (e.name.clone(), e.path.clone())).collect::<Vec<_>>()
        });
        let loading = tree.is_some_and(|t| t.loading.contains(&pick.current));
        let failed = tree.is_none_or(|t| matches!(t.status, crate::ui::sidebar::RemoteStatus::Failed(_)));
        let mut keep = true;
        let mut confirm = false;
        let mut go = None;
        modal(ctx, egui::Id::unique("remote-folder-pick")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(t!("Zielordner auf „{site}“", site = site));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(pick.current != "/", egui::Button::new("⬆")).on_hover_text(t!("Übergeordneter Ordner")).clicked() {
                    go = Some(remote::parent(&pick.current));
                }
                ui.label(RichText::new(&pick.current).monospace());
            });
            ui.add_space(4.0);
            egui::Frame::new().fill(theme.sidebar_bg).corner_radius(6.0).inner_margin(4.0).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    match &listing {
                        None if failed => {
                            ui.label(RichText::new(t!("Keine Verbindung zum Server.")).color(theme.gutter_fg));
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(t!("Lade…"));
                            });
                        }
                        Some(dirs) if dirs.is_empty() => {
                            ui.label(RichText::new(t!("Keine Unterordner")).color(theme.gutter_fg));
                        }
                        Some(dirs) => {
                            for (name, path) in dirs {
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(rect, 5.0, theme.hover);
                                }
                                icons::folder(ui.painter(), rect.left_center() + vec2(14.0, 0.0), theme.accent, false);
                                ui.painter().text(
                                    rect.left_center() + vec2(28.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    name,
                                    egui::FontId::proportional(13.0),
                                    theme.foreground,
                                );
                                if resp.clicked() {
                                    go = Some(path.clone());
                                }
                            }
                        }
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(RichText::new(t!("Klick auf einen Ordner öffnet ihn.")).small().color(theme.gutter_fg));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let label = if pick.copy { t!("Hierher kopieren") } else { t!("Hierher verschieben") };
                if ui.add_enabled(listing.is_some(), primary_button(label, theme.accent)).clicked() {
                    confirm = true;
                }
                if ui.button(t!("Abbrechen")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        });
        if let Some(path) = go {
            pick.current = path;
        }
        // Folders not listed yet are read on demand.
        if self.sidebar.remote.get(&pick.site_id).is_some_and(|t| !t.dirs.contains_key(&pick.current) && !t.loading.contains(&pick.current))
            && !failed
            && !loading
        {
            self.remote_request(&pick.site_id.clone(), Request::List(pick.current.clone()));
        }
        if confirm {
            let target = DirRef::Remote { site_id: pick.site_id.clone(), path: pick.current.clone() };
            self.sidebar.remote_dirs.insert(pick.site_id.clone(), pick.current.clone());
            self.transfer(std::mem::take(&mut pick.items), target, pick.copy);
            return false;
        }
        keep
    }
}
