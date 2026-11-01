//! CSS class names across a project: where a class is written (selectors in
//! CSS and `<style>`, `class` attributes, JavaScript like `classList.add('x')`
//! or `querySelector('.x')`), for renaming it everywhere, and which classes
//! are defined but never used.

use std::collections::BTreeSet;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// CSS, SCSS, Less.
    Css,
    /// HTML, PHP, templates (with `<style>` and `<script>` inside).
    Markup,
    /// JavaScript and TypeScript.
    Js,
    /// JSX/TSX/Vue: markup attributes and JavaScript.
    Mixed,
}

pub fn kind_of(name: &str) -> Option<Kind> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "css" | "scss" | "sass" | "less" => Kind::Css,
        "html" | "htm" | "php" | "phtml" | "twig" | "blade" | "tpl" | "xhtml" => Kind::Markup,
        "js" | "mjs" | "cjs" | "ts" => Kind::Js,
        "jsx" | "tsx" | "vue" | "svelte" | "astro" => Kind::Mixed,
        _ => return None,
    })
}

fn ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c >= 0x80
}

/// Class selectors (`.name`) in CSS text, outside comments and strings:
/// (name, byte range of the name).
fn css_classes(src: &str, base: usize, out: &mut Vec<(String, Range<usize>)>) {
    let b = src.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => i = src[i + 2..].find("*/").map_or(b.len(), |e| i + 4 + e),
            // `//` comments in SCSS/Less (not in `url(//…)`).
            b'/' if b.get(i + 1) == Some(&b'/') && (i == 0 || !matches!(b[i - 1], b'(' | b':')) => i = src[i..].find('\n').map_or(b.len(), |e| i + e),
            q @ (b'"' | b'\'') => {
                let mut j = i + 1;
                while j < b.len() && b[j] != q && b[j] != b'\n' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                i = j + 1;
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'.' => {
                let start = i + 1;
                // In a block, a part that ends with `;` or `}` is a declaration
                // (`background: url(a.png)`); one that ends with `{` a nested selector.
                let selector = depth == 0 || src[i..].find([';', '{', '}']).is_none_or(|e| b[i + e] == b'{');
                let prev_ok = selector && (i == 0 || b[i - 1] != b'.' && !b[i - 1].is_ascii_digit());
                let first_ok = b.get(start).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_' || *c == b'-' || *c >= 0x80)
                    && !(b[start] == b'-' && b.get(start + 1).is_some_and(|c| c.is_ascii_digit()));
                if prev_ok && first_ok {
                    let mut j = start;
                    while j < b.len() && ident_char(b[j]) {
                        j += 1;
                    }
                    out.push((src[start..j].to_string(), base + start..base + j));
                    i = j;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
}

/// Whitespace-separated class names in an attribute value or class list
/// string; parts with template code (`<?`, `{`, `$`) are left alone.
fn tokens(value: &str, base: usize, out: &mut Vec<(String, Range<usize>)>) {
    let mut i = 0;
    for part in value.split(|c: char| c.is_whitespace()) {
        let start = i;
        i += part.len() + 1;
        if part.is_empty() || part.contains(['<', '{', '}', '$', '?', '(', ')']) {
            continue;
        }
        out.push((part.to_string(), base + start..base + start + part.len()));
    }
}

/// `class="…"`, `className="…"`, `:class="…"` attribute values.
fn class_attrs(src: &str, out: &mut Vec<(String, Range<usize>)>) {
    let lower = src.to_ascii_lowercase();
    let b = src.as_bytes();
    let mut from = 0;
    while let Some(off) = lower[from..].find("class") {
        let at = from + off;
        from = at + 5;
        let before = if at == 0 { b' ' } else { b[at - 1] };
        let name_end = if lower[at..].starts_with("classname") { at + 9 } else { at + 5 };
        if !(before.is_ascii_whitespace() || before == b':') {
            continue;
        }
        let rest = &src[name_end..];
        let t = rest.trim_start();
        let Some(t) = t.strip_prefix('=') else { continue };
        let t2 = t.trim_start();
        let Some(q) = t2.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let value_start = src.len() - t2.len() + 1;
        let Some(len) = src[value_start..].find(q) else { continue };
        tokens(&src[value_start..value_start + len], value_start, out);
        from = value_start + len;
    }
}

/// The call a string literal at `pos` is an argument of (`add` for
/// `el.classList.add('x', 'y')`), or the property it is assigned to.
fn call_of(src: &str, pos: usize) -> Option<String> {
    let b = src.as_bytes();
    let mut depth = 0;
    let mut i = pos;
    let stop = pos.saturating_sub(300);
    while i > stop {
        i -= 1;
        match b[i] {
            b')' => depth += 1,
            b'(' if depth > 0 => depth -= 1,
            b'(' => {
                let name_end = src[..i].trim_end().len();
                let name_start = src[..name_end].rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).map_or(0, |p| p + 1);
                return Some(src[name_start..name_end].to_string());
            }
            b';' | b'{' | b'}' => break,
            b'=' if depth == 0 => {
                // `el.className = 'a b'` / `+=`
                let lhs = src[..i].trim_end_matches('+').trim_end();
                let name_start = lhs.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |p| p + 1);
                return Some(format!("={}", &lhs[name_start..]));
            }
            _ => {}
        }
    }
    None
}

/// Class names in JavaScript strings that are class lists or selectors.
fn js_classes(src: &str, base: usize, out: &mut Vec<(String, Range<usize>)>) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => i = src[i..].find('\n').map_or(b.len(), |e| i + e),
            b'/' if b.get(i + 1) == Some(&b'*') => i = src[i + 2..].find("*/").map_or(b.len(), |e| i + 4 + e),
            q @ (b'"' | b'\'' | b'`') => {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && b[j] != q {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let end = j.min(b.len());
                let value = &src[start..end];
                if !value.contains("${") {
                    match call_of(src, i).as_deref() {
                        Some("add" | "remove" | "toggle" | "contains" | "replace" | "addClass" | "removeClass" | "toggleClass" | "hasClass" | "getElementsByClassName" | "=className") => {
                            tokens(value, base + start, out);
                        }
                        Some("querySelector" | "querySelectorAll" | "closest" | "matches" | "$" | "jQuery" | "find" | "children" | "is" | "not" | "filter" | "parents" | "parent" | "siblings" | "next" | "prev" | "on") => {
                            css_classes(value, base + start, out);
                        }
                        _ => {}
                    }
                }
                i = end + 1;
            }
            _ => i += 1,
        }
    }
}

/// Byte ranges of the contents of `<tag …>…</tag>` blocks.
fn blocks(src: &str, tag: &str) -> Vec<Range<usize>> {
    let lower = src.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = lower[from..].find(&format!("<{tag}")) {
        let at = from + off;
        let Some(gt) = lower[at..].find('>') else { break };
        let start = at + gt + 1;
        let end = lower[start..].find(&format!("</{tag}")).map_or(src.len(), |e| start + e);
        out.push(start..end);
        from = end;
    }
    out
}

/// Every class name written in a file: (name, byte range of the name).
pub fn all_names(src: &str, kind: Kind) -> Vec<(String, Range<usize>)> {
    let mut out = Vec::new();
    match kind {
        Kind::Css => css_classes(src, 0, &mut out),
        Kind::Js => js_classes(src, 0, &mut out),
        Kind::Markup | Kind::Mixed => {
            class_attrs(src, &mut out);
            for r in blocks(src, "style") {
                css_classes(&src[r.clone()], r.start, &mut out);
            }
            if kind == Kind::Mixed {
                js_classes(src, 0, &mut out);
            } else {
                for r in blocks(src, "script") {
                    js_classes(&src[r.clone()], r.start, &mut out);
                }
            }
        }
    }
    out.sort_by_key(|(_, r)| r.start);
    out.dedup_by_key(|(_, r)| r.start);
    out
}

/// Where `class` is written in a file (byte ranges of the name).
pub fn occurrences(src: &str, kind: Kind, class: &str) -> Vec<Range<usize>> {
    all_names(src, kind).into_iter().filter(|(n, _)| n == class).map(|(_, r)| r).collect()
}

/// Classes a file defines in selectors (CSS files and `<style>` blocks).
pub fn defined(src: &str, kind: Kind) -> Vec<(String, Range<usize>)> {
    let mut out = Vec::new();
    match kind {
        Kind::Css => css_classes(src, 0, &mut out),
        Kind::Markup | Kind::Mixed => {
            for r in blocks(src, "style") {
                css_classes(&src[r.clone()], r.start, &mut out);
            }
        }
        Kind::Js => {}
    }
    out
}

/// Classes a file uses (attributes and JavaScript; not selectors in CSS).
pub fn used(src: &str, kind: Kind) -> BTreeSet<String> {
    let defined: BTreeSet<usize> = defined(src, kind).into_iter().map(|(_, r)| r.start).collect();
    all_names(src, kind).into_iter().filter(|(_, r)| !defined.contains(&r.start)).map(|(n, _)| n).collect()
}

/// A valid class name for renaming to.
pub fn valid(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty() && (b[0].is_ascii_alphabetic() || b[0] == b'_' || b[0] == b'-' || b[0] >= 0x80) && b.iter().all(|c| ident_char(*c))
}

/// The class name at byte `pos`, if the cursor is on one.
pub fn name_at(src: &str, kind: Kind, pos: usize) -> Option<String> {
    all_names(src, kind).into_iter().find(|(_, r)| r.start <= pos && pos <= r.end).map(|(n, _)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(src: &'a str, kind: Kind, class: &str) -> Vec<&'a str> {
        occurrences(src, kind, class).into_iter().map(|r| &src[r.start.saturating_sub(1)..r.end + 1]).collect()
    }

    #[test]
    fn css_selectors() {
        let css = "/* .btn alt */\n.btn, a.btn:hover > .btn-big { margin: .5em; background: url(img.btn.png); content: \".btn\"; }\n@media (x) { .nav .btn{} }\n.btn2 {}";
        assert_eq!(found(css, Kind::Css, "btn"), [".btn,", ".btn:", ".btn{"]);
        assert_eq!(found(css, Kind::Css, "btn-big").len(), 1);
        assert!(found(css, Kind::Css, "5em").is_empty());
    }

    #[test]
    fn markup_and_scripts() {
        let html = r##"<div class="card btn  big" data-class="btn"><a class='btn' href="#">x</a>
<p class="<?php echo $c; ?> btn">y</p>
<style>.btn { color: red }</style>
<script>
el.classList.add('btn', 'other'); el.classList.toggle("btn");
document.querySelectorAll('.list .btn > a'); $(".btn").addClass('btn active');
const s = 'btn'; el.className = 'btn';
</script>"##;
        let hits = occurrences(html, Kind::Markup, "btn");
        assert_eq!(hits.len(), 10, "{:?}", hits.iter().map(|r| &html[r.start - 5..r.end + 3]).collect::<Vec<_>>());
        // Not in data-class, not in a plain JS string.
        assert!(hits.iter().all(|r| !html[..r.start].ends_with("data-class=\"")));
        assert!(!hits.iter().any(|r| html[..r.start].ends_with("const s = '")));
    }

    #[test]
    fn jsx_and_usage() {
        let jsx = "export const A = () => <button className=\"btn primary\" onClick={() => el.classList.remove('primary')}>x</button>;";
        assert_eq!(occurrences(jsx, Kind::Mixed, "primary").len(), 2);
        let css = ".used{} .unused{} .x .used{}";
        assert_eq!(defined(css, Kind::Css).len(), 4);
        let page = "<div class=\"used\"></div><style>.inline{}</style>";
        assert_eq!(used(page, Kind::Markup), BTreeSet::from(["used".to_string()]));
        assert_eq!(defined(page, Kind::Markup)[0].0, "inline");
        assert!(valid("btn-primary") && valid("_x") && !valid("1x") && !valid("a b") && !valid(""));
        assert_eq!(name_at(page, Kind::Markup, 13), Some("used".to_string()));
    }
}
