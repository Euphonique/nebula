//! Jump bar above the editor (like Nova): path and symbol breadcrumbs.
//! Clicking a folder or file shows its siblings, clicking a symbol shows
//! the document outline.

use eframe::egui::{self, FontId, Rect, RichText, Sense, pos2, vec2};

use super::NebulaApp;
use crate::editor::DocSource;
use crate::remote::Request;
use crate::ui::icons;
use crate::ui::sidebar::DirRef;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Root,
    Folder,
    File,
    Symbol,
}

struct Part {
    label: String,
    kind: Kind,
    /// Folder whose entries the popup lists, and the entry to highlight.
    dir: Option<(DirRef, String)>,
}

/// One entry of a folder popup.
struct Entry {
    name: String,
    path: String,
    is_dir: bool,
}

enum Pick {
    Open(DirRef, String),
    Enter(DirRef),
    Line(usize),
}

impl NebulaApp {
    fn jump_parts(&mut self, i: usize) -> Vec<Part> {
        let doc = &self.docs[i];
        let head = doc.primary_selection().head;
        let key = (doc.id, doc.version, head);
        if self.outline_cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let path = crate::editor::outline::symbol_path(doc, head);
            self.outline_cache = Some((key, path));
        }
        let mut symbols = self.outline_cache.as_ref().map(|(_, p)| p.clone()).unwrap_or_default();
        if symbols.is_empty() {
            // Other languages: the outline symbol around the cursor.
            let outline = self.doc_symbols(i);
            let line = self.docs[i].char_to_line(head);
            symbols.extend(crate::editor::symbols::current(&outline, line).map(|n| outline[n].name.clone()));
        }
        let doc = &self.docs[i];

        let mut parts = Vec::new();
        match &doc.source {
            DocSource::Remote { site_id, path } => {
                let site = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default();
                let remote = |p: String| DirRef::Remote { site_id: site_id.clone(), path: p };
                parts.push(Part { label: site, kind: Kind::Root, dir: Some((remote("/".into()), String::new())) });
                let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                for (n, s) in segs.iter().enumerate() {
                    let parent = format!("/{}", segs[..n].join("/"));
                    let own = format!("/{}", segs[..=n].join("/"));
                    parts.push(Part {
                        label: s.to_string(),
                        kind: if n + 1 == segs.len() { Kind::File } else { Kind::Folder },
                        dir: Some((remote(parent), own)),
                    });
                }
            }
            DocSource::Local(p) => {
                let rel = self.sidebar.root.as_ref().and_then(|root| p.strip_prefix(root).ok().map(|r| (root.clone(), r)));
                let local = |p: &std::path::Path| DirRef::Local(p.to_path_buf());
                match rel {
                    Some((root, r)) => {
                        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        parts.push(Part { label: name, kind: Kind::Root, dir: Some((local(&root), String::new())) });
                        let comps: Vec<String> =
                            r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                        let mut dir = root.clone();
                        for (n, c) in comps.iter().enumerate() {
                            let own = dir.join(c);
                            parts.push(Part {
                                label: c.clone(),
                                kind: if n + 1 == comps.len() { Kind::File } else { Kind::Folder },
                                dir: Some((local(&dir), own.to_string_lossy().into_owned())),
                            });
                            dir = own;
                        }
                    }
                    None => {
                        let parent = p.parent();
                        if let Some(parent) = parent
                            && let Some(name) = parent.file_name()
                        {
                            parts.push(Part {
                                label: name.to_string_lossy().into_owned(),
                                kind: Kind::Root,
                                dir: parent.parent().map(|g| (local(g), parent.to_string_lossy().into_owned())),
                            });
                        }
                        parts.push(Part {
                            label: doc.title(),
                            kind: Kind::File,
                            dir: parent.map(|d| (local(d), p.to_string_lossy().into_owned())),
                        });
                    }
                }
            }
            DocSource::Untitled(_) => parts.push(Part { label: doc.title(), kind: Kind::File, dir: None }),
        }
        parts.extend(symbols.into_iter().map(|s| Part { label: s, kind: Kind::Symbol, dir: None }));
        parts
    }

    /// Breadcrumbs for the active document, like Nova's jump bar.
    pub(super) fn jump_bar(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.active else { return };
        let theme = self.theme.clone();
        // As high as the tab rows of the side panels.
        let (full, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        ui.painter().rect_filled(full, 0.0, theme.jumpbar_bg);
        ui.painter().line_segment(
            [full.left_bottom(), full.right_bottom()],
            egui::Stroke::new(1.0, theme.separator.gamma_multiply(if theme.dark { 0.8 } else { 0.5 })),
        );

        // Contents on the line of the panel tabs (10 pt from the top, 26 pt high).
        let inner = full.with_min_y(full.top() + 6.0);
        // Minimap and split view toggles at the right end.
        let buttons = Rect::from_min_size(pos2(full.right() - 78.0, full.top() + 10.0), vec2(70.0, 26.0));
        let mut right = ui.new_child(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::left_to_right(egui::Align::Center)));
        right.spacing_mut().item_spacing.x = 2.0;
        let size = vec2(34.0, 26.0);
        let tip = t!("Geteilte Ansicht ({shortcut_label})", shortcut_label = self.shortcut_label(super::palette::Command::ToggleSplit));
        if icons::icon_button(&mut right, &theme, size, self.split.is_some(), icons::split_view)
            .on_hover_text(tip)
            .clicked()
        {
            self.toggle_split();
        }
        let minimap = self.settings.editor.minimap;
        if icons::icon_button(&mut right, &theme, size, minimap, icons::minimap).on_hover_text(t!("Minimap ein/aus")).clicked() {
            self.settings.editor.minimap = !minimap;
        }

        let bar = inner.with_max_x(buttons.left() - 4.0);
        let painter = ui.painter().with_clip_rect(bar);

        let parts = self.jump_parts(i);
        let remote = matches!(self.docs[i].source, DocSource::Remote { .. });
        let mut x = bar.left() + 10.0;
        let cy = bar.center().y;
        let font = FontId::proportional(12.5);
        let mut pick = None;
        for (n, part) in parts.iter().enumerate() {
            if n > 0 {
                icons::chevron_thin(&painter, pos2(x + 7.0, cy), theme.text_faint);
                x += 17.0;
            }
            let icon_w = match part.kind {
                Kind::Root => 20.0,
                Kind::File => 17.0,
                Kind::Symbol => 15.0,
                Kind::Folder => 0.0,
            };
            let color = if part.kind == Kind::File { theme.foreground } else { theme.text_muted };
            let g = painter.layout_no_wrap(part.label.clone(), font.clone(), color);
            let rect = Rect::from_min_size(pos2(x, cy - 11.0), vec2(icon_w + g.size().x + 8.0, 22.0));
            let clickable = part.dir.is_some() || part.kind == Kind::Symbol;
            let resp = ui.interact(rect.intersect(bar), egui::Id::unique(("jump_bar", n)), if clickable { Sense::click() } else { Sense::hover() });
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
            if clickable && (resp.hovered() || open) {
                painter.rect_filled(rect, 4.0, theme.hover);
            }
            let mut ix = x + 4.0;
            match part.kind {
                Kind::Root if remote => icons::server(&painter, pos2(ix + 7.0, cy), theme.text_muted),
                Kind::Root => icons::folder(&painter, pos2(ix + 8.0, cy), theme.text_muted, false),
                Kind::File => icons::file(&painter, pos2(ix + 6.0, cy), &part.label),
                Kind::Symbol => {
                    painter.rect_stroke(
                        Rect::from_center_size(pos2(ix + 5.0, cy), vec2(8.0, 8.0)),
                        2.0,
                        egui::Stroke::new(1.2, theme.color_of("tag.name")),
                        egui::StrokeKind::Inside,
                    );
                }
                Kind::Folder => {}
            }
            ix += icon_w;
            painter.galley(pos2(ix, cy - g.size().y / 2.0), g, color);
            x = rect.right();

            if resp.clicked() {
                self.jump_nav = part.dir.as_ref().map(|(d, _)| d.clone());
                self.jump_reveal = true;
            }
            if clickable {
                let highlight = part.dir.as_ref().map(|(_, h)| h.clone()).unwrap_or_default();
                egui::Popup::menu(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_width(rect.width().clamp(240.0, 360.0));
                        pick = if part.kind == Kind::Symbol {
                            self.symbol_menu(ui, i)
                        } else {
                            self.folder_menu(ui, part.dir.as_ref().map(|(d, _)| d), &highlight)
                        };
                        if matches!(pick, Some(Pick::Open(..) | Pick::Line(_))) {
                            ui.close();
                        }
                    });
            }
            if x > bar.right() - 12.0 {
                break;
            }
        }

        match pick {
            Some(Pick::Open(DirRef::Local(_), path)) => self.open_local(std::path::Path::new(&path)),
            Some(Pick::Open(DirRef::Remote { site_id, .. }, path)) => self.open_remote(&site_id, &path),
            Some(Pick::Enter(dir)) => self.jump_nav = Some(dir),
            Some(Pick::Line(line)) => {
                if let Some(i) = self.active {
                    self.jump_to_line(i, line);
                }
            }
            None => {}
        }
    }

    fn folder_entries(&mut self, dir: &DirRef) -> Option<Vec<Entry>> {
        match dir {
            DirRef::Local(p) => Some(
                self.sidebar
                    .local_children(p)
                    .into_iter()
                    .map(|e| Entry { name: e.name, path: e.path.to_string_lossy().into_owned(), is_dir: e.is_dir })
                    .collect(),
            ),
            DirRef::Remote { site_id, path } => {
                let listed = self.sidebar.remote.get(site_id).and_then(|t| t.dirs.get(path)).map(|v| {
                    let mut v: Vec<Entry> =
                        v.iter().map(|e| Entry { name: e.name.clone(), path: e.path.clone(), is_dir: e.is_dir }).collect();
                    v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                    v
                });
                let loading = self.sidebar.remote.get(site_id).is_some_and(|t| t.loading.contains(path));
                if listed.is_none() && !loading {
                    self.remote_request(&site_id.clone(), Request::List(path.clone()));
                }
                listed
            }
        }
    }

    fn folder_menu(&mut self, ui: &mut egui::Ui, start: Option<&DirRef>, highlight: &str) -> Option<Pick> {
        let theme = self.theme.clone();
        let dir = self.jump_nav.clone().or_else(|| start.cloned())?;
        let mut pick = None;
        // Header with the folder name and a way back up after descending.
        let (name, parent) = match &dir {
            DirRef::Local(p) => (
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned()),
                p.parent().map(|p| DirRef::Local(p.to_path_buf())),
            ),
            DirRef::Remote { site_id, path } => (
                if path == "/" { "/".into() } else { path.rsplit('/').next().unwrap_or_default().to_string() },
                (path != "/").then(|| DirRef::Remote { site_id: site_id.clone(), path: crate::remote::parent(path) }),
            ),
        };
        ui.horizontal(|ui| {
            if let Some(parent) = parent
                && ui.small_button("‹").on_hover_text(t!("Übergeordneter Ordner")).clicked()
            {
                pick = Some(Pick::Enter(parent));
            }
            ui.label(RichText::new(name).small().color(theme.text_faint));
        });
        let Some(entries) = self.folder_entries(&dir) else {
            ui.label(RichText::new(t!("Lade…")).color(theme.text_faint));
            return pick;
        };
        if entries.is_empty() {
            ui.label(RichText::new("(leer)").color(theme.text_faint));
        }
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for e in entries {
                let resp = menu_row(ui, &theme, e.path == highlight, e.is_dir, &e.name);
                if e.path == highlight && std::mem::take(&mut self.jump_reveal) {
                    resp.scroll_to_me(Some(egui::Align::Center));
                }
                if resp.clicked() {
                    pick = Some(if e.is_dir {
                        Pick::Enter(match &dir {
                            DirRef::Local(_) => DirRef::Local(e.path.into()),
                            DirRef::Remote { site_id, .. } => DirRef::Remote { site_id: site_id.clone(), path: e.path },
                        })
                    } else {
                        Pick::Open(dir.clone(), e.path)
                    });
                }
            }
        });
        pick
    }

    fn symbol_menu(&mut self, ui: &mut egui::Ui, i: usize) -> Option<Pick> {
        let theme = self.theme.clone();
        let symbols = self.doc_symbols(i);
        let line = self.docs[i].char_to_line(self.docs[i].primary_selection().head);
        let current = crate::editor::symbols::current(&symbols, line);
        if symbols.is_empty() {
            ui.label(RichText::new(t!("Keine Symbole")).color(theme.text_faint));
            return None;
        }
        let mut pick = None;
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for (n, s) in symbols.iter().enumerate() {
                let label = format!("{}{}  {}", "   ".repeat(s.depth), s.kind.badge(), s.name);
                let resp = ui.add(egui::Button::selectable(current == Some(n), label));
                if current == Some(n) && std::mem::take(&mut self.jump_reveal) {
                    resp.scroll_to_me(Some(egui::Align::Center));
                }
                if resp.clicked() {
                    pick = Some(Pick::Line(s.line));
                }
            }
        });
        pick
    }
}

/// A popup row with a file or folder icon; folders get a `›`.
fn menu_row(ui: &mut egui::Ui, theme: &crate::theme::Theme, selected: bool, is_dir: bool, name: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let p = ui.painter();
    if selected || resp.hovered() {
        p.rect_filled(rect, 4.0, if resp.hovered() { theme.hover } else { theme.selection.gamma_multiply(0.6) });
    }
    let cy = rect.center().y;
    if is_dir {
        icons::folder(p, pos2(rect.left() + 12.0, cy), theme.text_muted, false);
        icons::chevron_thin(p, pos2(rect.right() - 10.0, cy), theme.text_faint);
    } else {
        icons::file(p, pos2(rect.left() + 10.0, cy), name);
    }
    p.text(pos2(rect.left() + 26.0, cy), egui::Align2::LEFT_CENTER, name, FontId::proportional(13.0), theme.foreground);
    resp
}
