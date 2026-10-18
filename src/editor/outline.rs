//! Symbol path at the cursor for the jump bar (e.g. `body › main#content › ul`).

use std::sync::LazyLock;

use regex::Regex;

use super::document::Document;
use crate::lang::Language;

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// How far back to scan; keeps this cheap on huge files.
const WINDOW: usize = 300_000;

pub fn symbol_path(doc: &Document, pos: usize) -> Vec<String> {
    let before = doc.slice(pos.saturating_sub(WINDOW)..pos);
    match doc.language {
        l if l.is_css() => css_path(&before),
        Language::Html | Language::Php => {
            let mut path = html_path(&before);
            // Inside <style>: continue with the CSS rule.
            if path.last().is_some_and(|p| p == "style")
                && let Some(i) = before.to_ascii_lowercase().rfind("<style")
                && let Some(gt) = before[i..].find('>')
            {
                path.extend(css_path(&before[i + gt + 1..]));
            }
            path
        }
        _ => Vec::new(),
    }
}

fn html_path(before: &str) -> Vec<String> {
    static TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<!--.*?-->|<(/?)([A-Za-z][\w:-]*)((?:[^<>"']|"[^"]*"|'[^']*')*?)(/?)>"#)
            .unwrap()
    });
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bid\s*=\s*["']([^"']+)["']"#).unwrap());
    static CLASS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\bclass\s*=\s*["']([^"']+)["']"#).unwrap());

    let mut stack: Vec<(String, String)> = Vec::new();
    for cap in TAG.captures_iter(before) {
        let Some(name) = cap.get(2) else { continue };
        let name = name.as_str().to_ascii_lowercase();
        if &cap[1] == "/" {
            if let Some(i) = stack.iter().rposition(|(n, _)| *n == name) {
                stack.truncate(i);
            }
            continue;
        }
        if &cap[4] == "/" || VOID.contains(&name.as_str()) {
            continue;
        }
        let attrs = &cap[3];
        let mut label = name.clone();
        if let Some(id) = ID.captures(attrs) {
            label.push('#');
            label.push_str(&id[1]);
        } else if let Some(class) = CLASS.captures(attrs)
            && let Some(first) = class[1].split_whitespace().next()
        {
            label.push('.');
            label.push_str(first);
        }
        stack.push((name, label));
    }
    stack.into_iter().map(|(_, label)| label).collect()
}

fn css_path(before: &str) -> Vec<String> {
    // Selectors of all blocks still open at the cursor.
    let mut stack: Vec<String> = Vec::new();
    let mut segment_start = 0;
    let mut in_comment = false;
    let bytes = before.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if in_comment {
            if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_comment = false;
                i += 1;
                segment_start = i + 1;
            }
        } else {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    in_comment = true;
                    i += 1;
                }
                b'{' => {
                    let selector = before[segment_start..i].split_whitespace().collect::<Vec<_>>().join(" ");
                    stack.push(shorten(&selector));
                    segment_start = i + 1;
                }
                b'}' => {
                    stack.pop();
                    segment_start = i + 1;
                }
                b';' => segment_start = i + 1,
                _ => {}
            }
        }
        i += 1;
    }
    stack
}

fn shorten(s: &str) -> String {
    if s.chars().count() > 40 {
        format!("{}…", s.chars().take(39).collect::<String>())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::DocSource;
    use std::path::PathBuf;

    fn path_at_end(name: &str, text: &str) -> Vec<String> {
        let d = Document::new(DocSource::Local(PathBuf::from(name)), text);
        symbol_path(&d, d.len_chars())
    }

    #[test]
    fn html_elements() {
        let p = path_at_end(
            "a.html",
            "<html><body><br><main id=\"content\"><ul class=\"list big\"><li>a</li><li>",
        );
        assert_eq!(p, ["html", "body", "main#content", "ul.list", "li"]);
    }

    #[test]
    fn css_rules_and_style_blocks() {
        assert_eq!(path_at_end("a.css", "a{} @media (x) {\n .hero h1 { color: red"), ["@media (x)", ".hero h1"]);
        assert_eq!(path_at_end("a.html", "<head><style>body { mar"), ["head", "style", "body"]);
    }
}
