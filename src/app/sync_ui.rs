//! "Synchronisieren": reads the project and its server folder, shows what
//! would change in both directions and carries out what the user confirms.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, RichText};

use super::{Dialog, NebulaApp, modal, primary_button};
use crate::remote::{self, Request, ScannedFile, SyncOp};
use crate::settings::PublishTarget;
use crate::sync::{self, Item, Kind, LocalFile, Resolution};

/// Most local files a synchronization looks at.
const LOCAL_LIMIT: usize = 50_000;

pub(super) struct SyncRun {
    root: PathBuf,
    target: PublishTarget,
    stage: Stage,
}

enum Stage {
    /// Waiting for the server listing.
    Scanning { tag: u64, local: Vec<LocalFile> },
    Preview { items: Vec<Item> },
    /// Transfers running on the server connection.
    Running { tag: u64, transferred: HashSet<String> },
    /// Reading both sides again to remember the new state.
    Rescanning { tag: u64, local: Vec<LocalFile>, transferred: HashSet<String> },
}

impl NebulaApp {
    pub(super) fn start_sync(&mut self) {
        if self.sync.is_some() {
            self.dialog = Some(Dialog::Sync);
            return;
        }
        let Some((root, target)) = self.publish_target() else {
            self.info(t!("Verknüpfe das Projekt zuerst mit einem Serverordner."));
            self.open_publish_setup();
            return;
        };
        let local = match sync::scan_local(&root, &target.exclude_list(), LOCAL_LIMIT) {
            Ok(local) => local,
            Err(e) => {
                self.error(t!("Synchronisieren nicht möglich: {e}", e = e));
                return;
            }
        };
        let request = Request::Scan { root: remote::normalize(&target.remote_path), exclude: target.exclude_list() };
        let Some(tag) = self.send_remote(&target.site_id, request) else {
            self.error(t!("Der verknüpfte Server existiert nicht mehr."));
            return;
        };
        self.sync = Some(SyncRun { root, target, stage: Stage::Scanning { tag, local } });
        self.dialog = Some(Dialog::Sync);
    }

    /// True if the reply belonged to the synchronization.
    pub(super) fn sync_scanned(&mut self, tag: u64, files: Vec<ScannedFile>, truncated: bool) -> bool {
        let Some(run) = &mut self.sync else { return false };
        match std::mem::replace(&mut run.stage, Stage::Preview { items: Vec::new() }) {
            Stage::Scanning { tag: t, .. } | Stage::Rescanning { tag: t, .. } if t == tag && truncated => {
                // Files beyond the limit would look deleted or new.
                self.sync = None;
                if matches!(self.dialog, Some(Dialog::Sync)) {
                    self.dialog = None;
                }
                self.error(t!("Synchronisieren nicht möglich: Der Serverordner enthält sehr viele Dateien – große Ordner (z. B. vendor) in die Ausschlussliste aufnehmen."));
                true
            }
            Stage::Scanning { tag: t, local } if t == tag => {
                let mut items = sync::plan(&local, &files, &run.target.synced);
                // Decided conflicts show the newer side first.
                items.sort_by_key(|i| (order(i.kind), i.rel.clone()));
                if items.is_empty() {
                    // Remember the state so the next run knows both sides.
                    let snap = sync::snapshot(&local, &files, &run.target.synced, &HashSet::new());
                    let root = run.root.clone();
                    self.sync = None;
                    self.store_snapshot(&root, snap);
                    if matches!(self.dialog, Some(Dialog::Sync)) {
                        self.dialog = None;
                    }
                    self.info(t!("Alles synchron – keine Unterschiede."));
                } else {
                    run.stage = Stage::Preview { items };
                }
                true
            }
            Stage::Rescanning { tag: t, local, transferred } if t == tag => {
                let snap = sync::snapshot(&local, &files, &run.target.synced, &transferred);
                let root = run.root.clone();
                // Uploaded and downloaded files count as published.
                if let Some(t) = self.settings.publish.get_mut(&root.to_string_lossy().into_owned()) {
                    for f in local.iter().filter(|f| transferred.contains(&f.rel)) {
                        t.uploaded.insert(f.rel.clone(), f.mtime);
                    }
                }
                self.sync = None;
                self.store_snapshot(&root, snap);
                true
            }
            other => {
                run.stage = other;
                false
            }
        }
    }

    fn store_snapshot(&mut self, root: &std::path::Path, snap: std::collections::BTreeMap<String, sync::SyncedState>) {
        if let Some(t) = self.settings.publish.get_mut(&root.to_string_lossy().into_owned()) {
            t.synced = snap;
        }
        let _ = self.settings.save();
    }

    /// The transfers finished: report and read both sides again.
    pub(super) fn sync_done(&mut self, tag: u64, done: usize, failed: Vec<String>) -> bool {
        let Some(run) = &mut self.sync else { return false };
        let Stage::Running { tag: t, transferred } = &run.stage else { return false };
        if *t != tag {
            return false;
        }
        let transferred = transferred.clone();
        let (root, target) = (run.root.clone(), run.target.clone());
        self.sidebar.refresh_local(None);
        self.remote_request(&target.site_id, Request::List(remote::normalize(&target.remote_path)));
        if failed.is_empty() {
            self.info(if done == 1 { t!("Synchronisiert: 1 Schritt.").into() } else { t!("Synchronisiert: {done} Schritte.", done = done) });
        } else {
            self.error(t!("Synchronisiert mit Fehlern ({done} erledigt):\n{failed}", done = done, failed = failed.join("\n")));
        }
        // Without a complete local list the old state is kept; files then show
        // up as conflicts next time instead of being remembered wrongly.
        let Ok(local) = sync::scan_local(&root, &target.exclude_list(), LOCAL_LIMIT) else {
            self.sync = None;
            return true;
        };
        let request = Request::Scan { root: remote::normalize(&target.remote_path), exclude: target.exclude_list() };
        match self.send_remote(&target.site_id, request) {
            Some(tag) => {
                if let Some(run) = &mut self.sync {
                    run.stage = Stage::Rescanning { tag, local, transferred };
                }
            }
            None => self.sync = None,
        }
        true
    }

    /// A request of the synchronization failed: stop it.
    pub(super) fn sync_failed(&mut self, tag: u64, message: &str) -> bool {
        let Some(run) = &self.sync else { return false };
        let ours = match &run.stage {
            Stage::Scanning { tag: t, .. } | Stage::Running { tag: t, .. } | Stage::Rescanning { tag: t, .. } => *t == tag,
            Stage::Preview { .. } => false,
        };
        if ours {
            self.sync = None;
            if matches!(self.dialog, Some(Dialog::Sync)) {
                self.dialog = None;
            }
            self.error(t!("Synchronisieren fehlgeschlagen: {message}", message = message));
        }
        ours
    }

    /// Returns whether the dialog stays open.
    pub(super) fn sync_dialog(&mut self, ctx: &egui::Context) -> bool {
        let theme = self.theme.clone();
        let mut keep = true;
        let mut run_now = false;
        let mut cancel = false;
        let site = self.sync.as_ref().and_then(|r| self.sites.get(&r.target.site_id)).map(|s| s.display_name()).unwrap_or_default();
        let Some(run) = &mut self.sync else { return false };
        modal(ctx, egui::Id::unique("sync")).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.heading(t!("Synchronisieren"));
            ui.label(
                RichText::new(format!("{}  ⇄  {site}: {}", run.root.display(), remote::normalize(&run.target.remote_path)))
                    .small()
                    .color(theme.gutter_fg),
            );
            ui.add_space(8.0);
            match &mut run.stage {
                Stage::Scanning { .. } | Stage::Rescanning { .. } => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t!("Lese Projekt und Server ein…"));
                    });
                    ui.add_space(8.0);
                    if ui.button(t!("Abbrechen")).clicked() {
                        cancel = true;
                    }
                }
                Stage::Running { .. } => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t!("Überträgt… (Fortschritt in der Übertragungsliste)"));
                    });
                    ui.add_space(8.0);
                    if ui.button(t!("Im Hintergrund weiter")).clicked() {
                        keep = false;
                    }
                }
                Stage::Preview { items } => {
                    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                        for kind in [Kind::Conflict, Kind::Upload, Kind::Download, Kind::DeleteRemote, Kind::DeleteLocal] {
                            let group: Vec<usize> = items.iter().enumerate().filter(|(_, i)| i.kind == kind).map(|(n, _)| n).collect();
                            if group.is_empty() {
                                continue;
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{} ({})", title(kind), group.len())).strong());
                                if kind == Kind::Conflict {
                                    if ui.small_button(t!("Neuere Seite übernehmen")).clicked() {
                                        for &n in &group {
                                            items[n].resolution = items[n].suggestion;
                                        }
                                    }
                                } else {
                                    if ui.small_button(t!("Alle")).clicked() {
                                        group.iter().for_each(|&n| items[n].selected = true);
                                    }
                                    if ui.small_button(t!("Keine")).clicked() {
                                        group.iter().for_each(|&n| items[n].selected = false);
                                    }
                                }
                            });
                            if matches!(kind, Kind::DeleteLocal) {
                                ui.label(RichText::new(t!("Lokale Dateien landen im Papierkorb.")).small().color(theme.gutter_fg));
                            }
                            for &n in &group {
                                let item = &mut items[n];
                                ui.horizontal(|ui| {
                                    if kind == Kind::Conflict {
                                        ui.selectable_value(&mut item.resolution, Resolution::Skip, "–").on_hover_text(t!("Überspringen"));
                                        ui.selectable_value(&mut item.resolution, Resolution::Upload, "↑").on_hover_text(t!("Lokale Fassung hochladen"));
                                        ui.selectable_value(&mut item.resolution, Resolution::Download, "↓").on_hover_text(t!("Serverfassung herunterladen"));
                                        ui.label(RichText::new(&item.rel).monospace());
                                    } else {
                                        let color = if matches!(kind, Kind::DeleteLocal | Kind::DeleteRemote) { theme.danger } else { theme.foreground };
                                        ui.checkbox(&mut item.selected, RichText::new(&item.rel).monospace().color(color));
                                    }
                                    ui.label(RichText::new(&item.note).small().color(theme.gutter_fg));
                                });
                            }
                        }
                    });
                    ui.add_space(10.0);
                    let count = items.iter().filter(|i| i.action().is_some()).count();
                    ui.horizontal(|ui| {
                        let label = if count == 1 { t!("1 Änderung ausführen").to_string() } else { t!("{count} Änderungen ausführen", count = count) };
                        if ui.add_enabled(count > 0, primary_button(&label, theme.accent)).clicked() {
                            run_now = true;
                        }
                        if ui.button(t!("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                }
            }
        });
        if cancel {
            self.sync = None;
            return false;
        }
        if run_now {
            self.run_sync();
        }
        keep
    }

    fn run_sync(&mut self) {
        let Some(run) = &mut self.sync else { return };
        let Stage::Preview { items, .. } = &run.stage else { return };
        let remote_root = remote::normalize(&run.target.remote_path);
        let (mut ops, mut transferred, mut trash_failed) = (Vec::new(), HashSet::new(), Vec::new());
        for item in items {
            let local = run.root.join(item.rel.replace('/', "\\"));
            let remote_path = remote::join(&remote_root, &item.rel);
            match item.action() {
                Some(Kind::Upload) => {
                    ops.push(SyncOp::Upload { local, remote: remote_path });
                    transferred.insert(item.rel.clone());
                }
                Some(Kind::Download) => {
                    ops.push(SyncOp::Download { remote: remote_path, local });
                    transferred.insert(item.rel.clone());
                }
                Some(Kind::DeleteRemote) => ops.push(SyncOp::Delete(remote_path)),
                Some(Kind::DeleteLocal) => {
                    if !crate::win32::move_to_recycle_bin(&local) {
                        trash_failed.push(item.rel.clone());
                    }
                }
                _ => {}
            }
        }
        let site_id = run.target.site_id.clone();
        if !trash_failed.is_empty() {
            self.error(t!("Nicht in den Papierkorb verschoben:\n{trash_failed}", trash_failed = trash_failed.join("\n")));
        }
        if ops.is_empty() {
            // Only local deletions: no server round trip needed.
            let tag = u64::MAX;
            if let Some(run) = &mut self.sync {
                run.stage = Stage::Running { tag, transferred };
            }
            self.sync_done(tag, 0, Vec::new());
            return;
        }
        match self.send_remote(&site_id, Request::Sync(ops)) {
            Some(tag) => {
                if let Some(run) = &mut self.sync {
                    run.stage = Stage::Running { tag, transferred };
                }
            }
            None => {
                self.sync = None;
                self.error(t!("Der verknüpfte Server existiert nicht mehr."));
            }
        }
    }
}

fn order(kind: Kind) -> u8 {
    match kind {
        Kind::Conflict => 0,
        Kind::Upload => 1,
        Kind::Download => 2,
        Kind::DeleteRemote => 3,
        Kind::DeleteLocal => 4,
    }
}

fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Conflict => t!("Konflikte – bitte entscheiden"),
        Kind::Upload => t!("Hochladen"),
        Kind::Download => t!("Herunterladen"),
        Kind::DeleteRemote => t!("Auf dem Server löschen"),
        Kind::DeleteLocal => t!("Lokal löschen"),
    }
}
