//! Preview and source linked: Ctrl+click on an element in the preview shows
//! its start tag in the editor, Ctrl+double-click edits plain text in place
//! (the change lands in the source), and the element at the cursor is
//! outlined in the preview.

use std::ops::Range;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::Deserialize;

use super::NebulaApp;
use crate::editor::document::{Change, EditKind, Selection};
use crate::html_locate::{self, ElementSig};

/// Pause after the cursor moved before the preview follows.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(super) struct LinkState {
    /// (document, version, cursor) last shown in the preview.
    pub last: Option<(u64, u64, usize)>,
    /// A different position seen at this time, shown once it settles.
    pending: Option<((u64, u64, usize), Instant)>,
}

#[derive(Deserialize)]
struct Message {
    kind: String,
    #[serde(default)]
    el: ElementSig,
    #[serde(default)]
    before: String,
    #[serde(default)]
    after: String,
}

fn char_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

fn byte_of(text: &str, ch: usize) -> usize {
    text.char_indices().nth(ch).map_or(text.len(), |(b, _)| b)
}

impl NebulaApp {
    pub(super) fn preview_link_tick(&mut self, ctx: &egui::Context) {
        for msg in self.preview.messages() {
            if let Ok(m) = serde_json::from_str::<Message>(&msg) {
                self.preview_message(m);
            }
        }
        if !self.settings.preview_link {
            return;
        }
        // Source → preview: the element at the cursor of the shown page.
        let Some(page_doc) = self.preview_state.page.as_ref().map(|p| p.doc_id) else { return };
        let Some(i) = self.active.filter(|&i| self.docs[i].id == page_doc) else { return };
        let doc = &self.docs[i];
        let key = (doc.id, doc.version, doc.primary_selection().head);
        if self.preview_link.last == Some(key) {
            return;
        }
        let now = Instant::now();
        match self.preview_link.pending {
            Some((k, since)) if k == key => {
                if now.duration_since(since) < SETTLE {
                    ctx.request_repaint_after(SETTLE - now.duration_since(since));
                    return;
                }
            }
            _ => {
                self.preview_link.pending = Some((key, now));
                ctx.request_repaint_after(SETTLE);
                return;
            }
        }
        self.preview_link.pending = None;
        self.preview_link.last = Some(key);
        let doc = &mut self.docs[i];
        let text = doc.text();
        let query = html_locate::element_at(&text, byte_of(&text, key.2));
        let json = serde_json::to_string(&query).unwrap_or_else(|_| "null".into());
        self.preview.run(&format!("window.__nebulaHighlight && __nebulaHighlight({json})"));
    }

    /// The best fitting start tag: in the page's document first, then in the
    /// other open HTML/PHP files (includes, templates).
    fn locate_element(&mut self, sig: &ElementSig) -> Option<(usize, Range<usize>)> {
        let page_doc = self.preview_state.page.as_ref().map(|p| p.doc_id);
        let mut order: Vec<usize> = (0..self.docs.len()).filter(|&i| self.docs[i].language.is_markup() && self.docs[i].hex.is_none()).collect();
        order.sort_by_key(|&i| Some(self.docs[i].id) != page_doc);
        let mut best: Option<(usize, Range<usize>, u32)> = None;
        for i in order {
            let text = self.docs[i].text();
            if let Some((range, score)) = html_locate::find_start_tag(&text, sig)
                && best.as_ref().is_none_or(|(_, _, s)| score > *s)
            {
                best = Some((i, char_of(&text, range.start)..char_of(&text, range.end), score));
            }
        }
        best.map(|(i, r, _)| (i, r))
    }

    fn preview_message(&mut self, m: Message) {
        match m.kind.as_str() {
            // A page (re)loaded: show the cursor's element again.
            "ready" => {
                self.preview_link.last = None;
                self.preview_link.pending = None;
            }
            "reveal" => match self.locate_element(&m.el) {
                Some((i, range)) => {
                    self.activate(i);
                    let doc = &mut self.docs[i];
                    doc.set_selection(Selection::new(range.start, range.end));
                    doc.view.center_line = Some(doc.char_to_line(range.start));
                    self.focus_editor = true;
                    // Outline the clicked element (the cursor is on it now).
                    self.preview_link.last = None;
                }
                None => self.info(t!("Dieses Element wurde in den geöffneten Dateien nicht gefunden.")),
            },
            "edit" => {
                let Some((i, tag)) = self.locate_element(&m.el) else {
                    self.info(t!("Dieses Element wurde in den geöffneten Dateien nicht gefunden."));
                    return;
                };
                let doc = &mut self.docs[i];
                let text = doc.text();
                let tag_end = byte_of(&text, tag.end);
                match html_locate::text_range(&text, tag_end, &m.before) {
                    Some(r) if !doc.read_only => {
                        let range = char_of(&text, r.start)..char_of(&text, r.end);
                        let new = html_locate::encode(&m.after);
                        let end = range.start + new.chars().count();
                        doc.apply(vec![Change::new(range.clone(), new)], Some(vec![Selection::new(range.start, end)]), EditKind::Other);
                        doc.view.center_line = Some(doc.char_to_line(range.start));
                        self.activate(i);
                    }
                    _ => self.info(t!("Dieser Text steht nicht als reiner Text in der Datei (z. B. von PHP erzeugt) und lässt sich nur im Code ändern.")),
                }
            }
            _ => {}
        }
    }
}
