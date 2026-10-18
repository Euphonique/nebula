//! Find and replace within a document.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use super::document::{Change, Document, EditKind, Selection};

#[derive(Default)]
pub struct FindState {
    pub visible: bool,
    pub show_replace: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    pub matches: Vec<Range<usize>>,
    pub current: Option<usize>,
    pub error: Option<String>,
    /// Request focus on the query field next frame.
    pub focus_query: bool,
    key: Option<(u64, u64, String, bool, bool, bool)>,
}

impl FindState {
    fn regex(&self) -> Result<Option<Regex>, String> {
        if self.query.is_empty() {
            return Ok(None);
        }
        let mut pattern = if self.use_regex {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        if self.whole_word {
            pattern = format!(r"\b(?:{pattern})\b");
        }
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .build()
            .map(Some)
            .map_err(|e| e.to_string().lines().last().unwrap_or("Ungültiger Ausdruck").to_string())
    }

    /// Recomputes matches when the query or document changed.
    pub fn update(&mut self, doc: &mut Document) {
        let key = (
            doc.id,
            doc.version,
            self.query.clone(),
            self.case_sensitive,
            self.whole_word,
            self.use_regex,
        );
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        self.matches.clear();
        self.error = None;
        let re = match self.regex() {
            Ok(Some(re)) => re,
            Ok(None) => return,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let text = doc.text();
        // Convert byte offsets to char offsets incrementally.
        let mut chars = 0usize;
        let mut last_byte = 0usize;
        for m in re.find_iter(&text) {
            if m.start() == m.end() {
                continue;
            }
            chars += text[last_byte..m.start()].chars().count();
            let start = chars;
            chars += m.as_str().chars().count();
            last_byte = m.end();
            self.matches.push(start..chars);
            if self.matches.len() >= 100_000 {
                break;
            }
        }
        self.sync_current(doc);
    }

    fn sync_current(&mut self, doc: &Document) {
        let sel = doc.primary_selection().range();
        self.current = self.matches.iter().position(|m| *m == sel);
    }

    /// Selects the next (or previous) match relative to the primary selection.
    pub fn find(&mut self, doc: &mut Document, forward: bool) {
        self.update(doc);
        if self.matches.is_empty() {
            return;
        }
        let sel = doc.primary_selection().range();
        let idx = if forward {
            self.matches
                .iter()
                .position(|m| m.start >= sel.end && *m != sel)
                .unwrap_or(0)
        } else {
            self.matches
                .iter()
                .rposition(|m| m.end <= sel.start && *m != sel)
                .unwrap_or(self.matches.len() - 1)
        };
        let m = self.matches[idx].clone();
        doc.set_selection(Selection::new(m.start, m.end));
        doc.view.center_line = Some(doc.char_to_line(m.start));
        self.current = Some(idx);
    }

    fn replacement_for(&self, doc: &Document, range: Range<usize>) -> String {
        if !self.use_regex {
            return self.replacement.clone();
        }
        let text = doc.slice(range);
        match self.regex() {
            Ok(Some(re)) => re.replace(&text, self.replacement.as_str()).into_owned(),
            _ => self.replacement.clone(),
        }
    }

    pub fn replace_current(&mut self, doc: &mut Document) {
        self.update(doc);
        let sel = doc.primary_selection().range();
        if self.matches.contains(&sel) {
            let text = self.replacement_for(doc, sel.clone());
            let end = sel.start + text.chars().count();
            doc.apply(
                vec![Change::new(sel.clone(), text)],
                Some(vec![Selection::cursor(end)]),
                EditKind::Other,
            );
            self.update(doc);
        }
        self.find(doc, true);
    }

    pub fn replace_all(&mut self, doc: &mut Document) -> usize {
        self.update(doc);
        let changes: Vec<Change> = self
            .matches
            .iter()
            .map(|m| Change::new(m.clone(), self.replacement_for(doc, m.clone())))
            .collect();
        let n = changes.len();
        if n > 0 {
            doc.apply(changes, None, EditKind::Other);
            self.update(doc);
        }
        n
    }
}
