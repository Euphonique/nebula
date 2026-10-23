//! Find and replace within a document.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use super::document::{Change, Document, EditKind, Selection};

/// The replacement as the regex crate expects it. Without regex mode it is
/// literal (`$` stays `$`); with it, `\1`…`\9` mean groups like `$1`, and
/// `\n`, `\t`, `\\` are a line break, a tab and a backslash.
pub fn replacement_template(raw: &str, regex: bool) -> String {
    if !regex {
        return raw.replace('$', "$$");
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some(d @ '0'..='9') => {
                chars.next();
                out.push_str(&format!("${{{d}}}"));
            }
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// A replacement shown before it is made: the line around the match.
pub struct Preview {
    pub line: usize,
    pub before: String,
    pub old: String,
    pub new: String,
    pub after: String,
}

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
    /// Search only in this range of a document: (doc id, chars, length of the document when set).
    pub scope: Option<(u64, Range<usize>, usize)>,
    key: Option<(u64, u64, String, bool, bool, bool, Option<Range<usize>>)>,
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
            .map_err(|e| e.to_string().lines().last().unwrap_or(t!("Ungültiger Ausdruck")).to_string())
    }

    /// Limits the search to the primary selection of `doc`, or lifts the limit.
    pub fn set_scope(&mut self, doc: &Document, on: bool) {
        let sel = doc.primary_selection().range();
        self.scope = (on && !sel.is_empty()).then(|| (doc.id, sel, doc.len_chars()));
    }

    /// The search range in `doc` (follows edits by the change in length).
    pub fn scope_in(&mut self, doc: &Document) -> Option<Range<usize>> {
        let (id, range, len) = self.scope.as_mut()?;
        if *id != doc.id {
            return None;
        }
        let now = doc.len_chars();
        if now != *len {
            range.end = (range.end as isize + now as isize - *len as isize).max(range.start as isize) as usize;
            *len = now;
        }
        range.end = range.end.min(now);
        Some(range.clone())
    }

    /// Recomputes matches when the query or document changed.
    pub fn update(&mut self, doc: &mut Document) {
        let scope = self.scope_in(doc);
        let key = (
            doc.id,
            doc.version,
            self.query.clone(),
            self.case_sensitive,
            self.whole_word,
            self.use_regex,
            scope.clone(),
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
            if let Some(s) = &scope
                && (start < s.start || chars > s.end)
            {
                if start > s.end {
                    break;
                }
                continue;
            }
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
            Ok(Some(re)) => re.replace(&text, replacement_template(&self.replacement, true).as_str()).into_owned(),
            _ => self.replacement.clone(),
        }
    }

    /// What replacing would do at the current match, or else the next one
    /// after the cursor.
    pub fn preview(&self, doc: &Document) -> Option<Preview> {
        let head = doc.primary_selection().range().start;
        let m = match self.current {
            Some(c) => self.matches.get(c)?.clone(),
            None => self.matches.iter().find(|m| m.start >= head).or(self.matches.first())?.clone(),
        };
        let line = doc.char_to_line(m.start);
        let (ls, le) = (doc.line_to_char(line), doc.line_end(line).max(m.start));
        // Some context around the match, cut at 40 chars each side.
        let before = doc.slice(m.start.saturating_sub(40).max(ls)..m.start);
        let after = doc.slice(m.end.min(le)..le.min(m.end + 40).max(m.end.min(le)));
        Some(Preview { line, before, old: doc.slice(m.clone()), new: self.replacement_for(doc, m), after })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::DocSource;

    #[test]
    fn replace_in_selection_only() {
        let mut d = Document::new(DocSource::Untitled(1), "a a\na a\na a\n");
        d.set_selection(Selection::new(4, 7)); // second line
        let mut f = FindState { query: "a".into(), replacement: "bb".into(), ..Default::default() };
        f.set_scope(&d, true);
        assert_eq!(f.replace_all(&mut d), 2);
        assert_eq!(d.rope.to_string(), "a a\nbb bb\na a\n");
        // The range grew with the replacement; nothing outside matches.
        f.update(&mut d);
        assert!(f.matches.is_empty());
        f.set_scope(&d, false);
        f.update(&mut d);
        assert_eq!(f.matches.len(), 4);
    }

    #[test]
    fn replacement_templates() {
        assert_eq!(replacement_template("$preis", false), "$$preis");
        assert_eq!(replacement_template(r"\2-\1\n\t\\x\q", true), "${2}-${1}\n\t\\x\\q");
        let re = Regex::new(r"(\w+)@(\w+)").unwrap();
        assert_eq!(re.replace("anna@web", replacement_template(r"\2: \1", true).as_str()), "web: anna");
        assert_eq!(re.replace("anna@web", replacement_template("$1 kostet $5", false).as_str()), "$1 kostet $5");
    }

    #[test]
    fn preview_and_regex_replace() {
        let mut d = Document::new(DocSource::Untitled(1), "<p>Datum: 2026-10-23</p>\nund 2025-01-02\n");
        let mut f = FindState { query: r"(\d{4})-(\d\d)-(\d\d)".into(), replacement: r"\3.\2.\1".into(), use_regex: true, ..Default::default() };
        f.update(&mut d);
        let p = f.preview(&d).unwrap();
        assert_eq!((p.line, p.before.as_str(), p.old.as_str(), p.new.as_str(), p.after.as_str()), (0, "<p>Datum: ", "2026-10-23", "23.10.2026", "</p>"));
        assert_eq!(f.replace_all(&mut d), 2);
        assert_eq!(d.rope.to_string(), "<p>Datum: 23.10.2026</p>\nund 02.01.2025\n");
    }
}
