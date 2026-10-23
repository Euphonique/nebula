//! Opening database tabs and doing what they ask (save settings, host keys).

use std::sync::Arc;

use eframe::egui;

use super::NebulaApp;
use crate::db_view::{Action, DbView};
use crate::editor::{DocSource, Document};

impl NebulaApp {
    /// Opens (or shows) the database tab of a server.
    pub(super) fn open_db_tab(&mut self, ctx: &egui::Context, site_id: &str) {
        if let Some(i) = self.docs.iter().position(|d| d.db.as_ref().is_some_and(|v| v.site_id() == site_id)) {
            self.activate(i);
            return;
        }
        let Some(site) = self.sites.get(site_id).cloned() else { return };
        let view = Arc::new(DbView::new(&site));
        if !site.db.user.trim().is_empty() {
            view.connect(&site, ctx);
        }
        self.untitled += 1;
        let mut doc = Document::new(DocSource::Untitled(self.untitled), "");
        doc.suggested_name = Some(t!("DB · {name}", name = site.display_name()));
        doc.read_only = true;
        doc.db = Some(view);
        let at = self.active.map_or(self.docs.len(), |i| i + 1);
        self.docs.insert(at, doc);
        self.activate(at);
    }

    pub(super) fn handle_db_action(&mut self, ctx: &egui::Context, view: &DbView, action: Action) {
        match action {
            Action::None => {}
            Action::HostKey { site_id, prompt } => self.host_keys.push_back((site_id, prompt)),
            Action::Save { site_id, settings, password } => {
                let Some(site) = self.sites.sites.iter_mut().find(|s| s.id == site_id) else { return };
                site.db = settings;
                let site = site.clone();
                if !password.is_empty()
                    && let Err(e) = site.set_db_password(&password)
                {
                    self.error(format!("{e:#}"));
                }
                if let Err(e) = self.sites.save() {
                    self.error(t!("Server konnten nicht gespeichert werden: {e}", e = format!("{:#}", e)));
                }
                view.connect(&site, ctx);
            }
        }
    }
}
