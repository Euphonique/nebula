//! Matching HTML/XML tags: the opening and closing tag name belonging
//! together, for highlighting and for renaming both at once.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::Document;
use crate::lang::Language;

/// How far around the cursor tags are searched (chars).
const WINDOW: usize = 200_000;

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
];

pub fn supported(language: Language) -> bool {
    matches!(language, Language::Html | Language::Php | Language::Xml | Language::Markdown)
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

struct Tag {
    /// Name range in chars, relative to the window.
    name: Range<usize>,
    name_text: String,
    closing: bool,
    self_closing: bool,
}

/// Tags in `text` (comments skipped), with char positions.
fn tags(text: &str) -> Vec<Tag> {
    static TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<!--.*?-->|<(/?)([A-Za-z][\w:.-]*)((?:[^<>"']|"[^"]*"|'[^']*')*?)(/?)>"#).unwrap()
    });
    let mut out = Vec::new();
    let (mut byte, mut chars) = (0usize, 0usize);
    for c in TAG.captures_iter(text) {
        let Some(name) = c.get(2) else { continue };
        chars += text[byte..name.start()].chars().count();
        byte = name.start();
        let len = name.as_str().chars().count();
        out.push(Tag {
            name: chars..chars + len,
            name_text: name.as_str().to_ascii_lowercase(),
            closing: &c[1] == "/",
            self_closing: &c[4] == "/" || VOID.contains(&name.as_str().to_ascii_lowercase().as_str()),
        });
    }
    out
}

/// Name ranges (chars) of the tag at `pos` and its partner: (opening, closing).
pub fn pair_at(doc: &Document, pos: usize) -> Option<(Range<usize>, Range<usize>)> {
    // Quick check: the cursor touches a tag name right after "<" or "</".
    let line = doc.char_to_line(pos);
    let line_start = doc.line_to_char(line);
    let before = doc.slice(line_start..pos);
    let lt = before.rfind('<')?;
    let after_lt = before[lt + 1..].trim_start_matches('/');
    if !after_lt.chars().all(is_name_char) || after_lt.is_empty() && !doc.char_at(pos).is_some_and(|c| c.is_alphabetic()) {
        return None;
    }

    let start = pos.saturating_sub(WINDOW);
    let end = (pos + WINDOW).min(doc.len_chars());
    let text = doc.slice(start..end);
    let tags = tags(&text);
    let rel = pos - start;
    let i = tags.iter().position(|t| t.name.start <= rel && rel <= t.name.end)?;
    let tag = &tags[i];
    if tag.self_closing {
        return None;
    }
    let abs = |r: &Range<usize>| r.start + start..r.end + start;
    let mut depth = 0i32;
    if !tag.closing {
        for t in &tags[i + 1..] {
            if t.name_text != tag.name_text || t.self_closing {
                continue;
            }
            if t.closing {
                if depth == 0 {
                    return Some((abs(&tag.name), abs(&t.name)));
                }
                depth -= 1;
            } else {
                depth += 1;
            }
        }
    } else {
        for t in tags[..i].iter().rev() {
            if t.name_text != tag.name_text || t.self_closing {
                continue;
            }
            if !t.closing {
                if depth == 0 {
                    return Some((abs(&t.name), abs(&tag.name)));
                }
                depth -= 1;
            } else {
                depth += 1;
            }
        }
    }
    None
}

/// After an edit inside one tag name: the change that makes the partner tag
/// name equal again. `pair` is the pair found before the edit, `pos` the cursor
/// before it and `delta` the change in document length.
pub fn sync_partner(doc: &Document, pair: &(Range<usize>, Range<usize>), pos: usize, delta: isize) -> Option<super::document::Change> {
    let (open, close) = pair;
    let editing_open = open.start <= pos && pos <= open.end;
    let (edited, other) = if editing_open { (open, close) } else { (close, open) };
    // The edited name now runs from its start to the end of the name characters.
    let new_end = (edited.end as isize + delta) as usize;
    if new_end < edited.start || new_end > doc.len_chars() {
        return None;
    }
    let cursor = doc.primary_selection().head;
    if cursor < edited.start || cursor > new_end {
        return None;
    }
    let name = doc.slice(edited.start..new_end);
    if name.is_empty() || !name.chars().all(is_name_char) || doc.char_at(new_end).is_some_and(is_name_char) {
        return None;
    }
    let other = if other.start > edited.start {
        (other.start as isize + delta) as usize..(other.end as isize + delta) as usize
    } else {
        other.clone()
    };
    (doc.slice(other.clone()) != name).then(|| super::document::Change::new(other, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::DocSource;

    #[test]
    fn finds_pairs() {
        let mut d = Document::new(DocSource::Untitled(1), "<div>\n  <div class=\"a\"><br><img src=x/></div>\n  <!-- </div> -->\n</div>\n");
        d.language = Language::Html;
        // Cursor in the first "div".
        let (open, close) = pair_at(&d, 2).unwrap();
        assert_eq!(d.slice(open.clone()), "div");
        assert_eq!(open, 1..4);
        assert_eq!(d.slice(close.clone()), "div");
        assert!(close.start > 60, "outer closing tag, not the commented one");
        // From the inner closing tag back to its opening tag.
        let inner_close = d.text().find("</div>").unwrap() + 3;
        let (o, _) = pair_at(&d, inner_close).unwrap();
        assert_eq!(o.start, 9);
        // Not in a tag name.
        assert!(pair_at(&d, 15).is_none());
    }

    #[test]
    fn renames_partner() {
        let mut d = Document::new(DocSource::Untitled(1), "<div>x</div>");
        let pair = pair_at(&d, 4).unwrap();
        d.set_selection(crate::editor::document::Selection::cursor(4));
        d.apply(vec![crate::editor::document::Change::new(4..4, "x")], None, crate::editor::document::EditKind::Typing);
        let change = sync_partner(&d, &pair, 4, 1).unwrap();
        d.apply(vec![change], None, crate::editor::document::EditKind::Typing);
        assert_eq!(d.rope.to_string(), "<divx>x</divx>");
    }
}
