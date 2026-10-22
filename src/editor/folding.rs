//! Code folding (brackets and tags, or indentation for languages without
//! them) and the mapping between visual rows and document lines when folds
//! hide lines or long lines wrap.

use std::sync::Arc;

use super::document::Document;
use crate::lang::Language;

/// Folding is skipped for very long documents (ranges are recomputed per edit).
const MAX_FOLD_LINES: usize = 200_000;

/// Wrapping is skipped for huge documents (rows are recomputed per edit).
pub const MAX_WRAP_CHARS: usize = 8_000_000;

/// Indentation of a line in columns, or `None` for blank lines.
fn indent_of(doc: &Document, line: usize, tab: usize) -> Option<usize> {
    indent_of_text(doc.rope.line(line), tab)
}

fn indent_of_text(text: ropey::RopeSlice, tab: usize) -> Option<usize> {
    let mut col = 0;
    for ch in text.chars() {
        match ch {
            ' ' => col += 1,
            '\t' => col = (col / tab + 1) * tab,
            '\n' | '\r' => return None,
            _ => return Some(col),
        }
    }
    None
}

/// Foldable ranges `(start, end)`: lines `start+1..=end` can be hidden under `start`.
pub fn fold_ranges(doc: &mut Document, tab: usize) -> Arc<Vec<(usize, usize)>> {
    if let Some((v, r)) = &doc.view.fold_ranges
        && *v == doc.version
    {
        return r.clone();
    }
    let lines = doc.len_lines();
    let mut ranges = Vec::new();
    if lines <= MAX_FOLD_LINES {
        ranges = match Flavor::of(doc.language) {
            Some(flavor) => {
                let text = doc.text();
                syntax_ranges(&text, doc.language, flavor)
            }
            None => indent_ranges(doc, tab),
        };
    }
    let ranges = Arc::new(ranges);
    doc.view.fold_ranges = Some((doc.version, ranges.clone()));
    ranges
}

fn indent_ranges(doc: &Document, tab: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (line, indent)
    let mut last_nonblank = 0;
    for line in 0..doc.len_lines() {
        let Some(indent) = indent_of(doc, line, tab) else { continue };
        while let Some(&(start, ind)) = stack.last() {
            if ind < indent {
                break;
            }
            stack.pop();
            if last_nonblank > start {
                ranges.push((start, last_nonblank));
            }
        }
        stack.push((line, indent));
        last_nonblank = line;
    }
    while let Some((start, _)) = stack.pop() {
        if last_nonblank > start {
            ranges.push((start, last_nonblank));
        }
    }
    ranges.sort_unstable();
    ranges
}

/// How code is tokenized for bracket matching.
#[derive(Clone, Copy, PartialEq)]
struct Flavor {
    line_comment: bool,
    hash_comment: bool,
    backtick: bool,
    /// `'` starts a string only when it closes right away (Rust lifetimes).
    char_quote: bool,
}

impl Flavor {
    const C: Flavor = Flavor { line_comment: true, hash_comment: false, backtick: false, char_quote: false };
    const CSS: Flavor = Flavor { line_comment: false, ..Flavor::C };
    const JS: Flavor = Flavor { backtick: true, ..Flavor::C };
    const PHP: Flavor = Flavor { hash_comment: true, backtick: true, ..Flavor::C };

    /// Languages folded by brackets and tags; `None` folds by indentation.
    fn of(lang: Language) -> Option<Flavor> {
        Some(match lang {
            Language::Css => Flavor::CSS,
            Language::JavaScript | Language::TypeScript | Language::Tsx | Language::Go => Flavor::JS,
            Language::Php => Flavor::PHP,
            Language::Rust => Flavor { char_quote: true, ..Flavor::C },
            Language::Html | Language::Xml | Language::Scss | Language::Less | Language::Json
            | Language::C | Language::Cpp | Language::CSharp | Language::Java => Flavor::C,
            _ => return None,
        })
    }
}

/// Where the code region being scanned ends.
#[derive(Clone, Copy, PartialEq)]
enum Region {
    /// Markup text between tags.
    Text,
    /// The whole file is code.
    Code,
    Script,
    Style,
    /// `<?php ... ?>`
    Php,
}

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
];

/// Byte scanner over the text that keeps track of the current line.
struct Scan<'a> {
    b: &'a [u8],
    i: usize,
    line: usize,
    line_start: usize,
    ranges: Vec<(usize, usize)>,
}

impl Scan<'_> {
    fn at(&self, s: &str) -> bool {
        self.b[self.i..].starts_with(s.as_bytes())
    }

    fn at_ci(&self, s: &str) -> bool {
        self.b.len() - self.i >= s.len() && self.b[self.i..self.i + s.len()].eq_ignore_ascii_case(s.as_bytes())
    }

    fn bump(&mut self) {
        if self.b[self.i] == b'\n' {
            self.line += 1;
            self.line_start = self.i + 1;
        }
        self.i += 1;
    }

    /// Advances to `j`, counting lines.
    fn skip_to(&mut self, j: usize) {
        let j = j.min(self.b.len());
        while self.i < j {
            self.bump();
        }
    }

    /// Advances past the next `end` (or to the end of the text).
    fn skip_past(&mut self, end: &str) {
        let rest = &self.b[self.i..];
        let j = rest.windows(end.len()).position(|w| w == end.as_bytes()).map_or(self.b.len(), |p| self.i + p + end.len());
        self.skip_to(j);
    }

    /// Only whitespace (or the `<?php` of a template section) before the
    /// current position on its line.
    fn first_on_line(&self) -> bool {
        let prefix = self.b[self.line_start..self.i].trim_ascii();
        prefix.is_empty() || prefix == b"<?php" || prefix == b"<?"
    }

    /// A block from `start` closes at the current position.
    fn close(&mut self, start: usize) {
        // A closer at the start of its line stays visible below the fold.
        let end = if self.first_on_line() { self.line.saturating_sub(1) } else { self.line };
        if end > start {
            self.ranges.push((start, end));
        }
    }

    fn name(&mut self) -> String {
        let s = self.i;
        while self.i < self.b.len() && (self.b[self.i].is_ascii_alphanumeric() || b"-_:.".contains(&self.b[self.i])) {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[s..self.i]).to_ascii_lowercase()
    }
}

fn syntax_ranges(text: &str, lang: Language, flavor: Flavor) -> Vec<(usize, usize)> {
    let markup = lang.is_markup();
    let html = matches!(lang, Language::Html | Language::Php);
    let mut s = Scan { b: text.as_bytes(), i: 0, line: 0, line_start: 0, ranges: Vec::new() };
    let mut region = if markup { Region::Text } else { Region::Code };
    // Brackets stay open across regions: PHP templates open a block in one
    // `<?php ?>` section and close it in another.
    let mut brackets: Vec<(u8, usize)> = Vec::new();
    let mut tags: Vec<(String, usize)> = Vec::new();
    // Block comments are folded as a whole.
    let comment = |s: &mut Scan, open: &str, close: &str| {
        let start = s.line;
        s.i += open.len();
        let rest = &s.b[s.i..];
        let Some(p) = rest.windows(close.len()).position(|w| w == close.as_bytes()) else {
            s.skip_to(s.b.len());
            return;
        };
        s.skip_to(s.i + p);
        s.close(start);
        s.i += close.len();
    };

    while s.i < s.b.len() {
        let c = s.b[s.i];
        if region == Region::Text {
            if s.at("<!--") {
                comment(&mut s, "<!--", "-->");
            } else if s.at("<?") {
                if lang == Language::Php {
                    region = Region::Php;
                    s.i += 2;
                } else {
                    s.skip_past("?>");
                }
            } else if s.at("<![CDATA[") {
                s.skip_past("]]>");
            } else if s.at("</") {
                let first = s.first_on_line();
                let closer_line = s.line;
                s.i += 2;
                let name = s.name();
                s.skip_past(">");
                if let Some(pos) = tags.iter().rposition(|(n, _)| *n == name) {
                    let start = tags[pos].1;
                    tags.truncate(pos);
                    let end = if first { closer_line.saturating_sub(1) } else { closer_line };
                    if end > start {
                        s.ranges.push((start, end));
                    }
                }
            } else if c == b'<' && s.b.get(s.i + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
                let start = s.line;
                s.i += 1;
                let name = s.name();
                // Attributes up to the closing `>`, honoring quotes.
                let mut self_closing = false;
                while s.i < s.b.len() {
                    match s.b[s.i] {
                        b'>' => {
                            self_closing = s.b[s.i - 1] == b'/';
                            s.i += 1;
                            break;
                        }
                        q @ (b'"' | b'\'') => {
                            s.bump();
                            while s.i < s.b.len() && s.b[s.i] != q {
                                s.bump();
                            }
                            if s.i < s.b.len() {
                                s.i += 1;
                            }
                        }
                        b'<' if s.at("<?") => s.skip_past("?>"),
                        _ => s.bump(),
                    }
                }
                let void = html && VOID_ELEMENTS.contains(&name.as_str());
                if !self_closing && !void {
                    if html && name == "script" {
                        region = Region::Script;
                    } else if html && name == "style" {
                        region = Region::Style;
                    }
                    tags.push((name, start));
                }
            } else {
                s.bump();
            }
            continue;
        }

        // Code.
        let flavor = match region {
            Region::Script => Flavor::JS,
            Region::Style => Flavor::CSS,
            Region::Php => Flavor::PHP,
            _ => flavor,
        };
        match region {
            Region::Script if s.at_ci("</script") => {
                region = Region::Text;
                continue;
            }
            Region::Style if s.at_ci("</style") => {
                region = Region::Text;
                continue;
            }
            Region::Php if s.at("?>") => {
                region = Region::Text;
                s.i += 2;
                continue;
            }
            _ => {}
        }
        match c {
            b'/' if s.at("/*") => comment(&mut s, "/*", "*/"),
            b'/' if flavor.line_comment && s.at("//") => skip_line_comment(&mut s, region),
            b'#' if flavor.hash_comment => skip_line_comment(&mut s, region),
            b'"' | b'\'' | b'`' => {
                if (c == b'`' && !flavor.backtick)
                    || (c == b'\'' && flavor.char_quote && s.b.get(s.i + 1) != Some(&b'\\') && s.b.get(s.i + 2) != Some(&b'\''))
                {
                    s.i += 1;
                    continue;
                }
                s.i += 1;
                while s.i < s.b.len() {
                    match s.b[s.i] {
                        b'\\' => {
                            s.bump();
                            if s.i < s.b.len() {
                                s.bump();
                            }
                        }
                        q if q == c => {
                            s.i += 1;
                            break;
                        }
                        // Only template strings span lines.
                        b'\n' if c != b'`' => break,
                        _ => s.bump(),
                    }
                }
            }
            b'{' | b'[' | b'(' => {
                brackets.push((c, s.line));
                s.i += 1;
            }
            b'}' | b']' | b')' => {
                let open = match c {
                    b'}' => b'{',
                    b']' => b'[',
                    _ => b'(',
                };
                if let Some(pos) = brackets.iter().rposition(|(b, _)| *b == open) {
                    let start = brackets[pos].1;
                    brackets.truncate(pos);
                    s.close(start);
                }
                s.i += 1;
            }
            _ => s.bump(),
        }
    }

    let mut ranges = s.ranges;
    // One range per start line: the largest.
    ranges.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    ranges.dedup_by_key(|r| r.0);
    ranges
}

fn skip_line_comment(s: &mut Scan, region: Region) {
    while s.i < s.b.len() && s.b[s.i] != b'\n' {
        // `?>` ends PHP code even inside a line comment.
        if region == Region::Php && s.at("?>") {
            return;
        }
        s.i += 1;
    }
}

/// The fold range starting at `line`, if any.
pub fn range_at(doc: &mut Document, line: usize, tab: usize) -> Option<(usize, usize)> {
    let ranges = fold_ranges(doc, tab);
    ranges.binary_search_by_key(&line, |r| r.0).ok().map(|i| ranges[i])
}

pub fn toggle(doc: &mut Document, line: usize, tab: usize) {
    if doc.view.folds.remove(&line) {
        doc.view.fold_gen += 1;
    } else if range_at(doc, line, tab).is_some() {
        doc.view.folds.insert(line);
        doc.view.fold_gen += 1;
        // Move cursors out of the hidden lines.
        let (s, e) = range_at(doc, line, tab).unwrap();
        let hidden_start = doc.line_to_char(s + 1);
        let hidden_end = doc.line_end(e);
        let target = doc.line_end(s);
        for sel in &mut doc.selections {
            if (hidden_start..=hidden_end).contains(&sel.head) {
                *sel = super::document::Selection::cursor(target);
            }
        }
        doc.normalize_selections();
    }
}

/// Folds or unfolds the innermost block around the primary cursor.
pub fn toggle_at_cursor(doc: &mut Document, tab: usize) {
    let line = doc.char_to_line(doc.primary_selection().head);
    let ranges = fold_ranges(doc, tab);
    if doc.view.folds.contains(&line) || ranges.iter().any(|r| r.0 == line) {
        toggle(doc, line, tab);
        return;
    }
    if let Some(&(s, _)) = ranges.iter().filter(|(s, e)| *s < line && line <= *e).max_by_key(|r| r.0) {
        toggle(doc, s, tab);
    }
}

pub fn fold_all(doc: &mut Document, tab: usize) {
    let ranges = fold_ranges(doc, tab);
    doc.view.folds = ranges.iter().map(|r| r.0).collect();
    doc.view.fold_gen += 1;
    let head = doc.primary_selection().head;
    let line = doc.char_to_line(head);
    unfold_containing(doc, line, tab);
}

pub fn unfold_all(doc: &mut Document) {
    if !doc.view.folds.is_empty() {
        doc.view.folds.clear();
        doc.view.fold_gen += 1;
    }
}

/// Opens every fold that hides `line`.
pub fn unfold_containing(doc: &mut Document, line: usize, tab: usize) {
    if doc.view.folds.is_empty() {
        return;
    }
    let ranges = fold_ranges(doc, tab);
    let before = doc.view.folds.len();
    doc.view
        .folds
        .retain(|&s| !ranges.iter().any(|&(rs, re)| rs == s && s < line && line <= re));
    if doc.view.folds.len() != before {
        doc.view.fold_gen += 1;
    }
}

/// One visual row: a whole line, or the part of a wrapped line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    /// First char of the row, relative to the line start.
    pub start: usize,
    /// Byte offset of that char within the line.
    pub byte: usize,
    /// Display column of that char.
    pub col: usize,
    /// Columns of indentation drawn before a continuation row.
    pub pad: usize,
}

/// Maps visual rows to document lines.
#[derive(Clone)]
pub struct Rows {
    /// `None` means one row per line, all visible.
    map: Option<Arc<Vec<Row>>>,
    lines: usize,
    /// Rows are wrapped at this many columns.
    pub wrap: Option<usize>,
}

impl Rows {
    pub fn count(&self) -> usize {
        self.map.as_ref().map_or(self.lines, |m| m.len())
    }

    pub fn row(&self, row: usize) -> Row {
        match &self.map {
            Some(m) => m[row.min(m.len() - 1)],
            None => Row { line: row.min(self.lines - 1), start: 0, byte: 0, col: 0, pad: 0 },
        }
    }

    pub fn line_of_row(&self, row: usize) -> usize {
        self.row(row).line
    }

    /// First row of a line; hidden lines map to the row of their fold start.
    pub fn row_of_line(&self, line: usize) -> usize {
        let Some(m) = &self.map else { return line };
        let i = m.partition_point(|r| r.line < line);
        if i < m.len() && m[i].line == line {
            return i;
        }
        let Some(prev) = i.checked_sub(1) else { return 0 };
        m.partition_point(|r| r.line < m[prev].line)
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        self.map.as_ref().is_some_and(|m| {
            let i = m.partition_point(|r| r.line < line);
            !(i < m.len() && m[i].line == line)
        })
    }

    /// Start of the next row of the same line (char offset in the line), if
    /// the line continues on it.
    pub fn continues(&self, row: usize) -> Option<usize> {
        let m = self.map.as_ref()?;
        let (a, b) = (m.get(row)?, m.get(row + 1)?);
        (a.line == b.line).then_some(b.start)
    }

    /// Row and visual column within the row of a char position.
    pub fn locate(&self, doc: &Document, pos: usize, tab: usize) -> (usize, usize) {
        let line = doc.char_to_line(pos);
        let col = doc.column_of(pos, tab);
        let first = self.row_of_line(line);
        if self.wrap.is_none() || self.is_hidden(line) {
            return (first, col);
        }
        let offset = pos - doc.line_to_char(line);
        let mut row = first;
        while let Some(next) = self.continues(row) {
            if next > offset {
                break;
            }
            row += 1;
        }
        let r = self.row(row);
        (row, col.saturating_sub(r.col) + r.pad)
    }

    /// Char position at a visual column within a row.
    pub fn pos_at(&self, doc: &Document, row: usize, col: usize, tab: usize) -> usize {
        let r = self.row(row);
        // Rows can be a frame older than the text.
        let line = r.line.min(doc.len_lines() - 1);
        let pos = doc.pos_at_column(line, r.col + col.saturating_sub(r.pad), tab);
        let start = doc.line_to_char(line);
        let pos = pos.max(start + r.start);
        let pos = match self.continues(row) {
            // The row ends before the first char of the next one.
            Some(next) => pos.min(start + next - 1),
            None => pos,
        };
        pos.min(doc.line_end(line))
    }

    /// Visual column where a row ends (including its indentation).
    pub fn row_width(&self, doc: &Document, row: usize, tab: usize) -> usize {
        let r = self.row(row);
        let line = r.line.min(doc.len_lines() - 1);
        let end = match self.continues(row) {
            Some(next) => (doc.line_to_char(line) + next).min(doc.line_end(line)),
            None => doc.line_end(line),
        };
        doc.column_of(end, tab).saturating_sub(r.col) + r.pad
    }
}

pub fn rows(doc: &mut Document, tab: usize, wrap: Option<usize>) -> Rows {
    let lines = doc.len_lines();
    let wrap = wrap.filter(|_| doc.len_chars() <= MAX_WRAP_CHARS).map(|w| w.max(10));
    if doc.view.folds.is_empty() && wrap.is_none() {
        return Rows { map: None, lines, wrap };
    }
    if let Some((key, m)) = &doc.view.rows_cache
        && *key == (doc.version, doc.view.fold_gen, wrap, tab)
    {
        return Rows { map: Some(m.clone()), lines, wrap };
    }
    let ranges = if doc.view.folds.is_empty() { Arc::new(Vec::new()) } else { fold_ranges(doc, tab) };
    let mut hidden_until = None::<usize>;
    let mut map = Vec::with_capacity(lines);
    let mut folded = ranges.iter().filter(|r| doc.view.folds.contains(&r.0)).peekable();
    // Walking the lines in order is much faster than looking each one up.
    let mut texts = doc.rope.lines();
    for line in 0..lines {
        let text = texts.next();
        if let Some(end) = hidden_until {
            if line <= end {
                continue;
            }
            hidden_until = None;
        }
        match wrap {
            Some(w) => wrap_line(text.unwrap_or_else(|| doc.rope.line(line)), line, w, tab, &mut map),
            None => map.push(Row { line, start: 0, byte: 0, col: 0, pad: 0 }),
        }
        while let Some(&&(s, e)) = folded.peek() {
            if s < line {
                folded.next();
                continue;
            }
            if s == line {
                hidden_until = Some(hidden_until.map_or(e, |h: usize| h.max(e)));
                folded.next();
                continue;
            }
            break;
        }
    }
    let map = Arc::new(map);
    doc.view.rows_cache = Some(((doc.version, doc.view.fold_gen, wrap, tab), map.clone()));
    Rows { map: Some(map), lines, wrap }
}

/// Splits a line into rows of at most `width` columns, preferably after
/// spaces. Continuation rows are indented like the line.
fn wrap_line(text: ropey::RopeSlice, line: usize, width: usize, tab: usize, out: &mut Vec<Row>) {
    out.push(Row { line, start: 0, byte: 0, col: 0, pad: 0 });
    // Most lines fit: short enough even if every char were a tab, or
    // short enough without tabs.
    let len = text.len_chars();
    if len * tab <= width || (len <= width && text.chunks().all(|c| !c.contains('\t'))) {
        return;
    }
    let indent = indent_of_text(text, tab).unwrap_or(0);
    let pad = if indent <= width / 2 { indent } else { 0 };
    let mut row_col = 0;
    let mut avail = width;
    let mut col = 0;
    let mut byte = 0;
    let mut in_text = false;
    // Break opportunity: (char, byte, column) of the char after a space.
    let mut last_break: Option<(usize, usize, usize)> = None;
    for (i, ch) in text.chars().enumerate() {
        if ch == '\n' || ch == '\r' {
            break;
        }
        let w = if ch == '\t' { tab - col % tab } else { 1 };
        if col + w > row_col + avail && col > row_col {
            let (bi, bb, bc) = match last_break {
                Some(b) if b.2 > row_col => b,
                _ => (i, byte, col),
            };
            out.push(Row { line, start: bi, byte: bb, col: bc, pad });
            row_col = bc;
            avail = width - pad;
            last_break = None;
        }
        col += w;
        byte += ch.len_utf8();
        if ch == ' ' || ch == '\t' {
            // Not inside the indentation: that would leave an empty row.
            if in_text {
                last_break = Some((i + 1, byte, col));
            }
        } else {
            in_text = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::DocSource;

    fn doc(text: &str) -> Document {
        Document::new(DocSource::Untitled(1), text)
    }

    fn with_lang(text: &str, lang: Language) -> Document {
        let mut d = doc(text);
        d.language = lang;
        d
    }

    #[test]
    fn ranges_follow_indentation() {
        let mut d = with_lang("a:\n    b\n    c:\n        d\n    e\nx", Language::Yaml);
        let r = fold_ranges(&mut d, 4);
        assert_eq!(*r, vec![(0, 4), (2, 3)]);
    }

    #[test]
    fn ranges_follow_tags_and_brackets() {
        let html = "<ul>\n<li>a</li>\n<li>\nb\n</li>\n</ul>\n<br>\n<p>x\ny</p>";
        let mut d = with_lang(html, Language::Html);
        assert_eq!(*fold_ranges(&mut d, 4), vec![(0, 4), (2, 3), (7, 8)]);

        // Braces across PHP sections, strings and comments ignored.
        let php = "<div>\n<?php if ($a) { // {\n  echo '}';\n?>\n<b>x</b>\n<?php } ?>\n</div>";
        let mut d = with_lang(php, Language::Php);
        assert_eq!(*fold_ranges(&mut d, 4), vec![(0, 5), (1, 4)]);
        // `{` in the comment and `}` in the string don't count.
        let mut d = with_lang("a {\n  b: '}';\n  /* { */\n}", Language::Css);
        assert_eq!(*fold_ranges(&mut d, 4), vec![(0, 2)]);

        let js = "function f() {\n  return [\n    1,\n  ];\n}\n/*\n a\n*/";
        let mut d = with_lang(js, Language::JavaScript);
        assert_eq!(*fold_ranges(&mut d, 4), vec![(0, 3), (1, 2), (5, 6)]);
    }

    #[test]
    fn folding_hides_rows_and_maps_back() {
        let mut d = with_lang("a {\n    b;\n    c;\n}\nd {\n    e;\n}", Language::Css);
        toggle(&mut d, 0, 4);
        let rows = rows(&mut d, 4, None);
        assert_eq!(rows.count(), 5);
        assert_eq!(rows.line_of_row(1), 3);
        assert_eq!(rows.row_of_line(2), 0);
        assert!(rows.is_hidden(1));
        unfold_all(&mut d);
        assert_eq!(super::rows(&mut d, 4, None).count(), 7);
    }

    #[test]
    fn wrapping_splits_long_lines() {
        let mut d = doc("aaaa bbbb cccc dddd eeee ffff\nshort");
        let rows = rows(&mut d, 4, Some(10));
        // "aaaa bbbb " | "cccc dddd " | "eeee ffff" | "short"
        assert_eq!(rows.count(), 4);
        assert_eq!(rows.row(1), Row { line: 0, start: 10, byte: 10, col: 10, pad: 0 });
        assert_eq!(rows.row_of_line(1), 3);
        assert_eq!(rows.locate(&d, 12, 4), (1, 2));
        assert_eq!(rows.pos_at(&d, 1, 40, 4), 19);
        assert_eq!(rows.row_width(&d, 2, 4), 9);

        // Continuation rows keep the indentation of the line.
        let mut d = doc("    aaaa bbbb cccc");
        let rows = super::rows(&mut d, 4, Some(10));
        // "    aaaa " | "    bbbb " | "    cccc"
        assert_eq!(rows.count(), 3);
        assert_eq!(rows.row(1), Row { line: 0, start: 9, byte: 9, col: 9, pad: 4 });
        assert_eq!(rows.locate(&d, 10, 4), (1, 5));
        assert_eq!(rows.pos_at(&d, 1, 5, 4), 10);
        assert_eq!(rows.pos_at(&d, 1, 0, 4), 9);
    }
}
