//! Live preview next to the editor: picks the page to show, serves its
//! resources (open buffers first, then disk or server) and refreshes it
//! while typing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, FontId, Rect, Sense, pos2, vec2};

use super::NebulaApp;
use crate::editor::DocSource;
use crate::preview::{self, ORIGIN, ProtocolRequest};
use crate::remote::{self, Request};
use crate::settings::PreviewDevice;
use crate::ui::icons;

/// Where URL paths of the current page resolve to.
#[derive(Clone, PartialEq, Debug)]
enum Base {
    Local(PathBuf),
    Remote { site_id: String, root: String },
    Untitled,
}

/// The page shown in the preview.
pub(super) struct Page {
    doc_id: u64,
    base: Base,
    /// Shown from the site's web URL (PHP); refreshed after saving.
    live: bool,
    title: String,
    /// Open documents the page loaded, with the version that was served.
    served: HashMap<u64, u64>,
}

#[derive(Default)]
pub(super) struct PreviewState {
    pub page: Option<Page>,
    /// Remote files fetched for the preview: (site, path) -> bytes.
    cache: HashMap<(String, String), Arc<Vec<u8>>>,
    /// Remote reads in flight: tag -> responder and content type.
    pub pending: HashMap<u64, (String, wry::RequestAsyncResponder, &'static str)>,
    /// A served document changed: refresh at this time (css only?).
    refresh_at: Option<(Instant, bool)>,
    /// Width of the visible web view (toasts move out of its way).
    pub covered_width: f32,
}

const DEBOUNCE: Duration = Duration::from_millis(250);

impl NebulaApp {
    /// Shows the preview in the right panel, or closes the panel if it shows the preview.
    pub(super) fn toggle_preview(&mut self) {
        use crate::settings::RightTab;
        if self.settings.right_panel && self.settings.right_tab == RightTab::Preview {
            self.settings.right_panel = false;
        } else {
            self.settings.right_panel = true;
            self.settings.right_tab = RightTab::Preview;
        }
        self.sync_preview_flag();
    }

    /// The preview runs while the right panel shows it.
    pub(super) fn sync_preview_flag(&mut self) {
        use crate::settings::RightTab;
        self.settings.preview = self.settings.right_panel && self.settings.right_tab == RightTab::Preview;
        if !self.settings.preview {
            self.preview.place(None, 1.0);
        }
    }

    /// The web URL of a document when the site has one configured.
    pub(super) fn live_url(&self, source: &DocSource) -> Option<String> {
        match source {
            DocSource::Remote { site_id, path } => {
                let site = self.sites.get(site_id)?;
                let base = site.web_url.trim().trim_end_matches('/');
                let home = self.remote_home(site_id);
                let rel = path.strip_prefix(&home)?;
                (!base.is_empty()).then(|| format!("{base}{}", if rel.starts_with('/') { rel.to_string() } else { format!("/{rel}") }))
            }
            DocSource::Local(p) => {
                let (root, t) = self.publish_target()?;
                let site = self.sites.get(&t.site_id)?;
                let base = site.web_url.trim().trim_end_matches('/');
                let rel = p.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
                let site_root = remote::normalize(&site.root_path);
                let remote_dir = remote::normalize(&t.remote_path);
                let below = remote_dir.strip_prefix(&site_root).unwrap_or("");
                (!base.is_empty()).then(|| format!("{base}{below}/{rel}"))
            }
            DocSource::Untitled(_) => None,
        }
    }

    fn remote_home(&self, site_id: &str) -> String {
        let site_root = self.sites.get(site_id).map(|s| remote::normalize(&s.root_path)).unwrap_or_else(|| "/".into());
        self.sidebar
            .remote
            .get(site_id)
            .and_then(|t| t.home.clone())
            .filter(|h| !h.is_empty())
            .unwrap_or(site_root)
    }

    /// Base and URL path of a document.
    fn page_location(&self, source: &DocSource, id: u64) -> (Base, String) {
        match source {
            DocSource::Local(p) => {
                let root = self
                    .sidebar
                    .root
                    .clone()
                    .filter(|r| p.starts_with(r))
                    .or_else(|| p.parent().map(Path::to_path_buf))
                    .unwrap_or_default();
                let rel = p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/");
                (Base::Local(root), format!("/{rel}"))
            }
            DocSource::Remote { site_id, path } => {
                let home = self.remote_home(site_id);
                let (root, rel) = match path.strip_prefix(&home) {
                    Some(rel) if home != "/" => (home.clone(), rel.to_string()),
                    _ => ("/".to_string(), path.clone()),
                };
                let rel = if rel.starts_with('/') { rel } else { format!("/{rel}") };
                (Base::Remote { site_id: site_id.clone(), root }, rel)
            }
            DocSource::Untitled(_) => (Base::Untitled, format!("/__untitled/{id}.html")),
        }
    }

    /// Follows the active document and refreshes after edits.
    pub(super) fn update_preview(&mut self, ctx: &egui::Context) {
        if !self.settings.preview || !self.preview.is_ready() {
            return;
        }
        // Show the active document if it is a page; otherwise keep the last page
        // (editing a stylesheet updates the page that uses it).
        if let Some(i) = self.active {
            let doc = &self.docs[i];
            let name = match &doc.source {
                DocSource::Untitled(_) if doc.language.is_markup() => "x.html".to_string(),
                s => s.file_name(),
            };
            let current = self.preview_state.page.as_ref().map(|p| p.doc_id);
            if preview::is_page(&name) && current != Some(doc.id) {
                let (base, path) = self.page_location(&doc.source, doc.id);
                let php = name.to_ascii_lowercase().ends_with(".php");
                let live_url = if php { self.live_url(&doc.source) } else { None };
                let title = doc.title();
                let doc_id = doc.id;
                if self.preview_state.page.as_ref().is_none_or(|p| p.base != base) {
                    self.preview_state.cache.clear();
                }
                let url = live_url.clone().unwrap_or_else(|| format!("{ORIGIN}{}", preview::percent_encode(&path)));
                self.preview_state.page = Some(Page { doc_id, base, live: live_url.is_some(), title, served: HashMap::new() });
                self.preview_state.refresh_at = None;
                self.preview.navigate(&url);
            }
        }
        // The page document was closed.
        if let Some(page) = &self.preview_state.page
            && self.doc_index(page.doc_id).is_none()
            && self.active.is_none()
        {
            self.preview_state.page = None;
        }

        // Served documents that changed since.
        if let Some(page) = &self.preview_state.page
            && !page.live
        {
            let mut changed = false;
            let mut css_only = true;
            for (doc_id, version) in &page.served {
                if let Some(i) = self.doc_index(*doc_id)
                    && self.docs[i].version != *version
                {
                    changed = true;
                    css_only &= self.docs[i].language.is_css();
                }
            }
            if changed && self.preview_state.refresh_at.is_none() {
                self.preview_state.refresh_at = Some((Instant::now() + DEBOUNCE, css_only));
            }
        }
        if let Some((at, css_only)) = self.preview_state.refresh_at {
            let now = Instant::now();
            if now >= at {
                self.preview_state.refresh_at = None;
                // Mark as served now so typing doesn't queue another refresh
                // before the page asks again.
                if let Some(page) = &mut self.preview_state.page {
                    for (doc_id, version) in page.served.iter_mut() {
                        if let Some(d) = self.docs.iter().find(|d| d.id == *doc_id) {
                            *version = d.version;
                        }
                    }
                }
                if css_only {
                    self.preview.reload_css();
                } else {
                    self.preview.reload();
                }
            } else {
                ctx.request_repaint_after(at - now);
            }
        }
    }

    /// A document was saved: pages shown from the live site reload.
    pub(super) fn preview_saved(&mut self, source: &DocSource) {
        let Some(page) = &self.preview_state.page else { return };
        if !page.live {
            // Saved files on the server might be cached for the preview.
            if let DocSource::Remote { site_id, path } = source {
                self.preview_state.cache.remove(&(site_id.clone(), path.clone()));
            }
            return;
        }
        let same_site = match (source, &page.base) {
            (DocSource::Remote { site_id, .. }, Base::Remote { site_id: s, .. }) => s == site_id,
            (DocSource::Local(_), Base::Local(_)) => true,
            _ => false,
        };
        if same_site {
            self.preview_state.refresh_at = Some((Instant::now() + Duration::from_millis(400), false));
        }
    }

    /// Answers resource requests from the page.
    pub(super) fn serve_preview(&mut self) {
        for ProtocolRequest { path, responder } in self.preview.requests() {
            self.serve_one(path, responder);
        }
    }

    fn serve_one(&mut self, path: String, responder: wry::RequestAsyncResponder) {
        let Some(page) = &self.preview_state.page else {
            preview::not_found(responder, &path);
            return;
        };
        let base = page.base.clone();
        let dark = self.theme.dark;

        // Untitled documents live under a fixed path.
        let source = if let Some(id) = path.strip_prefix("/__untitled/").and_then(|s| s.strip_suffix(".html")) {
            let Some(i) = id.parse().ok().and_then(|id| self.doc_index(id)) else {
                preview::not_found(responder, &path);
                return;
            };
            self.docs[i].source.clone()
        } else {
            match &base {
                Base::Local(root) => {
                    let rel = path.trim_start_matches('/');
                    let mut p = root.clone();
                    for seg in rel.split('/').filter(|s| !s.is_empty() && *s != "." && *s != "..") {
                        p.push(seg);
                    }
                    DocSource::Local(p)
                }
                Base::Remote { site_id, root } => DocSource::Remote {
                    site_id: site_id.clone(),
                    path: remote::join(root, path.trim_start_matches('/')),
                },
                Base::Untitled => {
                    preview::not_found(responder, &path);
                    return;
                }
            }
        };

        // Open documents are served from the buffer, with unsaved changes
        // (image tabs have no text: those come from disk or server).
        if let Some(i) = self.docs.iter().position(|d| d.source == source && d.image.is_none()) {
            let doc = &mut self.docs[i];
            let name = match &doc.source {
                DocSource::Untitled(_) => "x.html".to_string(),
                s => s.file_name(),
            };
            let text = doc.text();
            let (id, version) = (doc.id, doc.version);
            let title = doc.title();
            if let Some(page) = &mut self.preview_state.page {
                page.served.insert(id, version);
            }
            if preview::is_markdown(&name) {
                let html = preview::markdown_page(&title, &text, dark);
                preview::respond(responder, 200, "text/html; charset=utf-8", html.into_bytes());
            } else {
                preview::respond(responder, 200, preview::content_type(&name), text.as_bytes().to_vec());
            }
            return;
        }

        match source {
            DocSource::Local(mut p) => {
                if p.is_dir() {
                    match ["index.html", "index.htm", "index.php"].iter().map(|n| p.join(n)).find(|f| f.is_file()) {
                        Some(index) => p = index,
                        None => {
                            preview::not_found(responder, &path);
                            return;
                        }
                    }
                }
                let name = p.to_string_lossy().into_owned();
                match std::fs::read(&p) {
                    Ok(bytes) if preview::is_markdown(&name) => {
                        let title = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let html = preview::markdown_page(&title, &String::from_utf8_lossy(&bytes), dark);
                        preview::respond(responder, 200, "text/html; charset=utf-8", html.into_bytes());
                    }
                    Ok(bytes) => preview::respond(responder, 200, preview::content_type(&name), bytes),
                    Err(_) => preview::not_found(responder, &path),
                }
            }
            DocSource::Remote { site_id, path: mut remote_path } => {
                if remote_path.ends_with('/') {
                    remote_path.push_str("index.html");
                }
                let ty = preview::content_type(&remote_path);
                if let Some(bytes) = self.preview_state.cache.get(&(site_id.clone(), remote_path.clone())) {
                    preview::respond(responder, 200, ty, bytes.to_vec());
                    return;
                }
                match self.send_remote(&site_id, Request::Read(remote_path.clone())) {
                    Some(tag) => {
                        self.preview_state.pending.insert(tag, (site_id, responder, ty));
                    }
                    None => preview::not_found(responder, &remote_path),
                }
            }
            DocSource::Untitled(_) => preview::not_found(responder, &path),
        }
    }

    /// A remote read for the preview finished. Returns false if `tag` isn't one.
    pub(super) fn preview_read_done(&mut self, tag: u64, path: &str, data: Option<Vec<u8>>) -> bool {
        let Some((site_id, responder, ty)) = self.preview_state.pending.remove(&tag) else { return false };
        match data {
            Some(bytes) => {
                if preview::is_markdown(path) {
                    let html = preview::markdown_page(path, &String::from_utf8_lossy(&bytes), self.theme.dark);
                    preview::respond(responder, 200, "text/html; charset=utf-8", html.into_bytes());
                } else {
                    self.preview_state.cache.insert((site_id, path.to_string()), Arc::new(bytes.clone()));
                    preview::respond(responder, 200, ty, bytes);
                }
            }
            None => preview::not_found(responder, path),
        }
        true
    }

    pub(super) fn preview_site_failed(&mut self, site_id: &str) {
        let tags: Vec<u64> = self
            .preview_state
            .pending
            .iter()
            .filter(|(_, (s, _, _))| s == site_id)
            .map(|(t, _)| *t)
            .collect();
        for tag in tags {
            if let Some((_, responder, _)) = self.preview_state.pending.remove(&tag) {
                preview::respond(responder, 502, "text/plain; charset=utf-8", b"Server nicht erreichbar".to_vec());
            }
        }
    }

    /// The preview in the right panel: buttons in the panel header (from
    /// `right` leftwards, a LIVE badge at `badge_x`); returns the area for the web view.
    pub(super) fn preview_pane(&mut self, ui: &mut egui::Ui, header: Rect, body: Rect, right: f32, badge_x: f32) -> Option<Rect> {
        let theme = self.theme.clone();
        let cy = header.center().y;
        if let Some(page) = &self.preview_state.page {
            let r = Rect::from_min_size(pos2(badge_x, header.top()), vec2(40.0, header.height()));
            if page.live {
                ui.painter().text(pos2(badge_x, cy), Align2::LEFT_CENTER, "LIVE", FontId::proportional(10.0), theme.success);
            }
            ui.interact(r, egui::Id::unique("preview_title"), Sense::hover()).on_hover_text(&page.title);
        }

        // Buttons, right to left.
        let mut x = right;
        let device = self.settings.preview_device;
        let mut button = |ui: &mut egui::Ui, tip: &str, active: bool, paint: fn(&egui::Painter, egui::Pos2, egui::Color32)| {
            let r = Rect::from_center_size(pos2(x, cy), vec2(26.0, 24.0));
            x -= 28.0;
            let resp = ui.interact(r, egui::Id::unique(("preview_btn", tip)), Sense::click()).on_hover_text(tip);
            if active {
                ui.painter().rect_filled(r, 6.0, theme.hover.gamma_multiply(2.2));
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, theme.hover);
            }
            paint(ui.painter(), r.center(), if resp.hovered() || active { theme.foreground } else { theme.text_muted });
            resp
        };
        if button(ui, t!("Entwicklertools"), false, icons::devtools).clicked() {
            self.preview.open_devtools();
        }
        if button(ui, t!("Im Browser öffnen"), false, icons::external).clicked() {
            self.open_in_browser(&ui.ctx().clone());
        }
        let landscape = self.settings.preview_landscape;
        let devices = button(ui, t!("Gerätebreite"), device != PreviewDevice::Fit, icons::devices);
        egui::Popup::menu(&devices).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let mut choices: Vec<PreviewDevice> = PreviewDevice::ALL.to_vec();
            choices.extend(self.settings.preview_widths.iter().map(|&w| PreviewDevice::Custom(w)));
            let mut remove = None;
            for d in choices {
                let label = match (d, d.width(landscape)) {
                    (PreviewDevice::Custom(w), _) => t!("{w} px", w = w),
                    (_, Some(w)) => t!("{label} ({w} px)", label = d.label(), w = w),
                    (_, None) => d.label().to_string(),
                };
                ui.horizontal(|ui| {
                    if ui.selectable_label(device == d, label).clicked() {
                        self.settings.preview_device = d;
                        ui.close();
                    }
                    if let PreviewDevice::Custom(w) = d
                        && ui.small_button("×").on_hover_text(t!("Breite entfernen")).clicked()
                    {
                        remove = Some(w);
                    }
                });
            }
            if let Some(w) = remove {
                self.settings.preview_widths.retain(|&x| x != w);
                if self.settings.preview_device == PreviewDevice::Custom(w) {
                    self.settings.preview_device = PreviewDevice::Fit;
                }
            }
            ui.separator();
            ui.checkbox(&mut self.settings.preview_landscape, t!("Querformat (Handy, Tablet)"));
            // A new width: typed in, added with Enter or the button.
            ui.horizontal(|ui| {
                let id = egui::Id::unique("preview-new-width");
                let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
                let field = ui.add(egui::TextEdit::singleline(&mut text).hint_text(t!("Breite in px")).desired_width(90.0));
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let value = text.trim().trim_end_matches("px").trim().parse::<u32>().ok().filter(|w| (200..=7680).contains(w));
                if (ui.add_enabled(value.is_some(), egui::Button::new(t!("Hinzufügen"))).clicked() || enter)
                    && let Some(w) = value
                {
                    if !self.settings.preview_widths.contains(&w) {
                        self.settings.preview_widths.push(w);
                        self.settings.preview_widths.sort_unstable();
                    }
                    self.settings.preview_device = PreviewDevice::Custom(w);
                    text.clear();
                }
                ui.data_mut(|d| d.insert_temp(id, text));
            });
        });
        if button(ui, t!("Neu laden"), false, icons::refresh).clicked() {
            self.preview_state.cache.clear();
            self.preview.reload();
        }
        if button(ui, t!("Vor"), false, icons::forward).clicked() {
            self.preview.go(true);
        }
        if button(ui, t!("Zurück"), false, icons::back).clicked() {
            self.preview.go(false);
        }

        if let Some(err) = &self.preview.error {
            ui.painter().text(body.center(), Align2::CENTER_CENTER, err, FontId::proportional(12.5), theme.text_muted);
            return None;
        }
        if self.preview_state.page.is_none() {
            ui.painter().text(
                body.center(),
                Align2::CENTER_CENTER,
                t!("Öffne eine HTML-, Markdown- oder Bilddatei."),
                FontId::proportional(12.5),
                theme.text_muted,
            );
            return None;
        }
        let Some(width) = device.width(landscape) else {
            self.preview.set_zoom(1.0);
            return Some(body);
        };
        // A device width: the page in its own width, centered; scaled down if
        // the panel is narrower.
        let area = Rect::from_min_max(body.min + vec2(10.0, 26.0), body.max - vec2(10.0, 10.0));
        let zoom = (area.width() / width).min(1.0);
        let view = Rect::from_center_size(
            pos2(area.center().x, area.top() + area.height() / 2.0),
            vec2(width * zoom, area.height()),
        );
        let painter = ui.painter();
        painter.rect_filled(body, 0.0, theme.hover.gamma_multiply(0.6));
        painter.rect_stroke(view.expand(1.0), 2.0, egui::Stroke::new(1.0, theme.gutter_fg.gamma_multiply(0.5)), egui::StrokeKind::Outside);
        let label = if device.turns() && landscape { t!("{device} quer", device = device.label()) } else { device.label().to_string() };
        let mut info = t!("{label} · {width} px", label = label, width = width);
        if zoom < 0.999 {
            info.push_str(&format!(" · {} %", (zoom * 100.0).round()));
        }
        painter.text(pos2(area.center().x, body.top() + 13.0), Align2::CENTER_CENTER, info, FontId::proportional(11.0), theme.text_muted);
        self.preview.set_zoom(zoom as f64);
        Some(view)
    }
}

impl NebulaApp {
    /// Creates, positions or hides the web view after the layout pass.
    pub(super) fn place_preview(&mut self, ctx: &egui::Context, frame: &eframe::Frame, rect: Option<Rect>) {
        if !self.settings.preview || self.docs.is_empty() {
            self.preview.place(None, 1.0);
            self.preview_state.covered_width = 0.0;
            return;
        }
        if !self.preview.ensure(frame, ctx) {
            return;
        }
        self.update_preview(ctx);
        // The web view is a native window above everything egui draws, so it
        // steps aside while dialogs and menus are open.
        let covered = self.dialog.is_some() || !self.host_keys.is_empty() || egui::Popup::is_any_open(ctx);
        let rect = rect.filter(|_| !covered);
        self.preview_state.covered_width = rect.map_or(0.0, |r| r.width());
        self.preview.place(rect, ctx.pixels_per_point());
        // Clicking anywhere in the app takes keyboard focus back from the page.
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.preview.release_focus();
        }
    }
}
