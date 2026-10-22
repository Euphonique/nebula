//! Settings page for extensions: installed bundles with their preferences,
//! the Extension Library and the extension console.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use eframe::egui::{self, Color32, RichText, Ui, vec2};
use serde_json::Value;

use super::NebulaApp;
use super::extensions::ExtStatus;
use crate::ext::library::{self, LibraryEntry};
use crate::ext::manifest::{ConfigItem, Extension};
use crate::ext::ToHost;
use crate::theme::Theme;

#[derive(Default, Clone, Copy, PartialEq)]
pub enum Tab {
    #[default]
    Installed,
    Library,
    Console,
}

pub enum LibMsg {
    Catalog(anyhow::Result<Vec<LibraryEntry>>),
    Details(String, anyhow::Result<LibraryEntry>),
    Installed(String, anyhow::Result<Extension>),
    Logo(String, Option<(usize, usize, Vec<u8>)>),
}

#[derive(Default)]
pub struct ExtPageState {
    pub tab: Tab,
    pub query: String,
    pub category: String,
    catalog: Option<Arc<Vec<LibraryEntry>>>,
    loading: bool,
    focus_search: bool,
    shown: usize,
    error: Option<String>,
    details: Option<LibraryEntry>,
    installing: Vec<String>,
    logos: HashMap<String, Option<egui::TextureHandle>>,
    rx: Option<Receiver<LibMsg>>,
    tx: Option<crossbeam_channel::Sender<LibMsg>>,
    console_filter: String,
    /// Text buffers for list preferences, by key.
    list_text: HashMap<String, String>,
}

impl ExtPageState {
    fn channel(&mut self) -> crossbeam_channel::Sender<LibMsg> {
        if self.tx.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded();
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        self.tx.clone().unwrap()
    }
}

fn spawn(ctx: &egui::Context, f: impl FnOnce() -> LibMsg + Send + 'static, tx: crossbeam_channel::Sender<LibMsg>) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(f());
        ctx.request_repaint();
    });
}

impl NebulaApp {
    pub(super) fn extensions_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.poll_library(&ctx);
        let mut st = std::mem::take(&mut self.settings_ui.ext);
        ui.horizontal(|ui| {
            for (tab, label) in [(Tab::Installed, t!("Installiert")), (Tab::Library, t!("Bibliothek")), (Tab::Console, t!("Konsole"))] {
                if ui.add(egui::Button::selectable(st.tab == tab, RichText::new(label).size(13.5))).clicked() {
                    st.tab = tab;
                }
            }
        });
        ui.add_space(8.0);
        match st.tab {
            Tab::Installed => self.installed_tab(ui, &mut st),
            Tab::Library => self.library_tab(ui, &mut st),
            Tab::Console => self.console_tab(ui, &mut st),
        }
        self.settings_ui.ext = st;
    }

    fn poll_library(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.settings_ui.ext.rx.clone() else { return };
        for msg in rx.try_iter() {
            let st = &mut self.settings_ui.ext;
            match msg {
                LibMsg::Catalog(res) => {
                    st.loading = false;
                    match res {
                        Ok(all) => {
                            st.catalog = Some(Arc::new(all));
                            st.error = None;
                        }
                        Err(e) => st.error = Some(t!("Die Bibliothek ist nicht erreichbar: {e}", e = e)),
                    }
                }
                LibMsg::Details(id, res) => {
                    if let Ok(d) = res
                        && st.details.as_ref().is_some_and(|x| x.identifier == id)
                    {
                        st.details = Some(d);
                    }
                }
                LibMsg::Logo(url, img) => {
                    let tex = img.map(|(w, h, rgba)| {
                        ctx.load_texture(url.clone(), egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba), egui::TextureOptions::LINEAR)
                    });
                    st.logos.insert(url, tex);
                }
                LibMsg::Installed(id, res) => {
                    st.installing.retain(|x| x != &id);
                    match res {
                        Ok(ext) => {
                            let name = ext.name.clone();
                            self.settings.disabled_extensions.retain(|d| d != &ext.identifier);
                            self.ext.reload(&self.settings.disabled_extensions.clone());
                            let themes = if ext.themes.is_empty() { "" } else { t!(" Ihre Themes sind unter Design auswählbar.") };
                            self.info(t!("„{name}“ wurde installiert.{themes}", name = name, themes = themes));
                        }
                        Err(e) => self.error(t!("Installation fehlgeschlagen: {e}", e = e)),
                    }
                }
            }
        }
    }

    fn logo(&mut self, ctx: &egui::Context, st: &mut ExtPageState, url: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = st.logos.get(url) {
            return t.clone();
        }
        st.logos.insert(url.to_string(), None);
        let tx = st.channel();
        let u = url.to_string();
        spawn(ctx, move || {
            let img = library::get_bytes(&u).ok().and_then(|b| library::decode_png(&b));
            LibMsg::Logo(u, img)
        }, tx);
        None
    }

    fn local_logo(&mut self, ctx: &egui::Context, st: &mut ExtPageState, ext: &Extension) -> Option<egui::TextureHandle> {
        let path = ext.image()?;
        let key = format!("file:{}", path.display());
        if let Some(t) = st.logos.get(&key) {
            return t.clone();
        }
        let tex = std::fs::read(&path).ok().and_then(|b| library::decode_png(&b)).map(|(w, h, rgba)| {
            ctx.load_texture(key.clone(), egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba), egui::TextureOptions::LINEAR)
        });
        st.logos.insert(key, tex.clone());
        tex
    }

    // ---- installed ------------------------------------------------------------------

    fn installed_tab(&mut self, ui: &mut Ui, st: &mut ExtPageState) {
        let ctx = ui.ctx().clone();
        let theme = self.theme.clone();
        ui.horizontal(|ui| {
            if ui.button(t!("Aus Ordner installieren…")).clicked()
                && let Some(dir) = rfd::FileDialog::new().set_title(t!("Ordner einer Erweiterung (.novaextension)")).pick_folder()
            {
                match library::install_folder(&dir) {
                    Ok(ext) => {
                        self.ext.reload(&self.settings.disabled_extensions.clone());
                        self.info(t!("„{name}“ wurde installiert.", name = ext.name));
                    }
                    Err(e) => self.error(t!("Installation fehlgeschlagen: {e}", e = e)),
                }
            }
            if ui.button(t!("Aus ZIP installieren…")).clicked()
                && let Some(file) = rfd::FileDialog::new().add_filter(t!("Erweiterung"), &["zip"]).pick_file()
            {
                match std::fs::read(&file).map_err(anyhow::Error::from).and_then(|b| library::install_zip(&b, "")) {
                    Ok(ext) => {
                        self.ext.reload(&self.settings.disabled_extensions.clone());
                        self.info(t!("„{name}“ wurde installiert.", name = ext.name));
                    }
                    Err(e) => self.error(t!("Installation fehlgeschlagen: {e}", e = e)),
                }
            }
            if ui.button(t!("Neu laden")).on_hover_text(t!("Erweiterungen neu einlesen und starten")).clicked() {
                self.ext.status.clear();
                self.ext.reload(&self.settings.disabled_extensions.clone());
            }
            if ui.button(t!("Ordner öffnen")).clicked() {
                let dir = crate::ext::manifest::extensions_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
            }
        });
        ui.add_space(6.0);
        if self.ext.installed.is_empty() {
            ui.add_space(30.0);
            ui.label(RichText::new(t!("Noch keine Erweiterungen installiert.")).size(15.0));
            ui.label(RichText::new(t!("Unter „Bibliothek“ findest du alle Erweiterungen aus Novas Extension Library.")).color(theme.text_muted));
            if ui.button(t!("Bibliothek öffnen")).clicked() {
                st.tab = Tab::Library;
            }
            return;
        }
        for (path, err) in &self.ext.load_errors {
            ui.label(RichText::new(format!("{}: {err}", path.display())).color(theme.danger).small());
        }
        let selected = self
            .settings_ui
            .selected_extension
            .clone()
            .filter(|id| self.ext.installed.iter().any(|e| &e.identifier == id))
            .or_else(|| self.ext.installed.first().map(|e| e.identifier.clone()));
        ui.horizontal_top(|ui| {
            // List.
            ui.vertical(|ui| {
                ui.set_width(190.0);
                egui::ScrollArea::vertical().id_salt("ext-list").max_height(430.0).show(ui, |ui| {
                    let exts: Vec<Extension> = self.ext.installed.clone();
                    for e in exts {
                        let disabled = self.settings.disabled_extensions.contains(&e.identifier);
                        let status = self.ext.status.get(&e.identifier).cloned();
                        let (dot, tip) = match (&status, disabled) {
                            (_, true) => (theme.text_faint, t!("Deaktiviert")),
                            (Some(ExtStatus::Active), _) => (theme.success, t!("Läuft")),
                            (Some(ExtStatus::Failed(_)), _) => (theme.danger, t!("Fehler beim Start")),
                            (None, _) if e.has_scripts() => (theme.text_muted, t!("Wartet (startet bei passender Datei)")),
                            (None, _) => (theme.accent, t!("Aktiv (ohne Skript)")),
                        };
                        let sel = selected.as_deref() == Some(&e.identifier);
                        let resp = ui.add(egui::Button::selectable(sel, RichText::new(&e.name).color(if disabled { theme.text_faint } else { theme.foreground })).min_size(vec2(182.0, 26.0)));
                        ui.painter().circle_filled(resp.rect.right_center() - vec2(10.0, 0.0), 3.5, dot);
                        if resp.on_hover_text(tip).clicked() {
                            self.settings_ui.selected_extension = Some(e.identifier.clone());
                        }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                let Some(id) = selected else { return };
                let Some(ext) = self.ext.installed.iter().find(|e| e.identifier == id).cloned() else { return };
                egui::ScrollArea::vertical().id_salt("ext-details").max_height(430.0).auto_shrink([false, false]).show(ui, |ui| {
                    self.extension_details(ui, st, &ext, &ctx);
                });
            });
        });
    }

    fn extension_details(&mut self, ui: &mut Ui, st: &mut ExtPageState, ext: &Extension, ctx: &egui::Context) {
        let theme = self.theme.clone();
        let disabled = self.settings.disabled_extensions.contains(&ext.identifier);
        ui.horizontal(|ui| {
            if let Some(tex) = self.local_logo(ctx, st, ext) {
                ui.add(egui::Image::new(&tex).fit_to_exact_size(vec2(48.0, 48.0)).corner_radius(10.0));
            }
            ui.vertical(|ui| {
                ui.label(RichText::new(&ext.name).size(18.0).strong());
                ui.label(RichText::new(t!("{organization} · Version {version}", organization = ext.organization, version = ext.version)).color(theme.text_muted));
            });
        });
        if !ext.description.is_empty() {
            ui.add(egui::Label::new(&ext.description).wrap());
        }
        if let Some(ExtStatus::Failed(e)) = self.ext.status.get(&ext.identifier) {
            ui.add(egui::Label::new(RichText::new(e.lines().take(6).collect::<Vec<_>>().join("\n")).color(theme.danger).monospace().size(11.0)).wrap());
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(if disabled { t!("Aktivieren") } else { t!("Deaktivieren") }).clicked() {
                if disabled {
                    self.settings.disabled_extensions.retain(|d| d != &ext.identifier);
                } else {
                    self.settings.disabled_extensions.push(ext.identifier.clone());
                }
                let _ = self.settings.save();
                self.ext.reload(&self.settings.disabled_extensions.clone());
            }
            if ui.button(t!("Deinstallieren")).clicked() {
                self.settings.disabled_extensions.push(ext.identifier.clone());
                self.ext.reload(&self.settings.disabled_extensions.clone());
                match library::uninstall(ext) {
                    Ok(()) => self.info(t!("„{name}“ wurde entfernt.", name = ext.name)),
                    Err(e) => self.error(t!("Entfernen fehlgeschlagen: {e}", e = e)),
                }
                self.settings.disabled_extensions.retain(|d| d != &ext.identifier);
                self.ext.reload(&self.settings.disabled_extensions.clone());
            }
            if ui.button(t!("Im Explorer zeigen")).clicked() {
                let _ = std::process::Command::new("explorer").arg(&ext.path).spawn();
            }
            if let Some(url) = ext.repository.as_ref().or(ext.homepage.as_ref()) {
                ui.hyperlink_to(t!("Website"), url);
            }
        });

        // Preferences.
        if !ext.config.is_empty() {
            section(ui, t!("Einstellungen"));
            for item in &ext.config {
                self.config_item(ui, st, ext, item, false);
            }
        }
        if !ext.config_workspace.is_empty() {
            section(ui, "Projekt-Einstellungen");
            if self.sidebar.root.is_none() {
                ui.label(RichText::new(t!("Öffne einen Projektordner, um diese Einstellungen festzulegen.")).color(theme.text_muted));
            } else {
                for item in &ext.config_workspace {
                    self.config_item(ui, st, ext, item, true);
                }
            }
        }

        // Contributions.
        let cmds: Vec<_> = self.ext.commands.iter().filter(|c| c.ext == ext.identifier).cloned().collect();
        if !cmds.is_empty() {
            section(ui, t!("Befehle"));
            for c in cmds {
                ui.horizontal(|ui| {
                    if ui.small_button(t!("Ausführen")).clicked() {
                        self.run_ext_command(&c);
                    }
                    ui.label(&c.title);
                    if let Some(sc) = &c.shortcut {
                        ui.label(RichText::new(ui.ctx().format_shortcut(sc)).small().color(theme.text_faint));
                    }
                });
            }
        }
        let mut parts = Vec::new();
        if !ext.themes.is_empty() {
            parts.push(t!("{themes_count} Theme(s)", themes_count = ext.themes.len()));
        }
        if !ext.clips.is_empty() {
            parts.push(t!("{clips_count} Clips", clips_count = ext.clips.len()));
        }
        if !ext.sidebars.is_empty() {
            parts.push(t!("{sidebars_count} Seitenleiste(n)", sidebars_count = ext.sidebars.len()));
        }
        if !parts.is_empty() {
            ui.label(RichText::new(t!("Enthält: {parts}", parts = parts.join(", "))).color(theme.text_muted));
        }
        if !ext.themes.is_empty() {
            ui.label(RichText::new(t!("Die Themes stehen unter Einstellungen → Design und Themes zur Auswahl.")).small().color(theme.text_faint));
        }

        if let Some(readme) = ext.readme() {
            section(ui, t!("Beschreibung"));
            markdown(ui, &readme, &theme);
        }
    }

    fn config_item(&mut self, ui: &mut Ui, st: &mut ExtPageState, ext: &Extension, item: &ConfigItem, ws: bool) {
        let theme = self.theme.clone();
        if item.kind == "section" {
            ui.add_space(6.0);
            ui.label(RichText::new(&item.title).strong());
            if !item.description.is_empty() {
                ui.add(egui::Label::new(RichText::new(&item.description).small().color(theme.text_muted)).wrap());
            }
            // Extensions often repeat sections in the project settings: keep their IDs apart.
            ui.indent(("cfg", ws, &item.title), |ui| {
                for c in &item.children {
                    self.config_item(ui, st, ext, c, ws);
                }
            });
            return;
        }
        if item.kind == "command" {
            if let Some(cmd) = &item.command
                && ui.button(&item.title).clicked()
            {
                self.ext.host.send(ToHost::Command { ext: ext.identifier.clone(), name: cmd.clone(), editor: None, args: Vec::new() });
            }
            return;
        }
        let Some(key) = item.key.clone() else { return };
        let current = self.ext.host.config.lock().unwrap().get(ws, &key);
        let mut new: Option<Value> = None;
        ui.add_space(4.0);
        match item.kind.as_str() {
            "boolean" => {
                let mut b = current.as_bool().unwrap_or(false);
                if ui.checkbox(&mut b, &item.title).changed() {
                    new = Some(Value::Bool(b));
                }
            }
            "number" => {
                ui.horizontal(|ui| {
                    ui.label(&item.title);
                    let mut n = current.as_f64().unwrap_or(0.0);
                    let mut dv = egui::DragValue::new(&mut n);
                    if let (Some(min), Some(max)) = (item.min, item.max) {
                        dv = dv.range(min..=max);
                    }
                    if ui.add(dv).changed() {
                        new = Some(serde_json::json!(n));
                    }
                });
            }
            "enum" => {
                ui.horizontal(|ui| {
                    ui.label(&item.title);
                    let cur = current.as_str().unwrap_or("").to_string();
                    let shown = item.values.iter().find(|v| v.0 == cur).map_or(cur.clone(), |v| v.1.clone());
                    egui::ComboBox::from_id_salt(("enum", ws, &key)).selected_text(shown).show_ui(ui, |ui| {
                        for (raw, title) in &item.values {
                            if ui.selectable_label(*raw == cur, title).clicked() {
                                new = Some(Value::String(raw.clone()));
                            }
                        }
                    });
                });
            }
            "text" | "stringArray" | "pathArray" => {
                ui.label(&item.title);
                let is_list = item.kind != "text";
                let buf = st.list_text.entry(format!("{ws}:{key}")).or_insert_with(|| match &current {
                    Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("\n"),
                    Value::String(s) => s.clone(),
                    _ => String::new(),
                });
                let resp = ui.add(egui::TextEdit::multiline(buf).desired_rows(if is_list { 3 } else { 4 }).desired_width(f32::INFINITY).hint_text(if is_list { t!("Ein Eintrag pro Zeile") } else { item.placeholder.as_str() }));
                if resp.changed() {
                    new = Some(if is_list {
                        Value::Array(buf.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Value::String(l.to_string())).collect())
                    } else {
                        Value::String(buf.clone())
                    });
                }
            }
            _ => {
                // string and path
                ui.horizontal(|ui| {
                    ui.label(&item.title);
                    let mut s = current.as_str().map(str::to_string).unwrap_or_else(|| if current.is_null() { String::new() } else { current.to_string() });
                    let w = if item.kind == "path" { ui.available_width() - 40.0 } else { ui.available_width() };
                    if ui.add(egui::TextEdit::singleline(&mut s).hint_text(item.placeholder.as_str()).desired_width(w.max(120.0))).changed() {
                        new = Some(if s.is_empty() { Value::Null } else { Value::String(s.clone()) });
                    }
                    if item.kind == "path" && ui.button("…").clicked() {
                        let picked = rfd::FileDialog::new().pick_file();
                        if let Some(p) = picked {
                            new = Some(Value::String(crate::ext::host::js_path(&p)));
                        }
                    }
                });
            }
        }
        if !item.description.is_empty() {
            ui.add(egui::Label::new(RichText::new(&item.description).small().color(theme.text_faint)).wrap());
        }
        if let Some(v) = new {
            self.ext.host.config.lock().unwrap().set(ws, &key, v);
            self.ext.host.send(ToHost::ConfigChanged { workspace: ws, key });
        }
    }

    // ---- library -----------------------------------------------------------------------

    fn library_tab(&mut self, ui: &mut Ui, st: &mut ExtPageState) {
        let ctx = ui.ctx().clone();
        let theme = self.theme.clone();
        let mut reset = false;
        ui.horizontal(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text(t!("Erweiterungen suchen…")).desired_width(300.0));
            if std::mem::take(&mut st.focus_search) {
                resp.request_focus();
            }
            if resp.changed() {
                reset = true;
            }
            let cat_label = library::CATEGORIES.iter().find(|c| c.0 == st.category).map_or(t!("Alle"), |c| crate::i18n::tr(c.1));
            egui::ComboBox::from_id_salt("ext-cat").selected_text(cat_label).show_ui(ui, |ui| {
                for (id, label) in library::CATEGORIES {
                    if ui.selectable_label(st.category == *id, crate::i18n::tr(label)).clicked() {
                        st.category = id.to_string();
                        reset = true;
                    }
                }
            });
            if ui.button(t!("Aktualisieren")).clicked() && !st.loading {
                st.loading = true;
                let tx = st.channel();
                spawn(&ctx, || LibMsg::Catalog(library::catalog()), tx);
            }
            if st.loading {
                ui.spinner();
            }
        });
        if reset || st.shown == 0 {
            st.shown = 40;
        }
        if st.catalog.is_none() && !st.loading && st.error.is_none() {
            // Show the cached catalog right away and refresh it in the background.
            st.catalog = library::cached_catalog().map(Arc::new);
            st.loading = true;
            st.focus_search = true;
            let tx = st.channel();
            spawn(&ctx, || LibMsg::Catalog(library::catalog()), tx);
        }
        if let Some(e) = &st.error {
            ui.label(RichText::new(e).color(theme.danger));
        }
        ui.add_space(4.0);

        if let Some(details) = st.details.clone() {
            if ui.button(t!("← Zurück zur Liste")).clicked() {
                st.details = None;
                return;
            }
            egui::ScrollArea::vertical().id_salt("lib-details").max_height(420.0).auto_shrink([false, false]).show(ui, |ui| {
                self.library_row(ui, st, &details, &ctx, true);
                ui.add_space(6.0);
                if !details.entitlements.is_empty() {
                    ui.label(RichText::new(t!("Berechtigungen: {entitlements}", entitlements = details.entitlements.join(", "))).small().color(theme.text_muted));
                }
                if let Some(repo) = &details.repository {
                    ui.hyperlink_to(t!("Quellcode"), repo);
                }
                match &details.readme {
                    Some(r) => markdown(ui, r, &theme),
                    None => {
                        ui.spinner();
                    }
                }
            });
            return;
        }

        let Some(catalog) = st.catalog.clone() else { return };
        let matches = library::filter(&catalog, &st.query, &st.category);
        ui.label(RichText::new(t!("{matches_count} Erweiterungen", matches_count = matches.len())).small().color(theme.text_faint));
        let more = matches.len() > st.shown;
        let results: Vec<LibraryEntry> = matches.into_iter().take(st.shown).cloned().collect();
        egui::ScrollArea::vertical().id_salt("lib-list").max_height(420.0).auto_shrink([false, false]).show(ui, |ui| {
            for entry in &results {
                if self.library_row(ui, st, entry, &ctx, false) {
                    let id = entry.identifier.clone();
                    st.details = Some(entry.clone());
                    if entry.readme.is_none() {
                        let tx = st.channel();
                        spawn(&ctx, move || LibMsg::Details(id.clone(), library::details(&id)), tx);
                    }
                }
                ui.separator();
            }
            if more && ui.button(t!("Mehr anzeigen")).clicked() {
                st.shown += 40;
            }
        });
    }

    /// One library entry; returns true when its details were requested.
    fn library_row(&mut self, ui: &mut Ui, st: &mut ExtPageState, entry: &LibraryEntry, ctx: &egui::Context, detailed: bool) -> bool {
        let theme = self.theme.clone();
        let mut open = false;
        ui.horizontal(|ui| {
            let logo = entry.logo.as_ref().and_then(|u| self.logo(ctx, st, u));
            let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), egui::Sense::hover());
            match logo {
                Some(tex) => {
                    egui::Image::new(&tex).corner_radius(9.0).paint_at(ui, rect);
                }
                None => {
                    ui.painter().rect_filled(rect, 9.0, theme.hover.gamma_multiply(2.0));
                }
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let name = RichText::new(&entry.name).strong().size(14.5);
                    if detailed {
                        ui.label(name);
                    } else if ui.add(egui::Label::new(name).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        open = true;
                    }
                    ui.label(RichText::new(format!("{} · {}", entry.organization, entry.version)).small().color(theme.text_muted));
                });
                ui.add(egui::Label::new(RichText::new(&entry.description).color(theme.foreground)).wrap());
                ui.horizontal(|ui| {
                    let cats: Vec<&str> = entry
                        .categories
                        .iter()
                        .map(|c| library::CATEGORIES.iter().find(|x| x.0 == c).map_or(c.as_str(), |x| crate::i18n::tr(x.1)))
                        .collect();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let installed = self.ext.installed.iter().find(|e| e.identifier == entry.identifier);
                        if st.installing.contains(&entry.identifier) {
                            ui.spinner();
                            ui.label(t!("Wird installiert…"));
                        } else {
                            let label = match installed {
                                Some(e) if e.version != entry.version && !entry.version.is_empty() => Some(t!("Aktualisieren")),
                                Some(_) => None,
                                None => Some(t!("Installieren")),
                            };
                            match label {
                                Some(l) => {
                                    if ui.add(super::primary_button(l, theme.accent)).clicked() {
                                        st.installing.push(entry.identifier.clone());
                                        let e = entry.clone();
                                        let tx = st.channel();
                                        spawn(ctx, move || LibMsg::Installed(e.identifier.clone(), library::install(&e)), tx);
                                    }
                                }
                                None => {
                                    ui.label(RichText::new(t!("✓ Installiert")).color(theme.success));
                                }
                            }
                        }
                        if !detailed && ui.button(t!("Details")).clicked() {
                            open = true;
                        }
                        // Added last in the right-to-left row, so it gets the space left over.
                        ui.add(egui::Label::new(RichText::new(t!("{cats} · {downloads} Downloads", cats = cats.join(", "), downloads = entry.downloads)).small().color(theme.text_faint)).truncate());
                    });
                });
            });
        });
        open
    }

    // ---- console -------------------------------------------------------------------------

    fn console_tab(&mut self, ui: &mut Ui, st: &mut ExtPageState) {
        let theme = self.theme.clone();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.console_filter).hint_text(t!("Filtern")).desired_width(240.0));
            if ui.button(t!("Leeren")).clicked() {
                self.ext.log.clear();
            }
        });
        ui.add_space(4.0);
        let filter = st.console_filter.to_lowercase();
        let names: HashMap<String, String> = self.ext.installed.iter().map(|e| (e.identifier.clone(), e.name.clone())).collect();
        egui::ScrollArea::vertical().id_salt("ext-console").max_height(440.0).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for line in &self.ext.log {
                let name = names.get(&line.ext).cloned().unwrap_or_else(|| line.ext.clone());
                if !filter.is_empty() && !line.text.to_lowercase().contains(&filter) && !name.to_lowercase().contains(&filter) {
                    continue;
                }
                let color = match line.level.as_str() {
                    "error" => theme.danger,
                    "warning" => theme.modified,
                    "info" => theme.text_muted,
                    _ => theme.foreground,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&name).small().color(theme.text_faint));
                    ui.add(egui::Label::new(RichText::new(&line.text).monospace().size(11.5).color(color)).wrap());
                });
            }
            if self.ext.log.is_empty() {
                ui.label(RichText::new(t!("Keine Ausgaben.")).color(theme.text_muted));
            }
        });
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).strong().size(14.0));
    ui.separator();
}

/// Renders Markdown with egui widgets (headings, paragraphs, lists, code, links).
pub fn markdown(ui: &mut Ui, text: &str, theme: &Theme) {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
    let mut spans: Vec<RichText> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut heading: Option<u8> = None;
    let mut strong = false;
    let mut emph = false;
    let mut code_block: Option<String> = None;
    let mut list_depth = 0usize;
    let mut link: Option<String> = None;
    let mut link_text = String::new();

    let flush = |ui: &mut Ui, spans: &mut Vec<RichText>, links: &mut Vec<(String, String)>, prefix: Option<String>| {
        if spans.is_empty() && links.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if let Some(p) = prefix {
                ui.label(p);
            }
            for s in spans.drain(..) {
                ui.label(s);
            }
            for (t, u) in links.drain(..) {
                ui.hyperlink_to(t, u);
            }
        });
    };

    for ev in parser {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => heading = Some(level as u8),
            Event::End(TagEnd::Heading(_)) => {
                ui.add_space(6.0);
                flush(ui, &mut spans, &mut links, None);
                heading = None;
                ui.add_space(2.0);
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                let prefix = (list_depth > 0).then(|| format!("{}• ", "   ".repeat(list_depth - 1)));
                flush(ui, &mut spans, &mut links, prefix);
                if list_depth == 0 {
                    ui.add_space(4.0);
                }
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                ui.add_space(3.0);
            }
            Event::Start(Tag::Item) => {}
            Event::End(TagEnd::Item) => {
                let prefix = format!("{}• ", "   ".repeat(list_depth.saturating_sub(1)));
                flush(ui, &mut spans, &mut links, Some(prefix));
            }
            Event::Start(Tag::Strong) => strong = true,
            Event::End(TagEnd::Strong) => strong = false,
            Event::Start(Tag::Emphasis) => emph = true,
            Event::End(TagEnd::Emphasis) => emph = false,
            Event::Start(Tag::CodeBlock(kind)) => {
                let _ = matches!(kind, CodeBlockKind::Fenced(_));
                code_block = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(code) = code_block.take() {
                    egui::Frame::new().fill(theme.hover.gamma_multiply(1.5)).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(code.trim_end()).monospace().size(11.5));
                    });
                    ui.add_space(4.0);
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                link = Some(dest_url.to_string());
                link_text.clear();
            }
            Event::End(TagEnd::Link) => {
                if let Some(u) = link.take() {
                    // Keep links inline with the text.
                    flush(ui, &mut spans, &mut Vec::new(), None);
                    links.push((std::mem::take(&mut link_text), u));
                }
            }
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => {}
            Event::Text(t) => {
                if let Some(c) = code_block.as_mut() {
                    c.push_str(&t);
                } else if link.is_some() {
                    link_text.push_str(&t);
                } else {
                    let mut r = RichText::new(t.to_string());
                    if let Some(level) = heading {
                        r = r.strong().size(match level {
                            1 => 20.0,
                            2 => 17.0,
                            _ => 15.0,
                        });
                    }
                    if strong {
                        r = r.strong();
                    }
                    if emph {
                        r = r.italics();
                    }
                    spans.push(r);
                }
            }
            Event::Code(t) => spans.push(RichText::new(t.to_string()).monospace().background_color(theme.hover.gamma_multiply(1.5))),
            Event::SoftBreak => spans.push(RichText::new(" ")),
            Event::HardBreak => flush(ui, &mut spans, &mut links, None),
            Event::Rule => {
                ui.separator();
            }
            _ => {}
        }
    }
    flush(ui, &mut spans, &mut links, None);
    let _ = Arc::new(());
    let _ = Color32::TRANSPARENT;
}
