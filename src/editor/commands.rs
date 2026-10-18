//! Editing commands that operate on a [`Document`] independent of the UI.

use std::sync::LazyLock;

use regex::Regex;

use super::document::{Change, Document, EditKind, Selection};
use crate::settings::EditorSettings;

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr", "!doctype",
];

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum CharClass {
    Space,
    Word,
    Punct,
    Newline,
}

fn class_of(c: char) -> CharClass {
    if c == '\n' {
        CharClass::Newline
    } else if c.is_whitespace() {
        CharClass::Space
    } else if is_word_char(c) {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

fn indent_unit(settings: &EditorSettings) -> String {
    if settings.use_spaces {
        " ".repeat(settings.tab_size)
    } else {
        "\t".into()
    }
}

fn leading_whitespace(line: &str) -> &str {
    let end = line
        .char_indices()
        .find(|(_, c)| !matches!(c, ' ' | '\t'))
        .map_or(line.len(), |(i, _)| i);
    &line[..end]
}

// ---- movement ----------------------------------------------------------------

pub fn word_boundary_left(doc: &Document, pos: usize) -> usize {
    let mut p = pos;
    while p > 0 && class_of(doc.rope.char(p - 1)) == CharClass::Space {
        p -= 1;
    }
    if p == 0 {
        return 0;
    }
    let class = class_of(doc.rope.char(p - 1));
    if class == CharClass::Newline {
        return p - 1;
    }
    while p > 0 && class_of(doc.rope.char(p - 1)) == class {
        p -= 1;
    }
    p
}

pub fn word_boundary_right(doc: &Document, pos: usize) -> usize {
    let len = doc.len_chars();
    let mut p = pos;
    while p < len && class_of(doc.rope.char(p)) == CharClass::Space {
        p += 1;
    }
    if p >= len {
        return len;
    }
    let class = class_of(doc.rope.char(p));
    if class == CharClass::Newline {
        return p + 1;
    }
    while p < len && class_of(doc.rope.char(p)) == class {
        p += 1;
    }
    p
}

/// Range of the word under `pos`, if any.
pub fn word_at(doc: &Document, pos: usize) -> Option<std::ops::Range<usize>> {
    let len = doc.len_chars();
    let is_word = |p: usize| p < len && is_word_char(doc.rope.char(p));
    let at = if is_word(pos) {
        pos
    } else if pos > 0 && is_word(pos - 1) {
        pos - 1
    } else {
        return None;
    };
    let mut start = at;
    while start > 0 && is_word(start - 1) {
        start -= 1;
    }
    let mut end = at;
    while is_word(end) {
        end += 1;
    }
    Some(start..end)
}

pub fn move_horizontal(doc: &mut Document, forward: bool, word: bool, extend: bool) {
    let len = doc.len_chars();
    let mut sels = doc.selections.clone();
    for s in &mut sels {
        let head = if !extend && !s.is_empty() && !word {
            if forward { s.range().end } else { s.range().start }
        } else if forward {
            if word {
                word_boundary_right(doc, s.head)
            } else {
                (s.head + 1).min(len)
            }
        } else if word {
            word_boundary_left(doc, s.head)
        } else {
            s.head.saturating_sub(1)
        };
        *s = if extend {
            Selection::new(s.anchor, head)
        } else {
            Selection::cursor(head)
        };
    }
    doc.selections = sels;
    doc.normalize_selections();
    doc.view.scroll_to_cursor = true;
}

pub fn move_vertical(doc: &mut Document, lines: isize, extend: bool, tab_size: usize) {
    let last_line = doc.len_lines() - 1;
    let mut sels = doc.selections.clone();
    for s in &mut sels {
        let line = doc.char_to_line(s.head);
        let goal = s.goal.unwrap_or_else(|| doc.column_of(s.head, tab_size));
        let target = line as isize + lines;
        let head = if target < 0 {
            0
        } else if target as usize > last_line {
            doc.len_chars()
        } else {
            doc.pos_at_column(target as usize, goal, tab_size)
        };
        let anchor = if extend { s.anchor } else { head };
        *s = Selection {
            anchor,
            head,
            goal: Some(goal),
        };
    }
    doc.selections = sels;
    doc.normalize_selections();
    doc.view.scroll_to_cursor = true;
}

/// Smart home: toggles between the first non-blank character and column 0.
pub fn move_home(doc: &mut Document, extend: bool) {
    let mut sels = doc.selections.clone();
    for s in &mut sels {
        let line = doc.char_to_line(s.head);
        let start = doc.line_to_char(line);
        let text = doc.line_text(line);
        let indent = start + leading_whitespace(&text).chars().count();
        let head = if s.head == indent { start } else { indent };
        *s = if extend {
            Selection::new(s.anchor, head)
        } else {
            Selection::cursor(head)
        };
    }
    doc.selections = sels;
    doc.normalize_selections();
    doc.view.scroll_to_cursor = true;
}

pub fn move_end(doc: &mut Document, extend: bool) {
    let mut sels = doc.selections.clone();
    for s in &mut sels {
        let head = doc.line_end(doc.char_to_line(s.head));
        *s = if extend {
            Selection::new(s.anchor, head)
        } else {
            Selection::cursor(head)
        };
    }
    doc.selections = sels;
    doc.normalize_selections();
    doc.view.scroll_to_cursor = true;
}

pub fn move_to(doc: &mut Document, pos: usize, extend: bool) {
    let anchor = if extend {
        doc.primary_selection().anchor
    } else {
        pos
    };
    doc.set_selection(Selection::new(anchor, pos));
}

pub fn select_all(doc: &mut Document) {
    let len = doc.len_chars();
    doc.selections = vec![Selection::new(0, len)];
    doc.primary = 0;
}

/// Expands each selection to whole lines (repeated use extends downwards).
pub fn select_line(doc: &mut Document) {
    let mut sels = doc.selections.clone();
    for s in &mut sels {
        let r = s.range();
        let first = doc.char_to_line(r.start);
        let last = doc.char_to_line(r.end);
        let start = doc.line_to_char(first);
        let end_line = if !s.is_empty() && r.end == doc.line_to_char(last) && last > first {
            last
        } else {
            last + 1
        };
        let end = doc.line_to_char(end_line);
        *s = Selection::new(start, end);
    }
    doc.selections = sels;
    doc.normalize_selections();
    doc.view.scroll_to_cursor = true;
}

// ---- insertion -------------------------------------------------------------------

fn closing_pair(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

/// Inserts typed text at every selection, applying auto-indent, bracket
/// pairing and HTML tag completion.
pub fn type_text(doc: &mut Document, text: &str, settings: &EditorSettings) {
    if text.is_empty() {
        return;
    }
    if text == "\n" {
        insert_newline(doc, settings);
        return;
    }
    let mut chars = text.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };

    if let Some(c) = single {
        // Overtype a closing character that was auto-inserted.
        if matches!(c, ')' | ']' | '}' | '"' | '\'' | '`')
            && settings.auto_pair
            && doc
                .selections
                .iter()
                .all(|s| s.is_empty() && doc.char_at(s.head) == Some(c))
        {
            move_horizontal(doc, true, false, false);
            doc.break_undo_group();
            return;
        }

        if c == '>' && settings.auto_close_tags && doc.language.is_markup() {
            insert_with_cursor(doc, EditKind::Typing, |doc, sel| {
                if !sel.is_empty() {
                    return (">".into(), 1);
                }
                match open_tag_before(doc, sel.head) {
                    Some(name) => (format!("></{name}>"), 1),
                    None => (">".into(), 1),
                }
            });
            return;
        }

        if c == '/' && settings.auto_close_tags && doc.language.is_markup() {
            let completes = doc
                .selections
                .iter()
                .all(|s| s.is_empty() && s.head > 0 && doc.char_at(s.head - 1) == Some('<'));
            if completes {
                insert_with_cursor(doc, EditKind::Typing, |doc, sel| {
                    match unclosed_tag_before(doc, sel.head - 1) {
                        Some(name) => {
                            let t = format!("/{name}>");
                            let n = t.chars().count();
                            (t, n)
                        }
                        None => ("/".into(), 1),
                    }
                });
                return;
            }
        }

        if let Some(close) = closing_pair(c).filter(|_| settings.auto_pair) {
            insert_with_cursor(doc, EditKind::Typing, |doc, sel| {
                if !sel.is_empty() {
                    // Wrap the selection.
                    let inner = doc.slice(sel.range());
                    let n = inner.chars().count() + 2;
                    return (format!("{c}{inner}{close}"), n);
                }
                let next = doc.char_at(sel.head);
                let prev = sel.head.checked_sub(1).and_then(|p| doc.char_at(p));
                let next_ok = next.is_none_or(|n| n.is_whitespace() || ")]}>,;:".contains(n));
                let is_quote = c == close;
                let prev_ok = !is_quote || prev.is_none_or(|p| !is_word_char(p) && p != '\\');
                if next_ok && prev_ok {
                    (format!("{c}{close}"), 1)
                } else {
                    (c.to_string(), 1)
                }
            });
            return;
        }

        if matches!(c, '}' | ']' | ')') {
            // Outdent a closing bracket typed on an otherwise blank line.
            insert_with_cursor_range(doc, EditKind::Typing, |doc, sel| {
                let line = doc.char_to_line(sel.head);
                let start = doc.line_to_char(line);
                let before = doc.slice(start..sel.head);
                if sel.is_empty() && !before.is_empty() && before.trim().is_empty() {
                    let unit = indent_unit(settings);
                    let new_indent = strip_one_indent(&before, &unit, settings.tab_size);
                    let n = new_indent.chars().count() + 1;
                    (start..sel.head, format!("{new_indent}{c}"), n)
                } else {
                    (sel.range(), c.to_string(), 1)
                }
            });
            return;
        }
    }

    let kind = if text.chars().count() == 1 {
        EditKind::Typing
    } else {
        EditKind::Other
    };
    doc.replace_selections(kind, |_| text.to_string());
}

fn strip_one_indent(indent: &str, unit: &str, tab_size: usize) -> String {
    if let Some(stripped) = indent.strip_suffix(unit) {
        return stripped.to_string();
    }
    if let Some(stripped) = indent.strip_suffix('\t') {
        return stripped.to_string();
    }
    let trimmed = indent.trim_end_matches(' ');
    let spaces = indent.len() - trimmed.len();
    let remove = spaces.min(tab_size).max(1).min(indent.len());
    indent[..indent.len() - remove].to_string()
}

/// Inserts text produced per selection; the cursor ends up `offset` chars
/// into the inserted text.
fn insert_with_cursor(
    doc: &mut Document,
    kind: EditKind,
    mut f: impl FnMut(&Document, &Selection) -> (String, usize),
) {
    insert_with_cursor_range(doc, kind, |doc, sel| {
        let (text, offset) = f(doc, sel);
        (sel.range(), text, offset)
    });
}

fn insert_with_cursor_range(
    doc: &mut Document,
    kind: EditKind,
    mut f: impl FnMut(&Document, &Selection) -> (std::ops::Range<usize>, String, usize),
) {
    let mut sels = doc.selections.clone();
    sels.sort_by_key(|s| s.range().start);
    let mut changes = Vec::with_capacity(sels.len());
    let mut after = Vec::with_capacity(sels.len());
    let mut delta: isize = 0;
    for sel in &sels {
        let (range, text, offset) = f(doc, sel);
        let new_start = (range.start as isize + delta) as usize;
        after.push(Selection::cursor(new_start + offset));
        delta += text.chars().count() as isize - range.len() as isize;
        changes.push(Change::new(range, text));
    }
    doc.apply(changes, Some(after), kind);
}

fn insert_newline(doc: &mut Document, settings: &EditorSettings) {
    let unit = indent_unit(settings);
    insert_with_cursor_range(doc, EditKind::Other, |doc, sel| {
        let range = sel.range();
        let line = doc.char_to_line(range.start);
        let line_start = doc.line_to_char(line);
        let line_text = doc.line_text(line);
        let indent = leading_whitespace(&line_text).to_string();
        let before: String = doc.slice(line_start..range.start);
        let prev = before.trim_end().chars().last();
        let next = doc.char_at(range.end);

        let opens = matches!(prev, Some('{' | '[' | '('));
        let between_pair = matches!((prev, next), (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')')));
        let between_tags = doc.language.is_markup()
            && before.trim_end().ends_with('>')
            && next == Some('<')
            && doc.char_at(range.end + 1) == Some('/')
            && last_tag_is_open(&before);

        // Strip trailing whitespace before the cursor so lines don't keep it.
        let trim_from = line_start + before.trim_end_matches([' ', '\t']).chars().count();
        let replace = if trim_from < range.start && before.trim().is_empty() {
            range.clone()
        } else {
            trim_from.min(range.start)..range.end
        };

        if between_pair || between_tags {
            let text = format!("\n{indent}{unit}\n{indent}");
            let offset = 1 + indent.chars().count() + unit.chars().count();
            (replace, text, offset)
        } else if opens || (doc.language.is_markup() && last_tag_is_open(&before)) {
            let text = format!("\n{indent}{unit}");
            let n = text.chars().count();
            (replace, text, n)
        } else {
            let text = format!("\n{indent}");
            let n = text.chars().count();
            (replace, text, n)
        }
    });
}

/// True when the text ends in a start tag that is not void or self-closing.
fn last_tag_is_open(before: &str) -> bool {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<([A-Za-z][\w:-]*)(?:\s[^<>]*)?>\s*$").unwrap());
    RE.captures(before).is_some_and(|c| {
        let name = c[1].to_ascii_lowercase();
        !before.trim_end().ends_with("/>") && !VOID_ELEMENTS.contains(&name.as_str())
    })
}

/// If the text directly before `pos` is an unfinished start tag (`<div class="x"`),
/// returns its name.
fn open_tag_before(doc: &Document, pos: usize) -> Option<String> {
    let line_start = doc.line_to_char(doc.char_to_line(pos));
    let from = pos.saturating_sub(2000).max(line_start.saturating_sub(2000));
    let before = doc.slice(from..pos);
    let lt = before.rfind('<')?;
    let tag = &before[lt + 1..];
    if tag.contains('>') || tag.ends_with('/') {
        return None;
    }
    // Ignore '>' inside quotes: count quotes.
    if tag.matches('"').count() % 2 == 1 || tag.matches('\'').count() % 2 == 1 {
        return None;
    }
    let name: String = tag
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | ':' | '_'))
        .collect();
    let first = name.chars().next()?;
    if !first.is_alphabetic() || VOID_ELEMENTS.contains(&name.to_ascii_lowercase().as_str()) {
        return None;
    }
    Some(name)
}

/// Finds the innermost element still open before `pos`.
fn unclosed_tag_before(doc: &Document, pos: usize) -> Option<String> {
    static TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<!--.*?-->|<(/?)([A-Za-z][\w:-]*)(?:[^<>"']|"[^"]*"|'[^']*')*?(/?)>"#)
            .unwrap()
    });
    let from = pos.saturating_sub(200_000);
    let text = doc.slice(from..pos);
    let mut stack: Vec<String> = Vec::new();
    for cap in TAG.captures_iter(&text) {
        let Some(name) = cap.get(2) else { continue };
        let name = name.as_str();
        let lower = name.to_ascii_lowercase();
        if &cap[1] == "/" {
            if let Some(i) = stack.iter().rposition(|n| n.eq_ignore_ascii_case(name)) {
                stack.truncate(i);
            }
        } else if &cap[3] != "/" && !VOID_ELEMENTS.contains(&lower.as_str()) {
            stack.push(name.to_string());
        }
    }
    stack.pop()
}

// ---- deletion -------------------------------------------------------------------

pub fn backspace(doc: &mut Document, word: bool, settings: &EditorSettings) {
    let changes: Vec<Change> = doc
        .selections
        .iter()
        .map(|s| {
            if !s.is_empty() {
                return Change::new(s.range(), "");
            }
            let pos = s.head;
            if pos == 0 {
                return Change::new(0..0, "");
            }
            if word {
                return Change::new(word_boundary_left(doc, pos)..pos, "");
            }
            let prev = doc.rope.char(pos - 1);
            // Remove an empty auto-pair together.
            if settings.auto_pair
                && let (Some(close), Some(next)) = (closing_pair(prev), doc.char_at(pos))
                && close == next
            {
                return Change::new(pos - 1..pos + 1, "");
            }
            // Within leading spaces, delete back to the previous tab stop.
            let line_start = doc.line_to_char(doc.char_to_line(pos));
            let before = doc.slice(line_start..pos);
            if settings.use_spaces && !before.is_empty() && before.chars().all(|c| c == ' ') {
                let col = before.chars().count();
                let target = (col - 1) / settings.tab_size * settings.tab_size;
                return Change::new(line_start + target..pos, "");
            }
            Change::new(pos - 1..pos, "")
        })
        .collect();
    let changes = dedup_overlapping(changes);
    doc.apply(changes, None, EditKind::Deleting);
}

pub fn delete_forward(doc: &mut Document, word: bool) {
    let len = doc.len_chars();
    let changes: Vec<Change> = doc
        .selections
        .iter()
        .map(|s| {
            if !s.is_empty() {
                Change::new(s.range(), "")
            } else if word {
                Change::new(s.head..word_boundary_right(doc, s.head), "")
            } else {
                Change::new(s.head..(s.head + 1).min(len), "")
            }
        })
        .collect();
    let changes = dedup_overlapping(changes);
    doc.apply(changes, None, EditKind::Deleting);
}

fn dedup_overlapping(mut changes: Vec<Change>) -> Vec<Change> {
    changes.sort_by_key(|c| c.range.start);
    let mut out: Vec<Change> = Vec::with_capacity(changes.len());
    for c in changes {
        if let Some(last) = out.last_mut()
            && c.range.start < last.range.end
        {
            last.range.end = last.range.end.max(c.range.end);
            continue;
        }
        out.push(c);
    }
    out
}

// ---- line operations ------------------------------------------------------------

/// Distinct line indices touched by the selections, in ascending order.
fn selected_lines(doc: &Document) -> Vec<usize> {
    let mut lines = Vec::new();
    for s in &doc.selections {
        let r = s.range();
        let first = doc.char_to_line(r.start);
        let mut last = doc.char_to_line(r.end);
        if last > first && r.end == doc.line_to_char(last) {
            last -= 1;
        }
        lines.extend(first..=last);
    }
    lines.sort_unstable();
    lines.dedup();
    lines
}

pub fn indent(doc: &mut Document, settings: &EditorSettings) {
    let multi_line = doc.selections.iter().any(|s| {
        let r = s.range();
        doc.char_to_line(r.start) != doc.char_to_line(r.end)
    });
    if !multi_line {
        // Insert indentation at the cursor, aligned to the next tab stop.
        insert_with_cursor(doc, EditKind::Typing, |doc, sel| {
            if !settings.use_spaces {
                return ("\t".into(), 1);
            }
            let col = doc.column_of(sel.range().start, settings.tab_size);
            let n = settings.tab_size - col % settings.tab_size;
            (" ".repeat(n), n)
        });
        return;
    }
    let unit = indent_unit(settings);
    let changes = selected_lines(doc)
        .into_iter()
        .filter(|&l| doc.line_len(l) > 0)
        .map(|l| {
            let p = doc.line_to_char(l);
            Change::new(p..p, unit.clone())
        })
        .collect();
    doc.apply(changes, None, EditKind::Other);
}

pub fn outdent(doc: &mut Document, settings: &EditorSettings) {
    let changes = selected_lines(doc)
        .into_iter()
        .filter_map(|l| {
            let text = doc.line_text(l);
            let p = doc.line_to_char(l);
            let n = if text.starts_with('\t') {
                1
            } else {
                text.chars()
                    .take(settings.tab_size)
                    .take_while(|&c| c == ' ')
                    .count()
            };
            (n > 0).then(|| Change::new(p..p + n, ""))
        })
        .collect();
    doc.apply(changes, None, EditKind::Other);
}

pub fn toggle_comment(doc: &mut Document) {
    let lang = doc.language;
    let lines = selected_lines(doc);
    if lines.is_empty() {
        return;
    }
    if let Some(token) = lang.line_comment() {
        let texts: Vec<String> = lines.iter().map(|&l| doc.line_text(l)).collect();
        let non_blank: Vec<&String> = texts.iter().filter(|t| !t.trim().is_empty()).collect();
        let all_commented =
            !non_blank.is_empty() && non_blank.iter().all(|t| t.trim_start().starts_with(token));
        let min_indent = non_blank
            .iter()
            .map(|t| leading_whitespace(t).chars().count())
            .min()
            .unwrap_or(0);
        let mut changes = Vec::new();
        for (&l, text) in lines.iter().zip(&texts) {
            if text.trim().is_empty() {
                continue;
            }
            let start = doc.line_to_char(l);
            if all_commented {
                let ws = leading_whitespace(text).chars().count();
                let rest = &text[leading_whitespace(text).len()..];
                let mut n = token.chars().count();
                if rest[token.len()..].starts_with(' ') {
                    n += 1;
                }
                changes.push(Change::new(start + ws..start + ws + n, ""));
            } else {
                let p = start + min_indent;
                changes.push(Change::new(p..p, format!("{token} ")));
            }
        }
        doc.apply(changes, None, EditKind::Other);
        return;
    }
    let Some((open, close)) = lang.block_comment() else {
        return;
    };
    // Block comment around the selected lines' content.
    let first = lines[0];
    let last = *lines.last().unwrap();
    let start = doc.line_to_char(first);
    let end = doc.line_end(last);
    let text = doc.slice(start..end);
    let trimmed = text.trim();
    if trimmed.starts_with(open) && trimmed.ends_with(close) && trimmed.len() >= open.len() + close.len() {
        let lead = text.len() - text.trim_start().len();
        let s = start + text[..lead].chars().count();
        let inner = &trimmed[open.len()..trimmed.len() - close.len()];
        let inner = inner.strip_prefix(' ').unwrap_or(inner);
        let inner = inner.strip_suffix(' ').unwrap_or(inner);
        let e = s + trimmed.chars().count();
        doc.apply(vec![Change::new(s..e, inner)], None, EditKind::Other);
    } else {
        let lead = leading_whitespace(&text);
        let s = start + lead.chars().count();
        let body = &text[lead.len()..];
        let e = s + body.chars().count();
        doc.apply(
            vec![Change::new(s..e, format!("{open} {body} {close}"))],
            None,
            EditKind::Other,
        );
    }
}

pub fn duplicate_lines(doc: &mut Document) {
    let mut changes = Vec::new();
    for s in &doc.selections {
        let r = s.range();
        let first = doc.char_to_line(r.start);
        let last = doc.char_to_line(r.end);
        let start = doc.line_to_char(first);
        let end = doc.line_end(last);
        let text = doc.slice(start..end);
        changes.push(Change::new(end..end, format!("\n{text}")));
    }
    let changes = dedup_overlapping(changes);
    let before = doc.selections.clone();
    doc.apply(changes, None, EditKind::Other);
    // Selections stay on the original lines; move them to the copies.
    doc.selections = before
        .iter()
        .map(|s| {
            let r = s.range();
            let first = doc.char_to_line(r.start);
            let last = doc.char_to_line(r.end);
            let shift = doc.line_to_char(last + 1) - doc.line_to_char(first);
            Selection::new(s.anchor + shift, s.head + shift)
        })
        .collect();
    doc.normalize_selections();
}

pub fn delete_lines(doc: &mut Document) {
    let lines = selected_lines(doc);
    let total = doc.len_lines();
    let changes = group_runs(&lines)
        .into_iter()
        .map(|(first, last)| {
            let start = doc.line_to_char(first);
            if last + 1 < total {
                Change::new(start..doc.line_to_char(last + 1), "")
            } else if first > 0 {
                // Last line: remove the preceding newline instead.
                Change::new(start - 1..doc.len_chars(), "")
            } else {
                Change::new(0..doc.len_chars(), "")
            }
        })
        .collect();
    doc.apply(changes, None, EditKind::Other);
}

fn group_runs(lines: &[usize]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &l in lines {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == l => *last = l,
            _ => runs.push((l, l)),
        }
    }
    runs
}

pub fn move_lines(doc: &mut Document, up: bool) {
    let lines = selected_lines(doc);
    let runs = group_runs(&lines);
    let total = doc.len_lines();
    // Ensure the document ends with a line we can swap with.
    if (up && runs.first().is_some_and(|r| r.0 == 0))
        || (!up && runs.last().is_some_and(|r| r.1 + 1 >= total))
    {
        return;
    }
    let mut changes = Vec::new();
    let mut shift: Vec<(usize, usize, isize)> = Vec::new();
    for (first, last) in runs {
        let block_start = doc.line_to_char(first);
        let block_end = doc.line_end(last);
        let block = doc.slice(block_start..block_end);
        if up {
            let prev_start = doc.line_to_char(first - 1);
            let prev = doc.slice(prev_start..doc.line_end(first - 1));
            changes.push(Change::new(prev_start..block_end, format!("{block}\n{prev}")));
            let d = -(prev.chars().count() as isize + 1);
            shift.push((block_start, block_end, d));
        } else {
            let next_end = doc.line_end(last + 1);
            let next = doc.slice(doc.line_to_char(last + 1)..next_end);
            changes.push(Change::new(block_start..next_end, format!("{next}\n{block}")));
            let d = next.chars().count() as isize + 1;
            shift.push((block_start, block_end, d));
        }
    }
    let after: Vec<Selection> = doc
        .selections
        .iter()
        .map(|s| {
            let d = shift
                .iter()
                .find(|(a, b, _)| s.range().start >= *a && s.range().start <= *b)
                .map_or(0, |x| x.2);
            Selection::new(
                (s.anchor as isize + d) as usize,
                (s.head as isize + d) as usize,
            )
        })
        .collect();
    doc.apply(changes, Some(after), EditKind::Other);
}

// ---- multi-cursor ---------------------------------------------------------------

/// Ctrl+D: select the word under the cursor, or add the next occurrence of the
/// primary selection.
pub fn select_next_occurrence(doc: &mut Document) {
    let primary = doc.primary_selection();
    if primary.is_empty() {
        if let Some(r) = word_at(doc, primary.head) {
            doc.selections[doc.primary] = Selection::new(r.start, r.end);
        }
        return;
    }
    let needle = doc.slice(primary.range());
    let text = doc.text();
    let last_end = doc
        .selections
        .iter()
        .map(|s| s.range().end)
        .max()
        .unwrap_or(0);
    let byte_from = doc.rope.char_to_byte(last_end);
    let found = text[byte_from..]
        .find(&needle)
        .map(|i| i + byte_from)
        .or_else(|| text.find(&needle));
    if let Some(byte) = found {
        let start = doc.rope.byte_to_char(byte);
        let end = start + needle.chars().count();
        if doc.selections.iter().any(|s| s.range() == (start..end)) {
            return;
        }
        doc.selections.push(Selection::new(start, end));
        doc.primary = doc.selections.len() - 1;
        let primary = doc.selections[doc.primary];
        doc.normalize_selections();
        doc.primary = doc
            .selections
            .iter()
            .position(|s| *s == primary)
            .unwrap_or(doc.primary);
        doc.view.scroll_to_cursor = true;
    }
}

/// Selects every occurrence of the primary selection (or word).
pub fn select_all_occurrences(doc: &mut Document) {
    let primary = doc.primary_selection();
    let range = if primary.is_empty() {
        match word_at(doc, primary.head) {
            Some(r) => r,
            None => return,
        }
    } else {
        primary.range()
    };
    let needle = doc.slice(range);
    if needle.is_empty() {
        return;
    }
    let text = doc.text();
    let n = needle.chars().count();
    let sels: Vec<Selection> = text
        .match_indices(&needle)
        .map(|(b, _)| {
            let s = doc.rope.byte_to_char(b);
            Selection::new(s, s + n)
        })
        .collect();
    if !sels.is_empty() {
        doc.selections = sels;
        doc.primary = 0;
        doc.normalize_selections();
    }
}

pub fn add_cursor_vertical(doc: &mut Document, down: bool, tab_size: usize) {
    let edge = if down {
        doc.selections.iter().max_by_key(|s| s.head)
    } else {
        doc.selections.iter().min_by_key(|s| s.head)
    }
    .copied()
    .unwrap();
    let line = doc.char_to_line(edge.head);
    let target = if down {
        line + 1
    } else if line == 0 {
        return;
    } else {
        line - 1
    };
    if target >= doc.len_lines() {
        return;
    }
    let goal = edge.goal.unwrap_or_else(|| doc.column_of(edge.head, tab_size));
    let pos = doc.pos_at_column(target, goal, tab_size);
    doc.selections.push(Selection {
        anchor: pos,
        head: pos,
        goal: Some(goal),
    });
    doc.primary = doc.selections.len() - 1;
    doc.normalize_selections();
    doc.view.scroll_to_cursor = true;
}

/// Collapses to the primary selection; returns false when there was nothing to collapse.
pub fn collapse_selections(doc: &mut Document) -> bool {
    if doc.selections.len() > 1 {
        let p = doc.primary_selection();
        doc.set_selection(p);
        true
    } else if !doc.selections[0].is_empty() {
        let head = doc.selections[0].head;
        doc.set_selection(Selection::cursor(head));
        true
    } else {
        false
    }
}

// ---- clipboard ------------------------------------------------------------------

/// Text to copy. With empty selections the whole lines are copied.
pub fn copy_text(doc: &Document) -> (String, bool) {
    if doc.selections.iter().all(|s| s.is_empty()) {
        let mut lines = Vec::new();
        for s in &doc.selections {
            let l = doc.char_to_line(s.head);
            lines.push(doc.line_text(l));
        }
        lines.dedup();
        (lines.join("\n") + "\n", true)
    } else {
        let parts: Vec<String> = doc
            .selections
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| doc.slice(s.range()))
            .collect();
        (parts.join("\n"), false)
    }
}

pub fn cut(doc: &mut Document) -> String {
    let (text, whole_lines) = copy_text(doc);
    if whole_lines {
        delete_lines(doc);
    } else {
        doc.replace_selections(EditKind::Other, |_| String::new());
    }
    text
}

pub fn paste(doc: &mut Document, text: &str) {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let parts: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    if doc.selections.len() > 1 && parts.len() == doc.selections.len() {
        let mut i = 0;
        doc.replace_selections(EditKind::Other, |_| {
            let p = parts[i].to_string();
            i += 1;
            p
        });
    } else {
        doc.replace_selections(EditKind::Other, |_| text.clone());
    }
}

// ---- brackets -------------------------------------------------------------------

/// Finds the bracket matching the one at or before `pos`.
pub fn matching_bracket(doc: &Document, pos: usize) -> Option<(usize, usize)> {
    const LIMIT: usize = 50_000;
    let check = |p: usize| -> Option<(usize, usize)> {
        let c = doc.char_at(p)?;
        let (open, close, forward) = match c {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => return None,
        };
        let mut depth = 0usize;
        if forward {
            let len = doc.len_chars();
            let end = (p + LIMIT).min(len);
            for (i, ch) in doc.rope.slice(p..end).chars().enumerate() {
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some((p, p + i));
                    }
                }
            }
        } else {
            let start = p.saturating_sub(LIMIT);
            let mut i = p + 1;
            for ch in doc.rope.slice(start..p + 1).chars().reversed() {
                i -= 1;
                if ch == close {
                    depth += 1;
                } else if ch == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some((i, p));
                    }
                }
            }
        }
        None
    };
    check(pos).or_else(|| pos.checked_sub(1).and_then(check))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::DocSource;
    use std::path::PathBuf;

    fn html(text: &str, cursor: usize) -> Document {
        let mut d = Document::new(DocSource::Local(PathBuf::from("a.html")), text);
        d.set_selection(Selection::cursor(cursor));
        d
    }

    #[test]
    fn auto_closes_tags() {
        let s = EditorSettings::default();
        let mut d = html("<div class=\"a\"", 14);
        type_text(&mut d, ">", &s);
        assert_eq!(d.rope.to_string(), "<div class=\"a\"></div>");
        assert_eq!(d.primary_selection().head, 15);

        let mut d = html("<br", 3);
        type_text(&mut d, ">", &s);
        assert_eq!(d.rope.to_string(), "<br>");
    }

    #[test]
    fn completes_closing_tag() {
        let s = EditorSettings::default();
        let mut d = html("<ul><li>x</li><", 15);
        type_text(&mut d, "/", &s);
        assert_eq!(d.rope.to_string(), "<ul><li>x</li></ul>");
    }

    #[test]
    fn newline_between_tags_indents() {
        let s = EditorSettings::default();
        let mut d = html("<div></div>", 5);
        type_text(&mut d, "\n", &s);
        assert_eq!(d.rope.to_string(), "<div>\n    \n</div>");
        assert_eq!(d.primary_selection().head, 10);
    }

    #[test]
    fn move_line_down_and_up() {
        let mut d = html("a\nb\nc", 0);
        move_lines(&mut d, false);
        assert_eq!(d.rope.to_string(), "b\na\nc");
        assert_eq!(d.primary_selection().head, 2);
        move_lines(&mut d, true);
        assert_eq!(d.rope.to_string(), "a\nb\nc");
    }

    #[test]
    fn toggles_css_comment() {
        let mut d = Document::new(DocSource::Local(PathBuf::from("a.css")), "  a { b: c; }");
        toggle_comment(&mut d);
        assert_eq!(d.rope.to_string(), "  /* a { b: c; } */");
        toggle_comment(&mut d);
        assert_eq!(d.rope.to_string(), "  a { b: c; }");
    }
}

/// Expands an Emmet abbreviation before the cursor (HTML-like files only).
/// Returns false if there was nothing to expand.
pub fn expand_emmet(doc: &mut Document, settings: &EditorSettings) -> bool {
    use crate::lang::Language;
    if !matches!(doc.language, Language::Html | Language::Php | Language::Xml)
        || doc.selections.len() != 1
        || !doc.selections[0].is_empty()
    {
        return false;
    }
    let pos = doc.selections[0].head;
    // Only at the end of a word, not in the middle of one.
    if doc.char_at(pos).is_some_and(|c| c.is_alphanumeric()) {
        return false;
    }
    let line = doc.char_to_line(pos);
    let line_start = doc.line_to_char(line);
    let before = doc.slice(line_start..pos);
    if doc.language == Language::Php {
        let lower = before.to_ascii_lowercase();
        if lower.rfind("<?") > lower.rfind("?>") {
            return false; // inside PHP code
        }
    }
    let Some(abbr) = super::emmet::abbreviation_before(&before) else { return false };
    let unit = indent_unit(settings);
    let Some(expansion) = super::emmet::expand(abbr, &unit) else { return false };
    let indent = leading_whitespace(&before).to_string();
    let text = expansion.replace('\n', &format!("\n{indent}"));
    let start = pos - abbr.chars().count();
    let (text, cursor) = match text.find("$0") {
        Some(i) => (text.replacen("$0", "", 1), start + text[..i].chars().count()),
        None => {
            let n = text.chars().count();
            (text, start + n)
        }
    };
    doc.apply(vec![Change::new(start..pos, text)], Some(vec![Selection::cursor(cursor)]), EditKind::Other);
    true
}

/// Applies line-wise changes as one undo step, keeping cursors on their lines.
fn apply_keeping_cursors(doc: &mut Document, changes: Vec<Change>) {
    if changes.is_empty() {
        return;
    }
    let map = |pos: usize| {
        let mut shift: isize = 0;
        for c in &changes {
            let new_len = c.text.chars().count();
            if pos < c.range.start {
                break;
            }
            if pos < c.range.end {
                return (c.range.start as isize + shift) as usize + (pos - c.range.start).min(new_len);
            }
            shift += new_len as isize - c.range.len() as isize;
        }
        (pos as isize + shift) as usize
    };
    let after = doc.selections.iter().map(|s| Selection { anchor: map(s.anchor), head: map(s.head), goal: None }).collect();
    doc.break_undo_group();
    doc.apply(changes, Some(after), EditKind::Other);
    doc.break_undo_group();
}

/// Removes spaces and tabs at line ends and/or ends the text with a line break.
pub fn tidy_whitespace(doc: &mut Document, trim: bool, final_newline: bool) {
    let mut changes = Vec::new();
    if trim {
        for line in 0..doc.len_lines() {
            let start = doc.line_to_char(line);
            let len = doc.line_len(line);
            let slice = doc.slice(start..start + len);
            let kept = len - slice.chars().rev().take_while(|c| *c == ' ' || *c == '\t').count();
            if kept < len {
                changes.push(Change::new(start + kept..start + len, ""));
            }
        }
    }
    let n = doc.len_chars();
    if final_newline && n > 0 && doc.char_at(n - 1) != Some('\n') {
        changes.push(Change::new(n..n, "\n"));
    }
    apply_keeping_cursors(doc, changes);
}

/// Rewrites the indentation of all lines with spaces or with tabs.
pub fn convert_indentation(doc: &mut Document, to_spaces: bool, tab_size: usize) {
    let tab_size = tab_size.max(1);
    let mut changes = Vec::new();
    for line in 0..doc.len_lines() {
        let start = doc.line_to_char(line);
        let indent: String = doc.rope.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if indent.is_empty() {
            continue;
        }
        let cols = indent.chars().fold(0, |col, c| if c == '\t' { col / tab_size * tab_size + tab_size } else { col + 1 });
        let new = if to_spaces {
            " ".repeat(cols)
        } else {
            format!("{}{}", "\t".repeat(cols / tab_size), " ".repeat(cols % tab_size))
        };
        if new != indent {
            changes.push(Change::new(start..start + indent.chars().count(), new));
        }
    }
    apply_keeping_cursors(doc, changes);
}

#[cfg(test)]
mod whitespace_tests {
    use super::*;
    use crate::editor::DocSource;

    #[test]
    fn tidy_and_convert() {
        let mut d = Document::new(DocSource::Untitled(1), "a  \n\tb\t\nc");
        d.set_selection(Selection::cursor(3)); // after "a "
        tidy_whitespace(&mut d, true, true);
        assert_eq!(d.rope.to_string(), "a\n\tb\nc\n");
        assert_eq!(d.primary_selection().head, 1);
        convert_indentation(&mut d, true, 4);
        assert_eq!(d.rope.to_string(), "a\n    b\nc\n");
        convert_indentation(&mut d, false, 4);
        assert_eq!(d.rope.to_string(), "a\n\tb\nc\n");
        d.undo();
        assert_eq!(d.rope.to_string(), "a\n    b\nc\n");
    }
}
